use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::json;

use crate::check::CheckResult;
use crate::config::Severity;
use crate::daemon::run_checks_smart;

#[derive(Debug, Deserialize)]
struct HookInput {
    cwd: PathBuf,
    #[serde(default)]
    hook_event_name: String,
    #[serde(default)]
    tool_input: ToolInput,
    /// Uniquely identifies the underlying tool call. Claude Code invokes every
    /// matching hook registration independently (e.g. a global and a project-level
    /// `PostToolUse` entry both matching `Edit|Write`), each with the same
    /// `tool_use_id` — used to dedupe so checks don't run and report twice for one
    /// edit. Absent for callers that don't send it, in which case dedup is skipped.
    #[serde(default)]
    tool_use_id: Option<String>,
    /// Only present on a `Stop` event — the session transcript's path, consulted by
    /// `task_stop::run_stop_hook` instead of anything in `tool_input` (which a `Stop`
    /// payload doesn't carry at all).
    #[serde(default)]
    transcript_path: Option<PathBuf>,
}

#[derive(Debug, Default, Deserialize)]
struct ToolInput {
    file_path: Option<PathBuf>,
    /// Present for the Edit tool.
    new_string: Option<String>,
    /// Present for the Edit tool — the text being replaced.
    old_string: Option<String>,
    /// Present for the Write tool — a full-file rewrite, so it's treated as
    /// unscoped (`changed_lines = None`) rather than diffed against anything.
    content: Option<String>,
    /// Present for the MultiEdit tool.
    edits: Option<Vec<EditItem>>,
}

#[derive(Debug, Deserialize)]
struct EditItem {
    new_string: String,
    old_string: Option<String>,
}

/// Builds the "what was actually written" summary logged alongside each hook
/// firing, straight from the already-parsed tool input.
fn build_edit_summary(tool_input: &ToolInput) -> crate::hook_log::EditSummary {
    if let Some(content) = &tool_input.content {
        crate::hook_log::EditSummary::write(content)
    } else if let Some(edits) = &tool_input.edits {
        let pairs: Vec<(Option<&str>, &str)> = edits
            .iter()
            .map(|e| (e.old_string.as_deref(), e.new_string.as_str()))
            .collect();
        crate::hook_log::EditSummary::multi_edit(&pairs)
    } else if let Some(new_string) = &tool_input.new_string {
        crate::hook_log::EditSummary::edit(tool_input.old_string.as_deref(), new_string)
    } else {
        crate::hook_log::EditSummary::Unknown
    }
}

/// Derive 1-indexed, inclusive changed-line ranges in the *current* (post-edit)
/// on-disk content of `file_path`, from the Edit/MultiEdit tool input that produced
/// it. Returns `None` when there's nothing to scope to at all: a `Write` (whole-file
/// rewrite) or no edit info. When a `new_string` can't be located *uniquely* in the
/// current file content — e.g. it's boilerplate that already occurs elsewhere, such
/// as an identical setup block duplicated across several `t.Run` subtests — we can't
/// tell which occurrence is the actual edit, so we scope to the union of *all*
/// occurrences rather than falling back to an unscoped whole-file check: that keeps
/// unrelated, pre-existing findings elsewhere in the file suppressed, at the cost of
/// occasionally including a same-text occurrence that isn't the one that changed.
///
/// A pure deletion (every `new_string` in this call is empty — nothing was added,
/// only removed) is scoped to `Some(vec![])` rather than falling through to `None`:
/// the edit introduced no new content for a check to flag, so there's nothing to
/// search for in the post-edit file, but that's not the same as "unscoped" — an
/// unscoped whole-file rescan would still re-surface unrelated, pre-existing
/// findings elsewhere in the file (see docs/go-primitive-obsession-false-positives.md's
/// "deletion-only edit flagged" entry).
fn compute_changed_lines(
    tool_input: &ToolInput,
    file_path: &PathBuf,
) -> Option<Vec<(usize, usize)>> {
    if tool_input.content.is_some() {
        return None;
    }

    let new_strings: Vec<&str> = if let Some(edits) = &tool_input.edits {
        edits.iter().map(|e| e.new_string.as_str()).collect()
    } else if let Some(new_string) = &tool_input.new_string {
        vec![new_string.as_str()]
    } else {
        return None;
    };

    let file_content = std::fs::read_to_string(file_path).ok()?;
    let mut ranges = Vec::new();
    let mut saw_non_empty_needle = false;
    for needle in new_strings {
        if needle.is_empty() {
            continue;
        }
        saw_non_empty_needle = true;
        for (byte_offset, _) in file_content.match_indices(needle) {
            let start_line = file_content[..byte_offset].matches('\n').count() + 1;
            let end_line = start_line + needle.matches('\n').count();
            ranges.push((start_line, end_line));
        }
    }

    if !saw_non_empty_needle {
        // Every edit in this tool call was a pure deletion.
        Some(Vec::new())
    } else if ranges.is_empty() {
        None
    } else {
        Some(ranges)
    }
}

/// Implements Claude Code's `PostToolUse` and `Stop` hook contracts, dispatching on
/// `hook_event_name` — both are wired to the same `kibitzer hook` command (see
/// `install.rs`), so this is the single entry point Claude Code actually invokes.
/// `PostToolUse`: run any in-scope checks for the edit and report back via stdout
/// (advisory) or exit 2 + stderr (blocking). `Stop`: see `task_stop::run_stop_hook`.
pub fn run_hook() -> Result<ExitCode> {
    let mut raw = String::new();
    std::io::stdin()
        .read_to_string(&mut raw)
        .context("reading hook input from stdin")?;
    let input: HookInput = serde_json::from_str(&raw).context("parsing hook input JSON")?;

    if input.hook_event_name == "Stop" {
        return crate::task_stop::run_stop_hook(&input.cwd, input.transcript_path.as_deref());
    }

    if let Some(tool_use_id) = &input.tool_use_id
        && !crate::dedup::claim(tool_use_id)
    {
        // Another hook registration (e.g. a global + project-level entry both
        // matching this tool) already claimed this exact tool call. Exit quietly
        // rather than running checks and reporting the same findings twice.
        return Ok(ExitCode::SUCCESS);
    }

    let Some(file_path) = input.tool_input.file_path.clone() else {
        return Ok(ExitCode::SUCCESS);
    };

    let changed_lines = compute_changed_lines(&input.tool_input, &file_path);
    let results = run_checks_smart(
        &input.cwd,
        &file_path,
        &input.hook_event_name,
        changed_lines.as_deref(),
    )?;

    let failures: Vec<_> = results.iter().filter(|r| !r.passed).collect();
    let blocking: Vec<_> = failures
        .iter()
        .filter(|r| r.severity == Severity::Blocking)
        .collect();

    crate::hook_log::record(
        input.tool_use_id.as_deref(),
        &input.cwd,
        &input.hook_event_name,
        &file_path,
        changed_lines.as_deref(),
        &build_edit_summary(&input.tool_input),
        &results,
        !blocking.is_empty(),
    );

    if results.is_empty() {
        return Ok(ExitCode::SUCCESS);
    }
    if failures.is_empty() {
        return Ok(ExitCode::SUCCESS);
    }

    if !blocking.is_empty() {
        for result in &blocking {
            eprintln!(
                "[kibitzer] {} (blocking): {}",
                result.check_name,
                result.describe()
            );
        }
        eprintln!(
            "[kibitzer] to disable a check or exclude a file, see \
             https://github.com/tstapler/kibitzer/blob/master/docs/suppressing-checks.md"
        );
        return Ok(ExitCode::from(2));
    }

    let mut context = failures
        .iter()
        .map(|r| render_advisory_context_line(r))
        .collect::<Vec<_>>()
        .join("\n");
    context.push_str(
        "\n\nIf any of the above looks like a false positive (fired on content the edit \
         didn't actually introduce, or on a pattern the check misidentifies), see \
         https://github.com/tstapler/kibitzer/blob/master/docs/reporting-false-positives.md \
         for how to file it — don't just note it in passing. To turn a check off (repo-wide) \
         or exclude a specific file, see \
         https://github.com/tstapler/kibitzer/blob/master/docs/suppressing-checks.md.",
    );

    let payload = json!({
        "hookSpecificOutput": {
            "hookEventName": "PostToolUse",
            "additionalContext": context,
        }
    });
    println!("{payload}");
    Ok(ExitCode::SUCCESS)
}

/// One line of the PostToolUse advisory `additionalContext` for a failed, non-blocking
/// check (Task 4.3.1c). `[skipped]`-prefixed for `result.plugin_missing` — a missing
/// plugin install, never a real check failure, so an agent shouldn't read it as "ran and
/// found a defect." A `plugin_missing` result can never reach the *blocking* printer above
/// instead, since `run_check`'s early return (Task 4.3.1b) forces `Severity::Advisory`.
fn render_advisory_context_line(result: &CheckResult) -> String {
    if result.plugin_missing {
        format!("[skipped] {}: {}", result.check_name, result.describe())
    } else {
        format!("{}: {}", result.check_name, result.describe())
    }
}

#[cfg(test)]
mod advisory_context_rendering_tests {
    use super::*;
    use crate::config::Severity;

    fn result(plugin_missing: bool) -> CheckResult {
        CheckResult {
            check_name: "kibitzer-stub-plugin".to_string(),
            severity: Severity::Advisory,
            passed: false,
            output: String::new(),
            message: Some("plugin 'kibitzer-stub-plugin' is not installed".to_string()),
            command: String::new(),
            findings: Vec::new(),
            plugin_missing,
        }
    }

    #[test]
    fn hook_advisory_context_renders_skipped_prefix_for_plugin_missing_result() {
        let line = render_advisory_context_line(&result(true));
        assert!(
            line.starts_with("[skipped] kibitzer-stub-plugin:"),
            "got: {line}"
        );
    }

    #[test]
    fn hook_advisory_context_renders_unprefixed_for_a_normal_failure() {
        let line = render_advisory_context_line(&result(false));
        assert!(!line.starts_with("[skipped]"), "got: {line}");
        assert!(line.starts_with("kibitzer-stub-plugin:"), "got: {line}");
    }
}

#[cfg(test)]
mod compute_changed_lines_tests {
    use super::*;

    fn write_temp(name: &str, content: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("kibitzer-hook-test-{}-{name}", std::process::id()));
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn locates_unique_new_string() {
        let path = write_temp("unique", "line1\nline2\ntarget line\nline4\n");
        let tool_input = ToolInput {
            file_path: None,
            old_string: None,
            new_string: Some("target line".to_string()),
            content: None,
            edits: None,
        };
        let ranges = compute_changed_lines(&tool_input, &path);
        std::fs::remove_file(&path).ok();
        assert_eq!(ranges, Some(vec![(3, 3)]));
    }

    #[test]
    fn unions_all_occurrences_when_new_string_is_ambiguous() {
        let path = write_temp("ambiguous", "dup line\nother\ndup line\n");
        let tool_input = ToolInput {
            file_path: None,
            old_string: None,
            new_string: Some("dup line".to_string()),
            content: None,
            edits: None,
        };
        let ranges = compute_changed_lines(&tool_input, &path);
        std::fs::remove_file(&path).ok();
        // Can't tell which occurrence is the real edit, so scope to both rather
        // than bailing to an unscoped whole-file check.
        assert_eq!(ranges, Some(vec![(1, 1), (3, 3)]));
    }

    #[test]
    fn duplicated_subtest_boilerplate_scopes_to_all_copies_not_whole_file() {
        // Mirrors the primitive-obsession false positive: identical setup lines
        // duplicated into several t.Run subtests make new_string non-unique, but
        // scoping to just those occurrences (rather than the whole file) should
        // still exclude an unrelated, pre-existing flaggable signature elsewhere.
        let content = "package main\n\
func unrelated(a, b string) {}\n\
func TestX(t *testing.T) {\n\
\tt.Run(\"a\", func(t *testing.T) {\n\
\t\tsvc := New(x)\n\
\t})\n\
\tt.Run(\"b\", func(t *testing.T) {\n\
\t\tsvc := New(x)\n\
\t})\n\
}\n";
        let path = write_temp("dup-boilerplate", content);
        let tool_input = ToolInput {
            file_path: None,
            old_string: None,
            new_string: Some("\t\tsvc := New(x)".to_string()),
            content: None,
            edits: None,
        };
        let ranges = compute_changed_lines(&tool_input, &path).unwrap();
        std::fs::remove_file(&path).ok();
        assert_eq!(ranges, vec![(5, 5), (8, 8)]);
        assert!(
            !ranges
                .iter()
                .any(|&(start, end)| (start..=end).contains(&2))
        );
    }

    #[test]
    fn deletion_only_edit_scopes_to_empty_ranges_instead_of_unscoped() {
        // Regression for docs/go-primitive-obsession-false-positives.md's
        // "deletion-only edit flagged" entry: an Edit whose `new_string` is empty
        // (pure deletion, nothing added) must not fall back to `None` (unscoped —
        // which re-triggers a whole-file rescan and re-surfaces unrelated,
        // pre-existing findings) — it should scope to zero ranges instead.
        let path = write_temp(
            "deletion-only",
            "package main\nfunc stillHere(a, b string) {}\n",
        );
        let tool_input = ToolInput {
            file_path: None,
            old_string: Some("func deadCode(a, b string) {}\n".to_string()),
            new_string: Some(String::new()),
            content: None,
            edits: None,
        };
        let ranges = compute_changed_lines(&tool_input, &path);
        std::fs::remove_file(&path).ok();
        assert_eq!(ranges, Some(Vec::new()));
    }

    #[test]
    fn multi_edit_of_only_deletions_scopes_to_empty_ranges() {
        let path = write_temp("multi-deletion-only", "package main\n");
        let tool_input = ToolInput {
            file_path: None,
            old_string: None,
            new_string: None,
            content: None,
            edits: Some(vec![
                EditItem {
                    new_string: String::new(),
                    old_string: Some("removed one\n".to_string()),
                },
                EditItem {
                    new_string: String::new(),
                    old_string: Some("removed two\n".to_string()),
                },
            ]),
        };
        let ranges = compute_changed_lines(&tool_input, &path);
        std::fs::remove_file(&path).ok();
        assert_eq!(ranges, Some(Vec::new()));
    }

    #[test]
    fn write_tool_is_always_unscoped() {
        let path = write_temp("write", "whatever\n");
        let tool_input = ToolInput {
            file_path: None,
            old_string: None,
            new_string: None,
            content: Some("whatever\n".to_string()),
            edits: None,
        };
        let ranges = compute_changed_lines(&tool_input, &path);
        std::fs::remove_file(&path).ok();
        assert_eq!(ranges, None);
    }
}
