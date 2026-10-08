//! Parsing one comment line into a directive, or a precise malformed diagnosis.

use std::sync::LazyLock;

use regex::Regex;

use super::sanitize::{MAX_RULE_ID_CHARS, MAX_RULES_PER_DIRECTIVE};
use super::types::*;

const LEADERS: &[&str] = &["<!--", "///", "//!", "//", "/**", "/*", "#", "*"];

/// Comment text without its leader (`//`, `#`, `/*`, `*`, `<!--`) and trailer (`*/`, `-->`).
fn strip_comment_syntax(line: &str) -> &str {
    let mut text = line.trim();
    if let Some(leader) = LEADERS.iter().find(|l| text.starts_with(**l)) {
        text = &text[leader.len()..];
    }
    let text = text.trim();
    let text = text
        .strip_suffix("*/")
        .or_else(|| text.strip_suffix("-->"))
        .unwrap_or(text);
    text.trim()
}

/// Whether any line of the comment `text` begins (after its leader) with `kibitzer:`.
pub fn is_directive_comment(text: &str) -> bool {
    text.lines()
        .any(|l| begins_marker_family(strip_comment_syntax(l)))
}

static NEAR_MISS_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^kibitzer\s*:\s*(ignore|disable|allow|suppress|false[-_ ]positive)\b").unwrap()
});

static LATER_DIRECTIVE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\bkibitzer:ignore\s+[a-z0-9-]+(?:,[a-z0-9-]+)*\s+--\s+\S").unwrap()
});

static NEAR_MISS_ANYWHERE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"kibitzer\s*:\s*(?:ignore|disable|allow|suppress|false[-_ ]positive)\b").unwrap()
});

/// The near-miss marker as written on a source line, for echoing back in the repair message.
pub(crate) fn near_miss_text(line: &str) -> Option<&str> {
    NEAR_MISS_ANYWHERE_RE.find(line).map(|m| m.as_str())
}

/// A directive line as written, normalised for repair messages: the rule list (split on commas
/// and whitespace, rejoined with commas) and the reason text after the separator, if any.
/// Reads the raw source line because the parse result does not carry the text.
pub(crate) fn echo_parts(line: &str) -> (String, String) {
    let rest = line
        .find(MARKER)
        .map_or("", |i| line[i + MARKER.len()..].trim());
    let rest = rest
        .strip_suffix("*/")
        .or_else(|| rest.strip_suffix("-->"))
        .unwrap_or(rest)
        .trim();
    let (rules, reason) = match find_separator(rest) {
        Some(i) => (&rest[..i], rest[i + 2..].trim()),
        None => match rest.find(['\u{2014}', '\u{2013}']) {
            Some(i) => (
                &rest[..i],
                rest[i..]
                    .trim_start_matches(['\u{2014}', '\u{2013}'])
                    .trim(),
            ),
            None => (rest, ""),
        },
    };
    let rules = rules
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|r| !r.is_empty())
        .collect::<Vec<_>>()
        .join(",");
    (rules, reason.to_string())
}

/// Finds a standalone `--` token (whitespace or text edge on both sides).
fn find_separator(rest: &str) -> Option<usize> {
    rest.match_indices("--").map(|(i, _)| i).find(|&i| {
        let before_ok = rest[..i]
            .chars()
            .next_back()
            .is_none_or(char::is_whitespace);
        let after_ok = rest[i + 2..].chars().next().is_none_or(char::is_whitespace);
        before_ok && after_ok
    })
}

fn has_dash_separator(rest: &str) -> bool {
    rest.split_whitespace()
        .any(|t| t == "\u{2014}" || t == "\u{2013}")
}

/// `Ok(None)` means the rule text is a doc placeholder (`<rule>`), not a directive.
fn parse_rule_list(text: &str) -> Result<Option<Vec<RuleId>>, MalformedReason> {
    if text.contains(['<', '>']) {
        return Ok(None);
    }
    if text.is_empty() {
        return Err(MalformedReason::MissingRule);
    }
    if text.split(',').take(MAX_RULES_PER_DIRECTIVE + 1).count() > MAX_RULES_PER_DIRECTIVE {
        return Err(MalformedReason::TooManyRules);
    }
    let mut rules: Vec<RuleId> = Vec::new();
    for element in text.split(',') {
        let rule = RuleId::new(element).ok_or_else(|| {
            if element.is_empty() || element.contains(char::is_whitespace) {
                MalformedReason::BadRuleList
            } else if element.len() > MAX_RULE_ID_CHARS
                && element
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            {
                MalformedReason::RuleTooLong
            } else {
                MalformedReason::BadRuleChar
            }
        })?;
        if !rules.contains(&rule) {
            rules.push(rule);
        }
    }
    Ok(Some(rules))
}

/// Parses one comment line (leader and trailer included or not). The returned directive
/// carries placeholder positions; scanners place it with its real rows.
pub(crate) fn parse_comment_line(text: &str) -> DirectiveParse {
    let text = strip_comment_syntax(text);
    match text
        .strip_prefix(MARKER)
        .filter(|r| r.is_empty() || r.starts_with(char::is_whitespace))
    {
        Some(rest) => parse_after_marker(rest.trim()),
        None => classify_without_marker(text),
    }
}

/// A comment that does not start with the marker: a near miss, a late marker, or prose.
fn classify_without_marker(text: &str) -> DirectiveParse {
    if NEAR_MISS_RE.is_match(text) {
        DirectiveParse::Malformed(MalformedReason::NearMissMarker)
    } else if LATER_DIRECTIVE_RE.is_match(text) {
        DirectiveParse::Malformed(MalformedReason::NotAtCommentStart)
    } else {
        DirectiveParse::NotADirective
    }
}

/// The text after `kibitzer:ignore`: `<rules> -- <reason>`.
fn parse_after_marker(rest: &str) -> DirectiveParse {
    let Some(sep) = find_separator(rest) else {
        return parse_without_separator(rest);
    };
    match parse_rule_list(rest[..sep].trim()) {
        Ok(Some(rules)) => directive_with_reason(rules, &rest[sep + 2..]),
        Ok(None) => DirectiveParse::NotADirective,
        Err(reason) => DirectiveParse::Malformed(reason),
    }
}

fn parse_without_separator(rest: &str) -> DirectiveParse {
    if rest.is_empty() {
        return DirectiveParse::Malformed(MalformedReason::MissingRule);
    }
    if has_dash_separator(rest) {
        return DirectiveParse::Malformed(MalformedReason::EmDashSeparator);
    }
    match parse_rule_list(rest) {
        Ok(None) => DirectiveParse::NotADirective,
        _ => DirectiveParse::Malformed(MalformedReason::MissingReason),
    }
}

fn directive_with_reason(rules: Vec<RuleId>, reason_text: &str) -> DirectiveParse {
    use DirectiveParse::{Malformed, Valid};
    match Reason::new(reason_text, &rules) {
        Ok(reason) => {
            let one = Line::new(1);
            Directive::new(rules, reason, one, one, true)
                .map_or(Malformed(MalformedReason::MissingRule), Valid)
        }
        Err(ReasonError::Blank) => Malformed(MalformedReason::MissingReason),
        Err(ReasonError::TooLong) => Malformed(MalformedReason::ReasonTooLong),
        Err(ReasonError::Weak(kind)) => Malformed(MalformedReason::WeakReason(kind)),
    }
}
