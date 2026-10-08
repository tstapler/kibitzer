use super::*;
use crate::config::Severity;

fn primitive_obsession_check() -> Check {
    Check {
        name: "native".to_string(),
        command: None,
        checker: Some("primitive-obsession".to_string()),
        architecture_checker: None,
        severity: Severity::Blocking,
        scope: vec![],
        triggers: vec![],
        message: Some("primitive obsession".to_string()),
        output_format: None,
        options: None,
    }
}

// Predates `test_support::unique_temp_dir` and stays on its own atomic-counter
// scheme rather than migrating: it's already collision-resistant even under
// parallel test threads, which nanosecond-timestamp uniqueness alone is not.
fn tmp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "kibitzer-native-check-test-{}-{name}-{}",
        std::process::id(),
        TMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn missing_file_degrades_to_failed_result_instead_of_erroring() {
    let dir = tmp_dir("missing-file");
    let file = dir.join("does-not-exist.go");

    let result = run_check(
        &primitive_obsession_check(),
        &dir,
        &file,
        None,
        &Registry::default(),
        &RunContext::default(),
    )
    .expect("a missing file must not abort the whole check run");
    assert!(!result.passed);
    assert!(result.output.contains("does-not-exist.go"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn file_not_matching_checkers_globs_is_skipped_rather_than_misparsed() {
    let dir = tmp_dir("wrong-glob");
    let file = dir.join("notes.md");
    std::fs::write(&file, "func f(a, b string) {}\n").unwrap();

    let result = run_check(
        &primitive_obsession_check(),
        &dir,
        &file,
        None,
        &Registry::default(),
        &RunContext::default(),
    )
    .unwrap();
    assert!(result.passed);
    assert_eq!(result.output, "");

    let _ = std::fs::remove_dir_all(&dir);
}

fn blank_imports_check() -> Check {
    Check {
        name: "native".to_string(),
        command: None,
        checker: Some("go-blank-imports".to_string()),
        architecture_checker: None,
        severity: Severity::Blocking,
        scope: vec![],
        triggers: vec![],
        message: Some("blank import".to_string()),
        output_format: None,
        options: None,
    }
}

#[test]
fn oversized_file_is_skipped_rather_than_parsed() {
    let dir = tmp_dir("oversized");
    let file = dir.join("huge.go");
    let mut content = String::from("package main\n\nimport (\n\t_ \"unjustified/pkg\"\n)\n");
    content.push_str(&"// padding\n".repeat(300_000)); // ~3.3MB, over MAX_NATIVE_CHECK_BYTES
    std::fs::write(&file, &content).unwrap();

    let result = run_check(
        &blank_imports_check(),
        &dir,
        &file,
        None,
        &Registry::default(),
        &RunContext::default(),
    )
    .unwrap();
    assert!(
        result.passed,
        "oversized file should be skipped rather than flagged"
    );
    assert!(result.output.is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}

// A `WholeRepoNative` (`architecture_checker`-set) `Check` has neither `checker`
// nor `command`, so it must be short-circuited before the `command`-only branch's
// `.expect()` — reached whenever a per-file trigger dispatch doesn't pre-filter by
// `is_per_file()` the way `run.rs::run_batch` does.
#[test]
fn whole_repo_native_check_dispatched_per_file_passes_trivially_instead_of_panicking() {
    let dir = tmp_dir("whole-repo-native-per-file");
    let file = dir.join("some-file.go");
    std::fs::write(&file, "package main\n").unwrap();

    let check = Check {
        name: "package-size".to_string(),
        command: None,
        checker: None,
        architecture_checker: Some("package-size".to_string()),
        severity: Severity::Advisory,
        scope: vec![],
        triggers: vec![],
        message: None,
        output_format: None,
        options: None,
    };

    let result = run_check(
        &check,
        &dir,
        &file,
        None,
        &Registry::default(),
        &RunContext::default(),
    )
    .unwrap();
    assert!(result.passed);
    assert!(result.output.is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}

// Proves criterion 6 concretely for a new native checker (not just
// primitive-obsession): two unjustified blank imports, one inside
// `changed_lines` and one outside it, and only the in-range one survives
// output-filtering and drives the pass/fail result.
#[test]
fn native_go_blank_imports_check_scopes_output_to_changed_lines() {
    let dir = tmp_dir("blank-imports-scoping");
    let file = dir.join("main.go");
    std::fs::write(
        &file,
        "package main\n\nimport (\n\t_ \"unjustified/outside\"\n\t_ \"unjustified/inside\"\n)\n",
    )
    .unwrap();

    // Line 4 (outside/pre-existing) is excluded; line 5 (inside) is the
    // only changed line, matching this file's `{file}:{line}:` findings.
    let registry = Registry::default();
    let run_ctx = RunContext::default();
    let result = run_check(
        &blank_imports_check(),
        &dir,
        &file,
        Some(&[(5, 5)]),
        &registry,
        &run_ctx,
    )
    .unwrap();
    assert!(!result.passed);
    assert!(result.output.contains("unjustified/inside"));
    assert!(!result.output.contains("unjustified/outside"));

    // Scoping to a range with no findings at all reports a pass, proving the
    // filtering — not just the checker itself — determines the outcome.
    let clean = run_check(
        &blank_imports_check(),
        &dir,
        &file,
        Some(&[(1, 1)]),
        &registry,
        &run_ctx,
    )
    .unwrap();
    assert!(clean.passed);

    let _ = std::fs::remove_dir_all(&dir);
}

// Regression for docs/go-primitive-obsession-false-positives.md's "pre-existing
// unchanged signatures flagged" entry: `changed_lines` scoping (already generic
// across every native checker via `run_native_check`) must exclude a
// primitive-obsession finding whose signature sits outside the edited range,
// even though `check_file` itself still parses and flags the whole file.
#[test]
fn native_primitive_obsession_check_scopes_output_to_changed_lines() {
    let dir = tmp_dir("primitive-obsession-scoping");
    let file = dir.join("tls.go");
    std::fs::write(
        &file,
        "package main\n\nfunc certCurrent(certFile, hashFile, want string) bool {\n\treturn true\n}\n\nfunc LoadTLSConfig(certFile, keyFile string) (int, error) {\n\treturn 0, nil\n}\n",
    )
    .unwrap();

    // Only line 7 (`LoadTLSConfig`) falls inside the edited range; line 3
    // (`certCurrent`) is pre-existing and untouched.
    let registry = Registry::default();
    let run_ctx = RunContext::default();
    let result = run_check(
        &primitive_obsession_check(),
        &dir,
        &file,
        Some(&[(7, 7)]),
        &registry,
        &run_ctx,
    )
    .unwrap();
    assert!(!result.passed);
    assert!(result.output.contains("LoadTLSConfig") || result.output.contains(":7:"));
    assert!(!result.output.contains("certCurrent"));

    // Scoping to a range that touches neither flagged signature reports a pass.
    let clean = run_check(
        &primitive_obsession_check(),
        &dir,
        &file,
        Some(&[(4, 4)]),
        &registry,
        &run_ctx,
    )
    .unwrap();
    assert!(clean.passed);

    let _ = std::fs::remove_dir_all(&dir);
}

// Regression for docs/go-primitive-obsession-false-positives.md's "deletion-only
// edit flagged" entry: a pure-deletion edit (`hook::compute_changed_lines` now
// returns `Some(vec![])` for one, instead of `None`/unscoped) must not re-surface
// an unrelated, pre-existing flaggable signature still left in the file.
#[test]
fn native_primitive_obsession_check_suppresses_findings_for_deletion_only_edit() {
    let dir = tmp_dir("primitive-obsession-deletion-only");
    let file = dir.join("tls.go");
    // What's left in the file after a hypothetical deletion of dead code —
    // `certCurrent` was already here, untouched by the edit.
    std::fs::write(
        &file,
        "package main\n\nfunc certCurrent(certFile, hashFile, want string) bool {\n\treturn true\n}\n",
    )
    .unwrap();

    let registry = Registry::default();
    let result = run_check(
        &primitive_obsession_check(),
        &dir,
        &file,
        Some(&[]),
        &registry,
        &RunContext::default(),
    )
    .unwrap();
    assert!(result.passed, "output: {}", result.output);
    assert_eq!(result.output, "");

    let _ = std::fs::remove_dir_all(&dir);
}

/// `json` is the old `{"accepted": [...]}` shape purely so call sites can keep
/// listing entries inline — split into one file per entry, the real on-disk layout.
fn write_accepted_findings(dir: &Path, json: &str) {
    let accepted_dir = dir.join(crate::accepted_findings::ACCEPTED_FINDINGS_DIR);
    std::fs::create_dir_all(&accepted_dir).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(json).unwrap();
    for (i, entry) in parsed["accepted"].as_array().unwrap().iter().enumerate() {
        std::fs::write(accepted_dir.join(format!("{i}.json")), entry.to_string()).unwrap();
    }
}

#[test]
fn accepted_finding_with_unchanged_content_suppresses_the_native_finding() {
    let dir = tmp_dir("accepted-unchanged");
    let file = dir.join("user.go");
    std::fs::write(
        &file,
        "package main\n\nfunc newUser(name, email string) {}\n",
    )
    .unwrap();
    write_accepted_findings(
        &dir,
        r#"{"accepted": [{"rule": "primitive-obsession", "file": "user.go", "line": 3, "content": "func newUser(name, email string) {}", "reason": "not worth a newtype here"}]}"#,
    );
    let run_ctx = RunContext::load(&dir).unwrap();

    let result = run_check(
        &primitive_obsession_check(),
        &dir,
        &file,
        None,
        &Registry::default(),
        &run_ctx,
    )
    .unwrap();
    assert!(result.passed, "output: {}", result.output);
    assert_eq!(result.output, "");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn accepted_finding_stops_suppressing_once_the_line_content_drifts() {
    let dir = tmp_dir("accepted-stale");
    let file = dir.join("user.go");
    // The signature grew a third same-typed parameter since the entry below was
    // written — the recorded `content` no longer matches, so the (now different)
    // finding must still surface rather than being silently swallowed forever.
    std::fs::write(
        &file,
        "package main\n\nfunc newUser(name, email, nickname string) {}\n",
    )
    .unwrap();
    write_accepted_findings(
        &dir,
        r#"{"accepted": [{"rule": "primitive-obsession", "file": "user.go", "line": 3, "content": "func newUser(name, email string) {}", "reason": "not worth a newtype here"}]}"#,
    );
    let run_ctx = RunContext::load(&dir).unwrap();

    let result = run_check(
        &primitive_obsession_check(),
        &dir,
        &file,
        None,
        &Registry::default(),
        &run_ctx,
    )
    .unwrap();
    assert!(!result.passed);
    assert!(
        result.output.contains("3 identifiers"),
        "output: {}",
        result.output
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn malformed_accepted_findings_file_surfaces_as_a_real_error() {
    let dir = tmp_dir("accepted-malformed");
    std::fs::write(
        dir.join("user.go"),
        "package main\n\nfunc newUser(name, email string) {}\n",
    )
    .unwrap();
    write_accepted_findings(
        &dir,
        r#"{"accepted": [{"rule": "primitive-obsession", "file": "user.go", "line": 3, "content": "func newUser(name, email string) {}", "reason": ""}]}"#,
    );

    // `AcceptedFindings` is loaded once per batch by the caller (see
    // `run_checks_for_trigger`'s doc comment), not inside `run_check`/`run_native_check`
    // — an entry with an empty reason must fail loudly right there, before any check
    // in the batch ever runs, rather than nondeterministically aborting mid-batch.
    let result = crate::accepted_findings::find_accepted_findings(&dir);
    assert!(
        result.is_err(),
        "an entry with an empty reason must not be silently accepted"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

fn ai_vocabulary_density_check(options: Option<serde_json::Value>) -> Check {
    Check {
        name: "native".to_string(),
        command: None,
        checker: Some("ai-vocabulary-density".to_string()),
        architecture_checker: None,
        severity: Severity::Advisory,
        scope: vec![],
        triggers: vec![],
        message: None,
        output_format: None,
        options,
    }
}

/// Without `options`, `ai-vocabulary-density`'s default 3-word threshold doesn't
/// fire on a single buzzword occurrence.
#[test]
fn unconfigured_checker_uses_its_own_default_threshold() {
    let dir = tmp_dir("checker-options-default");
    let file = dir.join("doc.md");
    std::fs::write(&file, "We should leverage this system.\n").unwrap();

    let result = run_check(
        &ai_vocabulary_density_check(None),
        &dir,
        &file,
        None,
        &Registry::default(),
        &RunContext::default(),
    )
    .unwrap();
    assert!(result.passed, "output: {}", result.output);

    let _ = std::fs::remove_dir_all(&dir);
}

/// End-to-end proof that `Check::options` reaches the native checker: lowering the
/// threshold to 1 via `options` makes the same file fail, through the exact
/// `run_check` -> `run_native_check` path production uses.
#[test]
fn check_options_reach_the_configured_native_checker() {
    let dir = tmp_dir("checker-options-configured");
    let file = dir.join("doc.md");
    std::fs::write(&file, "We should leverage this system.\n").unwrap();

    let check = ai_vocabulary_density_check(Some(serde_json::json!({ "threshold": 1 })));
    let result = run_check(
        &check,
        &dir,
        &file,
        None,
        &Registry::default(),
        &RunContext::default(),
    )
    .unwrap();
    assert!(!result.passed, "output: {}", result.output);
    assert!(result.output.contains("leverage"));

    let _ = std::fs::remove_dir_all(&dir);
}
