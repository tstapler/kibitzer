// Fixture-driven tests for the `test-quality` static checkers (Tasks 1.1.1a, 1.1.2a, 1.1.2c).
// Names follow the validation plan's should_X_When_Y convention.
#![allow(non_snake_case)]

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::check::{apply_test_quality_cap, run_checks_for_trigger};
use crate::check_result::CheckResult;
use crate::checker::{Checker, Finding, GrammarCache, lookup, run_checker_with_cache};
use crate::plugin::Registry;
use crate::run_context::RunContext;
use crate::test_code::{BANNED_MESSAGE_WORDS, banned_word_in};

fn fixture_path(dir: &str, file: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("testdata/test-quality")
        .join(dir)
        .join(file)
}

fn run_source(
    checker_name: &str,
    path: &Path,
    source: &str,
    options: Option<&Value>,
) -> Vec<Finding> {
    let checker = lookup(checker_name).unwrap_or_else(|| panic!("{checker_name} not registered"));
    let checker: Box<dyn Checker> = match options {
        Some(o) => checker.configure(o).unwrap().unwrap_or(checker),
        None => checker,
    };
    run_checker_with_cache(checker.as_ref(), path, source, &GrammarCache::new()).unwrap()
}

fn run_fixture(checker_name: &str, dir: &str, file: &str, options: Option<&Value>) -> Vec<Finding> {
    let path = fixture_path(dir, file);
    let source = std::fs::read_to_string(&path).unwrap();
    // Bare file name: the real path sits under `testdata/`, which the rule skips as non-test code.
    run_source(checker_name, Path::new(file), &source, options)
}

fn lines(findings: &[Finding]) -> Vec<usize> {
    findings.iter().map(|f| f.line).collect()
}

fn precision_first() -> Value {
    json!({"precision_first": true})
}

/// (fixture, expected lines in strict mode, expected lines in precision-first mode)
type Case = (&'static str, &'static [usize], &'static [usize]);

const GO_CASES: &[Case] = &[
    ("helper_assert_test.go", &[], &[]),
    ("closure_cleanup_test.go", &[], &[]),
    ("snapshot_test.go", &[], &[]),
    ("fuzz_seed_test.go", &[], &[]),
    ("example_output_test.go", &[], &[]),
    ("expected_panic_test.go", &[], &[]),
    ("mock_verify_test.go", &[], &[]),
    ("smoke_no_panic_test.go", &[], &[]),
    ("skipped_test.go", &[], &[]),
    ("generated_test.go", &[], &[]),
    ("weak_oracle_test.go", &[], &[]),
    ("tautology_pending_test.go", &[], &[]),
    ("receiver_method_helper_test.go", &[44, 54], &[44, 54]),
    ("generic_helper_test.go", &[25], &[25]),
    ("method_value_run_test.go", &[34], &[34]),
    ("subtest_dispatch_test.go", &[], &[]),
    ("subtest_dispatch_quiet_test.go", &[17], &[17]),
    ("func_alias_test.go", &[27], &[27]),
    ("go126_new_expr_test.go", &[16], &[16]),
    ("testify_suite_run_test.go", &[30], &[30]),
    ("handle_holder_collaborator_test.go", &[33], &[33]),
    ("handle_holder_generalized_test.go", &[86, 99], &[86, 99]),
    ("run_helper_returned_func_test.go", &[16, 24], &[16, 24]),
    ("run_generic_value_test.go", &[70, 75, 80], &[70, 75, 80]),
    ("cross_file_helper_test.go", &[7], &[]),
    (
        "k8s_stubs_test.go",
        &[7, 11, 14, 18, 22],
        &[7, 11, 14, 18, 22],
    ),
    ("empty_test.go", &[6], &[6]),
    ("helper_process_test.go", &[27, 34], &[27, 34]),
    ("scanner_handler_test.go", &[5], &[5]),
    ("a_test.go", &[6], &[6]),
    ("unresolved_helper_test.go", &[7], &[]),
    ("helper_depth_test.go", &[18, 25], &[18, 25]),
    (
        "callback_assertion_test.go",
        &[53, 55, 58, 60, 67, 99, 102],
        &[53, 55, 58, 60, 67, 99, 102],
    ),
    (
        "func_value_callback_test.go",
        &[40, 42, 44, 49, 51],
        &[40, 42, 44, 49, 51],
    ),
    ("guarded_panic_helper_test.go", &[29, 44, 49], &[29, 44, 49]),
    ("env_entrypoint_test.go", &[37, 43, 50], &[37, 43, 50]),
    ("t_helper_in_export_test.go", &[16], &[16]),
    (
        "main_harness_test.go",
        &[36, 40, 46, 65, 72],
        &[36, 40, 46, 65, 72],
    ),
    ("params_struct_handle_test.go", &[25, 29, 35], &[25, 29, 35]),
    ("mock_panic_expectation_test.go", &[20, 26], &[26]),
    (
        "six_vacuous_test.go",
        &[6, 7, 8, 9, 10, 11],
        &[6, 7, 8, 9, 10, 11],
    ),
];

const PYTHON_CASES: &[Case] = &[
    ("empty_test.py", &[1], &[1]),
    ("test_empty_body.py", &[4, 8], &[4, 8]),
    ("test_helper_assert.py", &[], &[]),
    ("test_expected_exception.py", &[], &[]),
    ("test_mock_verify.py", &[], &[]),
    ("test_snapshot_skip_smoke.py", &[], &[]),
    ("test_generated.py", &[], &[]),
    ("weak_oracle_test.py", &[], &[]),
    ("tautology_pending_test.py", &[], &[]),
    ("test_unresolved_helper.py", &[6, 10], &[10]),
    ("test_helper_depth.py", &[32, 40], &[32, 40]),
    ("test_unittest_class.py", &[6], &[6]),
    ("test_pytester_linematcher.py", &[21], &[21]),
    ("test_raises_alias.py", &[23], &[23]),
    ("test_same_class_helpers.py", &[25], &[25]),
    ("test_cross_file_base.py", &[7], &[7]),
    ("test_validate_reraise.py", &[11, 18], &[11, 18]),
    (
        "test_six_vacuous.py",
        &[4, 5, 6, 7, 8, 9],
        &[4, 5, 6, 7, 8, 9],
    ),
];

const KOTLIN_CASES: &[Case] = &[
    ("OraclesTest.kt", &[], &[]),
    ("SkipExemptTest.kt", &[], &[]),
    ("WeakOracleTest.kt", &[], &[]),
    ("EmptyTest.kt", &[7, 12, 17], &[7, 12, 17]),
    ("ComposeTest.kt", &[31], &[31]),
    ("HelperTest.kt", &[29, 34], &[34]),
    ("ScreenshotTest.kt", &[], &[]),
    ("CaptureNonGoldenTest.kt", &[8], &[8]),
    (
        "SixVacuousTest.kt",
        &[7, 8, 9, 10, 11, 12],
        &[7, 8, 9, 10, 11, 12],
    ),
];

fn assert_cases(checker: &str, dir: &str, cases: &[Case]) {
    let precision = precision_first();
    for (file, strict, precise) in cases {
        let got = lines(&run_fixture(checker, dir, file, None));
        assert_eq!(&got, strict, "{checker} strict on {file}");
        let got = lines(&run_fixture(checker, dir, file, Some(&precision)));
        assert_eq!(&got, precise, "{checker} precision-first on {file}");
    }
}

#[test]
fn test_no_assertion_should_MatchLabelsOnEveryGoFixture_When_StrictAndPrecisionFirst() {
    assert_cases("test-no-assertion-go", "go", GO_CASES);
}

#[test]
fn test_no_assertion_should_MatchLabelsOnEveryPythonFixture_When_StrictAndPrecisionFirst() {
    assert_cases("test-no-assertion-python", "python", PYTHON_CASES);
}

#[test]
fn test_no_assertion_should_MatchLabelsOnEveryKotlinFixture_When_StrictAndPrecisionFirst() {
    assert_cases("test-no-assertion-kotlin", "kotlin", KOTLIN_CASES);
}

#[test]
fn test_no_assertion_should_ReportZero_When_CrossFileHelperListedInAssertionHelpers() {
    let options = json!({"assertion_helpers": ["assertFailures"]});
    let got = run_fixture(
        "test-no-assertion-go",
        "go",
        "cross_file_helper_test.go",
        Some(&options),
    );
    assert!(got.is_empty(), "{got:?}");
}

#[test]
fn test_no_assertion_should_ReportZero_When_CrossFileBaseHelperListedInAssertionHelpers() {
    let options = json!({"assertion_helpers": ["self.run_wrapper_test"]});
    let got = run_fixture(
        "test-no-assertion-python",
        "python",
        "test_cross_file_base.py",
        Some(&options),
    );
    assert!(got.is_empty(), "{got:?}");
}

#[test]
fn test_no_assertion_should_SkipNonTestDirs_When_PathHasTestdataVendorOrThirdParty() {
    for (dir, file, checker, in_scope, skipped) in [
        (
            "go",
            "scanner_handler_test.go",
            "test-no-assertion-go",
            "tools/scanner/backend/handler_test.go",
            "tools/scanner/backend/testdata/handler_test.go",
        ),
        (
            "python",
            "test_cross_file_base.py",
            "test-no-assertion-python",
            "tests/test_a.py",
            "third_party/pytest/testing/test_a.py",
        ),
        (
            "kotlin",
            "EmptyTest.kt",
            "test-no-assertion-kotlin",
            "app/FooTest.kt",
            "vendor/lib/FooTest.kt",
        ),
    ] {
        let source = std::fs::read_to_string(fixture_path(dir, file)).unwrap();
        let got = run_source(checker, Path::new(in_scope), &source, None);
        assert!(!got.is_empty(), "{checker} must flag {in_scope}");
        for d in ["testdata", "vendor", "node_modules", "third_party"] {
            let path = format!("x/{d}/y/{}", in_scope.rsplit('/').next().unwrap());
            assert!(
                run_source(checker, Path::new(&path), &source, None).is_empty(),
                "{checker} must skip {path}"
            );
        }
        assert!(run_source(checker, Path::new(skipped), &source, None).is_empty());
    }
}

#[test]
fn test_no_assertion_should_SkipGo_When_DirComponentStartsWithUnderscore() {
    // Go tooling ignores `_*` directories (teleport render-tests `_sample`, delve `_fixtures`).
    let source = std::fs::read_to_string(fixture_path("go", "scanner_handler_test.go")).unwrap();
    for path in [
        "tooling/render-tests/_sample/package1/foo_test.go",
        "_fixtures/buildtest/main_test.go",
    ] {
        let got = run_source("test-no-assertion-go", Path::new(path), &source, None);
        assert!(got.is_empty(), "{path}: {got:?}");
    }
    // Controls: a mid-name underscore, and a `__tests__`-style dir for other languages, still count.
    let in_scope = "tooling/sample_dir/package1/foo_test.go";
    let got = run_source("test-no-assertion-go", Path::new(in_scope), &source, None);
    assert_eq!(lines(&got), [5], "{in_scope}");
    let py = std::fs::read_to_string(fixture_path("python", "test_cross_file_base.py")).unwrap();
    let got = run_source(
        "test-no-assertion-python",
        Path::new("pkg/_internal/tests/test_a.py"),
        &py,
        None,
    );
    assert!(!got.is_empty(), "underscore dirs are only skipped for Go");
}

#[test]
fn test_no_assertion_should_StillFlag_When_DirNameOnlyContainsNonTestWord() {
    let source = std::fs::read_to_string(fixture_path("go", "scanner_handler_test.go")).unwrap();
    for path in [
        "testdata_utils/h_test.go",
        "fixtures/h_test.go",
        "example_scripts/h_test.go",
    ] {
        let got = run_source("test-no-assertion-go", Path::new(path), &source, None);
        assert_eq!(lines(&got), [5], "{path}");
    }
}

#[test]
fn test_no_assertion_should_ReportZero_When_HelperAssertsViaTFatal() {
    let got = run_fixture("test-no-assertion-go", "go", "helper_assert_test.go", None);
    assert!(got.is_empty(), "{got:?}");
}

#[test]
fn test_no_assertion_should_ReportOne_When_PythonBodyOnlyCallsParse() {
    let got = run_fixture("test-no-assertion-python", "python", "empty_test.py", None);
    assert_eq!(lines(&got), [1]);
    assert!(got[0].message.starts_with("[test-no-assertion] test_x"));
}

#[test]
fn test_no_assertion_should_ReportOneAtFuncLine_When_GoBodyOnlyCallsParse() {
    let got = run_fixture("test-no-assertion-go", "go", "a_test.go", None);
    assert_eq!(lines(&got), [6]);
    assert!(got[0].message.starts_with("[test-no-assertion] TestA"));
    assert!(
        got[0]
            .message
            .contains("no assertion call found in body or same-file helpers")
    );
}

#[test]
fn test_no_assertion_should_ReportZero_When_KnownFalseNegativeWeakOracle() {
    for (dir, file, checker) in [
        ("go", "weak_oracle_test.go", "test-no-assertion-go"),
        ("python", "weak_oracle_test.py", "test-no-assertion-python"),
        ("kotlin", "WeakOracleTest.kt", "test-no-assertion-kotlin"),
    ] {
        let path = fixture_path(dir, file);
        let source = std::fs::read_to_string(&path).unwrap();
        assert!(
            source.contains("known false negative: weak oracle"),
            "{file} header must label the gap"
        );
        assert!(run_source(checker, &path, &source, None).is_empty());
    }
}

#[test]
fn test_no_assertion_should_ReportZero_When_FileNameIsNotATestFile() {
    let source = std::fs::read_to_string(fixture_path("go", "a_test.go")).unwrap();
    assert!(run_source("test-no-assertion-go", Path::new("a.go"), &source, None).is_empty());
    let source = std::fs::read_to_string(fixture_path("python", "empty_test.py")).unwrap();
    assert!(
        run_source(
            "test-no-assertion-python",
            Path::new("helpers.py"),
            &source,
            None
        )
        .is_empty()
    );
    let source = std::fs::read_to_string(fixture_path("kotlin", "EmptyTest.kt")).unwrap();
    assert!(
        run_source(
            "test-no-assertion-kotlin",
            Path::new("src/main/Helpers.kt"),
            &source,
            None
        )
        .is_empty()
    );
}

#[test]
fn test_no_assertion_should_AcceptConfiguredHelper_When_AssertionHelpersOptionSet() {
    let source = "package p\n\nimport \"testing\"\n\nfunc TestA(t *testing.T) {\n\tmustWork()\n}\n";
    let path = Path::new("a_test.go");
    assert_eq!(
        lines(&run_source("test-no-assertion-go", path, source, None)),
        [5]
    );
    let opts = json!({"assertion_helpers": ["mustWork"]});
    assert!(run_source("test-no-assertion-go", path, source, Some(&opts)).is_empty());
}

#[test]
fn test_no_assertion_should_AcceptConfiguredHelper_When_KotlinAssertionHelpersOptionSet() {
    let source = "class FooTest {\n    @Test\n    fun a() {\n        mustWork()\n    }\n}\n";
    let path = Path::new("FooTest.kt");
    assert_eq!(
        lines(&run_source("test-no-assertion-kotlin", path, source, None)),
        [2]
    );
    let opts = json!({"assertion_helpers": ["mustWork"]});
    assert!(run_source("test-no-assertion-kotlin", path, source, Some(&opts)).is_empty());
}

#[test]
fn test_no_assertion_should_SuppressKotlinFinding_When_IgnoreDirectiveAboveAnnotation() {
    let source = "class FooTest {\n    // kibitzer:ignore test-no-assertion -- smoke\n    @Test\n    fun a() {\n        run()\n    }\n}\n";
    let findings = run_source(
        "test-no-assertion-kotlin",
        Path::new("FooTest.kt"),
        source,
        None,
    );
    assert_eq!(
        lines(&findings),
        [3],
        "the finding line is the annotation line"
    );
}

#[test]
fn test_no_assertion_should_ExtendAssertionIshWords_When_OptionSet() {
    let source = "package p\n\nimport \"testing\"\n\nfunc TestA(t *testing.T) {\n\tlint.Ensure(Parse(\"\"))\n}\n";
    let path = Path::new("a_test.go");
    let opts = json!({"precision_first": true});
    assert_eq!(
        lines(&run_source(
            "test-no-assertion-go",
            path,
            source,
            Some(&opts)
        )),
        [5]
    );
    let opts = json!({"precision_first": true, "assertion_ish": ["ensure"]});
    assert!(run_source("test-no-assertion-go", path, source, Some(&opts)).is_empty());
}

#[test]
fn s6_config_should_FailNamingKeyAndValidKeys_When_UnknownOptionKey() {
    for name in [
        "test-no-assertion-go",
        "test-no-assertion-python",
        "test-no-assertion-kotlin",
    ] {
        let checker = lookup(name).unwrap();
        let err = checker
            .configure(&json!({"assertion_helper": ["x"]}))
            .err()
            .expect("typo must be an error");
        let text = format!("{err:#}");
        assert!(text.contains("assertion_helper"), "{text}");
        assert!(text.contains("assertion_helpers"), "{text}");
        assert!(text.contains("precision_first"), "{text}");
    }
}

#[test]
fn s6_config_should_Fail_When_OptionHasWrongType() {
    let checker = lookup("test-no-assertion-go").unwrap();
    assert!(
        checker
            .configure(&json!({"precision_first": "yes"}))
            .is_err()
    );
    assert!(
        checker
            .configure(&json!({"assertion_helpers": "x"}))
            .is_err()
    );
    assert!(checker.configure(&json!("not an object")).is_err());
}

#[test]
fn registry_should_ListOneInstancePerLanguage_When_FromOneFactory() {
    let names: Vec<String> = crate::checker::registry()
        .iter()
        .map(|c| c.name().to_string())
        .filter(|n| n.starts_with("test-no-assertion"))
        .collect();
    assert_eq!(
        names,
        [
            "test-no-assertion-go",
            "test-no-assertion-python",
            "test-no-assertion-kotlin"
        ]
    );
}

// ---------- message shape (Task 1.1.2c: UX-H1, H2, H6, H8, H10, A1) ----------

fn go_message() -> String {
    run_fixture("test-no-assertion-go", "go", "a_test.go", None)
        .remove(0)
        .message
}

fn kotlin_message() -> String {
    run_fixture("test-no-assertion-kotlin", "kotlin", "EmptyTest.kt", None)
        .remove(0)
        .message
}

fn python_message() -> String {
    run_fixture("test-no-assertion-python", "python", "empty_test.py", None)
        .remove(0)
        .message
}

#[test]
fn ux_h1_finding_should_HaveHeadlineFixNonClaimIgnore_When_AgentAddsEmptyTest() {
    for message in [go_message(), python_message(), kotlin_message()] {
        let rendered = format!("t_test:1: {message}");
        let rows: Vec<&str> = rendered.lines().collect();
        assert_eq!(rows.len(), 4, "{rendered}");
        assert!(rows[0].contains("[test-no-assertion]"));
        assert!(rows[1].starts_with("  Likely fix: "), "{}", rows[1]);
        assert!(rows[2].starts_with("  Not claimed: "), "{}", rows[2]);
        assert!(rows[3].starts_with("  Intentional? add `"), "{}", rows[3]);
    }
}

#[test]
fn ux_h2_message_should_ContainNoBannedWords_When_AllRulesEmit() {
    for message in [go_message(), python_message(), kotlin_message()] {
        assert_eq!(banned_word_in(&message), None, "{message}");
        let first = message.split_whitespace().next().unwrap();
        assert!(!first.eq_ignore_ascii_case("delete") && !first.eq_ignore_ascii_case("remove"));
        assert!(!message.starts_with("Delete"));
    }
    assert!(banned_word_in("This is critical").is_some());
    assert!(banned_word_in("you must fix").is_some());
    assert!(banned_word_in("Delete this test").is_some());
    assert!(BANNED_MESSAGE_WORDS.contains(&"urgent"));
}

#[test]
fn ux_a1_message_should_ListStrengthenFirstAndRequireNamingCoveringTest_When_NoAssertion() {
    for message in [go_message(), python_message(), kotlin_message()] {
        let fix = message.lines().nth(1).unwrap();
        assert!(
            fix.starts_with("  Likely fix: assert on")
                || fix.starts_with("  Likely fix: strengthen"),
            "{fix}"
        );
        assert!(fix.contains("name it in your reply"), "{fix}");
        assert!(fix.find("assert on").unwrap() < fix.find("Remove only if").unwrap());
    }
}

#[test]
fn ux_h8_finding_should_HaveQuietPathLineInRecordedVariant_When_AnyRuleInHookOrRunChecks() {
    let go = go_message();
    let go_last = go.lines().last().unwrap();
    assert_eq!(
        go_last,
        "  Intentional? add `// kibitzer:ignore test-no-assertion -- <why>` above the func. Finding wrong? report_false_positive."
    );
    let py = python_message();
    let py_last = py.lines().last().unwrap();
    assert_eq!(
        py_last,
        "  Intentional? add `# kibitzer:ignore test-no-assertion -- <why>` directly above the def (below any decorators). Finding wrong? report_false_positive."
    );
}

#[test]
fn ux_h8_finding_should_NameFunPlacement_When_Kotlin() {
    let last = kotlin_message().lines().last().unwrap().to_string();
    assert_eq!(
        last,
        "  Intentional? add `// kibitzer:ignore test-no-assertion -- <why>` above the fun (and its annotations). Finding wrong? report_false_positive."
    );
}

#[test]
fn ux_h10_finding_should_NameBothQuietPaths_When_AnyHookOrRunChecksFinding() {
    for message in [go_message(), python_message(), kotlin_message()] {
        assert!(message.contains("kibitzer:ignore"));
        assert!(message.contains("report_false_positive"));
        assert!(message.lines().any(|l| l.starts_with("  Not claimed: ")));
        assert!(message.lines().count() <= 4);
    }
}

// ---------- pipeline behavior in hook mode (UX-H3, H6, H7, H9) ----------

fn temp_repo(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kibitzer-tq-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn hook_run(root: &Path, file: &Path, ranges: Option<&[(usize, usize)]>) -> Vec<CheckResult> {
    let checks = crate::config::test_quality_checks();
    run_checks_for_trigger(
        &checks,
        "PostToolUse",
        root,
        file,
        ranges,
        &Registry::default(),
        &RunContext::default(),
    )
    .unwrap()
}

fn output_of(results: &[CheckResult]) -> String {
    results
        .iter()
        .map(|r| r.output.as_str())
        .filter(|o| !o.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn count_findings(output: &str) -> usize {
    output.matches("[test-no-assertion]").count()
}

#[test]
fn ux_h3_hook_should_BeSilent_When_DiffDoesNotTouchVacuousTest() {
    let root = temp_repo("h3");
    let file = root.join("a_test.go");
    std::fs::copy(fixture_path("go", "a_test.go"), &file).unwrap();
    let outside = hook_run(&root, &file, Some(&[(9, 14)]));
    assert_eq!(output_of(&outside), "");
    let inside = hook_run(&root, &file, Some(&[(6, 8)]));
    assert_eq!(count_findings(&output_of(&inside)), 1);
    let body_only = hook_run(&root, &file, Some(&[(7, 7)]));
    assert_eq!(
        output_of(&body_only),
        "",
        "an edit inside the body without touching the first line is covered only by Story 4.1.2"
    );
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn ux_h9_hook_should_ListSixFindingsAndNoMoreLine_When_SixVacuousTestsAndNoCapRecorded() {
    assert_eq!(crate::config::test_quality_hook_cap(), None);
    let root = temp_repo("h9");
    for (name, src) in [
        ("six_vacuous_test.go", "six_vacuous_test.go"),
        ("test_six_vacuous.py", "test_six_vacuous.py"),
        ("SixVacuousTest.kt", "SixVacuousTest.kt"),
    ] {
        let dir = match name.rsplit('.').next() {
            Some("go") => "go",
            Some("kt") => "kotlin",
            _ => "python",
        };
        let file = root.join(name);
        std::fs::copy(fixture_path(dir, src), &file).unwrap();
        let out = output_of(&hook_run(&root, &file, None));
        assert_eq!(count_findings(&out), 6, "{out}");
        assert!(!out.contains("more test-quality findings"), "{out}");
    }
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn ux_h7_extra_findings_should_CollapseToOneMoreLine_When_NPlus3() {
    let root = temp_repo("h7");
    let file = root.join("six_vacuous_test.go");
    std::fs::copy(fixture_path("go", "six_vacuous_test.go"), &file).unwrap();
    let mut results = hook_run(&root, &file, None);
    apply_test_quality_cap(&mut results, "PostToolUse", Some(3));
    let out = output_of(&results);
    assert_eq!(count_findings(&out), 3, "{out}");
    assert_eq!(
        out.lines()
            .filter(|l| l.contains(
                "+3 more test-quality findings in this file (run kibitzer run <file> for all)"
            ))
            .count(),
        1,
        "{out}"
    );
    let mut batch = hook_run(&root, &file, None);
    apply_test_quality_cap(&mut batch, "batch", Some(3));
    assert_eq!(
        count_findings(&output_of(&batch)),
        6,
        "kibitzer run passes no cap"
    );
    let mut unset = hook_run(&root, &file, None);
    apply_test_quality_cap(&mut unset, "PostToolUse", None);
    assert_eq!(
        count_findings(&output_of(&unset)),
        6,
        "unset cap never collapses"
    );
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn ux_h6_copied_ignore_line_should_Suppress_When_WhyReplaced() {
    let root = temp_repo("h6");
    let go = root.join("a_test.go");
    let directive = go_message()
        .lines()
        .last()
        .unwrap()
        .split('`')
        .nth(1)
        .unwrap()
        .replace("<why>", "smoke test, covered by TestOk");
    let source = std::fs::read_to_string(fixture_path("go", "a_test.go")).unwrap();
    let source = source.replacen("func TestA", &format!("{directive}\nfunc TestA"), 1);
    std::fs::write(&go, source).unwrap();
    assert_eq!(count_findings(&output_of(&hook_run(&root, &go, None))), 0);

    let py = root.join("empty_test.py");
    let directive = python_message()
        .lines()
        .last()
        .unwrap()
        .split('`')
        .nth(1)
        .unwrap()
        .replace("<why>", "only checks that parse does not raise");
    std::fs::write(
        &py,
        format!("{directive}\ndef test_x():\n    parse(\"\")\n"),
    )
    .unwrap();
    assert_eq!(count_findings(&output_of(&hook_run(&root, &py, None))), 0);
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn ux_h6_directive_should_Suppress_When_PlacedAboveDecoratorOfPythonTest() {
    let root = temp_repo("h6dec");
    let py = root.join("deco_test.py");
    let body = "@pytest.mark.slow\ndef test_x():\n    parse(\"\")\n";
    let directive = "# kibitzer:ignore test-no-assertion -- only checks parse does not raise\n";
    std::fs::write(&py, format!("{directive}{body}")).unwrap();
    let above_decorator = count_findings(&output_of(&hook_run(&root, &py, None)));
    std::fs::write(
        &py,
        format!("@pytest.mark.slow\n{directive}def test_x():\n    parse(\"\")\n"),
    )
    .unwrap();
    let above_def = count_findings(&output_of(&hook_run(&root, &py, None)));
    std::fs::remove_dir_all(&root).ok();
    assert_eq!(above_def, 0, "directive directly above def suppresses");
    assert_eq!(
        above_decorator, 1,
        "a directive above the decorators does not reach the def line, so the message says directly above the def"
    );
}

#[test]
fn ux_h4_hook_should_PrintNothing_When_NoIssues() {
    let root = temp_repo("h4");
    let file = root.join("helper_assert_test.go");
    std::fs::copy(fixture_path("go", "helper_assert_test.go"), &file).unwrap();
    let out = output_of(&hook_run(&root, &file, None));
    assert_eq!(out, "");
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn ux_h5_hook_should_PrintNothing_When_CsFile() {
    let root = temp_repo("h5");
    let file = root.join("FooTests.cs");
    std::fs::write(
        &file,
        "public class FooTests { [Fact] public void T() { Parse(\"\"); } }\n",
    )
    .unwrap();
    assert_eq!(output_of(&hook_run(&root, &file, None)), "");
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn ux_a2_hook_should_NeverEmitDynamicRule_When_AnyInput() {
    let root = temp_repo("a2");
    let file = root.join("six_vacuous_test.go");
    std::fs::copy(fixture_path("go", "six_vacuous_test.go"), &file).unwrap();
    let out = output_of(&hook_run(&root, &file, None));
    for dynamic in [
        "test-zero-unique-coverage",
        "test-zero-unique-kills",
        "test-pseudo-tested-region",
    ] {
        assert!(!out.contains(dynamic));
    }
    std::fs::remove_dir_all(&root).ok();
}
