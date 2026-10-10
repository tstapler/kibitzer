use std::path::Path;
use std::sync::LazyLock;

use anyhow::Result;
use regex::Regex;

use crate::checker::{CheckContext, Checker, Finding, Language};

/// Distancing adverbs that, trailing a noun phrase after a comma, turn a heading or lead
/// sentence into a category label ("The primitive, generically") instead of a statement
/// of content. Deliberately narrow: only the `<noun phrase>, <adverb>` end-of-text form
/// is flagged, never the adverb mid-sentence.
const DEFAULT_ADVERBS: &[&str] = &[
    "generically",
    "fundamentally",
    "essentially",
    "basically",
    "broadly",
    "generally",
    "abstractly",
    "conceptually",
    "theoretically",
    "ultimately",
    "nominally",
];

/// Longest stretch of the offending text echoed in a finding (the tail, where the adverb is).
const MAX_ECHOED_CHARS: usize = 80;

/// Flags a heading, or the first sentence of a prose paragraph, ending in `, <adverb>`.
/// Opt-in (not in `config::default_checks()`); see `docs/prose-checks.md`.
/// A project replaces [`DEFAULT_ADVERBS`] wholesale via the `adverbs` option.
pub struct HollowQualifierPhrasingChecker {
    adverbs: Vec<String>,
    ending: Regex,
}

/// Built once: `registry()` constructs every checker per file, so `Default` must be cheap.
static DEFAULT_ENDING: LazyLock<Regex> = LazyLock::new(|| {
    let defaults: Vec<String> = DEFAULT_ADVERBS.iter().map(|a| a.to_string()).collect();
    build_ending(&defaults).expect("default adverbs form a valid regex")
});

impl Default for HollowQualifierPhrasingChecker {
    fn default() -> Self {
        HollowQualifierPhrasingChecker {
            adverbs: DEFAULT_ADVERBS.iter().map(|a| a.to_string()).collect(),
            ending: DEFAULT_ENDING.clone(),
        }
    }
}

/// Per-project override shape for [`HollowQualifierPhrasingChecker::configure`]. Unknown
/// fields are rejected so a typo fails loudly at config-load time.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    adverbs: Option<Vec<String>>,
}

impl Checker for HollowQualifierPhrasingChecker {
    fn name(&self) -> &str {
        "hollow-qualifier-phrasing"
    }

    fn description(&self) -> &str {
        "flags a heading or lead sentence ending in ', <distancing adverb>' (e.g. 'The primitive, generically')"
    }

    fn language(&self) -> Option<Language> {
        None
    }

    fn file_globs(&self) -> &[&str] {
        &["**/*.md"]
    }

    fn check(&self, _file: &Path, ctx: &CheckContext) -> Result<Vec<Finding>> {
        Ok(self.check_source(ctx.source))
    }

    fn configure(&self, options: &serde_json::Value) -> Result<Option<Box<dyn Checker>>> {
        let opts: Options = serde_json::from_value(options.clone())?;
        let adverbs = opts.adverbs.unwrap_or_else(|| self.adverbs.clone());
        let ending = build_ending(&adverbs)?;
        Ok(Some(Box::new(HollowQualifierPhrasingChecker {
            adverbs,
            ending,
        })))
    }
}

inventory::submit! {
    crate::checker::CheckerFactory(|| vec![Box::new(HollowQualifierPhrasingChecker::default())])
}

/// `, <adverb>` at end of text, case-insensitive. Blank entries are dropped (one would
/// match the empty string and flag any text ending in a comma); an all-blank list is an
/// error so a config typo fails at load time.
fn build_ending(adverbs: &[String]) -> Result<Regex> {
    let usable: Vec<String> = adverbs
        .iter()
        .map(|a| a.trim())
        .filter(|a| !a.is_empty())
        .map(regex::escape)
        .collect();
    anyhow::ensure!(
        !usable.is_empty(),
        "hollow-qualifier-phrasing: `adverbs` must contain at least one non-blank word"
    );
    let alternation = usable.join("|");
    Ok(Regex::new(&format!(r"(?i),\s*(?:{alternation})\s*$"))?)
}

impl HollowQualifierPhrasingChecker {
    fn check_source(&self, body: &str) -> Vec<Finding> {
        use crate::markdown_text::{BlockKind, for_each_block, split_sentences};

        let mut findings = Vec::new();
        for_each_block(body, |kind, line, text| {
            let candidate = match kind {
                BlockKind::Heading => Some(text),
                BlockKind::Paragraph => split_sentences(text).into_iter().next(),
            };
            if let Some(text) = candidate {
                findings.extend(self.finding(line, kind, text));
            }
        });
        findings
    }

    fn finding(
        &self,
        line: usize,
        kind: crate::markdown_text::BlockKind,
        text: &str,
    ) -> Option<Finding> {
        // Inline code/emphasis flattening leaves stray spaces ("The  cache , x"); tidy
        // them so the echoed text reads like the source.
        let tidy = text
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .replace(" ,", ",");
        let trimmed = tidy.trim_end_matches(['.', '!', '?', ':']).trim_end();
        if !self.ending.is_match(trimmed) {
            return None;
        }
        let label = match kind {
            crate::markdown_text::BlockKind::Heading => "heading",
            crate::markdown_text::BlockKind::Paragraph => "lead sentence",
        };
        let count = trimmed.chars().count();
        let shown = if count > MAX_ECHOED_CHARS {
            let tail: String = trimmed.chars().skip(count - MAX_ECHOED_CHARS).collect();
            format!("...{tail}")
        } else {
            trimmed.to_string()
        };
        Some(Finding {
            line,
            message: format!(
                "{label} \"{shown}\" names the kind of thing, not the thing itself - say what it covers instead of qualifying it"
            ),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check_source(body: &str) -> Vec<Finding> {
        HollowQualifierPhrasingChecker::default().check_source(body)
    }

    #[test]
    fn flags_heading_ending_with_generically() {
        let findings = check_source("# The primitive, generically\n\nBody text here.\n");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].line, 1);
        assert!(findings[0].message.starts_with("heading"));
        assert!(findings[0].message.contains("The primitive, generically"));
    }

    #[test]
    fn flags_heading_case_insensitively() {
        assert_eq!(check_source("## The approach, Generically\n").len(), 1);
    }

    #[test]
    fn flags_heading_ending_with_fundamentally() {
        assert_eq!(check_source("## The approach, fundamentally\n").len(), 1);
    }

    #[test]
    fn flags_lead_sentence_ending_with_essentially() {
        let findings = check_source("The cache, essentially. It stores results.\n");
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.starts_with("lead sentence"));
    }

    #[test]
    fn allows_heading_without_distancing_adverb() {
        assert!(check_source("# The primitive\n\n## Why it works, in practice\n").is_empty());
    }

    #[test]
    fn allows_adverb_mid_heading() {
        assert!(check_source("# Generically, the primitive\n").is_empty());
    }

    #[test]
    fn ignores_non_lead_sentence_ending_with_generically() {
        let body = "The cache stores results. The primitive, generically.\n";
        assert!(check_source(body).is_empty());
    }

    #[test]
    fn ignores_list_item_ending_with_generically() {
        let body = "- The primitive, generically\n- Another, generically.\n";
        assert!(check_source(body).is_empty());
    }

    #[test]
    fn configure_overrides_adverb_list() {
        let configured = HollowQualifierPhrasingChecker::default()
            .configure(&serde_json::json!({ "adverbs": ["banana"] }))
            .unwrap()
            .unwrap();
        let ctx = CheckContext {
            source: "# The thing, banana\n\n# The thing, generically\n",
            tree: None,
        };
        let findings = configured.check(Path::new("doc.md"), &ctx).unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].line, 1);
    }

    #[test]
    fn configure_rejects_blank_only_adverb_list() {
        let err = HollowQualifierPhrasingChecker::default()
            .configure(&serde_json::json!({ "adverbs": ["", "  "] }));
        assert!(err.is_err());
    }

    #[test]
    fn ignores_yaml_front_matter() {
        let body = "---\ntitle: X\ndescription: a thing, essentially\n---\n\n# Real heading\n";
        assert!(check_source(body).is_empty());
    }

    #[test]
    fn truncates_long_lead_sentence_in_message() {
        let body = format!("{}, essentially. More.\n", "word ".repeat(60));
        let findings = check_source(&body);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.starts_with("lead sentence \"..."));
        assert!(findings[0].message.contains("essentially\""));
        assert!(findings[0].message.len() < 250);
    }

    #[test]
    fn configured_adverbs_are_matched_literally() {
        let configured = HollowQualifierPhrasingChecker::default()
            .configure(&serde_json::json!({ "adverbs": ["a|b"] }))
            .unwrap()
            .unwrap();
        let ctx = CheckContext {
            source: "# X, a|b\n\n# X, a\n",
            tree: None,
        };
        let findings = configured.check(Path::new("doc.md"), &ctx).unwrap();
        assert_eq!(findings.iter().map(|f| f.line).collect::<Vec<_>>(), vec![1]);
    }

    #[test]
    fn configure_without_adverbs_keeps_defaults() {
        let configured = HollowQualifierPhrasingChecker::default()
            .configure(&serde_json::json!({}))
            .unwrap()
            .unwrap();
        let ctx = CheckContext {
            source: "# The primitive, generically\n",
            tree: None,
        };
        assert_eq!(
            configured.check(Path::new("doc.md"), &ctx).unwrap().len(),
            1
        );
    }

    #[test]
    fn handles_crlf_line_endings() {
        let findings = check_source("# The primitive, generically\r\n\r\nBody.\r\n");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].line, 1);
    }

    #[test]
    fn flags_blockquoted_heading() {
        assert_eq!(check_source("> ## The primitive, generically\n").len(), 1);
    }

    #[test]
    fn reports_findings_in_document_order() {
        let body = "# One, generically\n\nThe cache, essentially. Rest.\n\n## Two, basically\n";
        let lines: Vec<usize> = check_source(body).iter().map(|f| f.line).collect();
        assert_eq!(lines, vec![1, 3, 5]);
    }

    /// Known false positive, pinned so a future fix is a deliberate change: a trailing
    /// inline-code adverb is flattened like prose (see docs/prose-checks.md).
    #[test]
    fn flags_trailing_inline_code_adverb_known_false_positive() {
        assert_eq!(check_source("## Cache, `generically`\n").len(), 1);
    }

    #[test]
    fn configure_rejects_unknown_option_key() {
        let err = HollowQualifierPhrasingChecker::default()
            .configure(&serde_json::json!({ "adverb": ["generically"] }));
        assert!(err.is_err());
    }

    #[test]
    fn configure_rejects_malformed_options() {
        let err = HollowQualifierPhrasingChecker::default()
            .configure(&serde_json::json!({ "adverbs": "generically" }));
        assert!(err.is_err());
    }
}
