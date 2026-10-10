use std::path::Path;

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

/// Flags a markdown heading, or the first sentence of a flowing-prose paragraph, that
/// ends with `, <adverb>`. Advisory style judgment; list items and non-lead sentences are
/// out of scope, matching the other prose checkers. Not wired into
/// `config::default_checks()`: opt-in until backtested false-positive rates justify it
/// (see `docs/prose-checks.md`).
///
/// [`DEFAULT_ADVERBS`] is the default list; a project replaces it wholesale via this
/// check's `adverbs` option (see [`Checker::configure`]).
pub struct HollowQualifierPhrasingChecker {
    adverbs: Vec<String>,
    /// Always `Some` after construction; kept optional only for the `?` ergonomics in `finding`.
    ending: Option<Regex>,
}

impl Default for HollowQualifierPhrasingChecker {
    fn default() -> Self {
        Self::with_adverbs(DEFAULT_ADVERBS.iter().map(|a| a.to_string()).collect())
            .expect("default adverbs form a valid regex")
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
        Ok(Some(Box::new(Self::with_adverbs(adverbs)?)))
    }
}

inventory::submit! {
    crate::checker::CheckerFactory(|| vec![Box::new(HollowQualifierPhrasingChecker::default())])
}

impl HollowQualifierPhrasingChecker {
    fn with_adverbs(adverbs: Vec<String>) -> Result<Self> {
        // A blank entry would make the alternation match the empty string, flagging
        // every text that merely ends in a comma.
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
        let ending = Some(Regex::new(&format!(r"(?i),\s*(?:{alternation})\s*$"))?);
        Ok(HollowQualifierPhrasingChecker { adverbs, ending })
    }

    fn check_source(&self, body: &str) -> Vec<Finding> {
        use crate::markdown_text::{BlockKind, for_each_block, split_sentences};

        let mut findings = Vec::new();
        for_each_block(body, |kind, line, text| {
            let candidate = match kind {
                BlockKind::Heading => Some(("heading", text)),
                BlockKind::Paragraph => split_sentences(text)
                    .into_iter()
                    .next()
                    .map(|lead| ("lead sentence", lead)),
            };
            if let Some((label, text)) = candidate {
                findings.extend(self.finding(line, label, text));
            }
        });
        findings
    }

    fn finding(&self, line: usize, kind: &str, text: &str) -> Option<Finding> {
        // Inline code/emphasis flattening leaves stray spaces ("The  cache , x"); tidy
        // them so the echoed text reads like the source.
        let tidy = text
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .replace(" ,", ",");
        let trimmed = tidy.trim_end_matches(['.', '!', '?', ':']).trim_end();
        if !self.ending.as_ref()?.is_match(trimmed) {
            return None;
        }
        let shown = if trimmed.chars().count() > MAX_ECHOED_CHARS {
            let tail: String = trimmed
                .chars()
                .rev()
                .take(MAX_ECHOED_CHARS)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            format!("...{tail}")
        } else {
            trimmed.to_string()
        };
        Some(Finding {
            line,
            message: format!(
                "{kind} \"{shown}\" names the kind of thing, not the thing itself - say what it covers instead of qualifying it"
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
        assert!(findings[0].message.contains("..."));
        assert!(findings[0].message.len() < 250);
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
