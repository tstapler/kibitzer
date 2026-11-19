//! End-to-end: `kibitzer run` over a real Go module clusters the same type-switch subject
//! across two files of one package, and honors `kibitzer:ignore`.

use std::path::{Path, PathBuf};
use std::process::Command;

const FINDING: &str = "[go-type-switch-clustering]";

const SWITCH: &str = "\tswitch r.kind.(type) {\n\tcase *Circle:\n\tcase *Square:\n\t}\n";

fn module(label: &str, second_file_body: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(label);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("shapes")).unwrap();
    std::fs::write(dir.join("go.mod"), "module example.com/m\n\ngo 1.22\n").unwrap();
    std::fs::write(
        dir.join("shapes/draw.go"),
        format!(
            "package shapes\n\ntype Circle struct{{}}\ntype Square struct{{}}\n\ntype Renderer struct{{ kind interface{{}} }}\n\nfunc (r *Renderer) Draw() {{\n{SWITCH}}}\n"
        ),
    )
    .unwrap();
    std::fs::write(dir.join("shapes/area.go"), second_file_body).unwrap();
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

fn area_file(directive: &str) -> String {
    format!("package shapes\n\nfunc (r *Renderer) Area() {{\n{directive}{SWITCH}}}\n")
}

#[test]
fn clusters_across_two_files() {
    let output = run(&module("tsc-cli-fires", &area_file("")));
    assert!(output.contains(FINDING), "{output}");
    assert!(output.contains("(Area, Draw)"), "{output}");
    assert!(
        output.contains("Replace Type Code with Subclasses"),
        "{output}"
    );
}

#[test]
fn single_method_is_not_reported() {
    let output = run(&module("tsc-cli-single", "package shapes\n"));
    assert!(!output.contains(FINDING), "{output}");
}

#[test]
fn ignore_directive_at_the_first_site_suppresses_the_finding() {
    // Anchor is the first site by file order: area.go sorts before draw.go.
    let ignored = area_file(
        "\t// kibitzer:ignore go-type-switch-clustering -- each shape kind is handled by a closed generated set of types\n",
    );
    let output = run(&module("tsc-cli-ignored", &ignored));
    assert!(!output.contains(FINDING), "{output}");
}
