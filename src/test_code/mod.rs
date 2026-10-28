//! `TestCodeProfile` seam: one place that knows what a test file, test function and assertion are per language
//! (Task 0.2.1a).
// Consumed by the test-quality checkers and the report (later tasks); reachable only from tests until then.
#![allow(dead_code)]

mod go;
mod java;
pub mod oracle_tag;
mod python;
mod rust;
mod ts;

use std::collections::BTreeSet;
use std::path::Path;

use tree_sitter::Node;

use crate::check_result::CheckResult;
use crate::checker::{CheckContext, Language};

pub use crate::checkers::file_size::is_generated;

/// Heuristic tag for a test; integration and snapshot findings are downgraded (pitfalls P8, P10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TestKind {
    Unit,
    Integration,
    Snapshot,
    Fuzz,
    Example,
}

/// One call (or assertion macro / statement) in a test body: the callee as written, with a
/// chained receiver reduced to its root (`expect(x).toBe(1)` is callee `expect`), and the
/// number of arguments of the call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssertionCall {
    pub callee: String,
    pub args: usize,
}

/// One test function. `line` is 1-based and is where a `kibitzer:ignore` directive goes above.
/// `body` is the function's full source text (annotations and skip markers included).
/// `calls` is every call in the body; [`TestCodeProfile::assertion_calls`] filters it by vocabulary,
/// which is how `assertion_calls` answers without re-walking or re-parsing a tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestFn {
    pub name: String,
    pub line: usize,
    pub body: String,
    pub kind: TestKind,
    pub calls: Vec<AssertionCall>,
}

/// Callee names and macros counted as assertions. A trailing `*` in an entry is a prefix match
/// (`t.Error*` matches `t.Errorf`). Resolved once: per-language base names from the profile,
/// merged with `assertion_helpers` from the repo config.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AssertionVocabulary {
    exact: BTreeSet<String>,
    prefixes: BTreeSet<String>,
}

/// Option key shared by every test-quality check.
pub const ASSERTION_HELPERS_KEY: &str = "assertion_helpers";
const TEST_QUALITY_CHECK_PREFIXES: &[&str] =
    &["test-no-assertion", "test-tautology", "test-mock-only"];

impl AssertionVocabulary {
    pub fn from_names(names: &[&str]) -> Self {
        let mut vocab = Self::default();
        for name in names {
            vocab.insert(name);
        }
        vocab
    }

    fn insert(&mut self, name: &str) {
        match name.strip_suffix('*') {
            Some(prefix) => self.prefixes.insert(prefix.to_string()),
            None => self.exact.insert(name.to_string()),
        };
    }

    pub fn contains(&self, callee: &str) -> bool {
        self.exact.contains(callee) || self.prefixes.iter().any(|p| callee.starts_with(p.as_str()))
    }

    pub fn merged(mut self, other: &AssertionVocabulary) -> Self {
        self.exact.extend(other.exact.iter().cloned());
        self.prefixes.extend(other.prefixes.iter().cloned());
        self
    }

    /// Helpers configured under `assertion_helpers` on any check named `test-no-assertion*`,
    /// `test-tautology*` or `test-mock-only*` in the repo's `.kibitzer/inspect.json`, merged.
    /// Read as raw JSON, not through `config::find_config`, which would reject a check whose
    /// `checker` is not registered. Missing or unreadable config yields an empty vocabulary.
    pub fn from_repo_config(root: &Path) -> Self {
        let mut vocab = Self::default();
        let path = crate::config::resolve_config_path(root);
        let Ok(raw) = std::fs::read_to_string(path) else {
            return vocab;
        };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&raw) else {
            return vocab;
        };
        let checks = json.get("checks").and_then(|c| c.as_array());
        for check in checks.into_iter().flatten() {
            let name = check.get("name").and_then(|n| n.as_str()).unwrap_or("");
            if !TEST_QUALITY_CHECK_PREFIXES
                .iter()
                .any(|p| name.starts_with(p))
            {
                continue;
            }
            let helpers = check
                .get("options")
                .and_then(|o| o.get(ASSERTION_HELPERS_KEY))
                .and_then(|h| h.as_array());
            for helper in helpers.into_iter().flatten().filter_map(|h| h.as_str()) {
                vocab.insert(helper);
            }
        }
        vocab
    }
}

/// Per-language bundle of test-file predicate, function discovery and base vocabulary.
/// Function discovery reads the tree already in the [`CheckContext`] and never parses.
#[derive(Clone, Copy)]
pub struct TestCodeProfile {
    pub language: Language,
    pub assertion_names: &'static [&'static str],
    pub snapshot_markers: &'static [&'static str],
    /// Substrings of a test body that mark it skipped or intentionally assertion-free.
    pub exemption_markers: &'static [&'static str],
    /// `None` for Rust, where `#[cfg(test)]` is inline and there is no test-file notion.
    is_test_file: Option<fn(&Path) -> bool>,
    collect: fn(&CheckContext) -> Vec<TestFn>,
}

/// Shell scripts have no test-function convention, so this profile discovers nothing.
fn bash_profile() -> TestCodeProfile {
    TestCodeProfile {
        language: Language::Bash,
        assertion_names: &[],
        snapshot_markers: &[],
        exemption_markers: &[],
        is_test_file: None,
        collect: |_| Vec::new(),
    }
}

impl TestCodeProfile {
    pub fn for_language(language: Language) -> TestCodeProfile {
        match language {
            Language::Go => go::profile(),
            Language::Python => python::profile(),
            Language::TypeScript | Language::Tsx | Language::JavaScript => ts::profile(language),
            Language::Java | Language::Kotlin => java::profile(language),
            Language::Rust => rust::profile(),
            Language::Bash => bash_profile(),
        }
    }

    /// `None` for a path whose extension has no profile (for example `.cs`).
    pub fn for_path(path: &Path) -> Option<TestCodeProfile> {
        let ext = path.extension()?.to_str()?;
        Language::from_extension(ext).map(Self::for_language)
    }

    pub fn is_test_file(&self, path: &Path) -> bool {
        self.is_test_file.is_some_and(|f| f(path))
    }

    /// [`is_test_file`](Self::is_test_file) minus generated files (`file_size::is_generated`).
    pub fn is_test_file_source(&self, path: &Path, source: &str) -> bool {
        self.is_test_file(path) && !is_generated(source)
    }

    /// Test functions in the already-parsed tree; empty when `ctx.tree` is `None`.
    pub fn test_functions(&self, ctx: &CheckContext) -> Vec<TestFn> {
        if ctx.tree.is_none() {
            return Vec::new();
        }
        (self.collect)(ctx)
    }

    pub fn vocabulary(&self) -> AssertionVocabulary {
        AssertionVocabulary::from_names(self.assertion_names)
    }

    /// Assertion calls of `f` under the profile's base vocabulary.
    pub fn assertion_calls(&self, f: &TestFn) -> Vec<AssertionCall> {
        self.assertion_calls_with(f, &self.vocabulary())
    }

    /// Assertion calls of `f` under `vocabulary` (usually `self.vocabulary().merged(&repo)`).
    pub fn assertion_calls_with(
        &self,
        f: &TestFn,
        vocabulary: &AssertionVocabulary,
    ) -> Vec<AssertionCall> {
        f.calls
            .iter()
            .filter(|c| vocabulary.contains(&c.callee))
            .cloned()
            .collect()
    }

    pub fn is_exempt(&self, f: &TestFn) -> bool {
        self.exemption_markers.iter().any(|m| f.body.contains(m))
    }

    /// Heuristic kind from name and body; language modules pass `Fuzz` themselves when the signature says so.
    fn classify(&self, name: &str, body: &str) -> TestKind {
        if self.snapshot_markers.iter().any(|m| body.contains(m)) {
            TestKind::Snapshot
        } else if name.to_ascii_lowercase().contains("integration") {
            TestKind::Integration
        } else {
            TestKind::Unit
        }
    }
}

pub(crate) fn node_text<'a>(node: Node, source: &'a str) -> &'a str {
    source.get(node.byte_range()).unwrap_or("")
}

/// 1-based line of `node`'s first row.
pub(crate) fn line_of(node: Node) -> usize {
    node.start_position().row + 1
}

/// Build a [`TestFn`] from the node spanning the whole function and the calls found in it.
pub(super) fn make_test_fn(
    profile: &TestCodeProfile,
    name: &str,
    line_node: Node,
    span: Node,
    source: &str,
    calls: Vec<AssertionCall>,
) -> TestFn {
    let body = node_text(span, source).to_string();
    TestFn {
        name: name.to_string(),
        line: line_of(line_node),
        kind: profile.classify(name, &body),
        body,
        calls,
    }
}

/// Number of top-level named arguments under an argument-list node.
pub(super) fn named_arg_count(args: Option<Node>) -> usize {
    args.map_or(0, |a| a.named_child_count())
}

/// `n` is the cap on test-quality findings per file in hook mode. Findings are the
/// `path:line: message` entries of the matching results' `output` (a finding may span
/// continuation lines). The first `n` findings, in result order, are kept; the rest are removed
/// and one `  +N more test-quality findings in this file (run kibitzer run <file> for all)` line is
/// appended to the last result that still has a kept finding. A result left with no finding gets
/// an empty `output` and `passed = true`. No-op when the total is at most `n`.
pub fn collapse_over_cap(results: &mut [CheckResult], n: usize) {
    let indexes: Vec<usize> = results
        .iter()
        .enumerate()
        .filter(|(_, r)| is_test_quality_result(&r.check_name))
        .map(|(i, _)| i)
        .collect();
    let total: usize = indexes
        .iter()
        .map(|&i| split_findings(&results[i].output).len())
        .sum();
    if total <= n {
        return;
    }
    let mut remaining = n;
    let mut last_kept = None;
    for &i in &indexes {
        let findings = split_findings(&results[i].output);
        let keep = findings.len().min(remaining);
        remaining -= keep;
        results[i].output = findings[..keep].join("\n");
        if keep == 0 {
            results[i].passed = true;
        } else {
            last_kept = Some(i);
        }
    }
    if let Some(i) = last_kept {
        results[i].output.push_str(&format!(
            "\n  +{} more test-quality findings in this file (run kibitzer run <file> for all)",
            total - n
        ));
    }
}

const COLLAPSIBLE_CHECK_PREFIXES: &[&str] = &[
    "test-no-assertion",
    "test-tautology",
    "test-mock-only",
    "test-assertion-removed",
];

fn is_test_quality_result(check_name: &str) -> bool {
    COLLAPSIBLE_CHECK_PREFIXES
        .iter()
        .any(|p| check_name.starts_with(p))
}

/// Group `output` lines into findings: a finding starts at a `<path>:<line>: ` line and
/// owns the lines up to the next such start.
fn split_findings(output: &str) -> Vec<String> {
    let mut findings: Vec<String> = Vec::new();
    for line in output.lines() {
        if starts_finding(line) || findings.is_empty() {
            findings.push(line.to_string());
        } else if let Some(last) = findings.last_mut() {
            last.push('\n');
            last.push_str(line);
        }
    }
    findings.retain(|f| !f.trim().is_empty());
    findings
}

fn starts_finding(line: &str) -> bool {
    let mut parts = line.splitn(3, ':');
    let (Some(path), Some(num), Some(rest)) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    !path.is_empty()
        && !path.starts_with(' ')
        && !num.is_empty()
        && num.bytes().all(|b| b.is_ascii_digit())
        && rest.starts_with(' ')
}

/// Words a hook finding never uses (UX-H2). A message also never opens with `Delete`/`Remove`.
pub const BANNED_MESSAGE_WORDS: &[&str] = &["must", "critical", "urgent"];
const BANNED_OPENERS: &[&str] = &["delete", "remove"];

/// The first banned word in `message`, if any: a [`BANNED_MESSAGE_WORDS`] word anywhere, or a
/// banned opener as the first word of the message or of its text after the `[rule]` tag.
pub fn banned_word_in(message: &str) -> Option<String> {
    let words: Vec<String> = message
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_ascii_lowercase)
        .collect();
    if let Some(w) = words
        .iter()
        .find(|w| BANNED_MESSAGE_WORDS.contains(&w.as_str()))
    {
        return Some(w.clone());
    }
    let after_tag = message.split_once("] ").map_or(message, |(_, rest)| rest);
    [message, after_tag]
        .iter()
        .filter_map(|m| m.split_whitespace().next())
        .map(str::to_ascii_lowercase)
        .find(|w| BANNED_OPENERS.contains(&w.as_str()))
}

/// The four-line hook text shared by the static test-quality rules (UX-H1, H2, H8, H10): headline,
/// likely fix, non-claim, and the V1 quiet-path line (a copyable `kibitzer:ignore` directive plus
/// `report_false_positive`). V2 would replace only the last line once G1 records a variant.
pub struct QualityMessage<'a> {
    pub rule: &'a str,
    pub headline: &'a str,
    pub likely_fix: &'a str,
    pub non_claim: &'a str,
    /// Line-comment leader of the language: `//` or `#`.
    pub comment_leader: &'a str,
    /// Where the directive goes, for example `above the func`.
    pub placement: &'a str,
}

impl QualityMessage<'_> {
    pub fn render(&self) -> String {
        format!(
            "[{rule}] {headline}\n  Likely fix: {fix}\n  Not claimed: {non_claim}\n  Intentional? add `{leader} kibitzer:ignore {rule} -- <why>` {placement}. Finding wrong? report_false_positive.",
            rule = self.rule,
            headline = self.headline,
            fix = self.likely_fix,
            non_claim = self.non_claim,
            leader = self.comment_leader,
            placement = self.placement,
        )
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::cell::Cell;

    use super::*;
    use crate::checker::GrammarCache;

    thread_local! {
        static PARSES: Cell<usize> = const { Cell::new(0) };
    }

    pub fn parse_count() -> usize {
        PARSES.with(|c| c.get())
    }

    /// Stand-in for the dispatcher: parses once through a fresh `GrammarCache` and counts it.
    pub fn parse_counted(language: Language, source: &str) -> tree_sitter::Tree {
        PARSES.with(|c| c.set(c.get() + 1));
        GrammarCache::new().parse(language, source).expect("parse")
    }

    pub fn functions(language: Language, source: &str) -> Vec<TestFn> {
        let tree = parse_counted(language, source);
        let ctx = CheckContext {
            source,
            tree: Some(&tree),
        };
        TestCodeProfile::for_language(language).test_functions(&ctx)
    }
}

#[cfg(test)]
mod tests {
    #![allow(non_snake_case)]
    use super::test_support::*;
    use super::*;
    use crate::config::Severity;

    fn go_src() -> &'static str {
        "package p\n\nfunc TestA(t *testing.T) { assert.Equal(t, 1, f()) }\n\nfunc TestB(t *testing.T) { f() }\n"
    }

    #[test]
    fn is_test_file_should_ReturnTrue_When_GoTestSuffix() {
        let p = TestCodeProfile::for_language(Language::Go);
        assert!(p.is_test_file(Path::new("src/pkg/foo_test.go")));
        assert!(!p.is_test_file(Path::new("src/pkg/foo.go")));
    }

    #[test]
    fn is_test_file_source_should_ReturnFalse_When_GeneratedHeader() {
        let p = TestCodeProfile::for_language(Language::Go);
        let path = Path::new("src/pkg/foo_test.go");
        let generated = "// Code generated by mockgen. DO NOT EDIT.\npackage p\n";
        assert!(!p.is_test_file_source(path, generated));
        assert!(p.is_test_file_source(path, "package p\n"));
    }

    #[test]
    fn test_functions_should_ReturnInlineRustTest_When_CfgTestModuleInLibRs() {
        let src =
            "pub fn f() {}\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn t() { assert!(x) }\n}\n";
        let fns = functions(Language::Rust, src);
        assert_eq!(fns.len(), 1);
        assert_eq!(fns[0].name, "t");
        assert!(
            !TestCodeProfile::for_language(Language::Rust).is_test_file(Path::new("src/lib.rs"))
        );
    }

    #[test]
    fn assertion_calls_should_ReturnCalleeAndArgs_When_TestifyAssertEqual() {
        let fns = functions(Language::Go, go_src());
        let a = fns.iter().find(|f| f.name == "TestA").unwrap();
        assert_eq!(a.line, 3);
        let calls = TestCodeProfile::for_language(Language::Go).assertion_calls(a);
        assert_eq!(
            calls,
            vec![AssertionCall {
                callee: "assert.Equal".into(),
                args: 3
            }]
        );
    }

    #[test]
    fn assertion_calls_should_ReturnEmpty_When_BodyHasNoAssertion() {
        let fns = functions(Language::Go, go_src());
        let b = fns.iter().find(|f| f.name == "TestB").unwrap();
        assert!(
            TestCodeProfile::for_language(Language::Go)
                .assertion_calls(b)
                .is_empty()
        );
    }

    #[test]
    fn profile_should_Exist_When_LanguageIsGoPythonTsJsJavaKotlinRustBash() {
        for lang in Language::ALL {
            assert_eq!(TestCodeProfile::for_language(*lang).language, *lang);
        }
        assert_eq!(Language::ALL.len(), 9);
        assert!(TestCodeProfile::for_path(Path::new("a/Foo.cs")).is_none());
        assert!(TestCodeProfile::for_path(Path::new("a/foo_test.go")).is_some());
    }

    fn repo_with_helpers(tag: &str, check_name: &str, options: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("kibitzer-test-code-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(dir.join(".kibitzer")).unwrap();
        let json = format!(
            r#"{{"checks":[{{"name":"{check_name}","checker":"x","severity":"advisory","options":{options}}}]}}"#
        );
        std::fs::write(dir.join(".kibitzer/inspect.json"), json).unwrap();
        dir
    }

    #[test]
    fn assertion_vocabulary_should_IncludeConfiguredHelper_When_OptionSet() {
        let dir = repo_with_helpers(
            "set",
            "test-no-assertion-go",
            r#"{"assertion_helpers":["requireOK"]}"#,
        );
        let vocab = AssertionVocabulary::from_repo_config(&dir);
        std::fs::remove_dir_all(&dir).ok();
        assert!(vocab.contains("requireOK"));
        let merged = TestCodeProfile::for_language(Language::Go)
            .vocabulary()
            .merged(&vocab);
        assert!(merged.contains("requireOK") && merged.contains("t.Errorf"));
    }

    #[test]
    fn assertion_vocabulary_should_MergeHelpers_When_SeveralChecksSetThem() {
        let dir = repo_with_helpers(
            "merge",
            "test-tautology-python",
            r#"{"assertion_helpers":["a"]}"#,
        );
        let json = r#"{"checks":[
            {"name":"test-tautology-python","options":{"assertion_helpers":["a"]}},
            {"name":"test-mock-only-go","options":{"assertion_helpers":["b"]}},
            {"name":"other-check","options":{"assertion_helpers":["c"]}}]}"#;
        std::fs::write(dir.join(".kibitzer/inspect.json"), json).unwrap();
        let vocab = AssertionVocabulary::from_repo_config(&dir);
        std::fs::remove_dir_all(&dir).ok();
        assert!(vocab.contains("a") && vocab.contains("b") && !vocab.contains("c"));
    }

    #[test]
    fn assertion_vocabulary_should_ExcludeHelper_When_OptionAbsent() {
        let dir = repo_with_helpers("absent", "test-no-assertion-go", "{}");
        let vocab = AssertionVocabulary::from_repo_config(&dir);
        std::fs::remove_dir_all(&dir).ok();
        assert!(!vocab.contains("requireOK"));
        assert!(
            !AssertionVocabulary::from_repo_config(Path::new("/nonexistent/kibitzer"))
                .contains("requireOK")
        );
    }

    #[test]
    fn parse_should_BeSharedAcrossThreeCheckersAndReport_When_OneFileProcessed() {
        let before = parse_count();
        let src = go_src();
        let tree = parse_counted(Language::Go, src);
        let ctx = CheckContext {
            source: src,
            tree: Some(&tree),
        };
        let profile = TestCodeProfile::for_language(Language::Go);
        for _ in 0..4 {
            assert_eq!(profile.test_functions(&ctx).len(), 2);
        }
        assert_eq!(parse_count() - before, 1);

        let none_ctx = CheckContext {
            source: src,
            tree: None,
        };
        assert!(profile.test_functions(&none_ctx).is_empty());
        assert_eq!(
            parse_count() - before,
            1,
            "test_functions must never parse on its own"
        );

        let before = parse_count();
        for _ in 0..1000 {
            let tree = parse_counted(Language::Go, src);
            let ctx = CheckContext {
                source: src,
                tree: Some(&tree),
            };
            profile.test_functions(&ctx);
        }
        assert_eq!(parse_count() - before, 1000);
    }

    fn result(name: &str, lines: &[&str]) -> CheckResult {
        CheckResult::new(name, Severity::Advisory, false, lines.join("\n"))
    }

    fn numbered(file: &str, n: usize) -> Vec<String> {
        (1..=n).map(|i| format!("{file}:{i}: finding")).collect()
    }

    #[test]
    fn collapse_over_cap_should_KeepNAndEmitOneMoreLine_When_FindingsSpanThreeRules() {
        let a = numbered("a_test.go", 2);
        let b = numbered("a_test.go", 2);
        let c = numbered("a_test.go", 1);
        let mut results = vec![
            result(
                "test-no-assertion-go",
                &a.iter().map(String::as_str).collect::<Vec<_>>(),
            ),
            result(
                "test-tautology-go",
                &b.iter().map(String::as_str).collect::<Vec<_>>(),
            ),
            result(
                "test-mock-only-go",
                &c.iter().map(String::as_str).collect::<Vec<_>>(),
            ),
        ];
        collapse_over_cap(&mut results, 3);
        let kept: usize = results
            .iter()
            .map(|r| {
                split_findings(&r.output)
                    .iter()
                    .filter(|f| starts_finding(f))
                    .count()
            })
            .sum();
        let more: Vec<&str> = results
            .iter()
            .flat_map(|r| r.output.lines())
            .filter(|l| l.contains("more test-quality findings"))
            .collect();
        assert_eq!(kept, 3);
        assert_eq!(
            more,
            vec!["  +2 more test-quality findings in this file (run kibitzer run <file> for all)"]
        );
    }

    #[test]
    fn collapse_over_cap_should_ReportThreeMore_When_NPlusThreeFindings() {
        let lines = numbered("x_test.go", 8);
        let mut results = vec![result(
            "test-no-assertion-go",
            &lines.iter().map(String::as_str).collect::<Vec<_>>(),
        )];
        collapse_over_cap(&mut results, 5);
        assert!(results[0].output.ends_with(
            "  +3 more test-quality findings in this file (run kibitzer run <file> for all)"
        ));
        assert_eq!(results[0].output.lines().count(), 6);
    }

    #[test]
    fn collapse_over_cap_should_NotChange_When_AtOrUnderCapOrOtherChecks() {
        let lines = numbered("x_test.go", 3);
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let mut results = vec![
            result("test-no-assertion-go", &refs),
            result("go-ignored-error", &refs),
        ];
        let before: Vec<String> = results.iter().map(|r| r.output.clone()).collect();
        collapse_over_cap(&mut results, 3);
        assert_eq!(
            results.iter().map(|r| r.output.clone()).collect::<Vec<_>>(),
            before
        );
        collapse_over_cap(&mut results, 1);
        assert_eq!(
            results[1].output, before[1],
            "non-test-quality results are never touched"
        );
    }

    #[test]
    fn collapse_over_cap_should_KeepContinuationLinesWithTheirFinding() {
        let mut results = vec![result(
            "test-no-assertion-go",
            &[
                "f_test.go:1: first",
                "  non-claim",
                "f_test.go:2: second",
                "  non-claim",
            ],
        )];
        collapse_over_cap(&mut results, 1);
        assert_eq!(
            results[0].output,
            "f_test.go:1: first\n  non-claim\n  +1 more test-quality findings in this file (run kibitzer run <file> for all)"
        );
    }
}
