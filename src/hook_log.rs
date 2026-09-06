//! Appends a structured record of every real `PostToolUse` hook firing (i.e. after
//! `dedup::claim` has already dropped duplicate registrations) to a local JSONL log,
//! so findings can be reviewed later: what triggered, on what commit/branch, what
//! was actually written, and whether it was blocking or just advisory. Best-effort —
//! a logging failure never affects the hook's real pass/fail behavior.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::check::CheckResult;

/// Truncate large edit payloads before logging so one giant Write doesn't dominate
/// the log file; full content is already on disk at `file_path` if it's ever needed.
const MAX_SNIPPET_LEN: usize = 4000;

fn log_path() -> PathBuf {
    crate::cache::default_cache_path()
        .parent()
        .map(|p| p.join("hook-log.jsonl"))
        .unwrap_or_else(|| std::env::temp_dir().join("kibitzer-hook-log.jsonl"))
}

fn git_output(cwd: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .ok()?;
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

    let entry = HookLogEntry {
        timestamp_unix,
        tool_use_id,
        cwd,
        git_commit: git_head(cwd),
        git_branch: git_branch(cwd),
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
