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

fn is_js_like(path: &Path) -> bool {
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
        graph.nodes.insert(dir_key(&js_module_dir(file)));
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
    fn ts_import_of_bare_package_specifier_is_ignored() {
        let dir = tmp_dir("ts-bare");
        let a = write(&dir, "a/index.ts", "import { z } from 'zod';\n");

        let graph = build(&dir, &[a]).unwrap();

        assert!(graph.edges.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
