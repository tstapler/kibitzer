# Research: Architecture — architecture-export

## 1. Existing systems read

### `src/architecture_checks.rs` (443 lines)
- `ArchitectureChecker` trait: `name()` + `check(&ImportGraph, &ArchitectureConfig) -> Vec<ArchFinding>`. Plain registry pattern: `registry() -> Vec<Box<dyn ArchitectureChecker>>` (`ImportCycleChecker`, `LayeringChecker`, `CouplingChecker`), `lookup(name)`.
- `ArchFinding { file: Option<PathBuf>, line: Option<usize>, message: String }` — graph-wide findings (coupling) carry no location; edge-derived findings (cycles, layering) do.
- `find_cycles()` (Tarjan's SCC) is `pub` specifically so `mermaid.rs` can reuse it without recomputing — already the "one model, multiple consumers" pattern this feature needs to generalize.
- This trait operates purely on the *existing* `ImportGraph` (package/module level only) — it has no notion of symbols. A new architecture-model layer sits *alongside*, not *inside*, this file; `ImportCycleChecker`/`LayeringChecker`/`CouplingChecker` stay as-is and become one input into the richer model, not a base to extend.

### `src/import_graph.rs` (379 lines)
- `ImportEdge { from: String, to: String, file: PathBuf, line: usize }`, `ImportGraph { nodes: BTreeSet<String>, edges: Vec<ImportEdge> }`.
- `build(repo_root, files) -> Result<ImportGraph>` dispatches by extension: `build_go` (needs `go.mod` to map file dir → import path), `build_js` (directory-granularity, relative-import resolution only, bare specifiers skipped). No Python/Java/Kotlin extraction exists yet — confirms the requirements doc's "language coverage is a major scope lever" rabbit hole is real, not hypothetical.
- Graph nodes are **directories/packages**, not files, and edges point between them — this granularity choice (package, not file) is already the right level for a "Component" C4 view; a "Code" (symbol) view needs a *finer* granularity nested underneath, not a replacement.

### `src/checker.rs` (300 lines)
- `Checker` trait (file-level, per-language) + `registry()`/`lookup()` — the pattern `rules.rs`'s `SyntaxRulesChecker` implements once per `Language` variant.
- `Language` enum (Go, TypeScript, Tsx, JavaScript, Python, Java, Kotlin) — **all 7** syntax-rules languages already have grammars wired via `Language::ts_language()`. This is broader coverage than `import_graph.rs`'s Go/JS-only.
- `GrammarCache`: parses at most once per `Language` per cache instance (keyed by `Language` alone — caller must construct a fresh cache per file). This is the reusable parse-sharing primitive; a symbol-extraction pass over the same files the syntax-rules checkers already visit can share parses through the same mechanism if run in the same pass, or re-parse (tree-sitter parses are cheap relative to I/O) if run as a separate pass.
- `rules.rs`'s `lang_config(lang: Language) -> LangRuleConfig` (a per-language table of node-kind strings: function_kinds, if_kind, param field names, etc.) is the **direct precedent** for a new `LangSymbolConfig` table driving symbol extraction — same enum, same per-language-table-of-node-kinds shape, no new abstraction needed.

### `src/mcp.rs` (441 lines)
- `architecture_assessment` tool composes, per call, in this order: `find_config` → `walk_and_collect_files` (+ scope filter via `matches_scope`) → run all `architecture_checker`-tagged `Check`s from config → run all 7 `SYNTAX_RULES_CHECKERS` over every file → build `ImportGraph` (again — a **second** full walk+parse, `import_graph::build` is not reused from the architecture-checker pass) → `mermaid::render_dependency_graph` (150-node cap, falls back to text).
- Notable inefficiency confirmed by reading: the import graph is built *twice* per call today — once implicitly (each `ArchitectureChecker::check` receives an `ImportGraph` built by `run_architecture_check`, not shown here but implied by `check.rs`) and once explicitly at the bottom for the diagram. A shared model eliminates this duplication as a side effect, not just adds new capability.
- Output is a single formatted `String` — no structured (JSON) return today. New MCP tool(s) for the model should return structured JSON (or a JSON string), not follow this string-formatting convention, since the whole point is queryability.

### `src/daemon.rs` (264 lines) — cache mechanism
- **Per-file, per-check-result cache**, not whole-repo. `Cache { entries: HashMap<String, CacheEntry> }` keyed by file path string; `CacheEntry` holds `file_stamp` (mtime+len, no content hash), `config_stamp`, `trigger`, and `Vec<CheckResult>`. Persisted to disk (`Cache::load`/`save`) and shared across daemon connections via `Arc<Mutex<Cache>>`.
- Confirmed via `src/cache.rs:1-60`: fingerprinting is `(mtime_secs, mtime_nanos, len)`, cheap and file-granular. There is **no whole-repo aggregate cache entry today** — every `RunChecks` request is resolved against one file.
- **Conclusion: does not fit as-is.** Caching a whole-repo architecture model needs a new cache entry shape — either (a) a new `Cache`-sibling structure keyed by `repo_root` (+ maybe `scope` glob) holding the serialized model plus a fingerprint of *all* scoped files' stamps (expensive to check on every call — O(files) stat calls, same cost the daemon already pays per `walk_and_collect_files`), or (b) an **in-memory-only** `Arc<Mutex<Option<(Fingerprint, ArchModel)>>>` on the daemon that isn't persisted to disk, since the model is inherently more expensive to rebuild than one file's checks and more likely to go stale (any file in the repo touching it, not just one). Recommend (b): don't extend the persisted per-file `Cache` struct's schema for this — add a separate, session-scoped cache the daemon owns, invalidated on a coarse "does the file-stamp set match" check, not the fine-grained per-check cache. This is additive to `daemon.rs`, not a refactor of it.

### `src/install.rs` (pure/I-O split precedent)
- `run_install` (I/O: resolves paths, reads/writes `settings.json`, prints) calls `merge_hook` (pure: takes `&mut Value` + `&str` command, returns `Result<bool>`, no I/O) — confirmed at `src/install.rs:14-55`, `merge_hook` referenced but not shown in the read range (grep confirms it's a separate fn below). This is the shape to replicate for the new module: a pure builder that takes already-loaded ASTs/graph and returns the model, separate from the function that does the walking/parsing/file I/O.

### `src/lsp.rs` (231 lines)
- Diagnostics are **disk-based today, not buffer-based** — `check_and_publish` reads `path` off disk via `run_checks_for_trigger`, and the module comment at `src/lsp.rs:88-95` explicitly documents this as a known gap: *"diagnostics reflect the last-saved content, not unsaved keystrokes... Wiring in the live buffer (via LSP's incremental sync) is real future work, called out in issue #11, not done here."* `did_change` re-runs from disk, not from the in-memory `DidChangeTextDocumentParams` content.
- This is a **direct, pre-existing precedent** for the exact tension requirement §3 asks about, and it's already resolved once in this codebase in favor of "disk-based, documented gap, not blocking" — see §3 below.

### `src/mermaid.rs` (162 lines)
- Pure function `render_dependency_graph(&ImportGraph) -> String`, no I/O, calls back into `architecture_checks::find_cycles` for highlighting. 150-node cap with text fallback. This is the existing degrade-gracefully precedent the new C4-like diagram renderer should match (same cap philosophy, likely a different constant tuned per level).

## 2. Data flow and module boundary

Pipeline: **source files → parsed ASTs (per language) → package/symbol tree model → N consumer views** (JSON file, MCP responses, LSP symbols, diagram).

Proposed new module: **`src/arch_model.rs`**, following the `install.rs` pure/I-O split:

- **Pure**: `pub fn build_model(repo_root: &Path, files: &[PathBuf], import_graph: &ImportGraph, cache: &GrammarCache) -> Result<ArchModel>` — takes an already-built `ImportGraph` (don't rebuild it internally; reuse `import_graph::build`'s output, fixing the double-build seen in `mcp.rs` today) plus already-parsed/parseable files, walks each file's tree-sitter AST via a new `LangSymbolConfig` table (modeled directly on `rules.rs::lang_config`) to extract type/interface/function declarations, and assembles the tree. No file I/O beyond reading source (which every other checker already does) — testable with in-memory fixtures like `import_graph.rs`'s own tests.
- **I/O boundary stays in callers**: `check.rs` (or a new thin `arch_export.rs` for the CLI) does `walk_and_collect_files` + `find_config`, then calls `import_graph::build` + `arch_model::build_model`, then hands the `ArchModel` to whichever consumer needs it:
  - CLI (`kibitzer arch export --out <file> [--scope <glob>] [--format json]`): serializes `ArchModel` to a file via `serde_json::to_writer_pretty`. New `Command::Arch { action: ArchAction::Export { .. } }` variant in `main.rs`, alongside the existing `Daemon`/`Check` subcommand-of-subcommand pattern.
  - MCP: new tool(s) in `mcp.rs` (e.g. `arch_query`) call `build_model` and return `serde_json::to_string(&model_or_subview)` instead of the hand-formatted strings `architecture_assessment` uses — a genuinely different response shape from the existing tools, which is fine, they serve a different purpose (query/navigate vs. one-shot advisory report).
  - LSP: `lsp.rs` gains a `workspace/symbol` and/or `textDocument/documentSymbol` handler that calls `build_model` scoped to the relevant file(s) and maps `ArchModel` symbol nodes to `lsp_types::SymbolInformation`/`DocumentSymbol` — a pure mapping function, same shape as `diagnostics_from_result`.
  - Diagram: extend `mermaid.rs` (or add `arch_diagram.rs` if the C4-like Component/Code rendering diverges enough from the existing dependency-graph renderer to warrant its own file — likely yes, since C4 Component boxes containing Code-level children is a materially different Mermaid shape, e.g. `graph TD` subgraphs, than the flat node-and-edge diagram `render_dependency_graph` produces today) — takes `&ArchModel` (or a level-filtered view of it) and returns a `String`, same signature shape as `render_dependency_graph`.

This keeps `import_graph.rs` and `architecture_checks.rs` untouched (existing checks keep working exactly as today) and adds `arch_model.rs` as the new shared-model owner, exactly matching the requirement's "one model, multiple views" framing.

## 3. Consistency requirement — flagging the tension explicitly

**Per-invocation snapshot (no live-updating) is sufficient for CLI export and MCP querying** — both are already request/response, one-shot-per-call in this codebase (`architecture_assessment` recomputes from scratch every call today; nothing currently persists results across calls other than the per-file `Cache` in `daemon.rs`).

**LSP is a genuine tension, but the codebase has already resolved an identical tension once, and this feature should follow the same resolution rather than solve it differently:**

`src/lsp.rs:88-95`'s existing comment documents that `did_change` diagnostics are disk-based, not live-buffer, and calls that out as a known, deliberately deferred gap (issue #11), not a blocker. Editors generally tolerate this for diagnostics (mildly stale = annoying but not broken); workspace/document symbols have the same tolerance in practice (most LSP clients re-request document symbols on save or on-demand, not on every keystroke), so **workspace-symbol integration should reuse the exact same disk-based snapshot model** the diagnostics path already uses — read `path` off disk, no `DidChangeTextDocumentParams` buffer wiring — rather than introducing a new live-buffer code path just for this feature. Doing otherwise would mean this feature invents a *better* consistency model than the rest of `lsp.rs` has, which is scope creep relative to "Out of Scope: real-time incremental updates" in the requirements doc, and would leave two different staleness models in the same file for a reader to reconcile.

Flag for the plan phase: if a human's expectation is "workspace symbols reflect my unsaved edits," that expectation is *already violated* by every other kibitzer LSP diagnostic today, so this feature isn't making anything worse — but it's worth one sentence in the plan doc's non-functional requirements saying so explicitly, since the requirements doc's "editors expect workspace symbols to reflect the currently-open buffer" framing reads as if this were a new problem when it's an existing, accepted one.

## 4. EventStorming Event-Command-Policy table

**Skipped.** This is a single-repo, static-analysis feature: one actor (the tool operator or an AI agent driving the CLI/MCP/LSP interfaces), no multi-party workflow, no state machine with competing actors or async policies triggering other commands. The "commands" here are just tool invocations against pure/read-only data derived from source files, and the "events" are simply "model built" / "query answered" — there's no business process where one actor's action creates a domain event another actor or policy reacts to. An E-C-P table would just restate the CLI/MCP/LSP entry points already covered in §2 under a different label. Confirmed by the requirements doc itself flagging this as "very likely NOT warranted."

## 5. Proposed shared model shape

```rust
// src/arch_model.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchModel {
    pub repo_root: PathBuf,
    /// Package/module nodes, keyed by the same path string `ImportGraph::nodes` uses
    /// today (so the two stay trivially cross-referenceable without a translation step).
    pub packages: BTreeMap<String, PackageNode>,
    /// Import edges, reusing `import_graph::ImportEdge` directly rather than a parallel
    /// type — no divergence between the checkers' view and the model's view of imports.
    pub import_edges: Vec<ImportEdge>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageNode {
    pub path: String,           // matches ImportGraph node key
    pub files: Vec<PathBuf>,
    pub symbols: Vec<SymbolNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolNode {
    pub name: String,
    pub kind: SymbolKind,       // Type | Interface | Function | Method | ...
    pub file: PathBuf,
    pub line: usize,
    pub exported: bool,         // language-specific export/visibility rule already
                                 // needed per-language (pub in Go, capitalization in Go,
                                 // export in TS, def/class in Python, etc.)
    /// Populated only for Method/function-on-type symbols; lets a Code-level diagram
    /// nest methods under their owning type without a second pass.
    pub parent: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SymbolKind {
    Type,
    Interface,
    Function,
    Method,
}

/// The two C4-like views this feature renders from one `ArchModel` — Component
/// (package/module graph, i.e. what `mermaid::render_dependency_graph` already draws)
/// and Code (symbols within a package/file). Threaded through query/filter APIs so CLI
/// export, MCP tools, and the diagram renderer all express "how deep do I want this" the
/// same way instead of three different ad hoc flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModelLevel {
    Component,
    Code,
}

impl ArchModel {
    /// Package-name substring/prefix lookup — the query surface backing an MCP
    /// "find package" tool and CLI `--package` filter.
    pub fn package(&self, path: &str) -> Option<&PackageNode> { .. }

    /// Symbol-name lookup across all packages — backs an MCP "find symbol" tool.
    pub fn find_symbol(&self, name: &str) -> Vec<(&str, &SymbolNode)> { .. }

    /// Reuses `glob::matches_scope` (already used by `walk_and_collect_files`/
    /// `architecture_assessment`'s `scope` param) so filtering is one consistent
    /// glob dialect across the whole tool, not a second pattern language for this
    /// feature alone.
    pub fn filtered(&self, scope: &[String], level: ModelLevel) -> ArchModel { .. }
}
```

Design notes:
- `import_edges: Vec<ImportEdge>` reuses `import_graph::ImportEdge` verbatim (already `Serialize`-able if `#[derive(Serialize, Deserialize)]` is added there — currently it only derives `Debug, Clone, PartialEq, Eq`; that's a one-line addition, not a new type).
- `packages: BTreeMap<String, PackageNode>` (not `Vec`) so package lookup by path is O(log n) and JSON output is stably ordered (matches `ImportGraph`'s existing `BTreeSet<String>` ordering choice, same rationale).
- `ModelLevel` directly answers the requirement's "filterable/queryable by... 'level' (component vs. code)" ask, and doubles as the parameter the two diagram renderers (Component graph vs. Code graph) key off of, and as an MCP tool parameter.
- `scope` filtering reuses `glob::matches_scope` rather than inventing a second glob dialect, per the "Whether monorepo-style repos need any filtering/scoping beyond the existing `scope` glob pattern" open question — answer: no, reuse it.
