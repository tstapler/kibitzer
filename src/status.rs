//! `kibitzer status`: summarizes the hook-firing log (`hook_log.rs`) so you can see
//! whether the PostToolUse hook is actually running and which checks are firing,
//! without hand-parsing the raw JSONL.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use serde::Deserialize;

use crate::check::CheckResult;
use crate::hook_log::log_path;

#[derive(Deserialize)]
struct HookLogEntry {
    timestamp_unix: u64,
    cwd: PathBuf,
    #[serde(default)]
    results: Vec<CheckResult>,
    blocked: bool,
}

#[derive(Default)]
struct CheckStats {
    fired: u64,
    failed: u64,
    blocked: u64,
}

fn format_age(secs_ago: u64) -> String {
    if secs_ago < 60 {
        format!("{secs_ago}s ago")
    } else if secs_ago < 3600 {
        format!("{}m ago", secs_ago / 60)
    } else if secs_ago < 86400 {
        format!("{}h ago", secs_ago / 3600)
    } else {
        format!("{}d ago", secs_ago / 86400)
    }
}

/// Reads `hook-log.jsonl` and prints a summary: total firings, time range, blocked
/// count, per-check pass/fail/blocked counts, and per-repo firing counts. Malformed
/// lines (e.g. from an older log format) are counted and skipped rather than failing
/// the whole report, matching `hook_log::record`'s best-effort philosophy.
pub fn run_status() -> Result<ExitCode> {
    let path = log_path();
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(_) => {
            println!(
                "No hook log found at {} — the PostToolUse hook hasn't fired yet.",
                path.display()
            );
            return Ok(ExitCode::SUCCESS);
        }
    };

    let mut total = 0u64;
    let mut blocked_total = 0u64;
    let mut unparsed = 0u64;
    let mut first_ts = u64::MAX;
    let mut last_ts = 0u64;
    let mut by_check: HashMap<String, CheckStats> = HashMap::new();
    let mut by_repo: HashMap<String, u64> = HashMap::new();

    for line in raw.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let entry: HookLogEntry = match serde_json::from_str(line) {
            Ok(entry) => entry,
            Err(_) => {
                unparsed += 1;
                continue;
            }
        };

        total += 1;
        first_ts = first_ts.min(entry.timestamp_unix);
        last_ts = last_ts.max(entry.timestamp_unix);
        if entry.blocked {
            blocked_total += 1;
        }
        *by_repo.entry(entry.cwd.display().to_string()).or_default() += 1;

        for result in &entry.results {
            let stats = by_check.entry(result.check_name.clone()).or_default();
            stats.fired += 1;
            if !result.passed {
                stats.failed += 1;
                if result.severity == crate::config::Severity::Blocking {
                    stats.blocked += 1;
                }
            }
        }
    }

    if total == 0 {
        println!(
            "Hook log at {} exists but has no parseable entries yet.",
            path.display()
        );
        return Ok(ExitCode::SUCCESS);
    }

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(last_ts);

    println!("kibitzer hook log: {}", path.display());
    println!("{total} firings ({blocked_total} blocked, {unparsed} unparsed lines skipped)");
    println!(
        "first: {}, last: {}",
        format_age(now.saturating_sub(first_ts)),
        format_age(now.saturating_sub(last_ts))
    );

    println!("\nBy check:");
    let mut checks: Vec<_> = by_check.into_iter().collect();
    checks.sort_by_key(|(_, stats)| std::cmp::Reverse(stats.fired));
    for (name, stats) in checks {
        println!(
            "  {:<40} fired {:>5}  failed {:>5}  blocked {:>5}",
            name, stats.fired, stats.failed, stats.blocked
        );
    }

    println!("\nBy repo:");
    let mut repos: Vec<_> = by_repo.into_iter().collect();
    repos.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    for (repo, count) in repos {
        println!("  {count:>5}  {repo}");
    }

    Ok(ExitCode::SUCCESS)
}
