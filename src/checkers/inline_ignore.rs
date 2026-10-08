//! Reports malformed `kibitzer:ignore` comments, unknown rule names, and heavy ignore use.
//! Findings use the meta rules `ignore-syntax` and `ignore-volume`, which no directive can cover.

use std::path::Path;
use std::sync::LazyLock;

use anyhow::Result;

use crate::checker::{CheckContext, Checker, Finding, Language};
use crate::inline_ignores::{
    DirectiveParse, MalformedReason, RuleId, ScanMemo, WeakReason, did_you_mean, echo_parts,
    known_rule, near_miss_text,
};

/// Every grammar-backed extension plus markdown: the files whose comments the scanners read.
pub(crate) fn scope_globs() -> Vec<String> {
    Language::all_globs()
        .into_iter()
        .chain(std::iter::once("**/*.md".to_string()))
        .collect()
}

static GLOBS: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    scope_globs()
        .into_iter()
        .map(|glob| &*Box::leak(glob.into_boxed_str()))
        .collect()
});

pub const NAME: &str = "inline-ignore";

pub struct InlineIgnoreChecker;

impl Checker for InlineIgnoreChecker {
    fn name(&self) -> &str {
        NAME
    }

    fn description(&self) -> &str {
        "flags malformed kibitzer:ignore comments, unknown rule names, and heavy ignore use"
    }

    // Raw-text scan: the directive scanners parse comments themselves.
    fn language(&self) -> Option<Language> {
        None
    }

    fn file_globs(&self) -> &[&str] {
        &GLOBS
    }

    fn tolerates_unreadable_files(&self) -> bool {
        true
    }

    fn check(&self, file: &Path, ctx: &CheckContext) -> Result<Vec<Finding>> {
        self.check_with_scans(file, ctx, &ScanMemo::default())
    }

    fn check_with_scans(
        &self,
        file: &Path,
        ctx: &CheckContext,
        scans: &ScanMemo,
    ) -> Result<Vec<Finding>> {
        let lines: Vec<&str> = ctx.source.lines().collect();
        let mut findings = Vec::new();
        let mut valid_rows = Vec::new();
        for scanned in scans.scan(file, ctx.source).iter() {
            let row = scanned.row.get();
            let line = lines.get(row - 1).copied().unwrap_or_default();
            match &scanned.parse {
                DirectiveParse::NotADirective => {}
                DirectiveParse::Malformed(reason) => {
                    findings.push(syntax_finding(row, malformed_message(*reason, line)));
                }
                DirectiveParse::Valid(d) => {
                    valid_rows.push(row);
                    findings.extend(unknown_rule_findings(row, d.rules()));
                }
            }
        }
        findings.extend(volume_finding(&valid_rows));
        findings.sort_by_key(|f| f.line);
        Ok(findings)
    }
}

/// One `did you mean` finding per unrecognised rule that has a near match.
fn unknown_rule_findings(row: usize, rules: &[RuleId]) -> impl Iterator<Item = Finding> + '_ {
    rules
        .iter()
        .filter(|rule| !known_rule(rule.as_str()))
        .filter_map(move |rule| {
            let near = did_you_mean(rule.as_str())?;
            Some(syntax_finding(
                row,
                format!(
                    "unknown rule '{rule}' - did you mean '{near}'?",
                    rule = rule.as_str()
                ),
            ))
        })
}

/// Reported at the threshold row only, so the finding is inside the changed lines of the
/// edit that adds the 5th directive and later directives add no repeat noise.
fn volume_finding(valid_rows: &[usize]) -> Option<Finding> {
    let &row = valid_rows.get(IGNORE_VOLUME_THRESHOLD - 1)?;
    Some(Finding {
        line: row,
        message: format!(
            "[ignore-volume] {} inline ignores in this file; tell the user you are silencing this many checks here, and either fix the code or ask the user whether a check is wrong",
            valid_rows.len()
        ),
    })
}

const IGNORE_VOLUME_THRESHOLD: usize = 5;

fn syntax_finding(line: usize, message: String) -> Finding {
    Finding {
        line,
        message: format!("[ignore-syntax] {message}"),
    }
}

/// ASCII-only repair text for one malformed directive; the offending line supplies the echo.
fn malformed_message(reason: MalformedReason, line: &str) -> String {
    let (rules, written_reason) = echo_parts(line);
    let rules = if rules.is_empty() {
        "<rule>".to_string()
    } else {
        rules
    };
    match reason {
        MalformedReason::MissingRule => {
            "kibitzer:ignore needs a rule id. Write: kibitzer:ignore <rule> -- <why>".to_string()
        }
        MalformedReason::MissingReason => format!(
            "kibitzer:ignore {rules} has no reason. Write: kibitzer:ignore {rules} -- <why this is acceptable>"
        ),
        MalformedReason::WeakReason(WeakReason::TooShort) => format!(
            "reason '{written_reason}' is too short to explain the code. Write: kibitzer:ignore {rules} -- {CONCRETE_REASON}"
        ),
        MalformedReason::WeakReason(WeakReason::RuleEcho) => format!(
            "reason repeats the rule id instead of saying why the code is acceptable. Write: kibitzer:ignore {rules} -- {CONCRETE_REASON}"
        ),
        MalformedReason::NearMissMarker => format!(
            "'{}' not recognized; use 'kibitzer:ignore'",
            near_miss_text(line).unwrap_or("kibitzer:")
        ),
        MalformedReason::NotAtCommentStart => format!(
            "kibitzer:ignore must start the comment; it was found after other text and suppresses nothing. Write it as its own comment: kibitzer:ignore {rules} -- <why>"
        ),
        MalformedReason::EmDashSeparator => format!(
            "use ASCII '--' (two hyphens) between the rule and the reason, not an em dash. Write: kibitzer:ignore {rules} -- <why>"
        ),
        MalformedReason::BadRuleChar => format!(
            "rule ids use only lowercase ASCII letters, digits and '-'. Write: kibitzer:ignore {rules} -- <why>"
        ),
        MalformedReason::BadRuleList => format!(
            "rule list must be comma-separated with no spaces. Write: kibitzer:ignore {rules} -- <why>"
        ),
    }
}

const CONCRETE_REASON: &str = "<the concrete constraint that makes this code acceptable>";

inventory::submit! {
    crate::checker::CheckerFactory(|| vec![Box::new(InlineIgnoreChecker)])
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;

    fn run(path: &str, source: &str) -> Vec<String> {
        let ctx = CheckContext { source, tree: None };
        InlineIgnoreChecker
            .check(Path::new(path), &ctx)
            .unwrap()
            .into_iter()
            .map(|f| format!("{}: {}", f.line, f.message))
            .collect()
    }

    fn go(comment: &str) -> Vec<String> {
        run(
            "x.go",
            &format!("package main\n\n{comment}\nfunc f(b bool) {{}}\n"),
        )
    }

    #[test]
    fn inline_ignore_should_ScanOnce_When_CheckerAndIgnorePassShareTheMemo() {
        use crate::inline_ignores::{InlineIgnoreContext, apply_inline_ignores};
        use std::sync::atomic::Ordering;
        let src =
            "package main\n\n// kibitzer:ignore flag-arg -- legacy api pinned\nfunc f(b bool) {}\n";
        let path = Path::new("x.go");
        let ctx = InlineIgnoreContext::default();
        let findings = crate::checker::run_checker_configured_with_scans(
            NAME,
            path,
            src,
            None,
            &ctx.scan_memo,
        )
        .unwrap();
        assert_eq!(findings.len(), 1, "{findings:?}");
        apply_inline_ignores(
            findings,
            path,
            src,
            NAME,
            crate::config::Severity::Advisory,
            &ctx,
        );
        // Both the checker and the ignore pass asked the memo; only one parsed.
        assert_eq!(ctx.scan_memo.hash_calls.load(Ordering::Relaxed), 2);
        assert_eq!(ctx.scan_memo.scans.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn inline_ignore_checker_should_BeRegisteredRawTextAdvisoryScope() {
        let checker = crate::checker::lookup(NAME).expect("registered");
        assert_eq!(checker.language(), None);
        let globs = checker.file_globs();
        assert!(globs.contains(&"**/*.md"));
        for lang in Language::ALL {
            for ext in lang.extensions() {
                assert!(globs.contains(&format!("**/*.{ext}").as_str()), "{ext}");
            }
        }
        assert_eq!(globs.len(), 13);
    }

    #[test]
    fn inline_ignore_should_Report_When_MissingReason() {
        let src = "package main\n\n\n\n\n\n\n// kibitzer:ignore flag-argument\nfunc f(b bool) {}\n";
        assert_eq!(
            run("x.go", src),
            vec![
                "8: [ignore-syntax] kibitzer:ignore flag-argument has no reason. Write: kibitzer:ignore flag-argument -- <why this is acceptable>"
            ]
        );
    }

    #[test]
    fn inline_ignore_should_Report_When_MissingRule() {
        assert_eq!(
            go("// kibitzer:ignore -- why not"),
            vec![
                "3: [ignore-syntax] kibitzer:ignore needs a rule id. Write: kibitzer:ignore <rule> -- <why>"
            ]
        );
    }

    #[test]
    fn inline_ignore_should_SuggestRule_When_UnknownRuleNearKnown() {
        assert_eq!(
            go("// kibitzer:ignore flag-arg -- legacy api pinned"),
            vec!["3: [ignore-syntax] unknown rule 'flag-arg' - did you mean 'flag-argument'?"]
        );
    }

    #[test]
    fn inline_ignore_should_EmitNothingForUnknownRule_When_NoNearMatch() {
        assert!(go("// kibitzer:ignore made-up-rule -- legacy api pinned").is_empty());
    }

    #[test]
    fn inline_ignore_should_ReportEachUnknownElementAndStillSuppress_When_CommaListMixesKnownAndUnknown()
     {
        let out = go("// kibitzer:ignore flag-argument,flag-arg -- legacy api pinned");
        assert_eq!(
            out,
            vec!["3: [ignore-syntax] unknown rule 'flag-arg' - did you mean 'flag-argument'?"]
        );
        let src = "package main\n\n// kibitzer:ignore flag-argument,flag-arg -- legacy api pinned\nfunc f(b bool) {\n\tif b {\n\t\tprintln(\"x\")\n\t}\n}\n";
        let findings =
            crate::checker::run_checker_configured("syntax-rules", Path::new("x.go"), src, None)
                .unwrap();
        let applied = crate::inline_ignores::apply_inline_ignores(
            findings,
            Path::new("x.go"),
            src,
            "syntax-rules",
            crate::config::Severity::Advisory,
            &crate::inline_ignores::InlineIgnoreContext::default(),
        );
        assert!(applied.kept.is_empty(), "{:?}", applied.kept);
        assert_eq!(applied.dropped.len(), 1);
    }

    #[test]
    fn inline_ignore_should_Report_When_NearMissMarker() {
        assert_eq!(
            go("// kibitzer: ignore foo -- bar baz"),
            vec!["3: [ignore-syntax] 'kibitzer: ignore' not recognized; use 'kibitzer:ignore'"]
        );
        assert_eq!(
            go("// kibitzer:false-positive primitive-obsession -- id is opaque"),
            vec![
                "3: [ignore-syntax] 'kibitzer:false-positive' not recognized; use 'kibitzer:ignore'"
            ]
        );
    }

    #[test]
    fn inline_ignore_should_TellUseAsciiDoubleHyphen_When_EmDashSeparator() {
        for dash in ['\u{2014}', '\u{2013}'] {
            let out = go(&format!(
                "// kibitzer:ignore flag-argument {dash} legacy API"
            ));
            assert_eq!(
                out,
                vec![
                    "3: [ignore-syntax] use ASCII '--' (two hyphens) between the rule and the reason, not an em dash. Write: kibitzer:ignore flag-argument -- <why>"
                ]
            );
            assert!(!out[0].contains("no reason"));
        }
    }

    #[test]
    fn inline_ignore_should_NotClaimCommaProblem_When_RuleHasUppercase() {
        let out = go("// kibitzer:ignore Flag-Argument -- legacy api pinned");
        assert_eq!(out.len(), 1, "{out:?}");
        assert!(out[0].contains("lowercase ASCII"), "{out:?}");
        assert!(!out[0].contains("comma-separated"), "{out:?}");
    }

    #[test]
    fn inline_ignore_should_Report_When_BadRuleList() {
        for text in ["a, b", "a,,b", "a,"] {
            let out = go(&format!("// kibitzer:ignore {text} -- legacy api pinned"));
            assert_eq!(out.len(), 1, "{text}");
            assert!(
                out[0].contains(
                    "rule list must be comma-separated with no spaces. Write: kibitzer:ignore "
                ),
                "{out:?}"
            );
        }
        assert!(
            go("// kibitzer:ignore a, b -- legacy api pinned")[0].contains("ignore a,b -- <why>")
        );
    }

    #[test]
    fn inline_ignore_should_Report_When_DirectiveNotAtCommentStart() {
        assert_eq!(
            go("// TODO kibitzer:ignore flag-argument -- legacy API, callers pinned"),
            vec![
                "3: [ignore-syntax] kibitzer:ignore must start the comment; it was found after other text and suppresses nothing. Write it as its own comment: kibitzer:ignore flag-argument -- <why>"
            ]
        );
    }

    #[test]
    fn inline_ignore_should_RenderSeparateMessages_When_ReasonTooShortVersusRuleEcho() {
        let short = &go("// kibitzer:ignore flag-argument -- needed")[0];
        let echo = &go("// kibitzer:ignore flag-argument -- flag-argument")[0];
        assert!(short.contains("reason 'needed' is too short to explain the code. Write: kibitzer:ignore flag-argument -- <the concrete constraint that makes this code acceptable>"), "{short}");
        assert!(echo.contains("reason repeats the rule id instead of saying why the code is acceptable. Write: kibitzer:ignore flag-argument -- <the concrete constraint"), "{echo}");
        assert_ne!(short, echo);
        for m in [short, echo] {
            assert!(!m.contains("at least two words"));
            assert!(m.contains("concrete constraint"));
        }
    }

    #[test]
    fn inline_ignore_should_Report_When_TrailingCommentIsMalformed() {
        let out = run(
            "x.go",
            "package main\n\nvar x = 1 // kibitzer:ignore flag-argument\n",
        );
        assert_eq!(out.len(), 1);
        assert!(
            out[0].starts_with("3: [ignore-syntax] kibitzer:ignore flag-argument has no reason."),
            "{out:?}"
        );
    }

    #[test]
    fn inline_ignore_should_ReadMarkdownComments_When_MdFile() {
        let out = run(
            "n.md",
            "# T\n\n<!-- kibitzer:ignore markdown-link-integrity -->\n",
        );
        assert_eq!(out.len(), 1);
        assert!(out[0].contains("has no reason"), "{out:?}");
    }

    #[test]
    fn inline_ignore_messages_should_ContainNoEmDash_When_AllReasonsRendered() {
        let cases = [
            "// kibitzer:ignore -- why not",
            "// kibitzer:ignore flag-argument",
            "// kibitzer:ignore flag-argument -- needed",
            "// kibitzer:ignore flag-argument -- flag-argument",
            "// kibitzer: ignore flag-argument -- legacy api",
            "// TODO kibitzer:ignore flag-argument -- legacy api pinned",
            "// kibitzer:ignore flag-argument \u{2014} legacy api",
            "// kibitzer:ignore a, b -- legacy api pinned",
            "// kibitzer:ignore flag-arg -- legacy api pinned",
        ];
        for case in cases {
            let out = go(case);
            assert_eq!(out.len(), 1, "{case}");
            assert!(!out[0].contains(['\u{2014}', '\u{2013}']), "{}", out[0]);
            assert!(out[0].is_ascii(), "{}", out[0]);
        }
    }

    #[test]
    fn inline_ignore_should_EmitNothing_When_DirectiveValid() {
        assert!(go("// kibitzer:ignore flag-argument -- legacy api pinned").is_empty());
        assert!(run("x.go", "package main\n").is_empty());
    }

    fn go_with_directives(count: usize, fifth_at: usize) -> String {
        let mut source = String::from("package main\n");
        let mut row = 1;
        for n in 0..count {
            let target = if n == 4 { fifth_at } else { row + 2 };
            while row + 1 < target {
                source.push('\n');
                row += 1;
            }
            source.push_str(&format!(
                "// kibitzer:ignore flag-argument -- pinned by caller {n}\n"
            ));
            row += 1;
        }
        source
    }

    fn volume_findings(source: &str) -> Vec<String> {
        run("x.go", source)
            .into_iter()
            .filter(|f| f.contains("[ignore-volume]"))
            .collect()
    }

    #[test]
    fn inline_ignore_should_EmitVolumeFinding_When_FiveValidDirectives() {
        let source = go_with_directives(5, 40);
        assert_eq!(
            volume_findings(&source),
            vec![
                "40: [ignore-volume] 5 inline ignores in this file; tell the user you are silencing this many checks here, and either fix the code or ask the user whether a check is wrong"
            ]
        );
    }

    #[test]
    fn inline_ignore_should_EmitVolumeOncePerFile_When_SevenValidDirectives() {
        let source = go_with_directives(7, 40);
        let found = volume_findings(&source);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].starts_with("40: "), "{found:?}");
    }

    #[test]
    fn inline_ignore_should_EmitNothing_When_FourValidDirectives() {
        assert!(volume_findings(&go_with_directives(4, 40)).is_empty());
    }

    #[test]
    fn inline_ignore_should_NotCountMalformedDirectives_When_CountingVolume() {
        let source = format!(
            "{}// kibitzer:ignore flag-argument\n",
            go_with_directives(4, 40)
        );
        assert!(volume_findings(&source).is_empty());
    }
}
