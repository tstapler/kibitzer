//! Closes the gap this fix is for: `ArchFinding.severity_override` was set correctly by
//! every checker (verified by architecture_checks.rs/declaration_checks.rs's own unit
//! tests) but was never threaded past `run_architecture_check` into `CheckResult` — so
//! `mcp.rs`/`main.rs` had no way to render a finding's own effective severity, only the
//! one `CheckResult.severity` flattened uniformly across every finding a `Check` produced.
//! These tests prove `CheckResult.findings` now carries that per-finding data end to end.

use super::*;
use crate::config::{ArchitectureConfig, Component, Severity};

fn component_deps_check(severity: Severity) -> Check {
    Check {
        name: "component-deps".to_string(),
        command: None,
        checker: None,
        architecture_checker: Some("component-deps".to_string()),
        severity,
        scope: vec![],
        triggers: vec![],
        message: None,
        output_format: None,
        options: None,
    }
}

/// A single Go package with no imports at all: `ComponentDependencyChecker` has no
/// edges to flag as a real violation, but the declared "ghost" component's glob
/// matches zero import-graph nodes, so the checker's own
/// `zero_match_advisory`-produced `ArchFinding` (`severity_override:
/// Some(Severity::Advisory)`) is the *only* finding produced.
fn write_zero_match_only_fixture(dir: &Path) {
    std::fs::create_dir_all(dir.join("pkg")).unwrap();
    std::fs::write(dir.join("go.mod"), "module fixture\ngo 1.21\n").unwrap();
    std::fs::write(dir.join("pkg/pkg.go"), "package pkg\n\nfunc F() {}\n").unwrap();
}

#[test]
fn component_deps_flags_zero_match_component_as_advisory_even_when_check_is_blocking() {
    let dir = std::env::temp_dir().join(format!(
        "kibitzer-findings-wiring-test-{}-{}",
        std::process::id(),
        TMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    write_zero_match_only_fixture(&dir);

    let arch_config = ArchitectureConfig {
        components: vec![Component {
            name: "ghost".to_string(),
            paths: vec!["**/ghost".to_string(), "**/ghost/**".to_string()],
        }],
        ..Default::default()
    };
    let files = walk_and_collect_files(&dir).unwrap();

    // Check.severity is Blocking — the exact configuration that, before this fix,
    // made a harmless zero-match advisory masquerade as a blocking finding once it
    // reached rendered output (the bug this fix closes).
    let result = run_architecture_check(
        &component_deps_check(Severity::Blocking),
        &dir,
        &files,
        &arch_config,
    )
    .unwrap();

    let _ = std::fs::remove_dir_all(&dir);

    assert!(!result.passed);
    // The single finding is the zero-match advisory, carrying its own narrowed
    // severity, independent of `result.severity` (which stays whatever the
    // uniform/flattened `CheckResult.severity` resolves to).
    assert_eq!(result.findings.len(), 1, "findings: {:?}", result.findings);
    assert_eq!(
        result.findings[0].severity_override,
        Some(Severity::Advisory)
    );
    assert!(result.findings[0].message.contains("[component]"));
    assert!(result.findings[0].message.contains("matched 0"));
}

/// Same additive-field precedent as `command` (`src/check.rs:26-32` at the time this
/// was written): a `cache.json` blob written before `findings` existed must still
/// deserialize — as an empty `Vec` — instead of `Cache::load` discarding the whole
/// cache on the first run after upgrade.
#[test]
fn cache_json_without_findings_field_deserializes_with_empty_findings() {
    let old_shape_json = r#"{
        "check_name": "component-deps",
        "severity": "blocking",
        "passed": false,
        "output": "some finding",
        "message": null,
        "command": "kibitzer check architecture component-deps"
    }"#;
    let result: CheckResult = serde_json::from_str(old_shape_json)
        .expect("a pre-`findings`-field CheckResult must still deserialize");
    assert!(result.findings.is_empty());
    assert_eq!(result.check_name, "component-deps");
}
