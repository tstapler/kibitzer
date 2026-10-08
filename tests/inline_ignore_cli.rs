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
