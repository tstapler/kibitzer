//! The third whole-repo parsing pass (per ADR-001), alongside `checker`'s per-file AST
//! checks and `import_graph`'s package/module dependency graph: a flat enumeration of
//! every top-level declaration (struct/class/interface/enum/function) across the repo,
//! feeding `declaration_checks`'s `ContentChecker` (Story 2.2.1) and `NamingChecker`
//! (Phase 3). Go and JS/TS first, mirroring `import_graph.rs`'s own initial-language
//! scope before Python/Java/Kotlin were added in later phases (Java/Kotlin land in
//! Epic 4.3 below).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tree_sitter::Node;

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
/// scoped to). Go, TypeScript/JavaScript, Java, and Kotlin are extracted; Python can
/// follow the same per-language dispatch pattern later (Phase 5).
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

    let go_files: Vec<&PathBuf> = files.iter().filter(|f| has_ext(f, "go")).collect();
    if !go_files.is_empty() {
        build_go_declarations(repo_root, &go_files, components, &mut graph)?;
    }

    let js_files: Vec<&PathBuf> = files
        .iter()
        .filter(|f| crate::import_graph::is_js_like(f))
        .collect();
    if !js_files.is_empty() {
        build_js_ts_declarations(repo_root, &js_files, components, &mut graph)?;
    }

    let java_files: Vec<&PathBuf> = files.iter().filter(|f| has_ext(f, "java")).collect();
    if !java_files.is_empty() {
        build_java_declarations(repo_root, &java_files, components, &mut graph)?;
    }

    let kotlin_files: Vec<&PathBuf> = files.iter().filter(|f| is_kotlin_like(f)).collect();
    if !kotlin_files.is_empty() {
        build_kotlin_declarations(repo_root, &kotlin_files, components, &mut graph)?;
    }

    Ok(graph)
}

fn has_ext(path: &Path, ext: &str) -> bool {
    path.extension().and_then(|e| e.to_str()) == Some(ext)
}

fn is_kotlin_like(path: &Path) -> bool {
    has_ext(path, "kt") || has_ext(path, "kts")
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
}
