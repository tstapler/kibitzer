//! The advisory hook footer: dismissal links plus the inline-ignore syntax hint, shed down
//! to a fixed character budget. Paid on every failing PostToolUse call, hence the ceiling.

use crate::inline_ignores::{HINT_RULE_LIMIT, Line, RuleId, syntax_hint, syntax_hint_limited};

const FALSE_POSITIVE_SENTENCE: &str = "If any of the above looks like a false positive (fired on content the edit \
     didn't actually introduce, or on a pattern the check misidentifies), see \
     https://github.com/tstapler/kibitzer/blob/master/docs/reporting-false-positives.md \
     for how to file it, don't just note it in passing.";
const TURN_OFF_SENTENCE: &str = "To turn a check off (repo-wide) or exclude a specific file, see \
     https://github.com/tstapler/kibitzer/blob/master/docs/suppressing-checks.md.";
const TURN_OFF_SHORT_SENTENCE: &str = "To turn a check off or exclude a file, see \
     https://github.com/tstapler/kibitzer/blob/master/docs/suppressing-checks.md.";

/// Character ceiling for the footer; the shedding below keeps typical and longest-case
/// footers under it.
const MAX_FOOTER_CHARS: usize = 638;

/// What the footer has shed, in drop order. The syntax line and the anchor are never shed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Trim {
    Nothing,
    FalsePositiveLink,
    TurnOffProse,
    SuppressingChecksLink,
}

const TRIM_ORDER: [Trim; 4] = [
    Trim::Nothing,
    Trim::FalsePositiveLink,
    Trim::TurnOffProse,
    Trim::SuppressingChecksLink,
];

fn render_footer(trim: Trim, hint: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if trim == Trim::Nothing {
        parts.push(FALSE_POSITIVE_SENTENCE);
    }
    match trim {
        Trim::Nothing | Trim::FalsePositiveLink => parts.push(TURN_OFF_SENTENCE),
        Trim::TurnOffProse => parts.push(TURN_OFF_SHORT_SENTENCE),
        Trim::SuppressingChecksLink => {}
    }
    if !hint.is_empty() {
        parts.push(hint);
    }
    parts.join(" ")
}

/// Advisory footer: links plus the syntax hint for `path`, within `max_chars`. Without an
/// `anchor` (no shown finding a directive could suppress) the hint is left out entirely. Sheds prose in
/// `TRIM_ORDER`, then truncates the `Rules:` list down to one id (never to zero). Only if
/// `max_chars` is below that irreducible floor does it return over budget (never a panic: a
/// hook must not die over footer length).
fn advisory_footer_within(
    path: &std::path::Path,
    anchor: Option<(&RuleId, Line)>,
    rule_ids: &[&RuleId],
    max_chars: usize,
) -> String {
    let full_hint = anchor
        .map(|_| syntax_hint(path, anchor, rule_ids))
        .unwrap_or_default();
    for trim in TRIM_ORDER {
        let footer = render_footer(trim, &full_hint);
        if footer.chars().count() <= max_chars {
            return footer;
        }
    }
    let last = TRIM_ORDER[TRIM_ORDER.len() - 1];
    let mut footer = render_footer(last, &full_hint);
    if anchor.is_none() {
        return footer;
    }
    for limit in (1..rule_ids.len().min(HINT_RULE_LIMIT)).rev() {
        footer = render_footer(last, &syntax_hint_limited(path, anchor, rule_ids, limit));
        if footer.chars().count() <= max_chars {
            break;
        }
    }
    footer
}

pub(crate) fn advisory_footer(
    path: &std::path::Path,
    anchor: Option<(&RuleId, Line)>,
    rule_ids: &[&RuleId],
) -> String {
    advisory_footer_within(path, anchor, rule_ids, MAX_FOOTER_CHARS)
}

#[cfg(test)]
#[allow(non_snake_case)]
mod footer_tests {
    use super::*;
    use crate::inline_ignores::{DirectiveParse, parse_comment_line};
    use std::path::Path;

    const BASELINE_FOOTER_CHARS: usize = 423;
    const FP_LINK: &str = "reporting-false-positives.md";
    const SUPPRESS_LINK: &str = "suppressing-checks.md";

    fn rid(name: &str) -> RuleId {
        RuleId::new(name).unwrap()
    }

    fn chars(text: &str) -> usize {
        text.chars().count()
    }

    /// The pre-hint footer, byte for byte as shipped (em dash included), pins the baseline.
    #[test]
    fn hook_footer_should_Measure423Characters_When_BeforeSyntaxHint() {
        let legacy = format!(
            "{} {TURN_OFF_SENTENCE}",
            FALSE_POSITIVE_SENTENCE.replace("it, don't", "it \u{2014} don't")
        );
        assert_eq!(chars(&legacy), BASELINE_FOOTER_CHARS);
        assert_eq!(BASELINE_FOOTER_CHARS, 423);
    }

    #[test]
    fn hook_footer_should_EndWithSyntaxAndAnchor_When_AnchorKnown() {
        let rule = rid("flag-argument");
        let footer = advisory_footer(
            Path::new("src/foo.go"),
            Some((&rule, Line::new(20))),
            &[&rule],
        );
        assert!(footer.ends_with("Rules: flag-argument."), "{footer}");
        assert!(
            footer.contains("// kibitzer:ignore flag-argument -- <why>"),
            "{footer}"
        );
        assert!(footer.contains("above line 20"), "{footer}");
        assert!(
            footer.contains(FP_LINK) && footer.contains(SUPPRESS_LINK),
            "{footer}"
        );
        assert!(!footer.contains("false-positive "), "{footer}");
    }

    #[test]
    fn hook_footer_should_OmitDirectiveHint_When_NoSuppressibleAnchor() {
        let footer = advisory_footer(Path::new("x.py"), None, &[]);
        assert!(!footer.contains("kibitzer:ignore"), "{footer}");
        assert!(!footer.contains("Rules:"), "{footer}");
        assert!(footer.contains(SUPPRESS_LINK), "{footer}");
    }

    fn longest_case() -> (Vec<RuleId>, String) {
        let ids: Vec<RuleId> = [
            "markdown-link-integrity",
            "duplicate-code-cross-file",
            "primitive-obsession",
            "repetitive-sentence-structure",
            "missing-paragraph-break",
            "comment-quality-markdown",
            "extra-rule",
        ]
        .iter()
        .map(|n| rid(n))
        .collect();
        let refs: Vec<&RuleId> = ids.iter().collect();
        let footer = advisory_footer(
            Path::new("docs/a/very/long/path/to/some-notes-file.md"),
            Some((&ids[0], Line::new(1234))),
            &refs,
        );
        (ids, footer)
    }

    #[test]
    fn hook_footer_budget_should_Be638Characters_When_Pinned() {
        assert_eq!(MAX_FOOTER_CHARS, 638);
    }

    #[test]
    fn hook_footer_should_StayBounded_When_RuleIdIsAtTheMaxLength() {
        assert!(RuleId::new(&"a".repeat(65)).is_none());
        let long = rid(&"a".repeat(64));
        let footer = advisory_footer(
            Path::new("src/foo.go"),
            Some((&long, Line::new(3))),
            &[&long],
        );
        assert!(
            chars(&footer) <= MAX_FOOTER_CHARS,
            "{} chars",
            chars(&footer)
        );
        assert!(footer.contains("above line 3"), "{footer}");
    }

    #[test]
    fn hook_footer_should_StayWithinBudgetAndKeepAnchor_When_LongestCase() {
        let (_ids, footer) = longest_case();
        assert!(
            chars(&footer) <= MAX_FOOTER_CHARS,
            "{} chars: {footer}",
            chars(&footer)
        );
        assert!(footer.contains("above line 1234"), "{footer}");
        assert!(
            footer.contains("<!-- kibitzer:ignore markdown-link-integrity -- <why> -->"),
            "{footer}"
        );
        assert!(
            footer.contains("Rules: markdown-link-integrity"),
            "{footer}"
        );
    }

    #[test]
    fn hook_footer_should_StayWithinBudget_When_TypicalGoCase() {
        let a = rid("flag-argument");
        let b = rid("primitive-obsession");
        let footer = advisory_footer(
            Path::new("src/foo.go"),
            Some((&a, Line::new(20))),
            &[&a, &b],
        );
        assert!(
            chars(&footer) <= MAX_FOOTER_CHARS,
            "{} chars",
            chars(&footer)
        );
    }

    #[test]
    fn hook_footer_should_DropInOrder_When_BudgetExceeded() {
        let ids: Vec<RuleId> = (0..4).map(|i| rid(&format!("some-rule-{i}"))).collect();
        let refs: Vec<&RuleId> = ids.iter().collect();
        let anchor = Some((&ids[0], Line::new(7)));
        let path = Path::new("a.go");
        let hint = syntax_hint(path, anchor, &refs);
        let len_at = |trim| chars(&render_footer(trim, &hint));

        let at = |cap| advisory_footer_within(path, anchor, &refs, cap);
        let f = at(len_at(Trim::Nothing) - 1);
        assert!(
            !f.contains(FP_LINK) && f.contains("(repo-wide)") && f.contains(SUPPRESS_LINK),
            "{f}"
        );
        let f = at(len_at(Trim::FalsePositiveLink) - 1);
        assert!(
            !f.contains("(repo-wide)") && f.contains(SUPPRESS_LINK),
            "{f}"
        );
        let f = at(len_at(Trim::TurnOffProse) - 1);
        assert!(
            !f.contains(SUPPRESS_LINK)
                && f.contains("Rules: some-rule-0, some-rule-1, some-rule-2, some-rule-3."),
            "{f}"
        );
        let f = at(len_at(Trim::SuppressingChecksLink) - 1);
        assert!(
            f.contains("Rules: some-rule-0, some-rule-1, some-rule-2, ..."),
            "{f}"
        );
        let f = at(1);
        assert!(f.contains("Rules: some-rule-0, ..."), "{f}");
        assert!(
            f.contains("kibitzer:ignore some-rule-0 -- <why>") && f.contains("above line 7"),
            "{f}"
        );
    }

    /// Fills `<rule>`/`<why>` in the footer's example and strips nothing else: the parser
    /// drops the leader and trailer itself.
    fn example_line(footer: &str, open: &str) -> String {
        let start = footer.find(open).expect("example leader in footer");
        let rest = &footer[start..];
        let end = rest.find(", ").unwrap_or(rest.len());
        rest[..end].to_string()
    }

    #[test]
    fn hook_footer_example_should_ParseAsValidDirective_When_PlaceholdersFilled() {
        for (path, open) in [
            ("a.go", "//"),
            ("a.py", "#"),
            ("a.md", "<!--"),
            ("a.rs", "//"),
        ] {
            let rule = rid("flag-argument");
            let footer = advisory_footer(Path::new(path), Some((&rule, Line::new(9))), &[&rule]);
            let example = example_line(&footer, &format!("{open} kibitzer:ignore"))
                .replace("<rule>", "flag-argument")
                .replace("<why>", "legacy callers");
            match parse_comment_line(&example) {
                DirectiveParse::Valid(d) => {
                    assert_eq!(d.rules(), &[rid("flag-argument")], "{example}");
                }
                other => panic!("example {example:?} parsed as {other:?}"),
            }
        }
    }
}
