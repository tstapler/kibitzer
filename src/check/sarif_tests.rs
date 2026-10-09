use super::*;

fn sarif_log(results_json: &str) -> String {
    format!(r#"{{"version": "2.1.0", "runs": [{{"results": [{results_json}]}}]}}"#)
}

#[test]
fn renders_counts_and_findings() {
    let log = sarif_log(
        r#"{"level": "error", "ruleId": "no-foo", "message": {"text": "found a foo"},
            "locations": [{"physicalLocation": {"artifactLocation": {"uri": "src/lib.rs"},
            "region": {"startLine": 12}}}]}"#,
    );
    let rendered = render_sarif_output(log.as_bytes()).unwrap();
    assert_eq!(
        rendered,
        "1 error(s)\nsrc/lib.rs:12: [error] found a foo (no-foo)"
    );
}

#[test]
fn defaults_missing_level_to_warning() {
    let log = sarif_log(r#"{"message": {"text": "no level given"}, "locations": []}"#);
    let rendered = render_sarif_output(log.as_bytes()).unwrap();
    assert_eq!(rendered, "1 warning(s)\n[warning] no level given");
}

#[test]
fn empty_results_render_as_zero_findings() {
    let log = r#"{"version": "2.1.0", "runs": [{"results": []}]}"#;
    let rendered = render_sarif_output(log.as_bytes()).unwrap();
    assert_eq!(rendered, "0 findings");
}

#[test]
fn counts_multiple_levels_separately() {
    let log = sarif_log(
        r#"{"level": "error", "message": {"text": "e1"}, "locations": []},
           {"level": "error", "message": {"text": "e2"}, "locations": []},
           {"level": "note", "message": {"text": "n1"}, "locations": []}"#,
    );
    let rendered = render_sarif_output(log.as_bytes()).unwrap();
    assert!(rendered.starts_with("2 error(s), 1 note(s)\n"));
}

#[test]
fn returns_none_for_invalid_json() {
    assert!(render_sarif_output(b"not json").is_none());
}
