//! Shared engine for the `sentinel-return-<lang>` checks (Fowler's Replace Error Code with
//! Exception): flags a `return -1` / `return null` / `return None` written inside a
//! `catch`/`except` block, i.e. a function that swallows a failure and reports it as a
//! magic value its callers must remember to compare against. Not for Go, where a returned
//! error value is the idiom (and `go-ignored-error`/`go-error-context` assume it).
//!
//! v1 flags the sentinel-return pattern alone; call-site cross-referencing is not done.
//! Known false-positive shapes are suppressed (see `docs/sentinel-return-false-positives.md`).
//! Matching is by node-kind string and source text, so one walker serves every grammar.

use anyhow::{Context, Result};
use tree_sitter::Node;

use crate::checker::{CheckContext, Finding};

pub(super) struct LangSpec {
    pub check_name: &'static str,
    pub catch_kinds: &'static [&'static str],
    pub function_kinds: &'static [&'static str],
    /// Kinds whose text starts with `return` (Kotlin has no `return_statement`).
    pub return_kinds: &'static [&'static str],
    /// A declared return type containing one of these marks the function as intentionally
    /// nullable, so a `null` return is its contract rather than a sentinel.
    pub nullable_type_markers: &'static [&'static str],
}

const SEARCH_NAME_MARKERS: &[&str] = &["indexof", "find", "search", "lookup"];
const FACTORY_NAME_PREFIXES: &[&str] = &["create", "make", "build", "new", "of", "from", "valueof"];

pub(super) fn check(spec: &LangSpec, ctx: &CheckContext) -> Result<Vec<Finding>> {
    let tree = ctx
        .tree
        .with_context(|| format!("{} checker requires a parsed tree", spec.check_name))?;
    let src = ctx.source.as_bytes();
    let mut findings = Vec::new();
    crate::tree_walk::walk_preorder(tree.root_node(), &mut |n| {
        if spec.return_kinds.contains(&n.kind())
            && let Some(sentinel) = sentinel_value(n, src)
            && let Some(function) = enclosing_function_if_in_catch(spec, n)
            && !is_suppressed(spec, function, src, sentinel)
        {
            findings.push(Finding {
                line: n.start_position().row + 1,
                message: format!(
                    "returns sentinel `{sentinel}` from an error handler — callers must remember \
                     to check for it; consider throwing instead (Replace Error Code with Exception)"
                ),
            });
        }
        true
    });
    Ok(findings)
}

/// The returned expression's text when it is `null`/`None` or a negative integer literal.
fn sentinel_value<'a>(ret: Node, src: &'a [u8]) -> Option<&'a str> {
    let text = ret.utf8_text(src).ok()?.trim();
    let value = text.strip_prefix("return")?;
    if !value.is_empty() && !value.starts_with(|c: char| c.is_whitespace()) {
        return None;
    }
    let value = value.trim().trim_end_matches(';').trim();
    let is_negative_int = value
        .strip_prefix('-')
        .is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()));
    (value == "null" || value == "None" || is_negative_int).then_some(value)
}

/// Climbs from the return: the nearest function boundary must not come before a catch.
fn enclosing_function_if_in_catch<'a>(spec: &LangSpec, ret: Node<'a>) -> Option<Node<'a>> {
    let mut in_catch = false;
    let mut cur = ret.parent();
    while let Some(n) = cur {
        if spec.function_kinds.contains(&n.kind()) {
            return in_catch.then_some(n);
        }
        if spec.catch_kinds.contains(&n.kind()) {
            in_catch = true;
        }
        cur = n.parent();
    }
    None
}

fn is_suppressed(spec: &LangSpec, function: Node, src: &[u8], sentinel: &str) -> bool {
    if let Some(name) = function_name(function, src) {
        let lower = name.to_lowercase();
        if SEARCH_NAME_MARKERS.iter().any(|m| lower.contains(m)) {
            return true;
        }
        // `tryX` and `read` (InputStream-style `-1` at EOF) return a sentinel by contract.
        if lower.starts_with("try") || lower == "read" {
            return true;
        }
        let is_null = sentinel == "null" || sentinel == "None";
        if is_null && FACTORY_NAME_PREFIXES.iter().any(|p| lower.starts_with(p)) {
            return true;
        }
    }
    let declared = declared_return_type(function, src);
    let is_null = sentinel == "null" || sentinel == "None";
    is_null && declared.is_some_and(|t| spec.nullable_type_markers.iter().any(|m| t.contains(m)))
}

fn function_name<'a>(function: Node, src: &'a [u8]) -> Option<&'a str> {
    if let Some(name) = function.child_by_field_name("name") {
        return name.utf8_text(src).ok();
    }
    // `const f = () => ...` — the name lives on the declarator.
    let parent = function.parent()?;
    parent
        .child_by_field_name("name")
        .and_then(|n| n.utf8_text(src).ok())
}

/// Declared return type text, from the grammar's field or, for Kotlin, the `: Type` that
/// precedes the body.
fn declared_return_type<'a>(function: Node, src: &'a [u8]) -> Option<&'a str> {
    if let Some(t) = function
        .child_by_field_name("return_type")
        .or_else(|| function.child_by_field_name("type"))
    {
        return t.utf8_text(src).ok();
    }
    let mut cursor = function.walk();
    let body_start = function
        .child_by_field_name("body")
        .or_else(|| {
            function
                .children(&mut cursor)
                .find(|c| c.kind() == "function_body")
        })
        .map_or(function.end_byte(), |b| b.start_byte());
    let header = std::str::from_utf8(&src[function.start_byte()..body_start]).ok()?;
    let after_params = header.rsplit_once(')')?.1;
    after_params.split_once(':').map(|(_, t)| t.trim())
}

/// Defines one language's `SentinelReturnChecker` and registers it; each
/// `sentinel_return_<lang>.rs` is a single invocation plus its tests.
macro_rules! sentinel_checker {
    ($name:literal, $lang:expr, $globs:expr, $spec:expr, $desc:literal) => {
        pub struct SentinelReturnChecker;

        const SPEC: $crate::checkers::sentinel_return::LangSpec = $spec;

        impl $crate::checker::Checker for SentinelReturnChecker {
            fn name(&self) -> &str {
                $name
            }
            fn description(&self) -> &str {
                $desc
            }
            fn language(&self) -> Option<$crate::checker::Language> {
                Some($lang)
            }
            fn file_globs(&self) -> &[&str] {
                $globs
            }
            fn check(
                &self,
                _file: &std::path::Path,
                ctx: &$crate::checker::CheckContext,
            ) -> anyhow::Result<Vec<$crate::checker::Finding>> {
                $crate::checkers::sentinel_return::check(&SPEC, ctx)
            }
        }

        inventory::submit! {
            $crate::checker::CheckerFactory(|| vec![Box::new(SentinelReturnChecker)])
        }
    };
}
pub(super) use sentinel_checker;
