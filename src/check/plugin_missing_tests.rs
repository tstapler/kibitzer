//! Task 4.3.1b: a plugin-backed check whose registered binary is missing from disk must
//! short-circuit with `plugin_missing: true` and a forced `Advisory` severity — regardless
//! of the check's own configured severity — without ever spawning the command.

use super::*;
use crate::config::{OutputFormat, Severity};
use crate::plugin::test_support::with_xdg_data_home;
use crate::plugin::{InstalledPlugin, PluginName, Registry};
use std::time::Instant;

#[test]
fn run_check_returns_plugin_missing_result_with_advisory_severity_when_binary_absent() {
    with_xdg_data_home("check-plugin-missing", |dir| {
        let missing_binary = dir.join("kibitzer-check-plugin-missing-test-nonexistent-binary");
        let plugin = InstalledPlugin {
            name: PluginName::parse("kibitzer-stub-plugin").unwrap(),
            version: "0.1.0".to_string(),
            min_kibitzer_version: "0.1.0".to_string(),
            sha256: "deadbeef".to_string(),
            binary_path: missing_binary,
            severity: Severity::Advisory,
            scope: vec!["**/*".to_string()],
            triggers: vec!["batch".to_string()],
            output_format: OutputFormat::Sarif,
        };
        Registry::save(
            &crate::plugin::default_registry_path(),
            &Registry {
                plugins: vec![plugin],
            },
        )
        .unwrap();

        // Configured `Blocking`, even though a real plugin manifest would normally
        // set its own severity — proves the forced-Advisory downgrade happens
        // regardless of what the `Check` itself is configured with.
        let check = Check {
            name: "kibitzer-stub-plugin".to_string(),
            command: Some("echo should-never-run {file}".to_string()),
            checker: None,
            architecture_checker: None,
            severity: Severity::Blocking,
            scope: vec![],
            triggers: vec![],
            message: None,
            output_format: None,
            options: None,
        };

        let registry = Registry::load(&crate::plugin::default_registry_path());
        let started = Instant::now();
        let result = run_check(
            &check,
            Path::new("."),
            Path::new("irrelevant.txt"),
            None,
            &registry,
            &RunContext::default(),
        )
        .unwrap();

        assert!(
            started.elapsed() < Duration::from_secs(1),
            "should return near-instantly without spawning a process"
        );
        assert!(result.plugin_missing);
        assert!(!result.passed);
        assert_eq!(result.severity, Severity::Advisory);
        assert!(result.output.is_empty(), "no subprocess should have run");
        assert!(
            result.command.is_empty(),
            "short-circuit result carries no substituted command"
        );
        assert!(
            result
                .message
                .as_deref()
                .unwrap_or("")
                .contains("is not installed"),
            "message: {:?}",
            result.message
        );
    });
}
