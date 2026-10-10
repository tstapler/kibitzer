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
    /// `None` when `adverbs` is empty (nothing to match).
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
        let ending = if adverbs.is_empty() {
            None
        } else {
            let alternation = adverbs
                .iter()
                .map(|a| regex::escape(a.trim()))
                .collect::<Vec<_>>()
                .join("|");
            Some(Regex::new(&format!(r"(?i),\s*(?:{alternation})\s*$"))?)
        };
        Ok(HollowQualifierPhrasingChecker { adverbs, ending })
    }

    fn check_source(&self, body: &str) -> Vec<Finding> {
        let mut findings = Vec::new();
        crate::markdown_text::for_each_heading(body, |line, text| {
            findings.extend(self.finding(line, "heading", text));
        });
        crate::markdown_text::for_each_paragraph(body, |p| {
            let lead = crate::markdown_text::split_sentences(&p.text)
                .into_iter()
                .next();
            if let Some(lead) = lead {
                findings.extend(self.finding(p.line, "lead sentence", lead));
            }
        });
        findings.sort_by_key(|f| f.line);
        findings
    }

    fn finding(&self, line: usize, kind: &str, text: &str) -> Option<Finding> {
        let trimmed = text
            .trim()
            .trim_end_matches(['.', '!', '?', ':'])
            .trim_end();
        if !self.ending.as_ref()?.is_match(trimmed) {
            return None;
        }
        Some(Finding {
            line,
            message: format!(
                "{kind} \"{trimmed}\" names the kind of thing, not the thing itself - say what it covers instead of qualifying it"
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
