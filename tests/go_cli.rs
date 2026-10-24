//! CLI-contract tests for `kibitzer go package-focus` and `kibitzer go build-cost`,
//! spawning the real built binary against the dogfood Go fixtures.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const MIXED: &str = "mixed_concerns";
const PACKAGE: &str = "package";
const LEVEL: &str = "level";

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/dogfood-architecture")
}

fn kibitzer(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_kibitzer"))
        .arg("go")
        .args(args)
        .output()
        .expect("spawn kibitzer")
}

fn package_focus(mode: Option<&str>) -> Vec<serde_json::Value> {
    let dir = fixtures();
    let dir = dir.to_str().unwrap();
    let mut args = vec!["package-focus", dir];
    if let Some(mode) = mode {
        args.extend(["--mode", mode]);
    }
    let out = kibitzer(&args);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("stdout is a JSON array")
}

fn packages(reports: &[serde_json::Value]) -> Vec<&str> {
    reports
        .iter()
        .map(|r| r[PACKAGE].as_str().unwrap())
        .collect()
}

#[test]
fn package_focus_reports_mixed_concerns_but_not_cohesive_packages() {
    let reports = package_focus(None);
    let names = packages(&reports);
    assert!(names.iter().any(|n| n.ends_with(MIXED)), "{names:?}");
    assert!(
        !names.iter().any(|n| n.ends_with("cohesive_parser")),
        "{names:?}"
    );
    assert!(
        !names.iter().any(|n| n.ends_with("shared_lock")),
        "{names:?}"
    );
}

#[test]
fn package_focus_report_matches_documented_json_schema() {
    let reports = package_focus(Some("combined"));
    let report = reports
        .iter()
        .find(|r| r[PACKAGE].as_str().unwrap().ends_with(MIXED))
        .expect("mixed_concerns report");
    for key in [
        PACKAGE,
        LEVEL,
        "hypothesis",
        "components",
        "cross_boundary_edges",
        "import_direction",
        "required_new_exports",
        "cycle_risks",
        "contrary_evidence",
        "coverage",
    ] {
        assert!(report.get(key).is_some(), "missing `{key}` in {report}");
    }
    assert_eq!(report[LEVEL], "advisory");
    assert!(!report["contrary_evidence"].as_array().unwrap().is_empty());
    assert!(report["coverage"]["notes"].is_array());
    assert!(report.get("anchor").is_none(), "anchor is internal");
}

#[test]
fn package_focus_modes_differ_as_documented() {
    let size_only = package_focus(Some("size-only"));
    assert!(
        size_only.iter().all(|r| r[LEVEL] == "informational"),
        "size-only is always informational"
    );
    let graph_only = package_focus(Some("graph-only"));
    assert!(packages(&graph_only).iter().any(|n| n.ends_with(MIXED)));
}

#[test]
fn package_focus_rejects_unknown_mode() {
    let dir = fixtures();
    let out = kibitzer(&["package-focus", dir.to_str().unwrap(), "--mode", "bogus"]);
    assert!(!out.status.success());
}

#[test]
fn build_cost_exits_zero_with_a_summary_or_a_diagnostic() {
    let dir = fixtures().join("multi_import");
    let out = kibitzer(&["build-cost", dir.to_str().unwrap()]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout);
    // Without a usable `go` the command still exits 0 and says why; with one it reports counts.
    assert!(
        text.contains("go build-cost") || text.contains("cold"),
        "unexpected output: {text}"
    );
}
