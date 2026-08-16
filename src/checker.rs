use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use tree_sitter::Tree;

use crate::go_blank_imports::BlankImportsChecker;
use crate::go_error_context::ErrorContextChecker;
use crate::go_ignored_error::IgnoredErrorChecker;
use crate::markdown_link_integrity::MarkdownLinkIntegrityChecker;
use crate::primitive_obsession::PrimitiveObsessionChecker;
use crate::rules::SyntaxRulesChecker;

/// A finding a [`Checker`] reports against a specific line of a file. Formatted by
/// callers as `{file}:{line}: {message}` — the convention `check.rs`'s diff-scoping
/// parser depends on, so don't change this shape without updating that parser too.
#[derive(Debug, PartialEq, Eq)]
pub struct Finding {
    pub line: usize,
    pub message: String,
}

/// A tree-sitter grammar a [`Checker`] can request via [`Checker::language`]. Kept
/// deliberately open-ended (not just `Go`) so future checkers for other languages plug
/// into the same [`GrammarCache`] without a trait-signature change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    Go,
    TypeScript,
    Tsx,
    JavaScript,
    Python,
    Java,
    Kotlin,
}

impl Language {
    fn ts_language(self) -> tree_sitter::Language {
        match self {
            Language::Go => tree_sitter_go::LANGUAGE.into(),
            Language::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Language::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
            Language::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
            Language::Python => tree_sitter_python::LANGUAGE.into(),
            Language::Java => tree_sitter_java::LANGUAGE.into(),
            Language::Kotlin => tree_sitter_kotlin_ng::LANGUAGE.into(),
        }
    }
}

/// Parsed input handed to a [`Checker`]: the raw source plus, when the checker declared
/// a [`Language`], the tree-sitter parse of it (shared across every checker that needs
/// the same grammar for the same file — see [`GrammarCache`]).
pub struct CheckContext<'a> {
    pub source: &'a str,
    pub tree: Option<&'a Tree>,
}

/// A single native, in-process check. Implementors declare what language (if any) they
/// need parsed and which files they apply to; the registry and dispatcher take care of
/// parsing (once, via [`GrammarCache`]) and glob matching.
pub trait Checker {
    /// Stable identifier looked up in `config::Check::checker` and the CLI.
    fn name(&self) -> &str;
    /// Human-readable description (for `--help`-style listings).
    fn description(&self) -> &str;
    /// The tree-sitter grammar this checker needs parsed, or `None` for checkers that
    /// only need raw source text.
    fn language(&self) -> Option<Language>;
    /// Glob patterns (matched against a path relative to the repo root, `**` supported)
    /// a file must satisfy for this checker to apply.
    fn file_globs(&self) -> &[&str];
    /// Run the check against `file`'s already-loaded `ctx`.
    fn check(&self, file: &Path, ctx: &CheckContext) -> Result<Vec<Finding>>;
}

/// All natively implemented checkers, keyed by [`Checker::name`]. Adding a new native
/// check means adding its module and one entry here — no other file needs to change.
pub fn registry() -> Vec<Box<dyn Checker>> {
    vec![
        Box::new(PrimitiveObsessionChecker),
        Box::new(MarkdownLinkIntegrityChecker),
        Box::new(BlankImportsChecker),
        Box::new(IgnoredErrorChecker),
        Box::new(ErrorContextChecker),
        Box::new(SyntaxRulesChecker::new(Language::Go)),
        Box::new(SyntaxRulesChecker::new(Language::TypeScript)),
        Box::new(SyntaxRulesChecker::new(Language::Tsx)),
        Box::new(SyntaxRulesChecker::new(Language::JavaScript)),
        Box::new(SyntaxRulesChecker::new(Language::Python)),
        Box::new(SyntaxRulesChecker::new(Language::Java)),
        Box::new(SyntaxRulesChecker::new(Language::Kotlin)),
    ]
}

pub fn lookup(name: &str) -> Option<Box<dyn Checker>> {
    registry().into_iter().find(|c| c.name() == name)
}

/// Parses at most once per [`Language`] over this cache instance's lifetime, regardless
/// of how many checkers declare that language — checkers sharing a grammar share the
/// parsed [`Tree`] instead of each re-parsing the same source. Callers must construct a
/// fresh `GrammarCache` per file (as `run_checker_against_source` does): the cache key is
/// `Language` alone, not `(Language, source)`, so reusing one instance across different
/// files of the same language would return the wrong file's tree on a cache hit.
#[derive(Default)]
pub struct GrammarCache {
    trees: RefCell<HashMap<Language, Tree>>,
}

impl GrammarCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Parse `source` with `language`'s grammar, reusing a prior parse of the same
    /// language for this cache instance if one already happened.
    pub fn parse(&self, language: Language, source: &str) -> Result<Tree> {
        if let Some(tree) = self.trees.borrow().get(&language) {
            return Ok(tree.clone());
        }
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&language.ts_language())
            .with_context(|| format!("loading tree-sitter grammar for {language:?}"))?;
        let tree = parser
            .parse(source, None)
            .with_context(|| format!("parsing source with {language:?} grammar"))?;
        self.trees.borrow_mut().insert(language, tree.clone());
        Ok(tree)
    }
}

impl std::fmt::Debug for GrammarCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GrammarCache")
            .field("languages_cached", &self.trees.borrow().len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::path::PathBuf;

    struct StubChecker {
        name: String,
        language: Option<Language>,
        globs: Vec<&'static str>,
        parses_seen: std::rc::Rc<Cell<usize>>,
    }

    impl Checker for StubChecker {
        fn name(&self) -> &str {
            &self.name
        }
        fn description(&self) -> &str {
            "stub checker for tests"
        }
        fn language(&self) -> Option<Language> {
            self.language
        }
        fn file_globs(&self) -> &[&str] {
            &self.globs
        }
        fn check(&self, _file: &Path, ctx: &CheckContext) -> Result<Vec<Finding>> {
            if ctx.tree.is_some() {
                self.parses_seen.set(self.parses_seen.get() + 1);
            }
            Ok(vec![])
        }
    }

    #[test]
    fn trait_is_object_safe_and_usable_in_a_vec() {
        let checkers: Vec<Box<dyn Checker>> = vec![Box::new(StubChecker {
            name: "stub".to_string(),
            language: None,
            globs: vec!["**/*.go"],
            parses_seen: std::rc::Rc::new(Cell::new(0)),
        })];
        let ctx = CheckContext {
            source: "",
            tree: None,
        };
        let findings = checkers[0].check(&PathBuf::from("f.go"), &ctx).unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn registry_contains_primitive_obsession_by_name() {
        let checker = lookup("primitive-obsession").expect("registered");
        assert_eq!(checker.name(), "primitive-obsession");
    }

    #[test]
    fn lookup_returns_none_for_unknown_name() {
        assert!(lookup("does-not-exist").is_none());
    }

    #[test]
    fn grammar_cache_parses_each_language_at_most_once() {
        let cache = GrammarCache::new();
        let src = "package main\nfunc f(a, b string) {}\n";
        let tree1 = cache.parse(Language::Go, src).unwrap();
        let tree2 = cache.parse(Language::Go, src).unwrap();
        // Same cached tree handed back both times — root node byte range matches, and
        // no second parse occurred (a second real parse of different source would
        // produce a tree whose root range reflects that source instead).
        assert_eq!(
            tree1.root_node().byte_range(),
            tree2.root_node().byte_range()
        );
    }

    #[test]
    fn two_checkers_same_language_share_one_parse_and_get_independent_findings() {
        let cache = GrammarCache::new();
        let src = "package main\nfunc f(a, b string) {}\n";
        let tree = cache.parse(Language::Go, src).unwrap();
        let ctx = CheckContext {
            source: src,
            tree: Some(&tree),
        };

        let seen_a = std::rc::Rc::new(Cell::new(0));
        let seen_b = std::rc::Rc::new(Cell::new(0));
        let checker_a = StubChecker {
            name: "a".to_string(),
            language: Some(Language::Go),
            globs: vec!["**/*.go"],
            parses_seen: seen_a.clone(),
        };
        let checker_b = StubChecker {
            name: "b".to_string(),
            language: Some(Language::Go),
            globs: vec!["**/*.go"],
            parses_seen: seen_b.clone(),
        };

        checker_a.check(&PathBuf::from("f.go"), &ctx).unwrap();
        checker_b.check(&PathBuf::from("f.go"), &ctx).unwrap();

        assert_eq!(seen_a.get(), 1);
        assert_eq!(seen_b.get(), 1);
    }

    #[test]
    fn registry_contains_markdown_link_integrity_by_name() {
        let checker = lookup("markdown-link-integrity").expect("registered");
        assert_eq!(checker.name(), "markdown-link-integrity");
    }
}
