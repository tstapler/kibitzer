use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::json;

use crate::config::Severity;
use crate::daemon::run_checks_smart;

#[derive(Debug, Deserialize)]
struct HookInput {
    cwd: PathBuf,
    #[serde(default)]
    hook_event_name: String,
    #[serde(default)]
    tool_input: ToolInput,
}

#[derive(Debug, Default, Deserialize)]
struct ToolInput {
    file_path: Option<PathBuf>,
    /// Present for the Edit tool.
    new_string: Option<String>,
    /// Present for the Write tool — a full-file rewrite, so it's treated as
    /// unscoped (`changed_lines = None`) rather than diffed against anything.
    content: Option<String>,
    /// Present for the MultiEdit tool.
    edits: Option<Vec<EditItem>>,
}

#[derive(Debug, Deserialize)]
struct EditItem {
    new_string: String,
}

/// Derive 1-indexed, inclusive changed-line ranges in the *current* (post-edit)
/// on-disk content of `file_path`, from the Edit/MultiEdit tool input that produced
/// it. Returns `None` when there's nothing to scope to: a `Write` (whole-file
/// rewrite), no edit info at all, or a `new_string` that can't be located in the
/// current file content (e.g. a later edit already changed it again).
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
    for needle in new_strings {
        if needle.is_empty() {
            continue;
        }
        let Some(byte_offset) = file_content.find(needle) else {
            continue;
        };
        let start_line = file_content[..byte_offset].matches('\n').count() + 1;
        let end_line = start_line + needle.matches('\n').count();
        ranges.push((start_line, end_line));
    }

    if ranges.is_empty() {
        None
    } else {
        Some(ranges)
    }
}

/// Implements the Claude Code `PostToolUse` hook contract: read the event off stdin,
/// run any in-scope checks, and report back via stdout (advisory) or exit 2 + stderr
/// (blocking).
pub fn run_hook() -> Result<ExitCode> {
    let mut raw = String::new();
    std::io::stdin()
        .read_to_string(&mut raw)
        .context("reading hook input from stdin")?;
    let input: HookInput = serde_json::from_str(&raw).context("parsing hook input JSON")?;

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
    if results.is_empty() {
        return Ok(ExitCode::SUCCESS);
    }

    let failures: Vec<_> = results.iter().filter(|r| !r.passed).collect();
    if failures.is_empty() {
        return Ok(ExitCode::SUCCESS);
    }

    let blocking: Vec<_> = failures
        .iter()
        .filter(|r| r.severity == Severity::Blocking)
        .collect();

    if !blocking.is_empty() {
        for result in &blocking {
            eprintln!(
                "[kibitzer] {} (blocking): {}",
                result.check_name,
                result.message.as_deref().unwrap_or(&result.output)
            );
        }
        return Ok(ExitCode::from(2));
    }

    let context = failures
        .iter()
        .map(|r| {
            format!(
                "{}: {}",
                r.check_name,
                r.message.as_deref().unwrap_or(&r.output)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let payload = json!({
        "hookSpecificOutput": {
            "hookEventName": "PostToolUse",
            "additionalContext": context,
        }
    });
    println!("{payload}");
    Ok(ExitCode::SUCCESS)
}
