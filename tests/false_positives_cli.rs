//! Exercises the real `kibitzer check false-positives list` binary against a private
//! `XDG_DATA_HOME`, so it never reads the queue of the machine running the tests.

use std::path::PathBuf;
use std::process::Command;

fn data_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kibitzer-fp-cli-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("kibitzer")).unwrap();
    dir
}

fn list(dir: &PathBuf) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_kibitzer"))
        .args(["check", "false-positives", "list"])
        .env("XDG_DATA_HOME", dir)
        .output()
        .unwrap();
    assert!(out.status.success());
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn empty_queue_says_so() {
    let dir = data_dir("empty");
    assert!(list(&dir).contains("no queued false-positive reports"));
}

#[test]
fn reports_are_grouped_by_check_name_in_sorted_order() {
    let dir = data_dir("grouped");
    let report = |check: &str| {
        format!(
            r#"{{"date":"2026-09-01","check_name":"{check}","file":"a.go","what_changed":"x","why_false_positive":"y"}}"#
        )
    };
    let queue = format!("{}\n{}\n", report("zeta-check"), report("alpha-check"));
    std::fs::write(dir.join("kibitzer/false-positive-reports.jsonl"), queue).unwrap();

    let stdout = list(&dir);
    let alpha = stdout
        .find("## docs/alpha-check-false-positives.md")
        .unwrap();
    let zeta = stdout
        .find("## docs/zeta-check-false-positives.md")
        .unwrap();
    assert!(alpha < zeta);
}
