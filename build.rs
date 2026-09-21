//! Generates one typed `<Lang>Kind` enum per tree-sitter grammar this crate uses, from a
//! vendored copy of that grammar's own `node-types.json` (`codegen/node-types/*.json`).
//! Every checker that currently matches `node.kind()` against a hand-typed string literal
//! (`"throw_statement"`, `"if_statement"`, ...) can match a `<Lang>Kind` variant instead —
//! a typo'd variant name is a compile error, where a typo'd string literal just silently
//! never matches. See `docs/typed-node-kinds.md` for the migration this unblocks.
//!
//! `node-types.json` is vendored rather than located inside the grammar crate's own
//! registry checkout at build time: none of the `tree-sitter-*` grammar crates declare a
//! `links` key or export `DEP_*` build-script metadata, so there is no portable, public
//! way for this crate's `build.rs` to find another crate's source directory (the registry
//! cache path is real but explicitly not a stable API). Re-vendor the file (and bump the
//! version comment below) if a grammar dependency's version bumps enough to add new node
//! kinds a checker needs.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::Path;

/// One `(module file stem, generated enum name, vendored node-types.json)` triple per
/// grammar — matches `checker::Language`'s variants (TypeScript/Tsx are separate grammars
/// with separate node-types.json, so they get separate enums like that enum already treats
/// them as separate `Language` variants).
const GRAMMARS: &[(&str, &str, &str)] = &[
    ("go", "GoKind", "codegen/node-types/go.json"),
    (
        "typescript",
        "TypeScriptKind",
        "codegen/node-types/typescript.json",
    ),
    ("tsx", "TsxKind", "codegen/node-types/tsx.json"),
    (
        "javascript",
        "JavaScriptKind",
        "codegen/node-types/javascript.json",
    ),
    ("python", "PythonKind", "codegen/node-types/python.json"),
    ("java", "JavaKind", "codegen/node-types/java.json"),
    ("kotlin", "KotlinKind", "codegen/node-types/kotlin.json"),
    ("rust", "RustKind", "codegen/node-types/rust.json"),
];

fn main() {
    let out_dir = env::var("OUT_DIR").expect("OUT_DIR set by cargo");
    for (module, enum_name, json_path) in GRAMMARS {
        println!("cargo:rerun-if-changed={json_path}");
        let raw =
            fs::read_to_string(json_path).unwrap_or_else(|e| panic!("reading {json_path}: {e}"));
        let node_types: Vec<serde_json::Value> =
            serde_json::from_str(&raw).unwrap_or_else(|e| panic!("parsing {json_path}: {e}"));
        let generated = generate_enum(enum_name, &node_types);
        let dest = Path::new(&out_dir).join(format!("{module}_kind.rs"));
        fs::write(&dest, generated).unwrap_or_else(|e| panic!("writing {dest:?}: {e}"));
    }
}

/// Builds the full `enum <name> { ... }` + `impl` source for one grammar's vendored
/// `node-types.json`. Only entries that are both `"named": true` and *not* a `subtypes`
/// grouping (an abstract supertype like Go's `_expression`, which is a grammar-internal
/// category — `Node::kind()` returns the concrete subtype, never the supertype's own name)
/// become a variant, since those are the only strings `Node::kind()` can actually return.
fn generate_enum(enum_name: &str, node_types: &[serde_json::Value]) -> String {
    let kinds = concrete_named_kinds(node_types);
    let mut out = String::new();
    write_enum_decl(&mut out, enum_name, &kinds);
    write_enum_impl(&mut out, enum_name, &kinds);
    out
}

/// Every `(PascalCase variant, raw kind string)` this grammar's `node.kind()` can
/// actually return — see [`generate_enum`]'s doc comment for the `named`/`subtypes`
/// filter. `BTreeMap` so generated output (and variant order) is stable across runs
/// regardless of node-types.json's own array order.
fn concrete_named_kinds(node_types: &[serde_json::Value]) -> BTreeMap<String, String> {
    let mut kinds: BTreeMap<String, String> = BTreeMap::new();
    for entry in node_types {
        let named = entry
            .get("named")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let is_supertype = entry.get("subtypes").is_some();
        if !named || is_supertype {
            continue;
        }
        let Some(kind_str) = entry.get("type").and_then(|v| v.as_str()) else {
            continue;
        };
        let variant = escape_reserved(to_pascal_case(kind_str));
        // A rare grammar-internal collision (two distinct kind strings normalizing to the
        // same identifier) keeps whichever sorts first — acceptable because an
        // unreachable variant is harmless and no grammar in `GRAMMARS` has hit this as
        // of the versions vendored in codegen/node-types/.
        kinds.entry(variant).or_insert_with(|| kind_str.to_string());
    }
    kinds
}

fn write_enum_decl(out: &mut String, enum_name: &str, kinds: &BTreeMap<String, String>) {
    out.push_str("#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]\n");
    out.push_str("#[non_exhaustive]\n");
    out.push_str(&format!("pub enum {enum_name} {{\n"));
    for variant in kinds.keys() {
        out.push_str(&format!("    {variant},\n"));
    }
    out.push_str("    /// A `Node::kind()` string this grammar's vendored node-types.json\n");
    out.push_str("    /// doesn't list as a named, concrete kind — a synthetic tree-sitter\n");
    out.push_str("    /// kind (`ERROR`/`MISSING`) or an anonymous/punctuation token.\n");
    out.push_str("    Other,\n");
    out.push_str("}\n\n");
}

fn write_enum_impl(out: &mut String, enum_name: &str, kinds: &BTreeMap<String, String>) {
    out.push_str(&format!("impl {enum_name} {{\n"));
    out.push_str("    pub fn from_kind_str(s: &str) -> Self {\n");
    out.push_str("        match s {\n");
    for (variant, raw) in kinds {
        out.push_str(&format!("            {raw:?} => Self::{variant},\n"));
    }
    out.push_str("            _ => Self::Other,\n");
    out.push_str("        }\n");
    out.push_str("    }\n\n");
    out.push_str("    pub fn of(node: tree_sitter::Node) -> Self {\n");
    out.push_str("        Self::from_kind_str(node.kind())\n");
    out.push_str("    }\n\n");
    out.push_str("    pub fn as_str(self) -> &'static str {\n");
    out.push_str("        match self {\n");
    for (variant, raw) in kinds {
        out.push_str(&format!("            Self::{variant} => {raw:?},\n"));
    }
    out.push_str("            Self::Other => \"\",\n");
    out.push_str("        }\n");
    out.push_str("    }\n");
    out.push_str("}\n");
}

/// `snake_case` (as tree-sitter grammar node kinds always are) -> `PascalCase` Rust
/// identifier. A leading digit (not expected in practice, but not guaranteed absent from
/// every current or future grammar) gets an `N` prefix, since `1foo` isn't a legal
/// identifier.
fn to_pascal_case(snake: &str) -> String {
    let mut out = String::with_capacity(snake.len());
    let mut capitalize_next = true;
    for ch in snake.chars() {
        if ch == '_' || ch == '-' {
            capitalize_next = true;
            continue;
        }
        if capitalize_next {
            out.extend(ch.to_uppercase());
            capitalize_next = false;
        } else {
            out.push(ch);
        }
    }
    if out.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        out.insert(0, 'N');
    }
    if out.is_empty() {
        out.push_str("Empty");
    }
    out
}

/// A PascalCased grammar node kind can still collide with a Rust keyword usable in type
/// position — e.g. Rust's own grammar has a `self` node kind, and `Self` is a keyword
/// even `r#Self` can't escape (path-relative keywords aren't allowed as raw identifiers).
/// Renamed rather than raw-identifier-escaped for that reason.
fn escape_reserved(variant: String) -> String {
    match variant.as_str() {
        "Self" => "SelfValue".to_string(),
        _ => variant,
    }
}
