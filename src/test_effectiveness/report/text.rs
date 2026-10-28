//! Minimal plain-text rendering so the default `--format text` is usable.
//!
//! The real renderer (wrap rule, legend, "Before acting" block, stratified `--top`, per-mode columns) is
//! Task 2.3.1b; this file is deliberately small and is meant to be replaced by it.

use std::collections::BTreeMap;

use crate::inline_ignores::sanitize::escape_path;

use super::envelope::Scope;
use super::json::tier_str;
use super::{FindingValue, Report, TestFinding};

fn bare_name(id: &str) -> &str {
    id.rsplit_once("::").map_or(id, |(_, name)| name)
}

fn row(f: &TestFinding, mode_shows_confidence: bool) -> String {
    let mut out = format!(
        "{}:{}: [{}] {}\n",
        escape_path(&f.file),
        f.line,
        f.rule.rule_id(),
        escape_path(bare_name(&f.test_id))
    );
    let alone = match &f.value {
        FindingValue::UniqueCoverage {
            unique_units,
            covered_units,
            unit_kind,
        } => format!("{unique_units} unique {unit_kind}s of {covered_units} covered"),
        FindingValue::UniqueKills {
            unique_mutants,
            killed_mutants,
        } => format!("{unique_mutants} unique mutants of {killed_mutants} killed"),
        FindingValue::Static { evidence } => evidence.clone(),
    };
    out.push_str(&format!("    alone: {alone}\n"));
    if mode_shows_confidence {
        out.push_str(&format!("    conf: {}\n", tier_str(f.confidence)));
    }
    if !f.overlaps_with.is_empty() {
        let partners: Vec<String> = f
            .overlaps_with
            .iter()
            .map(|p| {
                let star = if f.overlaps_also_flagged.contains(p) {
                    "*"
                } else {
                    ""
                };
                format!("{}{star}", escape_path(bare_name(p)))
            })
            .collect();
        out.push_str(&format!("    overlaps: {}\n", partners.join(", ")));
    }
    out
}

fn not_assessed_line(scope: &Scope) -> String {
    let parts: Vec<String> = scope
        .by_reason
        .iter()
        .filter(|(_, n)| *n > 0)
        .map(|(r, n)| format!("{} {n}", r.label()))
        .collect();
    if parts.is_empty() {
        "Not assessed (0)".to_string()
    } else {
        format!(
            "Not assessed ({}): {}",
            scope.not_assessed(),
            parts.join(" | ")
        )
    }
}

fn omitted_line(rest: &[TestFinding]) -> String {
    let mut by_rule: BTreeMap<&str, usize> = BTreeMap::new();
    for f in rest {
        *by_rule.entry(f.rule.rule_id()).or_default() += 1;
    }
    let parts: Vec<String> = by_rule.iter().map(|(r, n)| format!("{r} {n}")).collect();
    format!("... {} more: {}\n", rest.len(), parts.join(" | "))
}

/// Renders the report; `top == 0` shows every row.
pub fn render(report: &Report, top: usize) -> String {
    let env = &report.envelope;
    let mut out = format!("{}\nkibitzer tests report\n", env.banner());
    for input in &env.header.inputs {
        let version = input.tool_version.as_deref().unwrap_or("?");
        let commit = input.commit.as_deref().unwrap_or("unknown");
        let attribution = input.attribution.map_or(String::new(), |a| {
            let assumed = if input.attribution_assumed {
                " (assumed)"
            } else {
                ""
            };
            format!(", attribution: {a}{assumed}")
        });
        out.push_str(&format!(
            "input {}: {} {}, commit {}{}\n",
            input.flag,
            escape_path(&input.tool),
            escape_path(version),
            escape_path(commit),
            attribution
        ));
    }
    let scope = &env.header.scope;
    out.push_str(&format!(
        "scope: assessed {} of {} tests in the input ({} not assessed)\n",
        scope.assessed(),
        scope.tests_in_input,
        scope.not_assessed()
    ));
    for w in &env.warnings {
        out.push_str(&w.line());
        out.push('\n');
    }
    out.push('\n');
    let shows = env.mode.shows_confidence();
    if report.findings.is_empty() {
        out.push_str("note: no tests flagged.\n");
    } else {
        let heading = if shows {
            "Listed in order of confidence, then rule id, then test id. Order is not a removal priority."
        } else {
            "Listed in test id order."
        };
        out.push_str(&format!(
            "{} ({})\n{heading}\n\n",
            "Flagged tests",
            report.findings.len()
        ));
        let shown = if top == 0 {
            report.findings.len()
        } else {
            top.min(report.findings.len())
        };
        for f in &report.findings[..shown] {
            out.push_str(&row(f, shows));
        }
        if shown < report.findings.len() {
            out.push_str(&omitted_line(&report.findings[shown..]));
        }
    }
    out.push('\n');
    out.push_str(&not_assessed_line(scope));
    out.push('\n');
    out.push_str(env_non_endorsement(report));
    out.push('\n');
    out
}

fn env_non_endorsement(report: &Report) -> &'static str {
    if report.envelope.mode.shows_confidence() {
        "Absence of a flag does not mean a test is valuable."
    } else {
        "Absence from this list does not mean a test is valuable."
    }
}
