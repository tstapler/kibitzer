//! The third whole-repo parsing pass (per ADR-001), alongside `checker`'s per-file AST
//! checks and `import_graph`'s package/module dependency graph: a flat enumeration of
//! every top-level declaration (struct/class/interface/enum/function) across the repo,
//! feeding `declaration_checks`'s `ContentChecker` (Story 2.2.1) and `NamingChecker`
//! (Phase 3). Go and JS/TS first, mirroring `import_graph.rs`'s own initial-language
//! scope before Java/Kotlin/Python were added in later phases (Java/Kotlin land in
//! Epic 4.3, Python in Epic 5.3, both below).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tree_sitter::Node;

use crate::checker::Language;
use crate::config::{Component, component_of};

/// The shape of one enumerated declaration. `Enum` has no producer yet in Go/JS/TS
/// (Go has no enum node kind; JS/TS `enum` support can follow later) — kept here now so
/// `ContentRule`/`NamingRule`'s `allowed_kinds`/`kind` string parsing (Story 2.2.1) has a
/// stable variant set to validate against from the start, matching plan.md Task 2.1.1a's
/// literal enum definition.
///
/// `Function` is never produced for Java (Story 4.3.1): Java has no top-level functions
/// outside a class/interface body, unlike Go/JS/TS/Kotlin, so method declarations are
/// out of scope for Java content/naming rules in this module — documented here rather
/// than silently half-supported. Kotlin *does* have top-level `fun`s, but extracting
/// them is outside this story's verified scope too (no AC/test names this story).
///
/// No `Object` variant exists for Kotlin's `object` declarations — see
/// `collect_kotlin_declarations`'s doc comment for why `object` maps onto `Class`
/// instead of a new variant.
///
/// `Struct`/`Interface`/`Enum` are never produced for Python (Story 5.3.1): Python has
/// no struct/interface/enum node kind distinct from `class_definition` (an
/// `abc.ABC`/`Protocol`/`enum.Enum` subclass is still, syntactically, a `class`), so
/// every Python class maps onto `Class` — matching Java's `Function`-inapplicable note
/// above as the same kind of documented, not silently half-supported, scope limit.
// Consumed by `declaration_checks::ContentChecker` starting Story 2.2.1 and
// `NamingChecker` starting Phase 3 — not yet called from `main`'s reachable paths
// outside tests (same forward-scaffolding precedent as `config.rs`'s `ContentRule`/
// `NamingRule`).
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclKind {
    Struct,
    Class,
    Interface,
    Enum,
    Function,
}

/// One enumerated declaration: what it's called, what kind of thing it is, and where it
/// lives — `component` resolved the same way `ImportGraph` nodes eventually get resolved
/// to a component (via `config::component_of`), but matched against the declaration's
/// *file's* repo-relative path rather than a package-identity string (Go/JS/TS package
/// names aren't derived per-declaration the way `ImportGraph` nodes are).
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declaration {
    pub name: String,
    pub kind: DeclKind,
    pub file: PathBuf,
    pub line: usize,
    pub component: Option<String>,
}

/// A repo's full declaration inventory — flat, not graph-shaped (there's no edge concept
/// analogous to `ImportEdge` here), but named `DeclarationGraph` to match `ImportGraph`'s
/// naming convention as this pass's top-level output type.
#[allow(dead_code)]
#[derive(Debug, Default, Clone)]
pub struct DeclarationGraph {
    pub declarations: Vec<Declaration>,
}

/// Build the declaration graph over `files` (already filtered to files kibitzer is
/// scoped to). Go, TypeScript/JavaScript, Java, Kotlin, and Python are extracted.
// Consumed by `check.rs`'s `AnyArchitectureChecker::Declaration` arm starting Story
// 2.2.1, once `declaration_checks::lookup` returns `Some` for the first time — not yet
// called outside tests.
#[allow(dead_code)]
pub fn build(
    repo_root: &Path,
    files: &[PathBuf],
    components: &[Component],
) -> Result<DeclarationGraph> {
    let mut graph = DeclarationGraph::default();

    let go_files: Vec<&PathBuf> = files_for(files, Language::Go);
    if !go_files.is_empty() {
        build_go_declarations(repo_root, &go_files, components, &mut graph)?;
    }

    let js_files: Vec<&PathBuf> = files
        .iter()
        .filter(|f| crate::import_graph::is_js_like(Language::for_path(f)))
        .collect();
    if !js_files.is_empty() {
        build_js_ts_declarations(repo_root, &js_files, components, &mut graph)?;
    }

    let java_files: Vec<&PathBuf> = files_for(files, Language::Java);
    if !java_files.is_empty() {
        build_java_declarations(repo_root, &java_files, components, &mut graph)?;
    }

    let kotlin_files: Vec<&PathBuf> = files_for(files, Language::Kotlin);
    if !kotlin_files.is_empty() {
        build_kotlin_declarations(repo_root, &kotlin_files, components, &mut graph)?;
    }

    let python_files: Vec<&PathBuf> = files_for(files, Language::Python);
    if !python_files.is_empty() {
        build_python_declarations(repo_root, &python_files, components, &mut graph)?;
    }

    // No Rust arm yet: this graph doesn't extract Rust declarations at all (a separate,
    // not-yet-built feature — see `symbol_extract.rs` for the equivalent Rust support
    // that *does* exist, for architecture-export/LSP purposes rather than
    // content/naming-rule checking). Left out deliberately rather than silently, unlike
    // the bug `Language::extensions`'s doc comment describes: this function is still
    // `#[allow(dead_code)]`/test-only, so there's no live path for this gap to bite
    // through yet.
    Ok(graph)
}

/// Every file in `files` whose extension `Language::for_path` maps to `lang` — see
/// `import_graph.rs`'s identically-named, identically-purposed helper.
fn files_for(files: &[PathBuf], lang: Language) -> Vec<&PathBuf> {
    files
        .iter()
        .filter(|f| Language::for_path(f) == Some(lang))
        .collect()
}

/// Resolves `file`'s repo-relative path to a component name via the same
/// `matches_scope`-based resolution `component_of` uses for `ImportGraph` nodes — here
/// matched against the file path string itself rather than a package-identity string.
fn resolve_component(repo_root: &Path, file: &Path, components: &[Component]) -> Option<String> {
    let rel = file.strip_prefix(repo_root).unwrap_or(file);
    let rel_str = rel.to_string_lossy().replace('\\', "/");
    component_of(&rel_str, components).map(str::to_string)
}

// ---------------------------------------------------------------------------------
// Go
// ---------------------------------------------------------------------------------

/// Recursively walks the tree collecting `(name, kind, line)` triples. Verified against
/// tree-sitter-go's real `to_sexp()` output (not guessed by analogy), per this
/// codebase's stated node-kind discipline (see `rules.rs`'s `LangRuleConfig` doc comment):
///
/// - `type_declaration` wraps one or more `type_spec` children, each carrying a `name`
///   field (`type_identifier`) and a `type` field whose *own* node kind distinguishes
///   `struct_type` from `interface_type` — e.g. `type Order struct { ID string }` parses
///   as `(type_declaration (type_spec name: (type_identifier) type: (struct_type ...)))`.
/// - `function_declaration` (top-level `func F(...)`) and `method_declaration` (`func (r
///   T) M(...)`) both carry a `name` field (`identifier`/`field_identifier`
///   respectively) directly usable without distinguishing the two further — both map to
///   `DeclKind::Function`.
fn collect_go_declarations(node: Node, src: &[u8], out: &mut Vec<(String, DeclKind, usize)>) {
    match node.kind() {
        "type_spec" => {
            if let Some(name_node) = node.child_by_field_name("name")
                && let Some(type_node) = node.child_by_field_name("type")
                && let Ok(name) = name_node.utf8_text(src)
            {
                let kind = match type_node.kind() {
                    "struct_type" => Some(DeclKind::Struct),
                    "interface_type" => Some(DeclKind::Interface),
                    _ => None,
                };
                if let Some(kind) = kind {
                    out.push((name.to_string(), kind, node.start_position().row + 1));
                }
            }
        }
        "function_declaration" | "method_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name")
                && let Ok(name) = name_node.utf8_text(src)
            {
                out.push((
                    name.to_string(),
                    DeclKind::Function,
                    node.start_position().row + 1,
                ));
            }
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_go_declarations(child, src, out);
    }
}

/// Uses a fresh `tree_sitter::Parser` per file, matching `import_graph.rs::build_go`'s
/// existing pattern rather than `checker::GrammarCache` — `declarations.rs` is its own
/// whole-repo pass with its own parse lifetime, not a per-file `Checker` sharing a cache
/// across many different checkers (architecture.md §4, point 3). Threading a shared
/// `GrammarCache` instance through `build()` would also work; a fresh parser is chosen
/// here to keep this new module's diff self-contained, matching `import_graph.rs`'s own
/// precedent line-for-line.
fn build_go_declarations(
    repo_root: &Path,
    files: &[&PathBuf],
    components: &[Component],
    graph: &mut DeclarationGraph,
) -> Result<()> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_go::LANGUAGE.into())
        .context("loading tree-sitter-go grammar")?;

    for file in files {
        let source =
            std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
        let tree = parser
            .parse(&source, None)
            .with_context(|| format!("parsing {} with tree-sitter-go", file.display()))?;

        let component = resolve_component(repo_root, file, components);
        let mut decls = Vec::new();
        collect_go_declarations(tree.root_node(), source.as_bytes(), &mut decls);
        for (name, kind, line) in decls {
            graph.declarations.push(Declaration {
                name,
                kind,
                file: (*file).clone(),
                line,
                component: component.clone(),
            });
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------------
// TypeScript / JavaScript
// ---------------------------------------------------------------------------------

/// Recursively walks the tree collecting `(name, kind, line)` triples. Verified against
/// tree-sitter-typescript's real `to_sexp()` output: `class_declaration` and
/// `interface_declaration` both carry a `name` field (`type_identifier`) directly,
/// whether top-level or wrapped in an `export_statement`/`export default` — the recursive
/// walk finds them regardless of the wrapper, so no special-casing of `export_statement`
/// is needed (matching `import_graph.rs::collect_js_imports`'s own approach of recursing
/// into every child unconditionally). `function_declaration` carries a `name` field
/// (`identifier`).
fn collect_js_ts_declarations(node: Node, src: &[u8], out: &mut Vec<(String, DeclKind, usize)>) {
    let kind = match node.kind() {
        "class_declaration" => Some(DeclKind::Class),
        "interface_declaration" => Some(DeclKind::Interface),
        "function_declaration" => Some(DeclKind::Function),
        _ => None,
    };
    if let Some(kind) = kind
        && let Some(name_node) = node.child_by_field_name("name")
        && let Ok(name) = name_node.utf8_text(src)
    {
        out.push((name.to_string(), kind, node.start_position().row + 1));
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_js_ts_declarations(child, src, out);
    }
}

/// Reuses `import_graph::js_ts_language` for per-extension grammar selection (made
/// `pub(crate)` there for this — see Task 2.1.2a) rather than duplicating the
/// `.tsx`/`.ts`/other match.
fn build_js_ts_declarations(
    repo_root: &Path,
    files: &[&PathBuf],
    components: &[Component],
    graph: &mut DeclarationGraph,
) -> Result<()> {
    for file in files {
        let source =
            std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;

        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&crate::import_graph::js_ts_language(file))
            .with_context(|| format!("loading tree-sitter grammar for {}", file.display()))?;
        let tree = parser
            .parse(&source, None)
            .with_context(|| format!("parsing {}", file.display()))?;

        let component = resolve_component(repo_root, file, components);
        let mut decls = Vec::new();
        collect_js_ts_declarations(tree.root_node(), source.as_bytes(), &mut decls);
        for (name, kind, line) in decls {
            graph.declarations.push(Declaration {
                name,
                kind,
                file: (*file).clone(),
                line,
                component: component.clone(),
            });
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------------
// Java
// ---------------------------------------------------------------------------------

/// Recursively walks the tree collecting `(name, kind, line)` triples. Verified against
/// tree-sitter-java 0.23.5's real `to_sexp()` output (this file's test module): unlike
/// Kotlin below, `class_declaration`, `interface_declaration`, and `record_declaration`
/// are each their own dedicated node kind — no shared-kind keyword disambiguation is
/// needed — and each carries a `name` field directly, e.g. `public interface Repository
/// {}` parses as `(interface_declaration (modifiers) name: (identifier)
/// body: (interface_body))`.
///
/// A leading annotation (`@Deprecated`, `@FunctionalInterface`) nests inside the
/// declaration node's own `modifiers` child and does not shift or hide the `name` field —
/// confirmed via real `to_sexp()` output: `(class_declaration (modifiers
/// (marker_annotation name: (identifier))) name: (identifier) body: (class_body))`. So,
/// unlike Kotlin's positional keyword scan, Java's classification is pure
/// `node.kind()` dispatch plus `child_by_field_name("name")` — the same shape Go/JS/TS
/// already use — and is unaffected by annotations for exactly that reason.
///
/// `record_declaration` maps to `DeclKind::Class`: this module's schema (`DeclKind`) has
/// no dedicated "record" variant, and a Java record is fundamentally a restricted class
/// (javac itself compiles a record to a `final class`), making `Class` the closest
/// existing category rather than an invented one.
///
/// Java has no top-level functions outside a class/interface/record body — method
/// declarations are intentionally never extracted here (see `DeclKind`'s doc comment).
fn collect_java_declarations(node: Node, src: &[u8], out: &mut Vec<(String, DeclKind, usize)>) {
    let kind = match node.kind() {
        "class_declaration" | "record_declaration" => Some(DeclKind::Class),
        "interface_declaration" => Some(DeclKind::Interface),
        _ => None,
    };
    if let Some(kind) = kind
        && let Some(name_node) = node.child_by_field_name("name")
        && let Ok(name) = name_node.utf8_text(src)
    {
        out.push((name.to_string(), kind, node.start_position().row + 1));
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_java_declarations(child, src, out);
    }
}

/// Fresh `tree_sitter::Parser` per file, matching `build_go_declarations`/
/// `build_js_ts_declarations`'s own precedent above.
fn build_java_declarations(
    repo_root: &Path,
    files: &[&PathBuf],
    components: &[Component],
    graph: &mut DeclarationGraph,
) -> Result<()> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_java::LANGUAGE.into())
        .context("loading tree-sitter-java grammar")?;

    for file in files {
        let source =
            std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
        let tree = parser
            .parse(&source, None)
            .with_context(|| format!("parsing {} with tree-sitter-java", file.display()))?;

        let component = resolve_component(repo_root, file, components);
        let mut decls = Vec::new();
        collect_java_declarations(tree.root_node(), source.as_bytes(), &mut decls);
        for (name, kind, line) in decls {
            graph.declarations.push(Declaration {
                name,
                kind,
                file: (*file).clone(),
                line,
                component: component.clone(),
            });
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------------
// Kotlin
// ---------------------------------------------------------------------------------

/// Recursively walks the tree collecting `(name, kind, line)` triples. Verified against
/// tree-sitter-kotlin-ng 1.1.0's real `to_sexp()` output (Task 4.1.2c, this file's test
/// module, plus further verification for this story below):
///
/// - `class` and `interface` share **one** node kind, `class_declaration` — distinguished
///   only by an anonymous (field-less) keyword token child whose kind is the literal
///   `"class"` or `"interface"`. That keyword can be preceded by a `modifiers` node
///   (e.g. `abstract class Foo`, or an annotation like `@Suppress("unused")`), so it's
///   found by scanning raw children for kind, never by a fixed positional index.
/// - `object` declarations are a wholly separate node kind, `object_declaration` — not a
///   `class_declaration` variant, so it needs its own match arm rather than a keyword
///   check alongside class/interface.
///
/// `object_declaration` maps to `DeclKind::Class`: `DeclKind` has no `Object` variant,
/// and `declaration_checks::kind_name()`'s match over `DeclKind` is exhaustive and lives
/// in a file outside this story's `src/declarations.rs`-only scope — adding a new variant
/// here would leave that match non-exhaustive with no in-scope way to fix it. `Class` is
/// the closest existing category semantically (a Kotlin `object` compiles to a singleton
/// JVM class), and is used here as the only in-scope mapping once a new variant is ruled
/// out — not an arbitrary, unexamined choice.
///
/// Kotlin, unlike Java, does have top-level free functions (`fun`) — but no AC/test name
/// in Story 4.3.1 calls for extracting them, so `DeclKind::Function` is intentionally
/// never produced here either, matching this story's verified scope (class/interface/
/// object only, mirroring `DeclKind`'s doc comment).
///
/// Verified further (this file's malformed-source tests below): a single-line brace body
/// immediately following the header (e.g. `interface Repository { fun save() }`, all on
/// one line) can itself misparse internally in this grammar version — the body comes back
/// as `enum_class_body` wrapping an `ERROR` node instead of a clean `class_body` — while a
/// multi-line body (`{` then a newline) parses cleanly. Regardless, the *outer*
/// `class_declaration`/`object_declaration` node's own `kind()`, keyword child, and
/// `name` field stay fully intact in both cases, because this walk only ever reads the
/// declaration node's own direct children — never anything inside its body — so
/// classification here is unaffected either way.
fn collect_kotlin_declarations(node: Node, src: &[u8], out: &mut Vec<(String, DeclKind, usize)>) {
    match node.kind() {
        "class_declaration" => {
            let mut kw_cursor = node.walk();
            let keyword_kind = node
                .children(&mut kw_cursor)
                .find(|c| c.kind() == "class" || c.kind() == "interface")
                .map(|c| c.kind());
            let kind = match keyword_kind {
                Some("class") => Some(DeclKind::Class),
                Some("interface") => Some(DeclKind::Interface),
                _ => None,
            };
            if let Some(kind) = kind
                && let Some(name_node) = node.child_by_field_name("name")
                && let Ok(name) = name_node.utf8_text(src)
            {
                out.push((name.to_string(), kind, node.start_position().row + 1));
            }
        }
        "object_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name")
                && let Ok(name) = name_node.utf8_text(src)
            {
                out.push((
                    name.to_string(),
                    DeclKind::Class,
                    node.start_position().row + 1,
                ));
            }
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_kotlin_declarations(child, src, out);
    }
}

/// Fresh `tree_sitter::Parser` per file, matching `build_go_declarations`/
/// `build_js_ts_declarations`/`build_java_declarations`'s own precedent above.
fn build_kotlin_declarations(
    repo_root: &Path,
    files: &[&PathBuf],
    components: &[Component],
    graph: &mut DeclarationGraph,
) -> Result<()> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_kotlin_ng::LANGUAGE.into())
        .context("loading tree-sitter-kotlin-ng grammar")?;

    for file in files {
        let source =
            std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
        let tree = parser
            .parse(&source, None)
            .with_context(|| format!("parsing {} with tree-sitter-kotlin-ng", file.display()))?;

        let component = resolve_component(repo_root, file, components);
        let mut decls = Vec::new();
        collect_kotlin_declarations(tree.root_node(), source.as_bytes(), &mut decls);
        for (name, kind, line) in decls {
            graph.declarations.push(Declaration {
                name,
                kind,
                file: (*file).clone(),
                line,
                component: component.clone(),
            });
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------------
// Python
// ---------------------------------------------------------------------------------

/// Classifies a single `class_definition`/`function_definition` node — shared by the
/// direct match arm below and by the `decorated_definition` unwrap, so a decorator
/// can never change how the wrapped declaration is classified.
fn classify_python_definition(node: Node, src: &[u8]) -> Option<(String, DeclKind, usize)> {
    let kind = match node.kind() {
        "class_definition" => DeclKind::Class,
        "function_definition" => DeclKind::Function,
        _ => return None,
    };
    let name = node.child_by_field_name("name")?.utf8_text(src).ok()?;
    Some((name.to_string(), kind, node.start_position().row + 1))
}

/// Only `module`'s **direct named children** are considered — this walk deliberately
/// does not recurse into a `class_definition`'s or `function_definition`'s own `body`,
/// which is what keeps a nested method (e.g. `__init__` inside a class) from being
/// extracted as if it were a top-level declaration. Verified against real
/// `tree-sitter-python` 0.23.6 `to_sexp()` output (Story 5.1.1, this file's
/// `python_class_and_function_definition_shapes` test above):
///
/// - `class_definition` and `function_definition` both carry a named `name` field
///   (`identifier`) directly, same `child_by_field_name` shape as Go/JS/Java —
///   `classify_python_definition` handles both.
/// - A top-level function sits one level below `module`; a method nested inside a
///   class sits two levels below (`module → class_definition → body (block) →
///   function_definition`). Because this function only iterates `module`'s own
///   `named_children()` and never descends into a matched node's `body`, the nested
///   case is structurally unreachable here — this is what makes the walk depth-aware
///   rather than a blind recursive kind-match (which would also find `__init__`).
/// - A decorated top-level declaration (`@dataclass\nclass Order: ...` or
///   `@app.route(...)\ndef handler(): ...`) does **not** produce a `class_definition`/
///   `function_definition` as `module`'s direct child — it wraps in a
///   `decorated_definition` node instead, confirmed via `tree-sitter-python`
///   0.23.6's `node-types.json` (`decorated_definition` has a required `definition`
///   field of type `class_definition`/`function_definition`, plus one-or-more
///   `decorator` children). A depth-aware walk that only matched on
///   `class_definition`/`function_definition` node kind would silently drop every
///   decorated top-level declaration — the same silent-misclassification risk
///   pre-mortem #3 flagged for Java/Kotlin's positional walks, here from an unhandled
///   node kind rather than a positional miss. So `decorated_definition` is unwrapped
///   via its `definition` field and the inner node is classified the normal way,
///   still without recursing into that node's own `body` — decorators never hide or
///   reclassify what they wrap, matching Java's "annotations don't shift positional
///   classification" precedent (`java_declarations_annotation_does_not_shift_positional_classification`).
fn collect_python_declarations(root: Node, src: &[u8], out: &mut Vec<(String, DeclKind, usize)>) {
    let mut cursor = root.walk();
    for child in root.named_children(&mut cursor) {
        match child.kind() {
            "class_definition" | "function_definition" => {
                if let Some(decl) = classify_python_definition(child, src) {
                    out.push(decl);
                }
            }
            "decorated_definition" => {
                if let Some(inner) = child.child_by_field_name("definition")
                    && let Some(decl) = classify_python_definition(inner, src)
                {
                    out.push(decl);
                }
            }
            _ => {}
        }
    }
}

/// Fresh `tree_sitter::Parser` per file, matching `build_go_declarations`/
/// `build_js_ts_declarations`/`build_java_declarations`/`build_kotlin_declarations`'s
/// own precedent above.
fn build_python_declarations(
    repo_root: &Path,
    files: &[&PathBuf],
    components: &[Component],
    graph: &mut DeclarationGraph,
) -> Result<()> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_python::LANGUAGE.into())
        .context("loading tree-sitter-python grammar")?;

    for file in files {
        let source =
            std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
        let tree = parser
            .parse(&source, None)
            .with_context(|| format!("parsing {} with tree-sitter-python", file.display()))?;

        let component = resolve_component(repo_root, file, components);
        let mut decls = Vec::new();
        collect_python_declarations(tree.root_node(), source.as_bytes(), &mut decls);
        for (name, kind, line) in decls {
            graph.declarations.push(Declaration {
                name,
                kind,
                file: (*file).clone(),
                line,
                component: component.clone(),
            });
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn tmp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "kibitzer-declarations-test-{}-{name}-{}",
            std::process::id(),
            TMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(dir: &Path, rel: &str, contents: &str) -> PathBuf {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, contents).unwrap();
        path
    }

    // --- Story 2.1.1: DeclKind/Declaration/DeclarationGraph + Go extraction ---

    #[test]
    fn go_declarations_finds_struct_and_function() {
        let dir = tmp_dir("go-struct-fn");
        let domain_go_path = write(
            &dir,
            "domain/domain.go",
            "package domain\n\ntype Order struct {\n    ID string\n}\n\n\nfunc Validate(o Order) error { return nil }\n",
        );
        let components = vec![Component {
            name: "domain".into(),
            paths: vec!["**/domain".into(), "**/domain/**".into()],
        }];

        let graph = build(&dir, std::slice::from_ref(&domain_go_path), &components).unwrap();

        assert!(graph.declarations.contains(&Declaration {
            name: "Order".to_string(),
            kind: DeclKind::Struct,
            file: domain_go_path.clone(),
            line: 3,
            component: Some("domain".to_string()),
        }));
        assert!(graph.declarations.contains(&Declaration {
            name: "Validate".to_string(),
            kind: DeclKind::Function,
            file: domain_go_path.clone(),
            line: 8,
            component: Some("domain".to_string()),
        }));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn go_declarations_finds_interface() {
        let dir = tmp_dir("go-interface");
        let domain_go_path = write(
            &dir,
            "domain/domain.go",
            "package domain\n\ntype Order struct {\n    ID string\n}\n\ntype Repository interface { Save(Order) error }\n",
        );
        let components = vec![Component {
            name: "domain".into(),
            paths: vec!["**/domain".into(), "**/domain/**".into()],
        }];

        let graph = build(&dir, std::slice::from_ref(&domain_go_path), &components).unwrap();

        assert!(graph.declarations.iter().any(|d| d.name == "Repository"
            && d.kind == DeclKind::Interface
            && d.component.as_deref() == Some("domain")));

        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- Epic 4.1 / Task 4.1.2c: Kotlin class/interface/object to_sexp() verification ---
    //
    // Real `to_sexp()` output (`tree-sitter-kotlin-ng` 1.1.0, matching `Cargo.lock`),
    // pinned ahead of the real Kotlin extraction landing in Story 4.3 — confirms the
    // plan's Domain Glossary assumption *half* right:
    // - `class Order(val id: String)` and `interface Repository { fun save() }` both
    //   parse to the **same** node kind, `class_declaration`, with field `name` —
    //   distinguished only by the raw (unnamed) keyword child at position 0 (`"class"`
    //   vs. `"interface"`), same no-named-fields-for-everything-else situation
    //   `syntax-rules.md` already documents for Kotlin's function nodes.
    // - `object Singleton { val x = 1 }` does **not** share `class_declaration` — it
    //   is a wholly separate node kind, `object_declaration`, also with field `name`.
    //   This refutes a shared-node-kind-for-all-three assumption; `object` needs its
    //   own match arm, not a keyword-child check alongside class/interface.
    // - `abstract class Base` wraps the keyword in a preceding `modifiers
    //   (inheritance_modifier)` node before the `"class"` keyword child — confirms
    //   modifiers can precede the distinguishing keyword and must be skipped over
    //   (search by kind, not by fixed positional index).

    fn parse_kotlin(src: &str) -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_kotlin_ng::LANGUAGE.into())
            .expect("loading tree-sitter-kotlin-ng grammar");
        parser.parse(src, None).expect("parsing Kotlin fixture")
    }

    #[test]
    fn kotlin_class_and_interface_share_class_declaration_node_kind() {
        let class_tree = parse_kotlin("class Order(val id: String)\n");
        let iface_tree = parse_kotlin("interface Repository {\n    fun save()\n}\n");

        let class_node = class_tree.root_node().named_child(0).unwrap();
        let iface_node = iface_tree.root_node().named_child(0).unwrap();

        assert_eq!(class_node.kind(), "class_declaration");
        assert_eq!(iface_node.kind(), "class_declaration");

        // Distinguishing keyword is the raw (unnamed) first child.
        assert_eq!(class_node.child(0).unwrap().kind(), "class");
        assert_eq!(iface_node.child(0).unwrap().kind(), "interface");
    }

    #[test]
    fn kotlin_object_is_a_distinct_node_kind_from_class_declaration() {
        let object_tree = parse_kotlin("object Singleton {\n    val x = 1\n}\n");
        let object_node = object_tree.root_node().named_child(0).unwrap();

        assert_eq!(object_node.kind(), "object_declaration");
        assert_ne!(object_node.kind(), "class_declaration");
    }

    #[test]
    fn kotlin_abstract_class_wraps_keyword_in_a_preceding_modifiers_node() {
        let tree = parse_kotlin("abstract class Base\n");
        let class_node = tree.root_node().named_child(0).unwrap();

        assert_eq!(class_node.kind(), "class_declaration");
        // The distinguishing "class"/"interface" keyword is no longer at raw index 0
        // once modifiers precede it — a real extractor must search children by kind,
        // not assume a fixed positional index.
        assert_eq!(class_node.child(0).unwrap().kind(), "modifiers");
        let has_class_keyword = (0..class_node.child_count())
            .any(|i| class_node.child(i as u32).unwrap().kind() == "class");
        assert!(has_class_keyword);
    }

    // --- Epic 5.1 / Story 5.1.1: Python class_definition/function_definition
    // to_sexp() verification ("one additional shape variant" beyond the six import
    // node kinds verified in `import_graph.rs`) ---
    //
    // Real `to_sexp()` output (`tree-sitter-python` 0.23.6, matching `Cargo.lock`),
    // pinned ahead of the real Python extraction landing in Story 5.3.1:
    // - `class Order:` produces node kind `class_definition` with a named `name` field
    //   (`identifier`) and a named `body` field (`block`) — proper fields, same as
    //   Go/JS's straightforward `child_by_field_name` pattern, not Kotlin's positional
    //   children.
    // - `def validate(...) -> bool:` at module level produces node kind
    //   `function_definition`, also with named `name`/`parameters`/`body` fields (plus
    //   an optional `return_type` field when a `->` annotation is present).
    // - A method nested inside a class (`__init__` inside `class Order:`) is **not** a
    //   direct child of `module` — it sits two levels deeper:
    //   `module → class_definition → body (block) → function_definition`. A top-level
    //   function (`validate`) is one level deep: `module → function_definition`
    //   directly. This structural difference — not node kind, both are
    //   `function_definition` — is exactly what Story 5.3.1's "the nested `__init__`
    //   is not extracted as a separate top-level declaration" requires: only walk
    //   `module`'s direct named children, don't recurse into `class_definition`'s body.

    fn parse_python(src: &str) -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .expect("loading tree-sitter-python grammar");
        parser.parse(src, None).expect("parsing Python fixture")
    }

    /// Story 5.3.1's fixture, parsed and asserted on real `to_sexp()`-derived structure
    /// (not just node kind — the whole point of this test is confirming the *nesting
    /// depth* distinction between a top-level function and a class method).
    #[test]
    fn python_class_and_function_definition_shapes() {
        let src = "class Order:\n    def __init__(self, id: str):\n        self.id = id\n\ndef validate(order: Order) -> bool:\n    return bool(order.id)\n";
        let tree = parse_python(src);
        let root = tree.root_node();
        assert!(!root.has_error());

        // Module-level children: class_definition, then function_definition — exactly
        // 2, confirming the nested __init__ is NOT a direct child of module.
        assert_eq!(root.named_child_count(), 2);

        let class_node = root.named_child(0).unwrap();
        assert_eq!(class_node.kind(), "class_definition");
        assert_eq!(
            class_node
                .child_by_field_name("name")
                .unwrap()
                .utf8_text(src.as_bytes())
                .unwrap(),
            "Order"
        );

        let top_level_fn = root.named_child(1).unwrap();
        assert_eq!(top_level_fn.kind(), "function_definition");
        assert_eq!(
            top_level_fn
                .child_by_field_name("name")
                .unwrap()
                .utf8_text(src.as_bytes())
                .unwrap(),
            "validate"
        );

        // The nested __init__ is two levels below module: class_definition -> body
        // (block) -> function_definition. Confirms the exclusion boundary a top-level-
        // only walk must respect.
        let class_body = class_node.child_by_field_name("body").unwrap();
        assert_eq!(class_body.kind(), "block");
        let nested_method = class_body.named_child(0).unwrap();
        assert_eq!(nested_method.kind(), "function_definition");
        assert_eq!(
            nested_method
                .child_by_field_name("name")
                .unwrap()
                .utf8_text(src.as_bytes())
                .unwrap(),
            "__init__"
        );
    }

    // --- Story 2.1.2: JS/TS declaration extraction ---

    #[test]
    fn ts_declarations_finds_class_and_interface() {
        let dir = tmp_dir("ts-class-interface");
        let order_ts_path = write(
            &dir,
            "web/src/domain/Order.ts",
            "export interface Repository { save(o: Order): Promise<void>; }\nexport class Order { id: string; }\n",
        );

        let graph = build(&dir, std::slice::from_ref(&order_ts_path), &[]).unwrap();

        assert!(graph.declarations.iter().any(|d| d.name == "Repository"
            && d.kind == DeclKind::Interface
            && d.file == order_ts_path
            && d.line == 1));
        assert!(graph.declarations.iter().any(|d| d.name == "Order"
            && d.kind == DeclKind::Class
            && d.file == order_ts_path
            && d.line == 2));

        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- Epic 4.3 / Story 4.3.1: Java + Kotlin declaration extraction ---

    #[test]
    fn java_declarations_distinguishes_class_and_interface() {
        let dir = tmp_dir("java-class-interface");
        let path = write(
            &dir,
            "com/example/infra/DbClient.java",
            "package com.example.infra;\n\npublic interface Repository {}\n\npublic class DbClient implements Repository {}\n",
        );

        let graph = build(&dir, std::slice::from_ref(&path), &[]).unwrap();

        assert!(
            graph
                .declarations
                .iter()
                .any(|d| d.name == "Repository" && d.kind == DeclKind::Interface && d.file == path)
        );
        assert!(
            graph
                .declarations
                .iter()
                .any(|d| d.name == "DbClient" && d.kind == DeclKind::Class && d.file == path)
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn kotlin_declarations_distinguishes_class_interface_object() {
        let dir = tmp_dir("kotlin-class-interface-object");
        // Story 4.1.2c's verified fixture shapes (multi-line bodies — see
        // `collect_kotlin_declarations`'s doc comment for why single-line bodies are
        // avoided here), combined into one file.
        let path = write(
            &dir,
            "domain/Order.kt",
            "class Order(val id: String)\n\ninterface Repository {\n    fun save()\n}\n\nobject Singleton {\n    val x = 1\n}\n",
        );

        let graph = build(&dir, std::slice::from_ref(&path), &[]).unwrap();

        assert!(
            graph
                .declarations
                .iter()
                .any(|d| d.name == "Order" && d.kind == DeclKind::Class && d.file == path)
        );
        assert!(
            graph
                .declarations
                .iter()
                .any(|d| d.name == "Repository" && d.kind == DeclKind::Interface && d.file == path)
        );
        // `object` maps to `Class` — see `collect_kotlin_declarations`'s doc comment.
        assert!(
            graph
                .declarations
                .iter()
                .any(|d| d.name == "Singleton" && d.kind == DeclKind::Class && d.file == path)
        );
        assert_eq!(graph.declarations.len(), 3);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn java_declarations_annotation_does_not_shift_positional_classification() {
        let dir = tmp_dir("java-annotated");
        let path = write(
            &dir,
            "com/example/domain/Order.java",
            "package com.example.domain;\n\n@Deprecated\npublic class Order {}\n\n@FunctionalInterface\npublic interface Validator { boolean validate(Order o); }\n",
        );

        let graph = build(&dir, std::slice::from_ref(&path), &[]).unwrap();

        assert!(
            graph
                .declarations
                .iter()
                .any(|d| d.name == "Order" && d.kind == DeclKind::Class)
        );
        assert!(
            graph
                .declarations
                .iter()
                .any(|d| d.name == "Validator" && d.kind == DeclKind::Interface)
        );
        // Neither misclassified as the other, nor dropped.
        assert!(
            !graph
                .declarations
                .iter()
                .any(|d| d.name == "Order" && d.kind == DeclKind::Interface)
        );
        assert!(
            !graph
                .declarations
                .iter()
                .any(|d| d.name == "Validator" && d.kind == DeclKind::Class)
        );
        assert_eq!(graph.declarations.len(), 2);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn kotlin_declarations_annotation_does_not_shift_positional_classification() {
        let dir = tmp_dir("kotlin-annotated");
        let path = write(
            &dir,
            "domain/Order.kt",
            "@Suppress(\"unused\")\nclass Order(val id: String)\n\n@Suppress(\"unused\")\ninterface Repository {\n    fun save()\n}\n",
        );

        let graph = build(&dir, std::slice::from_ref(&path), &[]).unwrap();

        assert!(
            graph
                .declarations
                .iter()
                .any(|d| d.name == "Order" && d.kind == DeclKind::Class)
        );
        assert!(
            graph
                .declarations
                .iter()
                .any(|d| d.name == "Repository" && d.kind == DeclKind::Interface)
        );
        assert!(
            !graph
                .declarations
                .iter()
                .any(|d| d.name == "Order" && d.kind == DeclKind::Interface)
        );
        assert!(
            !graph
                .declarations
                .iter()
                .any(|d| d.name == "Repository" && d.kind == DeclKind::Class)
        );
        assert_eq!(graph.declarations.len(), 2);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn java_declarations_malformed_source_never_misclassifies() {
        // Reuses Story 4.1.1's malformed-source shape (unclosed class body) applied to
        // a *second* declaration following a well-formed first one, per this story's AC.
        //
        // Verified real behavior (neither of the AC's literal (a)/(b) options): the
        // grammar recovers by inserting a `MISSING "}"` node inside `Broken`'s
        // `class_body`, leaving *both* `class_declaration` nodes' own `kind()` and
        // `name` field fully intact — so both `Order` and `Broken` come back correctly
        // classified as `Class` with their correct names. This still satisfies the AC's
        // actual invariant ("never a Declaration with a wrong kind or a wrong name
        // silently accepted as correct") — `Broken` genuinely is a class named `Broken`,
        // even though its body's closing brace is missing — it's just a third outcome
        // the AC's (a)/(b) framing didn't anticipate.
        let dir = tmp_dir("java-malformed");
        let path = write(
            &dir,
            "com/example/domain/Order.java",
            "public class Order {}\n\npublic class Broken {\n",
        );

        let graph = build(&dir, std::slice::from_ref(&path), &[]).unwrap();

        assert!(
            graph
                .declarations
                .iter()
                .any(|d| d.name == "Order" && d.kind == DeclKind::Class)
        );
        // No wrong kind/name ever accepted: whatever else is present, nothing is
        // misclassified.
        assert!(
            graph
                .declarations
                .iter()
                .all(|d| (d.name == "Order" || d.name == "Broken") && d.kind == DeclKind::Class)
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn kotlin_declarations_malformed_source_never_misclassifies() {
        // Reuses Story 4.1.2's malformed-source shape (unclosed parameter list,
        // `MISSING ")"` recovery) applied to a second declaration after a well-formed
        // first one, per this story's AC.
        //
        // Verified real behavior, same shape as the Java case above: the `MISSING ")"`
        // node nests inside `Broken`'s `class_parameters`, leaving both
        // `class_declaration` nodes' own `kind()`, keyword child, and `name` field
        // intact — both `Order` and `Broken` come back correctly classified as `Class`.
        // No wrong kind or wrong name is ever silently accepted.
        let dir = tmp_dir("kotlin-malformed");
        let path = write(
            &dir,
            "domain/Order.kt",
            "class Order(val id: String)\n\nclass Broken(val id: String\n",
        );

        let graph = build(&dir, std::slice::from_ref(&path), &[]).unwrap();

        assert!(
            graph
                .declarations
                .iter()
                .any(|d| d.name == "Order" && d.kind == DeclKind::Class)
        );
        assert!(
            graph
                .declarations
                .iter()
                .all(|d| (d.name == "Order" || d.name == "Broken") && d.kind == DeclKind::Class)
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- Epic 5.3 / Story 5.3.1: Python declaration extraction ---

    /// Story 5.3.1's AC fixture verbatim: `Order` (class, line 1) and `validate`
    /// (top-level function, line 5) are extracted; the nested `__init__` method is not.
    #[test]
    fn python_declarations_finds_class_and_module_level_function_only() {
        let dir = tmp_dir("python-class-fn");
        let path = write(
            &dir,
            "app/domain/order.py",
            "class Order:\n    def __init__(self, id: str):\n        self.id = id\n\ndef validate(order: Order) -> bool:\n    return bool(order.id)\n",
        );
        let components = vec![Component {
            name: "domain".into(),
            paths: vec!["**/domain".into(), "**/domain/**".into()],
        }];

        let graph = build(&dir, std::slice::from_ref(&path), &components).unwrap();

        assert!(graph.declarations.contains(&Declaration {
            name: "Order".to_string(),
            kind: DeclKind::Class,
            file: path.clone(),
            line: 1,
            component: Some("domain".to_string()),
        }));
        assert!(graph.declarations.contains(&Declaration {
            name: "validate".to_string(),
            kind: DeclKind::Function,
            file: path.clone(),
            line: 5,
            component: Some("domain".to_string()),
        }));
        // The nested __init__ must never appear as its own top-level declaration.
        assert!(!graph.declarations.iter().any(|d| d.name == "__init__"));
        assert_eq!(graph.declarations.len(), 2);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Shape-variant test (this story's Java/Kotlin-annotation precedent, applied to
    /// Python's own wrapping construct): a decorated top-level class/function wraps in
    /// a `decorated_definition` node rather than exposing `class_definition`/
    /// `function_definition` directly as `module`'s child (confirmed via real
    /// `to_sexp()` output — see `collect_python_declarations`'s doc comment). A walk
    /// that only matched on `class_definition`/`function_definition` kind would
    /// silently drop both declarations below; this asserts the unwrap keeps them.
    #[test]
    fn python_declarations_decorator_does_not_hide_top_level_declaration() {
        let dir = tmp_dir("python-decorated");
        let path = write(
            &dir,
            "app/domain/order.py",
            "@dataclass\nclass Order:\n    id: str\n\n@app.route(\"/x\")\ndef handler():\n    pass\n",
        );

        let graph = build(&dir, std::slice::from_ref(&path), &[]).unwrap();

        assert!(
            graph
                .declarations
                .iter()
                .any(|d| d.name == "Order" && d.kind == DeclKind::Class && d.line == 2)
        );
        assert!(
            graph
                .declarations
                .iter()
                .any(|d| d.name == "handler" && d.kind == DeclKind::Function && d.line == 6)
        );
        assert_eq!(graph.declarations.len(), 2);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn python_declarations_malformed_source_never_misclassifies() {
        // Reuses Story 4.1.1/4.1.2's malformed-source shape (unclosed parameter list)
        // applied to a second declaration following a well-formed first one.
        //
        // Verified real behavior (this file's `zzz_probe_python_shapes`-style
        // verification, run directly against `to_sexp()`): unlike Java/Kotlin's
        // `MISSING` recovery, tree-sitter-python's error recovery here does *not* keep
        // the broken declaration's outer node intact — `def broken(` with no closing
        // paren produces `(module (class_definition ...) (ERROR (identifier)))`, i.e.
        // `Broken`'s would-be `function_definition` is swallowed entirely into a bare
        // `ERROR` node at `module` level, not a `function_definition` with an internal
        // `MISSING` token. Since this walk only matches `class_definition`/
        // `function_definition`/`decorated_definition` by kind, the `ERROR` node is
        // simply skipped — `broken` is never extracted at all, rather than extracted
        // under the wrong kind or wrong name. This still satisfies the invariant this
        // story's malformed-source tests all check: no Declaration with a wrong kind or
        // wrong name is ever silently accepted as correct; the well-formed declaration
        // before the malformed one is unaffected.
        let dir = tmp_dir("python-malformed");
        let path = write(
            &dir,
            "app/order.py",
            "class Order:\n    pass\n\ndef broken(\n",
        );

        let graph = build(&dir, std::slice::from_ref(&path), &[]).unwrap();

        assert!(
            graph
                .declarations
                .iter()
                .any(|d| d.name == "Order" && d.kind == DeclKind::Class)
        );
        // No wrong kind/name ever accepted: whatever else is present, nothing is
        // misclassified.
        assert!(
            graph
                .declarations
                .iter()
                .all(|d| d.name == "Order" && d.kind == DeclKind::Class)
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
