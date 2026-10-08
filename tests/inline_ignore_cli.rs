//! Exercises the real `kibitzer run` binary against temp directories, covering the
//! `inline-ignore` checker's end-to-end contract (output lines, exit status).
#![allow(non_snake_case)]

use std::path::PathBuf;
use std::process::{Command, Output};

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kibitzer-cli-ii-{name}-{}", std::process::id()));
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

fn kibitzer_run(dir: &PathBuf) -> Output {
    Command::new(env!("CARGO_BIN_EXE_kibitzer"))
        .arg("run")
        .arg(dir)
        .output()
        .unwrap()
}

#[test]
fn kibitzer_run_should_PrintBothLines_When_MalformedIgnoreAboveRealFinding() {
    let dir = temp_dir("both-lines");
    std::fs::write(
        dir.join("main.go"),
        "package main\n\n// kibitzer:ignore flag-argument\nfunc f(b bool) {\n\tif b {\n\t\tprintln(\"x\")\n\t}\n}\n",
    )
    .unwrap();
    let stdout = String::from_utf8(kibitzer_run(&dir).stdout).unwrap();
    assert!(
        stdout.contains("[ignore-syntax] kibitzer:ignore flag-argument has no reason"),
        "{stdout}"
    );
    assert!(stdout.contains("[flag-argument]"), "{stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn kibitzer_run_should_PrintNoInlineIgnoreOutput_When_DirHasBinaryFiles() {
    let dir = temp_dir("binary");
    std::fs::write(dir.join("blob.png"), b"\x89PNG\r\n\x1a\n\xff\xfe").unwrap();
    std::fs::write(dir.join("bad.go"), b"package main\n// \xff\xfe\n").unwrap();
    let out = kibitzer_run(&dir);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(!stdout.contains("inline-ignore"), "{stdout}");
    assert!(!stdout.contains("ignore-syntax"), "{stdout}");
    assert!(out.status.success(), "{stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

const COVERED_GO: &str = "package main\n\n// kibitzer:ignore flag-argument -- legacy api pinned\nfunc f(b bool) {\n\tif b {\n\t\tprintln(\"x\")\n\t}\n}\n";

fn run_with_args(dir: &PathBuf, extra: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_kibitzer"))
        .arg("run")
        .arg(dir)
        .args(extra)
        .output()
        .unwrap();
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn kibitzer_run_should_PrintCoveredFinding_When_NoInlineIgnoresFlag() {
    let dir = temp_dir("raw-flag");
    std::fs::write(dir.join("main.go"), COVERED_GO).unwrap();
    let raw = run_with_args(&dir, &["--no-inline-ignores"]);
    assert!(raw.contains("[flag-argument]"), "{raw}");
    let default = run_with_args(&dir, &[]);
    assert!(!default.contains("[flag-argument]"), "{default}");
    let _ = std::fs::remove_dir_all(&dir);
}

const TWO_COVERED_GO: &str = "package main\n\n// kibitzer:ignore flag-argument -- legacy api pinned\nfunc f(b bool) {\n\tif b {\n\t\tprintln(\"x\")\n\t}\n}\n\n// kibitzer:ignore flag-argument -- legacy api pinned\nfunc g(c bool) {\n\tif c {\n\t\tprintln(\"y\")\n\t}\n}\n";
const COVERED_MD: &str = "# T\n\n<!-- kibitzer:ignore markdown-link-integrity -- placeholder for later -->\nSee [foo] here.\n";

fn last_line(stdout: &str) -> &str {
    stdout.lines().last().unwrap_or_default()
}

#[test]
fn kibitzer_run_should_PrintFooterWithCount_When_ThreeSuppressed() {
    let dir = temp_dir("footer-count");
    std::fs::write(dir.join("main.go"), TWO_COVERED_GO).unwrap();
    std::fs::write(dir.join("notes.md"), COVERED_MD).unwrap();
    let stdout = run_with_args(&dir, &[]);
    assert_eq!(
        last_line(&stdout),
        "[kibitzer] 3 findings suppressed inline (1 from blocking checks) (rerun with --no-inline-ignores to see them)",
        "{stdout}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn kibitzer_run_should_OmitBlockingShare_When_NoBlockingFindingSuppressed() {
    let dir = temp_dir("footer-no-blocking");
    std::fs::write(dir.join("main.go"), TWO_COVERED_GO).unwrap();
    let stdout = run_with_args(&dir, &[]);
    assert_eq!(
        last_line(&stdout),
        "[kibitzer] 2 findings suppressed inline (rerun with --no-inline-ignores to see them)",
        "{stdout}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn kibitzer_run_should_PrintNoFooter_When_NothingSuppressed() {
    let dir = temp_dir("footer-none");
    std::fs::write(dir.join("main.go"), TWO_COVERED_GO).unwrap();
    let raw = run_with_args(&dir, &["--no-inline-ignores"]);
    assert!(!raw.contains("suppressed inline"), "{raw}");
    let clean = temp_dir("footer-clean");
    std::fs::write(clean.join("main.go"), "package main\n").unwrap();
    assert!(!run_with_args(&clean, &[]).contains("suppressed inline"));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&clean);
}

const HINT: &str = "[kibitzer] to dismiss a finding you judged acceptable: <comment> kibitzer:ignore <rule> -- <why> (docs/suppressing-checks.md)";

#[test]
fn kibitzer_run_should_PrintSyntaxHintOnce_When_AtLeastOneFindingReported() {
    let dir = temp_dir("hint");
    let uncovered = "package main\n\nfunc f(b bool) {\n\tif b {\n\t\tprintln(\"x\")\n\t}\n}\n\nfunc g(c bool) {\n\tif c {\n\t\tprintln(\"y\")\n\t}\n}\n";
    std::fs::write(dir.join("main.go"), uncovered).unwrap();
    let stdout = run_with_args(&dir, &[]);
    assert_eq!(stdout.lines().filter(|l| *l == HINT).count(), 1, "{stdout}");
    let clean = temp_dir("hint-clean");
    std::fs::write(clean.join("main.go"), "package main\n").unwrap();
    assert!(!run_with_args(&clean, &[]).contains("kibitzer:ignore <rule>"));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&clean);
}

#[test]
fn kibitzer_run_should_PrintHintBeforeFooter_When_FindingAndSuppressionBothPresent() {
    let dir = temp_dir("hint-order");
    let source =
        format!("{TWO_COVERED_GO}\nfunc h(d bool) {{\n\tif d {{\n\t\tprintln(\"z\")\n\t}}\n}}\n");
    std::fs::write(dir.join("main.go"), source).unwrap();
    let stdout = run_with_args(&dir, &[]);
    let lines: Vec<&str> = stdout.lines().collect();
    let hint = lines.iter().position(|l| *l == HINT).expect(&stdout);
    assert_eq!(hint + 2, lines.len(), "hint then footer last: {stdout}");
    assert!(lines[hint + 1].contains("suppressed inline"), "{stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

const UNKNOWN_RULE_GO: &str =
    "package main\n\n// kibitzer:ignore made-up-rule -- legacy API, callers pinned\nfunc f() {}\n";

#[test]
fn kibitzer_run_should_PointToCheckList_When_IgnoreNamesUnknownRule() {
    let dir = temp_dir("unknown-rule");
    std::fs::write(dir.join("main.go"), UNKNOWN_RULE_GO).unwrap();
    let stdout = run_with_args(&dir, &[]);
    assert!(
        stdout.contains(
            "main.go:3: [unused-ignore] 'made-up-rule' is not a known rule or checker; run 'kibitzer check list' to see valid names"
        ),
        "{stdout}"
    );
    assert!(!stdout.contains("remove it"), "{stdout}");
    // Disabled ignores drop nothing, so every rule would look unknown: stay quiet.
    assert!(!run_with_args(&dir, &["--no-inline-ignores"]).contains("[unused-ignore]"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn kibitzer_run_should_PrintNoUnusedIgnore_When_IgnoreSuppressesFinding() {
    let dir = temp_dir("known-rule-used");
    std::fs::write(dir.join("main.go"), COVERED_GO).unwrap();
    let stdout = run_with_args(&dir, &[]);
    assert!(!stdout.contains("[unused-ignore]"), "{stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}
