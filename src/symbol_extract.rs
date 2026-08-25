//! Per-language symbol extraction — turns an already-parsed tree-sitter `Tree` into
//! `SymbolNode`s (types, interfaces, functions, methods). Mirrors `rules.rs`'s
//! table-driven `LangRuleConfig`/`lang_config` shape: a per-`Language` table of node-kind
//! strings (`LangSymbolConfig`/`lang_symbol_config`) drives a single generic recursive
//! walker (`extract_symbols_for_file`) instead of hand-rolling per-language walk
//! functions.
//!
//! Node-kind names and field names below were verified against real `to_sexp()` output
//! (Task 1.2.1a), not guessed from grammar docs — see the Pattern Decisions/Pitfalls
//! research this plan cites. Notably:
//! - Go's `type_declaration` wraps one or more `type_spec` children (grouped
//!   `type (...)` blocks produce several); each `type_spec` carries `name` and `type`
//!   fields, and is classified `Interface` iff its `type` field is `interface_type`,
//!   else `Type`.
//! - Go's `function_declaration`/`method_declaration` name fields never include a
//!   trailing `[T any]` type-parameter list — that's a sibling `type_parameters` field,
//!   not part of `name`'s text — so generic-parameter stripping is defensive-only for Go,
//!   not load-bearing. Same for TS: `type_parameters` is a distinct field from `name` on
//!   `class_declaration`/`function_declaration`. `strip_generic_params` is still applied
//!   unconditionally per the Pattern Decisions table, in case a future language's grammar
//!   folds generics into the name text.
//! - TS/JS export detection: an exported declaration is always reached via
//!   `export_statement`'s `declaration` field in this grammar (verified for
//!   interface/class/type-alias/function) — there was no direct-child-without-a-field
//!   case in practice, so `is_exported` just checks the immediate parent's kind.
//!
//! This function takes no `GrammarCache` and does no file I/O — the caller (Epic 1.3's
//! `build_model`, or the LSP `document_symbol` handler) parses the tree and owns the file
//! path; `SymbolNode::file` is left empty (`PathBuf::new()`) here for the caller to fill
//! in, since `extract_symbols_for_file`'s signature (Story 1.2.2) takes no file path.
//!
//! Epic 1.2 (this file) has no real caller yet — `build_model` (Epic 1.3) is the first
//! one, hence the blanket `dead_code` allow below, matching `arch_model.rs`'s precedent.

#![allow(dead_code)]

use std::path::PathBuf;

use tree_sitter::{Node, Tree};

use crate::arch_model::{SymbolKind, SymbolNode};
use crate::checker::Language;

/// Per-`Language` table of node-kind strings driving symbol extraction — the
/// type/interface sibling of `rules.rs`'s `LangRuleConfig`.
struct LangSymbolConfig {
    /// Node kinds that introduce a `Type` symbol on their own (not via `class_declaration`,
    /// which is checked unconditionally regardless of language — see `classify_node`).
    /// Go's `type_declaration` is handled specially (it can wrap multiple `type_spec`
    /// children, each independently classified `Type`/`Interface`), so this list exists
    /// mainly so `Story 1.2.1`'s AC (`type_kinds == &["type_declaration"]` for Go) has a
    /// place to live, and so TS's `type_alias_declaration` has one too.
    type_kinds: &'static [&'static str],
    /// Node kinds that always classify as `Interface` (TS's `interface_declaration`). Go
    /// has none here — Go interfaces are a `type_spec` variant, differentiated inline.
    interface_kinds: &'static [&'static str],
    /// Declaration-like node kinds checked for `Function`/`Method` classification.
    /// Reuses (duplicated, not shared) `rules.rs::lang_config`'s function-kind lists,
    /// since the two tables serve different rule sets and are expected to diverge later.
    /// A kind here without a resolvable `name` field (e.g. an anonymous `arrow_function`)
    /// simply produces no symbol — `classify_node` skips it rather than erroring.
    function_kinds: &'static [&'static str],
    /// Locates a declaration's name node. Field-based (`child_by_field_name("name")`)
    /// for every symbol-producing node kind in Go/TS/Tsx/JS (verified via `to_sexp()`).
    name_finder: fn(Node) -> Option<Node>,
    /// Whether a declaration is exported. Go: uppercase-first-letter of the name text
    /// (second arg is the source, needed to read that text). TS/JS: whether the
    /// immediate parent is an `export_statement`.
    is_exported: fn(Node, &str) -> bool,
}

fn field_name(node: Node) -> Option<Node> {
    node.child_by_field_name("name")
}

fn go_is_exported(node: Node, source: &str) -> bool {
    node.child_by_field_name("name")
        .map(|n| node_text(n, source))
        .and_then(|text| text.chars().next())
        .map(|c| c.is_uppercase())
        .unwrap_or(false)
}

fn js_ts_is_exported(node: Node, _source: &str) -> bool {
    node.parent()
        .map(|p| p.kind() == "export_statement")
        .unwrap_or(false)
}

/// Empty config for languages Epic 1.2 doesn't cover yet (Python/Java/Kotlin — Phase 5).
/// Returns no symbols rather than panicking, so a mixed-language repo containing files in
/// a not-yet-supported language doesn't crash `build_model` (Epic 1.3) once it's wired up.
fn unimplemented_config() -> LangSymbolConfig {
    LangSymbolConfig {
        type_kinds: &[],
        interface_kinds: &[],
        function_kinds: &[],
        name_finder: field_name,
        is_exported: |_, _| false,
    }
}

fn lang_symbol_config(lang: Language) -> LangSymbolConfig {
    match lang {
        Language::Go => LangSymbolConfig {
            type_kinds: &["type_declaration"],
            interface_kinds: &[],
            function_kinds: &["function_declaration", "method_declaration"],
            name_finder: field_name,
            is_exported: go_is_exported,
        },
        Language::TypeScript => LangSymbolConfig {
            type_kinds: &["type_alias_declaration"],
            interface_kinds: &["interface_declaration"],
            function_kinds: &[
                "function_declaration",
                "function_expression",
                "generator_function_declaration",
                "method_definition",
                "arrow_function",
            ],
            name_finder: field_name,
            is_exported: js_ts_is_exported,
        },
        Language::Tsx => lang_symbol_config(Language::TypeScript),
        Language::JavaScript => LangSymbolConfig {
            type_kinds: &[],
            interface_kinds: &[],
            function_kinds: &[
                "function_declaration",
                "function_expression",
                "generator_function_declaration",
                "method_definition",
                "arrow_function",
            ],
            name_finder: field_name,
            is_exported: js_ts_is_exported,
        },
        Language::Python | Language::Java | Language::Kotlin => unimplemented_config(),
    }
}

fn node_text<'a>(node: Node, source: &'a str) -> &'a str {
    &source[node.byte_range()]
}

/// Strips a trailing `[...]`/`<...>` type-parameter list from a raw name, per the
/// Pattern Decisions table's generic-identity rule (`F[T any]` → `F`). Defensive: in
/// every grammar verified for this epic, the `name` field's text never actually includes
/// the type-parameter list (it's a sibling field), but this guards against a future
/// language folding them together.
fn strip_generic_params(name: &str) -> String {
    match name.find(['[', '<']) {
        Some(idx) => name[..idx].to_string(),
        None => name.to_string(),
    }
}

fn build_id(package_path: &str, parent: Option<&str>, name: &str) -> String {
    match parent {
        Some(p) => format!("{package_path}::{p}.{name}"),
        None => format!("{package_path}::{name}"),
    }
}

/// Go method/parent detection: the `receiver` field is a `parameter_list` wrapping one
/// `parameter_declaration`, whose `type` field is either the receiver type's identifier
/// directly (value receiver) or a `pointer_type` wrapping it (pointer receiver).
fn go_receiver_type_name(method: Node, source: &str) -> Option<String> {
    let receiver = method.child_by_field_name("receiver")?;
    let mut cursor = receiver.walk();
    let decl = receiver
        .children(&mut cursor)
        .find(|c| c.kind() == "parameter_declaration")?;
    let ty = decl.child_by_field_name("type")?;
    let ident = if ty.kind() == "pointer_type" {
        ty.named_child(0)?
    } else {
        ty
    };
    Some(node_text(ident, source).to_string())
}

/// TS/JS method/parent detection: walk up ancestors to the enclosing `class_declaration`
/// and read its `name` field.
fn enclosing_class_name(node: Node, source: &str) -> Option<String> {
    let mut cur = node.parent();
    while let Some(n) = cur {
        if n.kind() == "class_declaration" {
            return n
                .child_by_field_name("name")
                .map(|nm| node_text(nm, source).to_string());
        }
        cur = n.parent();
    }
    None
}

/// Go's `type_declaration` can wrap multiple `type_spec` children (a grouped
/// `type (...)` block) — each is classified and emitted independently.
fn go_type_declaration_symbols(
    node: Node,
    source: &str,
    package_path: &str,
    out: &mut Vec<SymbolNode>,
) {
    let mut cursor = node.walk();
    for spec in node
        .children(&mut cursor)
        .filter(|c| c.kind() == "type_spec")
    {
        let Some(name_node) = spec.child_by_field_name("name") else {
            continue;
        };
        let name = strip_generic_params(node_text(name_node, source));
        let kind = match spec.child_by_field_name("type") {
            Some(t) if t.kind() == "interface_type" => SymbolKind::Interface,
            _ => SymbolKind::Type,
        };
        let exported = go_is_exported(spec, source);
        let line = spec.start_position().row + 1;
        let id = build_id(package_path, None, &name);
        out.push(SymbolNode {
            id,
            name,
            kind,
            file: PathBuf::new(),
            line,
            exported,
            parent: None,
        });
    }
}

fn classify_node(
    node: Node,
    language: Language,
    cfg: &LangSymbolConfig,
    source: &str,
    package_path: &str,
) -> Option<SymbolNode> {
    let kind = node.kind();

    let symbol_kind = if kind == "class_declaration" {
        // Universal across TS/Tsx/JS regardless of `type_kinds`/`interface_kinds`
        // membership — Story 1.2.1's AC groups class_declaration under Type for every
        // JS-family language, including JS itself (whose type_kinds/interface_kinds are
        // both empty).
        SymbolKind::Type
    } else if cfg.interface_kinds.contains(&kind) {
        SymbolKind::Interface
    } else if cfg.type_kinds.contains(&kind) {
        SymbolKind::Type
    } else if cfg.function_kinds.contains(&kind) {
        if language == Language::Go {
            if node.child_by_field_name("receiver").is_some() {
                SymbolKind::Method
            } else {
                SymbolKind::Function
            }
        } else if kind == "method_definition" {
            SymbolKind::Method
        } else {
            SymbolKind::Function
        }
    } else {
        return None;
    };

    let name_node = (cfg.name_finder)(node)?;
    let name = strip_generic_params(node_text(name_node, source));
    let exported = (cfg.is_exported)(node, source);
    let line = node.start_position().row + 1;

    let parent = if symbol_kind == SymbolKind::Method {
        if language == Language::Go {
            go_receiver_type_name(node, source)
        } else {
            enclosing_class_name(node, source)
        }
    } else {
        None
    };

    let id = build_id(package_path, parent.as_deref(), &name);

    Some(SymbolNode {
        id,
        name,
        kind: symbol_kind,
        file: PathBuf::new(),
        line,
        exported,
        parent,
    })
}

fn walk(
    node: Node,
    language: Language,
    cfg: &LangSymbolConfig,
    source: &str,
    package_path: &str,
    out: &mut Vec<SymbolNode>,
) {
    if language == Language::Go && node.kind() == "type_declaration" {
        go_type_declaration_symbols(node, source, package_path, out);
    } else if let Some(symbol) = classify_node(node, language, cfg, source, package_path) {
        out.push(symbol);
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk(child, language, cfg, source, package_path, out);
    }
}

/// Walks `tree.root_node()` and returns every in-scope `SymbolNode` — types, interfaces,
/// exported/unexported functions, and methods. Pruning (private-symbol exclusion,
/// generated-file skipping) happens later in `build_model` (Epic 1.3), not here: both
/// exported and unexported symbols are always returned.
///
/// No file I/O, no `GrammarCache` — the caller parses `source` into `tree` and owns the
/// file path (`SymbolNode::file` is left as `PathBuf::new()` here).
pub fn extract_symbols_for_file(
    language: Language,
    source: &str,
    tree: &Tree,
    package_path: &str,
) -> Vec<SymbolNode> {
    let cfg = lang_symbol_config(language);
    let mut symbols = Vec::new();
    walk(
        tree.root_node(),
        language,
        &cfg,
        source,
        package_path,
        &mut symbols,
    );
    symbols
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checker::GrammarCache;

    fn extract(language: Language, source: &str, package_path: &str) -> Vec<SymbolNode> {
        let cache = GrammarCache::new();
        let tree = cache.parse(language, source).expect("parses");
        extract_symbols_for_file(language, source, &tree, package_path)
    }

    fn find_by_name<'a>(symbols: &'a [SymbolNode], name: &str) -> &'a SymbolNode {
        symbols
            .iter()
            .find(|s| s.name == name)
            .unwrap_or_else(|| panic!("no symbol named {name} in {symbols:?}"))
    }

    // --- Story 1.2.1: LangSymbolConfig table ---

    #[test]
    fn lang_symbol_config_go_type_kinds_and_export_detection() {
        let cfg = lang_symbol_config(Language::Go);
        assert_eq!(cfg.type_kinds, &["type_declaration"]);

        let symbols = extract(Language::Go, "package pkg\n\ntype Foo struct{}\n", "pkg");
        assert!(find_by_name(&symbols, "Foo").exported);

        let symbols = extract(Language::Go, "package pkg\n\ntype foo struct{}\n", "pkg");
        assert!(!find_by_name(&symbols, "foo").exported);
    }

    #[test]
    fn go_interface_declaration_extracts_as_interface_symbol() {
        let symbols = extract(
            Language::Go,
            "package pkg\n\ntype Reader interface {\n\tRead() error\n}\n",
            "pkg",
        );
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].kind, SymbolKind::Interface);
        assert_eq!(symbols[0].name, "Reader");
        assert!(symbols[0].exported);
    }

    #[test]
    fn ts_exported_interface_extracts_as_interface_symbol() {
        let symbols = extract(
            Language::TypeScript,
            "export interface Shape {\n  area(): number;\n}\n",
            "pkg",
        );
        let shape = find_by_name(&symbols, "Shape");
        assert_eq!(shape.kind, SymbolKind::Interface);
        assert!(shape.exported);
    }

    #[test]
    fn js_class_extracts_as_type_symbol_no_interface_or_type_alias_kinds() {
        let cfg = lang_symbol_config(Language::JavaScript);
        assert!(cfg.type_kinds.is_empty());
        assert!(cfg.interface_kinds.is_empty());

        let symbols = extract(Language::JavaScript, "class Foo {}\n", "pkg");
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].kind, SymbolKind::Type);
        assert_eq!(symbols[0].name, "Foo");
    }

    // --- Story 1.2.2: extract_symbols_for_file ---

    #[test]
    fn extract_symbols_for_file_returns_both_exported_and_unexported_go_functions() {
        let symbols = extract(
            Language::Go,
            "package pkg\n\nfunc Do() {}\nfunc do() {}\n",
            "pkg",
        );
        assert_eq!(symbols.len(), 2);
        assert!(find_by_name(&symbols, "Do").exported);
        assert!(!find_by_name(&symbols, "do").exported);
    }

    #[test]
    fn extract_symbols_for_file_builds_owner_qualified_method_ids() {
        let symbols = extract(
            Language::Go,
            "package pkg\n\ntype T struct{}\n\nfunc (t T) M() {}\n",
            "pkg",
        );
        let m = find_by_name(&symbols, "M");
        assert_eq!(m.kind, SymbolKind::Method);
        assert_eq!(m.parent, Some("T".to_string()));
        assert_eq!(m.id, "pkg::T.M");
    }

    #[test]
    fn extract_symbols_for_file_disambiguates_same_named_methods_on_different_types() {
        let symbols = extract(
            Language::Go,
            "package pkg\n\ntype A struct{}\ntype B struct{}\n\nfunc (a A) Close() {}\nfunc (b B) Close() {}\n",
            "pkg",
        );
        let ids: Vec<&str> = symbols
            .iter()
            .filter(|s| s.name == "Close")
            .map(|s| s.id.as_str())
            .collect();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&"pkg::A.Close"));
        assert!(ids.contains(&"pkg::B.Close"));
    }

    #[test]
    fn extract_symbols_for_file_strips_generic_type_parameters_from_name() {
        let symbols = extract(
            Language::Go,
            "package pkg\n\nfunc F[T any](x T) T { return x }\n",
            "pkg",
        );
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "F");
    }

    #[test]
    fn extract_symbols_for_file_builds_plain_id_for_free_function() {
        let symbols = extract(
            Language::Go,
            "package pkg\n\nfunc Compute() {}\n",
            "app/domain",
        );
        let compute = find_by_name(&symbols, "Compute");
        assert_eq!(compute.parent, None);
        assert_eq!(compute.id, "app/domain::Compute");
    }

    #[test]
    fn extract_symbols_for_file_pointer_receiver_resolves_same_parent_as_value_receiver() {
        let symbols = extract(
            Language::Go,
            "package pkg\n\ntype T struct{}\n\nfunc (t *T) N() {}\n",
            "pkg",
        );
        let n = find_by_name(&symbols, "N");
        assert_eq!(n.parent, Some("T".to_string()));
        assert_eq!(n.id, "pkg::T.N");
    }

    // TS/JS parity pair (Task 1.2.2e)

    #[test]
    fn extract_symbols_for_file_ts_class_method_gets_owner_qualified_id() {
        let symbols = extract(
            Language::TypeScript,
            "export class Widget {\n  render(): void {}\n}\n",
            "pkg",
        );
        let widget = find_by_name(&symbols, "Widget");
        assert_eq!(widget.kind, SymbolKind::Type);
        assert!(widget.exported);

        let render = find_by_name(&symbols, "render");
        assert_eq!(render.kind, SymbolKind::Method);
        assert_eq!(render.parent, Some("Widget".to_string()));
        assert_eq!(render.id, "pkg::Widget.render");
    }

    #[test]
    fn extract_symbols_for_file_js_class_method_gets_owner_qualified_id() {
        let symbols = extract(Language::JavaScript, "class Foo {\n  bar() {}\n}\n", "pkg");

        let foo = find_by_name(&symbols, "Foo");
        assert_eq!(foo.kind, SymbolKind::Type);
        assert!(!foo.exported);

        let bar = find_by_name(&symbols, "bar");
        assert_eq!(bar.kind, SymbolKind::Method);
        assert_eq!(bar.parent, Some("Foo".to_string()));
        assert_eq!(bar.id, "pkg::Foo.bar");
    }

    #[test]
    fn extract_symbols_for_file_ts_type_alias_extracts_as_type() {
        let symbols = extract(
            Language::TypeScript,
            "export type Foo = { x: number };\n",
            "pkg",
        );
        let foo = find_by_name(&symbols, "Foo");
        assert_eq!(foo.kind, SymbolKind::Type);
        assert!(foo.exported);
    }

    #[test]
    fn extract_symbols_for_file_tsx_shares_typescript_config() {
        let symbols = extract(
            Language::Tsx,
            "export interface Shape {\n  area(): number;\n}\n",
            "pkg",
        );
        let shape = find_by_name(&symbols, "Shape");
        assert_eq!(shape.kind, SymbolKind::Interface);
        assert!(shape.exported);
    }
}
