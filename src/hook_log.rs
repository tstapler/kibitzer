//! Appends a structured record of every real `PostToolUse` hook firing (i.e. after
//! `dedup::claim` has already dropped duplicate registrations) to a local JSONL log,
//! so findings can be reviewed later: what triggered, on what commit/branch, what
//! was actually written, and whether it was blocking or just advisory. Best-effort —
//! a logging failure never affects the hook's real pass/fail behavior.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::check::CheckResult;

/// Truncate large edit payloads before logging so one giant Write doesn't dominate
/// the log file; full content is already on disk at `file_path` if it's ever needed.
const MAX_SNIPPET_LEN: usize = 4000;

pub(crate) fn log_path() -> PathBuf {
    crate::cache::default_cache_path()
        .parent()
        .map(|p| p.join("hook-log.jsonl"))
        .unwrap_or_else(|| std::env::temp_dir().join("kibitzer-hook-log.jsonl"))
}

fn git_output(cwd: &Path, args: &[&str]) -> Option<String> {
    let mut cmd = crate::git_cmd::git_command(cwd);
    cmd.args(args);
    let output = crate::git_cmd::bounded_output(cmd, crate::git_cmd::BASELINE_TIMEOUT)?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if value.is_empty() { None } else { Some(value) }
}

fn git_head(cwd: &Path) -> Option<String> {
    git_output(cwd, &["rev-parse", "HEAD"])
}

fn git_branch(cwd: &Path) -> Option<String> {
    git_output(cwd, &["rev-parse", "--abbrev-ref", "HEAD"])
}

fn truncate(s: &str) -> String {
    if s.len() <= MAX_SNIPPET_LEN {
        s.to_string()
    } else {
        format!(
            "{}... [truncated {} bytes]",
            &s[..MAX_SNIPPET_LEN],
            s.len() - MAX_SNIPPET_LEN
        )
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "tool", rename_all = "snake_case")]
pub enum EditSummary {
    Write {
        content: String,
    },
    Edit {
        old_string: Option<String>,
        new_string: String,
    },
    MultiEdit {
        edits: Vec<EditPair>,
    },
    Unknown,
}

#[derive(Debug, Serialize)]
pub struct EditPair {
    pub old_string: Option<String>,
    pub new_string: String,
}

impl EditSummary {
    pub fn write(content: &str) -> Self {
        EditSummary::Write {
            content: truncate(content),
        }
    }

    pub fn edit(old_string: Option<&str>, new_string: &str) -> Self {
        EditSummary::Edit {
            old_string: old_string.map(truncate),
            new_string: truncate(new_string),
        }
    }

    pub fn multi_edit(edits: &[(Option<&str>, &str)]) -> Self {
        EditSummary::MultiEdit {
            edits: edits
                .iter()
                .map(|(old, new)| EditPair {
                    old_string: old.map(truncate),
                    new_string: truncate(new),
                })
                .collect(),
        }
    }
}

#[derive(Serialize)]
struct HookLogEntry<'a> {
    timestamp_unix: u64,
    tool_use_id: Option<&'a str>,
    cwd: &'a Path,
    git_commit: Option<String>,
    git_branch: Option<String>,
    hook_event_name: &'a str,
    file_path: &'a Path,
    changed_lines: Option<&'a [(usize, usize)]>,
    edit: &'a EditSummary,
    results: &'a [CheckResult],
    blocked: bool,
}

/// Records one hook firing. Every field beyond the essentials is best-effort: a
/// missing git repo, an unwritable cache dir, or a serialization failure just
/// drops the log line rather than surfacing an error to the hook's caller.
#[allow(clippy::too_many_arguments)]
pub fn record(
    tool_use_id: Option<&str>,
    cwd: &Path,
    hook_event_name: &str,
    file_path: &Path,
    changed_lines: Option<&[(usize, usize)]>,
    edit: &EditSummary,
    results: &[CheckResult],
    blocked: bool,
) {
    let timestamp_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let (git_commit, git_branch) =
        crate::git_cmd::with_git_budget(crate::git_cmd::HOOK_GIT_BUDGET, || {
            (git_head(cwd), git_branch(cwd))
        });
    let entry = HookLogEntry {
        timestamp_unix,
        tool_use_id,
        cwd,
        git_commit,
        git_branch,
        hook_event_name,
        file_path,
        changed_lines,
        edit,
        results,
        blocked,
    };

    let Ok(line) = serde_json::to_string(&entry) else {
        return;
    };
    let path = log_path();
    let Some(parent) = path.parent() else { return };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    else {
        return;
    };
    let _ = writeln!(file, "{line}");
}

/// What a daemon note is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteKind {
    /// Hooks ran without the daemon (its runtime directory could not be trusted at all).
    Degraded,
    /// A refused `XDG_RUNTIME_DIR` was replaced by kibitzer's private fallback directory.
    Fallback,
}

impl NoteKind {
    fn tag(self) -> &'static str {
        match self {
            Self::Degraded => "degraded",
            Self::Fallback => "fallback",
        }
    }

    fn from_tag(tag: &str) -> Option<Self> {
        [Self::Degraded, Self::Fallback]
            .into_iter()
            .find(|k| k.tag() == tag)
    }

    fn event(self) -> &'static str {
        match self {
            Self::Degraded => "daemon_degraded",
            Self::Fallback => "daemon_fallback",
        }
    }
}

/// A note stays "already reported" this long, and at most `MAX_NOTES_PER_HOUR` distinct notes are
/// logged in that window, so a hook storm (or two alternating reasons) cannot flood the log.
const NOTE_WINDOW_SECS: u64 = 3600;
const MAX_NOTES_PER_HOUR: usize = 6;

/// One line of the marker file next to the log: `<unix ts>\t<kind tag>\t<text>`. The marker is the
/// state `kibitzer status` reads, so a cleared note disappears from it.
struct Note {
    ts: u64,
    kind: NoteKind,
    text: String,
}

fn read_notes(marker: &Path) -> Vec<Note> {
    std::fs::read_to_string(marker)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '\t');
            let ts = parts.next()?.parse().ok()?;
            let kind = NoteKind::from_tag(parts.next()?)?;
            Some(Note {
                ts,
                kind,
                text: parts.next()?.to_string(),
            })
        })
        .collect()
}

fn write_notes(marker: &Path, notes: &[Note]) {
    let body: String = notes
        .iter()
        .map(|n| format!("{}\t{}\t{}\n", n.ts, n.kind.tag(), n.text))
        .collect();
    let _ = std::fs::write(marker, body);
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn marker_in(dir: &Path) -> PathBuf {
    dir.join("daemon-degraded")
}

/// Records a daemon note in the marker and (once per hour per distinct note, at most
/// `MAX_NOTES_PER_HOUR` an hour) in the hook log.
pub fn note(kind: NoteKind, text: &str) {
    let path = log_path();
    let Some(parent) = path.parent() else { return };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }
    if !note_in(parent, unix_now(), kind, text) {
        return;
    }
    let line = serde_json::json!({
        "timestamp_unix": unix_now(),
        "event": kind.event(),
        "reason": text,
    });
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = writeln!(file, "{line}");
    }
}

/// Updates the marker under `dir`; true when this note is new and within the hourly cap, so the
/// caller should log it.
fn note_in(dir: &Path, now: u64, kind: NoteKind, text: &str) -> bool {
    let marker = marker_in(dir);
    let mut notes = read_notes(&marker);
    let recent = |n: &Note| now.saturating_sub(n.ts) < NOTE_WINDOW_SECS;
    if notes
        .iter()
        .any(|n| recent(n) && n.kind == kind && n.text == text)
    {
        return false;
    }
    // An old entry for the same note is replaced; an old entry is otherwise kept so `status`
    // still shows a degrade that has not recovered (its age is printed).
    notes.retain(|n| !(n.kind == kind && n.text == text));
    let reportable = notes.iter().filter(|n| recent(n)).count() < MAX_NOTES_PER_HOUR;
    notes.push(Note {
        ts: now,
        kind,
        text: text.to_string(),
    });
    notes.sort_by_key(|n| n.ts);
    let keep_from = notes.len().saturating_sub(MAX_NOTES_PER_HOUR * 2);
    write_notes(&marker, &notes[keep_from..]);
    reportable
}

/// Forgets the notes of `kinds`, as when the daemon is reachable again.
pub fn clear_notes(kinds: &[NoteKind]) {
    if let Some(parent) = log_path().parent() {
        clear_in(parent, kinds);
    }
}

fn clear_in(dir: &Path, kinds: &[NoteKind]) {
    let marker = marker_in(dir);
    let notes = read_notes(&marker);
    if notes.is_empty() {
        return;
    }
    let (gone, kept): (Vec<Note>, Vec<Note>) =
        notes.into_iter().partition(|n| kinds.contains(&n.kind));
    if gone.is_empty() {
        return;
    }
    if kept.is_empty() {
        let _ = std::fs::remove_file(&marker);
    } else {
        write_notes(&marker, &kept);
    }
}

/// Notes still in force (not cleared by a later success), newest last, with their unix times.
pub fn current_notes() -> Vec<(u64, NoteKind, String)> {
    log_path()
        .parent()
        .map(|dir| {
            read_notes(&marker_in(dir))
                .into_iter()
                .map(|n| (n.ts, n.kind, n.text))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
#[allow(non_snake_case)]
mod note_tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kz-notes-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn note_in_should_ReportOncePerHour_When_SameReasonRepeats() {
        let dir = scratch("once");
        assert!(note_in(&dir, 1000, NoteKind::Degraded, "a"));
        assert!(!note_in(&dir, 1500, NoteKind::Degraded, "a"));
        assert!(note_in(
            &dir,
            1000 + NOTE_WINDOW_SECS,
            NoteKind::Degraded,
            "a"
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn note_in_should_StayBounded_When_ReasonsAlternate() {
        let dir = scratch("alternate");
        let reported = (0..100u64)
            .filter(|i| note_in(&dir, 1000 + i, NoteKind::Degraded, &format!("r{}", i % 2)))
            .count();
        assert_eq!(reported, 2, "two alternating reasons are two notes an hour");
        let many = (0..100u64)
            .filter(|i| note_in(&dir, 2000 + i, NoteKind::Degraded, &format!("x{i}")))
            .count();
        assert!(many <= MAX_NOTES_PER_HOUR, "{many}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn clear_in_should_DropOnlyTheNamedKinds_And_RemoveEmptyMarker() {
        let dir = scratch("clear");
        note_in(&dir, 1000, NoteKind::Degraded, "d");
        note_in(&dir, 1001, NoteKind::Fallback, "f");
        clear_in(&dir, &[NoteKind::Degraded]);
        let left = read_notes(&marker_in(&dir));
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].kind, NoteKind::Fallback);
        clear_in(&dir, &[NoteKind::Fallback]);
        assert!(!marker_in(&dir).exists());
        clear_in(&dir, &[NoteKind::Fallback]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
