use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tree_sitter::Node;

use crate::checker::Language;

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
/// to). Go, TypeScript/JavaScript, Java, Kotlin, and Python are all extracted via
/// per-language dispatch below. Python uses a bespoke resolver (`build_python`)
/// rather than `build_qualified_name_language`'s shared family — it does not fit the
/// Go/Java/Kotlin "qualified name" family (see that function's doc comment, and
/// `build_python`'s own doc comment for why).
pub fn build(repo_root: &Path, files: &[PathBuf]) -> Result<ImportGraph> {
    let mut graph = ImportGraph::default();

    let go_files: Vec<&PathBuf> = files_for(files, Language::Go);
    if !go_files.is_empty() {
        build_qualified_name_language(repo_root, &go_files, &mut graph, &go_lang_config())?;
    }

    let js_files: Vec<&PathBuf> = files
        .iter()
        .filter(|f| is_js_like(Language::for_path(f)))
        .collect();
    if !js_files.is_empty() {
        build_js(&js_files, &mut graph)?;
    }

    let java_files: Vec<&PathBuf> = files_for(files, Language::Java);
    if !java_files.is_empty() {
        build_qualified_name_language(repo_root, &java_files, &mut graph, &java_lang_config())?;
    }

    let kotlin_files: Vec<&PathBuf> = files_for(files, Language::Kotlin);
    if !kotlin_files.is_empty() {
        build_qualified_name_language(repo_root, &kotlin_files, &mut graph, &kotlin_lang_config())?;
    }

    let python_files: Vec<&PathBuf> = files_for(files, Language::Python);
    if !python_files.is_empty() {
        build_python(repo_root, &python_files, &mut graph)?;
    }

    let rust_files: Vec<&PathBuf> = files_for(files, Language::Rust);
    if !rust_files.is_empty() {
        build_rust(repo_root, &rust_files, &mut graph)?;
    }

    Ok(graph)
}

/// Every file in `files` whose extension `Language::for_path` maps to `lang` — the
/// per-language file-filtering every builder below needs, all going through the one
/// extension table instead of each maintaining its own `has_ext` check (see
/// `Language::extensions`'s doc comment for why that used to be risky).
fn files_for(files: &[PathBuf], lang: Language) -> Vec<&PathBuf> {
    files
        .iter()
        .filter(|f| Language::for_path(f) == Some(lang))
        .collect()
}

/// TypeScript/TSX/JavaScript are one `build_js` call, unlike every other language here
/// (each its own `Language` variant, but not its own import-graph builder — they share
/// directory-based, not qualified-name-based, resolution).
pub(crate) fn is_js_like(lang: Option<Language>) -> bool {
    matches!(
        lang,
        Some(Language::TypeScript | Language::Tsx | Language::JavaScript)
    )
}

// ---------------------------------------------------------------------------------
// Shared "qualified name" import family: Go, Java, Kotlin.
//
// All three resolve imports the same way — a file declares its own package identity,
// import statements name other qualified paths, and an edge is added only when the
// imported path resolves to a package identity some *other* file in this run declared
// (the graph-membership guard: pre-mortem.md P1 #3(i)). What differs per language is
// (a) how a file's own package identity is computed and (b) how import statements are
// walked/extracted from that language's grammar — both captured in
// `QualifiedImportLangConfig` below, mirroring `rules.rs::LangRuleConfig`'s
// table-driven-generic-function precedent (`body_finder`/`params_finder`).
// ---------------------------------------------------------------------------------

/// Each `(normalized import target, 1-based line)` pair a `collect_imports` fn appends
/// to its output — factored into a named type purely to satisfy
/// `clippy::type_complexity` on the `collect_imports` field below.
type ImportTarget = (String, usize);

/// Per-language table for `build_qualified_name_language`. See the module doc comment
/// above for why Go/Java/Kotlin share one resolver while Python (`build_python`, when
/// added) does not: Python's relative-dot-counting + `__init__.py`-heuristic resolution
/// is a genuinely different algorithm, not a config variant of this one.
struct QualifiedImportLangConfig {
    /// Node kind of this grammar's package declaration, where it has one used by
    /// `package_identity` (informational/documentation for readers — Go's
    /// `package_identity` fn below ignores its parsed tree entirely; see its doc
    /// comment for why).
    #[allow(dead_code)]
    package_decl_kind: &'static str,
    /// Node kind of this grammar's import statement — informational for readers;
    /// `collect_imports` embeds its own kind check since walk/extraction specifics
    /// differ per grammar (see `rules.rs::LangRuleConfig`'s `if_kind` for the same
    /// documentation-only-field precedent).
    #[allow(dead_code)]
    import_stmt_kind: &'static str,
    /// Loads this grammar's tree-sitter `Language`.
    language: fn() -> tree_sitter::Language,
    /// This file's own package/module identity — the `graph.nodes` key and the `from`
    /// side of any edge produced by its imports. `None` means the file is skipped
    /// entirely (no node, no edges) — e.g. Go outside a resolvable module. The final
    /// `cache` parameter is a per-`build_qualified_name_language`-call memo, used only
    /// by Go's `go_package_identity` (its nearest-`go.mod` upward search is worth
    /// caching per directory); Java's and Kotlin's implementations accept and ignore it,
    /// the same accepted-but-unused-parameter convention `_tree`/`_src` (Go) and
    /// `_repo_root` (Java/Kotlin) already use.
    package_identity: fn(
        repo_root: &Path,
        file: &Path,
        tree: &tree_sitter::Tree,
        src: &[u8],
        cache: &mut PackageIdentityCache,
    ) -> Option<String>,
    /// Every (normalized import target, 1-based line) pair in the tree. Implementations
    /// must exclude imports that don't name a package at all (e.g. Java's `import
    /// static`, which names a member) — such imports must never appear in the output,
    /// not even as a wrong/malformed entry.
    collect_imports: fn(root: Node, src: &[u8], out: &mut Vec<ImportTarget>),
}

/// Dot-to-slash package-identity normalization, shared by Java and Kotlin (whose
/// dotted `package`/`import` paths need to match Go's already-`/`-separated identities
/// and this codebase's `/`-segment `Component.paths` globs — see Story 4.2.2's
/// `normalized_java_identity_matches_slash_globs` acceptance criterion).
fn normalize_package_identity(dotted: &str) -> String {
    dotted.replace('.', "/")
}

/// Refactor of the pre-Epic-4.2 `build_go`'s core loop (Story 4.2.1), now parameterized
/// by `cfg` instead of hardcoded Go node kinds. Preserves `build_go`'s exact
/// edge-construction invariant (pre-mortem.md P1 #3(i)): an edge from `pkg` to
/// `import_path` is added only when `import_path != pkg` (no self-edges) **and**
/// `import_path` is already present in `graph.nodes` — i.e. resolved to a package built
/// from a walked repo-local file. An import of an external/third-party package can
/// never become a graph edge, for any language driven by this function, because it was
/// never inserted into `graph.nodes` in the first place.
fn build_qualified_name_language(
    repo_root: &Path,
    files: &[&PathBuf],
    graph: &mut ImportGraph,
    cfg: &QualifiedImportLangConfig,
) -> Result<()> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&(cfg.language)())
        .context("loading tree-sitter grammar")?;

    // Owns this call's `go.mod` lookup cache (see `PackageIdentityCache`'s doc comment)
    // — one cache per `build()` invocation, not global/thread-local state, so results
    // never leak across unrelated `build()` calls (e.g. between test fixtures).
    let mut identity_cache = PackageIdentityCache::new();

    // Pass 1: parse every file and register its own package identity in `graph.nodes`
    // *before* pass 2 looks anything up there — an import can only resolve to a package
    // identity contributed by some file in this same run, regardless of which file
    // (earlier or later in `files`) declares it.
    let mut file_packages: Vec<(&PathBuf, String, tree_sitter::Tree, String)> = Vec::new();
    for file in files {
        let source =
            std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
        let tree = parser
            .parse(&source, None)
            .with_context(|| format!("parsing {}", file.display()))?;

        if let Some(pkg) = (cfg.package_identity)(
            repo_root,
            file,
            &tree,
            source.as_bytes(),
            &mut identity_cache,
        ) {
            graph.nodes.insert(pkg.clone());
            graph.file_packages.insert((*file).clone(), pkg.clone());
            file_packages.push((file, pkg, tree, source));
        }
    }

    // Pass 2: extract each file's imports and add an edge only for those that resolve
    // to a `graph.nodes` entry (the graph-membership guard).
    for (file, pkg, tree, source) in &file_packages {
        let mut imports = Vec::new();
        (cfg.collect_imports)(tree.root_node(), source.as_bytes(), &mut imports);

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
// Go
// ---------------------------------------------------------------------------------

/// Per-`build_qualified_name_language`-call memo for `find_go_mod_upward`, keyed by the
/// directory the search started from (a Go file's own directory) to `(module path,
/// directory containing the resolving go.mod)`, or `None` when no `go.mod` was found.
/// Files sharing a directory (the overwhelmingly common case — a Go package is a
/// directory) hit this cache instead of re-reading and re-parsing the same `go.mod`.
/// Also threaded through `QualifiedImportLangConfig::package_identity`'s shared
/// signature so Java/Kotlin's implementations can accept-and-ignore it (see that field's
/// doc comment).
type PackageIdentityCache = std::collections::HashMap<PathBuf, Option<(String, PathBuf)>>;

/// Searches upward from `start_dir` for the nearest `go.mod`, the same algorithm real Go
/// tooling uses to resolve a file's module (`go env GOMOD` / `go list -m`: check each
/// ancestor directory in turn, stopping at the first `go.mod` found). Bounded to
/// `repo_root` — the tree kibitzer actually scanned — rather than walking all the way to
/// the filesystem root, since kibitzer has no business reading `go.mod` files outside
/// the repo it was invoked against. Returns the `module` directive's path plus the
/// directory that contains the resolving `go.mod`, since that directory (not
/// necessarily `repo_root`) is what a file's import path is computed relative to —
/// needed once the nearest `go.mod` isn't at `repo_root` itself, e.g. a Go module nested
/// under a larger, non-Go-rooted repo.
fn find_go_mod_upward(start_dir: &Path, repo_root: &Path) -> Option<(String, PathBuf)> {
    let mut dir = start_dir;
    loop {
        if let Ok(contents) = std::fs::read_to_string(dir.join("go.mod"))
            && let Some(module) = contents.lines().find_map(|line| {
                line.trim()
                    .strip_prefix("module ")
                    .map(|rest| rest.trim().to_string())
            })
        {
            return Some((module, dir.to_path_buf()));
        }

        if dir == repo_root {
            return None;
        }
        match dir.parent() {
            Some(parent) if parent.starts_with(repo_root) => dir = parent,
            _ => return None,
        }
    }
}

/// Cached wrapper around `find_go_mod_upward` — see `PackageIdentityCache`'s doc
/// comment.
fn go_module_for_dir(
    start_dir: &Path,
    repo_root: &Path,
    cache: &mut PackageIdentityCache,
) -> Option<(String, PathBuf)> {
    if let Some(cached) = cache.get(start_dir) {
        return cached.clone();
    }
    let result = find_go_mod_upward(start_dir, repo_root);
    cache.insert(start_dir.to_path_buf(), result.clone());
    result
}

fn go_package_import_path(module_path: &str, module_dir: &Path, file: &Path) -> Option<String> {
    let dir = file.parent()?;
    let rel = dir.strip_prefix(module_dir).unwrap_or(dir);
    let rel_str = rel.to_string_lossy().replace('\\', "/");
    if rel_str.is_empty() {
        Some(module_path.to_string())
    } else {
        Some(format!("{module_path}/{rel_str}"))
    }
}

/// Go's package identity is **not** derived from parsing the file's `package_clause`
/// node — that node names only the local package name (e.g. `a`), not the full import
/// path other files reference it by (e.g. `example.com/app/a`). It comes from the
/// nearest `go.mod`'s `module` directive (searched upward from the file's own
/// directory, per `find_go_mod_upward`) plus the file's directory relative to *that*
/// `go.mod`'s directory — not always `repo_root`, once a Go module is nested under a
/// larger repo. `tree`/`_src` are accepted-but-unused only to satisfy
/// `QualifiedImportLangConfig::package_identity`'s shared signature.
fn go_package_identity(
    repo_root: &Path,
    file: &Path,
    _tree: &tree_sitter::Tree,
    _src: &[u8],
    cache: &mut PackageIdentityCache,
) -> Option<String> {
    let dir = file.parent()?;
    let (module_path, module_dir) = go_module_for_dir(dir, repo_root, cache)?;
    go_package_import_path(&module_path, &module_dir, file)
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

fn go_lang_config() -> QualifiedImportLangConfig {
    QualifiedImportLangConfig {
        package_decl_kind: "package_clause",
        import_stmt_kind: "import_spec",
        language: || tree_sitter_go::LANGUAGE.into(),
        package_identity: go_package_identity,
        collect_imports: collect_go_imports,
    }
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
// Java (Story 4.2.2) — built against Epic 4.1's verified findings below.
// ---------------------------------------------------------------------------------

/// Strip a dotted qualified name's trailing member/class segment, leaving the package
/// portion — e.g. `"com.example.infra.DbClient"` -> `"com.example.infra"`. Used for
/// plain (non-wildcard) imports, where the qualified name is `pkg.Symbol`; a wildcard
/// import's qualified name is already just the package (no symbol segment to strip).
fn strip_last_segment(dotted: &str) -> &str {
    match dotted.rsplit_once('.') {
        Some((pkg, _member)) => pkg,
        None => dotted,
    }
}

/// Java's package identity comes from the file's `package_declaration` node — verified
/// via real `to_sexp()` output during development: `(program (package_declaration
/// (scoped_identifier ...)) ...)` — `package_declaration`'s dotted name is its single
/// positional (unfielded) child, same no-named-fields situation as `import_declaration`.
fn java_package_identity(
    _repo_root: &Path,
    _file: &Path,
    tree: &tree_sitter::Tree,
    src: &[u8],
    _cache: &mut PackageIdentityCache,
) -> Option<String> {
    let root = tree.root_node();
    let mut cursor = root.walk();
    let package_decl = root
        .named_children(&mut cursor)
        .find(|c| c.kind() == "package_declaration")?;
    let name_node = package_decl.named_child(0)?;
    let text = name_node.utf8_text(src).ok()?;
    Some(normalize_package_identity(text))
}

/// Walks every `import_declaration` (Java's single node kind for plain, wildcard, and
/// `import static` forms alike — Story 4.1.1's verified finding) and extracts the
/// package portion of each **non-static** import.
///
/// `import static` is excluded here, at the source: it names a member (a field or
/// method), not a package, so treating it as a package-graph edge would be wrong
/// (pre-mortem.md P1 #3). It is **not** distinguishable from a plain import via
/// `to_sexp()` or named-child inspection — both produce
/// `(import_declaration (scoped_identifier ...))` — so detection here uses a raw
/// (unnamed-included) child scan for the anonymous `"static"` token, per the verified
/// finding in the doc comment block below.
fn collect_java_imports(node: Node, src: &[u8], out: &mut Vec<(String, usize)>) {
    if node.kind() == "import_declaration" {
        let mut is_static = false;
        for i in 0..node.child_count() as u32 {
            if node.child(i).map(|c| c.kind()) == Some("static") {
                is_static = true;
                break;
            }
        }

        if !is_static
            && let Some(name_node) = node.named_child(0)
            && let Ok(text) = name_node.utf8_text(src)
        {
            // A second positional *named* child of kind `asterisk` marks a wildcard
            // import (Story 4.1.1's verified finding) — its qualified name is already
            // the package, with no trailing member segment to strip.
            let is_wildcard = node
                .named_child(1)
                .map(|c| c.kind() == "asterisk")
                .unwrap_or(false);
            let package_dotted = if is_wildcard {
                text
            } else {
                strip_last_segment(text)
            };
            out.push((
                normalize_package_identity(package_dotted),
                node.start_position().row + 1,
            ));
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_java_imports(child, src, out);
    }
}

fn java_lang_config() -> QualifiedImportLangConfig {
    QualifiedImportLangConfig {
        package_decl_kind: "package_declaration",
        import_stmt_kind: "import_declaration",
        language: || tree_sitter_java::LANGUAGE.into(),
        package_identity: java_package_identity,
        collect_imports: collect_java_imports,
    }
}

// ---------------------------------------------------------------------------------
// Kotlin (Story 4.2.2) — built against Epic 4.1's verified findings below.
// ---------------------------------------------------------------------------------

/// Kotlin's package identity comes from the file's `package_header` node (verified via
/// real `to_sexp()` output during development: `(source_file (package_header
/// (qualified_identifier ...)) ...)` — `package_header`'s dotted name is its single
/// positional (unfielded) child, same shape as Java's `package_declaration`).
fn kotlin_package_identity(
    _repo_root: &Path,
    _file: &Path,
    tree: &tree_sitter::Tree,
    src: &[u8],
    _cache: &mut PackageIdentityCache,
) -> Option<String> {
    let root = tree.root_node();
    let mut cursor = root.walk();
    let package_header = root
        .named_children(&mut cursor)
        .find(|c| c.kind() == "package_header")?;
    let name_node = package_header.named_child(0)?;
    let text = name_node.utf8_text(src).ok()?;
    Some(normalize_package_identity(text))
}

/// Walks every `import` node (Kotlin's single node kind for plain, wildcard, and
/// aliased forms alike — Story 4.1.2's verified finding) and extracts each one's
/// package portion.
///
/// A wildcard import (`import a.b.*`) is **byte-identical** in `to_sexp()`/named-child
/// shape to a plain import one segment shorter (`import a.b`) — the trailing `.*` is
/// two anonymous tokens (Story 4.1.2's verified real trap). Disambiguation here uses a
/// raw (unnamed-included) child check: the wildcard import's *last raw child* has kind
/// `"*"`. An aliased import (`import a.b.C as D`) carries a second positional
/// `identifier` child for the alias — irrelevant to package resolution, since the first
/// child's qualified name already includes the pre-alias symbol segment to strip, same
/// as a plain import.
fn collect_kotlin_imports(node: Node, src: &[u8], out: &mut Vec<(String, usize)>) {
    if node.kind() == "import"
        && let Some(name_node) = node.named_child(0)
        && let Ok(text) = name_node.utf8_text(src)
    {
        let is_wildcard = node.child_count() > 0
            && node
                .child(node.child_count() as u32 - 1)
                .map(|c| c.kind() == "*")
                .unwrap_or(false);
        let package_dotted = if is_wildcard {
            text
        } else {
            strip_last_segment(text)
        };
        out.push((
            normalize_package_identity(package_dotted),
            node.start_position().row + 1,
        ));
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_kotlin_imports(child, src, out);
    }
}

fn kotlin_lang_config() -> QualifiedImportLangConfig {
    QualifiedImportLangConfig {
        package_decl_kind: "package_header",
        import_stmt_kind: "import",
        language: || tree_sitter_kotlin_ng::LANGUAGE.into(),
        package_identity: kotlin_package_identity,
        collect_imports: collect_kotlin_imports,
    }
}

// ---------------------------------------------------------------------------------
// Java / Kotlin import node-kind verification (Epic 4.1) — real `to_sexp()` output
// pinned ahead of the extraction code (Story 4.2.2/4.2.3), per this codebase's
// verification discipline (`docs/syntax-rules.md`, `rules.rs`'s `LangRuleConfig` doc
// comment). Findings below are what Epic 4.2's implementation above was built against.
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
//   parses cleanly outside the `ERROR` node. (A missing `;` on an *import* statement
//   itself is a different, also-verified case — see
//   `java_import_graph_malformed_source_never_extracts_a_wrong_edge` below: that one
//   recovers with a `MISSING ";"` node inserted in place, not a flat `ERROR` wrapper.)
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

// ---------------------------------------------------------------------------------
// Python import node-kind verification (Epic 5.1 / Story 5.1.1) — real `to_sexp()`
// output pinned ahead of the extraction code (Story 5.2.1), per this codebase's
// verification discipline (`docs/syntax-rules.md`, `rules.rs`'s `LangRuleConfig` doc
// comment, and the Java/Kotlin precedent immediately above). No `build_python` exists
// yet; the findings below are what Epic 5.2's implementer builds against. Verified
// against `tree-sitter-python` 0.23.6, matching `Cargo.lock`.
//
// research/build-vs-buy.md §4 and research/pitfalls.md §1 predicted six distinct import
// node kinds from the grammar's `node-types.json` (not run through a live parser at
// research time). All six are confirmed present, with one refinement pitfalls.md didn't
// surface: `aliased_import` is never a standalone import — it always appears *nested*
// inside `import_statement`'s or `import_from_statement`'s `name` field, replacing the
// plain `dotted_name` there.
//
// - `import os` → `import_statement`, single required field `name` (accepts
//   `dotted_name` or `aliased_import`, and can repeat for `import os, sys`):
//   `(import_statement name: (dotted_name (identifier)))`.
// - `import os as o` (top-level aliased form) → same `import_statement` node kind, but
//   the `name` field's child is `aliased_import` instead of `dotted_name`:
//   `(import_statement name: (aliased_import name: (dotted_name (identifier)) alias:
//   (identifier)))` — confirmed via `python_top_level_aliased_import_shape` below.
// - `from app.infra import db_client` → `import_from_statement`, fields `module_name`
//   (required, `dotted_name` or `relative_import`) and `name` (optional, repeatable,
//   `dotted_name` or `aliased_import`):
//   `(import_from_statement module_name: (dotted_name (identifier) (identifier)) name:
//   (dotted_name (identifier)))`.
// - `from app.infra import db_client as db` → same `import_from_statement` node kind,
//   `name` field's child is `aliased_import` (same nesting pattern as the top-level
//   form above): `(import_from_statement module_name: (dotted_name (identifier)
//   (identifier)) name: (aliased_import name: (dotted_name (identifier)) alias:
//   (identifier)))`.
// - `from . import sibling` / `from ..domain import Order` (relative) → `module_name`
//   field's child is `relative_import`, wrapping `import_prefix` (the leading dots) and
//   an optional `dotted_name` (present only when a package follows the dots):
//   `(import_from_statement module_name: (relative_import (import_prefix)) name:
//   (dotted_name (identifier)))` for `from . import sibling` (no `dotted_name` inside
//   `relative_import` — one named child), and `(import_from_statement module_name:
//   (relative_import (import_prefix) (dotted_name (identifier))) name: (dotted_name
//   (identifier)))` for `from ..domain import Order` (two named children). **Dot count
//   is not encoded in `to_sexp()`/node kind at all** — `import_prefix`'s *raw source
//   text* is exactly N `.` characters (verified: `"."`, `".."`, `"..."` for one/two/three
//   leading dots respectively, via `Node::utf8_text`, both with and without a trailing
//   package name) — Story 5.2.1b's dot-counting must read `import_prefix`'s source text
//   length, not infer it from tree structure.
// - `from app.infra import *` (wildcard) → `import_from_statement` again, but `name` is
//   *absent* and an unfielded (positional) `wildcard_import` child appears instead:
//   `(import_from_statement module_name: (dotted_name (identifier) (identifier))
//   (wildcard_import))` — a walk that only reads the `name` field will silently miss
//   this form's target entirely; the presence of a local project name to bind is simply
//   absent, so this is (as pitfalls.md guessed) a package-level "import everything from
//   X" that still resolves via `module_name` alone, same as any other `import_from_statement`.
// - `from __future__ import annotations` → **`future_import_statement`**, a wholly
//   separate top-level node kind from `import_from_statement` (confirms pitfalls.md's
//   prediction) — no `module_name` field at all, just `name`:
//   `(future_import_statement name: (dotted_name (identifier)))`. A walk matching only
//   `import_from_statement`/`import_statement` will silently skip `__future__` imports
//   rather than mishandling them — which is the *correct* outcome per Story 5.2.1's
//   acceptance criteria (zero graph edges for `__future__`), but only if the walk
//   explicitly excludes this kind rather than never having considered it.
// - Malformed source (`import os\n\ndef broken(\n` — unclosed parameter list): recovers
//   as `(module (import_statement name: (dotted_name (identifier))) (ERROR (identifier)))`
//   — the well-formed leading `import` parses cleanly outside a single flat `ERROR` node
//   wrapping the truncated `def`'s name, same "ERROR wraps just the malformed tail"
//   recovery shape as Java's (not Kotlin's in-place `MISSING` style).
//
// See `declarations.rs`'s Python section (Epic 5.1's "one additional shape variant") for
// the `class_definition`/`function_definition` verification, including confirmation that
// a class's nested method sits two levels below `module` (`module` → `class_definition`
// → `body` (block) → `function_definition`), one level deeper than a module-level
// function (`module` → `function_definition` directly) — the structural distinction
// Story 5.3.1 needs to exclude nested methods from top-level extraction.

// ---------------------------------------------------------------------------------
// Python (Epic 5.2 / Story 5.2.1) — bespoke resolver, deliberately not a
// `QualifiedImportLangConfig` variant. Python has no parsed package-declaration node
// (identity comes from directory structure + `__init__.py` presence, not a tree
// node), relative imports resolve by counting leading dots rather than matching a
// qualified name, and absolute imports resolve via a heuristic package-tree walk
// rather than an exact declaration-vs-import match — three algorithms genuinely
// different from `build_qualified_name_language`'s single "declare identity, match
// import path" shape, not config variants of it. Built against the verified
// `to_sexp()` findings in the doc comment block immediately above.
// ---------------------------------------------------------------------------------

/// Task 5.2.1a — a file's own package identity, computed from directory structure
/// rather than a parsed declaration (Python has none): walk upward from the file's
/// own directory while each ancestor still contains an `__init__.py`, and take the
/// dotted path from the first ancestor that does *not* (the "first non-package
/// ancestor" — the package tree's root boundary) down to the file's own directory,
/// normalized via `normalize_package_identity` for consistency with every other
/// language's `/`-separated node keys. `None` if the file's own directory isn't
/// itself a package (no `__init__.py`) — mirrors `QualifiedImportLangConfig`'s
/// "skip the file entirely, no node, no edges" convention for Go/Java/Kotlin.
fn python_package_of(repo_root: &Path, file: &Path) -> Option<String> {
    let file_dir = file.parent()?;
    if !file_dir.join("__init__.py").is_file() {
        return None;
    }

    let mut topmost = file_dir.to_path_buf();
    while let Some(parent) = topmost.parent() {
        if parent == repo_root
            || !parent.starts_with(repo_root)
            || !parent.join("__init__.py").is_file()
        {
            break;
        }
        topmost = parent.to_path_buf();
    }

    let boundary = topmost.parent().unwrap_or(repo_root);
    let rel = file_dir.strip_prefix(boundary).ok()?;
    let segments: Vec<String> = rel
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    if segments.is_empty() {
        return None;
    }
    Some(normalize_package_identity(&segments.join(".")))
}

/// Task 5.2.1c — absolute-import resolution against the heuristic package tree. Both
/// `import_from_statement`'s `module_name` field and a plain `import_statement`'s
/// `name` field already exclude any imported symbol — Python's grammar keeps those
/// separate (unlike Java's `import pkg.Class`, which needs `strip_last_segment`) — so
/// resolution here is a straight dot-to-slash normalize-and-membership-check against
/// `graph.nodes`, the same invariant `build_qualified_name_language`'s pass 2 enforces
/// (pre-mortem.md P1 #3(i)): an import can only become an edge if it names a package
/// some file in this run actually declared.
fn resolve_python_absolute_import(dotted: &str, known: &BTreeSet<String>) -> Option<String> {
    let normalized = normalize_package_identity(dotted);
    known.contains(&normalized).then_some(normalized)
}

/// Task 5.2.1b — relative-import resolution by counting leading dots, grounded in
/// Story 5.1.1's verified finding (`python_relative_import_prefix_dot_count_is_in_source_text`):
/// dot count lives only in `import_prefix`'s raw source text, never in tree shape or
/// node kind. One dot means "the current package" — the importing file's own
/// directory; each additional dot pops one more directory level. Reuses `build_js`'s
/// `resolve_relative_import` precedent directly (a canonicalized-directory lookup
/// map), since Python's relative imports name packages, not files — there's no
/// extension/`index`-file resolution to layer on top the way JS needs.
fn resolve_python_relative_import(
    from_file: &Path,
    dots: usize,
    dotted_suffix: &str,
    known: &std::collections::HashMap<PathBuf, String>,
) -> Option<String> {
    let mut base = from_file.parent()?.to_path_buf();
    for _ in 1..dots {
        base = base.parent()?.to_path_buf();
    }
    if !dotted_suffix.is_empty() {
        base = base.join(dotted_suffix.replace('.', "/"));
    }
    known.get(&base.canonicalize().ok()?).cloned()
}

/// Each import target `collect_python_imports` finds, tagged by which of the two
/// resolvers (`resolve_python_absolute_import` / `resolve_python_relative_import`)
/// handles it.
enum PythonImportTarget {
    Absolute(String),
    Relative { dots: usize, suffix: String },
}

/// Task 5.2.1d — walks `import_statement` and `import_from_statement` (plain,
/// aliased, relative, and wildcard forms alike — a wildcard import's target still
/// resolves via its `module_name` field alone, exactly like any other
/// `import_from_statement`, per the verified finding in the doc comment block above:
/// the absent `name` field only means there's no single local symbol to bind, not
/// that the module path is unresolvable). `future_import_statement` is explicitly
/// excluded: it is a wholly separate node kind with no `module_name` field, and per
/// Story 5.2.1's acceptance criteria must produce zero edges and no error — excluded
/// at the source rather than mishandled as an unresolvable local import.
fn collect_python_imports(node: Node, src: &[u8], out: &mut Vec<(PythonImportTarget, usize)>) {
    match node.kind() {
        "import_statement" => {
            let mut cursor = node.walk();
            for name_node in node.children_by_field_name("name", &mut cursor) {
                let dotted_node = if name_node.kind() == "aliased_import" {
                    name_node.child_by_field_name("name")
                } else {
                    Some(name_node)
                };
                if let Some(dotted_node) = dotted_node
                    && let Ok(text) = dotted_node.utf8_text(src)
                {
                    out.push((
                        PythonImportTarget::Absolute(text.to_string()),
                        node.start_position().row + 1,
                    ));
                }
            }
        }
        "import_from_statement" => {
            if let Some(module_name) = node.child_by_field_name("module_name") {
                let line = node.start_position().row + 1;
                match module_name.kind() {
                    "dotted_name" => {
                        if let Ok(text) = module_name.utf8_text(src) {
                            out.push((PythonImportTarget::Absolute(text.to_string()), line));
                        }
                    }
                    "relative_import" => {
                        let mut dots = 0usize;
                        let mut suffix = String::new();
                        let mut rcursor = module_name.walk();
                        for child in module_name.named_children(&mut rcursor) {
                            match child.kind() {
                                "import_prefix" => {
                                    if let Ok(text) = child.utf8_text(src) {
                                        dots = text.chars().filter(|&c| c == '.').count();
                                    }
                                }
                                "dotted_name" => {
                                    if let Ok(text) = child.utf8_text(src) {
                                        suffix = text.to_string();
                                    }
                                }
                                _ => {}
                            }
                        }
                        out.push((PythonImportTarget::Relative { dots, suffix }, line));
                    }
                    _ => {}
                }
            }
        }
        // `future_import_statement` falls through here, deliberately unhandled — see
        // this function's doc comment.
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_python_imports(child, src, out);
    }
}

/// Task 5.2.1e — Python's bespoke `build()` entry point, wired into `.py` dispatch
/// above. Two passes, the same graph-membership-guard shape as
/// `build_qualified_name_language` (pre-mortem.md P1 #3(i)): pass 1 registers every
/// file's own package identity (and, for relative-import resolution, its
/// canonicalized directory) before pass 2 looks anything up — an import can resolve
/// to a package declared by any file in this run regardless of walk order — and
/// pass 2 adds an edge only when resolution actually finds a `graph.nodes` entry.
fn build_python(repo_root: &Path, files: &[&PathBuf], graph: &mut ImportGraph) -> Result<()> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_python::LANGUAGE.into())
        .context("loading tree-sitter-python grammar")?;

    let mut file_data: Vec<(&PathBuf, String, tree_sitter::Tree, String)> = Vec::new();
    let mut known_dirs: std::collections::HashMap<PathBuf, String> =
        std::collections::HashMap::new();

    for file in files {
        let source =
            std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
        let tree = parser
            .parse(&source, None)
            .with_context(|| format!("parsing {}", file.display()))?;

        if let Some(pkg) = python_package_of(repo_root, file) {
            graph.nodes.insert(pkg.clone());
            graph.file_packages.insert((*file).clone(), pkg.clone());
            if let Some(dir) = file.parent().and_then(|d| d.canonicalize().ok()) {
                known_dirs.insert(dir, pkg.clone());
            }
            file_data.push((file, pkg, tree, source));
        }
    }

    for (file, pkg, tree, source) in &file_data {
        let mut imports = Vec::new();
        collect_python_imports(tree.root_node(), source.as_bytes(), &mut imports);

        for (target, line) in imports {
            let resolved = match target {
                PythonImportTarget::Absolute(dotted) => {
                    resolve_python_absolute_import(&dotted, &graph.nodes)
                }
                PythonImportTarget::Relative { dots, suffix } => {
                    resolve_python_relative_import(file, dots, &suffix, &known_dirs)
                }
            };
            if let Some(to) = resolved
                && &to != pkg
            {
                graph.edges.push(ImportEdge {
                    from: pkg.clone(),
                    to,
                    file: (*file).clone(),
                    line,
                });
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------------
// Rust — bespoke resolver, deliberately not a `QualifiedImportLangConfig` variant.
// Package identity is crate- and directory-derived, not from a parsed declaration —
// Rust has no single "this file's package" node the way Go's `package_clause` or
// Java's `package_declaration` are (a file's real *module* path comes from where
// `mod` statements wire it into the crate tree, tracked project-wide, not from
// anything local to the file itself). `rust_module_key` instead follows the
// filesystem convention real Rust tooling defaults to (`foo/bar.rs` -> `bar` nested
// under `foo`; `foo/mod.rs`/`foo.rs` -> `foo` itself) — covers every file actually
// wired into its crate via a `mod` declaration matching its path, the dominant
// real-world case, but not one moved via a `#[path = "..."]` attribute override
// (accepted v1 limitation: no attempt to parse `#[path]`). Also file-granularity
// only: an inline `mod foo { ... }` block's contents stay attributed to the
// *enclosing file's* module key rather than a synthetic nested one, the same
// granularity every other language here uses (a JS/TS/Java/Kotlin nested class
// doesn't get its own node either).
//
// A repo can hold multiple crates (a Cargo workspace) sharing module names that
// would otherwise collide (`crate::foo` in one crate is unrelated to `crate::foo`
// in another) — `rust_module_key` scopes every node under the nearest ancestor
// directory containing a `Cargo.toml` (found via the same bounded upward walk
// `find_go_mod_upward` uses for `go.mod`), keyed by that crate directory's own
// repo-relative path, so a name collision across crates can't produce a false edge.
//
// `use` resolution (`resolve_rust_use_path`) only resolves `crate::`/`self::`/
// `super::`-prefixed paths — those are unambiguously same-crate. A bare leading
// segment (`use foo::Bar;`, Rust 2018+ "uniform paths") is deliberately left
// unresolved: disambiguating it from a genuine external-crate name needs a full
// symbol table, not just this file's own AST (accepted v1 limitation, same spirit as
// Go's stdlib-import skip and JS's bare-specifier skip). The graph-membership guard
// below excludes it from producing an edge either way, so this only means a real
// same-crate uniform-path import goes uncounted, never that a wrong edge appears.
// ---------------------------------------------------------------------------------

/// Bounded upward walk for the nearest ancestor `Cargo.toml`, identical in shape to
/// `find_go_mod_upward` — stops at `repo_root` rather than walking to the filesystem
/// root, since kibitzer has no business reading a `Cargo.toml` outside the repo it
/// was invoked against. Returns the crate root directory (the `Cargo.toml`'s own
/// directory), not the manifest path itself.
fn find_cargo_toml_upward(start_dir: &Path, repo_root: &Path) -> Option<PathBuf> {
    let mut dir = start_dir;
    loop {
        if dir.join("Cargo.toml").is_file() {
            return Some(dir.to_path_buf());
        }
        if dir == repo_root {
            return None;
        }
        match dir.parent() {
            Some(parent) if parent.starts_with(repo_root) => dir = parent,
            _ => return None,
        }
    }
}

/// Reads a crate's declared package name straight out of its `Cargo.toml`'s
/// `[package]` table — a simple line scan, not a full TOML parser, the same minimal
/// treatment this file already gives `go.mod`'s `module` line. A guaranteed-unique
/// key across an entire Cargo workspace (crate names must be unique for Cargo's own
/// dependency resolution to work), and far friendlier in rendered output than a
/// directory-relative path. Falls back to the crate directory's own name if no
/// `[package]`/`name` key is found — e.g. a workspace root manifest with no
/// `[package]` of its own, which `find_cargo_toml_upward` only returns for a file
/// that isn't under any workspace member's own directory (an unusual layout, but
/// still needs *some* stable key rather than an empty one).
fn rust_crate_key(crate_dir: &Path) -> String {
    let manifest = std::fs::read_to_string(crate_dir.join("Cargo.toml")).unwrap_or_default();
    let mut in_package_table = false;
    for line in manifest.lines() {
        let line = line.trim();
        if let Some(table) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            in_package_table = table == "package";
            continue;
        }
        if in_package_table
            && let Some(rest) = line.strip_prefix("name")
            && let Some(value) = rest.trim_start().strip_prefix('=')
            && let Some(name) = value
                .trim()
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
        {
            return name.to_string();
        }
    }
    crate_dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// A file's module key: the crate's package name alone for the crate root itself
/// (`src/main.rs`/`src/lib.rs`), or with `::`-joined module segments appended for
/// everything else. `None` when the file isn't inside a resolvable crate (no
/// ancestor `Cargo.toml`) or isn't under that crate's `src/` directory (a
/// non-standard layout — e.g. a build script or an example — deliberately out of
/// scope, mirroring every other resolver's "skip the file entirely" convention for
/// what it doesn't handle).
fn rust_module_key(repo_root: &Path, file: &Path) -> Option<String> {
    let file_dir = file.parent()?;
    let crate_dir = find_cargo_toml_upward(file_dir, repo_root)?;
    let src_dir = crate_dir.join("src");
    let rel = file.strip_prefix(&src_dir).ok()?;

    let crate_key = rust_crate_key(&crate_dir);

    let mut segments: Vec<String> = rel
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();

    // The file's own basename names a module only when it isn't one of the three
    // "this directory's own module" filenames — `main.rs`/`lib.rs` (crate root) and
    // `mod.rs` (the pre-2018 "directory as module" file) collapse into their
    // containing directory's key instead of adding a segment of their own.
    if let Some(last) = segments.pop() {
        let stem = last.strip_suffix(".rs").unwrap_or(&last);
        if !matches!(stem, "main" | "lib" | "mod") {
            segments.push(stem.to_string());
        }
    }

    if segments.is_empty() {
        Some(crate_key)
    } else {
        Some(format!("{crate_key}::{}", segments.join("::")))
    }
}

/// Recursively flattens a `use` path expression (`scoped_identifier`, or a leaf
/// `identifier`/`crate`/`self`/`super`) into its dot-free segments, e.g.
/// `crate::foo::Bar` -> `["crate", "foo", "Bar"]`. Verified via `to_sexp()`:
/// `scoped_identifier` always carries `path`/`name` fields, recursing through nested
/// `scoped_identifier`s until a leaf keyword (`crate`/`self`/`super`) or plain
/// `identifier` ends the chain.
fn flatten_rust_path(node: Node, src: &[u8]) -> Vec<String> {
    match node.kind() {
        "scoped_identifier" => {
            let mut segments = node
                .child_by_field_name("path")
                .map(|p| flatten_rust_path(p, src))
                .unwrap_or_default();
            if let Some(name) = node.child_by_field_name("name")
                && let Ok(text) = name.utf8_text(src)
            {
                segments.push(text.to_string());
            }
            segments
        }
        "crate" => vec!["crate".to_string()],
        "self" => vec!["self".to_string()],
        "super" => vec!["super".to_string()],
        "identifier" | "type_identifier" => node
            .utf8_text(src)
            .map(|t| vec![t.to_string()])
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// Expands one `use_declaration`'s `argument` into every raw path it names, alongside
/// the declaration's own line — a `scoped_use_list` (`use crate::foo::{Bar, Baz}`)
/// expands to one entry per braced item, each inheriting the shared path prefix; a
/// `use_wildcard` (`use std::io::*`) contributes the wildcarded module path itself,
/// treating the glob as a dependency on that whole module; `use_as_clause`
/// (`use super::qux as renamed`) resolves by its un-renamed `path` — the alias never
/// affects what's actually depended on; a bare `self` *as a use-list item*
/// (`use foo::{self, Bar}`, verified via `to_sexp()`) means "the prefix module
/// itself", a different meaning from `self` at the head of a path chain (handled by
/// `flatten_rust_path`/`resolve_rust_use_path` instead, where it means "current
/// module") — disambiguated here by `prefix` being non-empty only inside a list.
fn collect_rust_use_paths(
    argument: Node,
    src: &[u8],
    prefix: &[String],
    line: usize,
    out: &mut Vec<(Vec<String>, usize)>,
) {
    match argument.kind() {
        "self" if !prefix.is_empty() => {
            out.push((prefix.to_vec(), line));
        }
        "scoped_identifier" | "crate" | "self" | "super" | "identifier" | "type_identifier" => {
            let mut segments = prefix.to_vec();
            segments.extend(flatten_rust_path(argument, src));
            if !segments.is_empty() {
                out.push((segments, line));
            }
        }
        "scoped_use_list" => {
            let path_prefix = argument
                .child_by_field_name("path")
                .map(|p| flatten_rust_path(p, src))
                .unwrap_or_default();
            if let Some(list) = argument.child_by_field_name("list") {
                let mut cursor = list.walk();
                for item in list.named_children(&mut cursor) {
                    collect_rust_use_paths(item, src, &path_prefix, line, out);
                }
            }
        }
        "use_as_clause" => {
            if let Some(path) = argument.child_by_field_name("path") {
                collect_rust_use_paths(path, src, prefix, line, out);
            }
        }
        "use_wildcard" => {
            if let Some(inner) = argument.named_child(0) {
                collect_rust_use_paths(inner, src, prefix, line, out);
            }
        }
        _ => {}
    }
}

fn collect_rust_imports(node: Node, src: &[u8], out: &mut Vec<(Vec<String>, usize)>) {
    if node.kind() == "use_declaration"
        && let Some(argument) = node.child_by_field_name("argument")
    {
        collect_rust_use_paths(argument, src, &[], node.start_position().row + 1, out);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_rust_imports(child, src, out);
    }
}

/// Resolves one `use` path's raw segments (as `collect_rust_imports` found them)
/// against `own_module` (the importing file's own module key, from
/// `rust_module_key`) and `known` (every module key registered in this run).
/// `crate`/`self`/`super` are the only leading segments substituted — anything else
/// is left unresolved (see this section's module-level doc comment). Tries the full
/// resolved path first, then with its last segment stripped (an imported *item*
/// rather than the module itself, e.g. `use crate::foo::Bar` naming the type `Bar`
/// inside module `crate::foo`) — the same two shapes real code produces at this
/// syntactic position, tried in order rather than assumed unconditionally the way
/// Java/Kotlin's `strip_last_segment` can, since a bare Rust `use` path genuinely can
/// name either a module or an item.
///
/// Accepted v1 limitation: only a single leading `super` is popped — a chained
/// `super::super::foo` (real, valid Rust for reaching a grandparent module) treats
/// the second `super` as a literal path segment instead of popping again, so it
/// resolves to nothing rather than the intended target. Not fixed speculatively;
/// `super::super::` is rare enough in practice (most code reaches further ancestors
/// via `crate::` instead) that this hasn't shown up as a real miss yet.
fn resolve_rust_use_path(
    raw: &[String],
    own_module: &str,
    known: &BTreeSet<String>,
) -> Option<String> {
    let (head, rest) = raw.split_first()?;
    let mut own_segments: Vec<String> = own_module.split("::").map(String::from).collect();
    let mut resolved: Vec<String> = match head.as_str() {
        "crate" if !own_segments.is_empty() => vec![own_segments.remove(0)],
        "self" => own_segments,
        "super" => {
            own_segments.pop();
            own_segments
        }
        _ => return None,
    };
    resolved.extend(rest.iter().cloned());
    if resolved.is_empty() {
        return None;
    }

    let full = resolved.join("::");
    if known.contains(&full) {
        return Some(full);
    }
    if resolved.len() > 1 {
        resolved.pop();
        let without_last = resolved.join("::");
        if known.contains(&without_last) {
            return Some(without_last);
        }
    }
    None
}

/// Rust's bespoke `build()` entry point. Same two-pass graph-membership-guard shape
/// as `build_python`/`build_qualified_name_language`: pass 1 registers every file's
/// own module key before pass 2 resolves any `use` path against the full set, so
/// resolution never depends on file walk order.
fn build_rust(repo_root: &Path, files: &[&PathBuf], graph: &mut ImportGraph) -> Result<()> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .context("loading tree-sitter-rust grammar")?;

    let mut file_data: Vec<(&PathBuf, String, tree_sitter::Tree, String)> = Vec::new();

    for file in files {
        let source =
            std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
        let tree = parser
            .parse(&source, None)
            .with_context(|| format!("parsing {} with tree-sitter-rust", file.display()))?;

        if let Some(module) = rust_module_key(repo_root, file) {
            graph.nodes.insert(module.clone());
            graph.file_packages.insert((*file).clone(), module.clone());
            file_data.push((file, module, tree, source));
        }
    }

    for (file, module, tree, source) in &file_data {
        let mut imports = Vec::new();
        collect_rust_imports(tree.root_node(), source.as_bytes(), &mut imports);

        for (raw, line) in imports {
            if let Some(to) = resolve_rust_use_path(&raw, module, &graph.nodes)
                && &to != module
            {
                graph.edges.push(ImportEdge {
                    from: module.clone(),
                    to,
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

    /// Regression test for the real dogfood-architecture gap: `repo_root` (the
    /// directory kibitzer was invoked against) has **no** `go.mod` at all — it's a
    /// polyglot repo root, mirroring kibitzer's own repo root relative to
    /// `testdata/dogfood-architecture/`'s nested Go module — but a subdirectory two
    /// levels down does. `go_package_identity` must find that nested `go.mod` by
    /// searching upward from each file's own directory, not just check
    /// `repo_root/go.mod` and give up. Both files' internal imports of each other must
    /// still resolve to real graph edges.
    #[test]
    fn go_import_graph_finds_nested_module_below_a_non_go_repo_root() {
        let dir = tmp_dir("go-nested-module");
        // No go.mod at `dir` itself — only under the nested `nested-module` subdir.
        write(
            &dir,
            "nested-module/go.mod",
            "module nested.example/mod\n\ngo 1.21\n",
        );
        let a = write(
            &dir,
            "nested-module/a/a.go",
            "package a\n\nimport \"nested.example/mod/b\"\n\nfunc F() { b.G() }\n",
        );
        let b = write(
            &dir,
            "nested-module/b/b.go",
            "package b\n\nimport \"nested.example/mod/a\"\n\nfunc G() { a.F() }\n",
        );

        let graph = build(&dir, &[a, b]).unwrap();

        assert!(graph.nodes.contains("nested.example/mod/a"));
        assert!(graph.nodes.contains("nested.example/mod/b"));
        assert!(
            graph
                .edges
                .iter()
                .any(|e| e.from == "nested.example/mod/a" && e.to == "nested.example/mod/b")
        );
        assert!(
            graph
                .edges
                .iter()
                .any(|e| e.from == "nested.example/mod/b" && e.to == "nested.example/mod/a")
        );

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

    // --- Epic 4.2 / Story 4.2.1: graph-membership guard survives the refactor ---

    /// Task 4.2.1e — regression test proving `build_qualified_name_language` preserves
    /// pre-refactor `build_go`'s edge-construction invariant (pre-mortem.md P1 #3(i)):
    /// an import of an external/third-party package (never inserted into `graph.nodes`)
    /// never becomes a graph edge, even though it is a syntactically valid import
    /// sitting right next to a local one.
    #[test]
    fn build_qualified_name_language_never_creates_edges_to_non_graph_nodes() {
        let dir = tmp_dir("go-external-import");
        write(&dir, "go.mod", "module example.com/app\n\ngo 1.21\n");
        let a = write(&dir, "a/a.go", "package a\n\nfunc F() {}\n");
        let b = write(
            &dir,
            "b/b.go",
            "package b\n\nimport (\n\t\"example.com/app/a\"\n\t\"github.com/some-vendor/infra-client\"\n)\n\nfunc G() { a.F() }\n",
        );

        let graph = build(&dir, &[a, b]).unwrap();

        assert!(graph.nodes.contains("example.com/app/a"));
        assert!(graph.nodes.contains("example.com/app/b"));
        assert!(!graph.nodes.contains("github.com/some-vendor/infra-client"));
        assert!(
            graph
                .edges
                .iter()
                .any(|e| e.from == "example.com/app/b" && e.to == "example.com/app/a")
        );
        assert!(
            !graph
                .edges
                .iter()
                .any(|e| e.to == "github.com/some-vendor/infra-client")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- Epic 4.2 / Story 4.2.2: Java + Kotlin import extraction ---

    #[test]
    fn java_import_graph_finds_a_two_package_cycle_with_normalized_identity() {
        let dir = tmp_dir("java-cycle");
        let order = write(
            &dir,
            "src/main/java/com/example/domain/Order.java",
            "package com.example.domain;\n\nimport com.example.infra.DbClient;\n\npublic class Order {\n    public String id;\n}\n",
        );
        let db_client = write(
            &dir,
            "src/main/java/com/example/infra/DbClient.java",
            "package com.example.infra;\n\nimport com.example.domain.Order;\n\npublic class DbClient {\n    public Order order;\n}\n",
        );

        let graph = build(&dir, &[order, db_client]).unwrap();

        assert!(graph.nodes.contains("com/example/domain"));
        assert!(graph.nodes.contains("com/example/infra"));
        assert!(!graph.nodes.contains("com.example.domain"));
        assert!(
            graph
                .edges
                .iter()
                .any(|e| e.from == "com/example/domain" && e.to == "com/example/infra")
        );
        assert!(
            graph
                .edges
                .iter()
                .any(|e| e.from == "com/example/infra" && e.to == "com/example/domain")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn kotlin_import_graph_handles_plain_wildcard_and_aliased_imports() {
        let dir = tmp_dir("kotlin-forms");
        let order = write(
            &dir,
            "domain/Order.kt",
            "package com.example.domain\n\nimport com.example.infra.DbClient\n\nclass Order(val id: String)\n",
        );
        let db_client = write(
            &dir,
            "infra/DbClient.kt",
            "package com.example.infra\n\nimport com.example.domain.*\nimport com.example.util.Helper as UtilHelper\n\nclass DbClient\n",
        );
        let helper = write(
            &dir,
            "util/Helper.kt",
            "package com.example.util\n\nclass Helper\n",
        );

        let graph = build(&dir, &[order, db_client, helper]).unwrap();

        assert!(graph.nodes.contains("com/example/domain"));
        assert!(graph.nodes.contains("com/example/infra"));
        assert!(graph.nodes.contains("com/example/util"));
        // Plain import (Order.kt -> DbClient).
        assert!(
            graph
                .edges
                .iter()
                .any(|e| e.from == "com/example/domain" && e.to == "com/example/infra")
        );
        // Wildcard import (DbClient.kt -> com.example.domain.*).
        assert!(
            graph
                .edges
                .iter()
                .any(|e| e.from == "com/example/infra" && e.to == "com/example/domain")
        );
        // Aliased import (DbClient.kt -> com.example.util.Helper as UtilHelper).
        assert!(
            graph
                .edges
                .iter()
                .any(|e| e.from == "com/example/infra" && e.to == "com/example/util")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn normalized_java_identity_matches_slash_globs() {
        use crate::config::{Component, component_of};

        let dir = tmp_dir("java-glob");
        let order = write(
            &dir,
            "src/main/java/com/example/domain/Order.java",
            "package com.example.domain;\n\npublic class Order {}\n",
        );
        let db_client = write(
            &dir,
            "src/main/java/com/example/infra/DbClient.java",
            "package com.example.infra;\n\npublic class DbClient {}\n",
        );

        let graph = build(&dir, &[order, db_client]).unwrap();

        let components = vec![Component {
            name: "domain".to_string(),
            paths: vec!["**/domain".to_string()],
        }];
        assert_eq!(
            component_of("com/example/domain", &components),
            Some("domain")
        );
        assert!(graph.nodes.contains("com/example/domain"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Pre-mortem.md P1 #3(i) for Java specifically: an external import whose final
    /// segment happens to be spelled/cased like a `Component` name (`Component`) proves
    /// nothing about it being a real, locally-declared package — it must never enter
    /// `graph.nodes` or `graph.edges`.
    #[test]
    fn java_import_graph_excludes_external_third_party_imports_from_graph_nodes() {
        let dir = tmp_dir("java-external");
        let order = write(
            &dir,
            "src/main/java/com/example/domain/Order.java",
            "package com.example.domain;\n\nimport com.example.infra.DbClient;\nimport org.springframework.stereotype.Component;\n\npublic class Order {}\n",
        );
        let db_client = write(
            &dir,
            "src/main/java/com/example/infra/DbClient.java",
            "package com.example.infra;\n\npublic class DbClient {}\n",
        );

        let graph = build(&dir, &[order, db_client]).unwrap();

        assert!(graph.nodes.contains("com/example/domain"));
        assert!(graph.nodes.contains("com/example/infra"));
        assert!(!graph.nodes.contains("org/springframework/stereotype"));
        assert!(
            graph
                .edges
                .iter()
                .any(|e| e.from == "com/example/domain" && e.to == "com/example/infra")
        );
        assert!(
            !graph
                .edges
                .iter()
                .any(|e| e.to == "org/springframework/stereotype")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Pre-mortem.md P1 #3(ii): every import in a multi-import file is extracted, not
    /// just the first or last — 2 local imports (each producing an edge with the
    /// correct target and line number) and 2 external imports (excluded), never
    /// duplicated or misattributed.
    #[test]
    fn java_import_graph_extracts_every_import_in_a_multi_import_file() {
        let dir = tmp_dir("java-multi-import");
        let order = write(
            &dir,
            "src/main/java/com/example/domain/Order.java",
            "package com.example.domain;\n\nimport com.example.infra.DbClient;\nimport com.example.util.Helper;\nimport org.springframework.stereotype.Component;\nimport java.util.List;\n\npublic class Order {}\n",
        );
        let db_client = write(
            &dir,
            "src/main/java/com/example/infra/DbClient.java",
            "package com.example.infra;\n\npublic class DbClient {}\n",
        );
        let helper = write(
            &dir,
            "src/main/java/com/example/util/Helper.java",
            "package com.example.util;\n\npublic class Helper {}\n",
        );

        let graph = build(&dir, &[order, db_client, helper]).unwrap();

        let from_domain: Vec<_> = graph
            .edges
            .iter()
            .filter(|e| e.from == "com/example/domain")
            .collect();
        assert_eq!(
            from_domain.len(),
            2,
            "expected exactly 2 edges, got {from_domain:?}"
        );
        assert!(
            from_domain
                .iter()
                .any(|e| e.to == "com/example/infra" && e.line == 3)
        );
        assert!(
            from_domain
                .iter()
                .any(|e| e.to == "com/example/util" && e.line == 4)
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Real, verified outcome (checked via `to_sexp()`/`has_error()` during
    /// development, not assumed): a Java import statement missing its trailing `;`
    /// recovers with a `(MISSING ";")` node **inserted in place** — the same
    /// per-node-repair style Story 4.1.2 already documented for Kotlin's unclosed
    /// parameter list, and notably *not* the flat `ERROR`-wrapper style
    /// `java_malformed_source_recovery_shape` pinned for a broken class-body field.
    /// Because the surrounding `import_declaration` stays structurally intact, its
    /// `scoped_identifier` child is still cleanly extractable — so this fixture lands
    /// on neither acceptance-criterion outcome (a) (only the well-formed edge) nor (b)
    /// (empty): tree-sitter's per-import repair is good enough that **both** imports
    /// extract correctly. That still satisfies the acceptance criterion's actual
    /// invariant — never a truncated/mismatched target or a line misattributed to the
    /// wrong import — which this test asserts directly on both edges.
    #[test]
    fn java_import_graph_malformed_source_never_extracts_a_wrong_edge() {
        let dir = tmp_dir("java-malformed");
        let order = write(
            &dir,
            "src/main/java/com/example/domain/Order.java",
            "package com.example.domain;\n\nimport com.example.infra.DbClient;\nimport com.example.infra.Helper\n\npublic class Order {}\n",
        );
        let db_client = write(
            &dir,
            "src/main/java/com/example/infra/DbClient.java",
            "package com.example.infra;\n\npublic class DbClient {}\n",
        );

        let graph = build(&dir, &[order, db_client]).unwrap();

        let from_domain: Vec<_> = graph
            .edges
            .iter()
            .filter(|e| e.from == "com/example/domain")
            .collect();
        assert_eq!(
            from_domain.len(),
            2,
            "expected both imports' edges, got {from_domain:?}"
        );
        assert!(
            from_domain.iter().all(|e| e.to == "com/example/infra"),
            "every edge target must be the real package, never truncated/mismatched: {from_domain:?}"
        );
        assert!(
            from_domain.iter().any(|e| e.line == 3),
            "well-formed import's edge must keep its own line: {from_domain:?}"
        );
        assert!(
            from_domain.iter().any(|e| e.line == 4),
            "malformed import's edge, if produced, must carry its own line, not the well-formed import's: {from_domain:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Pre-mortem.md P1 #3(ii), Kotlin: reuses Story 4.1.2's verified malformed-source
    /// fixture (unclosed parameter list) — the well-formed leading `import` still
    /// parses cleanly outside the `MISSING ")"` recovery, so it extracts the correct
    /// edge and nothing else.
    #[test]
    fn kotlin_import_graph_malformed_source_never_extracts_a_wrong_edge() {
        let dir = tmp_dir("kotlin-malformed");
        let order = write(
            &dir,
            "domain/Order.kt",
            "package com.example.domain\n\nimport com.example.infra.DbClient\n\nclass Order(val id: String\n",
        );
        let db_client = write(
            &dir,
            "infra/DbClient.kt",
            "package com.example.infra\n\nclass DbClient\n",
        );

        let graph = build(&dir, &[order, db_client]).unwrap();

        let from_domain: Vec<_> = graph
            .edges
            .iter()
            .filter(|e| e.from == "com/example/domain")
            .collect();
        assert_eq!(
            from_domain.len(),
            1,
            "expected exactly the well-formed edge, got {from_domain:?}"
        );
        assert_eq!(from_domain[0].to, "com/example/infra");
        assert_eq!(from_domain[0].line, 3);

        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- Epic 5.1 / Story 5.1.1: Python to_sexp() verification ---

    fn parse_python(src: &str) -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .expect("loading tree-sitter-python grammar");
        parser.parse(src, None).expect("parsing Python fixture")
    }

    /// Task 5.1.1a — the exact fixture from plan.md's Story 5.1.1 acceptance criteria,
    /// covering all six import node kinds. Real `to_sexp()` output pinned per the doc
    /// comment above `dir_key`.
    #[test]
    fn python_import_to_sexp_fixture() {
        let src = "import os\nfrom app.infra import db_client\nfrom app.infra import db_client as db\nfrom . import sibling\nfrom ..domain import Order\nfrom app.infra import *\nfrom __future__ import annotations\n";
        let tree = parse_python(src);
        let sexp = tree.root_node().to_sexp();

        assert!(!tree.root_node().has_error());
        // import_statement (plain)
        assert!(sexp.contains("(import_statement name: (dotted_name (identifier)))"));
        // import_from_statement (plain from-import)
        assert!(sexp.contains(
            "(import_from_statement module_name: (dotted_name (identifier) (identifier)) name: (dotted_name (identifier)))"
        ));
        // aliased_import, nested inside import_from_statement's name field
        assert!(sexp.contains(
            "(import_from_statement module_name: (dotted_name (identifier) (identifier)) name: (aliased_import name: (dotted_name (identifier)) alias: (identifier)))"
        ));
        // relative_import with import_prefix only (no dotted_name — "from . import x")
        assert!(sexp.contains(
            "(import_from_statement module_name: (relative_import (import_prefix)) name: (dotted_name (identifier)))"
        ));
        // relative_import with import_prefix + dotted_name ("from ..domain import Order")
        assert!(sexp.contains(
            "(import_from_statement module_name: (relative_import (import_prefix) (dotted_name (identifier))) name: (dotted_name (identifier)))"
        ));
        // wildcard_import — unfielded, no `name` field present
        assert!(sexp.contains(
            "(import_from_statement module_name: (dotted_name (identifier) (identifier)) (wildcard_import))"
        ));
        // future_import_statement — distinct top-level node kind, no module_name field
        assert!(sexp.contains("(future_import_statement name: (dotted_name (identifier)))"));
    }

    /// Task 5.1.1b — the top-level aliased-import form (`import x as y`, as opposed to
    /// plan.md's `from x import y as z`): same `import_statement` node kind as a plain
    /// import, with the `name` field's child being `aliased_import` instead of
    /// `dotted_name`. Confirms `aliased_import` is never a standalone node — it always
    /// nests inside another import node's `name` field.
    #[test]
    fn python_top_level_aliased_import_shape() {
        let tree = parse_python("import os as o\n");
        let import_node = tree.root_node().named_child(0).unwrap();

        assert_eq!(import_node.kind(), "import_statement");
        let name_field = import_node.child_by_field_name("name").unwrap();
        assert_eq!(name_field.kind(), "aliased_import");
        assert_eq!(
            name_field
                .child_by_field_name("alias")
                .unwrap()
                .utf8_text("import os as o\n".as_bytes())
                .unwrap(),
            "o"
        );
    }

    /// Task 5.1.1b (continued) — dot count for a relative import is not encoded in
    /// node kind or `to_sexp()` shape at all; `import_prefix`'s raw source text is
    /// exactly N `.` characters, with or without a trailing package name. Grounds
    /// Story 5.2.1b's "count leading dots" resolution in verified real output.
    #[test]
    fn python_relative_import_prefix_dot_count_is_in_source_text() {
        for (src, expected_dots, has_dotted_name) in [
            ("from . import sibling\n", ".", false),
            ("from ..domain import Order\n", "..", true),
            ("from ... import x\n", "...", false),
        ] {
            let tree = parse_python(src);
            let import_from = tree.root_node().named_child(0).unwrap();
            assert_eq!(import_from.kind(), "import_from_statement");
            let module_name = import_from.child_by_field_name("module_name").unwrap();
            assert_eq!(module_name.kind(), "relative_import");

            let prefix = module_name.named_child(0).unwrap();
            assert_eq!(prefix.kind(), "import_prefix");
            assert_eq!(prefix.utf8_text(src.as_bytes()).unwrap(), expected_dots);

            assert_eq!(
                module_name.named_child_count(),
                if has_dotted_name { 2 } else { 1 },
                "relative_import's second named child (dotted_name) is present only when a package follows the dots: {src:?}"
            );
        }
    }

    /// Task 5.1.1a (malformed source) — a truncated `def` after a well-formed `import`
    /// recovers as a flat `ERROR` node wrapping just the malformed tail; the leading
    /// `import` still parses cleanly outside it. Same "ERROR wraps only the bad part"
    /// recovery shape as Java's `java_malformed_source_recovery_shape` above (contrast
    /// with Kotlin's in-place `MISSING` node style).
    #[test]
    fn python_malformed_source_recovery_shape() {
        let src = "import os\n\ndef broken(\n";
        let tree = parse_python(src);

        assert!(tree.root_node().has_error());
        let sexp = tree.root_node().to_sexp();
        assert!(sexp.contains("(ERROR (identifier))"));
        assert!(sexp.starts_with("(module (import_statement name: (dotted_name (identifier)))"));
    }

    // --- Epic 5.2 / Story 5.2.1: Python import extraction ---

    /// Story 5.2.1's first acceptance criterion: a relative import one level up into a
    /// sibling package (`from ..infra import db_client`) produces an edge from the
    /// importing file's own package to the resolved sibling — the non-trivial case,
    /// since same-directory relative imports produce no cross-node edge at all (same
    /// "skip same-dir" convention as `build_js`'s `to_dir != from_dir` check).
    #[test]
    fn python_relative_import_resolves_to_sibling_package() {
        let dir = tmp_dir("python-relative");
        let app_init = write(&dir, "app/__init__.py", "");
        let domain_init = write(&dir, "app/domain/__init__.py", "");
        let validator = write(
            &dir,
            "app/domain/validator.py",
            "from ..infra import db_client\n",
        );
        let infra_init = write(&dir, "app/infra/__init__.py", "");

        let graph = build(&dir, &[app_init, domain_init, validator, infra_init]).unwrap();

        assert!(graph.nodes.contains("app/domain"));
        assert!(graph.nodes.contains("app/infra"));
        assert!(
            graph
                .edges
                .iter()
                .any(|e| e.from == "app/domain" && e.to == "app/infra")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Story 5.2.1's second acceptance criterion: an absolute import
    /// (`from app.infra import db_client`) resolves via the `__init__.py`-rooted
    /// heuristic, normalized dot-to-slash the same way Java/Kotlin's qualified names
    /// are, so the resulting node key matches `app/domain`'s directory-based identity.
    #[test]
    fn python_absolute_import_resolves_via_init_py_heuristic() {
        let dir = tmp_dir("python-absolute");
        let app_init = write(&dir, "app/__init__.py", "");
        let domain_init = write(&dir, "app/domain/__init__.py", "");
        let order = write(
            &dir,
            "app/domain/order.py",
            "from app.infra import db_client\n",
        );
        let infra_init = write(&dir, "app/infra/__init__.py", "");

        let graph = build(&dir, &[app_init, domain_init, order, infra_init]).unwrap();

        assert!(graph.nodes.contains("app/domain"));
        assert!(graph.nodes.contains("app/infra"));
        assert!(
            graph
                .edges
                .iter()
                .any(|e| e.from == "app/domain" && e.to == "app/infra")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Story 5.2.1's third acceptance criterion: `from __future__ import annotations`
    /// produces zero graph edges and no error — regression-guards Story 5.1.1's
    /// finding that a naive walk matching only `import_from_statement` would miss
    /// `future_import_statement` entirely (here, `collect_python_imports` excludes it
    /// explicitly, so this is proven rather than accidental).
    #[test]
    fn python_future_import_produces_no_edge() {
        let dir = tmp_dir("python-future");
        let pkg_init = write(&dir, "pkg/__init__.py", "");
        let mod_file = write(&dir, "pkg/mod.py", "from __future__ import annotations\n");

        let graph = build(&dir, &[pkg_init, mod_file]).unwrap();

        assert!(graph.edges.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Judgment call beyond plan.md's 3 named tests, locking in the doc comment's
    /// verified finding above `collect_python_imports`: a wildcard import
    /// (`from app.infra import *`) still resolves via its `module_name` field alone,
    /// exactly like any other `import_from_statement` — it must not be silently
    /// skipped alongside `future_import_statement` just because its `name` field is
    /// absent.
    #[test]
    fn python_wildcard_import_resolves_via_module_name() {
        let dir = tmp_dir("python-wildcard");
        let app_init = write(&dir, "app/__init__.py", "");
        let domain_init = write(&dir, "app/domain/__init__.py", "");
        let order = write(&dir, "app/domain/order.py", "from app.infra import *\n");
        let infra_init = write(&dir, "app/infra/__init__.py", "");

        let graph = build(&dir, &[app_init, domain_init, order, infra_init]).unwrap();

        assert!(
            graph
                .edges
                .iter()
                .any(|e| e.from == "app/domain" && e.to == "app/infra")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Judgment call beyond plan.md's 3 named tests, mirroring the Java/Go graph-
    /// membership-guard tests already in this file (pre-mortem.md P1 #3(i)): an
    /// absolute import of a package no file in this run declared (no `__init__.py`
    /// registered it) must never become a graph node or edge, even though its dotted
    /// path is syntactically well-formed and superficially plausible.
    #[test]
    fn python_import_of_external_package_creates_no_edge() {
        let dir = tmp_dir("python-external");
        let app_init = write(&dir, "app/__init__.py", "");
        let domain_init = write(&dir, "app/domain/__init__.py", "");
        let order = write(
            &dir,
            "app/domain/order.py",
            "from django.db import models\n",
        );

        let graph = build(&dir, &[app_init, domain_init, order]).unwrap();

        assert!(!graph.nodes.contains("django/db"));
        assert!(graph.edges.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- Rust ---

    #[test]
    fn rust_import_graph_finds_a_two_module_cycle_via_crate_paths() {
        let dir = tmp_dir("rust-cycle");
        write(&dir, "Cargo.toml", "[package]\nname = \"app\"\n");
        let a = write(
            &dir,
            "src/a.rs",
            "use crate::b;\n\npub fn f() { b::g(); }\n",
        );
        let b = write(
            &dir,
            "src/b.rs",
            "use crate::a;\n\npub fn g() { a::f(); }\n",
        );
        write(&dir, "src/lib.rs", "mod a;\nmod b;\n");

        let graph = build(&dir, &[a, b]).unwrap();

        assert!(graph.nodes.contains("app::a"));
        assert!(graph.nodes.contains("app::b"));
        assert!(
            graph
                .edges
                .iter()
                .any(|e| e.from == "app::a" && e.to == "app::b")
        );
        assert!(
            graph
                .edges
                .iter()
                .any(|e| e.from == "app::b" && e.to == "app::a")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rust_nested_module_directory_resolves_to_a_qualified_key() {
        let dir = tmp_dir("rust-nested");
        write(&dir, "Cargo.toml", "[package]\nname = \"app\"\n");
        let node = write(&dir, "src/dom/node.rs", "pub struct Node;\n");

        let graph = build(&dir, std::slice::from_ref(&node)).unwrap();

        assert_eq!(
            graph.file_packages.get(&node),
            Some(&"app::dom::node".to_string())
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rust_mod_rs_collapses_into_its_directory_key() {
        let dir = tmp_dir("rust-mod-rs");
        write(&dir, "Cargo.toml", "[package]\nname = \"app\"\n");
        let mod_file = write(&dir, "src/dom/mod.rs", "pub struct Node;\n");

        let graph = build(&dir, std::slice::from_ref(&mod_file)).unwrap();

        assert_eq!(
            graph.file_packages.get(&mod_file),
            Some(&"app::dom".to_string())
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rust_super_resolves_to_a_sibling_module_via_the_importing_files_parent() {
        let dir = tmp_dir("rust-super-sibling");
        write(&dir, "Cargo.toml", "[package]\nname = \"app\"\n");
        // `dom::node` reaches its sibling `dom::element` via `super::element`: `super`
        // pops one level (`dom::node` -> `dom`), then `element` is looked up there.
        // (`self::element` would instead mean "a child of `dom::node` itself," which
        // is a different module — not what this fixture is testing.)
        let node = write(
            &dir,
            "src/dom/node.rs",
            "use super::element;\n\npub fn f() { element::g(); }\n",
        );
        let element = write(&dir, "src/dom/element.rs", "pub fn g() {}\n");
        write(&dir, "src/lib.rs", "mod dom;\n");

        let graph = build(&dir, &[node, element]).unwrap();

        assert!(
            graph
                .edges
                .iter()
                .any(|e| e.from == "app::dom::node" && e.to == "app::dom::element"),
            "edges: {:?}",
            graph.edges
        );
    }

    #[test]
    fn rust_super_resolves_to_the_crate_root_from_a_top_level_module() {
        let dir = tmp_dir("rust-super-root");
        write(&dir, "Cargo.toml", "[package]\nname = \"app\"\n");
        // A top-level module (`utils`, one level deep already) reaches the crate root
        // itself via a single `super::`.
        let utils = write(
            &dir,
            "src/utils.rs",
            "use super::other;\n\npub fn f() { other::g(); }\n",
        );
        let other = write(&dir, "src/other.rs", "pub fn g() {}\n");
        write(&dir, "src/lib.rs", "mod utils;\nmod other;\n");

        let graph = build(&dir, &[utils, other]).unwrap();

        assert!(
            graph
                .edges
                .iter()
                .any(|e| e.from == "app::utils" && e.to == "app::other"),
            "edges: {:?}",
            graph.edges
        );
    }

    #[test]
    fn rust_use_list_bare_self_item_resolves_to_the_prefix_module_itself() {
        let dir = tmp_dir("rust-use-self");
        write(&dir, "Cargo.toml", "[package]\nname = \"app\"\n");
        // `use crate::utils::{self, helper_fn};` — `self` here means "the `utils`
        // module itself," a different meaning from `self::` at the head of a path
        // (covered by the two `super` tests above via their non-conflicting cases).
        let consumer = write(
            &dir,
            "src/consumer.rs",
            "use crate::utils::{self, helper_fn};\n",
        );
        let utils = write(&dir, "src/utils.rs", "pub fn helper_fn() {}\n");
        write(&dir, "src/lib.rs", "mod consumer;\nmod utils;\n");

        let graph = build(&dir, &[consumer, utils]).unwrap();

        assert!(
            graph
                .edges
                .iter()
                .any(|e| e.from == "app::consumer" && e.to == "app::utils"),
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rust_scoped_use_list_and_wildcard_both_resolve() {
        let dir = tmp_dir("rust-use-list");
        write(&dir, "Cargo.toml", "[package]\nname = \"app\"\n");
        let consumer = write(
            &dir,
            "src/consumer.rs",
            "use crate::{utils::helper, other};\nuse crate::wild::*;\n",
        );
        let utils = write(&dir, "src/utils.rs", "pub mod helper {}\n");
        let other = write(&dir, "src/other.rs", "pub fn f() {}\n");
        let wild = write(&dir, "src/wild.rs", "pub fn g() {}\n");
        write(
            &dir,
            "src/lib.rs",
            "mod consumer;\nmod utils;\nmod other;\nmod wild;\n",
        );

        let graph = build(&dir, &[consumer, utils, other, wild]).unwrap();

        // `utils::helper` doesn't exist as its own file/module in this fixture, so it
        // falls back to the parent `utils` module per `resolve_rust_use_path`'s
        // module-vs-item fallback.
        assert!(
            graph
                .edges
                .iter()
                .any(|e| e.from == "app::consumer" && e.to == "app::utils"),
            "edges: {:?}",
            graph.edges
        );
        assert!(
            graph
                .edges
                .iter()
                .any(|e| e.from == "app::consumer" && e.to == "app::other")
        );
        assert!(
            graph
                .edges
                .iter()
                .any(|e| e.from == "app::consumer" && e.to == "app::wild")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rust_use_as_clause_resolves_by_its_unaliased_path() {
        let dir = tmp_dir("rust-use-as");
        write(&dir, "Cargo.toml", "[package]\nname = \"app\"\n");
        let consumer = write(&dir, "src/consumer.rs", "use crate::other as renamed;\n");
        let other = write(&dir, "src/other.rs", "pub fn f() {}\n");
        write(&dir, "src/lib.rs", "mod consumer;\nmod other;\n");

        let graph = build(&dir, &[consumer, other]).unwrap();

        assert!(
            graph
                .edges
                .iter()
                .any(|e| e.from == "app::consumer" && e.to == "app::other")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rust_external_crate_import_creates_no_edge() {
        let dir = tmp_dir("rust-external");
        write(&dir, "Cargo.toml", "[package]\nname = \"app\"\n");
        let a = write(
            &dir,
            "src/lib.rs",
            "use std::collections::HashMap;\nuse serde::Serialize;\n\npub fn f() -> HashMap<String, String> { HashMap::new() }\n",
        );

        let graph = build(&dir, &[a]).unwrap();

        assert!(graph.edges.is_empty(), "edges: {:?}", graph.edges);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rust_multi_crate_workspace_does_not_collide_on_same_named_modules() {
        let dir = tmp_dir("rust-workspace");
        write(
            &dir,
            "Cargo.toml",
            "[workspace]\nmembers = [\"a\", \"b\"]\n",
        );
        write(&dir, "a/Cargo.toml", "[package]\nname = \"a\"\n");
        write(&dir, "b/Cargo.toml", "[package]\nname = \"b\"\n");
        let a_utils = write(&dir, "a/src/utils.rs", "pub fn f() {}\n");
        let b_utils = write(&dir, "b/src/utils.rs", "pub fn g() {}\n");
        let a_lib = write(&dir, "a/src/lib.rs", "mod utils;\n");
        let b_lib = write(&dir, "b/src/lib.rs", "mod utils;\n");

        let graph = build(&dir, &[a_utils, b_utils, a_lib, b_lib]).unwrap();

        assert!(graph.nodes.contains("a::utils"));
        assert!(graph.nodes.contains("b::utils"));
        assert_ne!(
            graph.nodes.iter().filter(|n| n.ends_with("utils")).count(),
            1,
            "both crates' utils modules must get distinct, crate-scoped keys"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
