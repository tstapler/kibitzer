//! Intentional-oracle tagger (prototype): classifies a Go test that has no assertion call by the
//! mechanical shape of its body. Annotates only; a tag never suppresses a finding (ADR-004).
//!
//! Only the three classes that generalized across three held-out rounds are emitted:
//! `empty-stub`, `race-smoke` and `compile-check`. Lifecycle, no-panic, discarded-result and
//! liveness classes did not (see `docs/backtest-triage/oracle-tagger/RESULTS.md`), so a body that
//! matches none of the three gets no tag at all. Signals are text and statement-shape only: no
//! type information, so "is this call a conversion or a function call" is not decidable here.

use std::sync::LazyLock;

use regex::Regex;

use super::TestFn;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OracleTag {
    CompileCheck,
    RaceSmoke,
    EmptyStub,
}

impl OracleTag {
    pub fn label(self) -> &'static str {
        match self {
            Self::CompileCheck => "compile-check",
            Self::RaceSmoke => "race-smoke",
            Self::EmptyStub => "empty-stub",
        }
    }

    /// The extra finding line; ends with `[tag=<label>]` so a run can be parsed back.
    pub fn annotation(self) -> String {
        let text = match self {
            Self::CompileCheck => "Smoke-only: compile-check (blank-identifier declarations only)",
            Self::RaceSmoke => {
                "Smoke-only: race (goroutines and sync primitives, results discarded)"
            }
            Self::EmptyStub => "Stub: empty body or TODO only",
        };
        format!("{text} [tag={}]", self.label())
    }
}

/// Env flag read by the checker to append [`OracleTag::annotation`] to Go findings.
pub const ENV_FLAG: &str = "KIBITZER_ORACLE_TAGS";

pub fn enabled() -> bool {
    std::env::var(ENV_FLAG).is_ok_and(|v| v == "1")
}

/// Race intent in a test name. `race` must be a CamelCase or snake word (`TestRace_x`,
/// `TestProxyDataRace`), never a substring (`Trace`, `Grace`).
static RACE_NAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)data.?race|racy|race.?(cond|detect)|concurren|thread.?safe|atomic|multi.?thread|hammer|stress|contention|parallelism|mutex|(?-i:(?:^|[a-z_])Races?(?:[A-Z_0-9]|$))")
        .expect("valid regex")
});
/// Race intent in a comment: the race detector, a race condition, or concurrent / parallel access.
static RACE_COMMENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\braces?\b|-race\b|data.?race|racy|concurren|thread.?safe|atomic|in parallel|simultaneous")
        .expect("valid regex")
});
/// Race intent in the test name strong enough to tag a body with no goroutine of its own (the
/// callee spawns them).
static RACE_DETECTOR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)data.?race|racy|race.?(cond|detect)|(?-i:(?:^|[a-z_])Races?(?:[A-Z_0-9]|$))")
        .expect("valid regex")
});
/// Deadlock / hang intent: the oracle is "returns", which the race detector adds nothing to.
static LIVENESS_INTENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)dead.?lock|live.?lock|hangs?\b").expect("valid regex"));
static EXPLICIT_RACE_COMMENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)-race\b|data.?race|race.?detect").expect("valid regex"));
static CHANNEL_USE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<-|\bchan\b|\bselect\b").expect("valid regex"));
static JOIN_PRIMITIVE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"WaitGroup|\bMutex\b|\bRWMutex\b|errgroup").expect("valid regex"));
static COMPILE_NOTE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)compile.?time|compiles").expect("valid regex"));
static INTERFACE_NAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)interface|implements|satisf|conform|compile").expect("valid regex")
});
static COMPOSITE_RHS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^&?[A-Za-z_][\w.\[\]*]*\s*\{").expect("valid regex"));

/// What a top-level statement of the test body is, as far as the tagger cares.
#[derive(Debug, PartialEq)]
enum Stmt {
    BlankDecl,
    BlankAssign(String),
    /// A `:=` binding with at least one named variable.
    Define,
    Call(String),
    Other,
}

struct Scanned {
    /// Body with comments removed and literal contents blanked.
    code: String,
    /// Comment text, concatenated.
    comments: String,
}

/// Comment lines directly above the 1-based `line` of a declaration, joined; they hold intent
/// ("find the data race", "safe to mutate in parallel") that the function body does not.
pub fn leading_comment(source: &str, line: usize) -> String {
    let lines: Vec<&str> = source.lines().collect();
    let mut out = Vec::new();
    let mut at = line.saturating_sub(1);
    while at > 0 {
        let above = lines.get(at - 1).map_or("", |l| l.trim());
        if !above.starts_with("//") {
            break;
        }
        out.push(above);
        at -= 1;
    }
    out.reverse();
    out.join("\n")
}

pub fn tag_go(
    f: &TestFn,
    leading_comment: &str,
    helper_source: &dyn Fn(&str) -> Option<String>,
) -> Option<OracleTag> {
    let scanned = scan(&f.body);
    let inner = inner_body(&scanned.code);
    let stmts: Vec<Stmt> = split_statements(inner)
        .iter()
        .map(|s| classify(s))
        .collect();
    let comments = format!("{leading_comment}\n{}", scanned.comments);

    if inner.trim().is_empty()
        || delegates_only_to_stubs(&stmts, helper_source)
        || loops_over_stub_subtests(&scanned.code, inner, helper_source)
    {
        Some(OracleTag::EmptyStub)
    } else if is_compile_check(&stmts, &f.name, inner) || is_compile_note(&stmts, &scanned.comments)
    {
        Some(OracleTag::CompileCheck)
    } else if is_race_smoke(&scanned.code, inner, &f.name, &comments) {
        Some(OracleTag::RaceSmoke)
    } else {
        None
    }
}

/// The body is only calls to same-file helpers that are themselves stubs (empty, TODO-only, or
/// only `t.Skip*`). One level: a stub helper's own delegation is not followed.
fn delegates_only_to_stubs(stmts: &[Stmt], helper_source: &dyn Fn(&str) -> Option<String>) -> bool {
    !stmts.is_empty()
        && stmts.iter().all(|s| match s {
            Stmt::Call(c) if !c.contains(['.', '[', '(']) => {
                helper_source(c).is_some_and(|src| is_stub_body(&src))
            }
            _ => false,
        })
}

fn is_stub_body(helper_src: &str) -> bool {
    let scanned = scan(helper_src);
    let inner = inner_body(&scanned.code);
    split_statements(inner).iter().all(|s| {
        matches!(classify(s), Stmt::Call(c) if c.contains('.') && last_segment(&c).starts_with("Skip"))
    })
}

/// Setup bindings (`x := f()`) and `t.*` calls may surround the blank declarations. A discarded
/// constructor result (`_ = NewX(..)`) counts only when the test is named for an interface check.
/// A lone `t.Run(.., func(t *testing.T) {..})` is looked through to its closure body.
fn is_compile_check(stmts: &[Stmt], name: &str, inner: &str) -> bool {
    let unwrapped: Vec<Stmt>;
    let stmts = match unwrap_single_subtest(inner) {
        Some(body) if stmts.len() == 1 => {
            unwrapped = split_statements(body).iter().map(|s| classify(s)).collect();
            &unwrapped[..]
        }
        _ => stmts,
    };
    let relevant: Vec<&Stmt> = stmts
        .iter()
        .filter(|s| {
            !matches!(s, Stmt::Define) && !matches!(s, Stmt::Call(c) if c.starts_with("t."))
        })
        .collect();
    let named_for_interface = INTERFACE_NAME.is_match(name);
    !relevant.is_empty()
        && relevant.iter().all(|s| match s {
            Stmt::BlankDecl => true,
            Stmt::BlankAssign(rhs) => {
                let rhs = rhs.trim();
                rhs.starts_with('&')
                    || rhs.contains("(nil)")
                    || COMPOSITE_RHS.is_match(rhs)
                    || named_for_interface
            }
            _ => false,
        })
}

/// Body of the closure when the whole test is one `t.Run(name, func(t *testing.T) { .. })`.
fn unwrap_single_subtest(inner: &str) -> Option<&str> {
    let trimmed = inner.trim();
    let rest = trimmed.strip_prefix("t.Run(")?;
    let open = rest.find("func(")? + rest[rest.find("func(")?..].find('{')?;
    let mut depth = 0i32;
    for (i, c) in rest[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    let tail = rest[open + i + 1..].trim();
                    return (tail == ")").then(|| &rest[open + 1..open + i]);
                }
            }
            _ => {}
        }
    }
    None
}

/// Subtests run in a loop over a table, and the only calls anywhere in the body are `t.Run` and
/// same-file helpers that are themselves stubs (dolt `testIdentity` that only `t.Skip`s).
fn loops_over_stub_subtests(
    code: &str,
    inner: &str,
    helper_source: &dyn Fn(&str) -> Option<String>,
) -> bool {
    static CALL: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"([A-Za-z_][\w.]*)\s*\(").expect("valid regex"));
    if !code.contains("t.Run(") {
        return false;
    }
    let mut helpers = 0;
    for caps in CALL.captures_iter(inner) {
        let callee = &caps[1];
        match callee {
            "t.Run" | "func" => {}
            plain if !plain.contains('.') => {
                if helper_source(plain).is_none_or(|src| !is_stub_body(&src)) {
                    return false;
                }
                helpers += 1;
            }
            _ => return false,
        }
    }
    helpers > 0
}

/// A body that is only `t.Log`-style calls next to a comment saying the real check is the compiler.
fn is_compile_note(stmts: &[Stmt], comments: &str) -> bool {
    !stmts.is_empty()
        && stmts
            .iter()
            .all(|s| matches!(s, Stmt::Call(c) if c.starts_with("t.")))
        && COMPILE_NOTE.is_match(comments)
}

/// Race-detector intent: named in the test name or a comment, or (without that) an identical
/// goroutine fan-out joined by a WaitGroup/mutex with no channel traffic. Goroutines wired by
/// channels (producer/consumer, "does not deadlock") are liveness probes and get no tag.
fn is_race_smoke(code: &str, inner: &str, name: &str, comments: &str) -> bool {
    let go_count = count_go_statements(inner);
    if go_count == 0 {
        return RACE_DETECTOR.is_match(name);
    }
    if LIVENESS_INTENT.is_match(name) || LIVENESS_INTENT.is_match(comments) {
        return RACE_DETECTOR.is_match(name) || EXPLICIT_RACE_COMMENT.is_match(comments);
    }
    let cue = RACE_NAME.is_match(name) || RACE_COMMENT.is_match(comments);
    cue || (JOIN_PRIMITIVE.is_match(code)
        && !CHANNEL_USE.is_match(code)
        && (go_count >= 2 || go_in_loop(inner)))
}

fn last_segment(callee: &str) -> &str {
    callee.rsplit('.').next().unwrap_or(callee)
}

/// Strips comments and blanks string/rune literal contents (newlines inside raw strings become
/// spaces so statement splitting is not fooled).
fn scan(src: &str) -> Scanned {
    let chars: Vec<char> = src.chars().collect();
    let mut code = String::with_capacity(src.len());
    let mut comments = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        match (c, next) {
            ('/', Some('/')) => {
                i += 2;
                while i < chars.len() && chars[i] != '\n' {
                    comments.push(chars[i]);
                    i += 1;
                }
                comments.push('\n');
            }
            ('/', Some('*')) => {
                i += 2;
                while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                    comments.push(chars[i]);
                    i += 1;
                }
                i += 2;
                code.push(' ');
            }
            ('"', _) | ('\'', _) => {
                code.push(c);
                i += 1;
                while i < chars.len() && chars[i] != c {
                    if chars[i] == '\\' {
                        i += 1;
                    }
                    i += 1;
                }
                code.push(c);
                i += 1;
            }
            ('`', _) => {
                code.push('`');
                i += 1;
                while i < chars.len() && chars[i] != '`' {
                    i += 1;
                }
                code.push('`');
                i += 1;
            }
            _ => {
                code.push(c);
                i += 1;
            }
        }
    }
    Scanned { code, comments }
}

/// Text between the function's opening `{` and its final `}`.
fn inner_body(code: &str) -> &str {
    let start = code.find('{').map_or(0, |i| i + 1);
    let end = code.rfind('}').unwrap_or(code.len());
    if start <= end { &code[start..end] } else { "" }
}

/// Top-level statements: split at newlines or `;` outside any bracket.
fn split_statements(inner: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut depth = 0i32;
    for c in inner.chars() {
        match c {
            '(' | '{' | '[' => depth += 1,
            ')' | '}' | ']' => depth -= 1,
            _ => {}
        }
        if (c == '\n' || c == ';') && depth <= 0 {
            if !cur.trim().is_empty() {
                out.push(cur.trim().to_string());
            }
            cur.clear();
        } else {
            cur.push(c);
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

fn first_word(s: &str) -> &str {
    s.split(|c: char| !c.is_alphanumeric() && c != '_')
        .next()
        .unwrap_or("")
}

/// Index of the first top-level assignment operator and whether it is a `:=`.
fn find_assign(s: &str) -> Option<(usize, usize)> {
    let b: Vec<char> = s.chars().collect();
    let mut depth = 0i32;
    let mut byte = 0usize;
    for (i, &c) in b.iter().enumerate() {
        match c {
            '(' | '{' | '[' => depth += 1,
            ')' | '}' | ']' => depth -= 1,
            ':' if depth == 0 && b.get(i + 1) == Some(&'=') => return Some((byte, 2)),
            '=' if depth == 0 => {
                let prev = if i > 0 { b[i - 1] } else { ' ' };
                let next = b.get(i + 1).copied().unwrap_or(' ');
                if next != '=' && !matches!(prev, '=' | '!' | '<' | '>' | ':') {
                    return Some((byte, 1));
                }
            }
            _ => {}
        }
        byte += c.len_utf8();
    }
    None
}

fn classify(s: &str) -> Stmt {
    let word = first_word(s);
    if s.starts_with("var _ ") || s.starts_with("var _\t") {
        return Stmt::BlankDecl;
    }
    if matches!(
        word,
        "go" | "defer"
            | "for"
            | "if"
            | "switch"
            | "select"
            | "return"
            | "func"
            | "var"
            | "const"
            | "break"
            | "continue"
    ) || s.starts_with("<-")
    {
        return Stmt::Other;
    }
    if let Some((at, len)) = find_assign(s) {
        let rhs = s[at + len..].trim();
        if s[..at].split(',').all(|p| p.trim() == "_") {
            return Stmt::BlankAssign(rhs.to_string());
        }
        return if len == 2 { Stmt::Define } else { Stmt::Other };
    }
    if s.ends_with(')')
        && let Some(paren) = s.find('(')
    {
        return Stmt::Call(s[..paren].trim().to_string());
    }
    Stmt::Other
}

/// Goroutine launches at any nesting depth (usually inside loops): `go f()`, `go func() {...}()`
/// and `<recv>.Go(...)` (`sync.WaitGroup.Go`, `errgroup.Group.Go`).
fn count_go_statements(inner: &str) -> usize {
    static GO: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?m)(^|[{;\s])go\s+(func\b|[A-Za-z_][\w.]*\()|\b[A-Za-z_]\w*\.Go\(")
            .expect("valid regex")
    });
    GO.find_iter(inner).count()
}

fn is_go_launch(line: &str) -> bool {
    line.starts_with("go ") || count_go_statements(line) > 0
}

/// True when some `go` statement is lexically inside a `for` block.
fn go_in_loop(inner: &str) -> bool {
    let mut stack: Vec<bool> = Vec::new();
    for line in inner.lines() {
        let t = line.trim_start();
        let is_for = t.starts_with("for ") || t.starts_with("for{");
        if is_go_launch(t) && stack.iter().any(|&f| f) {
            return true;
        }
        for c in line.chars() {
            match c {
                '{' => stack.push(is_for),
                '}' => {
                    stack.pop();
                }
                _ => {}
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::checker::Language;

    use super::super::test_support::functions;
    use super::*;

    const FIXTURE: &str =
        include_str!("../../testdata/test-quality/go/oracle/oracle_cases_test.go");

    /// Same-file helper lookup for the fixture: text from `func name(` to the next column-0 `}`.
    fn fixture_helper(name: &str) -> Option<String> {
        let start = FIXTURE.find(&format!("func {name}("))?;
        let rest = &FIXTURE[start..];
        Some(rest[..rest.find("\n}\n")? + 3].to_string())
    }

    fn labels() -> BTreeMap<String, Option<&'static str>> {
        functions(Language::Go, FIXTURE)
            .iter()
            .map(|f| {
                (
                    f.name.clone(),
                    tag_go(f, &leading_comment(FIXTURE, f.line), &fixture_helper)
                        .map(OracleTag::label),
                )
            })
            .collect()
    }

    #[test]
    fn fixture_cases_get_the_expected_tag() {
        let expected: &[(&str, Option<&str>)] = &[
            ("TestCompile_interfaceAssertion", Some("compile-check")),
            ("TestCompile_compositeBlankAssign", Some("compile-check")),
            ("TestRace_waitGroupLoop", Some("race-smoke")),
            ("TestRace_namedConcurrent", Some("race-smoke")),
            ("TestLifecycle_startStop", None),
            ("TestAuditHook_nilLogger", None),
            ("TestAddCadvisorFlagsDoesNotPanic", None),
            ("TestEndpointSlice_handlers", None),
            ("TestDiscarded_nameNoDetection", None),
            ("TestDiscarded_returnsEmpty", None),
            ("TestStub_todoOnly", Some("empty-stub")),
            ("TestStub_empty", Some("empty-stub")),
            ("TestUntagged_logOnly", None),
            ("TestStub_delegatesToSkipHelper", Some("empty-stub")),
            (
                "TestStub_delegatesToSkipHelperWithOptions",
                Some("empty-stub"),
            ),
            ("TestStub_delegatesToEmptyHelper", Some("empty-stub")),
            ("TestStub_delegatesToTodoHelper", Some("empty-stub")),
            ("TestRace_waitGroupGoMethod", Some("race-smoke")),
            ("TestRace_bareGoNamedConcurrent", Some("race-smoke")),
            ("TestBareGo_noCue", None),
            ("TestLifecycle_runServerMultiShutdown", None),
            ("TestLifecycle_prefixedVerbs", None),
            ("TestStartStop_withSleep", None),
            ("TestStartStop_withSubjectCall", None),
            ("TestDelegates_toWorkingHelper", None),
            ("TestDelegates_twoLevels", None),
            ("TestDelegates_skipHelperPlusRealCall", None),
            ("TestCompile_blankDeclAfterSetup", Some("compile-check")),
            ("TestInterface_constructorDiscarded", Some("compile-check")),
            ("TestBuild_constructorDiscarded", None),
            ("TestFanOut_noDeadlock", None),
            ("TestDeadlockBw_closeAndReload", None),
            ("TestProduceConsume_pipeline", None),
            ("TestParallelLabels_commentOnly", Some("race-smoke")),
            ("TestProxyDataRace", Some("race-smoke")),
            ("TestTraceExporter_noGoroutines", None),
            ("TestStub_tableLoopToSkipHelper", Some("empty-stub")),
            ("TestDelegates_tableLoopToWorkingHelper", None),
        ];
        let got = labels();
        for (name, tag) in expected {
            assert_eq!(got.get(*name).copied(), Some(*tag), "{name}");
        }
    }

    #[test]
    fn dropped_classes_emit_no_tag() {
        // lifecycle-smoke, no-panic-smoke (cued and uncued), discarded-result and untagged
        // bodies in the fixture; none of them generalized across held-out rounds.
        let dropped = [
            "TestLifecycle_startStop",
            "TestLifecycle_runServerMultiShutdown",
            "TestLifecycle_prefixedVerbs",
            "TestAuditHook_nilLogger",
            "TestAddCadvisorFlagsDoesNotPanic",
            "TestEndpointSlice_handlers",
            "TestStartStop_withSleep",
            "TestDiscarded_nameNoDetection",
            "TestDiscarded_returnsEmpty",
        ];
        let got = labels();
        for name in dropped {
            assert_eq!(got.get(name).copied(), Some(None), "{name}");
        }
        let kept = ["compile-check", "race-smoke", "empty-stub"];
        assert!(got.values().flatten().all(|l| kept.contains(l)));
    }

    #[test]
    fn annotation_ends_with_parseable_tag() {
        assert!(
            OracleTag::RaceSmoke
                .annotation()
                .ends_with("[tag=race-smoke]")
        );
        assert!(
            OracleTag::EmptyStub
                .annotation()
                .ends_with("[tag=empty-stub]")
        );
    }

    #[test]
    fn scan_drops_comments_and_blanks_literals() {
        let s = scan("func T() {\n x := \"go func() {\" // go func\n}");
        assert!(!s.code.contains("go func"));
        assert!(s.comments.contains("go func"));
    }
}
