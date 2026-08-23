//! The third whole-repo parsing pass (per ADR-001), alongside `checker`'s per-file AST
//! checks and `import_graph`'s package/module dependency graph: a flat enumeration of
//! every top-level declaration (struct/class/interface/enum/function) across the repo,
//! feeding `declaration_checks`'s `ContentChecker` (Story 2.2.1) and `NamingChecker`
//! (Phase 3). Go and JS/TS first, mirroring `import_graph.rs`'s own initial-language
//! scope before Python/Java/Kotlin were added in later phases.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tree_sitter::Node;

use crate::config::{Component, component_of};

/// The shape of one enumerated declaration. `Enum` has no producer yet in Go/JS/TS
/// (Go has no enum node kind; JS/TS `enum` support can follow later) — kept here now so
/// `ContentRule`/`NamingRule`'s `allowed_kinds`/`kind` string parsing (Story 2.2.1) has a
/// stable variant set to validate against from the start, matching plan.md Task 2.1.1a's
/// literal enum definition.
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
/// scoped to). Only Go and TypeScript/JavaScript are extracted for now — Python/Java/
/// Kotlin can follow the same per-language dispatch pattern later (Phase 4/5).
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

    Ok(graph)
}

fn has_ext(path: &Path, ext: &str) -> bool {
    path.extension().and_then(|e| e.to_str()) == Some(ext)
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
}
