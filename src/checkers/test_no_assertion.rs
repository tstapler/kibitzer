//! `test-no-assertion`: flags a test function whose body and same-file helpers contain no
//! assertion call (Story 1.1.2). Advisory and registry-only: reachable by name, never chained
//! into `config::default_checks()` (ADR-004).
//!
//! One [`CheckerFactory`](crate::checker::CheckerFactory) returns one instance per supported
//! language (`test-no-assertion-go`, `-python`, `-kotlin`); each reads the tree the
//! dispatcher already parsed. The rule id a finding prints and a `kibitzer:ignore` directive
//! names is the bare `test-no-assertion`.
//!
//! Options: `assertion_helpers` (extra assertion callee names, trailing `*` is a prefix),
//! `precision_first` (an unresolved call whose name looks assertion-ish counts as asserting),
//! `assertion_ish` (extra words for that look). Any other key is a load-time error.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result, bail};
use tree_sitter::Node;

use crate::checker::{CheckContext, Checker, Finding, Language};
use crate::node_kind::{GoKind, PythonKind};
use crate::test_code::{
    AssertionVocabulary, QualityMessage, TestCodeProfile, TestFn, TestKind, line_of, node_text,
};
use crate::tree_walk::walk_preorder;

const RULE: &str = "test-no-assertion";
const SUPPORTED: &[Language] = &[Language::Go, Language::Python, Language::Kotlin];
/// Same-file helper hops followed from a test body: test -> helper -> helper.
const MAX_HELPER_DEPTH: usize = 2;
const DEFAULT_ASSERTION_ISH: &[&str] = &["assert", "check", "verify", "expect", "require", "must"];
const OPT_HELPERS: &str = "assertion_helpers";
const OPT_PRECISION: &str = "precision_first";
const OPT_ISH: &str = "assertion_ish";
const VALID_OPTIONS: &[&str] = &[OPT_HELPERS, OPT_ISH, OPT_PRECISION];

const LIKELY_FIX: &str = "assert on the return value or the raised error. Remove only if another test already asserts this behaviour (name it in your reply).";
const NON_CLAIM: &str =
    "that the test is worthless; it may only check that the code does not raise.";

struct Call {
    callee: String,
    args: Vec<String>,
}

/// One function in the file with every call in its subtree (closures included) and the
/// framework-handle parameter names visible inside it.
struct FnInfo {
    name: String,
    line: usize,
    calls: Vec<Call>,
    handles: Vec<String>,
}

pub struct TestNoAssertionChecker {
    lang: Language,
    extra_helpers: AssertionVocabulary,
    precision_first: bool,
    assertion_ish: Vec<String>,
}

impl TestNoAssertionChecker {
    pub fn new(lang: Language) -> Self {
        Self {
            lang,
            extra_helpers: AssertionVocabulary::default(),
            precision_first: false,
            assertion_ish: DEFAULT_ASSERTION_ISH
                .iter()
                .map(|w| w.to_string())
                .collect(),
        }
    }

    fn name_for(lang: Language) -> &'static str {
        match lang {
            Language::Go => "test-no-assertion-go",
            Language::Python => "test-no-assertion-python",
            Language::Kotlin => "test-no-assertion-kotlin",
            _ => "test-no-assertion-unsupported",
        }
    }

    fn globs(lang: Language) -> &'static [&'static str] {
        match lang {
            Language::Python => &["**/*.py"],
            Language::Kotlin => &["**/*.kt"],
            _ => &["**/*.go"],
        }
    }

    fn with_options(&self, options: &serde_json::Value) -> Result<Self> {
        let name = Self::name_for(self.lang);
        let Some(map) = options.as_object() else {
            bail!("{name}: `options` must be an object with keys {VALID_OPTIONS:?}");
        };
        let mut next = Self::new(self.lang);
        for (key, value) in map {
            match key.as_str() {
                OPT_HELPERS => {
                    let names: Vec<&str> = string_list(name, key, value)?;
                    next.extra_helpers = AssertionVocabulary::from_names(&names);
                }
                OPT_ISH => {
                    let words = string_list(name, key, value)?;
                    next.assertion_ish
                        .extend(words.iter().map(|w| w.to_ascii_lowercase()));
                }
                OPT_PRECISION => {
                    next.precision_first = value
                        .as_bool()
                        .with_context(|| format!("{name}: option `{key}` must be true or false"))?;
                }
                other => bail!(
                    "{name}: unknown option `{other}`; valid keys: {}",
                    VALID_OPTIONS.join(", ")
                ),
            }
        }
        Ok(next)
    }

    fn message(&self, test: &TestFn) -> String {
        let (leader, placement) = match self.lang {
            Language::Python => ("#", "directly above the def (below any decorators)"),
            Language::Kotlin => ("//", "above the fun (and its annotations)"),
            _ => ("//", "above the func"),
        };
        let headline = format!(
            "{}: no assertion call found in body or same-file helpers.",
            test.name
        );
        QualityMessage {
            rule: RULE,
            headline: &headline,
            likely_fix: LIKELY_FIX,
            non_claim: NON_CLAIM,
            comment_leader: leader,
            placement,
        }
        .render()
    }

    fn asserts(&self, ctx: &Scan, calls: &[Call], handles: &[String], depth: usize) -> bool {
        calls.iter().any(|call| {
            if ctx.vocab.contains(&call.callee) || self.supplemental(ctx, call) {
                return true;
            }
            if depth < MAX_HELPER_DEPTH
                && let Some(helper) = ctx.resolve(self.lang, &call.callee)
            {
                let mut seen = ctx.seen.borrow_mut();
                if !seen.contains(&helper.line) {
                    seen.push(helper.line);
                    drop(seen);
                    let found = self.asserts(ctx, &helper.calls, &helper.handles, depth + 1);
                    ctx.seen.borrow_mut().pop();
                    return found;
                }
            }
            passes_handle(call, handles)
                || (self.precision_first && self.looks_assertion_ish(&call.callee))
        })
    }

    /// Assertion shapes the profile vocabulary cannot express with a fixed receiver name.
    fn supplemental(&self, ctx: &Scan, call: &Call) -> bool {
        let last = call.callee.rsplit('.').next().unwrap_or(&call.callee);
        match self.lang {
            Language::Python => last.starts_with("assert") || last == "AssertionError",
            Language::Kotlin => {
                is_kotlin_oracle(last) || (ctx.screenshot_file && is_screenshot_capture(last))
            }
            _ => {
                last.starts_with("Assert")
                    || last.starts_with("Fatal")
                    || last.starts_with("Fail")
                    || (last.starts_with("Error")
                        && call.callee.contains('.')
                        && !call.args.is_empty())
            }
        }
    }

    /// JUnit4 `@Test(expected = X::class)`: the framework asserts the throw.
    fn expects_exception(&self, test: &TestFn) -> bool {
        self.lang == Language::Kotlin
            && test.body.contains("@Test(")
            && test.body.contains("expected =")
    }

    fn looks_assertion_ish(&self, callee: &str) -> bool {
        let lower = callee.to_ascii_lowercase();
        self.assertion_ish
            .iter()
            .any(|w| lower.contains(w.as_str()))
    }
}

fn string_list<'a>(name: &str, key: &str, value: &'a serde_json::Value) -> Result<Vec<&'a str>> {
    let items = value
        .as_array()
        .with_context(|| format!("{name}: option `{key}` must be an array of strings"))?;
    items
        .iter()
        .map(|v| {
            v.as_str()
                .with_context(|| format!("{name}: option `{key}` must be an array of strings"))
        })
        .collect()
}

/// Passing the framework handle to an unresolved function counts as asserting.
fn passes_handle(call: &Call, handles: &[String]) -> bool {
    call.args.iter().any(|a| handles.contains(a))
}

struct Scan {
    vocab: AssertionVocabulary,
    fns: Vec<FnInfo>,
    by_name: HashMap<String, usize>,
    seen: std::cell::RefCell<Vec<usize>>,
    /// Kotlin only: the file mentions screenshots, so `capture*` calls compare against goldens.
    screenshot_file: bool,
}

impl Scan {
    fn resolve(&self, lang: Language, callee: &str) -> Option<&FnInfo> {
        let name = match (lang, callee.split_once('.')) {
            (_, None) => callee,
            (Language::Python, Some(("self" | "cls", rest))) if !rest.contains('.') => rest,
            (Language::Kotlin, Some(("this", rest))) if !rest.contains('.') => rest,
            _ => return None,
        };
        self.by_name.get(name).map(|&i| &self.fns[i])
    }
}

impl Checker for TestNoAssertionChecker {
    fn name(&self) -> &str {
        Self::name_for(self.lang)
    }

    fn description(&self) -> &str {
        "flags a test function with no assertion call in its body or same-file helpers"
    }

    fn language(&self) -> Option<Language> {
        Some(self.lang)
    }

    fn file_globs(&self) -> &[&str] {
        Self::globs(self.lang)
    }

    fn check(&self, file: &Path, ctx: &CheckContext) -> Result<Vec<Finding>> {
        let profile = TestCodeProfile::for_language(self.lang);
        if !profile.is_test_file_source(file, ctx.source) {
            return Ok(Vec::new());
        }
        let tests = profile.test_functions(ctx);
        if tests.is_empty() {
            return Ok(Vec::new());
        }
        let scan = build_scan(
            self.lang,
            ctx,
            profile.vocabulary().merged(&self.extra_helpers),
        );
        let mut findings = Vec::new();
        for test in &tests {
            let checkable = matches!(test.kind, TestKind::Unit | TestKind::Integration);
            if !checkable || profile.is_exempt(test) || self.expects_exception(test) {
                continue;
            }
            let Some(info) = scan
                .fns
                .iter()
                .find(|f| f.line == test.line && f.name == test.name)
            else {
                continue;
            };
            scan.seen.borrow_mut().clear();
            if self.asserts(&scan, &info.calls, &info.handles, 0) {
                continue;
            }
            findings.push(Finding {
                line: test.line,
                message: self.message(test),
            });
        }
        Ok(findings)
    }

    fn configure(&self, options: &serde_json::Value) -> Result<Option<Box<dyn Checker>>> {
        Ok(Some(Box::new(self.with_options(options)?)))
    }
}

fn build_scan(lang: Language, ctx: &CheckContext, vocab: AssertionVocabulary) -> Scan {
    let mut fns = Vec::new();
    if let Some(tree) = ctx.tree {
        walk_preorder(tree.root_node(), &mut |node| {
            if let Some(name) = function_name(lang, node, ctx.source) {
                fns.push(FnInfo {
                    name: name.to_string(),
                    line: line_of(node),
                    calls: calls_in(lang, node, ctx.source),
                    handles: handles_in(lang, node, ctx.source),
                });
            }
            true
        });
    }
    let mut by_name = HashMap::new();
    for (i, f) in fns.iter().enumerate() {
        by_name.entry(f.name.clone()).or_insert(i);
    }
    Scan {
        vocab,
        fns,
        by_name,
        seen: Default::default(),
        screenshot_file: lang == Language::Kotlin
            && ctx.source.to_ascii_lowercase().contains("screenshot"),
    }
}

fn function_name<'a>(lang: Language, node: Node, source: &'a str) -> Option<&'a str> {
    let is_fn = match lang {
        Language::Go => matches!(
            GoKind::of(node),
            GoKind::FunctionDeclaration | GoKind::MethodDeclaration
        ),
        Language::Kotlin => node.kind() == "function_declaration",
        _ => PythonKind::of(node) == PythonKind::FunctionDefinition,
    };
    if !is_fn {
        return None;
    }
    let name = node.child_by_field_name("name").or_else(|| {
        let mut cursor = node.walk();
        node.children(&mut cursor)
            .find(|c| matches!(c.kind(), "simple_identifier" | "identifier"))
    })?;
    Some(node_text(name, source).trim_matches('`'))
}

fn calls_in(lang: Language, root: Node, source: &str) -> Vec<Call> {
    if lang == Language::Kotlin {
        return kotlin_calls_in(root, source);
    }
    let mut calls = Vec::new();
    walk_preorder(root, &mut |node| {
        let call_node = match lang {
            Language::Go => GoKind::of(node) == GoKind::CallExpression,
            _ => PythonKind::of(node) == PythonKind::Call,
        };
        if call_node && let Some(func) = node.child_by_field_name("function") {
            let args = node.child_by_field_name("arguments");
            let mut cursor = node.walk();
            let arg_texts = args
                .iter()
                .flat_map(|a| a.named_children(&mut cursor).collect::<Vec<_>>())
                .map(|a| node_text(a, source).to_string())
                .collect();
            calls.push(Call {
                callee: node_text(func, source).to_string(),
                args: arg_texts,
            });
        } else if lang == Language::Python && PythonKind::of(node) == PythonKind::AssertStatement {
            calls.push(Call {
                callee: "assert".to_string(),
                args: Vec::new(),
            });
        }
        true
    });
    calls
}

/// Kotlin oracle names the shared Java/Kotlin vocabulary leaves out: kotlin.test and Compose
/// `assert*`, MockK/Mockito `verify*`, Turbine `await*`/`expect*`, Kotest `should*`, stdlib
/// preconditions that throw.
fn is_kotlin_oracle(last: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "assert",
        "verify",
        "coVerify",
        "confirmVerified",
        "expect",
        "should",
        "await",
        "fail",
    ];
    const EXACT: &[&str] = &["check", "checkNotNull", "require", "requireNotNull"];
    PREFIXES.iter().any(|p| last.starts_with(p)) || EXACT.contains(&last)
}

/// Golden-image capture calls (Roborazzi `captureRoboImage`, or a project wrapper such as
/// `captureMultiTheme`) fail the test when the image differs, so they are the oracle.
fn is_screenshot_capture(last: &str) -> bool {
    last.starts_with("capture") || last == "verifyRoboImage" || last == "compareRoboImage"
}

/// Every call in a Kotlin function, a chain contributing one entry per link:
/// `onNodeWithText("x").assertIsDisplayed()` yields `onNodeWithText` and `assertIsDisplayed`.
/// A member call on a call result is reported by member name alone; otherwise the receiver
/// text is kept (`Assert.assertTrue`). Infix `a shouldBe b` is reported as a call to `shouldBe`.
fn kotlin_calls_in(root: Node, source: &str) -> Vec<Call> {
    let mut calls = Vec::new();
    walk_preorder(root, &mut |node| {
        match node.kind() {
            "call_expression" => {
                if let Some(callee) = node.named_child(0) {
                    let mut cursor = node.walk();
                    let mut args = Vec::new();
                    for child in node.named_children(&mut cursor) {
                        if child.kind() == "value_arguments" {
                            let mut inner = child.walk();
                            args.extend(
                                child
                                    .named_children(&mut inner)
                                    .map(|a| node_text(a, source).to_string()),
                            );
                        }
                    }
                    calls.push(Call {
                        callee: kotlin_callee(callee, source),
                        args,
                    });
                }
            }
            "infix_expression" => {
                if let Some(op) = node
                    .named_child(1)
                    .filter(|_| node.named_child_count() == 3)
                {
                    calls.push(Call {
                        callee: node_text(op, source).to_string(),
                        args: Vec::new(),
                    });
                }
            }
            // The grammar reads `f<T> { ... }` as `f < T > lambda`; recover `f` as the callee.
            "binary_expression" => {
                if let Some(name) = node.named_child(0).filter(|n| n.kind() == "identifier")
                    && source
                        .get(name.end_byte()..)
                        .is_some_and(|r| r.starts_with('<'))
                {
                    calls.push(Call {
                        callee: node_text(name, source).to_string(),
                        args: Vec::new(),
                    });
                }
            }
            _ => {}
        }
        true
    });
    calls
}

fn kotlin_callee(callee: Node, source: &str) -> String {
    if callee.kind() != "navigation_expression" {
        return node_text(callee, source).to_string();
    }
    let member = callee
        .named_child((callee.named_child_count() as u32).saturating_sub(1))
        .map_or("", |m| node_text(m, source))
        .trim_start_matches('.');
    match callee.named_child(0) {
        Some(recv)
            if matches!(
                recv.kind(),
                "identifier" | "simple_identifier" | "navigation_expression"
            ) =>
        {
            format!("{}.{member}", node_text(recv, source))
        }
        _ => member.to_string(),
    }
}

/// Names of framework-handle parameters (`t *testing.T`, Python `self`) visible in `root`.
fn handles_in(lang: Language, root: Node, source: &str) -> Vec<String> {
    if lang == Language::Python {
        return vec!["self".to_string()];
    }
    if lang == Language::Kotlin {
        return Vec::new();
    }
    let mut handles = Vec::new();
    walk_preorder(root, &mut |node| {
        if GoKind::of(node) == GoKind::ParameterDeclaration
            && node
                .child_by_field_name("type")
                .is_some_and(|t| node_text(t, source).contains("testing."))
        {
            let mut cursor = node.walk();
            handles.extend(
                node.children_by_field_name("name", &mut cursor)
                    .map(|n| node_text(n, source).to_string()),
            );
        }
        true
    });
    handles
}

inventory::submit! {
    crate::checker::CheckerFactory(|| {
        SUPPORTED
            .iter()
            .map(|&lang| Box::new(TestNoAssertionChecker::new(lang)) as Box<dyn Checker>)
            .collect()
    })
}

#[cfg(test)]
#[path = "test_quality_tests.rs"]
mod quality_tests;
