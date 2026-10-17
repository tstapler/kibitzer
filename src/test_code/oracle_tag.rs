//! Intentional-oracle tagger (prototype): classifies a Go test that has no assertion call by the
//! mechanical shape of its body. Annotates only; a tag never suppresses a finding (ADR-004).
//!
//! Tags are mutually exclusive and checked in a fixed order (see [`tag_go`]). Signals are text and
//! statement-shape only: no type information, so "is this call a conversion or a function call"
//! and "is this goroutine test checking liveness or a race" are not decidable here.

use std::sync::LazyLock;

use regex::Regex;

use super::TestFn;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OracleTag {
    CompileCheck,
    RaceSmoke,
    LifecycleSmoke,
    NoPanicSmoke { cued: bool },
    DiscardedResult,
    EmptyStub,
    Untagged,
}

impl OracleTag {
    pub fn label(self) -> &'static str {
        match self {
            Self::CompileCheck => "compile-check",
            Self::RaceSmoke => "race-smoke",
            Self::LifecycleSmoke => "lifecycle-smoke",
            Self::NoPanicSmoke { cued: true } => "no-panic-smoke",
            Self::NoPanicSmoke { cued: false } => "no-panic-smoke:uncued",
            Self::DiscardedResult => "discarded-result",
            Self::EmptyStub => "empty-stub",
            Self::Untagged => "untagged",
        }
    }

    /// The extra finding line; ends with `[tag=<label>]` so a run can be parsed back.
    pub fn annotation(self) -> String {
        let text = match self {
            Self::CompileCheck => "Smoke-only: compile-check (blank-identifier declarations only)",
            Self::RaceSmoke => {
                "Smoke-only: race (goroutines and sync primitives, results discarded)"
            }
            Self::LifecycleSmoke => "Smoke-only: lifecycle (start/stop calls, no result used)",
            Self::NoPanicSmoke { cued: true } => {
                "Smoke-only: no-panic (explicit cue in name or comment)"
            }
            Self::NoPanicSmoke { cued: false } => {
                "Smoke-only: no-panic, uncued (calls with no result used; lower confidence)"
            }
            Self::DiscardedResult => {
                "Likely defect: the name promises behaviour but the result is computed and dropped"
            }
            Self::EmptyStub => "Stub: empty body or TODO only",
            Self::Untagged => "No mechanical intent signal found",
        };
        format!("{text} [tag={}]", self.label())
    }
}

/// Env flag read by the checker to append [`OracleTag::annotation`] to Go findings.
pub const ENV_FLAG: &str = "KIBITZER_ORACLE_TAGS";

pub fn enabled() -> bool {
    std::env::var(ENV_FLAG).is_ok_and(|v| v == "1")
}

static NO_PANIC_CUE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(not|n't|no|never)\s+(cause\s+|raise\s+)?(a\s+)?(panic|crash)|(did|does|do|must|should)\s*not\s+(panic|crash)|smoke",
    )
    .expect("valid regex")
});
static NO_PANIC_NAME_CUE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(no|not)_?(panic|crash)").expect("valid regex"));
static RACE_CUE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)concurren|race|thread.?safe|atomic").expect("valid regex"));
static LIFECYCLE_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)start|stop|close|shutdown|lifecycle").expect("valid regex"));
static PROMISE_NAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"No[A-Z]|Returns|Empty|Error|[Ss]hould|When|Not[A-Z]").expect("valid regex")
});
static PANIC_ONLY_NAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)no_?panic|not_?panic|no_?crash|no_?(data_?)?race|race_?(free|safe)|panic")
        .expect("valid regex")
});
static SYNC_PRIMITIVE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"WaitGroup|\bchan\b|<-|\bMutex\b|\bRWMutex\b|errgroup").expect("valid regex")
});
static BLANK_ASSIGN_ANYWHERE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^\s*_\s*(,\s*_\s*)?:?=[^=]").expect("valid regex"));
static COMPILE_NOTE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)compile.?time|compiles").expect("valid regex"));
static COMPOSITE_RHS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^&?[A-Za-z_][\w.\[\]*]*\s*\{").expect("valid regex"));

const LIFECYCLE_CALLS: &[&str] = &[
    "Start", "Stop", "Close", "Run", "Shutdown", "Init", "Open", "Cancel", "Wait", "Begin", "End",
];
const SETUP_PREFIXES: &[&str] = &["New", "new", "With", "Make", "make", "Default"];

/// What a top-level statement of the test body is, as far as the tagger cares.
#[derive(Debug, PartialEq)]
enum Stmt {
    BlankDecl,
    BlankAssign(String),
    Go,
    Defer,
    Control,
    Recv,
    Bind { setup: bool },
    Call(String),
    Other,
}

struct Scanned {
    /// Body with comments removed and literal contents blanked.
    code: String,
    /// Comment text, concatenated.
    comments: String,
}

pub fn tag_go(f: &TestFn) -> OracleTag {
    let scanned = scan(&f.body);
    let inner = inner_body(&scanned.code);
    let stmts: Vec<Stmt> = split_statements(inner)
        .iter()
        .map(|s| classify(s))
        .collect();

    if inner.trim().is_empty() {
        return OracleTag::EmptyStub;
    }
    if is_compile_check(&stmts) || is_compile_note(&stmts, &scanned.comments) {
        return OracleTag::CompileCheck;
    }
    if is_race_smoke(&scanned.code, inner, &f.name, &scanned.comments) {
        return OracleTag::RaceSmoke;
    }
    let has_blank_assign = BLANK_ASSIGN_ANYWHERE.is_match(inner);
    let promises = PROMISE_NAME.is_match(&f.name) && !PANIC_ONLY_NAME.is_match(&f.name);
    if promises && has_blank_assign {
        return OracleTag::DiscardedResult;
    }
    let cued = NO_PANIC_CUE.is_match(&scanned.comments) || NO_PANIC_NAME_CUE.is_match(&f.name);
    if is_lifecycle_only(&stmts, inner) && (LIFECYCLE_NAME.is_match(&f.name) || !cued) {
        return OracleTag::LifecycleSmoke;
    }
    if cued {
        return OracleTag::NoPanicSmoke { cued: true };
    }
    if is_uncued_no_panic(&stmts) {
        return OracleTag::NoPanicSmoke { cued: false };
    }
    OracleTag::Untagged
}

fn is_compile_check(stmts: &[Stmt]) -> bool {
    let relevant: Vec<&Stmt> = stmts
        .iter()
        .filter(|s| !matches!(s, Stmt::Call(c) if c.starts_with("t.")))
        .collect();
    !relevant.is_empty()
        && relevant.iter().all(|s| match s {
            Stmt::BlankDecl => true,
            Stmt::BlankAssign(rhs) => {
                let rhs = rhs.trim();
                rhs.starts_with('&') || rhs.contains("(nil)") || COMPOSITE_RHS.is_match(rhs)
            }
            _ => false,
        })
}

/// A body that is only `t.Log`-style calls next to a comment saying the real check is the compiler.
fn is_compile_note(stmts: &[Stmt], comments: &str) -> bool {
    !stmts.is_empty()
        && stmts
            .iter()
            .all(|s| matches!(s, Stmt::Call(c) if c.starts_with("t.")))
        && COMPILE_NOTE.is_match(comments)
}

fn is_race_smoke(code: &str, inner: &str, name: &str, comments: &str) -> bool {
    let go_count = count_go_statements(inner);
    if go_count == 0 || !SYNC_PRIMITIVE.is_match(code) {
        return false;
    }
    let cue = RACE_CUE.is_match(name) || RACE_CUE.is_match(comments);
    cue || go_count >= 2 || go_in_loop(inner)
}

fn is_lifecycle_only(stmts: &[Stmt], inner: &str) -> bool {
    if count_go_statements(inner) > 0 {
        return false;
    }
    let mut lifecycle_calls = 0;
    for s in stmts {
        match s {
            Stmt::Call(c) => {
                let last = last_segment(c);
                if LIFECYCLE_CALLS.contains(&last) {
                    lifecycle_calls += 1;
                } else if !(c.starts_with("t.")
                    || c.starts_with("time.Sleep")
                    || last.starts_with("Set")
                    || last == "cancel")
                {
                    return false;
                }
            }
            Stmt::Bind { setup: true } | Stmt::Defer | Stmt::Recv => {}
            _ => return false,
        }
    }
    lifecycle_calls > 0
}

fn is_uncued_no_panic(stmts: &[Stmt]) -> bool {
    let mut subject_calls = 0;
    for s in stmts {
        match s {
            Stmt::Call(c) if c.starts_with("t.") => {}
            Stmt::Call(_) => subject_calls += 1,
            Stmt::BlankAssign(_) => subject_calls += 1,
            Stmt::Bind { setup: true } | Stmt::Defer => {}
            _ => return false,
        }
    }
    subject_calls > 0
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

fn is_setup_rhs(rhs: &str) -> bool {
    let rhs = rhs.trim();
    if rhs.starts_with('&')
        || rhs.starts_with("[]")
        || rhs.starts_with("map[")
        || COMPOSITE_RHS.is_match(rhs)
    {
        return true;
    }
    let Some(paren) = rhs.find('(') else {
        return true;
    };
    let callee = rhs[..paren].trim();
    callee.starts_with("t.")
        || callee.starts_with("context.")
        || callee.starts_with("time.")
        || SETUP_PREFIXES
            .iter()
            .any(|p| last_segment(callee).starts_with(p))
}

fn classify(s: &str) -> Stmt {
    let word = first_word(s);
    if s.starts_with("var _ ") || s.starts_with("var _\t") {
        return Stmt::BlankDecl;
    }
    match word {
        "go" => return Stmt::Go,
        "defer" => return Stmt::Defer,
        "for" | "if" | "switch" | "select" | "return" | "func" | "var" | "const" | "break"
        | "continue" => return Stmt::Control,
        _ => {}
    }
    if s.starts_with("<-") {
        return Stmt::Recv;
    }
    if let Some((at, len)) = find_assign(s) {
        let lhs = s[..at].trim();
        let rhs = s[at + len..].trim();
        if lhs.split(',').all(|p| p.trim() == "_") {
            return Stmt::BlankAssign(rhs.to_string());
        }
        return Stmt::Bind {
            setup: is_setup_rhs(rhs) || lhs.contains('.'),
        };
    }
    if s.ends_with(')')
        && let Some(paren) = s.find('(')
    {
        return Stmt::Call(s[..paren].trim().to_string());
    }
    Stmt::Other
}

/// `go` statements at any nesting depth (goroutines are usually launched inside loops).
fn count_go_statements(inner: &str) -> usize {
    static GO: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?m)(^|[{;\s])go\s+(func\b|[A-Za-z_][\w.]*\()").expect("valid regex")
    });
    GO.find_iter(inner).count()
}

/// True when some `go` statement is lexically inside a `for` block.
fn go_in_loop(inner: &str) -> bool {
    let mut stack: Vec<bool> = Vec::new();
    for line in inner.lines() {
        let t = line.trim_start();
        let is_for = t.starts_with("for ") || t.starts_with("for{");
        if t.starts_with("go ") && stack.iter().any(|&f| f) {
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

    fn labels() -> BTreeMap<String, &'static str> {
        functions(Language::Go, FIXTURE)
            .iter()
            .map(|f| (f.name.clone(), tag_go(f).label()))
            .collect()
    }

    #[test]
    fn fixture_cases_get_the_expected_tag() {
        let expected: &[(&str, &str)] = &[
            ("TestCompile_interfaceAssertion", "compile-check"),
            ("TestCompile_compositeBlankAssign", "compile-check"),
            ("TestRace_waitGroupLoop", "race-smoke"),
            ("TestRace_namedConcurrent", "race-smoke"),
            ("TestLifecycle_startStop", "lifecycle-smoke"),
            ("TestAuditHook_nilLogger", "no-panic-smoke"),
            ("TestAddCadvisorFlagsDoesNotPanic", "no-panic-smoke"),
            ("TestEndpointSlice_handlers", "no-panic-smoke:uncued"),
            ("TestDiscarded_nameNoDetection", "discarded-result"),
            ("TestDiscarded_returnsEmpty", "discarded-result"),
            ("TestStub_todoOnly", "empty-stub"),
            ("TestStub_empty", "empty-stub"),
            ("TestUntagged_logOnly", "untagged"),
        ];
        let got = labels();
        for (name, tag) in expected {
            assert_eq!(got.get(*name).copied(), Some(*tag), "{name}");
        }
    }

    #[test]
    fn annotation_ends_with_parseable_tag() {
        assert!(
            OracleTag::RaceSmoke
                .annotation()
                .ends_with("[tag=race-smoke]")
        );
        assert!(OracleTag::Untagged.annotation().ends_with("[tag=untagged]"));
    }

    #[test]
    fn scan_drops_comments_and_blanks_literals() {
        let s = scan("func T() {\n x := \"go func() {\" // go func\n}");
        assert!(!s.code.contains("go func"));
        assert!(s.comments.contains("go func"));
    }
}
