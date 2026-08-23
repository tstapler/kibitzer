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
/// to). Go, TypeScript/JavaScript, Java, and Kotlin are extracted; Python import
/// extraction can follow the same per-language dispatch pattern later (it does not fit
/// the Go/Java/Kotlin "qualified name" family — see `build_qualified_name_language`'s
/// doc comment).
pub fn build(repo_root: &Path, files: &[PathBuf]) -> Result<ImportGraph> {
    let mut graph = ImportGraph::default();

    let go_files: Vec<&PathBuf> = files.iter().filter(|f| has_ext(f, "go")).collect();
    if !go_files.is_empty() {
        build_qualified_name_language(repo_root, &go_files, &mut graph, &go_lang_config())?;
    }

    let js_files: Vec<&PathBuf> = files.iter().filter(|f| is_js_like(f)).collect();
    if !js_files.is_empty() {
        build_js(&js_files, &mut graph)?;
    }

    let java_files: Vec<&PathBuf> = files.iter().filter(|f| has_ext(f, "java")).collect();
    if !java_files.is_empty() {
        build_qualified_name_language(repo_root, &java_files, &mut graph, &java_lang_config())?;
    }

    let kotlin_files: Vec<&PathBuf> = files
        .iter()
        .filter(|f| has_ext(f, "kt") || has_ext(f, "kts"))
        .collect();
    if !kotlin_files.is_empty() {
        build_qualified_name_language(repo_root, &kotlin_files, &mut graph, &kotlin_lang_config())?;
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
    /// entirely (no node, no edges) — e.g. Go outside a resolvable module.
    package_identity:
        fn(repo_root: &Path, file: &Path, tree: &tree_sitter::Tree, src: &[u8]) -> Option<String>,
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

        if let Some(pkg) = (cfg.package_identity)(repo_root, file, &tree, source.as_bytes()) {
            graph.nodes.insert(pkg.clone());
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

/// Go's package identity is **not** derived from parsing the file's `package_clause`
/// node — that node names only the local package name (e.g. `a`), not the full import
/// path other files reference it by (e.g. `example.com/app/a`). It comes from
/// `go.mod`'s `module` directive plus the file's repo-relative directory, exactly as
/// pre-refactor `build_go` computed it. `tree`/`_src` are accepted-but-unused only to
/// satisfy `QualifiedImportLangConfig::package_identity`'s shared signature.
fn go_package_identity(
    repo_root: &Path,
    file: &Path,
    _tree: &tree_sitter::Tree,
    _src: &[u8],
) -> Option<String> {
    let module_path = go_module_path(repo_root)?;
    go_package_import_path(&module_path, repo_root, file)
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
}
