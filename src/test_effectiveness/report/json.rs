//! JSON rendering of a `Report` (Task 2.3.1a). Key order is part of the contract: the envelope keys first, then
//! `findings`, `not_assessed` and `suppressed`. `serde_json` keeps insertion order, so the order is explicit here.

use serde_json::{Map, Value, json};

use super::envelope::{Format, SCHEMA_VERSION, Scope};
use super::{FindingValue, NotAssessed, Report, TestFinding};

/// A class E condition: no report is produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassE {
    pub id: &'static str,
    pub message: String,
    pub hint: Option<String>,
}

fn value_json(v: &FindingValue) -> Value {
    match v {
        FindingValue::UniqueCoverage {
            unique_units,
            covered_units,
            unit_kind,
        } => json!({
            "kind": "unique_coverage",
            "unique_units": unique_units,
            "covered_units": covered_units,
            "unit_kind": unit_kind,
        }),
        FindingValue::UniqueKills {
            unique_mutants,
            killed_mutants,
        } => json!({
            "kind": "unique_kills",
            "unique_mutants": unique_mutants,
            "killed_mutants": killed_mutants,
            "unit_kind": "mutant",
        }),
        FindingValue::Static { evidence } => json!({"kind": "static", "evidence": evidence}),
    }
}

fn finding_json(f: &TestFinding, show_confidence: bool) -> Value {
    let mut m = Map::new();
    m.insert("rule".to_string(), json!(f.rule.rule_id()));
    m.insert("section".to_string(), json!(f.section.as_str()));
    m.insert("signal_mode".to_string(), json!(f.signal_mode.as_str()));
    m.insert("input".to_string(), json!(f.input));
    m.insert(
        "test".to_string(),
        json!({"id": f.test_id, "file": f.file, "line": f.line}),
    );
    m.insert("value".to_string(), value_json(&f.value));
    m.insert("baseline".to_string(), json!(f.baseline));
    m.insert(
        "attribution".to_string(),
        json!(f.attribution.map(super::envelope::snake)),
    );
    m.insert("tool".to_string(), json!(f.tool));
    m.insert("tool_version".to_string(), json!(f.tool_version));
    m.insert("commit".to_string(), json!(f.commit));
    if show_confidence {
        m.insert("confidence".to_string(), json!(tier_str(f.confidence)));
    }
    m.insert("cost_seconds".to_string(), json!(f.cost_seconds));
    m.insert("overlaps_with".to_string(), json!(f.overlaps_with));
    m.insert(
        "overlaps_also_flagged".to_string(),
        json!(f.overlaps_also_flagged),
    );
    m.insert("leave_one_out".to_string(), Value::Null);
    Value::Object(m)
}

pub fn tier_str(t: crate::test_effectiveness::matrix::Tier) -> &'static str {
    use crate::test_effectiveness::matrix::Tier;
    match t {
        Tier::High => "high",
        Tier::Medium => "medium",
        Tier::Low => "low",
    }
}

fn not_assessed_json(n: &NotAssessed) -> Value {
    json!({
        "test": {"id": n.id, "file": n.file, "line": n.line},
        "reason": Scope::reason_key(n.reason),
    })
}

/// The report as a JSON document (pretty-printed, trailing newline).
pub fn render(report: &Report) -> String {
    let show_confidence = report.envelope.mode.shows_confidence();
    let mut doc = report.envelope.to_json_map();
    doc.insert(
        "findings".to_string(),
        Value::Array(
            report
                .findings
                .iter()
                .map(|f| finding_json(f, show_confidence))
                .collect(),
        ),
    );
    doc.insert(
        "not_assessed".to_string(),
        Value::Array(report.not_assessed.iter().map(not_assessed_json).collect()),
    );
    doc.insert("suppressed".to_string(), json!([]));
    pretty(&Value::Object(doc))
}

/// The minimal error object of class E under `--format json` or `brief`.
pub fn error_doc(format: Format, e: &ClassE) -> String {
    pretty(&json!({
        "schema_version": SCHEMA_VERSION,
        "format": format.as_str(),
        "error": {"id": e.id, "message": e.message, "hint": e.hint},
    }))
}

fn pretty(v: &Value) -> String {
    let mut s = serde_json::to_string_pretty(v).expect("a Value always serializes");
    s.push('\n');
    s
}
