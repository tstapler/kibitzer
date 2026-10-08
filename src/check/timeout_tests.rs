//! Epic 4.2 (Tech Debt item a): a hung `command` check must be killed and reported as a
//! timeout instead of blocking `run_check` forever, while a normal fast-exiting command is
//! completely unaffected. Exercises [`run_check_with_timeout`] directly with a short
//! duration so the hang test doesn't actually wait out a real-world timeout.

use super::*;
use crate::config::Severity;
use std::time::Instant;

fn command_check(command: &str) -> Check {
    Check {
        name: "hangy".to_string(),
        command: Some(command.to_string()),
        checker: None,
        architecture_checker: None,
        severity: Severity::Blocking,
        scope: vec![],
        triggers: vec![],
        message: Some("command check".to_string()),
        output_format: None,
        options: None,
    }
}

#[test]
fn run_check_reports_timeout_and_kills_process_when_command_hangs() {
    let started = Instant::now();
    let result = run_check_with_timeout(
        &command_check("sleep 60"),
        Path::new("."),
        Path::new("irrelevant.txt"),
        None,
        Duration::from_millis(200),
        &Registry::default(),
        &RunContext::default(),
    )
    .unwrap();

    assert!(
        started.elapsed() < Duration::from_secs(1),
        "run_check_with_timeout should return in well under 1s, took {:?}",
        started.elapsed()
    );
    assert!(!result.passed);
    assert!(
        result
            .message
            .as_deref()
            .unwrap_or("")
            .contains("timed out")
            || result.output.contains("timed out"),
        "expected 'timed out' in message or output, got message={:?} output={:?}",
        result.message,
        result.output
    );
}

#[test]
fn run_check_behaves_unchanged_when_command_exits_quickly() {
    let result = run_check_with_timeout(
        &command_check("true"),
        Path::new("."),
        Path::new("irrelevant.txt"),
        None,
        Duration::from_millis(200),
        &Registry::default(),
        &RunContext::default(),
    )
    .unwrap();

    assert!(result.passed);
    assert!(!result.output.to_lowercase().contains("timed out"));
}
