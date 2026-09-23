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

use std::cell::Cell;
use std::path::PathBuf;

use tree_sitter::{Node, Tree};

use crate::arch_model::{AccessKind, SymbolKind, SymbolNode, TypeRelationKind};
use crate::checker::Language;
use crate::node_kind::{
    GoKind, JavaKind, JavaScriptKind, KotlinKind, RustKind, TsxKind, TypeScriptKind,
};

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

// SEAM(typed-node-kind-migration): `export_statement` is shared TypeScript/Tsx/JavaScript
// vocabulary reached from one function body — see ADR-001.
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
/// Non-goal (typed-node-kind-migration, Story 3.2.1 — see ADR-001): intentionally
/// generic over `kind: &str` rather than a `<Lang>Kind`, since it's shared by
/// `java_is_exported`/`kotlin_is_exported`/`kotlin_is_interface` below for both named
/// container nodes (`"modifiers"`/`"visibility_modifier"`, both `"named": true`) and the
/// anonymous keyword tokens they wrap (`"public"`/`"private"`/`"internal"`/`"interface"`,
/// all `"named": false` in `java.json`/`kotlin.json` — confirmed by inspecting both
/// files). Typing only the container-node calls would leave the actual decision-bearing
/// keyword check as a raw string anyway, so the whole helper stays generic.
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
/// `rules.rs`). A plain class's first keyword child is `"class"` instead. Non-goal, same
/// as `java_is_exported`/`kotlin_is_exported` above: `"interface"` is `"named": false` in
/// `kotlin.json`, so there's no `KotlinKind` variant to convert this to.
fn kotlin_is_interface(node: Node) -> bool {
    find_child_by_kind(node, "interface").is_some()
}

/// Rust: exported iff a plain, unrestricted `pub` is present. `pub(crate)`/
/// `pub(super)`/`pub(in path)` wrap a further named child inside `visibility_modifier`
/// (verified via `to_sexp()`: `pub(crate) enum Color` produces `(visibility_modifier
/// (crate))`, `pub(super) fn f` produces `(visibility_modifier (super))`, plain `pub fn`
/// produces an empty `(visibility_modifier)`) — those restricted forms don't count as
/// truly exported, the same restricted-visibility exclusion Kotlin's `internal` gets. No
/// `visibility_modifier` child at all means private (Rust's default), same as Go's
/// lowercase-first-letter default.
fn rust_is_exported(node: Node, _source: &str) -> bool {
    let mut cursor = node.walk();
    match node
        .children(&mut cursor)
        .find(|c| RustKind::of(*c) == RustKind::VisibilityModifier)
    {
        Some(vis) => vis.named_child_count() == 0,
        None => false,
    }
}

/// Rust has no `interface`/`class` keyword marking a `trait_item` as special the way
/// Kotlin's `class_declaration` needs disambiguating — `trait_item` is already its own
/// distinct node kind (see `interface_kinds` below), so no analogous helper is needed.
///
/// Walks up to the enclosing `impl_item` (if any) and reads its `type` field — the Self
/// type an impl block implements methods for (e.g. `S` in both `impl S` and `impl Trait
/// for S`, verified via `to_sexp()`: both produce a `type` field, `impl_item` additionally
/// carries a `trait` field only for the latter). Unlike every other language's
/// method/parent lookup (which reads an ancestor's `name` field), this reads `type` —
/// `impl_item` has no name of its own. `strip_generic_params` handles a generic Self type
/// (`impl<T> Foo<T>` has `type: (generic_type type: (type_identifier) ...)`, whose full
/// text is `Foo<T>`) the same way it handles a generic function name elsewhere.
fn rust_impl_type_name(node: Node, source: &str) -> Option<String> {
    let mut cur = node.parent();
    while let Some(n) = cur {
        if RustKind::of(n) == RustKind::ImplItem {
            return n
                .child_by_field_name("type")
                .map(|t| strip_generic_params(node_text(t, source)));
        }
        cur = n.parent();
    }
    None
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
        Language::Rust => LangSymbolConfig {
            type_kinds: &["struct_item", "enum_item", "union_item"],
            interface_kinds: &["trait_item"],
            // `function_item` covers free functions and impl/trait methods alike (one
            // node kind — verified via `to_sexp()`); `classify_node`'s Rust branch below
            // tells them apart by walking up for an enclosing `impl_item`, the same
            // enclosing-ancestor pattern Python/Kotlin use for `class_definition`/
            // `class_declaration`. A trait method *declaration* with no body
            // (`function_signature_item`) is a distinct kind, deliberately excluded —
            // it produces no symbol, matching Go interface methods never appearing here.
            function_kinds: &["function_item"],
            name_finder: field_name,
            is_exported: rust_is_exported,
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
        .find(|c| GoKind::of(*c) == GoKind::ParameterDeclaration)?;
    let ty = decl.child_by_field_name("type")?;
    let ident = if GoKind::of(ty) == GoKind::PointerType {
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
// SEAM(typed-node-kind-migration): `target_kinds` is a per-caller kind-name slice shared
// across TS/JS/Python/Java/Kotlin call sites, each spelling a different grammar's kind —
// see ADR-001.
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
        .filter(|c| GoKind::of(*c) == GoKind::TypeSpec)
    {
        let Some(name_node) = spec.child_by_field_name("name") else {
            continue;
        };
        let name = strip_generic_params(node_text(name_node, source));
        let kind = match spec.child_by_field_name("type") {
            Some(t) if GoKind::of(t) == GoKind::InterfaceType => SymbolKind::Interface,
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
            // Branch guard already guarantees a Go node — `receiver` is a field lookup,
            // not a `.kind()` comparison, so there's nothing else here to type.
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
        } else if language == Language::Rust {
            if rust_impl_type_name(node, source).is_some() {
                SymbolKind::Method
            } else {
                SymbolKind::Function
            }
        } else if language == Language::JavaScript {
            // Was a raw `kind == "method_definition"` fallback reached by process of
            // elimination (only TypeScript/Tsx/JavaScript remain once Go/Java/Kotlin/
            // Python/Rust are excluded above) — split into explicit per-language branches
            // here so each gets its own grammar's typed `Kind`, matching the branches
            // above rather than leaving one raw comparison behind.
            if JavaScriptKind::of(node) == JavaScriptKind::MethodDefinition {
                SymbolKind::Method
            } else {
                SymbolKind::Function
            }
        } else if language == Language::Tsx {
            if TsxKind::of(node) == TsxKind::MethodDefinition {
                SymbolKind::Method
            } else {
                SymbolKind::Function
            }
        } else {
            // Only Language::TypeScript remains here (Language is an 8-variant enum and
            // the other 7 are all excluded above).
            if TypeScriptKind::of(node) == TypeScriptKind::MethodDefinition {
                SymbolKind::Method
            } else {
                SymbolKind::Function
            }
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
            Language::Rust => rust_impl_type_name(node, source),
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
    if language == Language::Go && GoKind::of(node) == GoKind::TypeDeclaration {
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

/// One call site found inside a function/method body: the id of the enclosing
/// declaration (`caller_id`, built the same way `classify_node` builds `SymbolNode::id`,
/// so it's directly comparable) and the raw, unresolved text of the call's target —
/// resolving that text to a callee `SymbolNode::id` is `arch_model::build_model`'s job
/// (it alone has the whole-repo symbol index needed to disambiguate a bare name).
///
/// `file` is left as `PathBuf::new()` here, same convention as `SymbolNode::file` —
/// the caller (`arch_model::build_model`) fills it in.
#[derive(Debug, Clone, PartialEq)]
pub struct RawCallSite {
    pub caller_id: String,
    pub callee_text: String,
    pub file: PathBuf,
    pub line: usize,
}

/// Call-graph extraction is scoped to Go and TS/JS for v1 (see the issue's "reuse
/// existing per-language coverage" note) — every other `Language` variant returns no
/// call sites here even though `symbol_extract_for_file` already covers it.
fn call_graph_supports(language: Language) -> bool {
    matches!(
        language,
        Language::Go | Language::TypeScript | Language::Tsx | Language::JavaScript
    )
}

/// Reads a `call_expression`'s target text: the bare identifier for `Foo()`, or the full
/// qualified text for `pkg.Foo()`/`recv.Method()` — kept qualified (not trimmed to the
/// last segment here) so `arch_model::resolve_call_edges` can itself tell a bare call
/// from a qualified one and pick which symbol index to search first.
// SEAM(typed-node-kind-migration): the single hardest snippet in the whole migration —
// this match arm unions Go's `selector_expression` and JS/TS's `member_expression`
// vocabularies in one arm, which no single `<Lang>Kind` enum can express without a shared
// trait; deliberately excluded per ADR-001's Option C — see ADR-001.
fn callee_text_for(call: Node, source: &str) -> Option<String> {
    let function = call.child_by_field_name("function")?;
    match function.kind() {
        "identifier" | "selector_expression" | "member_expression" => {
            Some(node_text(function, source).to_string())
        }
        _ => None,
    }
}

/// Bundles `walk_calls`' per-file constants (language/config/source/package) so the
/// recursive walk itself only threads the two things that actually change per call
/// (`node`, `caller`) plus the output sink.
struct CallWalkCtx<'a> {
    language: Language,
    cfg: &'a LangSymbolConfig,
    source: &'a str,
    package_path: &'a str,
}

/// Recursive walk pairing each `call_expression` with its nearest *named* enclosing
/// function/method — an anonymous callback (e.g. a JS arrow function with no `name`
/// field) doesn't get its own caller context, so calls inside it attribute to whichever
/// named declaration encloses it instead, matching `classify_node`'s existing "no
/// resolvable name → no symbol" behavior for such nodes.
fn walk_calls(node: Node, ctx: &CallWalkCtx, caller: Option<&str>, out: &mut Vec<RawCallSite>) {
    let mut current_caller = caller.map(str::to_string);
    // SEAM(typed-node-kind-migration): `function_kinds` is `LangSymbolConfig`'s seamed
    // `&'static [&'static str]` field, shared across all languages by this one function
    // body — see ADR-001.
    if ctx.cfg.function_kinds.contains(&node.kind())
        && let Some(sym) = classify_node(node, ctx.language, ctx.cfg, ctx.source, ctx.package_path)
    {
        current_caller = Some(sym.id);
    }

    // SEAM(typed-node-kind-migration): `call_expression` is shared Go/TypeScript/Tsx/
    // JavaScript vocabulary reached from one function body (per `call_graph_supports`)
    // — see ADR-001. Surfaced during Story 4.1.1's completeness sweep, alongside the
    // already-seamed `function_kinds.contains` check just above.
    if node.kind() == "call_expression"
        && let Some(caller_id) = &current_caller
        && let Some(callee_text) = callee_text_for(node, ctx.source)
    {
        out.push(RawCallSite {
            caller_id: caller_id.clone(),
            callee_text,
            file: PathBuf::new(),
            line: node.start_position().row + 1,
        });
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_calls(child, ctx, current_caller.as_deref(), out);
    }
}

/// Walks `tree.root_node()` and returns every call site found inside a named
/// function/method body, paired with its enclosing declaration's `SymbolNode::id`. Empty
/// for a language `call_graph_supports` doesn't cover yet (see its doc comment). No file
/// I/O; `RawCallSite::file` is left empty for the caller to fill in, same convention as
/// `extract_symbols_for_file`.
pub fn extract_call_sites_for_file(
    language: Language,
    source: &str,
    tree: &Tree,
    package_path: &str,
) -> Vec<RawCallSite> {
    if !call_graph_supports(language) {
        return Vec::new();
    }
    let cfg = lang_symbol_config(language);
    let ctx = CallWalkCtx {
        language,
        cfg: &cfg,
        source,
        package_path,
    };
    let mut sites = Vec::new();
    walk_calls(tree.root_node(), &ctx, None, &mut sites);
    sites
}

// ---------------------------------------------------------------------------------
// Field access extraction (#39) — a method↔field usage graph, parallel to the
// method↔method call graph above. Go-only for v1 (see `field_access_supports`).
// ---------------------------------------------------------------------------------

/// Go receiver parameter's own bound identifier (`recv` in `func (recv *T) M()`), distinct
/// from `go_receiver_type_name`'s type text. `None` for an unnamed receiver
/// (`func (T) M()`) — such a method can't contain any `recv.Field` selector at all.
fn go_receiver_var_name(method: Node, source: &str) -> Option<String> {
    let receiver = method.child_by_field_name("receiver")?;
    let mut cursor = receiver.walk();
    let decl = receiver
        .children(&mut cursor)
        .find(|c| GoKind::of(*c) == GoKind::ParameterDeclaration)?;
    let name = decl.child_by_field_name("name")?;
    Some(node_text(name, source).to_string())
}

/// Appends every `(type_name, field_name)` pair declared directly on a Go struct type
/// under `type_decl` (a `type_declaration` node, possibly a grouped `type (...)` block —
/// each `type_spec` child is handled independently, matching
/// `go_type_declaration_symbols`'s precedent). The grammar's `commaSep1(field('name', ...))`
/// means one `field_declaration` can carry more than one `name` child for a shared-type
/// group (`X, Y int`), so every `name`-field child is collected, not just the first. An
/// embedded field (anonymous — no `name` field at all) is skipped: embedding introduces
/// promoted fields/methods this v1 extraction doesn't walk into.
fn go_struct_fields(type_decl: Node, source: &str, out: &mut Vec<(String, String)>) {
    let mut cursor = type_decl.walk();
    for spec in type_decl
        .children(&mut cursor)
        .filter(|c| GoKind::of(*c) == GoKind::TypeSpec)
    {
        let Some(name_node) = spec.child_by_field_name("name") else {
            continue;
        };
        let Some(struct_ty) = spec.child_by_field_name("type") else {
            continue;
        };
        if GoKind::of(struct_ty) != GoKind::StructType {
            continue;
        }
        let type_name = strip_generic_params(node_text(name_node, source));
        let mut sc = struct_ty.walk();
        let Some(field_list) = struct_ty
            .children(&mut sc)
            .find(|c| GoKind::of(*c) == GoKind::FieldDeclarationList)
        else {
            continue;
        };
        let mut fc = field_list.walk();
        for decl in field_list
            .children(&mut fc)
            .filter(|c| GoKind::of(*c) == GoKind::FieldDeclaration)
        {
            let mut nc = decl.walk();
            for field_name_node in decl.children_by_field_name("name", &mut nc) {
                out.push((
                    type_name.clone(),
                    node_text(field_name_node, source).to_string(),
                ));
            }
        }
    }
}

fn walk_struct_fields(
    node: Node,
    language: Language,
    source: &str,
    out: &mut Vec<(String, String)>,
) {
    if language == Language::Go && GoKind::of(node) == GoKind::TypeDeclaration {
        go_struct_fields(node, source, out);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_struct_fields(child, language, source, out);
    }
}

/// Extracts every `(type_name, field_name)` pair declared on a struct in `source` —
/// Go-only for v1 (see [`field_access_supports`]); other languages return an empty list.
/// Exists solely to give [`extract_field_access_sites_for_file`] a field-name set to match
/// a selector's accessed name against.
pub fn extract_struct_fields_for_file(
    language: Language,
    source: &str,
    tree: &Tree,
) -> Vec<(String, String)> {
    if language != Language::Go {
        return Vec::new();
    }
    let mut out = Vec::new();
    walk_struct_fields(tree.root_node(), language, source, &mut out);
    out
}

/// One field access site found inside a method body: the enclosing method's owner-
/// qualified id, the raw receiver-type/field-name text (resolved into a
/// `FieldAccessEdge`'s synthetic field id by `arch_model::build_model`, which alone has
/// the whole-package struct-field index needed to confirm `field_name` is really a
/// declared field, not e.g. a value read off some other selector), and the syntactic
/// access kind. `file` is left as `PathBuf::new()` here, same convention as
/// `RawCallSite::file` — the caller fills it in.
#[derive(Debug, Clone, PartialEq)]
pub struct RawFieldAccessSite {
    pub package_path: String,
    pub caller_id: String,
    pub receiver_type: String,
    pub field_name: String,
    pub access: AccessKind,
    pub file: PathBuf,
    pub line: usize,
}

/// Field-access extraction is Go-only for v1 — Go's fields are always accessed through an
/// explicit `recv.Field` selector, which this extraction matches by identifier; other
/// languages (attribute access in Python, class-field access in TS/JS/Java/Kotlin, `self.`
/// in Rust) would each need their own selector-shape handling, deferred rather than
/// guessed at.
fn field_access_supports(language: Language) -> bool {
    matches!(language, Language::Go)
}

/// The enclosing method context `walk_field_accesses` threads through the walk: `None`
/// outside any method body, or inside one whose receiver has no bound name/type to match
/// a selector's operand against (including a method whose own parameter list already
/// shadows the receiver name for the entire body — see `method_field_access_ctx`).
///
/// `shadowed` is interior-mutable rather than threaded as a fresh value per recursive
/// call: once a shadowing declaration is seen anywhere in the method body (see
/// `shadows_receiver`), every node visited afterward in the same pre-order walk — later
/// siblings included — shares this same `FieldAccessCtx` and must see the update. A `Cell`
/// gives that without restructuring the walk to thread `&mut` state through every
/// recursive call.
struct FieldAccessCtx {
    caller_id: String,
    receiver_var: String,
    receiver_type: String,
    shadowed: Cell<bool>,
}

/// True when `selector` is itself the `function` field of its parent `call_expression` —
/// i.e. `recv.Method()`, a method call already captured by `RawCallSite` extraction, never
/// also counted as a field access. Compared by byte range rather than `Node` identity
/// equality, which this tree-sitter version's `Node` doesn't implement.
fn selector_is_call_target(selector: Node) -> bool {
    selector.parent().is_some_and(|p| {
        GoKind::of(p) == GoKind::CallExpression
            && p.child_by_field_name("function").is_some_and(|f| {
                f.start_byte() == selector.start_byte() && f.end_byte() == selector.end_byte()
            })
    })
}

/// Syntactic write-detection for a selector already confirmed to be a field access: the
/// left side of an `=`/`:=`-style assignment, or the operand of a bare `++`/`--`. Anything
/// else — including `&recv.Field` handed to something that mutates it indirectly, or a
/// selector passed as a call argument — reads as `Read`. Documented v1 ceiling, see
/// `arch_model::AccessKind`'s doc comment.
fn selector_access_kind(selector: Node) -> AccessKind {
    let Some(list) = selector.parent() else {
        return AccessKind::Read;
    };
    match GoKind::of(list) {
        GoKind::IncStatement | GoKind::DecStatement => AccessKind::Write,
        GoKind::ExpressionList => {
            let Some(stmt) = list.parent() else {
                return AccessKind::Read;
            };
            let is_left = stmt.child_by_field_name("left").is_some_and(|left| {
                left.start_byte() == list.start_byte() && left.end_byte() == list.end_byte()
            });
            if matches!(
                GoKind::of(stmt),
                GoKind::AssignmentStatement | GoKind::ShortVarDeclaration
            ) && is_left
            {
                AccessKind::Write
            } else {
                AccessKind::Read
            }
        }
        _ => AccessKind::Read,
    }
}

/// Builds the receiver context for a `method_declaration` node — `None` for an unnamed or
/// untyped receiver (no bound identifier a selector's operand could match), or when the
/// method's own `parameters` list already declares a parameter with the same name as the
/// receiver (`func (t T) M(t int) { ... }`, from `walk_field_accesses`'s doc comment): the
/// receiver name is shadowed for the entire body in that case, so there is no method-wide
/// window where attributing `t.Field` to the receiver would ever be correct.
fn method_field_access_ctx(node: Node, source: &str, package_path: &str) -> Option<FieldAccessCtx> {
    let receiver_var = go_receiver_var_name(node, source)?;
    let receiver_type = go_receiver_type_name(node, source)?;
    if let Some(params) = node.child_by_field_name("parameters")
        && parameter_list_names(params, source).contains(&receiver_var)
    {
        return None;
    }
    let name = node
        .child_by_field_name("name")
        .map(|n| strip_generic_params(node_text(n, source)))
        .unwrap_or_default();
    Some(FieldAccessCtx {
        caller_id: build_id(package_path, Some(&receiver_type), &name),
        receiver_var,
        receiver_type,
        shadowed: Cell::new(false),
    })
}

/// Every parameter name declared directly on a `parameter_list` node (a Go
/// `parameter_declaration` may carry more than one `name` child for a shared-type group,
/// `func(a, b int)` — same `commaSep1(field('name', ...))` shape as `go_struct_fields`).
fn parameter_list_names(list: Node, source: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut cursor = list.walk();
    for decl in list
        .children(&mut cursor)
        .filter(|c| GoKind::of(*c) == GoKind::ParameterDeclaration)
    {
        let mut nc = decl.walk();
        for name_node in decl.children_by_field_name("name", &mut nc) {
            names.push(node_text(name_node, source).to_string());
        }
    }
    names
}

/// Every identifier name declared as a new binding by `node`, if `node` is one of the
/// Go binding-introducing statement shapes `walk_field_accesses` checks for shadowing:
/// `short_var_declaration` (`x := ...`) and `var_spec` (`var x ...`, inside a
/// `var_declaration`) both bind via an `expression_list`/multi-`name`-field shape;
/// `func_literal` binds its own parameters, checked via `parameter_list_names`. A plain
/// `range_clause` using `=` instead of `:=` (`for t = range xs`) reassigns an existing
/// binding rather than introducing one and is deliberately not treated as a new shadow
/// here — see this function's caller, `shadows_receiver`, for why `:=` ranges are still
/// covered. Reassignment of the receiver identifier itself via a plain `=` (not a `:=` or
/// `var`) is a further, smaller residual ceiling this still doesn't catch: rebinding `t`
/// to a different value of the same type without redeclaring it slips through, same
/// "unusual enough in idiomatic Go to leave undetected" tradeoff as the shadowing case
/// this function exists to catch.
fn shadow_candidate_names(node: Node, source: &str) -> Vec<String> {
    match GoKind::of(node) {
        GoKind::ShortVarDeclaration | GoKind::VarSpec => {
            // short_var_declaration: `left` is an expression_list of identifiers.
            let mut names = node
                .child_by_field_name("left")
                .map(|list| direct_identifier_names(list, source))
                .unwrap_or_default();
            // var_spec: one or more direct `name` fields, no wrapping expression_list.
            let mut nc = node.walk();
            for name_node in node.children_by_field_name("name", &mut nc) {
                names.push(node_text(name_node, source).to_string());
            }
            names
        }
        GoKind::FuncLiteral => node
            .child_by_field_name("parameters")
            .map(|p| parameter_list_names(p, source))
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// Every direct `identifier` child of `node` (e.g. an `expression_list`'s bare-name
/// elements) — shared by `shadow_candidate_names` and `shadows_receiver`'s `range_clause`
/// handling.
fn direct_identifier_names(node: Node, source: &str) -> Vec<String> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .filter(|c| GoKind::of(*c) == GoKind::Identifier)
        .map(|id| node_text(id, source).to_string())
        .collect()
}

/// Whether `node` introduces a new binding for `receiver_var`, shadowing it from this
/// point forward in the pre-order walk. `range_clause` is handled separately from
/// `shadow_candidate_names` (rather than folded into it) because distinguishing its `:=`
/// form (introduces new bindings, e.g. `for t, v := range xs`) from its `=` form
/// (reassigns existing ones, e.g. `for t = range xs`) needs a raw-text check for `:=`
/// between `left` and the `range` keyword — `to_sexp()` doesn't surface that anonymous
/// token as a distinct field the way it does for `short_var_declaration`.
fn shadows_receiver(node: Node, source: &str, receiver_var: &str) -> bool {
    if shadow_candidate_names(node, source)
        .iter()
        .any(|n| n == receiver_var)
    {
        return true;
    }
    range_clause_declared_names(node, source)
        .iter()
        .any(|n| n == receiver_var)
}

/// `range_clause`'s `left` identifiers, but only when it's the `:=` (declaring) form —
/// `for t = range xs` reassigns, `for t := range xs` declares. Empty for every other node
/// kind, and for a `=`-form range_clause.
fn range_clause_declared_names(node: Node, source: &str) -> Vec<String> {
    if GoKind::of(node) != GoKind::RangeClause {
        return Vec::new();
    }
    let Some(list) = node.child_by_field_name("left") else {
        return Vec::new();
    };
    let between = &source[list.end_byte()..node.end_byte()];
    if !between.trim_start().starts_with(":=") {
        return Vec::new();
    }
    direct_identifier_names(list, source)
}

/// Recursive walk pairing each on-receiver `selector_expression` with the nearest
/// enclosing `method_declaration`'s receiver context. Mirrors `walk_calls`'s
/// caller-context-threading shape, but Go-only (`method_declaration` is the only node kind
/// checked, rather than every `function_kinds` entry) since field access is Go-only (see
/// `field_access_supports`).
///
/// Guards against misattributing a shadowed receiver name to the receiver (see
/// `shadows_receiver`): a local variable, parameter, or closure parameter that rebinds
/// the receiver identifier inside the method body (`func (t T) M() { t := t.Clone();
/// t.OtherField() }`) makes every `receiver_var.Field` selector from that point on refer
/// to the shadow, not the receiver. `ctx.shadowed` is a coarse, method-wide "once
/// shadowed, always shadowed for the rest of this method" flag rather than exact
/// block-scope-exit tracking (a shadow in an `if` branch stays flagged after the branch
/// closes) — a deliberate simplification in the direction of fewer false positives
/// (missing a few legitimate post-shadow accesses) rather than more (misattributing a
/// shadow's accesses to the receiver), matching this repo's existing bias for LCOM/DIP's
/// documented ceilings. A method whose own receiver name is shadowed by its own parameter
/// list (`func (t T) M(t int)`) never gets a `ctx` at all — see `method_field_access_ctx`.
fn walk_field_accesses(
    node: Node,
    source: &str,
    package_path: &str,
    ctx: Option<&FieldAccessCtx>,
    out: &mut Vec<RawFieldAccessSite>,
) {
    let is_method = GoKind::of(node) == GoKind::MethodDeclaration;
    let owned_ctx = is_method
        .then(|| method_field_access_ctx(node, source, package_path))
        .flatten();
    let ctx = if is_method { owned_ctx.as_ref() } else { ctx };

    if let Some(c) = ctx
        && !c.shadowed.get()
        && shadows_receiver(node, source, &c.receiver_var)
    {
        c.shadowed.set(true);
    }

    if GoKind::of(node) == GoKind::SelectorExpression
        && let Some(c) = ctx
        && !c.shadowed.get()
        && !selector_is_call_target(node)
        && let Some(operand) = node.child_by_field_name("operand")
        && GoKind::of(operand) == GoKind::Identifier
        && node_text(operand, source) == c.receiver_var
        && let Some(field) = node.child_by_field_name("field")
    {
        out.push(RawFieldAccessSite {
            package_path: package_path.to_string(),
            caller_id: c.caller_id.clone(),
            receiver_type: c.receiver_type.clone(),
            field_name: node_text(field, source).to_string(),
            access: selector_access_kind(node),
            file: PathBuf::new(),
            line: node.start_position().row + 1,
        });
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_field_accesses(child, source, package_path, ctx, out);
    }
}

/// Walks `tree.root_node()` and returns every field access found inside a Go method body,
/// paired with the enclosing method's owner-qualified id. Empty for a language
/// [`field_access_supports`] doesn't cover. No file I/O — `RawFieldAccessSite::file` is
/// left empty for the caller to fill in, same convention as `extract_call_sites_for_file`.
pub fn extract_field_access_sites_for_file(
    language: Language,
    source: &str,
    tree: &Tree,
    package_path: &str,
) -> Vec<RawFieldAccessSite> {
    if !field_access_supports(language) {
        return Vec::new();
    }
    let mut sites = Vec::new();
    walk_field_accesses(tree.root_node(), source, package_path, None, &mut sites);
    sites
}

/// One raw `extends`/`implements` site found in source, before resolution against the
/// rest of the model (mirrors `RawCallSite`/`RawFieldAccessSite`'s "raw" naming — a later
/// pass turns these into `TypeRelationEdge`s). `kind_hint` is `None` only for source
/// syntax that can't disambiguate `Extends`/`Implements` on its own (e.g. Go embedded
/// fields, Kotlin bare-`type` supertype-list entries) — Java always supplies a hint since
/// its grammar distinguishes `extends`/`implements` syntactically.
#[derive(Debug, Clone, PartialEq)]
pub struct RawTypeRelationSite {
    pub type_id: String,
    pub package_path: String,
    pub target_text: String,
    pub kind_hint: Option<TypeRelationKind>,
    pub file: PathBuf,
    pub line: usize,
    /// `true` only for a Go embedded field whose `target_text` is package-qualified
    /// (`pkg.Type`) — the *only* shape where a `.` in `target_text` is a real
    /// package-qualifier meant to be routed through `file_import_aliases`. TS/JS's
    /// `React.Component`-style member-expression supertypes, Java's `pkg.Base`/
    /// `scoped_type_identifier` targets, and any other language's dotted text are real
    /// possibilities too, but resolving *those* by stripping the qualifier and doing a
    /// same-name lookup elsewhere in the model would be a guess, not a resolution — see
    /// `resolve_type_edge_target`'s doc comment in `arch_model.rs`. Defaults to `false`;
    /// only `go_embedded_type_relations`/`go_embedded_interface_relations` set it `true`.
    pub dotted_text_is_go_package_qualifier: bool,
}

/// Languages `extract_type_relation_sites_for_file` walks. Mirrors `call_graph_supports`/
/// `field_access_supports`'s per-feature capability-list convention.
pub fn type_relation_supports(language: Language) -> bool {
    matches!(
        language,
        Language::Go
            | Language::TypeScript
            | Language::Tsx
            | Language::JavaScript
            | Language::Java
            | Language::Kotlin
    )
}

fn walk_type_relations(
    node: Node,
    language: Language,
    source: &str,
    package_path: &str,
    out: &mut Vec<RawTypeRelationSite>,
) {
    match language {
        Language::Go => {
            if GoKind::of(node) == GoKind::TypeDeclaration {
                go_embedded_type_relations(node, source, package_path, out);
                go_embedded_interface_relations(node, source, package_path, out);
            }
        }
        Language::Java => match JavaKind::of(node) {
            JavaKind::ClassDeclaration => {
                java_class_type_relations(node, source, package_path, out)
            }
            JavaKind::InterfaceDeclaration => {
                java_interface_type_relations(node, source, package_path, out)
            }
            _ => {}
        },
        Language::TypeScript | Language::Tsx => {
            if TypeScriptKind::of(node) == TypeScriptKind::ClassDeclaration {
                ts_class_heritage_relations(node, source, package_path, out);
            }
        }
        Language::JavaScript => {
            if JavaScriptKind::of(node) == JavaScriptKind::ClassDeclaration {
                js_class_heritage_relation(node, source, package_path, out);
            }
        }
        Language::Kotlin if KotlinKind::of(node) == KotlinKind::ClassDeclaration => {
            kotlin_delegation_type_relations(node, source, package_path, out);
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_type_relations(child, language, source, package_path, out);
    }
}

/// Walks `tree.root_node()` and returns every `extends`/`implements` site found, per
/// [`type_relation_supports`]. No file I/O — `RawTypeRelationSite::file` is left empty for
/// the caller to fill in, same convention as `extract_call_sites_for_file`.
pub fn extract_type_relation_sites_for_file(
    language: Language,
    source: &str,
    tree: &Tree,
    package_path: &str,
) -> Vec<RawTypeRelationSite> {
    let mut out = Vec::new();
    if !type_relation_supports(language) {
        return out;
    }
    walk_type_relations(tree.root_node(), language, source, package_path, &mut out);
    out
}

/// Java `class_declaration` → `extends`/`implements` sites. `superclass` is a NAMED field
/// (`class_declaration`'s `superclass` field wraps the single supertype reference
/// directly); `interfaces` is also a named field, but wraps a `super_interfaces` node whose
/// own single `type_list` child holds one entry per implemented interface — verified
/// against the grammar's field-based/positional split the epic's task description calls
/// out (contrast with `java_interface_type_relations`'s positional `extends_interfaces`).
fn java_class_type_relations(
    class_decl: Node,
    source: &str,
    package_path: &str,
    out: &mut Vec<RawTypeRelationSite>,
) {
    let Some(name_node) = class_decl.child_by_field_name("name") else {
        return;
    };
    let class_name = node_text(name_node, source);
    let type_id = build_id(package_path, None, &strip_generic_params(class_name));
    let line = class_decl.start_position().row + 1;

    if let Some(superclass) = class_decl.child_by_field_name("superclass")
        && let Some(target) = superclass.named_child(0)
    {
        out.push(RawTypeRelationSite {
            type_id: type_id.clone(),
            package_path: package_path.to_string(),
            target_text: strip_generic_params(node_text(target, source)),
            kind_hint: Some(TypeRelationKind::Extends),
            file: PathBuf::new(),
            line,
            dotted_text_is_go_package_qualifier: false,
        });
    }

    if let Some(interfaces) = class_decl.child_by_field_name("interfaces")
        && let Some(type_list) = find_child_by_kind(interfaces, "type_list")
    {
        let mut cursor = type_list.walk();
        for target in type_list.named_children(&mut cursor) {
            out.push(RawTypeRelationSite {
                type_id: type_id.clone(),
                package_path: package_path.to_string(),
                target_text: strip_generic_params(node_text(target, source)),
                kind_hint: Some(TypeRelationKind::Implements),
                file: PathBuf::new(),
                line,
                dotted_text_is_go_package_qualifier: false,
            });
        }
    }
}

/// Java `interface_declaration` → `extends` sites. `extends_interfaces` is a POSITIONAL
/// child here (no field name) — unlike `class_declaration`'s named `interfaces` field —
/// and interface-to-interface inheritance uses the `extends` keyword, so every site here
/// is tagged `Extends`, never `Implements`, even though (like `interfaces` above) it wraps
/// a `type_list` of possibly multiple targets.
fn java_interface_type_relations(
    interface_decl: Node,
    source: &str,
    package_path: &str,
    out: &mut Vec<RawTypeRelationSite>,
) {
    let Some(name_node) = interface_decl.child_by_field_name("name") else {
        return;
    };
    let interface_name = node_text(name_node, source);
    let type_id = build_id(package_path, None, &strip_generic_params(interface_name));
    let line = interface_decl.start_position().row + 1;

    let Some(extends_interfaces) = find_child_by_kind(interface_decl, "extends_interfaces") else {
        return;
    };
    let Some(type_list) = find_child_by_kind(extends_interfaces, "type_list") else {
        return;
    };
    let mut cursor = type_list.walk();
    for target in type_list.named_children(&mut cursor) {
        out.push(RawTypeRelationSite {
            type_id: type_id.clone(),
            package_path: package_path.to_string(),
            target_text: strip_generic_params(node_text(target, source)),
            kind_hint: Some(TypeRelationKind::Extends),
            file: PathBuf::new(),
            line,
            dotted_text_is_go_package_qualifier: false,
        });
    }
}

/// TS/Tsx `class_declaration` → `extends`/`implements` sites. Both clauses live inside a
/// `class_heritage` node that is itself a positional (not field-based) child of
/// `class_declaration` — verified via `to_sexp()`: `(class_declaration name: ... (class_heritage
/// (extends_clause value: ...) (implements_clause (type_identifier) ...)) body: ...)`.
/// `extends_clause`'s superclass is its `value` field; `implements_clause` has no
/// analogous field — each implemented type is a bare positional (named) child, since TS
/// allows more than one. A generic superclass's type arguments (`Base<T>`) surface as a
/// sibling `type_arguments` field on `extends_clause`, not inside `value`'s own text, so
/// `strip_generic_params` here is defensive rather than load-bearing (same caveat as
/// `strip_generic_params`'s own doc comment) — kept for parity with every other call site
/// per the Pattern Decisions table.
fn ts_class_heritage_relations(
    class_decl: Node,
    source: &str,
    package_path: &str,
    out: &mut Vec<RawTypeRelationSite>,
) {
    let Some(name_node) = class_decl.child_by_field_name("name") else {
        return;
    };
    let type_id = build_id(
        package_path,
        None,
        &strip_generic_params(node_text(name_node, source)),
    );
    let line = class_decl.start_position().row + 1;
    let push = |target: Node, kind, out: &mut Vec<RawTypeRelationSite>| {
        out.push(RawTypeRelationSite {
            type_id: type_id.clone(),
            package_path: package_path.to_string(),
            target_text: strip_generic_params(node_text(target, source)),
            kind_hint: Some(kind),
            file: PathBuf::new(),
            line,
            dotted_text_is_go_package_qualifier: false,
        });
    };

    let Some(heritage) = find_child_by_kind(class_decl, "class_heritage") else {
        return;
    };

    if let Some(extends) = find_child_by_kind(heritage, "extends_clause")
        && let Some(value) = extends.child_by_field_name("value")
    {
        push(value, TypeRelationKind::Extends, out);
    }

    if let Some(implements) = find_child_by_kind(heritage, "implements_clause") {
        let mut cursor = implements.walk();
        for ty in implements.named_children(&mut cursor) {
            push(ty, TypeRelationKind::Implements, out);
        }
    }
}

/// Plain JS `class_declaration` → its (at most one) `extends` site. Deliberately separate
/// from [`ts_class_heritage_relations`], not a shared helper: plain JS's `class_heritage`
/// has none of TS's `extends_clause`/`implements_clause` substructure — verified via
/// `to_sexp()`, `class Dog extends Animal {}` produces `(class_heritage (identifier))`, a
/// single bare positional child that *is* the superclass expression. A class with no
/// `class_heritage` child at all (`class Dog {}`) emits nothing. JS has no `implements`
/// keyword, so this never emits a `TypeRelationKind::Implements` site.
fn js_class_heritage_relation(
    class_decl: Node,
    source: &str,
    package_path: &str,
    out: &mut Vec<RawTypeRelationSite>,
) {
    let Some(name_node) = class_decl.child_by_field_name("name") else {
        return;
    };
    let Some(heritage) = find_child_by_kind(class_decl, "class_heritage") else {
        return;
    };
    let Some(superclass) = heritage.named_child(0) else {
        return;
    };
    out.push(RawTypeRelationSite {
        type_id: build_id(
            package_path,
            None,
            &strip_generic_params(node_text(name_node, source)),
        ),
        package_path: package_path.to_string(),
        target_text: strip_generic_params(node_text(superclass, source)),
        kind_hint: Some(TypeRelationKind::Extends),
        file: PathBuf::new(),
        line: class_decl.start_position().row + 1,
        dotted_text_is_go_package_qualifier: false,
    });
}

/// Kotlin `class_declaration` (covers both plain classes and interfaces) →
/// `delegation_specifiers`, split three ways by shape (verified against
/// `tree-sitter-kotlin-ng`'s `grammar.js`; all children read positionally, no named
/// fields): `constructor_invocation` (`Base()`) → unambiguously a class → `Extends`;
/// `explicit_delegation` (`Iface by impl`) → unambiguously interface delegation →
/// `Implements`; a bare `type` (`user_type` etc.) → genuinely ambiguous → `kind_hint: None`
/// — NOT always an interface (Kotlin's "no primary constructor" idiom writes its
/// superclass bare too, see the `..._no_primary_constructor_superclass_...` test below),
/// so it's deliberately left unresolved rather than guessed.
fn kotlin_delegation_type_relations(
    class_decl: Node,
    source: &str,
    package_path: &str,
    out: &mut Vec<RawTypeRelationSite>,
) {
    let Some(name_node) = class_decl.child_by_field_name("name") else {
        return;
    };
    let type_id = build_id(
        package_path,
        None,
        &strip_generic_params(node_text(name_node, source)),
    );
    let line = class_decl.start_position().row + 1;

    let Some(specifiers) = find_child_by_kind(class_decl, "delegation_specifiers") else {
        return;
    };

    let mut cursor = specifiers.walk();
    for specifier in specifiers
        .children(&mut cursor)
        .filter(|c| KotlinKind::of(*c) == KotlinKind::DelegationSpecifier)
    {
        let Some((target, kind_hint)) = kotlin_delegation_specifier_target(specifier) else {
            continue;
        };
        out.push(RawTypeRelationSite {
            type_id: type_id.clone(),
            package_path: package_path.to_string(),
            target_text: strip_generic_params(node_text(target, source)),
            kind_hint,
            file: PathBuf::new(),
            line,
            dotted_text_is_go_package_qualifier: false,
        });
    }
}

/// Classifies one `delegation_specifier` child's shape and returns the type node to read the
/// target name from, paired with the `kind_hint` that shape implies. `None` only when the
/// specifier is malformed (an `annotation`-only child list, or a shape node missing its
/// expected first named child) — never used to mean "ambiguous", which is
/// `Some((_, None))`.
fn kotlin_delegation_specifier_target(specifier: Node) -> Option<(Node, Option<TypeRelationKind>)> {
    let mut cursor = specifier.walk();
    let shape = specifier
        .children(&mut cursor)
        .find(|c| KotlinKind::of(*c) != KotlinKind::Annotation)?;

    match KotlinKind::of(shape) {
        KotlinKind::ConstructorInvocation => {
            Some((shape.named_child(0)?, Some(TypeRelationKind::Extends)))
        }
        KotlinKind::ExplicitDelegation => {
            Some((shape.named_child(0)?, Some(TypeRelationKind::Implements)))
        }
        _ => Some((shape, None)),
    }
}

/// Resolves a Go embedded field's `type`-field node to its raw supertype name text,
/// unwrapping `pointer_type` (one level, same pattern as `go_receiver_type_name`) and
/// `generic_type` (drops the `type_arguments` child entirely, equivalent to
/// `strip_generic_params`) before matching the base shape: a bare `type_identifier`
/// name, or a `qualified_type`'s `package.Name` join. Any other shape (e.g. an
/// anonymous inline `struct_type`) has no name to report and yields `None`.
fn go_embedded_type_name(ty: Node, source: &str) -> Option<String> {
    match GoKind::of(ty) {
        GoKind::TypeIdentifier => Some(node_text(ty, source).to_string()),
        GoKind::QualifiedType => {
            let package = ty.child_by_field_name("package")?;
            let name = ty.child_by_field_name("name")?;
            Some(format!(
                "{}.{}",
                node_text(package, source),
                node_text(name, source)
            ))
        }
        GoKind::PointerType => go_embedded_type_name(ty.named_child(0)?, source),
        GoKind::GenericType => go_embedded_type_name(ty.child_by_field_name("type")?, source),
        _ => None,
    }
}

/// Appends one `RawTypeRelationSite` per embedded (anonymous) field declared directly on
/// a Go struct type under `type_decl` (a `type_declaration` node, possibly a grouped
/// `type (...)` block — each `type_spec` child is handled independently, matching
/// `go_struct_fields`'s precedent). Opposite branch from `go_struct_fields`: a
/// `field_declaration` with no `name` field-child is the embed this extracts (skipped
/// there); one with a `name` is an ordinary field and is skipped here. `kind_hint` is
/// always `None` — Go embeds are structurally ambiguous between `extends`/`implements`,
/// resolved later by symbol-kind lookup, not here.
fn go_embedded_type_relations(
    type_decl: Node,
    source: &str,
    package_path: &str,
    out: &mut Vec<RawTypeRelationSite>,
) {
    let mut cursor = type_decl.walk();
    for spec in type_decl
        .children(&mut cursor)
        .filter(|c| GoKind::of(*c) == GoKind::TypeSpec)
    {
        let Some(name_node) = spec.child_by_field_name("name") else {
            continue;
        };
        let Some(struct_ty) = spec.child_by_field_name("type") else {
            continue;
        };
        if GoKind::of(struct_ty) != GoKind::StructType {
            continue;
        }
        let type_name = strip_generic_params(node_text(name_node, source));
        let type_id = build_id(package_path, None, &type_name);
        let mut sc = struct_ty.walk();
        let Some(field_list) = struct_ty
            .children(&mut sc)
            .find(|c| GoKind::of(*c) == GoKind::FieldDeclarationList)
        else {
            continue;
        };
        let mut fc = field_list.walk();
        for decl in field_list
            .children(&mut fc)
            .filter(|c| GoKind::of(*c) == GoKind::FieldDeclaration)
        {
            if decl.child_by_field_name("name").is_some() {
                continue;
            }
            let Some(ty) = decl.child_by_field_name("type") else {
                continue;
            };
            let Some(target_text) = go_embedded_type_name(ty, source) else {
                continue;
            };
            out.push(RawTypeRelationSite {
                type_id: type_id.clone(),
                package_path: package_path.to_string(),
                target_text,
                kind_hint: None,
                file: PathBuf::new(),
                line: type_decl.start_position().row + 1,
                dotted_text_is_go_package_qualifier: true,
            });
        }
    }
}

/// Appends one `RawTypeRelationSite` per embedded interface declared directly on a Go
/// `interface_type` under `type_decl` (same grouped-`type (...)` handling as
/// `go_embedded_type_relations`). Opposite branch from `interface_declared_methods`
/// (`isp_fat_interface.rs`): a `type_elem` child is the embedded interface this extracts
/// (skipped there as "real cross-type/cross-package work, deferred"); a `method_elem`
/// child is an ordinary method signature and is skipped here. Unlike struct embedding
/// (ambiguous between `Extends`/`Implements`, resolved later by symbol-kind lookup), Go's
/// grammar only allows embedding an interface inside another interface — there's no
/// "embed a struct inside an interface" shape — so `kind_hint` is always
/// `Some(Implements)` here, no ambiguity to defer.
fn go_embedded_interface_relations(
    type_decl: Node,
    source: &str,
    package_path: &str,
    out: &mut Vec<RawTypeRelationSite>,
) {
    let mut cursor = type_decl.walk();
    for spec in type_decl
        .children(&mut cursor)
        .filter(|c| GoKind::of(*c) == GoKind::TypeSpec)
    {
        let Some(name_node) = spec.child_by_field_name("name") else {
            continue;
        };
        let Some(interface_ty) = spec.child_by_field_name("type") else {
            continue;
        };
        if GoKind::of(interface_ty) != GoKind::InterfaceType {
            continue;
        }
        let type_name = strip_generic_params(node_text(name_node, source));
        let type_id = build_id(package_path, None, &type_name);
        let mut ic = interface_ty.walk();
        for elem in interface_ty
            .children(&mut ic)
            .filter(|c| GoKind::of(*c) == GoKind::TypeElem)
        {
            let Some(ty) = elem.named_child(0) else {
                continue;
            };
            let Some(target_text) = go_embedded_type_name(ty, source) else {
                continue;
            };
            out.push(RawTypeRelationSite {
                type_id: type_id.clone(),
                package_path: package_path.to_string(),
                target_text,
                kind_hint: Some(TypeRelationKind::Implements),
                file: PathBuf::new(),
                line: type_decl.start_position().row + 1,
                dotted_text_is_go_package_qualifier: true,
            });
        }
    }
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

    fn type_relation_sites(
        language: Language,
        source: &str,
        package_path: &str,
    ) -> Vec<RawTypeRelationSite> {
        let cache = GrammarCache::new();
        let tree = cache.parse(language, source).expect("parses");
        extract_type_relation_sites_for_file(language, source, &tree, package_path)
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

    // --- extract_call_sites_for_file ---

    fn call_sites(language: Language, source: &str, package_path: &str) -> Vec<RawCallSite> {
        let cache = GrammarCache::new();
        let tree = cache.parse(language, source).expect("parses");
        extract_call_sites_for_file(language, source, &tree, package_path)
    }

    #[test]
    fn go_bare_call_resolves_caller_and_callee_text() {
        let sites = call_sites(
            Language::Go,
            "package pkg\n\nfunc Bar() {}\n\nfunc Foo() {\n\tBar()\n}\n",
            "pkg",
        );
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].caller_id, "pkg::Foo");
        assert_eq!(sites[0].callee_text, "Bar");
    }

    #[test]
    fn go_selector_call_keeps_the_full_qualified_text() {
        let sites = call_sites(
            Language::Go,
            "package pkg\n\nfunc Foo() {\n\tpkg.Qux()\n\tt.Method()\n}\n",
            "pkg",
        );
        assert_eq!(sites.len(), 2);
        assert!(sites.iter().all(|s| s.caller_id == "pkg::Foo"));
        assert_eq!(sites[0].callee_text, "pkg.Qux");
        assert_eq!(sites[1].callee_text, "t.Method");
    }

    #[test]
    fn go_method_call_attributes_to_the_owner_qualified_caller_id() {
        let sites = call_sites(
            Language::Go,
            "package pkg\n\ntype T struct{}\n\nfunc (t T) M() {\n\tBar()\n}\n",
            "pkg",
        );
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].caller_id, "pkg::T.M");
    }

    #[test]
    fn go_call_with_no_enclosing_function_is_dropped() {
        // A call outside any function/method body has no caller context to attribute to
        // (e.g. a package-level var initializer) — this crate doesn't model that as a
        // call edge at all rather than inventing a synthetic caller id.
        let sites = call_sites(Language::Go, "package pkg\n\nvar x = Bar()\n", "pkg");
        assert!(sites.is_empty(), "got: {sites:?}");
    }

    #[test]
    fn ts_bare_and_member_calls_both_attribute_to_enclosing_function() {
        let sites = call_sites(
            Language::TypeScript,
            "function bar() {}\n\nfunction foo() {\n  bar();\n  obj.method();\n}\n",
            "pkg",
        );
        assert_eq!(sites.len(), 2);
        assert!(sites.iter().all(|s| s.caller_id == "pkg::foo"));
        assert_eq!(sites[0].callee_text, "bar");
        assert_eq!(sites[1].callee_text, "obj.method");
    }

    #[test]
    fn ts_call_inside_anonymous_arrow_attributes_to_the_named_enclosing_function() {
        let sites = call_sites(
            Language::TypeScript,
            "function foo() {\n  const cb = () => { bar(); };\n}\n",
            "pkg",
        );
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].caller_id, "pkg::foo");
        assert_eq!(sites[0].callee_text, "bar");
    }

    #[test]
    fn ts_class_method_call_attributes_to_owner_qualified_caller_id() {
        let sites = call_sites(
            Language::TypeScript,
            "class T {\n  method() {\n    bar();\n  }\n}\n",
            "pkg",
        );
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].caller_id, "pkg::T.method");
    }

    #[test]
    fn python_call_sites_are_unsupported_in_v1() {
        // Call-graph extraction is scoped to Go/TS/JS for v1 (see the issue's scope
        // notes) — Python symbol extraction exists, but call sites are deliberately not
        // extracted for it yet.
        let sites = call_sites(
            Language::Python,
            "def bar():\n    pass\n\ndef foo():\n    bar()\n",
            "pkg",
        );
        assert!(sites.is_empty(), "got: {sites:?}");
    }

    // --- extract_struct_fields_for_file / extract_field_access_sites_for_file (#39) ---

    fn struct_fields(language: Language, source: &str) -> Vec<(String, String)> {
        let cache = GrammarCache::new();
        let tree = cache.parse(language, source).expect("parses");
        extract_struct_fields_for_file(language, source, &tree)
    }

    fn field_access_sites(
        language: Language,
        source: &str,
        package_path: &str,
    ) -> Vec<RawFieldAccessSite> {
        let cache = GrammarCache::new();
        let tree = cache.parse(language, source).expect("parses");
        extract_field_access_sites_for_file(language, source, &tree, package_path)
    }

    #[test]
    fn go_struct_fields_collects_every_declared_field() {
        let fields = struct_fields(
            Language::Go,
            "package pkg\n\ntype T struct {\n\tX, Y int\n\tName string\n}\n",
        );
        assert_eq!(
            fields,
            vec![
                ("T".to_string(), "X".to_string()),
                ("T".to_string(), "Y".to_string()),
                ("T".to_string(), "Name".to_string()),
            ]
        );
    }

    #[test]
    fn go_struct_fields_skips_embedded_anonymous_fields() {
        let fields = struct_fields(
            Language::Go,
            "package pkg\n\ntype T struct {\n\tOther\n\tName string\n}\n",
        );
        assert_eq!(fields, vec![("T".to_string(), "Name".to_string())]);
    }

    #[test]
    fn go_field_read_in_method_body_resolves_to_owner_qualified_caller() {
        let sites = field_access_sites(
            Language::Go,
            "package pkg\n\ntype T struct {\n\tX int\n}\n\nfunc (t T) Get() int {\n\treturn t.X\n}\n",
            "pkg",
        );
        assert_eq!(sites.len(), 1, "got: {sites:?}");
        assert_eq!(sites[0].caller_id, "pkg::T.Get");
        assert_eq!(sites[0].receiver_type, "T");
        assert_eq!(sites[0].field_name, "X");
        assert_eq!(sites[0].access, AccessKind::Read);
    }

    #[test]
    fn go_field_assignment_in_method_body_is_a_write() {
        let sites = field_access_sites(
            Language::Go,
            "package pkg\n\ntype T struct {\n\tX int\n}\n\nfunc (t *T) Set(v int) {\n\tt.X = v\n}\n",
            "pkg",
        );
        assert_eq!(sites.len(), 1, "got: {sites:?}");
        assert_eq!(sites[0].access, AccessKind::Write);
    }

    #[test]
    fn go_field_inc_dec_is_a_write() {
        let sites = field_access_sites(
            Language::Go,
            "package pkg\n\ntype T struct {\n\tN int\n}\n\nfunc (t *T) Bump() {\n\tt.N++\n}\n",
            "pkg",
        );
        assert_eq!(sites.len(), 1, "got: {sites:?}");
        assert_eq!(sites[0].access, AccessKind::Write);
    }

    #[test]
    fn go_method_call_on_receiver_is_not_also_counted_as_a_field_access() {
        let sites = field_access_sites(
            Language::Go,
            "package pkg\n\ntype T struct {\n\tX int\n}\n\nfunc (t T) Helper() {}\n\nfunc (t T) Run() {\n\tt.Helper()\n}\n",
            "pkg",
        );
        assert!(
            sites.is_empty(),
            "a call through the receiver shouldn't also register as a field access: {sites:?}"
        );
    }

    #[test]
    fn go_field_access_on_a_different_local_variable_is_not_attributed_to_the_receiver() {
        let sites = field_access_sites(
            Language::Go,
            "package pkg\n\ntype T struct {\n\tX int\n}\n\nfunc (t T) Run() {\n\tother := T{}\n\tother.X = 1\n}\n",
            "pkg",
        );
        assert!(
            sites.is_empty(),
            "access through a non-receiver variable must not count: {sites:?}"
        );
    }

    #[test]
    fn go_field_read_on_the_right_hand_side_of_short_var_declaration_is_not_a_write() {
        let sites = field_access_sites(
            Language::Go,
            "package pkg\n\ntype T struct {\n\tX int\n}\n\nfunc (t T) Get() int {\n\tv := t.X\n\treturn v\n}\n",
            "pkg",
        );
        assert_eq!(sites[0].access, AccessKind::Read);
    }

    #[test]
    fn go_unnamed_receiver_yields_no_field_access_sites() {
        let sites = field_access_sites(
            Language::Go,
            "package pkg\n\ntype T struct {\n\tX int\n}\n\nfunc (T) Get() int {\n\treturn 0\n}\n",
            "pkg",
        );
        assert!(sites.is_empty(), "got: {sites:?}");
    }

    // --- Receiver-name shadowing (deferred known ceiling from #38's PR #84) ---

    #[test]
    fn go_receiver_reused_by_short_var_declaration_shadows_for_rest_of_method() {
        // Before the fix: `t := t.Clone()` re-binds `t`, and the naive name-only match
        // attributed the following `t.Y = 1` to the *receiver*'s `Y` field — a false
        // field-access edge that could wrongly cluster this method with others touching
        // the receiver's real `Y` field (e.g. inflating LCOM4's cohesion signal).
        let sites = field_access_sites(
            Language::Go,
            "package pkg\n\ntype T struct {\n\tX, Y int\n}\n\nfunc (t T) Clone() T { return t }\n\nfunc (t T) Run() {\n\tt := t.Clone()\n\tt.Y = 1\n}\n",
            "pkg",
        );
        assert!(
            sites.is_empty(),
            "every access after the `t := t.Clone()` shadow must be excluded: {sites:?}"
        );
    }

    #[test]
    fn go_receiver_access_before_a_later_shadow_is_still_attributed() {
        // The shadow only takes effect from its declaration onward — an access earlier in
        // the same method, before `t` is re-bound, is still a genuine receiver access.
        let sites = field_access_sites(
            Language::Go,
            "package pkg\n\ntype T struct {\n\tX int\n}\n\nfunc (t T) Clone() T { return t }\n\nfunc (t T) Run() {\n\t_ = t.X\n\tt := t.Clone()\n\t_ = t\n}\n",
            "pkg",
        );
        assert_eq!(sites.len(), 1, "got: {sites:?}");
    }

    #[test]
    fn go_receiver_shadowed_by_closure_parameter_is_excluded() {
        let sites = field_access_sites(
            Language::Go,
            "package pkg\n\ntype T struct {\n\tX int\n}\n\nfunc (t T) Run() {\n\tf := func(t int) { _ = t }\n\tf(1)\n}\n",
            "pkg",
        );
        assert!(sites.is_empty(), "got: {sites:?}");
    }

    #[test]
    fn go_receiver_shadowed_by_own_parameter_yields_no_field_access_sites() {
        // `func (t T) M(t int)`, the exact example from `walk_field_accesses`'s original
        // doc comment — the receiver name is unusable for the method's entire body.
        let sites = field_access_sites(
            Language::Go,
            "package pkg\n\ntype T struct {\n\tX int\n}\n\nfunc (t T) M(t int) {\n\t_ = t\n}\n",
            "pkg",
        );
        assert!(sites.is_empty(), "got: {sites:?}");
    }

    #[test]
    fn go_receiver_shadowed_by_var_declaration_is_excluded() {
        let sites = field_access_sites(
            Language::Go,
            "package pkg\n\ntype T struct {\n\tX int\n}\n\nfunc (t T) Run() {\n\tvar t int\n\t_ = t\n}\n",
            "pkg",
        );
        assert!(sites.is_empty(), "got: {sites:?}");
    }

    #[test]
    fn go_receiver_shadowed_by_declaring_range_variable_is_excluded() {
        let sites = field_access_sites(
            Language::Go,
            "package pkg\n\ntype T struct {\n\tX int\n}\n\nfunc (t T) Run() {\n\tfor t, v := range []int{1} {\n\t\t_ = t\n\t\t_ = v\n\t}\n}\n",
            "pkg",
        );
        assert!(sites.is_empty(), "got: {sites:?}");
    }

    #[test]
    fn go_range_clause_reassigning_receiver_with_plain_equals_is_not_treated_as_a_shadow() {
        // `=`, not `:=` — reassigns the existing `t`, doesn't declare a new one. This repo
        // deliberately leaves plain-`=` reassignment of the receiver itself undetected (see
        // `shadow_candidate_names`'s doc comment), so this documents the boundary rather
        // than asserting a specific (currently unenforced) outcome for it.
        let sites = field_access_sites(
            Language::Go,
            "package pkg\n\ntype T struct {\n\tX int\n}\n\nfunc (t T) Run() {\n\tvar t T\n\tfor t = range []T{} {\n\t\t_ = t.X\n\t}\n}\n",
            "pkg",
        );
        // `var t T` already shadows before the loop even starts, so this stays empty either
        // way — the point of this test is documentation, not a new assertion.
        assert!(sites.is_empty(), "got: {sites:?}");
    }

    // --- Epic 1.4: Java extends/implements extraction ---

    #[test]
    fn java_class_type_relations_captures_superclass_and_multiple_interfaces() {
        let sites = type_relation_sites(
            Language::Java,
            "class Dog extends Animal implements Runnable, Named {}\n",
            "pkg",
        );
        assert_eq!(sites.len(), 3, "got: {sites:?}");
        assert!(sites.iter().all(|s| s.type_id == "pkg::Dog"));

        let extends: Vec<_> = sites
            .iter()
            .filter(|s| s.kind_hint == Some(TypeRelationKind::Extends))
            .collect();
        assert_eq!(extends.len(), 1);
        assert_eq!(extends[0].target_text, "Animal");

        let implements: Vec<&str> = sites
            .iter()
            .filter(|s| s.kind_hint == Some(TypeRelationKind::Implements))
            .map(|s| s.target_text.as_str())
            .collect();
        assert_eq!(implements, vec!["Runnable", "Named"]);
    }

    #[test]
    fn java_class_type_relations_empty_for_plain_class() {
        let sites = type_relation_sites(Language::Java, "class Dog {}\n", "pkg");
        assert!(sites.is_empty(), "got: {sites:?}");
    }

    #[test]
    fn java_interface_type_relations_captures_multiple_extends_targets_as_extends_kind() {
        let sites =
            type_relation_sites(Language::Java, "interface Foo extends Bar, Baz {}\n", "pkg");
        assert_eq!(sites.len(), 2, "got: {sites:?}");
        assert!(sites.iter().all(|s| s.type_id == "pkg::Foo"));
        assert!(
            sites
                .iter()
                .all(|s| s.kind_hint == Some(TypeRelationKind::Extends))
        );

        let targets: Vec<&str> = sites.iter().map(|s| s.target_text.as_str()).collect();
        assert_eq!(targets, vec!["Bar", "Baz"]);
    }

    // --- Epic 1.3: TS/Tsx/JS class_heritage extraction ---

    #[test]
    fn ts_class_heritage_relations_captures_extends_and_multiple_implements() {
        let sites = type_relation_sites(
            Language::TypeScript,
            "class Dog extends Animal implements Runnable, Named {}\n",
            "pkg",
        );
        assert_eq!(sites.len(), 3, "got: {sites:?}");
        assert!(sites.iter().all(|s| s.type_id == "pkg::Dog"));

        let extends: Vec<_> = sites
            .iter()
            .filter(|s| s.kind_hint == Some(TypeRelationKind::Extends))
            .collect();
        assert_eq!(extends.len(), 1);
        assert_eq!(extends[0].target_text, "Animal");

        let implements: Vec<&str> = sites
            .iter()
            .filter(|s| s.kind_hint == Some(TypeRelationKind::Implements))
            .map(|s| s.target_text.as_str())
            .collect();
        assert_eq!(implements, vec!["Runnable", "Named"]);
    }

    #[test]
    fn ts_class_heritage_relations_strips_generic_args() {
        let sites = type_relation_sites(
            Language::TypeScript,
            "class Dog<T> extends Base<T> {}\n",
            "pkg",
        );
        assert_eq!(sites.len(), 1, "got: {sites:?}");
        assert_eq!(sites[0].target_text, "Base");
    }

    #[test]
    fn tsx_class_heritage_relations_matches_typescript() {
        let source = "class Dog extends Animal {}\n";
        let ts_sites = type_relation_sites(Language::TypeScript, source, "pkg");
        let tsx_sites = type_relation_sites(Language::Tsx, source, "pkg");

        assert_eq!(ts_sites.len(), 1);
        assert_eq!(tsx_sites.len(), 1);
        assert_eq!(tsx_sites[0].target_text, ts_sites[0].target_text);
        assert_eq!(tsx_sites[0].kind_hint, ts_sites[0].kind_hint);
        assert_eq!(tsx_sites[0].kind_hint, Some(TypeRelationKind::Extends));
    }

    #[test]
    fn js_class_heritage_relation_captures_extends() {
        let sites =
            type_relation_sites(Language::JavaScript, "class Dog extends Animal {}\n", "pkg");
        assert_eq!(sites.len(), 1, "got: {sites:?}");
        assert_eq!(sites[0].target_text, "Animal");
        assert_eq!(sites[0].kind_hint, Some(TypeRelationKind::Extends));
    }

    #[test]
    fn js_class_heritage_relation_empty_for_no_extends() {
        let sites = type_relation_sites(Language::JavaScript, "class Dog {}\n", "pkg");
        assert!(sites.is_empty(), "got: {sites:?}");
    }

    // --- go_embedded_type_relations (Epic 1.2) ---

    #[test]
    fn go_embedded_type_relations_captures_same_package_embed() {
        let sites = type_relation_sites(
            Language::Go,
            "package pkg\n\ntype Animal struct{}\n\ntype Dog struct {\n\tAnimal\n\tName string\n}\n",
            "pkg",
        );
        assert_eq!(sites.len(), 1, "got: {sites:?}");
        assert_eq!(sites[0].type_id, "pkg::Dog");
        assert_eq!(sites[0].target_text, "Animal");
        assert_eq!(sites[0].kind_hint, None);
    }

    #[test]
    fn go_embedded_type_relations_unwraps_pointer_embed() {
        let sites = type_relation_sites(
            Language::Go,
            "package pkg\n\ntype Dog struct {\n\t*Animal\n}\n",
            "pkg",
        );
        assert_eq!(sites.len(), 1, "got: {sites:?}");
        assert_eq!(sites[0].target_text, "Animal");
    }

    #[test]
    fn go_embedded_type_relations_keeps_qualifier_on_cross_package_embed() {
        let sites = type_relation_sites(
            Language::Go,
            "package pkg\n\ntype Dog struct {\n\tother.Animal\n}\n",
            "pkg",
        );
        assert_eq!(sites.len(), 1, "got: {sites:?}");
        assert_eq!(sites[0].target_text, "other.Animal");
    }

    #[test]
    fn go_embedded_type_relations_strips_generic_args_on_generic_embed() {
        let sites = type_relation_sites(
            Language::Go,
            "package pkg\n\ntype Dog struct {\n\tBase[int]\n}\n",
            "pkg",
        );
        assert_eq!(sites.len(), 1, "got: {sites:?}");
        assert_eq!(sites[0].target_text, "Base");
    }

    #[test]
    fn go_embedded_type_relations_skips_anonymous_inline_struct_embed() {
        let sites = type_relation_sites(
            Language::Go,
            "package pkg\n\ntype Foo struct {\n\tstruct{ X int }\n}\n",
            "pkg",
        );
        assert!(sites.is_empty(), "got: {sites:?}");
    }

    #[test]
    fn go_embedded_type_relations_skips_named_field_not_embedded() {
        let sites = type_relation_sites(
            Language::Go,
            "package pkg\n\ntype Dog struct {\n\tName string\n\tAge int\n}\n",
            "pkg",
        );
        assert!(sites.is_empty(), "got: {sites:?}");
    }

    // --- go_embedded_interface_relations ---

    #[test]
    fn go_embedded_interface_relations_captures_embedded_interface_as_implements() {
        let sites = type_relation_sites(
            Language::Go,
            "package pkg\n\ntype Reader interface {\n\tRead(p []byte) (n int, err error)\n}\n\ntype Writer interface {\n\tWrite(p []byte) (n int, err error)\n}\n\ntype ReadWriter interface {\n\tReader\n\tWriter\n}\n",
            "pkg",
        );
        let mut targets: Vec<&str> = sites.iter().map(|s| s.target_text.as_str()).collect();
        targets.sort_unstable();
        assert_eq!(targets, vec!["Reader", "Writer"], "got: {sites:?}");
        assert!(
            sites
                .iter()
                .all(|s| s.kind_hint == Some(TypeRelationKind::Implements)),
            "got: {sites:?}"
        );
    }

    #[test]
    fn go_embedded_interface_relations_does_not_treat_a_method_signature_as_an_embed() {
        let sites = type_relation_sites(
            Language::Go,
            "package pkg\n\ntype Reader interface {\n\tRead(p []byte) (n int, err error)\n}\n",
            "pkg",
        );
        assert!(sites.is_empty(), "got: {sites:?}");
    }

    #[test]
    fn go_embedded_interface_relations_keeps_qualifier_on_cross_package_embedded_interface() {
        let sites = type_relation_sites(
            Language::Go,
            "package pkg\n\ntype Named interface {\n\tio.Reader\n\tName() string\n}\n",
            "pkg",
        );
        assert_eq!(sites.len(), 1, "got: {sites:?}");
        assert_eq!(sites[0].target_text, "io.Reader");
        assert_eq!(sites[0].kind_hint, Some(TypeRelationKind::Implements));
        assert!(sites[0].dotted_text_is_go_package_qualifier);
    }

    #[test]
    fn type_relation_supports_returns_true_for_supported_languages_false_for_python_and_rust() {
        for language in [
            Language::Go,
            Language::TypeScript,
            Language::Tsx,
            Language::JavaScript,
            Language::Java,
            Language::Kotlin,
        ] {
            assert!(
                type_relation_supports(language),
                "expected {language:?} to be supported"
            );
        }
        for language in [Language::Python, Language::Rust] {
            assert!(
                !type_relation_supports(language),
                "expected {language:?} to be unsupported"
            );
        }
    }

    #[test]
    fn extract_type_relation_sites_for_file_returns_empty_for_unsupported_language() {
        // Same Go embedding source that `go_embedded_type_relations_captures_same_package_embed`
        // proves DOES produce a site when walked as Go — parsed here under a language
        // `type_relation_supports` excludes, to prove the early return in
        // `extract_type_relation_sites_for_file` short-circuits before any walk happens,
        // not merely that the walk itself happens to find nothing.
        let sites = type_relation_sites(
            Language::Python,
            "package pkg\n\ntype Animal struct{}\n\ntype Dog struct {\n\tAnimal\n\tName string\n}\n",
            "pkg",
        );
        assert_eq!(sites, Vec::new());
    }

    // --- kotlin_delegation_type_relations (Epic 1.5) ---

    #[test]
    fn kotlin_delegation_type_relations_constructor_invocation_is_extends() {
        let sites = type_relation_sites(
            Language::Kotlin,
            "class Dog : Animal(), Runnable {}\n",
            "pkg",
        );
        let animal = sites
            .iter()
            .find(|s| s.target_text == "Animal")
            .unwrap_or_else(|| panic!("no Animal site in {sites:?}"));
        assert_eq!(animal.kind_hint, Some(TypeRelationKind::Extends));
        assert_eq!(animal.type_id, "pkg::Dog");
    }

    #[test]
    fn kotlin_delegation_type_relations_explicit_delegation_is_implements() {
        let sites = type_relation_sites(
            Language::Kotlin,
            "class Dog(impl: Runnable) : Runnable by impl {}\n",
            "pkg",
        );
        let runnable = sites
            .iter()
            .find(|s| s.target_text == "Runnable")
            .unwrap_or_else(|| panic!("no Runnable site in {sites:?}"));
        assert_eq!(runnable.kind_hint, Some(TypeRelationKind::Implements));
    }

    #[test]
    fn kotlin_delegation_type_relations_bare_type_is_ambiguous() {
        let sites = type_relation_sites(Language::Kotlin, "class Dog : Runnable {}\n", "pkg");
        let runnable = sites
            .iter()
            .find(|s| s.target_text == "Runnable")
            .unwrap_or_else(|| panic!("no Runnable site in {sites:?}"));
        assert_eq!(runnable.kind_hint, None);
    }

    #[test]
    fn kotlin_delegation_type_relations_no_primary_constructor_superclass_is_ambiguous_not_implements()
     {
        // Kotlin's "class with no primary constructor" idiom: each secondary constructor
        // delegates via `: super(...)`, and the superclass name is written bare after `:` —
        // exactly the same syntax a bare interface reference would use. `View` here is
        // actually a class (real Kotlin/Android code writes this for e.g. custom Views), so
        // classifying it `Implements` would be a real bug — this is the regression test for
        // that trap.
        let sites = type_relation_sites(
            Language::Kotlin,
            "class MyView : View {\n    constructor(ctx: Int) : super(ctx)\n}\n",
            "pkg",
        );
        let view = sites
            .iter()
            .find(|s| s.target_text == "View")
            .unwrap_or_else(|| panic!("no View site in {sites:?}"));
        assert_eq!(view.kind_hint, None);
    }

    #[test]
    fn kotlin_delegation_type_relations_splits_mixed_supertype_list() {
        let sites = type_relation_sites(
            Language::Kotlin,
            "class Dog : Animal(), Runnable, Named {}\n",
            "pkg",
        );
        assert_eq!(sites.len(), 3, "got: {sites:?}");
        assert!(sites.iter().all(|s| s.type_id == "pkg::Dog"));

        let find = |name: &str| {
            sites
                .iter()
                .find(|s| s.target_text == name)
                .unwrap_or_else(|| panic!("no {name} site in {sites:?}"))
        };
        assert_eq!(find("Animal").kind_hint, Some(TypeRelationKind::Extends));
        assert_eq!(find("Runnable").kind_hint, None);
        assert_eq!(find("Named").kind_hint, None);
    }
}
