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

#[test]
fn kibitzer_check_native_should_BypassInlineIgnores_When_RunDirectly() {
    let dir = temp_dir("check-native");
    let file = dir.join("main.go");
    std::fs::write(&file, COVERED_GO).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_kibitzer"))
        .args(["check", "native", "syntax-rules"])
        .arg(&file)
        .output()
        .unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("[flag-argument]"), "{stdout}");
    assert!(!out.status.success());
    let _ = std::fs::remove_dir_all(&dir);
}

const INLINE_AND_ACCEPTED_GO: &str = "package main\n\n// kibitzer:ignore flag-argument -- legacy api pinned\nfunc f(b bool) {\n\tif b {\n\t\tprintln(\"x\")\n\t}\n}\n\nfunc g(c bool) {\n\tif c {\n\t\tprintln(\"y\")\n\t}\n}\n";

#[test]
fn kibitzer_run_should_ApplyBothSuppressors_When_InlineAndAcceptedMatchDifferentFindings() {
    let dir = temp_dir("both-suppressors");
    std::fs::write(dir.join("main.go"), INLINE_AND_ACCEPTED_GO).unwrap();
    let before = run_with_args(&dir, &[]);
    assert!(before.contains("main.go:10: [flag-argument]"), "{before}");
    assert!(!before.contains("main.go:4:"), "{before}");

    std::fs::create_dir_all(dir.join(".kibitzer/accepted")).unwrap();
    std::fs::write(
        dir.join(".kibitzer/accepted/g.json"),
        r#"{"rule": "flag-argument", "file": "main.go", "line": 10, "content": "func g(c bool) {", "reason": "kept on purpose"}"#,
    )
    .unwrap();
    let both = run_with_args(&dir, &[]);
    assert!(!both.contains("[flag-argument]"), "{both}");

    let raw = run_with_args(&dir, &["--no-inline-ignores"]);
    assert!(raw.contains("main.go:4: [flag-argument]"), "{raw}");
    assert!(
        !raw.contains("main.go:10:"),
        "accepted still applies: {raw}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn kibitzer_run_should_KeepIgnoreSuppressing_When_FileEditedElsewhere() {
    let dir = temp_dir("edited-elsewhere");
    let file = dir.join("main.go");
    std::fs::write(&file, COVERED_GO).unwrap();
    assert!(!run_with_args(&dir, &[]).contains("[flag-argument]"));

    let edited = format!(
        "// header added above\n\n{COVERED_GO}\nfunc unrelated() {{\n\tprintln(\"z\")\n}}\n"
    );
    std::fs::write(&file, edited).unwrap();
    let after = run_with_args(&dir, &[]);
    assert!(!after.contains("[flag-argument]"), "{after}");
    assert!(!after.contains("[unused-ignore]"), "{after}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn kibitzer_run_should_ApplyBothSuppressors_When_InlineAndAcceptedMatchSameFinding() {
    let dir = temp_dir("same-finding");
    std::fs::write(dir.join("main.go"), COVERED_GO).unwrap();
    std::fs::create_dir_all(dir.join(".kibitzer/accepted")).unwrap();
    std::fs::write(
        dir.join(".kibitzer/accepted/f.json"),
        r#"{"rule": "flag-argument", "file": "main.go", "line": 4, "content": "func f(b bool) {", "reason": "kept on purpose"}"#,
    )
    .unwrap();
    let default = run_with_args(&dir, &[]);
    assert!(!default.contains("[flag-argument]"), "{default}");
    let raw = run_with_args(&dir, &["--no-inline-ignores"]);
    assert!(
        !raw.contains("[flag-argument]"),
        "accepted alone suppresses: {raw}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

fn repo_file(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel)
}

/// Lines whose own rule tag (the first `: [tag]`) is a directive diagnostic; a line that merely
/// quotes the tag inside a string literal does not count.
fn directive_diagnostics(stdout: &str) -> Vec<&str> {
    stdout
        .lines()
        .filter(|l| {
            l.split_once(": [").is_some_and(|(_, rest)| {
                rest.starts_with("ignore-syntax]") || rest.starts_with("unused-ignore]")
            })
        })
        .collect()
}

/// Copies of this repo's own files that mention the marker only in string literals and
/// prose, run through the real `kibitzer run`.
#[test]
fn kibitzer_run_should_NotSuppressOrReport_When_MarkerOnlyInStringLiteralsOfOwnRepo() {
    let dir = temp_dir("self-run");
    for rel in [
        "src/inline_ignores/types.rs",
        "src/inline_ignores/parse.rs",
        "src/inline_ignores/hint.rs",
        "src/inline_ignores/tests.rs",
        "src/checkers/inline_ignore.rs",
        "src/hook.rs",
        "src/inline_post_pass.rs",
        "tests/inline_ignore_cli.rs",
        "tests/hook_contract.rs",
    ] {
        let name = rel.rsplit('/').next().unwrap();
        std::fs::copy(repo_file(rel), dir.join(name)).unwrap();
    }
    let stdout = run_with_args(&dir, &[]);
    assert!(
        directive_diagnostics(&stdout).is_empty(),
        "{:#?}",
        directive_diagnostics(&stdout)
    );
    assert!(
        !stdout
            .lines()
            .any(|l| l.starts_with("[kibitzer]") && l.contains("suppressed inline")),
        "{stdout}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn docs_examples_should_NotSelfSuppress_When_KibitzerRunOnDocs() {
    let out = Command::new(env!("CARGO_BIN_EXE_kibitzer"))
        .arg("run")
        .arg(repo_file("docs"))
        .output()
        .unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        directive_diagnostics(&stdout).is_empty(),
        "{:#?}",
        directive_diagnostics(&stdout)
    );
}

#[test]
fn docs_should_ContainNoNoInlineStance_When_GrepRun() {
    let stances = ["no inline", "no inline/per-line", "still no inline"];
    let mut files = vec![repo_file("CLAUDE.md"), repo_file("README.md")];
    for entry in std::fs::read_dir(repo_file("docs")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "md") {
            files.push(path);
        }
    }
    for path in files {
        let text = std::fs::read_to_string(&path).unwrap().to_lowercase();
        for stance in stances {
            assert!(!text.contains(stance), "{} says {stance:?}", path.display());
        }
    }
}

fn suppressing_checks_doc() -> String {
    std::fs::read_to_string(repo_file("docs/suppressing-checks.md")).unwrap()
}

#[test]
fn docs_suppressing_checks_should_ContainGrammarAndAnchorTable_When_Read() {
    let doc = suppressing_checks_doc();
    for needle in [
        "## Dismiss one finding inline",
        "### Grammar",
        "kibitzer:ignore <rule>[,<rule>...] -- <reason>",
        "One marker.",
        "### Where the comment goes",
        "| Checker | Where the ignore goes |",
        "`file-size`",
        "`duplicate-code`",
        "`markdown-link-integrity`",
        "Never suppressible:",
        "`blocking-suppressed`",
    ] {
        assert!(
            doc.contains(needle),
            "suppressing-checks.md lacks {needle:?}"
        );
    }
}

#[test]
fn docs_should_StateHookStaleIgnoreGapAndCheckNativeBypass_When_Read() {
    let doc = suppressing_checks_doc().replace("\n  ", " ");
    assert!(
        doc.contains("only when the directive sits inside the lines just edited"),
        "stale-ignore hook gap missing"
    );
    assert!(
        doc.contains("A stale ignore elsewhere in a file is not reported by the hook"),
        "stale-ignore-elsewhere gap missing"
    );
    assert!(
        doc.contains("`kibitzer check native <name> <file>`** runs the raw checker and bypasses inline ignores"),
        "check native bypass missing"
    );
}

fn exit_code_with_args(dir: &PathBuf, extra: &[&str]) -> (Option<i32>, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_kibitzer"))
        .arg("run")
        .arg(dir)
        .args(extra)
        .output()
        .unwrap();
    (out.status.code(), String::from_utf8(out.stdout).unwrap())
}

#[test]
fn kibitzer_run_should_ExitNonzero_When_DenyBlockingSuppressionAndBlockingFindingSuppressed() {
    let dir = temp_dir("deny-blocking");
    std::fs::write(dir.join("notes.md"), COVERED_MD).unwrap();
    let (default_code, _) = exit_code_with_args(&dir, &[]);
    assert_eq!(default_code, Some(0), "default behaviour is unchanged");
    let (code, stdout) = exit_code_with_args(&dir, &["--deny-blocking-suppression"]);
    assert_eq!(code, Some(1), "{stdout}");
    assert!(stdout.contains("--deny-blocking-suppression"), "{stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn kibitzer_run_should_ExitZero_When_DenyBlockingSuppressionAndOnlyAdvisorySuppressed() {
    let dir = temp_dir("deny-advisory-only");
    std::fs::write(dir.join("main.go"), TWO_COVERED_GO).unwrap();
    let (code, stdout) = exit_code_with_args(&dir, &["--deny-blocking-suppression"]);
    assert_eq!(code, Some(0), "{stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}
