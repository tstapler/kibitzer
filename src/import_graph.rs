use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tree_sitter::Node;

/// A directed edge from one package/module directory to another, plus the specific
/// import statement (file + line) that produced it — kept so findings derived from the
/// graph can still point at a concrete location, per the `{file}:{line}: {message}`
/// convention every other checker follows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Every file this graph extracted imports for, mapped to the package/node key it
    /// was grouped under — the single source of truth for a file's package key, per
    /// language (Go: module-qualified import path; JS/TS: repo-relative directory).
    /// `arch_model::build_model` consults this so its own package grouping always
    /// matches this graph's node keys instead of re-deriving (and potentially
    /// mismatching) them.
    pub file_packages: BTreeMap<PathBuf, String>,
}

impl ImportGraph {
    pub fn edges_from<'a>(&'a self, node: &'a str) -> impl Iterator<Item = &'a ImportEdge> {
        self.edges.iter().filter(move |e| e.from == node)
    }
}

/// Build the import graph over `files` (already filtered to files kibitzer is scoped
/// to). Go, TypeScript/JavaScript, Python, Java, and Kotlin are extracted.
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

    let py_files: Vec<&PathBuf> = files.iter().filter(|f| has_ext(f, "py")).collect();
    if !py_files.is_empty() {
        build_python(&py_files, &mut graph)?;
    }

    let java_files: Vec<&PathBuf> = files.iter().filter(|f| has_ext(f, "java")).collect();
    if !java_files.is_empty() {
        build_java(&java_files, &mut graph)?;
    }

    let kotlin_files: Vec<&PathBuf> = files
        .iter()
        .filter(|f| has_ext(f, "kt") || has_ext(f, "kts"))
        .collect();
    if !kotlin_files.is_empty() {
        build_kotlin(&kotlin_files, &mut graph)?;
    }

    Ok(graph)
}

fn has_ext(path: &Path, ext: &str) -> bool {
    path.extension().and_then(|e| e.to_str()) == Some(ext)
}

fn is_js_like(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("ts") | Some("tsx") | Some("js") | Some("jsx") | Some("mjs") | Some("cjs")
    )
}

/// Returns `node`'s first direct (positional, not field-based) child of the given kind.
/// Duplicated from `symbol_extract.rs`'s identical helper — the two modules don't share
/// a utility module, and this is a small, self-contained lookup (same "duplicate small
/// helpers rather than force a shared-utility extraction for two call sites" precedent
/// Epic 5.3's plan text applies to `rules.rs`'s `kotlin_body`/`kotlin_params`).
fn find_child_by_kind<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    node.children(&mut cursor).find(|c| c.kind() == kind)
}

/// Depth-first search for the first descendant (including `node` itself) of the given
/// kind. Used to locate a file's single `package_declaration`/`package_header` node
/// without assuming it's always the first top-level child (defensive, though it always
/// is in practice for both grammars).
fn find_first_descendant_by_kind<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    if node.kind() == kind {
        return Some(node);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(found) = find_first_descendant_by_kind(child, kind) {
            return Some(found);
        }
    }
    None
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
            graph.file_packages.insert((*file).clone(), pkg.clone());
            file_packages.push((file, pkg));
        }
    }

    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_go::LANGUAGE.into())
        .context("loading tree-sitter-go grammar")?;

    for (file, pkg) in &file_packages {
        let source =
            std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
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

fn js_ts_language(file: &Path) -> tree_sitter::Language {
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
    candidates.into_iter().find_map(|c| {
        c.canonicalize()
            .ok()
            .and_then(|canon| known.get(&canon).cloned())
    })
}

fn build_js(files: &[&PathBuf], graph: &mut ImportGraph) -> Result<()> {
    let known_files: std::collections::HashMap<PathBuf, PathBuf> = files
        .iter()
        .filter_map(|f| f.canonicalize().ok().map(|canon| (canon, (*f).clone())))
        .collect();

    for file in files {
        let key = dir_key(&js_module_dir(file));
        graph.nodes.insert(key.clone());
        graph.file_packages.insert((*file).clone(), key);
    }

    for file in files {
        let source =
            std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;

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
    if s.is_empty() { ".".to_string() } else { s }
}

// ---------------------------------------------------------------------------------
// Python
// ---------------------------------------------------------------------------------

/// Python packages are directories, like JS/TS (not module-qualified like Go) — package
/// key is the file's own directory, same shape as `js_module_dir`/`dir_key`.
fn python_module_dir(file: &Path) -> PathBuf {
    file.parent().unwrap_or(Path::new(".")).to_path_buf()
}

/// Counts the leading `.`/`..` depth of a relative import's `import_prefix` node — one
/// child per dot (`import_prefix`'s children are unnamed `.` leaves, verified via
/// `to_sexp()`: `from ..other import bar` produces `(import_prefix (. .))`, two `.`
/// children, not one node whose text is `".."`).
fn python_relative_dot_count(import_prefix: Node) -> usize {
    let mut cursor = import_prefix.walk();
    import_prefix
        .children(&mut cursor)
        .filter(|c| c.kind() == ".")
        .count()
}

/// A single leading dot means "this package" (`from .sibling import foo` inside
/// `pkg/a.py` refers to `pkg.sibling`, i.e. stays in `pkg`); each additional dot goes up
/// one more directory level. So `dot_count` dots resolve to `dot_count - 1` levels above
/// `from_dir`. The dotted module name after the dots (e.g. `other` in `from ..other
/// import bar`) is deliberately *not* appended to the resolved path — package-level
/// granularity (matching `build_js`'s directory-level, not file-level, edges) only cares
/// which directory the import reaches, not which name inside it.
fn python_relative_target_dir(from_dir: &Path, dot_count: usize) -> Option<PathBuf> {
    let mut dir = from_dir.to_path_buf();
    for _ in 0..dot_count.saturating_sub(1) {
        dir = dir.parent()?.to_path_buf();
    }
    Some(dir)
}

/// Walks `import_from_statement` nodes, keeping only ones whose `module_name` field is a
/// `relative_import` (leading-dot form) — plain `import os` (`import_statement`) and
/// absolute `from os.path import join` (`module_name` is a bare `dotted_name`, no
/// `relative_import` wrapper) are skipped entirely, matching Go's stdlib-skip and JS's
/// bare-specifier-skip precedents: only imports whose target is unambiguously local (a
/// relative import can only mean "somewhere in this repo") produce edges.
fn collect_python_relative_imports(node: Node, out: &mut Vec<(usize, usize)>) {
    if node.kind() == "import_from_statement"
        && let Some(module_name) = node.child_by_field_name("module_name")
        && module_name.kind() == "relative_import"
        && let Some(prefix) = find_child_by_kind(module_name, "import_prefix")
    {
        out.push((
            python_relative_dot_count(prefix),
            node.start_position().row + 1,
        ));
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_python_relative_imports(child, out);
    }
}

fn build_python(files: &[&PathBuf], graph: &mut ImportGraph) -> Result<()> {
    for file in files {
        let key = dir_key(&python_module_dir(file));
        graph.nodes.insert(key.clone());
        graph.file_packages.insert((*file).clone(), key);
    }

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

        let mut imports = Vec::new();
        collect_python_relative_imports(tree.root_node(), &mut imports);

        let from_dir = python_module_dir(file);
        let from_key = dir_key(&from_dir);
        for (dot_count, line) in imports {
            let Some(target_dir) = python_relative_target_dir(&from_dir, dot_count) else {
                continue;
            };
            let to_key = dir_key(&target_dir);
            if to_key != from_key && graph.nodes.contains(&to_key) {
                graph.edges.push(ImportEdge {
                    from: from_key.clone(),
                    to: to_key,
                    file: (*file).clone(),
                    line,
                });
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------------
// Java / Kotlin — both key packages off a `package`/`import` declaration rather than
// directory layout (unlike Go/JS), so they share the "dotted-path text, strip the last
// segment off an import to get its package" shape. Kept as two small per-language
// functions (not one shared implementation) since the node kinds genuinely differ
// (`package_declaration`/`import_declaration` vs `package_header`/`import`, per
// `to_sexp()`) and forcing a shared abstraction over two call sites isn't worth it.
// ---------------------------------------------------------------------------------

/// Extracts the raw dotted-path text from a `package_declaration`/`import_declaration`
/// (Java) or `package_header`/`import` (Kotlin) node — these wrap a positional (not
/// field-based) `scoped_identifier`/`qualified_identifier`/plain `identifier` child whose
/// own source text is already the fully dotted path (e.g. `com.acme.app`), so no
/// recursive `scope`/`name` field reconstruction is needed.
fn dotted_path_text<'a>(node: Node, source: &'a str) -> Option<&'a str> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find(|c| {
            matches!(
                c.kind(),
                "scoped_identifier" | "qualified_identifier" | "identifier"
            )
        })
        .map(|n| node_text(n, source))
}

fn node_text<'a>(node: Node, source: &'a str) -> &'a str {
    &source[node.byte_range()]
}

/// The package portion of a fully qualified import path is everything before the last
/// `.` segment (that final segment is the imported class/type name, e.g.
/// `com.acme.domain.Bar` → package `com.acme.domain`). Known limitation, not covered by
/// this epic's acceptance criteria: a wildcard import (`import com.acme.domain.*;`) may
/// already end at the package name without a trailing class segment, in which case this
/// strips one segment too many — not fixed speculatively for v1.
fn package_of_import_path(path: &str) -> Option<&str> {
    path.rfind('.').map(|idx| &path[..idx])
}

// ---------------------------------------------------------------------------------
// Java
// ---------------------------------------------------------------------------------

fn java_package_name(root: Node, source: &str) -> Option<String> {
    find_first_descendant_by_kind(root, "package_declaration")
        .and_then(|n| dotted_path_text(n, source))
        .map(str::to_string)
}

fn collect_java_imports(node: Node, source: &str, out: &mut Vec<(String, usize)>) {
    if node.kind() == "import_declaration"
        && let Some(path) = dotted_path_text(node, source)
    {
        out.push((path.to_string(), node.start_position().row + 1));
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_java_imports(child, source, out);
    }
}

fn build_java(files: &[&PathBuf], graph: &mut ImportGraph) -> Result<()> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_java::LANGUAGE.into())
        .context("loading tree-sitter-java grammar")?;

    // Captures `(file, pkg, source, tree)` for every file so the second loop below can
    // reuse the already-read source and already-parsed tree instead of re-reading and
    // re-parsing each file a second time — the first pass still has to run to completion
    // before the second (package-declaration-derived keying means `graph.nodes` isn't
    // fully populated until every file's package is known), but there's no need to touch
    // disk or tree-sitter twice per file to get there.
    let mut file_packages: Vec<(&PathBuf, String, String, tree_sitter::Tree)> = Vec::new();
    for file in files {
        let source =
            std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
        let tree = parser
            .parse(&source, None)
            .with_context(|| format!("parsing {} with tree-sitter-java", file.display()))?;

        // Package-declaration-derived, not directory-derived — Java's package name and
        // its Maven/Gradle directory layout are linked by convention, not identity.
        let pkg = java_package_name(tree.root_node(), &source).unwrap_or_default();
        graph.nodes.insert(pkg.clone());
        graph.file_packages.insert((*file).clone(), pkg.clone());
        file_packages.push((file, pkg, source, tree));
    }

    for (file, pkg, source, tree) in &file_packages {
        let mut imports = Vec::new();
        collect_java_imports(tree.root_node(), source, &mut imports);

        for (import_path, line) in imports {
            let Some(to_pkg) = package_of_import_path(&import_path) else {
                continue;
            };
            if to_pkg != pkg && graph.nodes.contains(to_pkg) {
                graph.edges.push(ImportEdge {
                    from: pkg.clone(),
                    to: to_pkg.to_string(),
                    file: (*file).clone(),
                    line,
                });
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------------
// Kotlin
// ---------------------------------------------------------------------------------

fn kotlin_package_name(root: Node, source: &str) -> Option<String> {
    find_first_descendant_by_kind(root, "package_header")
        .and_then(|n| dotted_path_text(n, source))
        .map(str::to_string)
}

/// Kotlin's import-statement node kind is `import` — *not* `import_header` as might be
/// guessed by analogy with `package_header` (verified via `to_sexp()`, not assumed; this
/// is exactly the kind of grammar surprise the plan warned Kotlin would produce).
fn collect_kotlin_imports(node: Node, source: &str, out: &mut Vec<(String, usize)>) {
    if node.kind() == "import"
        && let Some(path) = dotted_path_text(node, source)
    {
        out.push((path.to_string(), node.start_position().row + 1));
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_kotlin_imports(child, source, out);
    }
}

fn build_kotlin(files: &[&PathBuf], graph: &mut ImportGraph) -> Result<()> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_kotlin_ng::LANGUAGE.into())
        .context("loading tree-sitter-kotlin-ng grammar")?;

    // Same shape as `build_java`: capture `(file, pkg, source, tree)` up front so the
    // second loop reuses the already-read source and already-parsed tree instead of
    // re-reading/re-parsing each file.
    let mut file_packages: Vec<(&PathBuf, String, String, tree_sitter::Tree)> = Vec::new();
    for file in files {
        let source =
            std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
        let tree = parser
            .parse(&source, None)
            .with_context(|| format!("parsing {} with tree-sitter-kotlin-ng", file.display()))?;

        let pkg = kotlin_package_name(tree.root_node(), &source).unwrap_or_default();
        graph.nodes.insert(pkg.clone());
        graph.file_packages.insert((*file).clone(), pkg.clone());
        file_packages.push((file, pkg, source, tree));
    }

    for (file, pkg, source, tree) in &file_packages {
        let mut imports = Vec::new();
        collect_kotlin_imports(tree.root_node(), source, &mut imports);

        for (import_path, line) in imports {
            let Some(to_pkg) = package_of_import_path(&import_path) else {
                continue;
            };
            if to_pkg != pkg && graph.nodes.contains(to_pkg) {
                graph.edges.push(ImportEdge {
                    from: pkg.clone(),
                    to: to_pkg.to_string(),
                    file: (*file).clone(),
                    line,
                });
            }
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
        assert!(
            graph
                .edges
                .iter()
                .any(|e| e.from == "example.com/app/a" && e.to == "example.com/app/b")
        );
        assert!(
            graph
                .edges
                .iter()
                .any(|e| e.from == "example.com/app/b" && e.to == "example.com/app/a")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn go_import_of_stdlib_package_is_ignored() {
        let dir = tmp_dir("go-stdlib");
        write(&dir, "go.mod", "module example.com/app\n\ngo 1.21\n");
        let a = write(
            &dir,
            "a/a.go",
            "package a\n\nimport \"fmt\"\n\nfunc F() { fmt.Println() }\n",
        );

        let graph = build(&dir, &[a]).unwrap();

        assert!(graph.edges.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ts_import_graph_finds_a_two_module_cycle() {
        let dir = tmp_dir("ts-cycle");
        let a = write(
            &dir,
            "a/index.ts",
            "import { g } from '../b/index';\nexport function f() {}\n",
        );
        let b = write(
            &dir,
            "b/index.ts",
            "import { f } from '../a/index';\nexport function g() {}\n",
        );

        let graph = build(&dir, &[a, b]).unwrap();

        let a_dir = dir_key(&dir.join("a"));
        let b_dir = dir_key(&dir.join("b"));
        assert!(graph.edges.iter().any(|e| e.from == a_dir && e.to == b_dir));
        assert!(graph.edges.iter().any(|e| e.from == b_dir && e.to == a_dir));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_packages_maps_go_files_to_module_qualified_keys() {
        let dir = tmp_dir("go-file-packages");
        write(&dir, "go.mod", "module example.com/app\n\ngo 1.21\n");
        let a = write(&dir, "domain/a.go", "package domain\n\nfunc A() {}\n");

        let graph = build(&dir, std::slice::from_ref(&a)).unwrap();

        assert_eq!(
            graph.file_packages.get(&a),
            Some(&"example.com/app/domain".to_string())
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_packages_maps_ts_files_to_directory_keys() {
        let dir = tmp_dir("ts-file-packages");
        let a = write(&dir, "web/index.ts", "export function f() {}\n");

        let graph = build(&dir, std::slice::from_ref(&a)).unwrap();

        let expected = dir_key(&dir.join("web"));
        assert_eq!(graph.file_packages.get(&a), Some(&expected));

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

    // --- Epic 5.1: Python ---

    #[test]
    fn python_single_dot_relative_import_same_directory_produces_no_edge() {
        let dir = tmp_dir("py-same-dir");
        let a = write(&dir, "pkg/a.py", "from .sibling import foo\n");

        let graph = build(&dir, &[a]).unwrap();

        assert!(graph.edges.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn python_double_dot_relative_import_produces_edge_to_parent_package() {
        let dir = tmp_dir("py-parent-dir");
        let a = write(&dir, "pkg/a.py", "x = 1\n");
        let b = write(&dir, "pkg/sub/b.py", "from ..other import bar\n");

        let graph = build(&dir, &[a, b]).unwrap();

        let pkg = dir_key(&dir.join("pkg"));
        let pkg_sub = dir_key(&dir.join("pkg/sub"));
        assert!(graph.edges.iter().any(|e| e.from == pkg_sub && e.to == pkg));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn python_absolute_import_is_ignored() {
        let dir = tmp_dir("py-absolute");
        let a = write(&dir, "pkg/a.py", "import os\n");

        let graph = build(&dir, &[a]).unwrap();

        assert!(graph.edges.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_packages_maps_python_files_to_directory_keys() {
        let dir = tmp_dir("py-file-packages");
        let a = write(&dir, "pkg/a.py", "x = 1\n");

        let graph = build(&dir, std::slice::from_ref(&a)).unwrap();

        let expected = dir_key(&dir.join("pkg"));
        assert_eq!(graph.file_packages.get(&a), Some(&expected));

        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- Epic 5.2: Java ---

    #[test]
    fn java_import_of_local_package_produces_edge_keyed_by_package_declaration() {
        let dir = tmp_dir("java-local-import");
        let foo = write(
            &dir,
            "src/main/java/com/acme/app/Foo.java",
            "package com.acme.app;\n\nimport com.acme.domain.Bar;\nimport java.util.List;\n\nclass Foo {}\n",
        );
        let bar = write(
            &dir,
            "src/main/java/com/acme/domain/Bar.java",
            "package com.acme.domain;\n\nclass Bar {}\n",
        );

        let graph = build(&dir, &[foo.clone(), bar]).unwrap();

        // Package node key is the declared package, not the directory path.
        assert!(graph.nodes.contains("com.acme.app"));
        assert!(graph.nodes.contains("com.acme.domain"));
        assert_eq!(
            graph.file_packages.get(&foo),
            Some(&"com.acme.app".to_string())
        );

        assert!(
            graph
                .edges
                .iter()
                .any(|e| e.from == "com.acme.app" && e.to == "com.acme.domain")
        );
        // The JDK import (java.util.List) isn't a locally-declared package.
        assert!(!graph.edges.iter().any(|e| e.to == "java.util"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- Epic 5.3: Kotlin ---

    #[test]
    fn kotlin_import_of_local_package_produces_edge_keyed_by_package_header() {
        let dir = tmp_dir("kotlin-local-import");
        let app = write(
            &dir,
            "app/App.kt",
            "package com.acme.app\n\nimport com.acme.domain.Bar\n\nclass App\n",
        );
        let domain = write(
            &dir,
            "domain/Domain.kt",
            "package com.acme.domain\n\nclass Bar\n",
        );

        let graph = build(&dir, &[app, domain]).unwrap();

        assert!(graph.nodes.contains("com.acme.app"));
        assert!(graph.nodes.contains("com.acme.domain"));
        assert!(
            graph
                .edges
                .iter()
                .any(|e| e.from == "com.acme.app" && e.to == "com.acme.domain")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
