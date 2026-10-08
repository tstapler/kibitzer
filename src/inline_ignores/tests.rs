// Test names follow the validation plan's should_X_When_Y convention.
#![allow(non_snake_case)]
use super::apply::AppliedIgnores;
use super::rules::{RULES, has_owner, levenshtein, rule_matches};
use super::scan::{scan_code_comments, scan_directives_with_cache, scan_markdown};
use super::*;
use crate::checker::{Finding, GrammarCache, Language};
use crate::config::Severity;
use regex::Regex;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;

fn rules(names: &[&str]) -> Vec<RuleId> {
    names.iter().map(|n| RuleId::new(n).unwrap()).collect()
}

fn valid(text: &str) -> Directive {
    match parse_comment_line(text) {
        DirectiveParse::Valid(d) => d,
        other => panic!("expected Valid for {text:?}, got {other:?}"),
    }
}

fn malformed(text: &str) -> MalformedReason {
    match parse_comment_line(text) {
        DirectiveParse::Malformed(m) => m,
        other => panic!("expected Malformed for {text:?}, got {other:?}"),
    }
}

fn go_scan(source: &str) -> Vec<Scanned> {
    scan_directives(Path::new("x.go"), source)
}

#[test]
fn parse_comment_line_should_ReturnDirective_When_KibitzerIgnore() {
    let source = "package main\n\n\n\n\n\n// kibitzer:ignore flag-argument -- legacy API, callers pinned\nfunc f() {}\n";
    let scanned = go_scan(source);
    assert_eq!(scanned.len(), 1);
    let expected = Directive::new(
        rules(&["flag-argument"]),
        Reason::new("legacy API, callers pinned", &[]).unwrap(),
        Line::new(7),
        Line::new(7),
        true,
    )
    .unwrap();
    assert_eq!(
        scanned[0],
        Scanned {
            row: Line::new(7),
            parse: DirectiveParse::Valid(expected)
        }
    );
}

#[test]
fn parse_comment_line_should_ReturnAllRules_When_CommaList() {
    let d = valid("// kibitzer:ignore a,b -- legacy API, callers pinned");
    assert_eq!(d.rules(), rules(&["a", "b"]).as_slice());
    assert_eq!(d.reason.as_str(), "legacy API, callers pinned");
}

#[test]
fn parse_comment_line_should_ReturnValid_When_ReasonPresent() {
    let d = valid("//   kibitzer:ignore   a,b   --   legacy API, callers pinned   ");
    assert_eq!(d.reason.as_str(), "legacy API, callers pinned");
    assert_eq!(d.rules().len(), 2);
}

#[test]
fn rule_id_new_should_Reject_When_PlaceholderOrUppercase() {
    assert!(RuleId::new("<rule>").is_none());
    assert!(RuleId::new("Foo_Bar").is_none());
    assert!(RuleId::new("").is_none());
    assert!(RuleId::new("flag-argument").is_some());
}

#[test]
fn parse_comment_line_should_ReturnNotADirective_When_RustdocPlaceholder() {
    assert_eq!(
        parse_comment_line("/// kibitzer:ignore <rule> -- <why>"),
        DirectiveParse::NotADirective
    );
}

#[test]
fn parse_comment_line_should_ReturnMalformedMissingReason_When_NoDashDash() {
    assert_eq!(
        malformed("// kibitzer:ignore flag-argument"),
        MalformedReason::MissingReason
    );
}

#[test]
fn parse_comment_line_should_ReturnMalformedMissingReason_When_ReasonEmptyAfterSeparator() {
    assert_eq!(
        malformed("// kibitzer:ignore flag-argument -- "),
        MalformedReason::MissingReason
    );
    assert_eq!(
        malformed("// kibitzer:ignore flag-argument --"),
        MalformedReason::MissingReason
    );
}

#[test]
fn parse_comment_line_should_ReturnMalformedMissingRule_When_OnlyReason() {
    assert_eq!(
        malformed("// kibitzer:ignore -- because"),
        MalformedReason::MissingRule
    );
    assert_eq!(
        malformed("// kibitzer:ignore"),
        MalformedReason::MissingRule
    );
}

#[test]
fn reason_new_should_Reject_When_EmptyOrBlank() {
    assert_eq!(Reason::new("  ", &[]), Err(ReasonError::Blank));
    assert_eq!(Reason::new("", &[]), Err(ReasonError::Blank));
}

#[test]
fn reason_new_should_ClassifyWeakReasons_When_TooShortOrRuleEcho() {
    let r = rules(&["flag-argument"]);
    assert_eq!(
        Reason::new("needed", &r),
        Err(ReasonError::Weak(WeakReason::TooShort))
    );
    for echo in ["flag-argument", "Flag Argument", "FLAG_ARGUMENT"] {
        assert_eq!(
            Reason::new(echo, &r),
            Err(ReasonError::Weak(WeakReason::RuleEcho)),
            "{echo}"
        );
    }
    assert!(Reason::new("legacy API, callers pinned", &r).is_ok());
}

#[test]
fn parse_comment_line_should_ReturnWeakReason_When_NeededOrEcho() {
    assert_eq!(
        malformed("// kibitzer:ignore flag-argument -- needed"),
        MalformedReason::WeakReason(WeakReason::TooShort)
    );
    assert_eq!(
        malformed("// kibitzer:ignore flag-argument -- flag-argument"),
        MalformedReason::WeakReason(WeakReason::RuleEcho)
    );
    assert_eq!(
        malformed("// kibitzer:ignore flag-argument -- Flag Argument"),
        MalformedReason::WeakReason(WeakReason::RuleEcho)
    );
}

#[test]
fn parse_comment_line_should_ReturnMalformed_When_EmDashSeparator() {
    for dash in ["\u{2014}", "\u{2013}"] {
        let line = format!("// kibitzer:ignore flag-argument {dash} legacy API, callers pinned");
        assert_eq!(malformed(&line), MalformedReason::EmDashSeparator, "{dash}");
    }
}

#[test]
fn parse_comment_line_should_ReturnBadRuleChar_When_UppercaseOrNonAscii() {
    for bad in ["Flag-Argument", "r\u{00e8}gle"] {
        assert_eq!(
            malformed(&format!("// kibitzer:ignore {bad} -- legacy api pinned")),
            MalformedReason::BadRuleChar,
            "{bad}"
        );
    }
}

#[test]
fn parse_comment_line_should_HandleRuleListEdges_When_CommaVariants() {
    let ok = valid("// kibitzer:ignore a,b -- legacy API, callers pinned");
    assert_eq!(ok.rules(), rules(&["a", "b"]).as_slice());
    let dedup = valid("// kibitzer:ignore a,a -- legacy API, callers pinned");
    assert_eq!(dedup.rules(), rules(&["a"]).as_slice());
    for bad in [
        "a, b -- why not",
        "a,,b -- why not",
        "a, -- why not",
        ",a -- why not",
    ] {
        assert_eq!(
            malformed(&format!("// kibitzer:ignore {bad}")),
            MalformedReason::BadRuleList,
            "{bad}"
        );
    }
    assert_eq!(
        parse_comment_line("// kibitzer:ignore a,<rule> -- why not"),
        DirectiveParse::NotADirective
    );
}

#[test]
fn parse_comment_line_should_ReturnNearMiss_When_MarkerMisspelled() {
    for text in [
        "// kibitzer: ignore foo -- bar",
        "// kibitzer:allow foo -- bar",
        "# kibitzer:false-positive primitive-obsession -- id is opaque",
        "// kibitzer:false_positive foo -- bar",
    ] {
        assert_eq!(malformed(text), MalformedReason::NearMissMarker, "{text}");
    }
}

#[test]
fn parse_comment_line_should_ReturnNotADirective_When_ProseMentionsKibitzer() {
    for text in [
        "// see kibitzer: allow list in docs",
        "// the kibitzer:ignore syntax is documented",
    ] {
        assert_eq!(
            parse_comment_line(text),
            DirectiveParse::NotADirective,
            "{text}"
        );
    }
}

#[test]
fn parse_comment_line_should_ReturnNotAtCommentStart_When_DirectiveFollowsOtherText() {
    for text in [
        "// TODO kibitzer:ignore flag-argument -- legacy API, callers pinned",
        "// legacy: kibitzer:ignore flag-argument -- legacy API, callers pinned",
    ] {
        assert_eq!(
            malformed(text),
            MalformedReason::NotAtCommentStart,
            "{text}"
        );
    }
}

#[test]
fn parse_comment_line_should_StayNotADirective_When_LaterMentionLacksFullGrammar() {
    for text in [
        "// the kibitzer:ignore syntax is documented",
        "// see kibitzer:ignore <rule> -- <why>",
    ] {
        assert_eq!(
            parse_comment_line(text),
            DirectiveParse::NotADirective,
            "{text}"
        );
    }
}

#[test]
fn parse_comment_line_should_StripLeader_When_SlashDoc_Hash_BlockStar_HtmlOpen() {
    for text in [
        "// kibitzer:ignore a -- two words",
        "/// kibitzer:ignore a -- two words",
        "//! kibitzer:ignore a -- two words",
        "# kibitzer:ignore a -- two words",
        "/* kibitzer:ignore a -- two words */",
        "/** kibitzer:ignore a -- two words */",
        " * kibitzer:ignore a -- two words",
        "<!-- kibitzer:ignore a -- two words -->",
    ] {
        let d = valid(text);
        assert_eq!(d.reason.as_str(), "two words", "{text}");
    }
}

#[test]
fn parse_comment_line_should_Tolerate_When_CrlfAndTabs() {
    let d = valid("//\tkibitzer:ignore\ta,b\t--\ttwo words\r");
    assert_eq!(d.rules().len(), 2);
    assert_eq!(d.reason.as_str(), "two words");
}

#[test]
fn is_directive_comment_should_MatchAnyKibitzerLine_When_BlockOrLine() {
    assert!(is_directive_comment("// kibitzer:ignore a -- two words"));
    assert!(is_directive_comment(
        "/*\n * kibitzer:ignore a -- two words\n */"
    ));
    assert!(!is_directive_comment("// ordinary comment"));
}

fn fixture(lang: Language) -> (&'static str, &'static str) {
    let line = "kibitzer:ignore flag-argument -- legacy API, callers pinned";
    match lang {
        Language::Go => (
            "package main\n// kibitzer:ignore flag-argument -- legacy API, callers pinned\nfunc f() {}\n",
            line,
        ),
        Language::TypeScript | Language::Tsx | Language::JavaScript => (
            "// kibitzer:ignore flag-argument -- legacy API, callers pinned\nfunction f() {}\n",
            line,
        ),
        Language::Python => (
            "# kibitzer:ignore flag-argument -- legacy API, callers pinned\ndef f():\n    pass\n",
            line,
        ),
        Language::Java => (
            "// kibitzer:ignore flag-argument -- legacy API, callers pinned\nclass A {}\n",
            line,
        ),
        Language::Kotlin => (
            "// kibitzer:ignore flag-argument -- legacy API, callers pinned\nfun f() {}\n",
            line,
        ),
        Language::Rust => (
            "// kibitzer:ignore flag-argument -- legacy API, callers pinned\nfn f() {}\n",
            line,
        ),
    }
}

#[test]
fn scan_code_comments_should_ParseDirective_When_EachLanguageAllGrammar() {
    let cache = GrammarCache::new();
    for &lang in Language::ALL {
        let (source, _) = fixture(lang);
        let tree = cache.parse(lang, source).unwrap();
        let scanned = scan_code_comments(lang, &tree, source);
        assert_eq!(scanned.len(), 1, "{lang:?}: {scanned:?}");
        let DirectiveParse::Valid(d) = &scanned[0].parse else {
            panic!("{lang:?}: not valid: {scanned:?}");
        };
        assert_eq!(d.rules(), rules(&["flag-argument"]).as_slice(), "{lang:?}");
        assert!(d.whole_line, "{lang:?}");
    }
}

#[test]
fn scan_code_comments_should_ReturnNothing_When_MarkerInStringLiteral() {
    let source = "fn main() {\n    let s = \"// kibitzer:ignore x -- y\";\n}\n";
    assert!(scan_directives(Path::new("a.rs"), source).is_empty());
}

#[test]
fn scan_code_comments_should_RecordWholeLineFalse_When_CommentTrailsCode() {
    let source = "package main\nvar x = f() // kibitzer:ignore a -- two words\n";
    let scanned = go_scan(source);
    let DirectiveParse::Valid(d) = &scanned[0].parse else {
        panic!("{scanned:?}")
    };
    assert!(!d.whole_line);
    assert_eq!(d.start_line, Line::new(2));
}

#[test]
fn scan_code_comments_should_ReportBlockCommentLineRow_When_DirectiveOnInnerLine() {
    let source = format!(
        "{}/*\n * kibitzer:ignore long-method -- generated code\n */\nclass A {{}}\n",
        "\n".repeat(9)
    );
    let scanned = scan_directives(Path::new("A.java"), &source);
    assert_eq!(scanned.len(), 1, "{scanned:?}");
    let DirectiveParse::Valid(d) = &scanned[0].parse else {
        panic!("{scanned:?}")
    };
    assert_eq!(d.start_line, Line::new(11));
    assert_eq!(scanned[0].row, Line::new(11));
}

#[test]
fn scan_code_comments_should_ConvertRowsToOneBased_When_TreeSitterRowSix() {
    let source = format!(
        "{}// kibitzer:ignore a -- two words\nfn f() {{}}\n",
        "\n".repeat(6)
    );
    let scanned = scan_directives(Path::new("a.rs"), &source);
    let DirectiveParse::Valid(d) = &scanned[0].parse else {
        panic!("{scanned:?}")
    };
    assert_eq!(d.start_line, Line::new(7));
}

#[test]
fn scan_markdown_should_ParseDirective_When_HtmlCommentOutsideFence() {
    let source =
        "# T\n\ntext\n\n<!-- kibitzer:ignore em-dash-overuse -- quoted source -->\n\nmore\n";
    let scanned = scan_directives(Path::new("a.md"), source);
    assert_eq!(scanned.len(), 1, "{scanned:?}");
    let DirectiveParse::Valid(d) = &scanned[0].parse else {
        panic!("{scanned:?}")
    };
    assert_eq!(d.start_line, Line::new(5));
    assert_eq!(d.rules(), rules(&["em-dash-overuse"]).as_slice());
    assert!(d.whole_line);
}

#[test]
fn scan_markdown_should_ParseDirective_When_InlineCommentInTableRow() {
    let source = "| a | b |\n|---|---|\n| x | <!-- kibitzer:ignore a -- two words --> |\n";
    let scanned = scan_markdown(source);
    assert_eq!(scanned.len(), 1, "{scanned:?}");
    let DirectiveParse::Valid(d) = &scanned[0].parse else {
        panic!("{scanned:?}")
    };
    assert_eq!(d.start_line, Line::new(3));
    assert!(!d.whole_line);
}

#[test]
fn scan_directives_should_NotListMarker_When_InRustStringAndMarkdownFence() {
    let rust = "const S: &str = \"// kibitzer:ignore a -- two words\";\n";
    assert!(scan_directives(Path::new("a.rs"), rust).is_empty());
    let fenced = "```md\n<!-- kibitzer:ignore a -- two words -->\n```\n";
    assert!(scan_directives(Path::new("a.md"), fenced).is_empty());
    let indented = "text\n\n    <!-- kibitzer:ignore a -- two words -->\n";
    assert!(scan_directives(Path::new("a.md"), indented).is_empty());
}

#[test]
fn scan_leading_comments_should_ParseDirective_When_ShellHashComment() {
    let source = "#!/bin/sh\n# kibitzer:ignore file-size -- vendored script\necho hi\n";
    let scanned = scan_directives(Path::new("x.sh"), source);
    assert_eq!(scanned.len(), 1, "{scanned:?}");
    let DirectiveParse::Valid(d) = &scanned[0].parse else {
        panic!("{scanned:?}")
    };
    assert_eq!(d.start_line, Line::new(2));
    assert!(d.whole_line);
}

#[test]
fn scan_leading_comments_should_ReturnNothing_When_MarkerInsideEchoString() {
    let source = "echo \"# kibitzer:ignore a -- b\"\n";
    assert!(scan_directives(Path::new("x.sh"), source).is_empty());
}

#[test]
fn scan_directives_should_ConstructNoParser_When_SourceLacksKibitzer() {
    let source = "package main\n\nfunc f() {}\n".repeat(3_333);
    let cache = GrammarCache::new();
    let scanned = scan_directives_with_cache(&cache, Path::new("x.go"), &source);
    assert!(scanned.is_empty());
    assert!(format!("{cache:?}").contains("languages_cached: 0"));
}

fn covers_row(d: &Directive, row: usize, rule_match: bool) -> bool {
    let rule = &d.rules()[0];
    let checker = if rule_match {
        rule.as_str()
    } else {
        "other-checker"
    };
    d.covers(rule, checker, "", Line::new(row))
}

fn dir(rule_names: &[&str], start: usize, end: usize, whole_line: bool) -> Directive {
    Directive::new(
        rules(rule_names),
        Reason::new("legacy api pinned", &[]).unwrap(),
        Line::new(start),
        Line::new(end),
        whole_line,
    )
    .unwrap()
}

#[test]
fn covers_should_BeTrueOnlyAtNextRow_When_WholeLineDirective() {
    let d = dir(&["flag-argument"], 9, 9, true);
    assert!(covers_row(&d, 10, true));
    assert!(covers_row(&d, 9, true));
    assert!(!covers_row(&d, 11, true));
    assert!(!covers_row(&d, 10, false));
}

#[test]
fn covers_should_StayOnOwnRow_When_TrailingComment() {
    let d = dir(&["flag-argument"], 9, 9, false);
    assert!(covers_row(&d, 9, true));
    assert!(!covers_row(&d, 10, true));
}

#[test]
fn covers_should_SpanAllRows_When_BlockComment() {
    let d = dir(&["flag-argument"], 4, 6, true);
    assert!(covers_row(&d, 5, true));
    assert!(covers_row(&d, 7, true));
    assert!(!covers_row(&d, 8, true));
}

#[test]
fn covers_should_TreatLineZeroAsLineOne() {
    let d = dir(&["file-size"], 1, 1, true);
    assert!(covers_row(&d, 0, true));
}

#[test]
fn rule_matches_should_MatchCheckerNameOrBracketPrefix() {
    let r = |s: &str| RuleId::new(s).unwrap();
    assert!(rule_matches(
        &r("flag-argument"),
        "syntax-rules-go",
        "[flag-argument] x"
    ));
    assert!(rule_matches(
        &r("primitive-obsession"),
        "primitive-obsession",
        "plain text"
    ));
    assert!(!rule_matches(
        &r("other"),
        "syntax-rules-go",
        "[flag-argument] x"
    ));
    assert!(rule_matches(
        &r("some-new-rule"),
        "unknown-checker",
        "[some-new-rule] m"
    ));
}

#[test]
fn rule_matches_should_BeFalse_When_DynamicPrefixCheckerAndBracketMatch() {
    let r = |s: &str| RuleId::new(s).unwrap();
    let msg = "[foo] used but never defined";
    assert!(!rule_matches(&r("foo"), "markdown-link-integrity", msg));
    assert!(rule_matches(
        &r("markdown-link-integrity"),
        "markdown-link-integrity",
        msg
    ));
}

#[test]
fn rule_matches_should_BeFalse_When_MetaRule() {
    let r = |s: &str| RuleId::new(s).unwrap();
    assert!(!rule_matches(
        &r("ignore-syntax"),
        "inline-ignore",
        "[ignore-syntax] bad"
    ));
    assert!(!rule_matches(&r("unused-ignore"), "unused-ignore", "x"));
    assert!(!rule_matches(
        &r("blocking-suppressed"),
        "inline-ignore",
        "[blocking-suppressed] x"
    ));
}

fn finding(line: usize, message: &str) -> Finding {
    Finding {
        line,
        message: message.to_string(),
    }
}

const GO_IGNORE_ABOVE_10: &str = "package main\n\n\n\n\n\n\n\n// kibitzer:ignore flag-argument -- legacy api pinned\nfunc f(b bool) {}\n";

fn apply(
    findings: Vec<Finding>,
    source: &str,
    checker: &str,
    severity: Severity,
    ctx: &InlineIgnoreContext,
) -> AppliedIgnores {
    apply_inline_ignores(findings, Path::new("x.go"), source, checker, severity, ctx)
}

#[test]
fn apply_inline_ignores_should_ReturnInputUnchanged_When_FindingsEmptyOrNoSubstring() {
    let ctx = InlineIgnoreContext::default();
    let out = apply(vec![], GO_IGNORE_ABOVE_10, "c", Severity::Advisory, &ctx);
    assert!(out.kept.is_empty() && out.dropped.is_empty());
    let out = apply(
        vec![finding(10, "[flag-argument] x")],
        "package main\nfunc f() {}\n",
        "c",
        Severity::Advisory,
        &ctx,
    );
    assert_eq!(out.kept.len(), 1);
    assert_eq!(ctx.scan_memo.scans.load(Ordering::Relaxed), 0);
    assert_eq!(ctx.scan_memo.hash_calls.load(Ordering::Relaxed), 0);
}

#[test]
fn apply_inline_ignores_should_ReturnRaw_When_ModeDisabled() {
    let ctx = InlineIgnoreContext::disabled();
    let out = apply(
        vec![finding(10, "[flag-argument] x")],
        GO_IGNORE_ABOVE_10,
        "c",
        Severity::Advisory,
        &ctx,
    );
    assert_eq!(out.kept.len(), 1);
    assert_eq!(ctx.scan_memo.hash_calls.load(Ordering::Relaxed), 0);
}

#[test]
fn apply_inline_ignores_should_DropAndReport_When_DirectiveCovers() {
    let ctx = InlineIgnoreContext::default();
    let out = apply(
        vec![
            finding(10, "[flag-argument] x"),
            finding(30, "[flag-argument] y"),
        ],
        GO_IGNORE_ABOVE_10,
        "syntax-rules-go",
        Severity::Blocking,
        &ctx,
    );
    assert_eq!(out.kept, vec![finding(30, "[flag-argument] y")]);
    assert_eq!(out.dropped.len(), 1);
    let d = &out.dropped[0];
    assert_eq!(
        (d.directive_start, d.directive_end),
        (Line::new(9), Line::new(9))
    );
    assert_eq!(d.rule.as_str(), "flag-argument");
    assert_eq!(d.reason.as_str(), "legacy api pinned");
    assert_eq!(d.finding_line, Line::new(10));
    assert_eq!(d.severity, Severity::Blocking);
}

#[test]
fn apply_inline_ignores_should_MatchCheckerName_When_NoBracketPrefix() {
    let source =
        "// kibitzer:ignore primitive-obsession -- ids are plain strings\nfn f(a: String) {}\n";
    let out = apply(
        vec![finding(2, "param a is a primitive")],
        source,
        "primitive-obsession",
        Severity::Advisory,
        &InlineIgnoreContext::default(),
    );
    assert!(out.kept.is_empty());
    assert_eq!(out.dropped.len(), 1);
}

#[test]
fn apply_inline_ignores_should_KeepFinding_When_DirectiveTwoLinesAbove() {
    let source = "// kibitzer:ignore flag-argument -- legacy api pinned\n\nfunc f() {}\n";
    let out = apply(
        vec![finding(3, "[flag-argument] x")],
        source,
        "c",
        Severity::Advisory,
        &InlineIgnoreContext::default(),
    );
    assert_eq!(out.kept.len(), 1);
}

#[test]
fn apply_inline_ignores_should_DropOnlyOwnRow_When_TrailingComment() {
    let source =
        "package main\nx := f() // kibitzer:ignore flag-argument -- legacy api\ny := g()\n";
    let out = apply(
        vec![
            finding(2, "[flag-argument] a"),
            finding(3, "[flag-argument] b"),
        ],
        source,
        "c",
        Severity::Advisory,
        &InlineIgnoreContext::default(),
    );
    assert_eq!(out.kept, vec![finding(3, "[flag-argument] b")]);
}

#[test]
fn apply_inline_ignores_should_NeverDrop_When_MetaRuleNamed() {
    let source = "// kibitzer:ignore ignore-syntax -- hide the repair\nbad\n";
    let out = apply(
        vec![finding(2, "[ignore-syntax] bad")],
        source,
        "inline-ignore",
        Severity::Advisory,
        &InlineIgnoreContext::default(),
    );
    assert_eq!(out.kept.len(), 1);
}

#[test]
fn apply_inline_ignores_should_DropAll_When_FileScopeRuleInHead() {
    let mut source = String::from("// kibitzer:ignore file-complexity -- generated tables\n");
    source.push_str(&"x\n".repeat(50));
    let out = apply(
        vec![
            finding(20, "[file-complexity] a"),
            finding(40, "[file-complexity] a"),
        ],
        &source,
        "file-complexity",
        Severity::Advisory,
        &InlineIgnoreContext::default(),
    );
    assert!(out.kept.is_empty());
    let out = apply(
        vec![finding(900, "[file-size] big")],
        "// kibitzer:ignore file-size -- generated tables\n",
        "file-size",
        Severity::Advisory,
        &InlineIgnoreContext::default(),
    );
    assert!(out.kept.is_empty());
}

#[test]
fn apply_inline_ignores_should_NotHeadScope_When_RuleNotFileScope() {
    let out = apply(
        vec![finding(40, "[flag-argument] a")],
        "// kibitzer:ignore flag-argument -- legacy api\n",
        "c",
        Severity::Advisory,
        &InlineIgnoreContext::default(),
    );
    assert_eq!(out.kept.len(), 1);
}

#[test]
fn apply_inline_ignores_should_CountDropped_When_CounterPresent() {
    let counter = Arc::new(SuppressionCounts::default());
    let ctx = InlineIgnoreContext {
        counter: Some(Arc::clone(&counter)),
        ..Default::default()
    };
    apply(
        vec![
            finding(10, "[flag-argument] x"),
            finding(30, "[flag-argument] y"),
        ],
        GO_IGNORE_ABOVE_10,
        "c",
        Severity::Blocking,
        &ctx,
    );
    apply(
        vec![finding(10, "[flag-argument] x")],
        GO_IGNORE_ABOVE_10,
        "c",
        Severity::Advisory,
        &ctx,
    );
    assert_eq!(counter.total.load(Ordering::Relaxed), 2);
    assert_eq!(counter.blocking.load(Ordering::Relaxed), 1);
}

#[test]
fn inline_ignore_context_should_KeepSeparateCounts_When_TwoRunsConcurrent() {
    let run = |n: usize| {
        let counter = Arc::new(SuppressionCounts::default());
        let ctx = InlineIgnoreContext {
            counter: Some(Arc::clone(&counter)),
            ..Default::default()
        };
        std::thread::spawn(move || {
            let source = format!(
                "{}\nx\n",
                (0..n)
                    .map(|_| "// kibitzer:ignore flag-argument -- legacy api\nf()")
                    .collect::<Vec<_>>()
                    .join("\n")
            );
            let findings = (0..n)
                .map(|i| finding(2 + i * 2, "[flag-argument] a"))
                .collect();
            apply(findings, &source, "c", Severity::Advisory, &ctx);
            counter.total.load(Ordering::Relaxed)
        })
    };
    let (a, b) = (run(2), run(3));
    assert_eq!((a.join().unwrap(), b.join().unwrap()), (2, 3));
}

#[test]
fn without_counter_should_NotCount_When_Replay() {
    let counter = Arc::new(SuppressionCounts::default());
    let ctx = InlineIgnoreContext {
        counter: Some(Arc::clone(&counter)),
        ..Default::default()
    };
    apply(
        vec![finding(10, "[flag-argument] x")],
        GO_IGNORE_ABOVE_10,
        "c",
        Severity::Blocking,
        &ctx.without_counter(),
    );
    assert_eq!(counter.total.load(Ordering::Relaxed), 0);
}

#[test]
fn scan_memo_should_ScanOncePerContentAndPath() {
    let ctx = InlineIgnoreContext::default();
    let f = || vec![finding(10, "[flag-argument] x")];
    for _ in 0..30 {
        apply(f(), GO_IGNORE_ABOVE_10, "c", Severity::Advisory, &ctx);
    }
    assert_eq!(ctx.scan_memo.scans.load(Ordering::Relaxed), 1);
    let edited = GO_IGNORE_ABOVE_10.replace("kibitzer:ignore", "removed");
    let edited = format!("{edited}// kibitzer is mentioned\n");
    let out = apply(f(), &edited, "c", Severity::Advisory, &ctx);
    assert_eq!(out.kept.len(), 1);
    assert_eq!(ctx.scan_memo.scans.load(Ordering::Relaxed), 2);
    apply_inline_ignores(
        f(),
        Path::new("other.go"),
        GO_IGNORE_ABOVE_10,
        "c",
        Severity::Advisory,
        &ctx,
    );
    assert_eq!(ctx.scan_memo.scans.load(Ordering::Relaxed), 3);
    let held = ctx.scan_memo.entry.lock().unwrap();
    assert_eq!(held.as_ref().unwrap().path, PathBuf::from("other.go"));
}

const FAST_PATH_CALLS: usize = 1_000;

fn big_go_source_without_marker() -> String {
    "func f(b bool) { _ = b }\n".repeat(5_000)
}

fn apply_once_kept(source: &str, ctx: &InlineIgnoreContext) -> usize {
    let findings = vec![finding(10, "[flag-argument] x")];
    apply(findings, source, "c", Severity::Advisory, ctx)
        .kept
        .len()
}

#[test]
fn apply_inline_ignores_should_UseNoScanOrHash_When_NoKibitzerSubstring() {
    let ctx = InlineIgnoreContext::default();
    let source = big_go_source_without_marker();
    for _ in 0..FAST_PATH_CALLS {
        assert_eq!(apply_once_kept(&source, &ctx), 1);
    }
    assert_eq!(ctx.scan_memo.scans.load(Ordering::Relaxed), 0);
    assert_eq!(ctx.scan_memo.hash_calls.load(Ordering::Relaxed), 0);
}

#[test]
fn inline_outcome_should_ExposeFirstAnchorAndDistinctRules() {
    let r = |s: &str| RuleId::new(s).unwrap();
    let outcome = InlineOutcome {
        shown: vec![
            Anchor {
                rule: r("a"),
                line: Line::new(5),
            },
            Anchor {
                rule: r("b"),
                line: Line::new(7),
            },
            Anchor {
                rule: r("a"),
                line: Line::new(9),
            },
        ],
        ..Default::default()
    };
    assert_eq!(outcome.first_anchor(), Some((&r("a"), Line::new(5))));
    assert_eq!(outcome.rule_ids(), vec![&r("a"), &r("b")]);
    assert_eq!(InlineOutcome::default().first_anchor(), None);
}

#[test]
fn inline_outcome_should_RoundTripJson_When_Populated() {
    let outcome = InlineOutcome {
        shown: vec![Anchor {
            rule: RuleId::new("a").unwrap(),
            line: Line::new(5),
        }],
        kept: vec![],
        dropped: vec![DroppedFinding {
            directive_start: Line::new(4),
            directive_end: Line::new(4),
            rule: RuleId::new("a").unwrap(),
            reason: Reason::new("legacy api pinned", &[]).unwrap(),
            finding_line: Line::new(5),
            severity: Severity::Blocking,
        }],
    };
    let json = serde_json::to_string(&outcome).unwrap();
    assert_eq!(
        serde_json::from_str::<InlineOutcome>(&json).unwrap(),
        outcome
    );
}

#[test]
fn anchor_rule_should_UseChecker_When_NoPrefixOrDynamic() {
    assert_eq!(anchor_rule("c", &finding(1, "[a-b] m")).as_str(), "a-b");
    assert_eq!(
        anchor_rule("primitive-obsession", &finding(1, "plain")).as_str(),
        "primitive-obsession"
    );
    assert_eq!(
        anchor_rule(
            "markdown-link-integrity",
            &finding(1, "[foo] used but never defined")
        )
        .as_str(),
        "markdown-link-integrity"
    );
}

#[test]
fn did_you_mean_should_SuggestNearestKnown_When_TypoOrTruncation() {
    assert_eq!(did_you_mean("flag-arg"), Some("flag-argument"));
    assert_eq!(did_you_mean("flag-arguments"), Some("flag-argument"));
    assert_eq!(did_you_mean("deep-nestng"), Some("deep-nesting"));
    assert_eq!(
        did_you_mean("primitive-obsesion"),
        Some("primitive-obsession")
    );
}

#[test]
fn did_you_mean_should_ReturnNone_When_NothingNear() {
    assert_eq!(did_you_mean("made-up-rule"), None);
    assert_eq!(did_you_mean("zzz"), None);
    assert_eq!(did_you_mean("flag-argument"), None);
}

#[test]
fn known_rule_should_AcceptRulesCheckersAndMeta_When_Queried() {
    assert!(known_rule("flag-argument"));
    assert!(known_rule("primitive-obsession"));
    assert!(known_rule("syntax-rules-rust"));
    assert!(known_rule("ignore-syntax"));
    assert!(!known_rule("made-up-rule"));
}

#[test]
fn levenshtein_should_CountEdits() {
    assert_eq!(levenshtein("kitten", "sitting"), 3);
    assert_eq!(levenshtein("", "abc"), 3);
    assert_eq!(levenshtein("same", "same"), 0);
}

/// Message-prefix ids (`"[id]`) in checker sources outside `#[cfg(test)]`.
fn static_rule_prefixes() -> Vec<(String, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files: Vec<PathBuf> = std::fs::read_dir(root.join("checkers"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    for top_level in [
        "single_call_site_delegation.rs",
        "god_class.rs",
        "isp_fat_interface.rs",
        "unreferenced_symbols.rs",
    ] {
        files.push(root.join(top_level));
    }
    let prefix = Regex::new(r#""\[([a-z][a-z0-9-]*)\]"#).unwrap();
    let mut found = Vec::new();
    for file in files {
        let name = file.file_name().unwrap().to_string_lossy().to_string();
        if !name.ends_with(".rs") || name.ends_with("_tests.rs") {
            continue;
        }
        let text = std::fs::read_to_string(&file).unwrap();
        let production = text.split("#[cfg(test)]").next().unwrap();
        for cap in prefix.captures_iter(production) {
            found.push((name.clone(), cap[1].to_string()));
        }
    }
    found
}

#[test]
fn rules_should_CoverEveryStaticRulePrefix_When_DriftGuardScansCheckerSources() {
    let found = static_rule_prefixes();
    assert!(found.len() >= 15, "scan found too little: {}", found.len());
    let missing: Vec<_> = found.iter().filter(|(_, id)| !known_rule(id)).collect();
    assert!(missing.is_empty(), "add these ids to RULES: {missing:?}");
}

#[test]
fn rules_should_GiveAnOwner_When_RuleComesFromSyntaxRulesOrCommentQuality() {
    let found = static_rule_prefixes();
    let owned_files = ["rules.rs", "comment_quality.rs", "file_size.rs"];
    let scanned = found
        .iter()
        .filter(|(file, _)| owned_files.contains(&file.as_str()))
        .count();
    assert!(
        scanned >= 11,
        "scan reached too few owned-file ids: {scanned}"
    );
    let ownerless: Vec<_> = found
        .iter()
        .filter(|(file, _)| {
            ["rules.rs", "comment_quality.rs", "file_size.rs"].contains(&file.as_str())
        })
        .filter(|(_, id)| !has_owner(id))
        .collect();
    assert!(
        ownerless.is_empty(),
        "add an owner entry to RULES for: {ownerless:?}"
    );
}

#[test]
fn rules_should_HaveUniqueIds() {
    let mut ids: Vec<&str> = RULES.iter().map(|r| r.id).collect();
    ids.sort_unstable();
    let before = ids.len();
    ids.dedup();
    assert_eq!(ids.len(), before);
}

fn directive_at(rule_list: &[&str], start: usize, end: usize, whole_line: bool) -> Directive {
    Directive::new(
        rules(rule_list),
        Reason::new("legacy api pinned", &[]).unwrap(),
        Line::new(start),
        Line::new(end),
        whole_line,
    )
    .unwrap()
}

fn raw(checker: &str, rule: &str, line: usize, message: &str) -> RawFinding {
    RawFinding {
        line: Line::new(line),
        checker: checker.to_string(),
        rule: RuleId::new(rule).unwrap(),
        message: message.to_string(),
    }
}

const SYNTAX: &[&str] = &["syntax-rules-go"];

#[test]
fn unused_ignores_should_Report_When_NoMatchingFinding() {
    let d = directive_at(&["flag-argument"], 9, 9, true);
    let out = unused_ignores(&[d], &[], SYNTAX, None);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].row, Line::new(9));
    assert_eq!(
        out[0].message,
        "[unused-ignore] kibitzer:ignore flag-argument suppresses nothing - remove it"
    );
}

#[test]
fn unused_ignores_should_BeEmpty_When_DirectiveCoversRawFinding() {
    let d = directive_at(&["flag-argument"], 9, 9, true);
    let f = raw("syntax-rules-go", "flag-argument", 10, "[flag-argument] x");
    assert!(unused_ignores(&[d], &[f], SYNTAX, None).is_empty());
}

#[test]
fn unused_ignores_should_Report_When_FindingTwoRowsBelowWholeLineDirective() {
    let d = directive_at(&["flag-argument"], 9, 9, true);
    let f = raw("syntax-rules-go", "flag-argument", 11, "[flag-argument] x");
    assert_eq!(unused_ignores(&[d], &[f], SYNTAX, None).len(), 1);
}

#[test]
fn unused_ignores_should_BeEmpty_When_AlsoShadowedByAccepted() {
    // Raw findings are taken before `accepted/` is applied, so a shadowed finding is still raw.
    let d = directive_at(&["flag-argument"], 4, 4, true);
    let shadowed = raw("syntax-rules-go", "flag-argument", 5, "[flag-argument] x");
    assert!(unused_ignores(&[d], &[shadowed], SYNTAX, None).is_empty());
}

#[test]
fn unused_ignores_should_SkipRule_When_OwningCheckerDisabledOrDidNotRun() {
    let d = directive_at(&["flag-argument"], 9, 9, true);
    assert!(unused_ignores(std::slice::from_ref(&d), &[], &["em-dash-overuse"], None).is_empty());
    assert!(unused_ignores(&[d], &[], &[], None).is_empty());
}

#[test]
fn unused_ignores_should_FailOpen_When_RuleOwnershipUnknown() {
    // `god-class` is a known rule whose emitting checker is outside the registry.
    let d = directive_at(&["god-class"], 9, 9, true);
    assert!(unused_ignores(std::slice::from_ref(&d), &[], SYNTAX, None).is_empty());
    assert!(unowned(&d, &[]).is_empty());
}

#[test]
fn unused_ignores_should_JudgeByCheckerName_When_MarkdownLinkIntegrity() {
    let d = directive_at(&["markdown-link-integrity"], 3, 3, true);
    let f = raw(
        "markdown-link-integrity",
        "markdown-link-integrity",
        4,
        "[foo] used but never defined",
    );
    let ran = ["markdown-link-integrity"];
    assert!(unused_ignores(std::slice::from_ref(&d), &[f], &ran, None).is_empty());
    assert_eq!(unused_ignores(&[d], &[], &ran, None).len(), 1);
}

#[test]
fn unused_ignores_should_PointToCheckList_When_RuleIsNeitherKnownNorCheckerName() {
    let d = directive_at(&["made-up-rule"], 9, 9, true);
    let out = unowned(&d, &[]);
    assert_eq!(out[0].kind, UnusedKind::UnknownRule);
    assert_eq!(
        out[0].message,
        "[unused-ignore] 'made-up-rule' is not a known rule or checker; run 'kibitzer check list' to see valid names"
    );
    assert!(!out[0].message.contains("remove it"));
}

#[test]
fn unused_ignores_should_StaySilent_When_UnknownRuleHasNearMatchOrWasUsed() {
    // A near miss is already reported by `[ignore-syntax]`; a built id that matched is used.
    let near = directive_at(&["flag-argumnt"], 9, 9, true);
    assert!(unowned(&near, &[]).is_empty());
    let plugin = directive_at(&["plugin-only-rule"], 9, 9, true);
    let used = DroppedFinding {
        directive_start: Line::new(9),
        directive_end: Line::new(9),
        rule: rid("plugin-only-rule"),
        reason: Reason::new("legacy api pinned", &[]).unwrap(),
        finding_line: Line::new(10),
        severity: Severity::Advisory,
    };
    assert!(unowned(&plugin, &[used]).is_empty());
}

fn unowned(d: &Directive, dropped: &[DroppedFinding]) -> Vec<UnusedVerdict> {
    let first = FirstPass {
        ran: SYNTAX.to_vec(),
        dropped: dropped.iter().collect(),
        kept: Vec::new(),
    };
    unowned_verdicts([d], &first, None)
}

#[test]
fn unused_ignores_should_ReportEachUnusedRule_When_CommaList() {
    let d = directive_at(&["flag-argument", "long-function"], 9, 9, true);
    let f = raw("syntax-rules-go", "long-function", 10, "[long-function] x");
    let out = unused_ignores(&[d], &[f], SYNTAX, None);
    assert_eq!(out.len(), 1);
    assert!(
        out[0].message.contains("flag-argument"),
        "{}",
        out[0].message
    );
}

#[test]
fn unused_ignores_should_OnlyJudgeTouchedDirectives_When_OnlyRowsGiven() {
    let near = directive_at(&["flag-argument"], 12, 12, true);
    let far = directive_at(&["flag-argument"], 50, 50, true);
    let out = unused_ignores(&[near, far], &[], SYNTAX, Some(&[(12, 12)]));
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].row, Line::new(12));
    let none = directive_at(&["flag-argument"], 50, 50, true);
    assert!(unused_ignores(&[none], &[], SYNTAX, Some(&[(1, 3)])).is_empty());
}

#[test]
fn unused_ignores_should_HonorFileHead_When_FileSizeRule() {
    let d = directive_at(&["file-size"], 1, 1, true);
    let f = raw("go-file-size", "file-size", 900, "[file-size] big");
    assert!(unused_ignores(&[d], &[f], &["go-file-size"], None).is_empty());
}

fn rid(name: &str) -> RuleId {
    RuleId::new(name).unwrap()
}

#[test]
fn syntax_hint_should_UseFileLeader_When_NoAnchor() {
    let go = syntax_hint(Path::new("src/foo.go"), None, &[]);
    assert!(go.contains("// kibitzer:ignore <rule> -- <why>"), "{go}");
    let py = syntax_hint(Path::new("x.py"), None, &[]);
    assert!(py.contains("# kibitzer:ignore <rule> -- <why>"), "{py}");
    let md = syntax_hint(Path::new("notes.md"), None, &[]);
    assert!(
        md.contains("<!-- kibitzer:ignore <rule> -- <why> -->"),
        "{md}"
    );
    let other = syntax_hint(Path::new("Makefile"), None, &[]);
    assert!(other.contains("# kibitzer:ignore <rule>"), "{other}");
    assert!(!go.contains("Rules:"), "{go}");
}

#[test]
fn syntax_hint_should_NameExactRow_When_AnchorGiven() {
    let rule = rid("flag-argument");
    let hint = syntax_hint(
        Path::new("src/foo.go"),
        Some((&rule, Line::new(20))),
        &[&rule],
    );
    assert!(hint.contains("above line 20"), "{hint}");
    assert!(
        hint.contains("// kibitzer:ignore flag-argument -- <why>"),
        "{hint}"
    );
    assert!(!hint.contains("false-positive"), "{hint}");
}

#[test]
fn syntax_hint_should_SayFileHead_When_FileScopeRule() {
    let rule = rid("file-size");
    let hint = syntax_hint(Path::new("a.go"), Some((&rule, Line::new(900))), &[&rule]);
    assert!(
        hint.contains("in the first 10 lines or on the anchor line"),
        "{hint}"
    );
}

#[test]
fn syntax_hint_should_ListRulesIncludingPrefixless_When_SecondFindingHasNoPrefix() {
    let a = rid("flag-argument");
    let b = rid("primitive-obsession");
    let hint = syntax_hint(Path::new("a.go"), Some((&a, Line::new(3))), &[&a, &b]);
    assert!(
        hint.contains("Rules: flag-argument, primitive-obsession"),
        "{hint}"
    );
    let first_prefixless = syntax_hint(Path::new("a.go"), Some((&b, Line::new(3))), &[&b, &a]);
    assert!(
        first_prefixless.contains("kibitzer:ignore primitive-obsession -- <why>"),
        "{first_prefixless}"
    );
}

#[test]
fn syntax_hint_should_TruncateRules_When_MoreThanSix() {
    let ids: Vec<RuleId> = (0..8).map(|i| rid(&format!("rule-{i}"))).collect();
    let refs: Vec<&RuleId> = ids.iter().collect();
    let hint = syntax_hint(Path::new("a.go"), Some((&ids[0], Line::new(1))), &refs);
    assert!(hint.contains("rule-5, ..."), "{hint}");
    assert!(!hint.contains("rule-6"), "{hint}");
}

#[test]
fn syntax_hint_limited_should_KeepOneRule_When_LimitIsOne() {
    let a = rid("flag-argument");
    let b = rid("file-size");
    let hint = syntax_hint_limited(Path::new("a.go"), Some((&a, Line::new(3))), &[&a, &b], 1);
    assert!(hint.contains("Rules: flag-argument, ..."), "{hint}");
}

#[test]
fn batch_syntax_hint_should_RenderTheDocumentedFooterLine_When_BuiltFromMarker() {
    assert_eq!(
        batch_syntax_hint(),
        "[kibitzer] to dismiss a finding you judged acceptable: <comment> kibitzer:ignore <rule> -- <why> (docs/suppressing-checks.md)"
    );
}

#[test]
fn batch_syntax_hint_should_ParseAsDirective_When_PlaceholdersFilled() {
    let hint = batch_syntax_hint();
    let example = hint
        .split_once(": ")
        .map(|(_, rest)| rest.trim_end_matches(" (docs/suppressing-checks.md)"))
        .unwrap();
    let filled = example
        .replace("<comment>", "//")
        .replace("<rule>", "flag-argument")
        .replace("<why>", "pinned by public API");
    assert!(matches!(
        parse_comment_line(&filled),
        DirectiveParse::Valid(_)
    ));
}

#[test]
fn rule_id_should_RejectCorruptValue_When_DeserializedFromJson() {
    assert!(serde_json::from_str::<RuleId>("\"Foo Bar\"").is_err());
    assert!(serde_json::from_str::<RuleId>("\"\"").is_err());
    let ok: RuleId = serde_json::from_str("\"flag-argument\"").unwrap();
    assert_eq!(serde_json::to_string(&ok).unwrap(), "\"flag-argument\"");
}

#[test]
fn reason_should_RejectCorruptValue_When_DeserializedFromJson() {
    assert!(serde_json::from_str::<Reason>("\"  \"").is_err());
    assert!(serde_json::from_str::<Reason>("\"needed\"").is_err());
    let ok: Reason = serde_json::from_str("\"legacy api pinned\"").unwrap();
    assert_eq!(serde_json::to_string(&ok).unwrap(), "\"legacy api pinned\"");
}
