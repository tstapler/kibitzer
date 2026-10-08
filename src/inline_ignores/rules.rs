//! Which rule a finding answers to, which checker owns a rule, and rule-name suggestions.

use std::sync::LazyLock;

use super::types::*;
use crate::checker::Finding;

/// Checkers whose leading `[x]` prefix is data (a link ref id), not a rule id.
const DYNAMIC_PREFIX_CHECKERS: &[&str] = &["markdown-link-integrity"];

/// The leading `[x]` of a finding message, if any.
fn bracket_prefix(message: &str) -> Option<&str> {
    let rest = message.strip_prefix('[')?;
    rest.split_once(']').map(|(rule, _)| rule)
}

/// Matching never consults a rule table, so a stale table cannot make an ignore fail closed.
pub(crate) fn rule_matches(directive_rule: &RuleId, checker_name: &str, message: &str) -> bool {
    if is_meta_rule(directive_rule.as_str()) {
        return false;
    }
    if directive_rule.as_str() == checker_name {
        return true;
    }
    !DYNAMIC_PREFIX_CHECKERS.contains(&checker_name)
        && bracket_prefix(message) == Some(directive_rule.as_str())
}

/// The rule a finding answers to: its leading `[x]` prefix, or the checker name when it has
/// none or the prefix is data (see `DYNAMIC_PREFIX_CHECKERS`).
pub(crate) fn anchor_rule(checker_name: &str, finding: &Finding) -> RuleId {
    let by_name = || RuleId::from_checker_name(checker_name);
    if DYNAMIC_PREFIX_CHECKERS.contains(&checker_name) {
        return by_name();
    }
    bracket_prefix(&finding.message)
        .and_then(RuleId::new)
        .unwrap_or_else(by_name)
}

/// How a checker name is recognised as the producer of a rule's findings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OwnerMatch {
    Prefix(&'static str),
    Suffix(&'static str),
    /// A checker named exactly like the rule, resolved through the checker registry.
    SameName,
    /// No per-file checker owns it, so it is judged from first-pass data only.
    Unowned,
}

pub(crate) struct RuleInfo {
    pub id: &'static str,
    pub owner: OwnerMatch,
    /// Diagnostics about directives themselves; suppressing them would hide the repair prompt.
    pub meta: bool,
}

pub(super) const fn rule(id: &'static str, owner: OwnerMatch) -> RuleInfo {
    RuleInfo {
        id,
        owner,
        meta: false,
    }
}

const fn meta_rule(id: &'static str, owner: OwnerMatch) -> RuleInfo {
    RuleInfo {
        id,
        owner,
        meta: true,
    }
}

/// The per-language checker families that own several rule ids each.
const COMMENT_QUALITY: OwnerMatch = OwnerMatch::Prefix("comment-quality");
const SYNTAX_RULES: OwnerMatch = OwnerMatch::Prefix("syntax-rules");

/// The one rule table: ids that appear as `[id]` message prefixes, who owns them, and which
/// are meta. Advisory only: it feeds the unknown-rule suggestion and the unused-ignore rerun
/// and never decides suppression, so a stale entry costs a hint, not an ignore. A registered
/// checker absent from the table is still owned by name (see `owner_matches`).
/// `rules_should_CoverEveryStaticRulePrefix_When_DriftGuardScansCheckerSources` guards drift.
pub(crate) const RULES: &[RuleInfo] = &[
    rule("commented-out-code", COMMENT_QUALITY),
    rule("over-commented", COMMENT_QUALITY),
    rule("verbose-comment", COMMENT_QUALITY),
    rule("long-function", SYNTAX_RULES),
    rule("deep-nesting", SYNTAX_RULES),
    rule("long-parameter-list", SYNTAX_RULES),
    rule("flag-argument", SYNTAX_RULES),
    rule("unreachable-code", SYNTAX_RULES),
    rule("replace-magic-literal", SYNTAX_RULES),
    rule("extract-variable", SYNTAX_RULES),
    rule("file-size", OwnerMatch::Suffix("file-size")),
    rule("single-call-site-delegation", OwnerMatch::Unowned),
    rule("god-class", OwnerMatch::Unowned),
    rule("isp-fat-interface", OwnerMatch::Unowned),
    rule("unreferenced-private-symbol", OwnerMatch::Unowned),
    rule("go-type-switch-density", OwnerMatch::SameName),
    rule("go-encapsulate-collection", OwnerMatch::SameName),
    meta_rule("inline-ignore", OwnerMatch::SameName),
    meta_rule("ignore-syntax", OwnerMatch::Unowned),
    meta_rule("unused-ignore", OwnerMatch::Unowned),
    meta_rule("ignore-volume", OwnerMatch::Unowned),
    meta_rule("blocking-suppressed", OwnerMatch::Unowned),
];

fn rule_info(rule: &str) -> Option<&'static RuleInfo> {
    RULES.iter().find(|r| r.id == rule)
}

fn is_meta_rule(rule: &str) -> bool {
    rule_info(rule).is_some_and(|r| r.meta)
}

/// The non-meta `RULES` ids plus every registered checker name, built once (the registry is fixed per process).
static SUGGESTION_NAMES: LazyLock<Vec<String>> = LazyLock::new(|| {
    let mut names: Vec<String> = RULES
        .iter()
        .filter(|r| !r.meta)
        .map(|r| r.id.to_string())
        .collect();
    names.extend(
        crate::checker::registry()
            .iter()
            .map(|c| c.name().to_string()),
    );
    names.sort();
    names.dedup();
    names
});

/// Whether `rule` names something kibitzer knows. Meta rules count as known so they are not
/// reported as typos, though they never suppress.
pub(crate) fn known_rule(rule: &str) -> bool {
    is_meta_rule(rule) || SUGGESTION_NAMES.iter().any(|n| n == rule)
}

pub(super) fn levenshtein(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut diag = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let up = row[j + 1];
            row[j + 1] = (diag + usize::from(ca != *cb)).min(up + 1).min(row[j] + 1);
            diag = up;
        }
    }
    row[b.len()]
}

const MAX_SUGGESTION_DISTANCE: usize = 2;

const MIN_PREFIX_SUGGESTION_LEN: usize = 4;

/// The closest known name within edit distance 2, or one the unknown text is a prefix of
/// (`flag-arg` for `flag-argument`; truncation is too far away for edit distance alone).
pub(crate) fn did_you_mean(unknown: &str) -> Option<&'static str> {
    if unknown.len() > super::sanitize::MAX_RULE_ID_CHARS {
        return None;
    }
    SUGGESTION_NAMES
        .iter()
        .filter_map(|name| {
            let prefix = unknown.len() >= MIN_PREFIX_SUGGESTION_LEN && name.starts_with(unknown);
            let score = if prefix {
                1
            } else {
                levenshtein(unknown, name)
            };
            (score <= MAX_SUGGESTION_DISTANCE && name != unknown).then_some((score, name))
        })
        .min_by_key(|(score, name)| (*score, name.len()))
        .map(|(_, name)| name.as_str())
}

/// Whether `checker` produces `rule`'s findings. `None` means ownership is unknown, so the
/// rule is not judged by rerun.
pub(super) fn owner_matches(rule: &str, checker: &str) -> Option<bool> {
    let same_name = || crate::checker::lookup(rule).map(|_| checker == rule);
    match rule_info(rule).map(|r| r.owner) {
        Some(OwnerMatch::Prefix(p)) => Some(checker.starts_with(p)),
        Some(OwnerMatch::Suffix(p)) => Some(checker.ends_with(p)),
        Some(OwnerMatch::Unowned) => None,
        Some(OwnerMatch::SameName) | None => same_name(),
    }
}

/// Whether any checker could own `rule` by the table; `false` means "judge from first-pass data".
pub(crate) fn has_owner(rule: &str) -> bool {
    owner_matches(rule, "").is_some()
}

/// Owner check against a check name, for choosing which checks to rerun.
pub(crate) fn owned_by(rule: &str, check_name: &str) -> bool {
    owner_matches(rule, check_name) == Some(true)
}

impl Directive {
    /// Whether this directive's `rule` silences a finding of `checker_name`/`message` on `line`.
    /// The one definition shared by the apply path and the unused-ignore judgement, so the
    /// two cannot disagree. A whole-line directive also covers the row below its last row; a
    /// trailing one only its own rows; a file-scope rule is also covered from the file head.
    pub(crate) fn covers(
        &self,
        rule: &RuleId,
        checker_name: &str,
        message: &str,
        line: Line,
    ) -> bool {
        if !rule_matches(rule, checker_name, message) {
            return false;
        }
        let on_row = (self.start_line <= line && line <= self.end_line)
            || (self.whole_line && line.get() == self.end_line.get() + 1);
        let in_head =
            self.start_line.get() <= FILE_HEAD_LINES && FILE_SCOPE_RULES.contains(&rule.as_str());
        on_row || in_head
    }
}
