//! `test-no-assertion`: flags a test function whose body and same-file helpers contain no
//! assertion call (Story 1.1.2). Advisory and registry-only: reachable by name, never chained
//! into `config::default_checks()` (ADR-004).
//!
//! One [`CheckerFactory`](crate::checker::CheckerFactory) returns one instance per supported
//! language (`test-no-assertion-go`, `-python`, `-kotlin`); each reads the tree the
//! dispatcher already parsed. The rule id a finding prints and a `kibitzer:ignore` directive
//! names is the bare `test-no-assertion`.
//!
//! Files under a `NON_TEST_DIRS` component (`testdata`, `vendor`, ...) are skipped for every language.
//!
//! Options: `assertion_helpers` (extra assertion callee names, trailing `*` is a prefix),
//! `precision_first` (an unresolved call whose name looks assertion-ish counts as asserting),
//! `assertion_ish` (extra words for that look). Any other key is a load-time error.

use std::collections::{HashMap, HashSet};
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
/// Python oracles recognised by method name on any receiver: `<x>.raises` / `<x>.warns` cover
/// aliases such as `py.test.raises`; the rest are pytester `LineMatcher` / `RunResult` checks
/// (`result.stdout.fnmatch_lines([...])`).
const PYTHON_ORACLE_METHODS: &[&str] = &[
    "raises",
    "warns",
    "fnmatch_lines",
    "fnmatch_lines_random",
    "re_match_lines",
    "re_match_lines_random",
    "no_fnmatch_line",
    "no_re_match_line",
    "assert_outcomes",
    "assert_contains",
];
/// Synthetic callee for a `validate(...)` call whose failure is re-raised by its `except`.
const RERAISED_VALIDATE: &str = "assert";
/// Synthetic Go callee for an `if <cond> { panic(...) }` that checks behaviour: a test without a
/// testing handle, or a helper it calls, fails by panicking on a violated condition. A bare
/// `panic` is not counted, and neither is an `if err != nil { panic(err) }` outside a `go`
/// statement: there `t.Fatal` is available, so the panic is setup-error propagation (nats
/// `TestSystemAccountWithBadRemoteLatencyUpdate` via `createAccount`), not an oracle.
const GUARDED_PANIC: &str = "guarded-panic";
/// Testing-handle methods that fail the test; passing one as a function value (`apply(t.Errorf)`)
/// makes the receiving helper an assertion site. `Log*` and `Skip*` do not fail, so are absent.
const GO_FAILING_METHODS: &[&str] = &["Error", "Errorf", "Fatal", "Fatalf", "Fail", "FailNow"];
/// Name fragments (any case) of an unresolved Go callee that is taken to verify the struct
/// literal carrying the testing handle into it (`CheckConsumeContract(Params{T: t})`).
const GO_VERIFYING_WORDS: &[&str] = &[
    "check", "verify", "assert", "require", "expect", "validate", "contract", "test",
];
/// Last-segment prefixes of the call that starts a subprocess entrypoint's server or plugin main.
const GO_ENTRYPOINT_PREFIXES: &[&str] = &["Serve", "Run", "Main", "Start", "Listen", "Sleep"];
/// Import path whose `Run(t, s)` hands the test to the suite's own methods.
const TESTIFY_SUITE_PATH: &str = "github.com/stretchr/testify/suite";
/// Marks a synthetic call: the body builds a struct literal that stores the testing handle in a
/// field (`Type.field` follows), so a same-file method of `Type` may fail the test through it.
const HOLDER_PREFIX: &str = "\0holder:";
/// Field marker of a positional struct literal element (`T{t, 1}`); resolved to a name by index.
const POSITIONAL_FIELD: &str = "#";
const OPT_HELPERS: &str = "assertion_helpers";
const OPT_PRECISION: &str = "precision_first";
const OPT_ISH: &str = "assertion_ish";
const VALID_OPTIONS: &[&str] = &[OPT_HELPERS, OPT_ISH, OPT_PRECISION];

/// Directory components whose files are never runnable tests of the project: Go tooling ignores
/// `testdata`, and `vendor` / `node_modules` / `third_party` hold someone else's code. Deliberately
/// excludes generic names like `fixtures` or `example_scripts`, which often do hold real tests.
/// Fixed list: the options plumbing is not worth a knob until a repo needs a different one.
pub const NON_TEST_DIRS: &[&str] = &["testdata", "vendor", "node_modules", "third_party"];

const LIKELY_FIX: &str = "assert on the return value or the raised error. Remove only if another test already asserts this behaviour (name it in your reply).";
const NON_CLAIM: &str =
    "that the test is worthless; it may only check that the code does not raise.";

struct Call {
    callee: String,
    args: Vec<String>,
    /// Go only: not a call but a function value handed to one (`run(testX)`, `{fn: testX}`); it
    /// asserts only through the same-file function it names, never by its name or arguments.
    fn_value: bool,
    /// Go only: an argument is a struct literal (or a variable bound to one) with a field set to
    /// the testing handle, so the callee can fail the test through it.
    carries_handle: bool,
    /// Go only: a method call on a variable bound to a struct literal carrying the testing handle.
    on_handle_holder: bool,
    /// Go only: a `t.Run(name, v.Field)` value read off a local variable. When no same-file
    /// method resolves, it is a func-typed field set elsewhere, so the body is unknowable here.
    dynamic_dispatch: bool,
}

impl Call {
    fn new(callee: String, args: Vec<String>) -> Self {
        Self {
            callee,
            args,
            fn_value: false,
            carries_handle: false,
            on_handle_holder: false,
            dynamic_dispatch: false,
        }
    }

    fn value(name: String) -> Self {
        Self {
            fn_value: true,
            ..Self::new(name, Vec::new())
        }
    }

    fn dispatched(name: String) -> Self {
        Self {
            dynamic_dispatch: true,
            ..Self::value(name)
        }
    }
}

/// One function in the file with every call in its subtree (closures included) and the
/// framework-handle parameter names visible inside it.
struct FnInfo {
    name: String,
    /// Go `func (r T) m()`; resolvable from a selector call regardless of receiver type.
    is_method: bool,
    line: usize,
    /// Byte range of the whole declaration, for the oracle tagger's one-level helper lookup.
    span: (usize, usize),
    calls: Vec<Call>,
    handles: Vec<String>,
    /// Go only: an env-gated subprocess entrypoint (`if os.Getenv(..) == "" { return }; Serve()`).
    env_entrypoint: bool,
    /// Go only: a top-level `t.Helper()` statement (not one inside a closure).
    marks_helper: bool,
    /// Go only: the body has a parse error (e.g. Go 1.26 `new(expr)`, unsupported by the grammar),
    /// so the calls seen here are incomplete and the test cannot be judged.
    parse_error: bool,
    /// Go methods only: `(receiver name, receiver type)`.
    receiver: Option<(String, String)>,
    /// Go only: parameter names in order (`""` for an unnamed one), for callback-argument following.
    params: Vec<String>,
    /// Python only: innermost enclosing class, for `self.method` resolution.
    class: Option<String>,
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

    fn asserts(
        &self,
        ctx: &Scan,
        calls: &[Call],
        handles: &[String],
        owner: Option<&str>,
        depth: usize,
    ) -> bool {
        calls.iter().any(|call| {
            if call.fn_value {
                return self
                    .follow_helper(ctx, call, handles, owner, depth)
                    .unwrap_or(
                        call.dynamic_dispatch
                            && ctx
                                .resolve(self.lang, &call.callee, handles, owner)
                                .is_none(),
                    );
            }
            if ctx.vocab.contains(&call.callee) || self.supplemental(ctx, call) {
                return true;
            }
            if self.lang == Language::Go
                && (delegates_to_handle_holder(ctx, call)
                    || runs_unseen_holder_method(ctx, call, handles)
                    || runs_testify_suite(ctx, call, handles))
            {
                return true;
            }
            if self.lang == Language::Go && callback_asserts(ctx, call, handles) {
                return true;
            }
            if let Some(found) = self.follow_helper(ctx, call, handles, owner, depth) {
                return found;
            }
            passes_handle(call, handles)
                || (call.carries_handle && names_verifier(&call.callee))
                || (self.precision_first && self.looks_assertion_ish(&call.callee))
        })
    }

    /// Whether the same-file function `call` names asserts; `None` when it cannot be resolved,
    /// the depth bound is hit, or it is already being followed (recursion).
    fn follow_helper(
        &self,
        ctx: &Scan,
        call: &Call,
        handles: &[String],
        owner: Option<&str>,
        depth: usize,
    ) -> Option<bool> {
        if depth >= MAX_HELPER_DEPTH {
            return None;
        }
        let helper = ctx.resolve(self.lang, &call.callee, handles, owner)?;
        let mut seen = ctx.seen.borrow_mut();
        if seen.contains(&helper.line) {
            return None;
        }
        seen.push(helper.line);
        drop(seen);
        let found = self.asserts(
            ctx,
            &helper.calls,
            &helper.handles,
            helper.class.as_deref(),
            depth + 1,
        );
        ctx.seen.borrow_mut().pop();
        Some(found)
    }

    /// Assertion shapes the profile vocabulary cannot express with a fixed receiver name.
    fn supplemental(&self, ctx: &Scan, call: &Call) -> bool {
        let last = call.callee.rsplit('.').next().unwrap_or(&call.callee);
        match self.lang {
            Language::Python => {
                last.starts_with("assert")
                    || last == "AssertionError"
                    || PYTHON_ORACLE_METHODS.contains(&last)
            }
            Language::Kotlin => {
                is_kotlin_oracle(last) || (ctx.screenshot_file && is_screenshot_capture(last))
            }
            _ => {
                call.callee == GUARDED_PANIC
                    || last.starts_with("Assert")
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

/// Go: `helper(t.Errorf, ...)` asserts when `helper` is not resolvable (precision-first: assume
/// it is an oracle) or resolves to a same-file function that invokes that parameter.
fn callback_asserts(ctx: &Scan, call: &Call, handles: &[String]) -> bool {
    let positions: Vec<usize> = call
        .args
        .iter()
        .enumerate()
        .filter(|(_, a)| is_failing_method_value(a, handles))
        .map(|(i, _)| i)
        .collect();
    if positions.is_empty() {
        return false;
    }
    match ctx.resolve_go(&call.callee, handles) {
        None => true,
        Some(helper) => positions.iter().any(|&i| ctx.invokes_param(helper, i, 0)),
    }
}

/// `suite.Run(t, s)` (under any import alias of testify's suite package): the assertions live in
/// the suite's methods. A `Run` on any other receiver does not qualify.
fn runs_testify_suite(ctx: &Scan, call: &Call, handles: &[String]) -> bool {
    call.callee
        .strip_suffix(".Run")
        .is_some_and(|pkg| ctx.suite_aliases.iter().any(|a| a == pkg))
        && call.args.first().is_some_and(|a| handles.contains(a))
}

/// A same-file holder type is judged by its methods; a package-qualified one (`pkg.Configuration{T: t}`)
/// is defined elsewhere, so handing it the handle is taken as delegating, as `passes_handle` does.
fn delegates_to_handle_holder(ctx: &Scan, call: &Call) -> bool {
    call.callee.strip_prefix(HOLDER_PREFIX).is_some_and(|key| {
        ctx.failing_holders.contains(key)
            || key.rsplit_once('.').is_some_and(|(ty, _)| ty.contains('.'))
    })
}

/// `ts.Run()` on a variable bound to a handle-carrying literal, where no same-file method is
/// named `Run`: the type's methods live in a sibling file of the package and cannot be judged.
fn runs_unseen_holder_method(ctx: &Scan, call: &Call, handles: &[String]) -> bool {
    call.on_handle_holder && ctx.resolve_go(&call.callee, handles).is_none()
}

fn is_failing_method_value(arg: &str, handles: &[String]) -> bool {
    arg.split_once('.').is_some_and(|(recv, method)| {
        handles.iter().any(|h| h == recv) && GO_FAILING_METHODS.contains(&method)
    })
}

/// `pkg.CheckContract`, `verifyAll`: the last segment contains a verification word.
fn names_verifier(callee: &str) -> bool {
    let last = callee
        .rsplit('.')
        .next()
        .unwrap_or(callee)
        .to_ascii_lowercase();
    GO_VERIFYING_WORDS.iter().any(|w| last.contains(w))
}

/// Passing the framework handle to an unresolved function counts as asserting.
fn passes_handle(call: &Call, handles: &[String]) -> bool {
    call.args.iter().any(|a| handles.contains(a))
}

struct Scan {
    vocab: AssertionVocabulary,
    fns: Vec<FnInfo>,
    by_name: HashMap<String, usize>,
    /// Go only: same-file methods by bare name, whatever their receiver type.
    by_method: HashMap<String, usize>,
    /// Python only: `(class, method)` -> fn index, and class -> same-file base names.
    methods: HashMap<(String, String), usize>,
    bases: HashMap<String, Vec<String>>,
    seen: std::cell::RefCell<Vec<usize>>,
    /// Go only: local names the testify `suite` package is imported under.
    suite_aliases: Vec<String>,
    /// Go only: `Type.field` pairs where a same-file method of `Type` fails the test through `field`.
    failing_holders: HashSet<String>,
    /// Kotlin only: the file mentions screenshots, so `capture*` calls compare against goldens.
    screenshot_file: bool,
}

impl Scan {
    fn resolve(
        &self,
        lang: Language,
        callee: &str,
        handles: &[String],
        owner: Option<&str>,
    ) -> Option<&FnInfo> {
        if lang == Language::Go {
            return self.resolve_go(callee, handles);
        }
        let name = match (lang, callee.split_once('.')) {
            (_, None) => callee,
            (Language::Python, Some(("self" | "cls", rest))) if !rest.contains('.') => {
                if let Some(found) = owner.and_then(|c| self.resolve_in_class_chain(c, rest)) {
                    return Some(found);
                }
                rest
            }
            (Language::Kotlin, Some(("this", rest))) if !rest.contains('.') => rest,
            _ => return None,
        };
        self.by_name.get(name).map(|&i| &self.fns[i])
    }
}

impl Scan {
    /// Source text of the same-file plain function `name` (Go oracle tagger only).
    fn helper_source(&self, name: &str, source: &str) -> Option<String> {
        let f = &self.fns[*self.by_name.get(name)?];
        source.get(f.span.0..f.span.1).map(str::to_string)
    }

    /// Go: does `f` call its `idx`-th parameter, or hand it on to a callee that does? A callee
    /// that cannot be resolved is assumed to call it. Depth-bounded, so recursion terminates.
    fn invokes_param(&self, f: &FnInfo, idx: usize, depth: usize) -> bool {
        let Some(param) = f.params.get(idx).filter(|p| !p.is_empty() && *p != "_") else {
            return false;
        };
        f.calls.iter().filter(|c| !c.fn_value).any(|c| {
            c.callee == *param
                || c.args.iter().enumerate().any(|(j, a)| {
                    a == param
                        && match self.resolve_go(&c.callee, &f.handles) {
                            None => true,
                            Some(next) => {
                                depth + 1 < MAX_HELPER_DEPTH
                                    && self.invokes_param(next, j, depth + 1)
                            }
                        }
                })
        })
    }

    /// Python `self.name`: the method on `class`, else on its same-file bases (nearest first).
    /// Cross-file bases are invisible under one-parse-per-file; `assertion_helpers` covers them.
    fn resolve_in_class_chain(&self, class: &str, name: &str) -> Option<&FnInfo> {
        let mut queue = vec![class.to_string()];
        let mut visited = Vec::new();
        while !queue.is_empty() {
            let current = queue.remove(0);
            if visited.contains(&current) {
                continue;
            }
            if let Some(&i) = self.methods.get(&(current.clone(), name.to_string())) {
                return Some(&self.fns[i]);
            }
            queue.extend(self.bases.get(&current).cloned().unwrap_or_default());
            visited.push(current);
        }
        None
    }

    /// Go callees: `helper`, `helper[T]`, `recv.method`, `recv.m(..).n[T]`. A selector resolves by
    /// its last segment to any same-file method; a test-handle receiver (`t.Run`) never does.
    /// Go: the body sets gomock expectations (`m.EXPECT().Foo()`) and, in it or a same-file helper it
    /// reaches (`createSuite(t)`, `defer s.tearDown()`), a controller is made or finished, which is
    /// what fails the test on an unmet expectation.
    fn verifies_expectations(&self, calls: &[Call], handles: &[String]) -> bool {
        calls.iter().any(|c| c.callee.contains(".EXPECT()"))
            && self.reaches_controller(calls, handles, &mut Vec::new())
    }

    fn reaches_controller(
        &self,
        calls: &[Call],
        handles: &[String],
        visited: &mut Vec<usize>,
    ) -> bool {
        calls.iter().any(|call| {
            let last = call.callee.rsplit('.').next().unwrap_or(&call.callee);
            if last == "NewController" || (last == "Finish" && call.args.is_empty()) {
                return true;
            }
            let Some(helper) = self.resolve_go(&call.callee, handles) else {
                return false;
            };
            if visited.len() >= MAX_HELPER_DEPTH || visited.contains(&helper.line) {
                return false;
            }
            visited.push(helper.line);
            let found = self.reaches_controller(&helper.calls, &helper.handles, visited);
            visited.pop();
            found
        })
    }

    fn resolve_go(&self, callee: &str, handles: &[String]) -> Option<&FnInfo> {
        let cleaned = without_comments_and_space(callee);
        let callee = strip_type_args(&cleaned);
        let Some(dot) = last_top_level_dot(callee) else {
            return self.by_name.get(callee).map(|&i| &self.fns[i]);
        };
        let root = callee.split(['.', '(', '[']).next().unwrap_or("");
        if handles.iter().any(|h| h == root) {
            return None;
        }
        let method = strip_type_args(&callee[dot + 1..]);
        self.by_method.get(method).map(|&i| &self.fns[i])
    }
}

/// A multi-line call chain may carry `// ...` comments between links; drop them and whitespace.
fn without_comments_and_space(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        if rest.starts_with("//") {
            rest = rest.find('\n').map_or("", |i| &rest[i..]);
        } else if rest.starts_with("/*") {
            rest = rest.find("*/").map_or("", |i| &rest[i + 2..]);
        } else {
            if !c.is_whitespace() {
                out.push(c);
            }
            rest = &rest[c.len_utf8()..];
        }
    }
    out
}

/// `helper[int]` -> `helper`; leaves anything not ending in a balanced `]` alone.
fn strip_type_args(callee: &str) -> &str {
    if !callee.ends_with(']') {
        return callee;
    }
    let mut depth = 0usize;
    for (i, c) in callee.char_indices().rev() {
        match c {
            ']' => depth += 1,
            '[' => {
                depth -= 1;
                if depth == 0 {
                    return &callee[..i];
                }
            }
            _ => {}
        }
    }
    callee
}

/// Byte index of the last `.` outside any parentheses or brackets.
fn last_top_level_dot(callee: &str) -> Option<usize> {
    let mut depth = 0usize;
    for (i, c) in callee.char_indices().rev() {
        match c {
            ')' | ']' => depth += 1,
            '(' | '[' => depth = depth.saturating_sub(1),
            '.' if depth == 0 => return Some(i),
            _ => {}
        }
    }
    None
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
        if in_non_test_dir(file, self.lang) {
            return Ok(Vec::new());
        }
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
            if self.lang == Language::Go && info.parse_error {
                // Fail quiet: an unparsed body may hold the assertions.
                continue;
            }
            if self.lang == Language::Go && info.env_entrypoint {
                // The test binary re-exec'd as a plugin/server process; the parent test asserts.
                continue;
            }
            if self.lang == Language::Go && info.calls.iter().any(|c| c.callee == "os.Exit") {
                // A Go subprocess re-exec stub (`TestHelperProcess`): the parent test asserts.
                continue;
            }
            if self.lang == Language::Go && info.marks_helper {
                // `t.Helper()` marks a function other tests call (boundary export_test.go), not a test.
                continue;
            }
            if self.lang == Language::Go && scan.verifies_expectations(&info.calls, &info.handles) {
                continue;
            }
            scan.seen.borrow_mut().clear();
            if self.asserts(&scan, &info.calls, &info.handles, info.class.as_deref(), 0) {
                continue;
            }
            let mut message = self.message(test);
            if self.lang == Language::Go
                && crate::test_code::oracle_tag::enabled()
                && let Some(tag) = crate::test_code::oracle_tag::tag_go(
                    test,
                    &crate::test_code::oracle_tag::leading_comment(ctx.source, test.line),
                    &|name| scan.helper_source(name, ctx.source),
                )
            {
                message.push_str("\n  ");
                message.push_str(&tag.annotation());
            }
            findings.push(Finding {
                line: test.line,
                message,
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
    let env_vars = match ctx.tree {
        Some(tree) if lang == Language::Go => env_derived_vars(tree.root_node(), ctx.source),
        _ => Vec::new(),
    };
    if let Some(tree) = ctx.tree {
        walk_preorder(tree.root_node(), &mut |node| {
            if let Some(name) = function_name(lang, node, ctx.source) {
                let handles = handles_in(lang, node, ctx.source);
                let marks_helper =
                    lang == Language::Go && calls_handle_helper(node, ctx.source, &handles);
                fns.push(FnInfo {
                    name: name.to_string(),
                    is_method: lang == Language::Go
                        && GoKind::of(node) == GoKind::MethodDeclaration,
                    line: line_of(node),
                    span: (node.start_byte(), node.end_byte()),
                    calls: calls_in(lang, node, ctx.source, &handles),
                    handles,
                    env_entrypoint: lang == Language::Go
                        && (is_env_gated_entrypoint(node, ctx.source)
                            || is_gated_main_harness(node, ctx.source)
                            || is_helper_process_mode(node, ctx.source, name, &env_vars)),
                    marks_helper,
                    parse_error: lang == Language::Go
                        && (node.has_error() || node.child_by_field_name("body").is_none()),
                    receiver: (lang == Language::Go)
                        .then(|| go_receiver(node, ctx.source))
                        .flatten(),
                    params: if lang == Language::Go {
                        go_params(node, ctx.source)
                    } else {
                        Vec::new()
                    },
                    class: (lang == Language::Python)
                        .then(|| enclosing_class(node, ctx.source))
                        .flatten(),
                });
            }
            true
        });
    }
    let mut by_name = HashMap::new();
    let mut by_method = HashMap::new();
    let mut methods = HashMap::new();
    for (i, f) in fns.iter().enumerate() {
        if let Some(class) = &f.class {
            methods.entry((class.clone(), f.name.clone())).or_insert(i);
        }
        by_name.entry(f.name.clone()).or_insert(i);
        if f.is_method {
            by_method.entry(f.name.clone()).or_insert(i);
        }
    }
    let suite_aliases = match ctx.tree {
        Some(tree) if lang == Language::Go => testify_suite_aliases(tree.root_node(), ctx.source),
        _ => Vec::new(),
    };
    let declared_types = match ctx.tree {
        Some(tree) if lang == Language::Go => go_declared_types(tree.root_node(), ctx.source),
        _ => HashMap::new(),
    };
    let failing_holders = failing_holders(&fns, &vocab, &declared_types);
    Scan {
        vocab,
        suite_aliases,
        failing_holders,
        fns,
        by_name,
        by_method,
        methods,
        bases: if lang == Language::Python {
            python_bases(ctx)
        } else {
            HashMap::new()
        },
        seen: Default::default(),
        screenshot_file: lang == Language::Kotlin
            && ctx.source.to_ascii_lowercase().contains("screenshot"),
    }
}

/// Go: `(receiver name, receiver type)` of a method declaration; the type loses `*` and type args.
fn go_receiver(func: Node, source: &str) -> Option<(String, String)> {
    let decl = func.child_by_field_name("receiver")?.named_child(0)?;
    let name = node_text(decl.child_by_field_name("name")?, source);
    let ty = node_text(decl.child_by_field_name("type")?, source);
    let ty = ty.trim_start_matches('*');
    let ty = ty.split('[').next().unwrap_or(ty);
    Some((name.to_string(), ty.to_string()))
}

/// Go: `Type.field` for each type with a same-file method that fails the test through its `field`
/// (`w.t.Error(..)`, or `require.NoError(w.t, ..)` with a vocabulary assertion).
fn failing_holders(
    fns: &[FnInfo],
    vocab: &AssertionVocabulary,
    types: &HashMap<String, Vec<String>>,
) -> HashSet<String> {
    let mut out = HashSet::new();
    for f in fns {
        let Some((recv, ty)) = &f.receiver else {
            continue;
        };
        for call in f.calls.iter().filter(|c| !c.fn_value) {
            let parts: Vec<&str> = call.callee.split('.').collect();
            if let [r, field, method] = parts[..]
                && r == recv
                && GO_FAILING_METHODS.contains(&method)
            {
                out.insert(format!("{ty}.{field}"));
            }
            for (i, arg) in call.args.iter().enumerate() {
                let Some(field) = arg
                    .strip_prefix(recv.as_str())
                    .and_then(|a| a.strip_prefix('.'))
                    .filter(|a| is_go_path(a) && !a.contains('.'))
                else {
                    continue;
                };
                if vocab.contains(&call.callee)
                    || forwards_to_failure(fns, vocab, &call.callee, i, 0)
                {
                    out.insert(format!("{ty}.{field}"));
                }
            }
        }
    }
    let positional: Vec<String> = out
        .iter()
        .filter_map(|key| {
            let (ty, field) = key.split_once('.')?;
            let at = types.get(ty)?.iter().position(|n| n == field)?;
            Some(format!("{ty}.{POSITIONAL_FIELD}{at}"))
        })
        .collect();
    out.extend(positional);
    out
}

/// Go: the same-file function `callee` names, by bare name or, for a selector, by method name.
fn find_same_file_fn<'a>(fns: &'a [FnInfo], callee: &str) -> Option<&'a FnInfo> {
    let name = callee.rsplit('.').next().unwrap_or(callee);
    let selector = callee.contains('.');
    fns.iter()
        .find(|f| f.name == name && f.is_method == selector)
}

/// Go: does handing the testing handle to `callee` as its `idx`-th argument fail the test? An
/// unresolvable callee is assumed to (as `passes_handle` does); a same-file one must fail on the
/// parameter itself, or hand it on. Depth-bounded, so recursion terminates.
fn forwards_to_failure(
    fns: &[FnInfo],
    vocab: &AssertionVocabulary,
    callee: &str,
    idx: usize,
    depth: usize,
) -> bool {
    let Some(target) = find_same_file_fn(fns, callee) else {
        return true;
    };
    let Some(param) = target
        .params
        .get(idx)
        .filter(|p| !p.is_empty() && *p != "_")
    else {
        return false;
    };
    target.calls.iter().filter(|c| !c.fn_value).any(|c| {
        let fails_directly = c
            .callee
            .strip_prefix(param.as_str())
            .and_then(|m| m.strip_prefix('.'))
            .is_some_and(|m| GO_FAILING_METHODS.contains(&m));
        fails_directly
            || c.args.iter().enumerate().any(|(j, a)| {
                a == param
                    && (vocab.contains(&c.callee)
                        || (depth < MAX_HELPER_DEPTH
                            && forwards_to_failure(fns, vocab, &c.callee, j, depth + 1)))
            })
    })
}

/// Go: local names the file imports testify's `suite` package under (`suite`, or an explicit alias).
fn testify_suite_aliases(root: Node, source: &str) -> Vec<String> {
    let mut aliases = Vec::new();
    walk_preorder(root, &mut |n| {
        if n.kind() == "import_spec"
            && n.child_by_field_name("path").is_some_and(|p| {
                node_text(p, source).trim_matches(['"', '`']) == TESTIFY_SUITE_PATH
            })
        {
            aliases.push(
                n.child_by_field_name("name")
                    .map_or("suite", |a| node_text(a, source))
                    .to_string(),
            );
        }
        true
    });
    aliases
}

/// Go: `Type{field: t}` stores the testing handle in a field; emits a synthetic call per field.
fn handle_holder_calls(node: Node, source: &str, handles: &[String]) -> Vec<Call> {
    let (Some(ty), Some(body)) = (
        node.child_by_field_name("type"),
        node.child_by_field_name("body"),
    ) else {
        return Vec::new();
    };
    let ty = node_text(ty, source);
    let ty = ty.split('[').next().unwrap_or(ty);
    let mut calls = Vec::new();
    let mut cursor = body.walk();
    for (index, el) in body.named_children(&mut cursor).enumerate() {
        let field = if el.kind() == "keyed_element" && el.named_child_count() == 2 {
            let (Some(key), Some(value)) = (el.named_child(0), el.named_child(1)) else {
                continue;
            };
            handles
                .iter()
                .any(|h| h == node_text(value, source))
                .then(|| node_text(key, source).to_string())
        } else if el.kind() == "literal_element" || el.kind() == "element" {
            handles
                .iter()
                .any(|h| h == node_text(el, source))
                .then(|| format!("{POSITIONAL_FIELD}{index}"))
        } else {
            None
        };
        if let Some(field) = field {
            calls.push(Call::new(
                format!("{HOLDER_PREFIX}{ty}.{field}"),
                Vec::new(),
            ));
        }
    }
    calls
}

/// Go: `type Name struct { ... }` -> its field names in declaration order; every other type
/// declaration maps to an empty list.
fn go_declared_types(root: Node, source: &str) -> HashMap<String, Vec<String>> {
    let mut types = HashMap::new();
    walk_preorder(root, &mut |n| {
        if n.kind() == "type_spec"
            && let Some(name) = n.child_by_field_name("name")
        {
            let fields = n
                .child_by_field_name("type")
                .filter(|t| t.kind() == "struct_type")
                .map(|decl| struct_field_names(decl, source))
                .unwrap_or_default();
            types.insert(node_text(name, source).to_string(), fields);
        }
        true
    });
    types
}

/// Field names of a Go `struct_type` in order; an embedded field is `""`.
fn struct_field_names(decl: Node, source: &str) -> Vec<String> {
    let mut fields = Vec::new();
    walk_preorder(decl, &mut |f| {
        if f.kind() == "field_declaration" {
            let mut cursor = f.walk();
            let names: Vec<String> = f
                .children_by_field_name("name", &mut cursor)
                .map(|x| node_text(x, source).to_string())
                .collect();
            if names.is_empty() {
                fields.push(String::new());
            }
            fields.extend(names);
        }
        true
    });
    fields
}

/// `try: validate(...)` / `except ...: ...; raise e`: the schema validator's failure is the
/// test failure, but only because the handler re-raises (a swallowing handler asserts nothing).
fn reraises_validate(try_node: Node, source: &str) -> bool {
    let mut cursor = try_node.walk();
    let (mut validates, mut reraises) = (false, false);
    for child in try_node.named_children(&mut cursor) {
        match child.kind() {
            "block" => walk_preorder(child, &mut |n| {
                if PythonKind::of(n) == PythonKind::Call
                    && let Some(func) = n.child_by_field_name("function")
                {
                    validates |= node_text(func, source).rsplit('.').next() == Some("validate");
                }
                true
            }),
            "except_clause" | "except_group_clause" => walk_preorder(child, &mut |n| {
                reraises |= n.kind() == "raise_statement";
                true
            }),
            _ => {}
        }
    }
    validates && reraises
}

/// Go only for the `_` rule: the toolchain ignores `_*` directories (`_sample`, `_fixtures`), but
/// `_internal` / `__tests__` hold real tests in other ecosystems.
fn in_non_test_dir(file: &Path, lang: Language) -> bool {
    file.components().any(|c| {
        NON_TEST_DIRS.iter().any(|d| c.as_os_str() == *d)
            || (lang == Language::Go && c.as_os_str().to_string_lossy().starts_with('_'))
    })
}

fn class_name<'a>(class: Node, source: &'a str) -> Option<&'a str> {
    class
        .child_by_field_name("name")
        .map(|n| node_text(n, source))
}

fn enclosing_class(node: Node, source: &str) -> Option<String> {
    let mut cur = node.parent();
    while let Some(n) = cur {
        if n.kind() == "class_definition" {
            return class_name(n, source).map(str::to_string);
        }
        cur = n.parent();
    }
    None
}

/// class name -> base names as written (last segment of a dotted base), whole file.
fn python_bases(ctx: &CheckContext) -> HashMap<String, Vec<String>> {
    let mut out = HashMap::new();
    let Some(tree) = ctx.tree else { return out };
    walk_preorder(tree.root_node(), &mut |node| {
        if node.kind() == "class_definition"
            && let Some(name) = class_name(node, ctx.source)
            && let Some(supers) = node.child_by_field_name("superclasses")
        {
            let mut cursor = supers.walk();
            let bases = supers
                .named_children(&mut cursor)
                .filter(|b| matches!(b.kind(), "identifier" | "attribute"))
                .map(|b| {
                    node_text(b, ctx.source)
                        .rsplit('.')
                        .next()
                        .unwrap_or("")
                        .to_string()
                })
                .collect();
            out.insert(name.to_string(), bases);
        }
        true
    });
    out
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

fn calls_in(lang: Language, root: Node, source: &str, handles: &[String]) -> Vec<Call> {
    if lang == Language::Kotlin {
        return kotlin_calls_in(root, source);
    }
    let carriers = if lang == Language::Go {
        handle_carrier_vars(root, source, handles)
    } else {
        Vec::new()
    };
    let (locals, aliases) = if lang == Language::Go {
        (
            go_locals(root, source),
            failing_aliases(root, source, handles),
        )
    } else {
        (Vec::new(), HashMap::new())
    };
    let range_vars = if lang == Language::Go {
        go_range_vars(root, source)
    } else {
        Vec::new()
    };
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
                .collect::<Vec<String>>();
            let callee = node_text(func, source).to_string();
            let carries_handle = lang == Language::Go
                && args.iter().any(|a| {
                    let mut c = a.walk();
                    a.named_children(&mut c).any(|arg| {
                        has_handle_field(arg, source, handles)
                            || carriers.iter().any(|v| v == node_text(arg, source))
                    })
                });
            if lang == Language::Go && callee.ends_with(".Run") {
                calls.extend(subtest_body_call(args, source, &locals, &range_vars));
            } else if lang == Language::Go {
                // `ExecAPITest(t, testX)` / `run(t, testX[int])`: a function value handed to a runner.
                let mut c = node.walk();
                calls.extend(
                    args.iter()
                        .flat_map(|a| a.named_children(&mut c).collect::<Vec<_>>())
                        .filter(|a| {
                            !matches!(a.kind(), "call_expression" | "type_conversion_expression")
                        })
                        .filter_map(|a| go_function_target(a, source))
                        .map(Call::value),
                );
            }
            if lang == Language::Go && is_mock_panic_expectation(&callee) {
                calls.push(Call::new(GUARDED_PANIC.to_string(), Vec::new()));
            }
            let callee = aliases.get(&callee).cloned().unwrap_or(callee);
            let mut call = Call::new(callee, arg_texts);
            call.carries_handle = carries_handle;
            call.on_handle_holder = lang == Language::Go
                && call.callee.split_once('.').is_some_and(|(root, m)| {
                    !m.contains('.') && carriers.iter().any(|v| v == root)
                });
            calls.push(call);
        } else if lang == Language::Go
            && node.kind() == "type_conversion_expression"
            && let Some(generic) = node
                .child_by_field_name("type")
                .filter(|t| t.kind() == "generic_type")
        {
            // tree-sitter-go reads the one-argument call `helper[T](t)` as a conversion.
            calls.push(Call::new(
                node_text(generic, source).to_string(),
                node.child_by_field_name("operand")
                    .map(|o| node_text(o, source).to_string())
                    .into_iter()
                    .collect(),
            ));
        } else if lang == Language::Go
            && node.kind() == "if_statement"
            && is_guarded_panic(node, source)
        {
            calls.push(Call::new(GUARDED_PANIC.to_string(), Vec::new()));
        } else if lang == Language::Go && node.kind() == "composite_literal" {
            calls.extend(handle_holder_calls(node, source, handles));
        } else if lang == Language::Go
            && node.kind() == "keyed_element"
            && let Some(value) = node.named_child(node.named_child_count().saturating_sub(1) as u32)
            && node.named_child_count() == 2
            && is_go_path(node_text(value, source))
        {
            // `Args{objAPITest: testX}`: a function value stored in a struct field.
            calls.push(Call::value(node_text(value, source).to_string()));
        } else if lang == Language::Python && node.kind() == "try_statement" {
            if reraises_validate(node, source) {
                calls.push(Call::new(RERAISED_VALIDATE.to_string(), Vec::new()));
            }
        } else if lang == Language::Python && PythonKind::of(node) == PythonKind::AssertStatement {
            calls.push(Call::new("assert".to_string(), Vec::new()));
        }
        true
    });
    calls
}

/// Go: the name a function-valued expression resolves through, or `None` when it is not one
/// (a literal, which the caller's own subtree walk already covers). Accepts an identifier, a
/// selector, a generic instantiation (`f[T]`, `f[A, B]`), a parenthesized form, and a call or
/// conversion of any of those (`f[T](..)`, `x.m(..)`), where the callee stands for the value
/// the call returns.
fn go_function_target(node: Node, source: &str) -> Option<String> {
    match node.kind() {
        "parenthesized_expression" => go_function_target(node.named_child(0)?, source),
        "call_expression" => go_function_target(node.child_by_field_name("function")?, source),
        "type_conversion_expression" => {
            go_function_target(node.child_by_field_name("type")?, source)
        }
        "identifier"
        | "selector_expression"
        | "index_expression"
        | "generic_type"
        | "type_instantiation_expression" => Some(node_text(node, source).to_string()),
        _ => None,
    }
}

/// Go: `t.Run(name, body)` -- the last argument names or builds the subtest body. Followed into a
/// same-file function (a returned closure is part of its subtree); one defined elsewhere, or read
/// off a value, is unknowable rather than assertion-free. A bare local that is not a range
/// variable (`body := func(..){..}`) keeps being an ordinary call so its literal is judged.
fn subtest_body_call(
    args: Option<Node>,
    source: &str,
    locals: &[String],
    range_vars: &[String],
) -> Option<Call> {
    let args = args?;
    let mut cursor = args.walk();
    let all: Vec<Node> = args.named_children(&mut cursor).collect();
    let [_, .., last] = all[..] else { return None };
    let target = go_function_target(last, source)?;
    let bare_local = !target.contains(['.', '(', '['])
        && locals.contains(&target)
        && !range_vars.contains(&target);
    Some(if bare_local {
        Call::new(target, Vec::new())
    } else {
        Call::dispatched(target)
    })
}

/// Go: names a `for ... range` loop binds, which can hold any function value.
fn go_range_vars(root: Node, source: &str) -> Vec<String> {
    let mut names = Vec::new();
    walk_preorder(root, &mut |n| {
        if n.kind() == "range_clause"
            && let Some(left) = n.child_by_field_name("left")
        {
            let mut cursor = left.walk();
            names.extend(
                left.named_children(&mut cursor)
                    .map(|x| node_text(x, source).to_string()),
            );
        }
        true
    });
    names
}

/// Go: names bound inside `root` (parameters, `:=`, `var`, range variables), so a selector on one
/// is a field or method of a value rather than a package-qualified name.
fn go_locals(root: Node, source: &str) -> Vec<String> {
    let mut names = go_params(root, source);
    walk_preorder(root, &mut |n| {
        let bound = match n.kind() {
            "short_var_declaration" | "range_clause" => n.child_by_field_name("left"),
            "var_spec" => n.child_by_field_name("name"),
            _ => None,
        };
        if let Some(bound) = bound {
            names.push(node_text(bound, source).to_string());
            let mut cursor = bound.walk();
            names.extend(
                bound
                    .named_children(&mut cursor)
                    .map(|x| node_text(x, source).to_string()),
            );
        }
        true
    });
    names
}

/// Go: `f := tb.Errorf` / `var f = t.Fatalf` / `f = tb.Fatalf` bind `f` to a failing testing
/// method, so calling `f(..)` fails the test. Maps alias name -> the method value it holds.
fn failing_aliases(root: Node, source: &str, handles: &[String]) -> HashMap<String, String> {
    let mut aliases = HashMap::new();
    if handles.is_empty() {
        return aliases;
    }
    walk_preorder(root, &mut |n| {
        let (left, right) = match n.kind() {
            "short_var_declaration" | "assignment_statement" => (
                n.child_by_field_name("left"),
                n.child_by_field_name("right"),
            ),
            "var_spec" => (
                n.child_by_field_name("name"),
                n.child_by_field_name("value"),
            ),
            _ => (None, None),
        };
        if let (Some(left), Some(right)) = (left, right) {
            let (left, right) = (node_text(left, source), node_text(right, source));
            if is_failing_method_value(right, handles) && is_go_path(left) && !left.contains('.') {
                aliases.insert(left.to_string(), right.to_string());
            }
        }
        true
    });
    aliases
}

/// Go: a struct literal field set to the testing handle (`Params{T: t}`), at any depth of `node`.
fn has_handle_field(node: Node, source: &str, handles: &[String]) -> bool {
    let mut found = false;
    walk_preorder(node, &mut |n| {
        if n.kind() == "keyed_element"
            && n.named_child_count() == 2
            && n.named_child(1)
                .is_some_and(|v| handles.iter().any(|h| h == node_text(v, source)))
        {
            found = true;
        }
        !found
    });
    found
}

/// Go: variables bound to a struct literal that carries the testing handle
/// (`params := Params{T: t}`), so `Check(params)` is seen to pass it on.
fn handle_carrier_vars(root: Node, source: &str, handles: &[String]) -> Vec<String> {
    let mut vars = Vec::new();
    if handles.is_empty() {
        return vars;
    }
    walk_preorder(root, &mut |n| {
        let (left, right) = match n.kind() {
            "short_var_declaration" | "assignment_statement" => (
                n.child_by_field_name("left"),
                n.child_by_field_name("right"),
            ),
            "var_spec" => (
                n.child_by_field_name("name"),
                n.child_by_field_name("value"),
            ),
            _ => (None, None),
        };
        if let (Some(left), Some(right)) = (left, right)
            && has_handle_field(right, source, handles)
        {
            let mut cursor = left.walk();
            let names: Vec<_> = left.named_children(&mut cursor).collect();
            if names.is_empty() {
                vars.push(node_text(left, source).to_string());
            }
            vars.extend(names.iter().map(|x| node_text(*x, source).to_string()));
        }
        true
    });
    vars
}

/// Mock expectation builder that makes the forbidden call panic and so fail the test:
/// mockery `m.EXPECT().Foo(..).Panic("..")` or testify `m.On("Foo").Panic("..")`.
fn is_mock_panic_expectation(callee: &str) -> bool {
    let callee = without_comments_and_space(callee);
    callee.ends_with(".Panic") && (callee.contains(".EXPECT()") || callee.contains(".On("))
}

/// Statements of a Go block, looking through the `statement_list` wrapper, without comments.
fn block_statements(block: Node) -> Vec<Node> {
    let mut cursor = block.walk();
    let mut stmts: Vec<Node> = block
        .named_children(&mut cursor)
        .filter(|s| s.kind() != "comment")
        .collect();
    if let [only] = stmts[..]
        && only.kind() == "statement_list"
    {
        let mut inner = only.walk();
        stmts = only.named_children(&mut inner).collect();
    }
    stmts.retain(|s| s.kind() != "comment");
    stmts
}

/// A test that is the re-exec'd subprocess of another test: it returns unless an environment
/// variable is set, and its remaining body ends by serving a plugin or running a main. Requires
/// the gate to cover the whole body, and a serve/run call, so a gated body that does nothing or
/// only calls ordinary code is still reported.
fn is_env_gated_entrypoint(func: Node, source: &str) -> bool {
    let Some(body) = func.child_by_field_name("body") else {
        return false;
    };
    let stmts = block_statements(body);
    let Some((gate, rest)) = stmts.split_first() else {
        return false;
    };
    if gate.kind() != "if_statement" || gate.child_by_field_name("alternative").is_some() {
        return false;
    }
    let Some(consequence) = gate.child_by_field_name("consequence") else {
        return false;
    };
    let head = &source[gate.start_byte()..consequence.start_byte()];
    if !(head.contains("os.Getenv(") || head.contains("os.LookupEnv(")) {
        return false;
    }
    let gated = block_statements(consequence);
    let serving = if rest.is_empty() {
        gated
    } else if gated.len() == 1 && gated[0].kind() == "return_statement" {
        rest.to_vec()
    } else {
        return false;
    };
    serving.last().is_some_and(|last| {
        last.kind() == "expression_statement"
            && last
                .named_child(0)
                .filter(|c| GoKind::of(*c) == GoKind::CallExpression)
                .and_then(|c| c.child_by_field_name("function"))
                .is_some_and(|f| {
                    let name = node_text(f, source).rsplit('.').next().unwrap_or("");
                    GO_ENTRYPOINT_PREFIXES.iter().any(|p| name.starts_with(p))
                })
    })
}

fn calls_handle_helper(func: Node, source: &str, handles: &[String]) -> bool {
    let Some(body) = func.child_by_field_name("body") else {
        return false;
    };
    block_statements(body).iter().any(|s| {
        s.kind() == "expression_statement"
            && s.named_child(0)
                .filter(|c| GoKind::of(*c) == GoKind::CallExpression)
                .and_then(|c| c.child_by_field_name("function"))
                .is_some_and(|f| {
                    let text = node_text(f, source);
                    handles.iter().any(|h| text == format!("{h}.Helper"))
                })
    })
}

/// A coverage/system-test harness: the body ends in `if <cond> { main() }` (beats `TestSystem` under
/// `-systemTest`, tidb `TestRunMain` under `isCoverageServer`). The gate may not be error handling,
/// and an ungated `main()` (mattermost `TestRunMain`) is deliberately left reported.
fn is_gated_main_harness(func: Node, source: &str) -> bool {
    let Some(body) = func.child_by_field_name("body") else {
        return false;
    };
    let Some(gate) = block_statements(body).pop() else {
        return false;
    };
    if gate.kind() != "if_statement" || gate.child_by_field_name("alternative").is_some() {
        return false;
    }
    let Some(consequence) = gate.child_by_field_name("consequence") else {
        return false;
    };
    let head = &source[gate.start_byte()..consequence.start_byte()];
    !head.contains("err")
        && block_statements(consequence).iter().any(|s| {
            s.kind() == "expression_statement"
                && s.named_child(0)
                    .filter(|c| GoKind::of(*c) == GoKind::CallExpression)
                    .and_then(|c| c.child_by_field_name("function"))
                    .is_some_and(|f| node_text(f, source) == "main")
        })
}

/// Package-level `var x = ... os.Getenv(..) ...` names: switches that put the test binary in a
/// re-exec'd mode.
fn env_derived_vars(root: Node, source: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cursor = root.walk();
    for decl in root.named_children(&mut cursor) {
        if decl.kind() != "var_declaration" {
            continue;
        }
        walk_preorder(decl, &mut |n| {
            if n.kind() == "var_spec"
                && let Some(value) = n.child_by_field_name("value")
                && let text = node_text(value, source)
                && (text.contains("os.Getenv(") || text.contains("os.LookupEnv("))
                && let Some(name) = n.child_by_field_name("name")
            {
                out.push(node_text(name, source).to_string());
            }
            true
        });
    }
    out
}

/// `func TestX_Helper(t) { if !helperMode { return }; ... }` where `helperMode` is an env-derived
/// package var: the test binary re-exec'd as a helper process (beats `gotest_test.go`), whose
/// parent test asserts. Needs "helper" in the test or switch name, so an ordinary env-gated
/// test (`slowMode`) is still reported.
fn is_helper_process_mode(func: Node, source: &str, name: &str, env_vars: &[String]) -> bool {
    let Some(body) = func.child_by_field_name("body") else {
        return false;
    };
    let Some(gate) = block_statements(body).first().copied() else {
        return false;
    };
    if gate.kind() != "if_statement" || gate.child_by_field_name("alternative").is_some() {
        return false;
    }
    let (Some(cond), Some(consequence)) = (
        gate.child_by_field_name("condition"),
        gate.child_by_field_name("consequence"),
    ) else {
        return false;
    };
    let switch = node_text(cond, source)
        .trim()
        .trim_start_matches('!')
        .trim();
    let returns =
        matches!(block_statements(consequence).as_slice(), [r] if r.kind() == "return_statement");
    returns
        && env_vars.iter().any(|v| v == switch)
        && [name, switch]
            .iter()
            .any(|n| n.to_ascii_lowercase().contains("helper"))
}

/// `if <cond> { ...panic(...)... }` (or its `else`) where the condition tests behaviour, or any
/// guard at all when lexically inside a `go` statement.
fn is_guarded_panic(if_node: Node, source: &str) -> bool {
    let Some(cond) = if_node.child_by_field_name("condition") else {
        return false;
    };
    let cond = node_text(cond, source);
    let error_propagation = cond.contains("err") && cond.contains("!= nil");
    if error_propagation && !inside_go_statement(if_node) {
        return false;
    }
    let mut panics = false;
    for field in ["consequence", "alternative"] {
        if let Some(branch) = if_node.child_by_field_name(field) {
            walk_preorder(branch, &mut |n| {
                panics |= GoKind::of(n) == GoKind::CallExpression
                    && n.child_by_field_name("function")
                        .is_some_and(|f| node_text(f, source) == "panic");
                true
            });
        }
    }
    panics
}

fn inside_go_statement(node: Node) -> bool {
    std::iter::successors(node.parent(), |n| n.parent()).any(|n| n.kind() == "go_statement")
}

/// Parameter names of a Go function or method in order, `""` standing in for an unnamed one.
fn go_params(func: Node, source: &str) -> Vec<String> {
    let mut out = Vec::new();
    let Some(params) = func.child_by_field_name("parameters") else {
        return out;
    };
    let mut cursor = params.walk();
    for decl in params.named_children(&mut cursor) {
        let mut inner = decl.walk();
        let names: Vec<String> = decl
            .children_by_field_name("name", &mut inner)
            .map(|n| node_text(n, source).to_string())
            .collect();
        if names.is_empty() {
            out.push(String::new());
        } else {
            out.extend(names);
        }
    }
    out
}

fn is_go_path(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
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
                    calls.push(Call::new(kotlin_callee(callee, source), args));
                }
            }
            "infix_expression" => {
                if let Some(op) = node
                    .named_child(1)
                    .filter(|_| node.named_child_count() == 3)
                {
                    calls.push(Call::new(node_text(op, source).to_string(), Vec::new()));
                }
            }
            // The grammar reads `f<T> { ... }` as `f < T > lambda`; recover `f` as the callee.
            "binary_expression" => {
                if let Some(name) = node.named_child(0).filter(|n| n.kind() == "identifier")
                    && source
                        .get(name.end_byte()..)
                        .is_some_and(|r| r.starts_with('<'))
                {
                    calls.push(Call::new(node_text(name, source).to_string(), Vec::new()));
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
