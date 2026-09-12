//! Domain types for the shared, repo-scoped architecture model — `ArchModel` and its
//! constituent types — plus this crate's I/O-owning helper layer for building one:
//! `collect_repo_files` (walk + import graph + file reads) and `load_cached_model`
//! (cache lookup, falling back to `collect_repo_files` + `build_model`). Every consumer
//! (CLI `architecture export`/`diagram`, MCP query tools, LSP symbol mapper/index) shares
//! these instead of reimplementing walk-and-parse itself.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::checker::{GrammarCache, Language};
use crate::import_graph::{ImportEdge, ImportGraph};
use crate::symbol_extract::{RawCallSite, extract_call_sites_for_file, extract_symbols_for_file};

/// The kind of a single extracted symbol. Exhaustively matched by every consumer (the
/// diagram renderer, the LSP `SymbolKind` mapper, the MCP `kind` filter) so a new kind
/// can't silently be missed by a stringly-typed comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SymbolKind {
    Type,
    Interface,
    Function,
    Method,
}

/// How deep a consumer wants to look: package/component granularity, or down to
/// individual symbols. Threaded through every consumer's "how deep" parameter (CLI
/// `--level`, MCP `level` field, diagram renderer) instead of each interface inventing
/// its own depth flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelLevel {
    Component,
    Code,
}

/// One extracted type/interface/function/method declaration.
///
/// `id` is deterministic and owner-qualified (`"{package_path}::{parent}.{name}"` for
/// methods, else `"{package_path}::{name}"`) so it's re-derivable across runs without a
/// lookup table, and so two same-named methods on different types in one package don't
/// collide.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SymbolNode {
    pub id: String,
    pub name: String,
    pub kind: SymbolKind,
    pub file: PathBuf,
    pub line: usize,
    pub exported: bool,
    /// Set for methods: the name of the owning type. `None` for types, interfaces, and
    /// free functions.
    pub parent: Option<String>,
}

/// One package/module-directory node in `ArchModel` — a path (matching `ImportGraph`'s
/// node key), the files under it, and its `SymbolNode`s.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackageNode {
    pub path: String,
    pub files: Vec<PathBuf>,
    pub symbols: Vec<SymbolNode>,
}

impl PackageNode {
    /// Robert Martin's Abstractness metric: the fraction of this package's
    /// type-level (`Type`/`Interface`) symbols that are `Interface`. `Function`/`Method`
    /// symbols aren't counted — abstractness is about the type surface, not free
    /// functions. A package with no `Type`/`Interface` symbols at all is treated as fully
    /// concrete (`0.0`) rather than undefined, so callers never have to special-case it.
    pub fn abstractness(&self) -> f64 {
        let mut interfaces = 0usize;
        let mut total = 0usize;
        for symbol in &self.symbols {
            match symbol.kind {
                SymbolKind::Interface => {
                    interfaces += 1;
                    total += 1;
                }
                SymbolKind::Type => total += 1,
                SymbolKind::Function | SymbolKind::Method => {}
            }
        }
        if total == 0 {
            0.0
        } else {
            interfaces as f64 / total as f64
        }
    }
}

/// What a `build_model` run excluded and why, embedded in `ArchModel` so a consumer
/// never mistakes "pruned" for "doesn't exist," and never mistakes "no supported
/// language in this file" for "no code here."
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PruningSummary {
    pub include_private: bool,
    pub excluded_dirs: Vec<String>,
    pub generated_files_skipped: usize,
    pub private_symbols_skipped: usize,
    /// The `SymbolNode::id` of every symbol excluded specifically by
    /// `include_private: false` (the ids counted in `private_symbols_skipped`).
    pub pruned_symbol_ids: Vec<String>,
    /// Files that were attempted but didn't parse cleanly — a path list (not just a
    /// count) so a consumer can identify *which* files failed.
    pub files_with_parse_errors: Vec<PathBuf>,
    /// Files under scope with no recognized extension at all — distinct from a parse
    /// error, since the file was never even attempted.
    pub unsupported_language_files: usize,
    /// Denominator so a consumer can compute the unsupported fraction without a second
    /// pass over the repo.
    pub total_files_scanned: usize,
}

/// One call edge. `to` is a `SymbolNode::id` when `resolved`; otherwise it's the call
/// site's raw, unresolved callee text (e.g. `"pkg.Qux"`) — never silently dropped, so a
/// consumer can still see what the call site named (see `resolve_call_edges`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CallEdge {
    pub from: String,
    pub to: String,
    pub resolved: bool,
    pub file: PathBuf,
    pub line: usize,
}

/// The shared, repo-scoped architecture model: packages (keyed by path), the import
/// edges between them, and the function/method call edges within them. The one model all
/// consumer interfaces (CLI export, MCP query, LSP symbols, diagram) read from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArchModel {
    pub repo_root: PathBuf,
    pub packages: BTreeMap<String, PackageNode>,
    pub import_edges: Vec<ImportEdge>,
    pub call_edges: Vec<CallEdge>,
    pub pruning: PruningSummary,
}

// ---------------------------------------------------------------------------------
// Epic 1.3: build_model orchestration, pruning, and the query API
// ---------------------------------------------------------------------------------

/// Input to `build_model`: pruning knobs only. Distinct from `PruningSummary`, which is
/// the *output* record of what was excluded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PruneConfig {
    pub include_private: bool,
}

/// A file whose first two lines (case-insensitive) mention "do not edit" or "code
/// generated" is treated as machine-generated and skipped entirely by `build_model` —
/// matching common generator banners (e.g. `// Code generated by protoc-gen-go. DO NOT
/// EDIT.`).
pub fn looks_generated(source: &str) -> bool {
    source.lines().take(2).any(|line| {
        let lower = line.to_lowercase();
        lower.contains("do not edit") || lower.contains("code generated")
    })
}

/// Maps a file extension to the `Language` `build_model` should parse it with —
/// `Language::for_path` is the single source of truth (see its doc comment for why: a
/// hand-rolled copy of this exact match here once silently missed `Rust`). Files with
/// no recognized extension return `None` and are counted as `unsupported_language_files`,
/// not silently dropped.
pub(crate) fn language_for_path(path: &Path) -> Option<Language> {
    Language::for_path(path)
}

/// The package/directory key a file groups under. `ImportGraph` is the single source of
/// truth for this mapping (`import_graph.file_packages`), since it already computes the
/// per-language key — Go's module-qualified import path (from `go.mod`), JS/TS's
/// repo-relative directory — while walking each file for import extraction. Consulting it
/// here (rather than re-deriving the key from the path alone) is what keeps
/// `PackageNode::path` equal to `ImportEdge::from`/`to` for a package that has edges.
///
/// Falls back to a plain repo-root-relative directory path only for files `import_graph`
/// didn't map — i.e. languages `import_graph.rs` doesn't extract imports for yet. Today
/// every such file is already uncounted-for-symbols too (`symbol_extract.rs` doesn't cover
/// them either), so this fallback is unreachable in practice, but it's kept so a file isn't
/// silently dropped from grouping if `import_graph.rs` support lags future
/// `language_for_path` additions.
fn package_key_for_file(repo_root: &Path, file: &Path, import_graph: &ImportGraph) -> String {
    if let Some(key) = import_graph.file_packages.get(file) {
        return key.clone();
    }
    let dir = file.parent().unwrap_or(Path::new("."));
    let rel = dir.strip_prefix(repo_root).unwrap_or(dir);
    let s = rel.to_string_lossy().replace('\\', "/");
    if s.is_empty() { ".".to_string() } else { s }
}

/// Assembles a pruned `ArchModel` from already-read `(path, source)` pairs and an
/// already-built `ImportGraph`. No file walking, no disk reads, no config-file reading —
/// the caller (CLI `arch_export.rs`, or a `ModelCache` build closure in `mcp.rs`/`lsp.rs`)
/// collects files and reads their source before calling this.
///
/// `file`'s call sites with `file` filled in — split out of `build_model`'s per-file loop
/// purely to keep that loop short; resolution against the whole-repo symbol index still
/// happens later, in `resolve_call_edges`.
fn collect_call_sites(
    language: Language,
    source: &str,
    tree: &tree_sitter::Tree,
    package_path: &str,
    file: &Path,
) -> Vec<RawCallSite> {
    extract_call_sites_for_file(language, source, tree, package_path)
        .into_iter()
        .map(|site| RawCallSite {
            file: file.to_path_buf(),
            ..site
        })
        .collect()
}

/// Groups `files` by `package_key_for_file`, skipping files with no recognized `Language`
/// (counted in `PruningSummary.unsupported_language_files` rather than silently dropped).
/// For each recognized file: generated files are skipped whole (`generated_files_skipped`
/// increments, no entry in that package's `files`); otherwise the file is parsed with a
/// fresh `GrammarCache` (one per file, matching `GrammarCache`'s one-cache-per-file
/// contract) and, if the parse tree has any error node, the whole file is skipped for
/// extraction (`files_with_parse_errors` records its path) rather than partially
/// extracted. Clean files are extracted via `extract_symbols_for_file`, then pruned by
/// `PruneConfig.include_private`. Call sites (`collect_call_sites`) are gathered from
/// every parsed file regardless of pruning; `resolve_call_edges` resolves them afterward
/// against the already-pruned `packages` map.
pub fn build_model(
    repo_root: &Path,
    files: &[(PathBuf, String)],
    import_graph: &ImportGraph,
    prune: &PruneConfig,
) -> Result<ArchModel> {
    let mut packages: BTreeMap<String, PackageNode> = BTreeMap::new();
    let mut generated_files_skipped = 0usize;
    let mut private_symbols_skipped = 0usize;
    let mut pruned_symbol_ids: Vec<String> = Vec::new();
    let mut files_with_parse_errors: Vec<PathBuf> = Vec::new();
    let mut unsupported_language_files = 0usize;
    let total_files_scanned = files.len();
    let mut raw_call_sites: Vec<RawCallSite> = Vec::new();

    for (path, source) in files {
        let Some(language) = language_for_path(path) else {
            unsupported_language_files += 1;
            continue;
        };

        if looks_generated(source) {
            generated_files_skipped += 1;
            continue;
        }

        let cache = GrammarCache::new();
        let tree = cache.parse(language, source)?;
        if tree.root_node().has_error() {
            files_with_parse_errors.push(path.clone());
            continue;
        }

        let package_path = package_key_for_file(repo_root, path, import_graph);
        let package = packages
            .entry(package_path.clone())
            .or_insert_with(|| PackageNode {
                path: package_path.clone(),
                files: Vec::new(),
                symbols: Vec::new(),
            });
        package.files.push(path.clone());

        let symbols = extract_symbols_for_file(language, source, &tree, &package_path);
        for symbol in symbols {
            if !prune.include_private && !symbol.exported {
                private_symbols_skipped += 1;
                pruned_symbol_ids.push(symbol.id);
                continue;
            }
            package.symbols.push(SymbolNode {
                file: path.clone(),
                ..symbol
            });
        }

        raw_call_sites.extend(collect_call_sites(
            language,
            source,
            &tree,
            &package_path,
            path,
        ));
    }

    let call_edges = resolve_call_edges(&packages, raw_call_sites);

    Ok(ArchModel {
        repo_root: repo_root.to_path_buf(),
        packages,
        import_edges: import_graph.edges.clone(),
        call_edges,
        pruning: PruningSummary {
            include_private: prune.include_private,
            excluded_dirs: vec![],
            generated_files_skipped,
            private_symbols_skipped,
            pruned_symbol_ids,
            files_with_parse_errors,
            unsupported_language_files,
            total_files_scanned,
        },
    })
}

/// The package half of `caller_id`'s `"{package_path}::{name}"` / `"{package_path}::
/// {parent}.{name}"` shape (`symbol_extract::build_id`'s own format) — used to prefer a
/// same-package resolution match over a global one.
fn caller_package(caller_id: &str) -> &str {
    caller_id.split_once("::").map_or("", |(pkg, _)| pkg)
}

/// Looks up `name` in a `SymbolNode.name -> [(package, id)]` index, preferring an
/// unambiguous same-package match (a call almost always targets a sibling in its own
/// package) and falling back to a globally unique match. `None` when `name` isn't in the
/// index at all, or when it's ambiguous at both tiers — resolution intentionally never
/// guesses among multiple candidates.
fn resolve_in<'a>(
    index: &HashMap<&str, Vec<(&'a str, &'a str)>>,
    caller_pkg: &str,
    name: &str,
) -> Option<&'a str> {
    let candidates = index.get(name)?;
    let same_package: Vec<&(&str, &str)> = candidates
        .iter()
        .filter(|(pkg, _)| *pkg == caller_pkg)
        .collect();
    if same_package.len() == 1 {
        return Some(same_package[0].1);
    }
    if candidates.len() == 1 {
        return Some(candidates[0].1);
    }
    None
}

type SymbolIndex<'a> = HashMap<&'a str, Vec<(&'a str, &'a str)>>;

/// Builds the `SymbolNode.name -> [(package, id)]` indexes `resolve_one_call_edge` looks
/// callee names up in, split by kind since a bare call resolves against `Function`s and a
/// qualified one against `Method`s first (see `resolve_one_call_edge`).
fn build_call_target_indexes(
    packages: &BTreeMap<String, PackageNode>,
) -> (SymbolIndex<'_>, SymbolIndex<'_>) {
    let mut functions_by_name: SymbolIndex = HashMap::new();
    let mut methods_by_name: SymbolIndex = HashMap::new();

    for (pkg_path, pkg) in packages {
        for sym in &pkg.symbols {
            let bucket = match sym.kind {
                SymbolKind::Function => &mut functions_by_name,
                SymbolKind::Method => &mut methods_by_name,
                SymbolKind::Type | SymbolKind::Interface => continue,
            };
            bucket
                .entry(sym.name.as_str())
                .or_default()
                .push((pkg_path.as_str(), sym.id.as_str()));
        }
    }

    (functions_by_name, methods_by_name)
}

/// Resolves one `RawCallSite`. A bare identifier is looked up among `Function` symbols; a
/// qualified call (`pkg.Foo()`/`recv.Method()`) is looked up among `Method` symbols by its
/// last segment first, falling back to `Function` symbols (a qualified free-function
/// call) when no method matches.
fn resolve_one_call_edge(
    functions_by_name: &SymbolIndex,
    methods_by_name: &SymbolIndex,
    site: RawCallSite,
) -> CallEdge {
    let caller_pkg = caller_package(&site.caller_id);
    let (target_name, qualified) = match site.callee_text.rsplit_once('.') {
        Some((_, last)) => (last, true),
        None => (site.callee_text.as_str(), false),
    };

    let primary = if qualified {
        methods_by_name
    } else {
        functions_by_name
    };
    let resolved = resolve_in(primary, caller_pkg, target_name).or_else(|| {
        qualified
            .then(|| resolve_in(functions_by_name, caller_pkg, target_name))
            .flatten()
    });

    match resolved {
        Some(id) => CallEdge {
            from: site.caller_id,
            to: id.to_string(),
            resolved: true,
            file: site.file,
            line: site.line,
        },
        None => CallEdge {
            from: site.caller_id,
            to: site.callee_text,
            resolved: false,
            file: site.file,
            line: site.line,
        },
    }
}

/// Resolves every `RawCallSite`'s `callee_text` against a whole-repo index of `Function`
/// and `Method` symbols built fresh from `packages` — the call graph's only source of
/// resolution truth.
fn resolve_call_edges(
    packages: &BTreeMap<String, PackageNode>,
    raw_sites: Vec<RawCallSite>,
) -> Vec<CallEdge> {
    let (functions_by_name, methods_by_name) = build_call_target_indexes(packages);
    raw_sites
        .into_iter()
        .map(|site| resolve_one_call_edge(&functions_by_name, &methods_by_name, site))
        .collect()
}

impl ArchModel {
    /// Exact-key lookup — a package's path, unlike a glob scope, is matched verbatim.
    pub fn package(&self, path: &str) -> Option<&PackageNode> {
        self.packages.get(path)
    }

    /// Every `(package_path, &SymbolNode)` pair across all packages whose `SymbolNode.name`
    /// exactly equals `name` — a repo can have same-named symbols in different packages
    /// (that's exactly what the owner-qualified `id` scheme exists to disambiguate), so
    /// this returns all matches rather than the first.
    ///
    /// Planned API surface for an MCP/LSP "find symbol by name" lookup that hasn't been
    /// wired up to a caller yet.
    #[allow(dead_code)]
    pub fn find_symbol(&self, name: &str) -> Vec<(&str, &SymbolNode)> {
        self.packages
            .iter()
            .flat_map(|(pkg_path, pkg)| {
                pkg.symbols
                    .iter()
                    .filter(move |s| s.name == name)
                    .map(move |s| (pkg_path.as_str(), s))
            })
            .collect()
    }

    /// Returns a filtered copy: only packages whose path matches `scope` (via
    /// `crate::glob::matches_scope` — empty `scope` keeps everything, matching that
    /// function's existing empty-means-all semantics), and with every package's `symbols`
    /// cleared when `level == ModelLevel::Component` (component view has no code-level
    /// detail; `packages`/`import_edges`/`call_edges` are unaffected by `level`, matching
    /// `import_edges`'s existing precedent of not being scope-filtered either — a
    /// consumer that wants call edges scoped to a package subset filters `call_edges`
    /// itself by `from`'s `"{package}::"` prefix).
    pub fn filtered(&self, scope: &[String], level: ModelLevel) -> ArchModel {
        let mut packages: BTreeMap<String, PackageNode> = self
            .packages
            .iter()
            .filter(|(path, _)| crate::glob::matches_scope(path, scope))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        if level == ModelLevel::Component {
            for pkg in packages.values_mut() {
                pkg.symbols.clear();
            }
        }

        ArchModel {
            repo_root: self.repo_root.clone(),
            packages,
            import_edges: self.import_edges.clone(),
            call_edges: self.call_edges.clone(),
            pruning: self.pruning.clone(),
        }
    }
}

// ---------------------------------------------------------------------------------
// Epic 1.4: in-process ModelCache
// ---------------------------------------------------------------------------------

/// Identifies a cached `ArchModel`. Deliberately excludes `scope`: `build_model` itself
/// is unscoped, and `scope`/`level` are applied per-call via `ArchModel::filtered` against
/// the cached, unscoped model — keying the cache on `scope` would fragment one repo's
/// cache into a separate rebuild per distinct `scope` a caller passes in a session. See
/// ADR-002 and the plan's Pattern Decisions table ("Model caching" row).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModelCacheKey {
    pub repo_root: PathBuf,
    pub include_private: bool,
}

/// The in-process cache entry: an `ArchModel` plus the file-stamp set it was built from,
/// used to decide whether a cache hit is still valid.
#[derive(Debug, Clone)]
struct CachedModel {
    model: Arc<ArchModel>,
    /// One `Stamp` per input file, in the same order as the `files` slice passed to
    /// `get_or_build` — `None` for a path that no longer stats (e.g. deleted since the
    /// cached build). Compared for exact equality (including length) against a fresh
    /// stamp set on every call, so an added/removed file or a changed mtime/len both
    /// count as stale.
    stamps: Vec<(PathBuf, Option<crate::cache::Stamp>)>,
}

/// A single-slot, in-process, in-memory-only cache of one `ArchModel` per ADR-002 — not a
/// `HashMap`, so there's no eviction/size-bound policy to design: a `ModelCacheKey`
/// mismatch (different `repo_root`, or `include_private` flipped) just replaces the one
/// slot outright, dropping the old `ArchModel`. No persistence, no daemon RPC.
#[derive(Debug, Default)]
pub struct ModelCache {
    slot: Mutex<Option<(ModelCacheKey, CachedModel)>>,
}

impl ModelCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the cached `ArchModel` for `key` if it's still fresh (same `key`, and every
    /// path in `files` stamps identically to what's cached); otherwise invokes `build`
    /// exactly once, caches the result under `key`/`files`' current stamps (replacing
    /// whatever was cached before, if anything), and returns it.
    pub fn get_or_build(
        &self,
        key: ModelCacheKey,
        files: &[PathBuf],
        build: impl FnOnce() -> Result<ArchModel>,
    ) -> Result<Arc<ArchModel>> {
        let current_stamps: Vec<(PathBuf, Option<crate::cache::Stamp>)> = files
            .iter()
            .map(|p| (p.clone(), crate::cache::stamp(p)))
            .collect();

        {
            let slot = self.slot.lock().expect("ModelCache mutex poisoned");
            if let Some((cached_key, cached)) = slot.as_ref()
                && *cached_key == key
                && cached.stamps == current_stamps
            {
                return Ok(Arc::clone(&cached.model));
            }
        }

        // `build` (a whole-repo walk + tree-sitter parse) runs unlocked so it never
        // serializes concurrent callers behind it, and — since this is called from
        // `async fn`s in mcp.rs — never blocks the async runtime while holding the lock.
        // A second concurrent caller that also decided to rebuild here just does
        // redundant work; the last writer to reacquire the lock below wins, matching
        // this cache's existing single-slot, no-generation-counter semantics.
        let model = Arc::new(build()?);

        let mut slot = self.slot.lock().expect("ModelCache mutex poisoned");
        *slot = Some((
            key,
            CachedModel {
                model: Arc::clone(&model),
                stamps: current_stamps,
            },
        ));
        Ok(model)
    }
}

// ---------------------------------------------------------------------------------
// Wave 2 prep: shared file-collection + cached-model-loading helpers
// ---------------------------------------------------------------------------------

/// Walks `repo_root`, builds its `ImportGraph`, and reads every walked file's contents that
/// `read_to_string` succeeds on (binary/non-UTF8 files are silently skipped, not fatal — a
/// repo with an image or other binary asset shouldn't crash a whole-repo build). Called from
/// `arch_export.rs::run_export` and `arch_diagram.rs::run_diagram`; `load_cached_model` below
/// inlines the same walk+graph+read sequence itself rather than calling this, to reuse the
/// file list it already collected for the cache staleness check.
pub(crate) fn collect_repo_files(
    repo_root: &Path,
) -> Result<(ImportGraph, Vec<(PathBuf, String)>)> {
    let all_files = crate::check::walk_and_collect_files(repo_root)
        .with_context(|| format!("walking {}", repo_root.display()))?;

    let graph = crate::import_graph::build(repo_root, &all_files)
        .with_context(|| format!("building import graph for {}", repo_root.display()))?;

    let files: Vec<(PathBuf, String)> = all_files
        .into_iter()
        .filter_map(|f| std::fs::read_to_string(&f).ok().map(|s| (f, s)))
        .collect();

    Ok((graph, files))
}

/// Returns the cached `ArchModel` for `(repo_root, include_private)`, rebuilding it on a
/// cache miss/staleness. Synchronous and blocking (file walk, disk reads, and tree-sitter
/// parsing all happen here on a miss) — a caller on the async runtime (`mcp.rs`, `lsp.rs`)
/// must wrap this in `tokio::task::spawn_blocking` itself; this function does not spawn
/// anything on its own. Called from `mcp.rs`'s `list_architecture_symbols`/
/// `get_architecture_node` and `lsp.rs::build_index`.
///
/// The file walk below feeds both `ModelCache::get_or_build`'s staleness check and (on a
/// miss) the build closure's file list — walking once and reusing `file_paths` for both,
/// rather than calling `collect_repo_files` again inside the closure, which would walk the
/// repo twice on every cache miss.
pub(crate) fn load_cached_model(
    cache: &ModelCache,
    repo_root: &Path,
    include_private: bool,
) -> Result<Arc<ArchModel>> {
    let file_paths = crate::check::walk_and_collect_files(repo_root)
        .with_context(|| format!("walking {}", repo_root.display()))?;

    let key = ModelCacheKey {
        repo_root: repo_root.to_path_buf(),
        include_private,
    };

    cache.get_or_build(key, &file_paths, || {
        let import_graph = crate::import_graph::build(repo_root, &file_paths)
            .with_context(|| format!("building import graph for {}", repo_root.display()))?;
        let files: Vec<(PathBuf, String)> = file_paths
            .iter()
            .filter_map(|f| std::fs::read_to_string(f).ok().map(|s| (f.clone(), s)))
            .collect();
        build_model(
            repo_root,
            &files,
            &import_graph,
            &PruneConfig { include_private },
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Regression guard: `language_for_path` is a separate extension->Language table
    /// from `checker.rs`'s registry, verified independently by a real-world backtest
    /// against Servo — a `.rs` file silently fell through to `None` (counted as
    /// "unsupported," never reaching `symbol_extract.rs`'s Rust support) until this
    /// arm was added.
    #[test]
    fn language_for_path_recognizes_rust() {
        assert_eq!(language_for_path(Path::new("foo.rs")), Some(Language::Rust));
    }

    static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn tmp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "kibitzer-arch-model-test-{}-{name}-{}",
            std::process::id(),
            TMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_fixture(dir: &Path, rel: &str, contents: &str) -> PathBuf {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, contents).unwrap();
        path
    }

    fn empty_pruning() -> PruningSummary {
        PruningSummary {
            include_private: false,
            excluded_dirs: vec![],
            generated_files_skipped: 0,
            private_symbols_skipped: 0,
            pruned_symbol_ids: vec![],
            files_with_parse_errors: vec![],
            unsupported_language_files: 0,
            total_files_scanned: 0,
        }
    }

    fn empty_package(path: &str) -> PackageNode {
        PackageNode {
            path: path.to_string(),
            files: vec![],
            symbols: vec![],
        }
    }

    fn symbol_of_kind(name: &str, kind: SymbolKind) -> SymbolNode {
        SymbolNode {
            id: format!("pkg::{name}"),
            name: name.to_string(),
            kind,
            file: PathBuf::from("pkg/file.go"),
            line: 1,
            exported: true,
            parent: None,
        }
    }

    #[test]
    fn abstractness_is_zero_with_no_type_level_symbols() {
        let mut pkg = empty_package("a");
        pkg.symbols
            .push(symbol_of_kind("Init", SymbolKind::Function));
        assert_eq!(pkg.abstractness(), 0.0);
    }

    #[test]
    fn abstractness_ignores_functions_and_methods() {
        let mut pkg = empty_package("a");
        pkg.symbols.push(symbol_of_kind("Widget", SymbolKind::Type));
        pkg.symbols
            .push(symbol_of_kind("Reader", SymbolKind::Interface));
        pkg.symbols
            .push(symbol_of_kind("New", SymbolKind::Function));
        pkg.symbols.push(symbol_of_kind("Read", SymbolKind::Method));
        assert_eq!(pkg.abstractness(), 0.5);
    }

    #[test]
    fn abstractness_is_one_when_all_types_are_interfaces() {
        let mut pkg = empty_package("a");
        pkg.symbols
            .push(symbol_of_kind("Reader", SymbolKind::Interface));
        pkg.symbols
            .push(symbol_of_kind("Writer", SymbolKind::Interface));
        assert_eq!(pkg.abstractness(), 1.0);
    }

    fn empty_model(repo_root: &Path) -> ArchModel {
        ArchModel {
            repo_root: repo_root.to_path_buf(),
            packages: BTreeMap::new(),
            import_edges: vec![],
            call_edges: vec![],
            pruning: empty_pruning(),
        }
    }

    #[test]
    fn arch_model_serializes_packages_in_btreemap_order_not_insertion_order() {
        let mut packages = BTreeMap::new();
        packages.insert("b".to_string(), empty_package("b"));
        packages.insert("a".to_string(), empty_package("a"));

        let model = ArchModel {
            repo_root: PathBuf::from("/repo"),
            packages,
            import_edges: vec![],
            call_edges: vec![],
            pruning: empty_pruning(),
        };

        let json = serde_json::to_string(&model).expect("serializes");
        let a_pos = json.find("\"a\"").expect("contains \"a\" key");
        let b_pos = json.find("\"b\"").expect("contains \"b\" key");
        assert!(
            a_pos < b_pos,
            "expected \"a\" before \"b\" in {json}, got a_pos={a_pos} b_pos={b_pos}"
        );
    }

    #[test]
    fn symbol_node_round_trips_through_serde_without_field_loss() {
        let original = SymbolNode {
            id: "pkg/foo::Bar".to_string(),
            name: "Bar".to_string(),
            kind: SymbolKind::Type,
            file: PathBuf::from("pkg/foo/bar.go"),
            line: 12,
            exported: true,
            parent: None,
        };

        let json = serde_json::to_string(&original).expect("serializes");
        let round_tripped: SymbolNode = serde_json::from_str(&json).expect("deserializes");

        assert_eq!(original, round_tripped);
    }

    #[test]
    fn symbol_kind_serializes_as_lowercase_string() {
        let json = serde_json::to_string(&SymbolKind::Function).expect("serializes");
        assert_eq!(json, "\"function\"");
    }

    #[test]
    fn model_level_serializes_as_lowercase_string() {
        let json = serde_json::to_string(&ModelLevel::Component).expect("serializes");
        assert_eq!(json, "\"component\"");

        let json = serde_json::to_string(&ModelLevel::Code).expect("serializes");
        assert_eq!(json, "\"code\"");
    }

    #[test]
    fn import_edge_serializes_with_no_new_fields() {
        let edge = ImportEdge {
            from: "a".to_string(),
            to: "b".to_string(),
            file: PathBuf::from("a.go"),
            line: 3,
        };

        let json = serde_json::to_string(&edge).expect("serializes");
        assert_eq!(json, r#"{"from":"a","to":"b","file":"a.go","line":3}"#);
    }

    // --- Story 1.3.1: build_model ---

    #[test]
    fn build_model_groups_files_into_packages_and_attaches_import_edges() {
        let repo_root = PathBuf::from("/repo");
        let files = vec![
            (
                PathBuf::from("/repo/app/domain/a.go"),
                "package domain\n\nfunc A() {}\n".to_string(),
            ),
            (
                PathBuf::from("/repo/app/domain/b.go"),
                "package domain\n\nfunc B() {}\n".to_string(),
            ),
            (
                PathBuf::from("/repo/app/handlers/h.go"),
                "package handlers\n\nfunc H() {}\n".to_string(),
            ),
        ];
        let mut import_graph = ImportGraph::default();
        import_graph.nodes.insert("app/domain".to_string());
        import_graph.nodes.insert("app/handlers".to_string());
        import_graph.edges.push(ImportEdge {
            from: "app/domain".to_string(),
            to: "app/handlers".to_string(),
            file: PathBuf::from("/repo/app/domain/a.go"),
            line: 3,
        });

        let model =
            build_model(&repo_root, &files, &import_graph, &PruneConfig::default()).unwrap();

        assert_eq!(model.packages.len(), 2);
        assert!(model.packages.contains_key("app/domain"));
        assert!(model.packages.contains_key("app/handlers"));
        assert_eq!(model.import_edges.len(), 1);
        assert_eq!(model.import_edges[0].from, "app/domain");
        assert_eq!(model.import_edges[0].to, "app/handlers");
    }

    #[test]
    fn build_model_resolves_a_same_package_call_to_the_callee_symbol_id() {
        let repo_root = PathBuf::from("/repo");
        let files = vec![(
            PathBuf::from("/repo/pkg/a.go"),
            "package pkg\n\nfunc Bar() {}\n\nfunc Foo() {\n\tBar()\n}\n".to_string(),
        )];
        let model = build_model(
            &repo_root,
            &files,
            &ImportGraph::default(),
            &PruneConfig::default(),
        )
        .unwrap();

        assert_eq!(model.call_edges.len(), 1, "got: {:?}", model.call_edges);
        let edge = &model.call_edges[0];
        assert_eq!(edge.from, "pkg::Foo");
        assert_eq!(edge.to, "pkg::Bar");
        assert!(edge.resolved);
        assert_eq!(edge.line, 6);
    }

    #[test]
    fn build_model_prefers_a_same_package_match_over_an_ambiguous_cross_package_one() {
        let repo_root = PathBuf::from("/repo");
        let files = vec![
            (
                PathBuf::from("/repo/pkg/a.go"),
                "package pkg\n\nfunc Bar() {}\n\nfunc Foo() {\n\tBar()\n}\n".to_string(),
            ),
            (
                PathBuf::from("/repo/other/b.go"),
                "package other\n\nfunc Bar() {}\n".to_string(),
            ),
        ];
        let model = build_model(
            &repo_root,
            &files,
            &ImportGraph::default(),
            &PruneConfig::default(),
        )
        .unwrap();

        let edge = model
            .call_edges
            .iter()
            .find(|e| e.from == "pkg::Foo")
            .expect("call edge from pkg::Foo");
        assert!(edge.resolved);
        assert_eq!(edge.to, "pkg::Bar");
    }

    #[test]
    fn build_model_leaves_an_ambiguous_cross_package_call_unresolved_with_raw_text() {
        let repo_root = PathBuf::from("/repo");
        let files = vec![
            (
                PathBuf::from("/repo/pkg/a.go"),
                "package pkg\n\nfunc Foo() {\n\tBar()\n}\n".to_string(),
            ),
            (
                PathBuf::from("/repo/one/b.go"),
                "package one\n\nfunc Bar() {}\n".to_string(),
            ),
            (
                PathBuf::from("/repo/two/c.go"),
                "package two\n\nfunc Bar() {}\n".to_string(),
            ),
        ];
        let model = build_model(
            &repo_root,
            &files,
            &ImportGraph::default(),
            &PruneConfig::default(),
        )
        .unwrap();

        let edge = model
            .call_edges
            .iter()
            .find(|e| e.from == "pkg::Foo")
            .expect("call edge from pkg::Foo");
        assert!(!edge.resolved);
        assert_eq!(edge.to, "Bar");
    }

    #[test]
    fn build_model_resolves_a_selector_call_to_the_receiver_method_by_last_segment() {
        let repo_root = PathBuf::from("/repo");
        let files = vec![(
            PathBuf::from("/repo/pkg/a.go"),
            "package pkg\n\ntype T struct{}\n\nfunc (t T) M() {}\n\nfunc Foo(t T) {\n\tt.M()\n}\n"
                .to_string(),
        )];
        let model = build_model(
            &repo_root,
            &files,
            &ImportGraph::default(),
            &PruneConfig::default(),
        )
        .unwrap();

        let edge = model
            .call_edges
            .iter()
            .find(|e| e.from == "pkg::Foo")
            .expect("call edge from pkg::Foo");
        assert!(edge.resolved);
        assert_eq!(edge.to, "pkg::T.M");
    }

    #[test]
    fn build_model_resolves_via_global_unique_fallback_when_no_same_package_candidate() {
        // No `Bar` in `pkg` itself — `resolve_in`'s same-package tier has zero candidates,
        // so this exercises its second tier: exactly one candidate anywhere in the repo.
        let repo_root = PathBuf::from("/repo");
        let files = vec![
            (
                PathBuf::from("/repo/pkg/a.go"),
                "package pkg\n\nfunc Foo() {\n\tBar()\n}\n".to_string(),
            ),
            (
                PathBuf::from("/repo/other/b.go"),
                "package other\n\nfunc Bar() {}\n".to_string(),
            ),
        ];
        let model = build_model(
            &repo_root,
            &files,
            &ImportGraph::default(),
            &PruneConfig::default(),
        )
        .unwrap();

        let edge = model
            .call_edges
            .iter()
            .find(|e| e.from == "pkg::Foo")
            .expect("call edge from pkg::Foo");
        assert!(edge.resolved);
        assert_eq!(edge.to, "other::Bar");
    }

    #[test]
    fn build_model_resolves_a_qualified_call_to_a_free_function_via_index_fallback() {
        // `other.Compute()` looks like a receiver method call syntactically, but
        // `Compute` is a free `Function`, not a `Method` — exercises
        // `resolve_one_call_edge`'s fallback from the Method index to the Function index.
        let repo_root = PathBuf::from("/repo");
        let files = vec![
            (
                PathBuf::from("/repo/pkg/a.go"),
                "package pkg\n\nfunc Foo() {\n\tother.Compute()\n}\n".to_string(),
            ),
            (
                PathBuf::from("/repo/other/b.go"),
                "package other\n\nfunc Compute() {}\n".to_string(),
            ),
        ];
        let model = build_model(
            &repo_root,
            &files,
            &ImportGraph::default(),
            &PruneConfig::default(),
        )
        .unwrap();

        let edge = model
            .call_edges
            .iter()
            .find(|e| e.from == "pkg::Foo")
            .expect("call edge from pkg::Foo");
        assert!(edge.resolved);
        assert_eq!(edge.to, "other::Compute");
    }

    #[test]
    fn build_model_skips_files_with_no_language_mapping() {
        let repo_root = PathBuf::from("/repo");
        let files = vec![
            (
                PathBuf::from("/repo/pkg/a.go"),
                "package pkg\n\nfunc A() {}\n".to_string(),
            ),
            (PathBuf::from("/repo/pkg/README.md"), "# hi\n".to_string()),
        ];
        let import_graph = ImportGraph::default();

        let model =
            build_model(&repo_root, &files, &import_graph, &PruneConfig::default()).unwrap();

        let pkg = model.package("pkg").expect("pkg exists");
        assert_eq!(pkg.files, vec![PathBuf::from("/repo/pkg/a.go")]);
        assert_eq!(model.pruning.unsupported_language_files, 1);
        assert_eq!(model.pruning.total_files_scanned, 2);
    }

    #[test]
    fn build_model_excludes_private_symbols_by_default_and_counts_them() {
        let repo_root = PathBuf::from("/repo");
        let source =
            "package pkg\n\nfunc A() {}\nfunc B() {}\nfunc C() {}\nfunc a() {}\nfunc b() {}\n";
        let files = vec![(PathBuf::from("/repo/pkg/f.go"), source.to_string())];
        let import_graph = ImportGraph::default();

        let model = build_model(
            &repo_root,
            &files,
            &import_graph,
            &PruneConfig {
                include_private: false,
            },
        )
        .unwrap();

        let pkg = model.package("pkg").unwrap();
        assert_eq!(pkg.symbols.len(), 3);
        assert_eq!(model.pruning.private_symbols_skipped, 2);
        assert_eq!(model.pruning.pruned_symbol_ids.len(), 2);
        assert!(
            model
                .pruning
                .pruned_symbol_ids
                .iter()
                .all(|id| id.ends_with("::a") || id.ends_with("::b"))
        );
    }

    #[test]
    fn build_model_includes_private_symbols_when_configured() {
        let repo_root = PathBuf::from("/repo");
        let source = "package pkg\n\nfunc A() {}\nfunc a() {}\n";
        let files = vec![(PathBuf::from("/repo/pkg/f.go"), source.to_string())];
        let import_graph = ImportGraph::default();

        let model = build_model(
            &repo_root,
            &files,
            &import_graph,
            &PruneConfig {
                include_private: true,
            },
        )
        .unwrap();

        let pkg = model.package("pkg").unwrap();
        assert_eq!(pkg.symbols.len(), 2);
        assert_eq!(model.pruning.private_symbols_skipped, 0);
    }

    #[test]
    fn build_model_skips_generated_files_and_counts_them() {
        let repo_root = PathBuf::from("/repo");
        let source =
            "// Code generated by protoc-gen-go. DO NOT EDIT.\npackage pkg\n\nfunc A() {}\n";
        let files = vec![(PathBuf::from("/repo/pkg/f.go"), source.to_string())];
        let import_graph = ImportGraph::default();

        let model =
            build_model(&repo_root, &files, &import_graph, &PruneConfig::default()).unwrap();

        assert_eq!(model.pruning.generated_files_skipped, 1);
        assert!(model.package("pkg").is_none());
    }

    #[test]
    fn build_model_skips_files_with_parse_errors_without_failing() {
        let repo_root = PathBuf::from("/repo");
        let good = "package pkg\n\nfunc Good() {}\n".to_string();
        let broken = "package pkg\n\nfunc Broken() {\n\tif true {\n".to_string();
        let files = vec![
            (PathBuf::from("/repo/pkg/good.go"), good),
            (PathBuf::from("/repo/pkg/broken.go"), broken),
        ];
        let import_graph = ImportGraph::default();

        let result = build_model(&repo_root, &files, &import_graph, &PruneConfig::default());
        let model = result.expect("build_model returns Ok even with a parse error");

        let pkg = model.package("pkg").unwrap();
        assert_eq!(pkg.symbols.len(), 1);
        assert_eq!(pkg.symbols[0].name, "Good");
        assert_eq!(
            model.pruning.files_with_parse_errors,
            vec![PathBuf::from("/repo/pkg/broken.go")]
        );
        assert_eq!(
            pkg.files,
            vec![PathBuf::from("/repo/pkg/good.go")],
            "a parse-error file gets the same treatment as a generated file — excluded from \
             PackageNode.files, not just from symbols"
        );
    }

    #[test]
    fn build_model_end_to_end_on_mixed_go_ts_fixture_produces_expected_shape() {
        let dir = tmp_dir("e2e-mixed");
        write_fixture(&dir, "go.mod", "module example.com/app\n\ngo 1.21\n");
        let go_a = write_fixture(
            &dir,
            "domain/a.go",
            "package domain\n\nfunc A() {}\nfunc b() {}\n",
        );
        let go_b = write_fixture(
            &dir,
            "handlers/h.go",
            "package handlers\n\nimport \"example.com/app/domain\"\n\nfunc H() { domain.A() }\n",
        );
        let ts_a = write_fixture(&dir, "web/index.ts", "export function Widget() {}\n");

        let file_paths = vec![go_a.clone(), go_b.clone(), ts_a.clone()];
        let import_graph = crate::import_graph::build(&dir, &file_paths).unwrap();

        let files: Vec<(PathBuf, String)> = file_paths
            .iter()
            .map(|p| (p.clone(), std::fs::read_to_string(p).unwrap()))
            .collect();

        let model = build_model(&dir, &files, &import_graph, &PruneConfig::default()).unwrap();

        assert_eq!(model.packages.len(), 3);
        // Go package keys are module-qualified import paths (matching `ImportGraph`'s
        // node keys), not plain repo-relative directories.
        let domain = model
            .package("example.com/app/domain")
            .expect("domain package exists under its module-qualified key");
        assert_eq!(domain.symbols.len(), 1);
        assert_eq!(domain.symbols[0].name, "A");

        let handlers = model
            .package("example.com/app/handlers")
            .expect("handlers package exists under its module-qualified key");
        assert_eq!(handlers.symbols.len(), 1);
        assert_eq!(handlers.symbols[0].name, "H");

        // JS/TS package keys come from `ImportGraph::file_packages` too — its `dir_key`
        // convention uses the file's actual parent directory path, not one relative to
        // `repo_root`, so look up the expected key via the same map rather than
        // hardcoding it here.
        let web_key = import_graph
            .file_packages
            .get(&ts_a)
            .expect("ts_a mapped in file_packages")
            .clone();
        let web = model
            .package(&web_key)
            .expect("web package exists under its file_packages key");
        assert_eq!(web.symbols.len(), 1);
        assert_eq!(web.symbols[0].name, "Widget");

        assert_eq!(model.import_edges.len(), import_graph.edges.len());
        assert_eq!(model.pruning.total_files_scanned, 3);
        assert_eq!(model.pruning.unsupported_language_files, 0);

        // `ArchModel::package()` resolves an `ImportEdge` endpoint directly — the
        // contract this fix restores: `PackageNode` keys must match `ImportGraph` node
        // keys so a consumer can look up an edge's `from`/`to` and get the real package.
        assert_eq!(import_graph.edges.len(), 1);
        let edge = &import_graph.edges[0];
        assert_eq!(edge.from, "example.com/app/handlers");
        assert_eq!(edge.to, "example.com/app/domain");
        assert!(
            model.package(&edge.from).is_some(),
            "package() should resolve edge.from"
        );
        assert!(
            model.package(&edge.to).is_some(),
            "package() should resolve edge.to"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn build_model_completes_under_5s_on_synthetic_fixture() {
        let repo_root = PathBuf::from("/repo");
        let mut files: Vec<(PathBuf, String)> = Vec::new();
        for i in 0..40 {
            let go_src = format!(
                "package pkg{i}\n\ntype Widget{i} struct{{}}\n\nfunc (w Widget{i}) Do() {{}}\n\nfunc Handle{i}() {{}}\n\nfunc helper{i}() {{}}\n"
            );
            files.push((PathBuf::from(format!("/repo/svc{i}/widget.go")), go_src));
            let ts_src = format!(
                "export interface Shape{i} {{ area(): number; }}\n\nexport function make{i}(): Shape{i} {{ return null as unknown as Shape{i}; }}\n"
            );
            files.push((PathBuf::from(format!("/repo/web/mod{i}/shape.ts")), ts_src));
        }
        // 40 Go + 40 TS = 80 files, within the plan's 50-100-file target range.

        let import_graph = ImportGraph::default();
        let start = std::time::Instant::now();
        let model =
            build_model(&repo_root, &files, &import_graph, &PruneConfig::default()).unwrap();
        let elapsed = start.elapsed();

        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "build_model took {elapsed:?} on an 80-file synthetic fixture, expected well under 5s"
        );
        assert_eq!(model.pruning.total_files_scanned, 80);
    }

    #[test]
    fn build_model_handles_large_synthetic_fixture_without_panicking() {
        let repo_root = PathBuf::from("/repo");
        let mut files: Vec<(PathBuf, String)> = Vec::new();
        for i in 0..2000 {
            let go_src = format!("package pkg{i}\n\nfunc Handle{i}() {{}}\n");
            files.push((PathBuf::from(format!("/repo/svc{i}/h.go")), go_src));
        }

        let import_graph = ImportGraph::default();
        let model = build_model(&repo_root, &files, &import_graph, &PruneConfig::default())
            .expect("build_model completes without error on a 2000-file synthetic fixture");

        assert_eq!(model.pruning.total_files_scanned, 2000);
    }

    // --- Story 1.3.2: query API ---

    #[test]
    fn arch_model_package_returns_exact_match_and_none_otherwise() {
        let mut packages = BTreeMap::new();
        packages.insert("app/domain".to_string(), empty_package("app/domain"));
        let model = ArchModel {
            repo_root: PathBuf::from("/repo"),
            packages,
            import_edges: vec![],
            call_edges: vec![],
            pruning: empty_pruning(),
        };

        assert!(model.package("app/domain").is_some());
        assert!(model.package("app/other").is_none());
    }

    #[test]
    fn arch_model_find_symbol_returns_a_match_per_package() {
        let mut pkg_a = empty_package("a");
        pkg_a.symbols.push(SymbolNode {
            id: "a::Init".to_string(),
            name: "Init".to_string(),
            kind: SymbolKind::Function,
            file: PathBuf::from("a/init.go"),
            line: 1,
            exported: true,
            parent: None,
        });
        let mut pkg_b = empty_package("b");
        pkg_b.symbols.push(SymbolNode {
            id: "b::Init".to_string(),
            name: "Init".to_string(),
            kind: SymbolKind::Function,
            file: PathBuf::from("b/init.go"),
            line: 1,
            exported: true,
            parent: None,
        });

        let mut packages = BTreeMap::new();
        packages.insert("a".to_string(), pkg_a);
        packages.insert("b".to_string(), pkg_b);

        let model = ArchModel {
            repo_root: PathBuf::from("/repo"),
            packages,
            import_edges: vec![],
            call_edges: vec![],
            pruning: empty_pruning(),
        };

        let results = model.find_symbol("Init");
        assert_eq!(results.len(), 2);
        assert!(results.iter().any(|(pkg, _)| *pkg == "a"));
        assert!(results.iter().any(|(pkg, _)| *pkg == "b"));
    }

    #[test]
    fn arch_model_filtered_scope_keeps_only_matching_packages_with_symbols_intact() {
        let mut ui_pkg = empty_package("web/ui");
        ui_pkg.symbols.push(SymbolNode {
            id: "web/ui::Widget".to_string(),
            name: "Widget".to_string(),
            kind: SymbolKind::Type,
            file: PathBuf::from("web/ui/widget.ts"),
            line: 1,
            exported: true,
            parent: None,
        });
        let api_pkg = empty_package("server/api");

        let mut packages = BTreeMap::new();
        packages.insert("web/ui".to_string(), ui_pkg);
        packages.insert("server/api".to_string(), api_pkg);

        let model = ArchModel {
            repo_root: PathBuf::from("/repo"),
            packages,
            import_edges: vec![],
            call_edges: vec![],
            pruning: empty_pruning(),
        };

        let filtered = model.filtered(&["web/**".to_string()], ModelLevel::Code);
        assert_eq!(filtered.packages.len(), 1);
        let ui = filtered.package("web/ui").expect("web/ui kept");
        assert_eq!(ui.symbols.len(), 1);
    }

    #[test]
    fn arch_model_filtered_component_level_clears_symbols_but_keeps_packages() {
        let mut ui_pkg = empty_package("web/ui");
        ui_pkg.symbols.push(SymbolNode {
            id: "web/ui::Widget".to_string(),
            name: "Widget".to_string(),
            kind: SymbolKind::Type,
            file: PathBuf::from("web/ui/widget.ts"),
            line: 1,
            exported: true,
            parent: None,
        });
        let api_pkg = empty_package("server/api");

        let mut packages = BTreeMap::new();
        packages.insert("web/ui".to_string(), ui_pkg);
        packages.insert("server/api".to_string(), api_pkg);

        let model = ArchModel {
            repo_root: PathBuf::from("/repo"),
            packages,
            import_edges: vec![],
            call_edges: vec![],
            pruning: empty_pruning(),
        };

        let filtered = model.filtered(&[], ModelLevel::Component);
        assert_eq!(filtered.packages.len(), 2);
        assert!(filtered.package("web/ui").unwrap().symbols.is_empty());
        assert!(filtered.package("server/api").unwrap().symbols.is_empty());
    }

    // --- Story 1.4.1: ModelCache ---

    #[test]
    fn model_cache_get_or_build_invokes_build_closure_once_for_repeat_call() {
        let dir = tmp_dir("cache-repeat");
        let file = write_fixture(&dir, "a.go", "package pkg\n\nfunc A() {}\n");
        let files = vec![file];

        let cache = ModelCache::new();
        let key = ModelCacheKey {
            repo_root: dir.clone(),
            include_private: false,
        };
        let calls = Rc::new(Cell::new(0));

        let calls1 = calls.clone();
        let dir1 = dir.clone();
        let first = cache
            .get_or_build(key.clone(), &files, move || {
                calls1.set(calls1.get() + 1);
                Ok(empty_model(&dir1))
            })
            .unwrap();

        let calls2 = calls.clone();
        let dir2 = dir.clone();
        let second = cache
            .get_or_build(key, &files, move || {
                calls2.set(calls2.get() + 1);
                Ok(empty_model(&dir2))
            })
            .unwrap();

        assert_eq!(calls.get(), 1);
        assert!(Arc::ptr_eq(&first, &second));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn model_cache_rebuilds_when_key_include_private_flips() {
        let dir = tmp_dir("cache-key-flip");
        let file = write_fixture(&dir, "a.go", "package pkg\n\nfunc A() {}\n");
        let files = vec![file];

        let cache = ModelCache::new();
        let key_a = ModelCacheKey {
            repo_root: dir.clone(),
            include_private: false,
        };
        let key_b = ModelCacheKey {
            repo_root: dir.clone(),
            include_private: true,
        };

        let dir1 = dir.clone();
        let first = cache
            .get_or_build(key_a, &files, move || Ok(empty_model(&dir1)))
            .unwrap();

        let dir2 = dir.clone();
        let second = cache
            .get_or_build(key_b, &files, move || {
                let mut m = empty_model(&dir2);
                m.pruning.include_private = true;
                Ok(m)
            })
            .unwrap();

        assert!(!Arc::ptr_eq(&first, &second));
        assert!(second.pruning.include_private);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn model_cache_rebuilds_when_a_file_stamp_changes() {
        let dir = tmp_dir("cache-stamp-change");
        let file = write_fixture(&dir, "a.go", "package pkg\n\nfunc A() {}\n");
        let files = vec![file.clone()];

        let cache = ModelCache::new();
        let key = ModelCacheKey {
            repo_root: dir.clone(),
            include_private: false,
        };

        let dir1 = dir.clone();
        let first = cache
            .get_or_build(key.clone(), &files, move || Ok(empty_model(&dir1)))
            .unwrap();

        // Changing the file's length changes its Stamp regardless of mtime resolution.
        std::fs::write(&file, "package pkg\n\nfunc A() {}\n// touched\n").unwrap();

        let dir2 = dir.clone();
        let second = cache
            .get_or_build(key, &files, move || Ok(empty_model(&dir2)))
            .unwrap();

        assert!(!Arc::ptr_eq(&first, &second));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn model_cache_rebuilds_when_a_file_is_added_or_removed() {
        let dir = tmp_dir("cache-file-list-change");
        let file_a = write_fixture(&dir, "a.go", "package pkg\n\nfunc A() {}\n");
        let file_b = write_fixture(&dir, "b.go", "package pkg\n\nfunc B() {}\n");

        let cache = ModelCache::new();
        let key = ModelCacheKey {
            repo_root: dir.clone(),
            include_private: false,
        };

        let dir1 = dir.clone();
        let first = cache
            .get_or_build(key.clone(), std::slice::from_ref(&file_a), move || {
                Ok(empty_model(&dir1))
            })
            .unwrap();

        let dir2 = dir.clone();
        let second = cache
            .get_or_build(key, &[file_a, file_b], move || Ok(empty_model(&dir2)))
            .unwrap();

        assert!(!Arc::ptr_eq(&first, &second));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
