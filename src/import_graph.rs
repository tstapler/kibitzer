use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tree_sitter::Node;

/// A directed edge from one package/module directory to another, plus the specific
/// import statement (file + line) that produced it — kept so findings derived from the
/// graph can still point at a concrete location, per the `{file}:{line}: {message}`
/// convention every other checker follows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportEdge {
    pub from: String,
    pub to: String,
    pub file: PathBuf,
    pub line: usize,
}

/// A repo's local import graph, at directory (package/module) granularity rather than
/// per-file — too dense at file granularity to reason about cycles or layering.
#[derive(Debug, Default, Clone)]
pub struct ImportGraph {
    pub nodes: BTreeSet<String>,
    pub edges: Vec<ImportEdge>,
}

impl ImportGraph {
    pub fn edges_from<'a>(&'a self, node: &'a str) -> impl Iterator<Item = &'a ImportEdge> {
        self.edges.iter().filter(move |e| e.from == node)
    }
}

/// Build the import graph over `files` (already filtered to files kibitzer is scoped
/// to). Only Go and TypeScript/JavaScript are extracted for now — Python/Kotlin/Java
/// import extraction can follow the same per-language dispatch pattern later.
pub fn build(repo_root: &Path, files: &[PathBuf]) -> Result<ImportGraph> {
    let mut graph = ImportGraph::default();

    let go_files: Vec<&PathBuf> = files.iter().filter(|f| has_ext(f, "go")).collect();
    if !go_files.is_empty() {
        build_go(repo_root, &go_files, &mut graph)?;
    }

    let js_files: Vec<&PathBuf> = files.iter().filter(|f| is_js_like(f)).collect();
    if !js_files.is_empty() {
        build_js(&js_files, &mut graph)?;
    }

    Ok(graph)
}

fn has_ext(path: &Path, ext: &str) -> bool {
    path.extension().and_then(|e| e.to_str()) == Some(ext)
}

pub(crate) fn is_js_like(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("ts") | Some("tsx") | Some("js") | Some("jsx") | Some("mjs") | Some("cjs")
    )
}

// ---------------------------------------------------------------------------------
// Go
// ---------------------------------------------------------------------------------

fn go_module_path(repo_root: &Path) -> Option<String> {
    let contents = std::fs::read_to_string(repo_root.join("go.mod")).ok()?;
    contents.lines().find_map(|line| {
        line.trim()
            .strip_prefix("module ")
            .map(|rest| rest.trim().to_string())
    })
}

fn go_package_import_path(module_path: &str, repo_root: &Path, file: &Path) -> Option<String> {
    let dir = file.parent()?;
    let rel = dir.strip_prefix(repo_root).unwrap_or(dir);
    let rel_str = rel.to_string_lossy().replace('\\', "/");
    if rel_str.is_empty() {
        Some(module_path.to_string())
    } else {
        Some(format!("{module_path}/{rel_str}"))
    }
}

fn collect_go_imports(node: Node, src: &[u8], out: &mut Vec<(String, usize)>) {
    if node.kind() == "import_spec"
        && let Some(path_node) = node.child_by_field_name("path")
        && let Ok(text) = path_node.utf8_text(src)
    {
        out.push((
            text.trim_matches('"').to_string(),
            path_node.start_position().row + 1,
        ));
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_go_imports(child, src, out);
    }
}

fn build_go(repo_root: &Path, files: &[&PathBuf], graph: &mut ImportGraph) -> Result<()> {
    let Some(module_path) = go_module_path(repo_root) else {
        // Not inside a resolvable Go module (no go.mod, or no `module` directive) —
        // there's nothing to map import paths back to local packages against.
        return Ok(());
    };

    let mut file_packages: Vec<(&PathBuf, String)> = Vec::new();
    for file in files {
        if let Some(pkg) = go_package_import_path(&module_path, repo_root, file) {
            graph.nodes.insert(pkg.clone());
            file_packages.push((file, pkg));
        }
    }

    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_go::LANGUAGE.into())
        .context("loading tree-sitter-go grammar")?;

    for (file, pkg) in &file_packages {
        let source = std::fs::read_to_string(file)
            .with_context(|| format!("reading {}", file.display()))?;
        let tree = parser
            .parse(&source, None)
            .with_context(|| format!("parsing {} with tree-sitter-go", file.display()))?;

        let mut imports = Vec::new();
        collect_go_imports(tree.root_node(), source.as_bytes(), &mut imports);

        for (import_path, line) in imports {
            if &import_path != pkg && graph.nodes.contains(&import_path) {
                graph.edges.push(ImportEdge {
                    from: pkg.clone(),
                    to: import_path,
                    file: (*file).clone(),
                    line,
                });
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------------
// TypeScript / JavaScript
// ---------------------------------------------------------------------------------

fn js_module_dir(file: &Path) -> PathBuf {
    file.parent().unwrap_or(Path::new(".")).to_path_buf()
}

pub(crate) fn js_ts_language(file: &Path) -> tree_sitter::Language {
    match file.extension().and_then(|e| e.to_str()) {
        Some("tsx") => tree_sitter_typescript::LANGUAGE_TSX.into(),
        Some("ts") => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        _ => tree_sitter_javascript::LANGUAGE.into(),
    }
}

/// `import_statement` and `export ... from ...` re-exports both carry a `source:`
/// field pointing to a `string` node whose actual path text lives in a nested
/// `string_fragment` child — the surrounding quote characters are separate leaves.
fn collect_js_imports(node: Node, src: &[u8], out: &mut Vec<(String, usize)>) {
    if (node.kind() == "import_statement" || node.kind() == "export_statement")
        && let Some(source_node) = node.child_by_field_name("source")
        && let Some(text) = string_fragment_text(source_node, src)
    {
        out.push((text, node.start_position().row + 1));
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_js_imports(child, src, out);
    }
}

fn string_fragment_text(string_node: Node, src: &[u8]) -> Option<String> {
    let mut cursor = string_node.walk();
    for child in string_node.children(&mut cursor) {
        if child.kind() == "string_fragment" {
            return child.utf8_text(src).ok().map(str::to_string);
        }
    }
    None
}

/// Resolves a relative import spec to one of the original (non-canonicalized) file
/// paths in `files` — matching happens on the canonical form (to correctly follow `..`
/// and any symlinks on the way, e.g. macOS's `/tmp` → `/private/tmp`), but the returned
/// path is the original one, so its directory maps back to the same graph node key
/// `files`' own entries were inserted under.
fn resolve_relative_import(
    from_file: &Path,
    spec: &str,
    known: &std::collections::HashMap<PathBuf, PathBuf>,
) -> Option<PathBuf> {
    let base = from_file.parent()?.join(spec);
    let mut candidates = vec![base.clone()];
    for ext in ["ts", "tsx", "js", "jsx", "mjs", "cjs"] {
        candidates.push(base.with_extension(ext));
    }
    for ext in ["ts", "tsx", "js", "jsx", "mjs", "cjs"] {
        candidates.push(base.join(format!("index.{ext}")));
    }
    candidates
        .into_iter()
        .find_map(|c| c.canonicalize().ok().and_then(|canon| known.get(&canon).cloned()))
}

fn build_js(files: &[&PathBuf], graph: &mut ImportGraph) -> Result<()> {
    let known_files: std::collections::HashMap<PathBuf, PathBuf> = files
        .iter()
        .filter_map(|f| f.canonicalize().ok().map(|canon| (canon, (*f).clone())))
        .collect();

    for file in files {
        graph.nodes.insert(dir_key(&js_module_dir(file)));
    }

    for file in files {
        let source = std::fs::read_to_string(file)
            .with_context(|| format!("reading {}", file.display()))?;

        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&js_ts_language(file))
            .with_context(|| format!("loading tree-sitter grammar for {}", file.display()))?;
        let tree = parser
            .parse(&source, None)
            .with_context(|| format!("parsing {}", file.display()))?;

        let mut imports = Vec::new();
        collect_js_imports(tree.root_node(), source.as_bytes(), &mut imports);

        let from_dir = dir_key(&js_module_dir(file));
        for (spec, line) in imports {
            if !(spec.starts_with("./") || spec.starts_with("../")) {
                continue; // bare/package specifiers aren't local — nothing to resolve
            }
            let Some(resolved) = resolve_relative_import(file, &spec, &known_files) else {
                continue;
            };
            let to_dir = dir_key(&js_module_dir(&resolved));
            if to_dir != from_dir {
                graph.edges.push(ImportEdge {
                    from: from_dir.clone(),
                    to: to_dir,
                    file: (*file).clone(),
                    line,
                });
            }
        }
    }

    Ok(())
}

fn dir_key(dir: &Path) -> String {
    let s = dir.to_string_lossy().replace('\\', "/");
    if s.is_empty() {
        ".".to_string()
    } else {
        s
    }
}

// ---------------------------------------------------------------------------------
// Java / Kotlin import node-kind verification (Epic 4.1) — real `to_sexp()` output
// pinned ahead of the extraction code (Story 4.2.2/4.2.3), per this codebase's
// verification discipline (`docs/syntax-rules.md`, `rules.rs`'s `LangRuleConfig` doc
// comment). No `build_java`/`build_kotlin` exist yet; the findings below are what
// Epic 4.2's implementer builds against.
//
// **Java** (`tree-sitter-java` 0.23.5, matches `Cargo.lock`): every import form —
// plain, wildcard, and `import static` — parses to the *same* node kind,
// `import_declaration`, with **positional** (unfielded) children:
// - Plain: `(import_declaration (scoped_identifier ...))` — one positional child, the
//   dotted path.
// - Wildcard (`import com.example.infra.*;`): `(import_declaration (scoped_identifier
//   ...) (asterisk))` — a second positional child, a **named** `asterisk` node. Unlike
//   Kotlin below, Java's wildcard marker is visible in `to_sexp()`/named-child
//   inspection directly.
// - `import static com.example.infra.Constants.MAX;`: **not** distinguishable from a
//   plain import by node kind or by any named child — `to_sexp()` is
//   `(import_declaration (scoped_identifier ...))`, identical in shape to a plain
//   import of the same path depth. The `static` keyword *is* present as an
//   **anonymous** (unnamed) token child at raw position 1 (`import`, `static`,
//   `scoped_identifier`, `;`) — confirmed via `Node::child(1)` (not
//   `named_child`/`to_sexp()`), so distinguishing `import static` requires a raw
//   (unnamed-included) child walk or a source-text check for `import static `, not a
//   `to_sexp()`-based check. This confirms pitfalls.md's flag that it "must be
//   confirmed via live `to_sexp()`, not assumed" — it is real and it is *not*
//   detectable from `to_sexp()` output alone.
// - Malformed source (missing `;` after a field, unclosed class body — see
//   `java_malformed_source_recovery_shape` below): tree-sitter-java recovers with a
//   single flat `ERROR` node wrapping the malformed declaration's tokens
//   (`(ERROR (modifiers) (identifier) (modifiers) (type_identifier) (identifier))`),
//   **not** a `MISSING` node — any leading valid syntax (the import line here) still
//   parses cleanly outside the `ERROR` node.
//
// **Kotlin** (`tree-sitter-kotlin-ng` 1.1.0, matches `Cargo.lock`): every import form
// is the single node kind `import` (not `import_header`/`import_list` — that's the
// unrelated `fwcd/tree-sitter-kotlin` fork), with **positional** (unfielded) children,
// same no-named-fields situation `syntax-rules.md` already documents for Kotlin's
// function nodes:
// - Plain: `(import (qualified_identifier ...))` — one positional child.
// - Wildcard (`import com.example.infra.*`): **also** `(import (qualified_identifier
//   ...))` with the *same* named-child shape as a plain import one path segment
//   shorter (`com.example.infra.*` and a plain `import com.example.infra` both dump
//   to the identical `(import (qualified_identifier (identifier) (identifier)
//   (identifier)))`, verified by direct comparison) — the trailing `.*` is two
//   **anonymous** tokens (`.` then `*`), invisible to `to_sexp()`/named-child
//   inspection. Unlike Java's named `asterisk` node, Kotlin's wildcard marker is
//   genuinely undetectable from `to_sexp()` alone; the real disambiguation is a raw
//   (unnamed-included) child walk checking whether the `import` node's *last* raw
//   child has kind `"*"` (verified: the wildcard import's raw children are `import`,
//   `qualified_identifier`, `.`, `*` — 4 total vs. the plain import's 2), or
//   equivalently a source-text check for a trailing `.*`. This is exactly the node
//   shape build-vs-buy.md flagged as unconfirmed from static schema data, and it is a
//   **real trap for Epic 4.2**: a named-child-only or `to_sexp()`-substring-only
//   extraction will silently mis-parse a wildcard import as a plain one.
// - Aliased (`import com.example.infra.Legacy as LegacyClient`): `(import
//   (qualified_identifier ...) (identifier))` — **two** positional children: the
//   `qualified_identifier` (the full path *including* the aliased name, e.g.
//   `com.example.infra.Legacy`) and a second positional `identifier` for the alias
//   itself (`LegacyClient`), with the `as` keyword an anonymous token between them.
// - Malformed source (`import com.example.infra.DbClient\n\nclass Order(val id:
//   String\n` — unclosed parameter list): tree-sitter-kotlin-ng recovers with an
//   explicit `MISSING ")"` node **inserted in place** inside the otherwise-intact
//   `class_declaration`/`primary_constructor`/`class_parameters` tree — a
//   structurally different recovery strategy from Java's flat `ERROR`-node wrapper
//   above; both are real, verified outcomes of the "parser.parse() essentially never
//   returns None" behavior adversarial-review.md flagged, but the *shape* of recovery
//   differs per grammar and must not be assumed to generalize from one language to the
//   other.
//
// See `declarations.rs`'s Kotlin section (Task 4.1.2c) for the `class_declaration`
// vs. `object_declaration` verification (Kotlin's `interface` shares `class_declaration`
// with `class`, distinguished only by an anonymous keyword child; `object` is a wholly
// separate node kind).

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn tmp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "kibitzer-import-graph-test-{}-{name}-{}",
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

    #[test]
    fn go_import_graph_finds_a_two_package_cycle() {
        let dir = tmp_dir("go-cycle");
        write(&dir, "go.mod", "module example.com/app\n\ngo 1.21\n");
        let a = write(
            &dir,
            "a/a.go",
            "package a\n\nimport \"example.com/app/b\"\n\nfunc F() { b.G() }\n",
        );
        let b = write(
            &dir,
            "b/b.go",
            "package b\n\nimport \"example.com/app/a\"\n\nfunc G() { a.F() }\n",
        );

        let graph = build(&dir, &[a, b]).unwrap();

        assert!(graph.nodes.contains("example.com/app/a"));
        assert!(graph.nodes.contains("example.com/app/b"));
        assert!(graph
            .edges
            .iter()
            .any(|e| e.from == "example.com/app/a" && e.to == "example.com/app/b"));
        assert!(graph
            .edges
            .iter()
            .any(|e| e.from == "example.com/app/b" && e.to == "example.com/app/a"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn go_import_of_stdlib_package_is_ignored() {
        let dir = tmp_dir("go-stdlib");
        write(&dir, "go.mod", "module example.com/app\n\ngo 1.21\n");
        let a = write(&dir, "a/a.go", "package a\n\nimport \"fmt\"\n\nfunc F() { fmt.Println() }\n");

        let graph = build(&dir, &[a]).unwrap();

        assert!(graph.edges.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ts_import_graph_finds_a_two_module_cycle() {
        let dir = tmp_dir("ts-cycle");
        let a = write(&dir, "a/index.ts", "import { g } from '../b/index';\nexport function f() {}\n");
        let b = write(&dir, "b/index.ts", "import { f } from '../a/index';\nexport function g() {}\n");

        let graph = build(&dir, &[a, b]).unwrap();

        let a_dir = dir_key(&dir.join("a"));
        let b_dir = dir_key(&dir.join("b"));
        assert!(graph.edges.iter().any(|e| e.from == a_dir && e.to == b_dir));
        assert!(graph.edges.iter().any(|e| e.from == b_dir && e.to == a_dir));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ts_import_of_bare_package_specifier_is_ignored() {
        let dir = tmp_dir("ts-bare");
        let a = write(&dir, "a/index.ts", "import { z } from 'zod';\n");

        let graph = build(&dir, &[a]).unwrap();

        assert!(graph.edges.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- Epic 4.1 / Story 4.1.1: Java to_sexp() verification ---

    fn parse_java(src: &str) -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_java::LANGUAGE.into())
            .expect("loading tree-sitter-java grammar");
        parser.parse(src, None).expect("parsing Java fixture")
    }

    /// Task 4.1.1a — the exact fixture from plan.md's Story 4.1.1 acceptance criteria:
    /// a plain qualified import, a wildcard import, and an `import static`, all real
    /// `to_sexp()` output pinned per the doc comment above `dir_key`.
    #[test]
    fn java_import_to_sexp_fixture() {
        let src = "package com.example.domain;\n\nimport com.example.infra.DbClient;\nimport com.example.infra.*;\nimport static com.example.infra.Constants.MAX;\n\npublic class Order {\n    public String id;\n}\n";
        let tree = parse_java(src);
        let sexp = tree.root_node().to_sexp();

        assert!(!tree.root_node().has_error());
        // Qualified import (also matches the `import static` form, which shares the
        // exact same shape — see Task 4.1.1b below).
        assert!(sexp.contains("(import_declaration (scoped_identifier"));
        // Wildcard import carries a second positional, *named* `asterisk` child.
        assert!(sexp.contains(
            "(import_declaration (scoped_identifier scope: (scoped_identifier scope: (identifier) name: (identifier)) name: (identifier)) (asterisk))"
        ));
    }

    /// Task 4.1.1b — `import static` is confirmed **not** distinguishable from a plain
    /// import by node kind, `to_sexp()`, or any named child: both produce
    /// `(import_declaration (scoped_identifier ...))`. The `static` keyword is present
    /// only as an anonymous (unnamed) token at raw child position 1 — visible via
    /// `Node::child(1)`, invisible to `named_child`/`to_sexp()`.
    #[test]
    fn java_import_static_is_not_distinguishable_via_to_sexp() {
        let static_tree = parse_java("import static com.example.infra.Constants.MAX;\n");
        let plain_tree = parse_java("import com.example.infra.DbClient;\n");

        let static_import = static_tree.root_node().named_child(0).unwrap();
        let plain_import = plain_tree.root_node().named_child(0).unwrap();

        assert_eq!(static_import.kind(), "import_declaration");
        assert_eq!(plain_import.kind(), "import_declaration");
        // Both have exactly one *named* child (the scoped_identifier) — `static`
        // contributes nothing to the named-child shape.
        assert_eq!(static_import.named_child_count(), 1);
        assert_eq!(plain_import.named_child_count(), 1);

        // The real disambiguation: raw (unnamed-included) child 1.
        assert_eq!(static_import.child(1).unwrap().kind(), "static");
        assert_ne!(plain_import.child(1).unwrap().kind(), "static");
    }

    /// Task 4.1.1c — malformed/truncated Java source (missing `;`, unclosed class
    /// body) recovers as a single flat `ERROR` node wrapping the malformed
    /// declaration; the syntactically valid leading `import` still parses cleanly
    /// outside it. No `MISSING` node appears for this fragment — contrast with
    /// Kotlin's `MISSING` recovery below.
    #[test]
    fn java_malformed_source_recovery_shape() {
        let src =
            "import com.example.infra.DbClient;\n\npublic class Order {\n    public String id\n";
        let tree = parse_java(src);

        assert!(tree.root_node().has_error());
        let sexp = tree.root_node().to_sexp();
        assert!(sexp.contains(
            "(ERROR (modifiers) (identifier) (modifiers) (type_identifier) (identifier))"
        ));
        // The valid import ahead of the malformed fragment still parses as a clean,
        // non-error import_declaration.
        assert!(sexp.starts_with("(program (import_declaration (scoped_identifier"));
    }

    // --- Epic 4.1 / Story 4.1.2: Kotlin to_sexp() verification (highest-risk) ---

    fn parse_kotlin(src: &str) -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_kotlin_ng::LANGUAGE.into())
            .expect("loading tree-sitter-kotlin-ng grammar");
        parser.parse(src, None).expect("parsing Kotlin fixture")
    }

    /// Task 4.1.2a — the exact fixture from plan.md's Story 4.1.2 acceptance criteria:
    /// a plain import, a wildcard import, and an aliased import, all real `to_sexp()`
    /// output pinned per the doc comment above `dir_key`.
    #[test]
    fn kotlin_import_to_sexp_fixture() {
        let src = "package com.example.domain\n\nimport com.example.infra.DbClient\nimport com.example.infra.*\nimport com.example.infra.Legacy as LegacyClient\n\nclass Order(val id: String)\n";
        let tree = parse_kotlin(src);
        let sexp = tree.root_node().to_sexp();

        assert!(!tree.root_node().has_error());
        // Plain import: one positional `qualified_identifier` child, 4 segments.
        assert!(sexp.contains(
            "(import (qualified_identifier (identifier) (identifier) (identifier) (identifier)))"
        ));
        // Wildcard import: 3-segment `qualified_identifier` — the trailing `.*` is
        // anonymous tokens, invisible here (see Task 4.1.2b test below for the real
        // disambiguation).
        assert!(
            sexp.contains("(import (qualified_identifier (identifier) (identifier) (identifier)))")
        );
        // Aliased import: `qualified_identifier` + a second positional `identifier`
        // for the alias name.
        assert!(sexp.contains(
            "(import (qualified_identifier (identifier) (identifier) (identifier) (identifier)) (identifier))"
        ));
    }

    /// Task 4.1.2b — the real, verified aliased-import shape: `import a.b.C as D`
    /// produces `(import (qualified_identifier ...) (identifier))`, where the
    /// `qualified_identifier` includes the pre-alias name (`C`) and the second
    /// positional `identifier` is the alias (`D`), with the `as` keyword an anonymous
    /// token between them (not a named field).
    #[test]
    fn kotlin_aliased_import_has_two_positional_children() {
        let tree = parse_kotlin("import com.example.infra.Legacy as LegacyClient\n");
        let import_node = tree.root_node().named_child(0).unwrap();

        assert_eq!(import_node.kind(), "import");
        assert_eq!(import_node.named_child_count(), 2);
        assert_eq!(
            import_node.named_child(0).unwrap().kind(),
            "qualified_identifier"
        );
        assert_eq!(import_node.named_child(1).unwrap().kind(), "identifier");
        assert_eq!(
            import_node
                .named_child(1)
                .unwrap()
                .utf8_text("import com.example.infra.Legacy as LegacyClient\n".as_bytes())
                .unwrap(),
            "LegacyClient"
        );
    }

    /// Task 4.1.2b (continued) — the real trap flagged in the doc comment above
    /// `dir_key`: a Kotlin wildcard import (`import a.b.*`) and a plain import one
    /// segment shorter (`import a.b`) produce **byte-identical** `to_sexp()` output —
    /// the wildcard `.*` is two anonymous tokens with no named-node representation.
    /// The only reliable disambiguation is a raw (unnamed-included) child inspection:
    /// the wildcard import's *last raw child* has kind `"*"`; the plain import has no
    /// such trailing child.
    #[test]
    fn kotlin_wildcard_import_is_indistinguishable_from_shorter_plain_import_via_to_sexp() {
        let wildcard_tree = parse_kotlin("import com.example.infra.*\n");
        let plain_tree = parse_kotlin("import com.example.infra\n");

        let wildcard_import = wildcard_tree.root_node().named_child(0).unwrap();
        let plain_import = plain_tree.root_node().named_child(0).unwrap();

        assert_eq!(wildcard_import.to_sexp(), plain_import.to_sexp());
        assert_eq!(wildcard_import.named_child_count(), 1);
        assert_eq!(plain_import.named_child_count(), 1);

        // Raw (unnamed-included) child counts differ: wildcard has `import`,
        // `qualified_identifier`, `.`, `*` (4); plain has just `import`,
        // `qualified_identifier` (2).
        assert_eq!(wildcard_import.child_count(), 4);
        assert_eq!(plain_import.child_count(), 2);
        assert_eq!(
            wildcard_import
                .child(wildcard_import.child_count() as u32 - 1)
                .unwrap()
                .kind(),
            "*"
        );
        assert_ne!(
            plain_import
                .child(plain_import.child_count() as u32 - 1)
                .unwrap()
                .kind(),
            "*"
        );
    }

    /// Task 4.1.2d — malformed/truncated Kotlin source (unclosed parameter list)
    /// recovers with an explicit `MISSING ")"` node **inserted in place** inside the
    /// otherwise-intact declaration tree — a different recovery strategy from Java's
    /// flat `ERROR`-node wrapper (`java_malformed_source_recovery_shape` above); the
    /// leading valid `import` still parses cleanly.
    #[test]
    fn kotlin_malformed_source_recovery_shape() {
        let src = "import com.example.infra.DbClient\n\nclass Order(val id: String\n";
        let tree = parse_kotlin(src);

        assert!(tree.root_node().has_error());
        let sexp = tree.root_node().to_sexp();
        assert!(sexp.contains("(MISSING \")\")"));
        assert!(sexp.starts_with("(source_file (import (qualified_identifier"));
    }
}
