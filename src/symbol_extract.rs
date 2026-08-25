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
//! This function takes no `GrammarCache` and does no file I/O — the caller (`arch_model.rs`'s
//! `build_model`, or `lsp.rs`'s `document_symbols_for_file`) parses the tree and owns the
//! file path; `SymbolNode::file` is left empty (`PathBuf::new()`) here for the caller to fill
//! in, since `extract_symbols_for_file`'s signature (Story 1.2.2) takes no file path.

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

/// Python: exported iff the name doesn't start with `_` (PEP 8 module-level convention).
fn python_is_exported(node: Node, source: &str) -> bool {
    node.child_by_field_name("name")
        .map(|n| node_text(n, source))
        .map(|text| !text.starts_with('_'))
        .unwrap_or(false)
}

/// Returns `node`'s first direct (positional, not field-based) child of the given kind —
/// the same "don't assume a field exists, look it up by kind" workaround `rules.rs`'s
/// `kotlin_body`/`kotlin_params` establish, applied here to Java/Kotlin's `modifiers`
/// node (verified via `to_sexp()`/`field_name_for_child`: `modifiers` is a positional
/// child of `class_declaration`/`interface_declaration`/`method_declaration`, not a
/// field — e.g. `(class_declaration (modifiers (public)) name: ... )`).
fn find_child_by_kind<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    node.children(&mut cursor).find(|c| c.kind() == kind)
}

/// Java: exported iff a `public` modifier is present among `modifiers`'s children.
/// Package-private/no-modifier declarations are `exported: false`.
fn java_is_exported(node: Node, _source: &str) -> bool {
    find_child_by_kind(node, "modifiers")
        .map(|m| find_child_by_kind(m, "public").is_some())
        .unwrap_or(false)
}

/// Kotlin's default visibility is public (the inverse of Java's default) — exported iff
/// neither a `private` nor `internal` `visibility_modifier` is present. Verified via
/// `to_sexp()`: `modifiers` wraps a `visibility_modifier` node whose own child is the
/// literal keyword (`private`/`internal`/`public`); a declaration with no `modifiers`
/// child at all has no visibility keyword in source and is public by default.
fn kotlin_is_exported(node: Node, _source: &str) -> bool {
    let Some(modifiers) = find_child_by_kind(node, "modifiers") else {
        return true;
    };
    let mut cursor = modifiers.walk();
    !modifiers
        .children(&mut cursor)
        .filter(|c| c.kind() == "visibility_modifier")
        .any(|vis| {
            find_child_by_kind(vis, "private").is_some()
                || find_child_by_kind(vis, "internal").is_some()
        })
}

/// Kotlin has no distinct `interface_declaration` node kind — `interface` is a
/// positional keyword child of `class_declaration` (`kind() == "interface"`) rather than
/// a modifier or field, verified via `to_sexp()` (this was explicitly *not* assumed, per
/// the pitfalls research's warning that Kotlin already broke one assumption in
/// `rules.rs`). A plain class's first keyword child is `"class"` instead.
fn kotlin_is_interface(node: Node) -> bool {
    find_child_by_kind(node, "interface").is_some()
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
        Language::Python => LangSymbolConfig {
            // `class_definition` is checked here (not via the universal
            // `class_declaration` branch `classify_node` special-cases — Python's kind
            // literal is different) — matching JS's `interface_kinds`-empty precedent
            // since Python has no first-class interface node (`Protocol` is a library
            // convention, not grammar-level).
            type_kinds: &["class_definition"],
            interface_kinds: &[],
            function_kinds: &["function_definition"],
            name_finder: field_name,
            is_exported: python_is_exported,
        },
        Language::Java => LangSymbolConfig {
            // `class_declaration` is redundantly listed even though `classify_node`'s
            // universal branch already classifies it as `Type` — self-documents Story
            // 5.2.1's AC directly in the table.
            type_kinds: &[
                "class_declaration",
                "enum_declaration",
                "record_declaration",
            ],
            interface_kinds: &["interface_declaration"],
            // Java has no free functions — every `method_declaration` classifies as
            // `Method` in `classify_node`'s language-specific branch below, never
            // `Function`.
            function_kinds: &["method_declaration"],
            name_finder: field_name,
            is_exported: java_is_exported,
        },
        Language::Kotlin => LangSymbolConfig {
            // Kotlin's `class_declaration` also covers interfaces (see
            // `kotlin_is_interface`) — `classify_node`'s universal
            // `kind == "class_declaration"` branch is guarded by a Kotlin-specific check
            // before defaulting to `Type`, so `type_kinds`/`interface_kinds` are both left
            // empty here; the real dispatch lives in `classify_node`.
            type_kinds: &[],
            interface_kinds: &[],
            function_kinds: &["function_declaration"],
            name_finder: field_name,
            is_exported: kotlin_is_exported,
        },
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

/// Accepted v1 limitation (Epic 5.2 goal, `plan.md`): this owner-qualified id scheme
/// resolves same-named methods on *different* types, but doesn't disambiguate Java
/// method **overloading** — two methods with the same name and parent but different
/// parameter lists (e.g. `save(String)`/`save(String, int)` on one class) collide on the
/// same id, since arity/parameter types aren't part of it. Whichever overload
/// `extract_symbols_for_file` visits last for a given `parent`/`name` pair is the one
/// that survives in `PackageNode::symbols` (last-extraction-wins) — an explicit v1
/// decision, not an undiscovered gap. Not fixed speculatively; revisit only if a real
/// ambiguous-lookup report comes in.
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

/// Method/parent detection for TS/JS/Python/Java/Kotlin: walk up ancestors (skipping
/// transparently through wrapper nodes like Python's `decorated_definition`, since we
/// only test each ancestor's own `kind()`) until one matches `target_kinds`, then read
/// its `name` field. TS/JS pass `&["class_declaration"]`; Python passes
/// `&["class_definition"]`; Java passes every type-declaration kind a method can live in
/// (`class_declaration`/`interface_declaration`/`enum_declaration`/`record_declaration`);
/// Kotlin passes `&["class_declaration"]` (covers both class and interface, since Kotlin
/// has no distinct interface node kind — see `kotlin_is_interface`).
fn enclosing_kind_name(node: Node, source: &str, target_kinds: &[&str]) -> Option<String> {
    let mut cur = node.parent();
    while let Some(n) = cur {
        if target_kinds.contains(&n.kind()) {
            return n
                .child_by_field_name("name")
                .map(|nm| node_text(nm, source).to_string());
        }
        cur = n.parent();
    }
    None
}

const JAVA_TYPE_KINDS: &[&str] = &[
    "class_declaration",
    "interface_declaration",
    "enum_declaration",
    "record_declaration",
];

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
        // Universal across TS/Tsx/JS/Java regardless of `type_kinds`/`interface_kinds`
        // membership — Story 1.2.1's AC groups class_declaration under Type for every
        // JS-family language, including JS itself (whose type_kinds/interface_kinds are
        // both empty). Kotlin is the one exception: it has no distinct
        // `interface_declaration` node kind, so a `class_declaration` whose first
        // positional child is the `interface` keyword must classify as `Interface`
        // instead (see `kotlin_is_interface`) — guarded here rather than folded into the
        // universal case, since every other language's `class_declaration` is never an
        // interface.
        if language == Language::Kotlin && kotlin_is_interface(node) {
            SymbolKind::Interface
        } else {
            SymbolKind::Type
        }
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
        } else if language == Language::Java {
            // Java has no free functions — `method_declaration` is always a Method,
            // never a Function; every one has a parent type (accepted v1 limitation:
            // overloaded methods on the same type collide on this owner-qualified id —
            // see the module-level note near `build_id`/Epic 5.2's goal in plan.md,
            // last-extraction-wins in `PackageNode::symbols`, not fixed for v1).
            SymbolKind::Method
        } else if language == Language::Kotlin {
            if enclosing_kind_name(node, source, &["class_declaration"]).is_some() {
                SymbolKind::Method
            } else {
                SymbolKind::Function
            }
        } else if language == Language::Python {
            if enclosing_kind_name(node, source, &["class_definition"]).is_some() {
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
        match language {
            Language::Go => go_receiver_type_name(node, source),
            Language::Java => enclosing_kind_name(node, source, JAVA_TYPE_KINDS),
            Language::Kotlin => enclosing_kind_name(node, source, &["class_declaration"]),
            Language::Python => enclosing_kind_name(node, source, &["class_definition"]),
            _ => enclosing_kind_name(node, source, &["class_declaration"]),
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
    fn go_grouped_type_declaration_emits_a_symbol_per_type_spec() {
        let symbols = extract(
            Language::Go,
            "package pkg\n\ntype (\n\tA struct{}\n\tB interface{ M() }\n)\n",
            "pkg",
        );
        assert_eq!(symbols.len(), 2, "got: {symbols:?}");
        assert_eq!(find_by_name(&symbols, "A").kind, SymbolKind::Type);
        assert_eq!(find_by_name(&symbols, "B").kind, SymbolKind::Interface);
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

    // --- Epic 5.1: Python ---

    #[test]
    fn python_class_and_nested_method_extract_with_owner_qualified_parent() {
        let symbols = extract(
            Language::Python,
            "class Widget:\n    def render(self): pass\n",
            "pkg",
        );
        let widget = find_by_name(&symbols, "Widget");
        assert_eq!(widget.kind, SymbolKind::Type);

        let render = find_by_name(&symbols, "render");
        assert_eq!(render.kind, SymbolKind::Method);
        assert_eq!(render.parent, Some("Widget".to_string()));
    }

    #[test]
    fn python_underscore_prefixed_function_is_not_exported() {
        let symbols = extract(Language::Python, "def _helper(): pass\n", "pkg");
        assert!(!find_by_name(&symbols, "_helper").exported);
    }

    #[test]
    fn python_decorated_class_definition_is_still_extracted_as_type() {
        let symbols = extract(
            Language::Python,
            "@dataclass\nclass Point:\n    x: int\n",
            "pkg",
        );
        let point = find_by_name(&symbols, "Point");
        assert_eq!(point.kind, SymbolKind::Type);
    }

    #[test]
    fn python_has_no_interface_kind() {
        let cfg = lang_symbol_config(Language::Python);
        assert!(cfg.interface_kinds.is_empty());
    }

    // --- Epic 5.2: Java ---

    #[test]
    fn java_public_interface_and_its_abstract_method_extract_correctly() {
        let symbols = extract(
            Language::Java,
            "public interface Shape { double area(); }",
            "com.acme.app",
        );
        let shape = find_by_name(&symbols, "Shape");
        assert_eq!(shape.kind, SymbolKind::Interface);
        assert!(shape.exported);

        let area = find_by_name(&symbols, "area");
        assert_eq!(area.kind, SymbolKind::Method);
        assert_eq!(area.parent, Some("Shape".to_string()));
    }

    #[test]
    fn java_class_without_public_modifier_is_not_exported() {
        let symbols = extract(Language::Java, "class Helper { void run() {} }", "pkg");
        assert!(!find_by_name(&symbols, "Helper").exported);
    }

    #[test]
    fn java_record_extracts_as_type_symbol() {
        let symbols = extract(Language::Java, "record Point(int x, int y) {}", "pkg");
        let point = find_by_name(&symbols, "Point");
        assert_eq!(point.kind, SymbolKind::Type);
    }

    #[test]
    fn java_never_produces_a_function_kind() {
        let symbols = extract(Language::Java, "class Helper { void run() {} }", "pkg");
        assert!(symbols.iter().all(|s| s.kind != SymbolKind::Function));
    }

    // --- Epic 5.3: Kotlin ---

    #[test]
    fn kotlin_interface_extracts_as_interface_symbol_not_type() {
        let symbols = extract(
            Language::Kotlin,
            "interface Repository {\n    fun save()\n}\n",
            "pkg",
        );
        let repo = find_by_name(&symbols, "Repository");
        assert_eq!(repo.kind, SymbolKind::Interface);
    }

    #[test]
    fn kotlin_private_class_is_not_exported_but_default_visibility_is() {
        let symbols = extract(
            Language::Kotlin,
            "private class Internal\nclass Public\n",
            "pkg",
        );
        assert!(!find_by_name(&symbols, "Internal").exported);
        assert!(find_by_name(&symbols, "Public").exported);
    }

    #[test]
    fn kotlin_method_in_class_gets_owner_qualified_parent() {
        let symbols = extract(
            Language::Kotlin,
            "class Box {\n    fun open(): Unit {}\n}\n",
            "pkg",
        );
        let open = find_by_name(&symbols, "open");
        assert_eq!(open.kind, SymbolKind::Method);
        assert_eq!(open.parent, Some("Box".to_string()));
    }

    #[test]
    fn kotlin_top_level_function_classifies_as_function_not_method() {
        let symbols = extract(Language::Kotlin, "fun topLevel() {}\n", "pkg");
        let f = find_by_name(&symbols, "topLevel");
        assert_eq!(f.kind, SymbolKind::Function);
        assert_eq!(f.parent, None);
    }
}
