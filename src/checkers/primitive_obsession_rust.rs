//! `primitive-obsession-rust`: a struct whose named fields are all raw primitives (`String`,
//! integers, floats, `bool`), at least three of them, at least two distinct types, one a `String`.
//!
//! The Go checker flags parameter piles; the Rust analogue is the data clump in a struct
//! (`struct User { name: String, age: u32, active: bool }`). Skipped to hold the false-positive
//! rate down: `#[cfg(test)]` code, serde/FFI-shaped structs (`Serialize`, `Deserialize`,
//! `#[repr(..)]`), and same-type bundles (`Rgb { r: u8, g: u8, b: u8 }`) that a newtype would not
//! disambiguate. See docs/primitive-obsession-rust-false-positives.md.

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{Context, Result};
use tree_sitter::Node;

use super::rust_common::leading_attrs;
use crate::checker::{CheckContext, Checker, Finding, Language};
use crate::tree_walk::walk_preorder;

const NAME: &str = "primitive-obsession-rust";
const MIN_FIELDS: usize = 3;
const PRIMITIVES: &[&str] = &[
    "String", "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128",
    "isize", "f32", "f64", "bool", "char",
];
const SKIP_ATTRS: &[&str] = &["Serialize", "Deserialize", "repr(", "cfg(test)"];

pub struct PrimitiveObsessionRustChecker;

impl Checker for PrimitiveObsessionRustChecker {
    fn name(&self) -> &str {
        NAME
    }

    fn description(&self) -> &str {
        "flags a Rust struct whose fields are all raw primitives of mixed types"
    }

    fn language(&self) -> Option<Language> {
        Some(Language::Rust)
    }

    fn file_globs(&self) -> &[&str] {
        &["**/*.rs"]
    }

    fn check(&self, _file: &Path, ctx: &CheckContext) -> Result<Vec<Finding>> {
        let tree = ctx
            .tree
            .context("primitive-obsession-rust requires a parsed tree")?;
        let mut findings = Vec::new();
        walk_preorder(tree.root_node(), &mut |node| {
            if node.kind() == "struct_item"
                && let Some(f) = check_struct(node, ctx.source)
            {
                findings.push(f);
            }
            !is_test_module(node, ctx.source)
        });
        Ok(findings)
    }
}

inventory::submit! {
    crate::checker::CheckerFactory(|| vec![Box::new(PrimitiveObsessionRustChecker)])
}

/// A `#[cfg(test)] mod`: its structs are fixtures, not domain types.
fn is_test_module(node: Node, source: &str) -> bool {
    node.kind() == "mod_item" && leading_attrs(node, source).contains("cfg(test)")
}

fn check_struct(node: Node, source: &str) -> Option<Finding> {
    let attrs = leading_attrs(node, source);
    if SKIP_ATTRS.iter().any(|a| attrs.contains(a)) {
        return None;
    }
    let body = node.child_by_field_name("body")?;
    if body.kind() != "field_declaration_list" {
        return None;
    }
    let mut cursor = body.walk();
    let mut types = BTreeSet::new();
    let mut count = 0;
    for field in body
        .children(&mut cursor)
        .filter(|c| c.kind() == "field_declaration")
    {
        let ty = source[field.child_by_field_name("type")?.byte_range()].trim();
        if !PRIMITIVES.contains(&ty) {
            return None;
        }
        types.insert(ty);
        count += 1;
    }
    // Stringly-typed signal: numeric-only records (positions, layout results, iterator state)
    // measured as mostly false positives in the backtest.
    if count < MIN_FIELDS || types.len() < 2 || !types.contains("String") {
        return None;
    }
    let name = node
        .child_by_field_name("name")
        .map_or("", |n| &source[n.byte_range()]);
    Some(Finding {
        line: node.start_position().row + 1,
        message: format!(
            "struct `{name}` has {count} fields that are all raw primitives — consider newtypes/value objects for the ones with domain meaning"
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str) -> Vec<Finding> {
        crate::test_support::check_rust_source(&PrimitiveObsessionRustChecker, src).unwrap()
    }

    #[test]
    fn flags_struct_of_three_mixed_primitives() {
        let f = run("struct User { name: String, age: u32, active: bool }");
        assert_eq!(f.len(), 1);
        assert!(f[0].message.contains("`User`"));
    }

    #[test]
    fn derive_does_not_hide_the_finding() {
        assert_eq!(
            run("#[derive(Debug, Clone)]\nstruct U { a: String, b: u32, c: bool }").len(),
            1
        );
    }

    #[test]
    fn numeric_only_record_is_not_flagged() {
        assert!(run("struct P { a: f64, b: u32, c: bool }").is_empty());
    }

    #[test]
    fn two_fields_are_below_threshold() {
        assert!(run("struct U { a: String, b: u32 }").is_empty());
    }

    #[test]
    fn a_named_type_field_clears_it() {
        assert!(run("struct U { a: String, b: u32, c: Id }").is_empty());
    }

    #[test]
    fn same_type_bundle_is_not_flagged() {
        assert!(run("struct Rgb { r: u8, g: u8, b: u8 }").is_empty());
    }

    #[test]
    fn serde_and_test_structs_are_skipped() {
        assert!(run("#[derive(Deserialize)]\nstruct U { a: String, b: u32, c: bool }").is_empty());
        assert!(run("#[cfg(test)]\nmod t { struct U { a: String, b: u32, c: bool } }").is_empty());
    }

    #[test]
    fn tuple_structs_are_not_checked() {
        assert!(run("struct U(String, u32, bool);").is_empty());
    }
}
