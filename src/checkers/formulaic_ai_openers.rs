use std::path::Path;

use anyhow::Result;

use crate::checker::{CheckContext, Checker, Finding, Language};
use crate::markdown_text::split_sentences;

/// Formulaic transition phrases a paragraph's first sentence starts with — the same
/// "mechanical proxy for a writing tell" idea as [`crate::checkers::repetitive_sentences::OPENERS`],
/// but targeting known AI-writing openers specifically (borrowed from analyzing
/// github.com/puneethkotha/humanizer-workbench's heuristics for LLM-generated prose).
///
/// Deliberately excludes single common transition words ("moreover", "furthermore",
/// "additionally", "overall,", "ultimately,") that an earlier version of this list
/// included: backtesting against microsoft/vscode-docs found "additionally" alone
/// accounted for 346 of 530 total hits across the corpus, including release notes from
/// 2016 — years before LLM-generated prose was a phenomenon. A single ordinary transition
/// word isn't a meaningful AI-writing signal on its own; only the more distinctive,
/// multi-word formulaic phrases below are specific enough to be worth flagging.
const AI_OPENERS: &[&str] = &[
    "in conclusion",
    "it is important to note",
    "it is worth noting",
    "in today's",
    "as we can see",
    "to summarize",
    "in summary",
];

/// Flags a paragraph whose *first* sentence opens with a formulaic AI-writing transition
/// phrase. Unlike [`crate::checkers::repetitive_sentences::RepetitiveSentencesChecker`], a single
/// occurrence is enough to flag — there's no run-of-3+ threshold here.
///
/// Intentionally NOT wired into `config::default_checks()`; opt in via
/// `.kibitzer/inspect.json`. A formulaic opener can appear in legitimate human writing too
/// (e.g. "In summary, the results support..."), so this has a higher false-positive rate
/// than requiring a 3+ run the way `repetitive_sentences` does.
///
/// [`AI_OPENERS`] is the default list; a project can override it via this check's
/// `options` (see [`Checker::configure`] and `docs/prose-checks.md`).
pub struct FormulaicAiOpenersChecker {
    openers: Vec<String>,
}

impl Default for FormulaicAiOpenersChecker {
    fn default() -> Self {
        FormulaicAiOpenersChecker {
            openers: AI_OPENERS.iter().map(|o| o.to_string()).collect(),
        }
    }
}

/// Per-project override shape for [`FormulaicAiOpenersChecker::configure`]. Unknown
/// fields are rejected so a typo fails loudly at config-load time instead of running
/// silently.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    openers: Option<Vec<String>>,
}

impl Checker for FormulaicAiOpenersChecker {
    fn name(&self) -> &str {
        "formulaic-ai-openers"
    }

    fn description(&self) -> &str {
        "flags a paragraph opening with a formulaic AI-writing transition phrase"
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
        Ok(Some(Box::new(FormulaicAiOpenersChecker {
            openers: opts.openers.unwrap_or_else(|| self.openers.clone()),
        })))
    }
}

inventory::submit! {
    crate::checker::CheckerFactory(|| vec![Box::new(FormulaicAiOpenersChecker::default())])
}

impl FormulaicAiOpenersChecker {
    fn check_source(&self, body: &str) -> Vec<Finding> {
        crate::markdown_text::check_paragraphs(body, |line, text| self.check_paragraph(line, text))
    }

    /// Checks only the paragraph's first sentence — a formulaic opener buried later in
    /// the paragraph isn't this defect, since the paragraph didn't *open* with it.
    fn check_paragraph(&self, line: usize, text: &str) -> Option<Finding> {
        let first_sentence = split_sentences(text)
            .into_iter()
            .next()?
            .trim()
            .to_lowercase();
        let phrase = self
            .openers
            .iter()
            .find(|opener| first_sentence.starts_with(opener.as_str()))?;
        Some(Finding {
            line,
            message: format!(
                "paragraph opens with a formulaic AI-writing transition (\"{phrase}\") — start with the actual point instead"
            ),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs the default opener list — the shorthand every non-`configure_*` test below
    /// uses instead of spelling out `FormulaicAiOpenersChecker::default()`.
    fn check_source(body: &str) -> Vec<Finding> {
        FormulaicAiOpenersChecker::default().check_source(body)
    }

    #[test]
    fn flags_formulaic_opener() {
        let body = "In conclusion, the results were clear. It worked well.\n";
        let findings = check_source(body);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("\"in conclusion\""));
    }

    #[test]
    fn ignores_opener_in_later_sentence() {
        let body = "The results were clear. In conclusion, it worked well.\n";
        assert!(check_source(body).is_empty());
    }

    #[test]
    fn ignores_list_items() {
        let body = "- In conclusion, this is a.\n- Moreover, this is b.\n";
        assert!(check_source(body).is_empty());
    }

    #[test]
    fn configure_overrides_opener_list() {
        let configured = FormulaicAiOpenersChecker::default()
            .configure(&serde_json::json!({ "openers": ["all in all"] }))
            .unwrap()
            .unwrap();
        let ctx = CheckContext {
            source: "All in all, the results were clear. It worked well.\n",
            tree: None,
        };
        let findings = configured.check(Path::new("doc.md"), &ctx).unwrap();
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("\"all in all\""));

        // The default opener no longer fires once the list is overridden.
        let ctx = CheckContext {
            source: "In conclusion, the results were clear. It worked well.\n",
            tree: None,
        };
        assert!(
            configured
                .check(Path::new("doc.md"), &ctx)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn configure_rejects_malformed_options() {
        let err = FormulaicAiOpenersChecker::default()
            .configure(&serde_json::json!({ "openers": "not-a-list" }));
        assert!(err.is_err());
    }

    #[test]
    fn configure_rejects_unknown_option_key() {
        let err = FormulaicAiOpenersChecker::default()
            .configure(&serde_json::json!({ "openerz": ["all in all"] }));
        assert!(err.is_err());
    }
}
