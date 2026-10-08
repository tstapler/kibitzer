use super::*;
use crate::config::OutputFormat;

fn sarif_check(sarif_json: &str) -> Check {
    Check {
        name: "sarif-linter".to_string(),
        command: Some(format!("cat <<'EOF'\n{sarif_json}\nEOF")),
        checker: None,
        architecture_checker: None,
        severity: Severity::Advisory,
        scope: vec![],
        triggers: vec![],
        message: Some("linter found issues".to_string()),
        output_format: Some(OutputFormat::Sarif),
        options: None,
    }
}

#[test]
fn renders_sarif_output_and_ignores_diff_scoping() {
    let sarif_json = r#"{"version": "2.1.0", "runs": [{"results": [
        {"level": "error", "ruleId": "no-foo", "message": {"text": "found a foo"},
         "locations": [{"physicalLocation": {"artifactLocation": {"uri": "src/lib.rs"},
         "region": {"startLine": 12}}}]}
    ]}]}"#;
    let result = run_check(
        &sarif_check(sarif_json),
        Path::new("."),
        Path::new("src/lib.rs"),
        Some(&[(1, 5)]),
        &Registry::default(),
        &RunContext::default(),
    )
    .unwrap();
    assert_eq!(
        result.output,
        "1 error(s)\nsrc/lib.rs:12: [error] found a foo (no-foo)"
    );
}

#[test]
fn falls_back_to_raw_output_when_stdout_is_not_sarif() {
    let check = Check {
        command: Some("echo 'not sarif at all'".to_string()),
        ..sarif_check("{}")
    };
    let result = run_check(
        &check,
        Path::new("."),
        Path::new("src/lib.rs"),
        None,
        &Registry::default(),
        &RunContext::default(),
    )
    .unwrap();
    assert_eq!(result.output.trim(), "not sarif at all");
}
