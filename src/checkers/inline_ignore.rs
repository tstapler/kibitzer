//! Reports malformed `kibitzer:ignore` comments, unknown rule names, and heavy ignore use.
//! Findings use the meta rules `ignore-syntax` and `ignore-volume`, which no directive can cover.

use std::path::Path;
use std::sync::LazyLock;

use anyhow::Result;

use crate::checker::{CheckContext, Checker, Finding, Language};

/// Every grammar-backed extension plus markdown: the files whose comments the scanners read.
static GLOBS: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    Language::all_globs()
        .into_iter()
        .chain(std::iter::once("**/*.md".to_string()))
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

    fn check(&self, _file: &Path, _ctx: &CheckContext) -> Result<Vec<Finding>> {
        Ok(Vec::new())
    }
}

inventory::submit! {
    crate::checker::CheckerFactory(|| vec![Box::new(InlineIgnoreChecker)])
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;

    #[test]
    fn inline_ignore_checker_should_BeRegisteredRawTextAdvisoryScope() {
        let checker = crate::checker::lookup("inline-ignore").expect("registered");
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
}
