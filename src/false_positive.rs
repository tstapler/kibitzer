//! Local queue for suspected-false-positive reports filed by an agent session (via the
//! `report_false_positive` MCP tool in `src/mcp.rs`) or the `check false-positives` CLI.
//! Kibitzer runs as an installed binary (`kibitzer mcp`) with no fixed relationship to its
//! own source checkout, so reports can't be appended straight into
//! `docs/<check>-false-positives.md` (see `docs/reporting-false-positives.md`) from an
//! arbitrary working directory — they land here first, and a maintainer periodically
//! drains the queue into the right doc.

use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FalsePositiveReport {
    /// `YYYY-MM-DD`, stamped when the report is filed (not user-supplied).
    pub date: String,
    pub check_name: String,
    pub file: String,
    pub what_changed: String,
    pub why_false_positive: String,
    #[serde(default)]
    pub mechanism: Option<String>,
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(default)]
    pub session: Option<String>,
}

/// `$XDG_DATA_HOME/kibitzer/false-positive-reports.jsonl`, falling back to
/// `$HOME/.local/share/kibitzer/false-positive-reports.jsonl` — mirrors
/// `plugin::default_plugin_dir()`'s shape against the same durable XDG base.
pub fn default_queue_path() -> PathBuf {
    if let Ok(dir) = std::env::var("XDG_DATA_HOME") {
        return PathBuf::from(dir)
            .join("kibitzer")
            .join("false-positive-reports.jsonl");
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("kibitzer")
        .join("false-positive-reports.jsonl")
}

pub fn append_report(path: &Path, report: &FalsePositiveReport) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("opening {}", path.display()))?;
    let line = serde_json::to_string(report).context("serializing false-positive report")?;
    writeln!(file, "{line}").with_context(|| format!("writing to {}", path.display()))?;
    Ok(())
}

/// Reads every queued report, skipping (rather than failing on) any line that doesn't
/// parse — a hand-edited or partially-written queue file shouldn't block triage of the
/// entries that are still readable.
pub fn read_reports(path: &Path) -> Result<Vec<FalsePositiveReport>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let reports = BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str::<FalsePositiveReport>(&line).ok())
        .collect();
    Ok(reports)
}

/// Renders one report in the `### <date> — <repo/session> — <summary>` shape
/// `docs/reporting-false-positives.md` documents, ready to append under a
/// `docs/<check>-false-positives.md`'s `## Log` heading.
pub fn format_markdown_entry(report: &FalsePositiveReport) -> String {
    let origin = match (&report.repo, &report.session) {
        (Some(repo), Some(session)) => format!("{repo} (session {session})"),
        (Some(repo), None) => repo.clone(),
        (None, Some(session)) => format!("session {session}"),
        (None, None) => "unknown repo/session".to_string(),
    };
    let mechanism = report
        .mechanism
        .clone()
        .unwrap_or_else(|| "not yet traced".to_string());
    format!(
        "### {date} — {origin} — {check_name}\n\n\
         - **Repo**: {origin}, file `{file}`.\n\
         - **What changed**: {what_changed}\n\
         - **Why it's a false positive**: {why_false_positive}\n\
         - **Mechanism**: {mechanism}\n",
        date = report.date,
        origin = origin,
        check_name = report.check_name,
        file = report.file,
        what_changed = report.what_changed,
        why_false_positive = report.why_false_positive,
        mechanism = mechanism,
    )
}

fn today_ymd() -> (i64, u32, u32) {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    civil_from_days(secs / 86400)
}

pub fn today_date_string() -> String {
    let (y, m, d) = today_ymd();
    format!("{y:04}-{m:02}-{d:02}")
}

/// Days-since-Unix-epoch → (year, month, day) in the proleptic Gregorian calendar (UTC).
/// Howard Hinnant's public-domain `civil_from_days` algorithm
/// (<http://howardhinnant.github.io/date_algorithms.html>) — used instead of pulling in a
/// date/time crate for one `YYYY-MM-DD` timestamp.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_path(name: &str) -> PathBuf {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        std::env::temp_dir().join(format!(
            "kibitzer-false-positive-test-{}-{name}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ))
    }

    fn sample() -> FalsePositiveReport {
        FalsePositiveReport {
            date: "2026-09-11".to_string(),
            check_name: "go-ignored-error".to_string(),
            file: "src/foo.go".to_string(),
            what_changed: "renamed a variable".to_string(),
            why_false_positive: "the error was already handled two lines up".to_string(),
            mechanism: Some("src/go_ignored_error.rs::scan_block".to_string()),
            repo: Some("tstapler/kibitzer".to_string()),
            session: Some("triage-1".to_string()),
        }
    }

    #[test]
    fn civil_from_days_matches_known_epoch_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19970), (2024, 9, 4));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
    }

    #[test]
    fn append_then_read_round_trips_a_report() {
        let path = tmp_path("roundtrip");
        append_report(&path, &sample()).expect("append succeeds");
        let reports = read_reports(&path).expect("read succeeds");
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].check_name, "go-ignored-error");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn append_is_cumulative_across_calls() {
        let path = tmp_path("cumulative");
        append_report(&path, &sample()).expect("append succeeds");
        append_report(&path, &sample()).expect("append succeeds");
        let reports = read_reports(&path).expect("read succeeds");
        assert_eq!(reports.len(), 2);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn read_reports_returns_empty_for_missing_file_not_an_error() {
        let path = tmp_path("missing");
        let reports = read_reports(&path).expect("missing file is not an error");
        assert!(reports.is_empty());
    }

    #[test]
    fn read_reports_skips_unparseable_lines() {
        let path = tmp_path("garbage");
        std::fs::write(&path, "not json\n").expect("write fixture");
        append_report(&path, &sample()).expect("append succeeds");
        let reports = read_reports(&path).expect("read succeeds despite garbage line");
        assert_eq!(reports.len(), 1);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn format_markdown_entry_includes_all_fields() {
        let entry = format_markdown_entry(&sample());
        assert!(entry.contains("### 2026-09-11"));
        assert!(entry.contains("tstapler/kibitzer"));
        assert!(entry.contains("session triage-1"));
        assert!(entry.contains("src/foo.go"));
        assert!(entry.contains("renamed a variable"));
        assert!(entry.contains("already handled two lines up"));
        assert!(entry.contains("src/go_ignored_error.rs::scan_block"));
    }

    #[test]
    fn format_markdown_entry_handles_missing_mechanism_and_origin() {
        let mut report = sample();
        report.mechanism = None;
        report.repo = None;
        report.session = None;
        let entry = format_markdown_entry(&report);
        assert!(entry.contains("not yet traced"));
        assert!(entry.contains("unknown repo/session"));
    }
}
