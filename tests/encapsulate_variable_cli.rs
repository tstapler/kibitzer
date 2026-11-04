//! End-to-end: `kibitzer run` over a real two-package Go module reports the cross-package
//! field write, and a `kibitzer:ignore encapsulate-variable` directive at the write site
//! suppresses it.

use std::path::{Path, PathBuf};
use std::process::Command;

const FINDING: &str = "[encapsulate-variable]";

fn module(label: &str, b_body: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(label);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("a")).unwrap();
    std::fs::create_dir_all(dir.join("b")).unwrap();
    std::fs::write(dir.join("go.mod"), "module example.com/m\n\ngo 1.22\n").unwrap();
    std::fs::write(
        dir.join("a/a.go"),
        "package a\n\ntype T struct{ Field int }\n",
    )
    .unwrap();
    std::fs::write(dir.join("b/b.go"), b_body).unwrap();
    dir
}

fn run(dir: &Path) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_kibitzer"))
        .arg("run")
        .arg(dir)
        .output()
        .expect("spawn kibitzer");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[test]
fn reports_cross_package_write() {
    let dir = module(
        "ev-flagged",
        "package b\n\nimport \"example.com/m/a\"\n\nfunc F(x *a.T) {\n\tx.Field = 1\n}\n",
    );
    let output = run(&dir);
    assert!(output.contains(FINDING), "{output}");
    assert!(output.contains("b.go"), "{output}");
}

#[test]
fn ignore_directive_at_the_write_site_suppresses_the_finding() {
    let dir = module(
        "ev-ignored",
        "package b\n\nimport \"example.com/m/a\"\n\nfunc F(x *a.T) {\n\t// kibitzer:ignore encapsulate-variable -- setter would be churn\n\tx.Field = 1\n}\n",
    );
    let output = run(&dir);
    assert!(!output.contains(FINDING), "{output}");

    let undirected = module(
        "ev-not-ignored",
        "package b\n\nimport \"example.com/m/a\"\n\nfunc F(x *a.T) {\n\tx.Field = 1\n}\n",
    );
    assert!(
        run(&undirected).contains(FINDING),
        "same body without the directive must fire"
    );
}

#[test]
fn same_package_write_is_not_reported() {
    let dir = module("ev-same-pkg", "package b\n");
    std::fs::write(
        dir.join("a/use.go"),
        "package a\n\nfunc F(x *T) {\n\tx.Field = 1\n}\n\nfunc G() {\n\tt := &T{}\n\tt.Field = 2\n}\n",
    )
    .unwrap();
    // Positive control: the same run must still report a genuine cross-package write.
    std::fs::write(
        dir.join("b/b.go"),
        "package b\n\nimport \"example.com/m/a\"\n\nfunc F(x *a.T) {\n\tx.Field = 1\n}\n",
    )
    .unwrap();
    let output = run(&dir);
    assert_eq!(output.matches(FINDING).count(), 1, "{output}");
    assert!(!output.contains("use.go"), "{output}");
}
