//! `kibitzer run` on a clean Rust-only directory: text mode prints the coverage note, `--json`
//! prints a parseable object without it.

use std::process::Command;

fn clean_rust_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("kibitzer-run-note-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("lib.rs"), "pub fn one() -> i32 {\n    1\n}\n").unwrap();
    dir
}

fn run(dir: &std::path::Path, extra: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_kibitzer"))
        .arg("run")
        .arg(dir)
        .args(extra)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn text_mode_notes_rust_coverage_and_json_mode_does_not() {
    let dir = clean_rust_dir();
    let text = run(&dir, &[]);
    let json = run(&dir, &["--json"]);
    std::fs::remove_dir_all(&dir).ok();

    assert!(text.contains("checked 1 .rs file"), "{text}");
    let parsed: serde_json::Value = serde_json::from_str(json.trim()).expect("--json is JSON");
    assert_eq!(parsed["blocking"], false);
    assert!(!json.contains(".rs file"), "{json}");
}
