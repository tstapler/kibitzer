//! Registry-only exposure of the test-quality checkers (Story 1.1.5): reachable by name,
//! off by default, opt-in through `.kibitzer/inspect.json`, typos fail at config load.
#![allow(non_snake_case)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const VACUOUS_GO: &str =
    "package p\n\nimport \"testing\"\n\nfunc TestA(t *testing.T) {\n\tParse(\"\")\n}\n";
const CLEAN_GO: &str = "package p\n\nimport \"testing\"\n\nfunc TestA(t *testing.T) {\n\tif Parse(\"\") == nil {\n\t\tt.Fatal(\"nil\")\n\t}\n}\n";

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kibitzer-tqreg-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let status = Command::new("git")
        .args(["init", "-q"])
        .current_dir(&dir)
        .status()
        .unwrap();
    assert!(status.success());
    dir
}

fn kibitzer(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_kibitzer"))
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap()
}

fn run(dir: &Path) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_kibitzer"))
        .arg("run")
        .arg(dir)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn write_config(dir: &Path, json: &str) {
    std::fs::create_dir_all(dir.join(".kibitzer")).unwrap();
    std::fs::write(dir.join(".kibitzer/inspect.json"), json).unwrap();
}

#[test]
fn default_checks_should_NotContainTestQualityEntries_When_BeforeG1() {
    let dir = temp_dir("nodefault");
    std::fs::write(dir.join("a_test.go"), VACUOUS_GO).unwrap();
    let stdout = run(&dir);
    assert!(!stdout.contains("[test-no-assertion]"), "{stdout}");
}

#[test]
fn kibitzer_run_should_ReportFinding_When_InspectJsonEnablesTestNoAssertion() {
    let dir = temp_dir("optin");
    std::fs::write(dir.join("a_test.go"), VACUOUS_GO).unwrap();
    write_config(
        &dir,
        r#"{"checks":[{"name":"test-no-assertion-go","checker":"test-no-assertion-go","severity":"advisory","scope":["**/*.go"]}]}"#,
    );
    let stdout = run(&dir);
    assert!(stdout.contains("[test-no-assertion] TestA"), "{stdout}");
    assert!(
        stdout.contains("kibitzer:ignore test-no-assertion -- <why>"),
        "{stdout}"
    );
}

#[test]
fn s6_check_list_should_ShowDefaultOff_When_BeforeG1() {
    let dir = temp_dir("list-off");
    let out = kibitzer(&dir, &["check", "list"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let go = stdout
        .lines()
        .find(|l| l.starts_with("test-no-assertion-go:"))
        .unwrap_or_else(|| panic!("{stdout}"));
    assert!(go.contains("language: Go"), "{go}");
    assert!(go.ends_with("state: off)"), "{go}");
    let py = stdout
        .lines()
        .find(|l| l.starts_with("test-no-assertion-python:"))
        .unwrap();
    assert!(
        py.contains("language: Python") && py.ends_with("state: off)"),
        "{py}"
    );
    let summary = stdout
        .lines()
        .find(|l| l.starts_with("test-no-assertion (rule):"))
        .unwrap_or_else(|| panic!("{stdout}"));
    assert_eq!(
        summary,
        "test-no-assertion (rule): languages: Go, Python, Kotlin (state: off)"
    );
}

#[test]
fn s6_check_list_should_ShowOn_When_ConfigEnablesInstance() {
    let dir = temp_dir("list-on");
    write_config(
        &dir,
        r#"{"checks":[{"name":"test-no-assertion-go","checker":"test-no-assertion-go","severity":"advisory"}]}"#,
    );
    let stdout = String::from_utf8_lossy(&kibitzer(&dir, &["check", "list"]).stdout).into_owned();
    let go = stdout
        .lines()
        .find(|l| l.starts_with("test-no-assertion-go:"))
        .unwrap();
    assert!(go.ends_with("state: on)"), "{go}");
    let py = stdout
        .lines()
        .find(|l| l.starts_with("test-no-assertion-python:"))
        .unwrap();
    assert!(py.ends_with("state: off)"), "{py}");
}

#[test]
fn s6_check_list_should_OnlyAppendState_When_ExistingChecker() {
    let dir = temp_dir("list-compat");
    let stdout = String::from_utf8_lossy(&kibitzer(&dir, &["check", "list"]).stdout).into_owned();
    let line = stdout
        .lines()
        .find(|l| l.starts_with("comment-quality-go:"))
        .unwrap();
    assert!(
        line.contains("(language: Go, globs: **/*.go, state: on)"),
        "{line}"
    );
}

#[test]
fn s6_config_should_FailNamingKey_When_TypoInAssertionHelpers() {
    let dir = temp_dir("typo");
    std::fs::write(dir.join("a_test.go"), CLEAN_GO).unwrap();
    write_config(
        &dir,
        r#"{"checks":[{"name":"test-no-assertion-go","checker":"test-no-assertion-go","severity":"advisory","options":{"assertion_helper":["x"]}}]}"#,
    );
    let out = Command::new(env!("CARGO_BIN_EXE_kibitzer"))
        .arg("run")
        .arg(&dir)
        .output()
        .unwrap();
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!out.status.success(), "{all}");
    assert!(all.contains("assertion_helper"), "{all}");
    assert!(all.contains("assertion_helpers"), "{all}");
    assert!(!all.contains("[test-no-assertion]"), "{all}");
}

#[test]
fn ux_h4_hook_should_PrintNothing_When_NoIssues() {
    let dir = temp_dir("h4");
    std::fs::write(dir.join("a_test.go"), CLEAN_GO).unwrap();
    write_config(
        &dir,
        r#"{"checks":[{"name":"test-no-assertion-go","checker":"test-no-assertion-go","severity":"advisory"}]}"#,
    );
    let stdout = run(&dir);
    assert!(!stdout.contains("test-no-assertion"), "{stdout}");
    assert!(!stdout.contains("0 findings"), "{stdout}");
}

#[test]
fn ux_h5_hook_should_PrintNothingAndExitUnchanged_When_CsFile() {
    let dir = temp_dir("h5");
    std::fs::write(
        dir.join("FooTests.cs"),
        "class FooTests { void T() { Parse(\"\"); } }\n",
    )
    .unwrap();
    write_config(
        &dir,
        r#"{"checks":[{"name":"test-no-assertion-go","checker":"test-no-assertion-go","severity":"advisory"}]}"#,
    );
    let baseline = Command::new(env!("CARGO_BIN_EXE_kibitzer"))
        .arg("run")
        .arg(&dir)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&baseline.stdout);
    assert!(!stdout.contains("test-no-assertion"), "{stdout}");
    assert!(baseline.status.success());
}

#[test]
fn ux_h6_copied_ignore_line_should_Suppress_When_WhyReplaced() {
    let dir = temp_dir("h6");
    write_config(
        &dir,
        r#"{"checks":[{"name":"test-no-assertion-go","checker":"test-no-assertion-go","severity":"advisory"}]}"#,
    );
    let source = VACUOUS_GO.replace(
        "func TestA",
        "// kibitzer:ignore test-no-assertion -- smoke test, covered elsewhere\nfunc TestA",
    );
    std::fs::write(dir.join("a_test.go"), source).unwrap();
    let stdout = run(&dir);
    assert!(!stdout.contains("[test-no-assertion]"), "{stdout}");
}
