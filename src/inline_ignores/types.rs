//! Directive data model: lines, rule ids, reasons, directives, and the outcomes a check carries.

use std::num::NonZeroUsize;

use serde::{Deserialize, Serialize};

use crate::config::Severity;

pub(crate) const MARKER: &str = "kibitzer:ignore";

/// The word every directive-like comment starts with (`kibitzer:ignore`, `kibitzer:disable`, ...).
pub(crate) const MARKER_PREFIX: &str = "kibitzer";

/// Cheap pre-check: a source without the marker word cannot hold a directive, so nothing is parsed.
pub(crate) fn has_marker(source: &str) -> bool {
    source.contains(MARKER_PREFIX)
}

/// Whether `text` begins `kibitzer:`, the start of any directive-family comment.
pub(crate) fn begins_marker_family(text: &str) -> bool {
    text.strip_prefix(MARKER_PREFIX)
        .is_some_and(|rest| rest.starts_with(':'))
}

/// 1-based line number; tree-sitter's 0-based rows are converted once at the scan boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Line(NonZeroUsize);

impl Line {
    /// Line 0 (a checker that reports "no line") is treated as line 1.
    pub fn new(one_based: usize) -> Self {
        Line(NonZeroUsize::new(one_based).unwrap_or(NonZeroUsize::MIN))
    }

    pub fn get(self) -> usize {
        self.0.get()
    }
}

/// A rule a directive names; `[a-z0-9-]+`, so a doc placeholder like `<rule>` is rejected.
/// Validated by `new`, and by `Deserialize` through it, so a corrupt cache value is rejected.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct RuleId(String);

fn is_rule_char(c: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'
}

impl RuleId {
    /// A checker's registered name used as its own rule id. A name outside `[a-z0-9-]+` is
    /// sanitised so the id still round-trips through `Deserialize` (a cache with one that
    /// does not is discarded whole).
    pub(super) fn from_checker_name(name: &str) -> Self {
        RuleId::new(name).unwrap_or_else(|| {
            let sanitised: String = name
                .to_ascii_lowercase()
                .chars()
                .map(|c| if is_rule_char(c) { c } else { '-' })
                .collect();
            RuleId::new(&sanitised).unwrap_or_else(|| RuleId("checker".to_string()))
        })
    }

    pub fn new(text: &str) -> Option<Self> {
        (!text.is_empty() && text.chars().all(is_rule_char)).then(|| RuleId(text.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for RuleId {
    type Error = String;

    fn try_from(text: String) -> Result<Self, String> {
        RuleId::new(&text).ok_or_else(|| format!("invalid rule id {text:?}: expected [a-z0-9-]+"))
    }
}

impl From<RuleId> for String {
    fn from(rule: RuleId) -> String {
        rule.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeakReason {
    TooShort,
    RuleEcho,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReasonError {
    Blank,
    Weak(WeakReason),
}

/// The justification after ` -- `. The constructor enforces a mechanical floor: not blank,
/// not an echo of a listed rule id, and more than one word. It stops `-- needed`, not a
/// determined two-word lie.
/// Validated by `new`, and by `Deserialize` through it (without the rule-echo check, which
/// needs the directive's rules), as for `RuleId`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Reason(String);

fn fold_for_echo(text: &str) -> String {
    text.to_lowercase()
        .split(|c: char| c == '-' || c == '_' || c.is_whitespace())
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

impl Reason {
    pub fn new(text: &str, rules: &[RuleId]) -> Result<Self, ReasonError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(ReasonError::Blank);
        }
        let folded = fold_for_echo(text);
        if rules.iter().any(|r| fold_for_echo(r.as_str()) == folded) {
            return Err(ReasonError::Weak(WeakReason::RuleEcho));
        }
        if text.split_whitespace().count() < 2 {
            return Err(ReasonError::Weak(WeakReason::TooShort));
        }
        Ok(Reason(text.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for Reason {
    type Error = String;

    fn try_from(text: String) -> Result<Self, String> {
        Reason::new(&text, &[]).map_err(|err| format!("invalid reason {text:?}: {err:?}"))
    }
}

impl From<Reason> for String {
    fn from(reason: Reason) -> String {
        reason.0
    }
}

/// One parsed, well-formed ignore comment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Directive {
    rules: Vec<RuleId>,
    pub reason: Reason,
    pub start_line: Line,
    pub end_line: Line,
    /// No code precedes the comment on its start row.
    pub whole_line: bool,
}

impl Directive {
    /// `None` when `rules` is empty: a directive that names nothing is not representable.
    pub fn new(
        rules: Vec<RuleId>,
        reason: Reason,
        start_line: Line,
        end_line: Line,
        whole_line: bool,
    ) -> Option<Self> {
        (!rules.is_empty()).then_some(Directive {
            rules,
            reason,
            start_line,
            end_line,
            whole_line,
        })
    }

    pub fn rules(&self) -> &[RuleId] {
        &self.rules
    }

    pub(crate) fn span(&self) -> LineSpan {
        LineSpan {
            start: self.start_line,
            end: self.end_line,
        }
    }

    pub(super) fn placed(mut self, start: Line, end: Line, whole_line: bool) -> Self {
        self.start_line = start;
        self.end_line = end;
        self.whole_line = whole_line;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MalformedReason {
    MissingRule,
    MissingReason,
    WeakReason(WeakReason),
    /// `kibitzer: ignore`, `kibitzer:false-positive`, `kibitzer:allow`, and similar.
    NearMissMarker,
    NotAtCommentStart,
    EmDashSeparator,
    BadRuleList,
    /// A comma element that is not lowercase ASCII letters, digits, and `-` (uppercase, non-ASCII).
    BadRuleChar,
}

/// Result of parsing one comment line. A malformed directive never suppresses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DirectiveParse {
    Valid(Directive),
    Malformed(MalformedReason),
    NotADirective,
}

/// A parse result and the 1-based row of the comment line it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scanned {
    pub row: Line,
    pub parse: DirectiveParse,
}

/// The well-formed directives among `scanned`, in source order.
pub(crate) fn valid_directives(scanned: &[Scanned]) -> Vec<&Directive> {
    scanned
        .iter()
        .filter_map(|s| match &s.parse {
            DirectiveParse::Valid(d) => Some(d),
            _ => None,
        })
        .collect()
}

/// Rules whose anchor is not a statement: a directive in the file head also covers them.
pub(crate) const FILE_SCOPE_RULES: &[&str] = &["file-size", "file-complexity"];

pub(crate) const FILE_HEAD_LINES: usize = 10;

/// A finding a directive removed, kept so advisories and counters read one source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DroppedFinding {
    pub directive_start: Line,
    pub directive_end: Line,
    pub rule: RuleId,
    pub reason: Reason,
    pub finding_line: Line,
    pub severity: Severity,
}

/// A finding's rule and the 1-based line it is anchored to; what the hook footer points at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Anchor {
    pub rule: RuleId,
    pub line: Line,
}

/// Inline-ignore result carried on `CheckResult`. `shown` lists findings still visible after
/// scoping and `accepted/` (capped), `kept` every finding that survived inline filtering
/// (whole file, capped), `dropped` what directives removed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InlineOutcome {
    #[serde(default)]
    pub shown: Vec<Anchor>,
    #[serde(default)]
    pub kept: Vec<Anchor>,
    #[serde(default)]
    pub dropped: Vec<DroppedFinding>,
}

impl DroppedFinding {
    pub(crate) fn directive_span(&self) -> LineSpan {
        LineSpan {
            start: self.directive_start,
            end: self.directive_end,
        }
    }
}

impl InlineOutcome {
    pub fn first_anchor(&self) -> Option<(&RuleId, Line)> {
        self.shown.first().map(|a| (&a.rule, a.line))
    }

    /// Distinct rule ids of the shown findings, in finding order.
    pub fn rule_ids(&self) -> Vec<&RuleId> {
        let mut ids: Vec<&RuleId> = Vec::new();
        for anchor in &self.shown {
            if !ids.contains(&&anchor.rule) {
                ids.push(&anchor.rule);
            }
        }
        ids
    }

    /// Distinct shown rule ids across several outcomes, in order.
    pub fn union_rule_ids<'a>(
        outcomes: impl IntoIterator<Item = &'a InlineOutcome>,
    ) -> Vec<&'a RuleId> {
        let mut ids: Vec<&RuleId> = Vec::new();
        for outcome in outcomes {
            for id in outcome.rule_ids() {
                if !ids.contains(&id) {
                    ids.push(id);
                }
            }
        }
        ids
    }
}

/// A finding as the checker reported it, before any directive was applied. Built from
/// structured findings, never parsed from rendered text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RawFinding {
    pub line: Line,
    pub checker: String,
    pub rule: RuleId,
    pub message: String,
}

/// The inclusive rows a directive comment occupies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LineSpan {
    pub start: Line,
    pub end: Line,
}

impl LineSpan {
    pub(crate) fn intersects(self, ranges: &[(usize, usize)]) -> bool {
        ranges
            .iter()
            .any(|&(s, e)| s <= self.end.get() && self.start.get() <= e)
    }
}

pub(crate) fn rows_intersect(d: &Directive, ranges: &[(usize, usize)]) -> bool {
    d.span().intersects(ranges)
}
