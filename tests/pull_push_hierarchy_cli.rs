//! End-to-end: `kibitzer run` (the default batch path, no config file) reports the five
//! inheritance-refactoring checks on small real repos, so registration, the `batch`
//! trigger and the default catalog are exercised, not just each checker's unit tests.

use std::path::{Path, PathBuf};
use std::process::Command;

const TAGS: [&str; 5] = [
    "[push-down-method]",
    "[push-down-field]",
    "[pull-up-method]",
    "[pull-up-constructor-body]",
    "[pull-up-field]",
];

const GO_SOURCE: &str = "package shapes

type Base struct {
	n     int
	p     int
	extra int
}

func (b *Base) log()   {}
func (b *Base) Shared() int { return 1 }
func (b *Base) Other() int  { return 2 }

type A struct {
	Base
	count int
}

type B struct {
	Base
	count int
}

func (a *A) Do() {
	a.n++
	a.log()
}

func (b *B) Do() {
	b.n++
	b.log()
}

func (a *A) RunA() int {
	a.p++
	return a.Shared()
}

func (b *B) RunB() int {
	return b.Other()
}
";

fn scratch(label: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(label);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn go_module(label: &str, subdir: &str, source: &str) -> PathBuf {
    let dir = scratch(label);
    std::fs::write(dir.join("go.mod"), "module example.com/m\n\ngo 1.22\n").unwrap();
    let pkg = dir.join(subdir);
    std::fs::create_dir_all(&pkg).unwrap();
    std::fs::write(pkg.join("shapes.go"), source).unwrap();
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
fn go_repo_reports_both_push_down_checks_and_the_go_pull_up_checks() {
    let output = run(&go_module("pph-go-fires", "shapes", GO_SOURCE));
    for tag in [
        "[push-down-method]",
        "[push-down-field]",
        "[pull-up-method]",
        "[pull-up-field]",
    ] {
        assert!(output.contains(tag), "missing {tag}:\n{output}");
    }
    // push-down-method: Shared is called only from A, Other only from B.
    assert!(
        output.contains("Base.Shared") && output.contains("Base.Other"),
        "{output}"
    );
    // push-down-field: `p` is touched only by A; `n` is used by both siblings so is not pushed down.
    assert!(output.contains("field 'p'"), "{output}");
    assert!(!output.contains("field 'n' of"), "{output}");
    // pull-up-method: Do has an identical body in A and B; pull-up-field: `count` in both.
    assert!(output.contains("'Do' has an identical body"), "{output}");
    assert!(output.contains("'count' (int)"), "{output}");
    // Go has no constructor keyword.
    assert!(!output.contains("[pull-up-constructor-body]"), "{output}");
}

#[test]
fn java_repo_reports_pull_up_constructor_body_and_pull_up_method() {
    let dir = scratch("pph-java-fires");
    std::fs::write(
        dir.join("Base.java"),
        "public abstract class Base {\n    protected Base(int x) {\n    }\n}\n",
    )
    .unwrap();
    for name in ["A", "B"] {
        std::fs::write(
            dir.join(format!("{name}.java")),
            format!(
                "public class {name} extends Base {{\n    private int x;\n    {name}(int x) {{\n        super(x);\n        this.x = x;\n    }}\n    void run() {{\n        step();\n        step();\n    }}\n    void step() {{\n    }}\n}}\n"
            ),
        )
        .unwrap();
    }
    let output = run(&dir);
    assert!(output.contains("[pull-up-constructor-body]"), "{output}");
    assert!(output.contains("[pull-up-method]"), "{output}");
    assert!(output.contains("'run' has an identical body"), "{output}");
    // Java has no field data yet, and no call-graph data.
    for tag in ["[pull-up-field]", "[push-down-method]", "[push-down-field]"] {
        assert!(!output.contains(tag), "unexpected {tag}:\n{output}");
    }
}

#[test]
fn a_supertype_with_a_single_subtype_reports_nothing() {
    let only_a = "package shapes

type Base struct{ n int }

func (b *Base) Shared() int { return 1 }

type A struct{ Base }

func (a *A) RunA() int {
	a.n++
	return a.Shared()
}
";
    let output = run(&go_module("pph-go-single", "shapes", only_a));
    for tag in TAGS {
        assert!(!output.contains(tag), "unexpected {tag}:\n{output}");
    }
}

#[test]
fn files_under_testdata_are_ignored() {
    let output = run(&go_module("pph-go-testdata", "testdata", GO_SOURCE));
    for tag in TAGS {
        assert!(!output.contains(tag), "unexpected {tag}:\n{output}");
    }
}
