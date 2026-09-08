//! Implements the Claude Code `Stop` hook: re-checks every file touched (via
//! Edit/Write/MultiEdit) since the last time this transcript was processed, unscoped
//! (whole-file, not diff-scoped) — closing the gap where `PostToolUse`'s diff-scoping
//! can drop a real finding whose location doesn't overlap the specific edit that
//! introduced it (e.g. a file-size/duplicate-code finding anchored away from the lines
//! that pushed it over).
//!
//! Deliberately advisory-only: `Stop` has no documented loop-prevention mechanism (no
//! `stop_hook_active`-style field — verified against <https://code.claude.com/docs/en/hooks>,
//! 2026-09), so a hook that could re-block forever if Claude's fix attempt doesn't touch
//! the same file again would have no safety net. This hook never exits 2; it only ever
//! reports via `additionalContext`.
//!
//! Parses the session transcript JSONL directly, which Claude Code's own docs describe
//! as an internal format that can change between releases — the same tradeoff
//! `backtest.rs` already accepts for the same reason (there's no documented alternative
//! that exposes "every file touched", only the single most recent one via
//! `last_tool_use`). Best-effort: a parse failure or format change degrades to "found
//! nothing new" rather than erroring the hook.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The trigger name `Stop`-hook checks run under — distinct from `"batch"` (manual/CI
/// whole-repo sweeps) and `"PostToolUse"` (per-edit, diff-scoped) even though, like
/// batch, it runs checks unscoped. Only per-file checks that opt in via
/// `Check.triggers` fire here; whole-repo `architecture_checker` checks stay
/// batch-only (unaffected — nothing here changes that constraint).
pub const TRIGGER: &str = "Stop";

/// Persistent, transcript-path-keyed record of how far into each transcript this hook
/// has already looked, so a later `Stop` firing only re-checks files touched *since*
/// the last one instead of the whole session every time. Byte offsets, not line
/// counts, so a partially-written trailing line can be safely deferred to the next run
/// (see [`new_lines_since`]).
#[derive(Debug, Default, Serialize, Deserialize)]
struct StopOffsets {
    entries: BTreeMap<String, u64>,
}

impl StopOffsets {
    fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_string(self)?)?;
        Ok(())
    }
}

fn default_offsets_path() -> PathBuf {
    if let Ok(dir) = std::env::var("XDG_CACHE_HOME") {
        return PathBuf::from(dir).join("kibitzer").join("stop-offsets.json");
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".cache")
        .join("kibitzer")
        .join("stop-offsets.json")
}

/// Returns every complete line appended to `path` after `from_offset`, plus the byte
/// offset that covers exactly those complete lines (i.e. the position right after the
/// last `\n` found) — a trailing line with no terminating newline yet (the transcript
/// writer mid-append when this hook happens to fire) is left unconsumed, so the next
/// call picks it up whole rather than a truncated half-line getting parsed as invalid
/// JSON and silently dropped forever.
fn new_lines_since(path: &Path, from_offset: u64) -> Option<(Vec<String>, u64)> {
    let raw = std::fs::read(path).ok()?;
    let from = (from_offset as usize).min(raw.len());
    let slice = &raw[from..];
    let Some(last_newline) = slice.iter().rposition(|&b| b == b'\n') else {
        return Some((Vec::new(), from_offset));
    };
    let complete = &slice[..=last_newline];
    let consumed = from as u64 + last_newline as u64 + 1;
    let text = String::from_utf8_lossy(complete);
    let lines = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(str::to_string)
        .collect();
    Some((lines, consumed))
}

/// Extracts every distinct file path targeted by an Edit/Write/MultiEdit `tool_use`
/// block across `lines` (each expected to be one JSONL transcript line). Mirrors
/// `backtest.rs::reconstruct_snapshots`'s field access for the same tool_use shape,
/// but only needs presence of `file_path` — not before/after content reconstruction,
/// since this hook re-checks the file's current on-disk state rather than replaying
/// history. Unparseable lines are skipped rather than treated as an error, matching
/// `backtest.rs`'s tolerance for the same not-a-stable-contract transcript format.
fn touched_files(lines: &[String]) -> BTreeSet<PathBuf> {
    let mut files = BTreeSet::new();
    for line in lines {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if value.get("type").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let Some(content) = value.pointer("/message/content").and_then(Value::as_array) else {
            continue;
        };
        for block in content {
            if block.get("type").and_then(Value::as_str) != Some("tool_use") {
                continue;
            }
            let name = block.get("name").and_then(Value::as_str).unwrap_or_default();
            if !matches!(name, "Edit" | "Write" | "MultiEdit") {
                continue;
            }
            if let Some(file_path) = block
                .pointer("/input/file_path")
                .and_then(Value::as_str)
                .map(PathBuf::from)
            {
                files.insert(file_path);
            }
        }
    }
    files
}

/// Runs the `Stop` hook: finds every file touched since this transcript was last
/// processed, re-checks each one (unscoped) against whatever checks opt into
/// [`TRIGGER`], and reports failures via `additionalContext`. Never blocks (see the
/// module doc for why) — always exits success.
pub fn run_stop_hook(cwd: &Path, transcript_path: Option<&Path>) -> Result<ExitCode> {
    run_stop_hook_with_offsets_path(cwd, transcript_path, &default_offsets_path())
}

/// Same as [`run_stop_hook`], but with the offsets-store path injected — lets tests
/// exercise this against a scratch file instead of the real
/// `~/.cache/kibitzer/stop-offsets.json`, which would otherwise both pollute real
/// state and make repeat test runs see stale offsets from a prior run.
fn run_stop_hook_with_offsets_path(
    cwd: &Path,
    transcript_path: Option<&Path>,
    offsets_path: &Path,
) -> Result<ExitCode> {
    let Some(transcript_path) = transcript_path else {
        return Ok(ExitCode::SUCCESS);
    };
    let Ok(metadata) = std::fs::metadata(transcript_path) else {
        return Ok(ExitCode::SUCCESS);
    };

    let mut offsets = StopOffsets::load(offsets_path);
    let key = transcript_path.to_string_lossy().into_owned();
    let from_offset = offsets.entries.get(&key).copied().unwrap_or(0).min(metadata.len());

    let Some((lines, new_offset)) = new_lines_since(transcript_path, from_offset) else {
        return Ok(ExitCode::SUCCESS);
    };

    // Advance and persist the offset *before* running any checks, regardless of what
    // they find — this is the actual loop-safety mechanism (see module doc): the same
    // transcript content is never re-evaluated on a later `Stop`, so a forced
    // continuation that doesn't touch the file again can never re-trigger on it.
    offsets.entries.insert(key, new_offset);
    let _ = offsets.save(offsets_path);

    let files = touched_files(&lines);
    if files.is_empty() {
        return Ok(ExitCode::SUCCESS);
    }

    let mut lines_out: Vec<String> = Vec::new();
    for file in &files {
        if !file.is_file() {
            // Deleted (or never existed — a path from a failed tool call) later in the
            // same task; nothing on disk to check, and checking it would just report a
            // spurious "couldn't read file" finding.
            continue;
        }
        let results = crate::daemon::run_checks_smart(cwd, file, TRIGGER, None)?;
        for result in results.iter().filter(|r| !r.passed) {
            lines_out.push(format!(
                "{}: {}: {}",
                file.display(),
                result.check_name,
                result.describe()
            ));
        }
    }

    if lines_out.is_empty() {
        return Ok(ExitCode::SUCCESS);
    }

    let mut context = format!(
        "kibitzer re-checked {} file(s) touched this task (unscoped — this can surface \
         findings a per-edit check missed if they didn't overlap the specific lines that \
         edit changed):\n",
        files.len()
    );
    context.push_str(&lines_out.join("\n"));

    let payload = serde_json::json!({
        "hookSpecificOutput": {
            "hookEventName": "Stop",
            "additionalContext": context,
        }
    });
    println!("{payload}");
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn tool_use_line(name: &str, file_path: &str) -> String {
        serde_json::json!({
            "type": "assistant",
            "message": {"content": [{"type": "tool_use", "name": name, "input": {"file_path": file_path}}]}
        })
        .to_string()
    }

    fn tmp_path(name: &str) -> PathBuf {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("kibitzer-task-stop-test-{}-{name}-{n}", std::process::id()))
    }

    #[test]
    fn new_lines_since_returns_only_complete_lines_and_defers_a_partial_trailing_one() {
        let path = tmp_path("partial-line");
        std::fs::write(&path, "line one\nline two\npartial third line").unwrap();

        let (lines, offset) = new_lines_since(&path, 0).unwrap();
        assert_eq!(lines, vec!["line one".to_string(), "line two".to_string()]);
        assert_eq!(offset, "line one\nline two\n".len() as u64);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn new_lines_since_picks_up_a_completed_line_on_the_next_call() {
        let path = tmp_path("completed-later");
        std::fs::write(&path, "line one\npartial").unwrap();
        let (first_lines, offset) = new_lines_since(&path, 0).unwrap();
        assert_eq!(first_lines, vec!["line one".to_string()]);

        std::fs::write(&path, "line one\npartial line now done\n").unwrap();
        let (second_lines, _) = new_lines_since(&path, offset).unwrap();
        assert_eq!(second_lines, vec!["partial line now done".to_string()]);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn new_lines_since_from_current_end_finds_nothing() {
        let path = tmp_path("no-new-content");
        std::fs::write(&path, "line one\n").unwrap();
        let len = std::fs::metadata(&path).unwrap().len();
        let (lines, offset) = new_lines_since(&path, len).unwrap();
        assert!(lines.is_empty());
        assert_eq!(offset, len);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn touched_files_extracts_edit_write_and_multi_edit_but_not_read() {
        let lines = vec![
            tool_use_line("Edit", "/repo/a.go"),
            tool_use_line("Write", "/repo/b.go"),
            tool_use_line("MultiEdit", "/repo/c.go"),
            tool_use_line("Read", "/repo/d.go"),
        ];
        let files = touched_files(&lines);
        assert_eq!(
            files,
            BTreeSet::from([
                PathBuf::from("/repo/a.go"),
                PathBuf::from("/repo/b.go"),
                PathBuf::from("/repo/c.go"),
            ])
        );
    }

    #[test]
    fn touched_files_dedupes_the_same_file_edited_twice() {
        let lines = vec![tool_use_line("Edit", "/repo/a.go"), tool_use_line("Edit", "/repo/a.go")];
        assert_eq!(touched_files(&lines).len(), 1);
    }

    #[test]
    fn touched_files_ignores_unparseable_lines() {
        let lines = vec!["not json at all".to_string(), tool_use_line("Edit", "/repo/a.go")];
        assert_eq!(touched_files(&lines).len(), 1);
    }

    #[test]
    fn run_stop_hook_with_no_transcript_path_is_a_quiet_noop() {
        let dir = tmp_path("no-transcript-cwd");
        std::fs::create_dir_all(&dir).unwrap();
        let offsets = tmp_path("no-transcript-offsets.json");

        let result = run_stop_hook_with_offsets_path(&dir, None, &offsets).unwrap();

        assert_eq!(result, ExitCode::SUCCESS);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn run_stop_hook_skips_a_touched_file_that_no_longer_exists() {
        let dir = tmp_path("deleted-file-cwd");
        std::fs::create_dir_all(&dir).unwrap();
        let transcript = tmp_path("deleted-file-transcript.jsonl");
        std::fs::write(
            &transcript,
            tool_use_line("Edit", &dir.join("gone.go").to_string_lossy()) + "\n",
        )
        .unwrap();
        let offsets = tmp_path("deleted-file-offsets.json");

        let result = run_stop_hook_with_offsets_path(&dir, Some(&transcript), &offsets).unwrap();

        assert_eq!(result, ExitCode::SUCCESS);

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_file(&transcript).ok();
        std::fs::remove_file(&offsets).ok();
    }

    #[test]
    fn run_stop_hook_advances_the_offset_so_a_second_call_finds_nothing_new() {
        let dir = tmp_path("advances-cwd");
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("a.go");
        std::fs::write(&target, "package main\n").unwrap();
        let transcript = tmp_path("advances-transcript.jsonl");
        std::fs::write(
            &transcript,
            tool_use_line("Edit", &target.to_string_lossy()) + "\n",
        )
        .unwrap();
        let offsets = tmp_path("advances-offsets.json");

        run_stop_hook_with_offsets_path(&dir, Some(&transcript), &offsets).unwrap();
        let stored = StopOffsets::load(&offsets);
        let key = transcript.to_string_lossy().into_owned();
        let after_first = *stored.entries.get(&key).expect("offset recorded");
        assert_eq!(after_first, std::fs::metadata(&transcript).unwrap().len());

        // Nothing new appended — a second call must not re-derive the same touched
        // files (which would defeat the whole point of tracking an offset).
        run_stop_hook_with_offsets_path(&dir, Some(&transcript), &offsets).unwrap();
        let stored_again = StopOffsets::load(&offsets);
        assert_eq!(*stored_again.entries.get(&key).unwrap(), after_first);

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_file(&transcript).ok();
        std::fs::remove_file(&offsets).ok();
    }
}
