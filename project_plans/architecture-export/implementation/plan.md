# Implementation Plan: architecture-export

**Feature**: A persisted, queryable, symbol-and-package-level architecture model of a repo — one shared `ArchModel`, exposed via a CLI export command, MCP query tools, LSP workspace/document symbols, and a C4-*like* diagram renderer.
**Date**: 2026-08-23
**Status**: Ready for implementation
**ADRs**: ADR-001 (MCP query tools return structured JSON, not the flat-string convention), ADR-002 (in-process model cache, not daemon-integrated)

**Note on task time estimates**: every `(~N min)` duration below is estimated LLM-subagent
execution time for that task, not human engineering hours — don't read these against a
human-hours estimation framework (e.g. an AIC-style 1-4h task-sizing convention) used
elsewhere in this org.

---

## CREATIVE pass — alternatives considered before committing

Three high-level approaches were weighed for how to build the symbol/package model itself
(the crux risk this feature's Rabbit Holes section flags):

1. **Extend kibitzer's existing `GrammarCache` + per-language node-kind-table pattern**
   (`src/checker.rs` + `src/rules.rs`'s `LangRuleConfig`), adding a parallel
   `LangSymbolConfig` table and per-language extraction functions.
   *Strength*: reuses a pattern already proven across all 7 grammars by 40+ existing
   tests (`src/rules.rs`), including known escape hatches (Kotlin's field-less nodes,
   Go's grouped parameter declarations) — no new correctness-risk class introduced.
   *Weakness*: still requires genuinely new, hand-verified per-language work for
   type/interface/exported-function node kinds, since `LangRuleConfig` today only covers
   function-body/param/nesting shapes, not type declarations.

2. **Adopt a tree-sitter query-language (`.scm`) DSL, or the `tree-sitter-graph` crate,
   for a single generic cross-language walker.**
   *Strength*: separates "what counts as a symbol" (declarative query files) from Rust
   control flow, which reads as more maintainable on paper.
   *Weakness*: per the Pitfalls research, the node-shape divergence that makes
   `rules.rs` hand-roll 7 per-language functions (field-based vs. positional children,
   Go's grouped parameter declarations, Kotlin's missing field names) doesn't shrink
   under a query DSL — it just relocates into 7 `.scm` files that are harder to unit-test
   than Rust match arms, and introduces a tree-sitter API surface (`Query`) kibitzer uses
   nowhere today.

3. **Adopt an external code-intelligence format/tool as the primary model** (SCIP, LSIF,
   universal-ctags, Structurizr) instead of building natively.
   *Strength*: reuses a maintained, standards-adjacent format instead of inventing one.
   *Weakness*: every option researched is either the wrong shape (SCIP/LSIF are
   reference-index formats, not package→symbol architecture trees; a flat node/edge
   encoding isn't the nested, jq-friendly tree the requirements ask for), unmaintained
   (LSIF deprecated by its own steward, `stack-graphs` archived 2025-09-09), or breaks
   kibitzer's offline/single-binary value proposition (Structurizr Lite requires a
   Docker/JVM viewer).

**Chosen: Approach 1.** It's the only option with zero new correctness-risk class and zero
new dependency, and it's the one every research file (`stack.md`, `architecture.md`,
`pitfalls.md`, `build-vs-buy.md`) converges on independently. Approaches 2 and 3 are
recorded in the Pattern Decisions table below as rejected alternatives, not silently
dropped.

---

## Domain Glossary

| Term | Definition | Notes |
|------|-----------|-------|
| `ArchModel` | The shared, repo-scoped architecture model: packages (keyed by path) plus the import edges between them. The one model all four consumer interfaces (CLI export, MCP query, LSP symbols, diagram) read from. | `src/arch_model.rs`. Serde-derived, no I/O inside it. |
| `PackageNode` | One package/module-directory node in `ArchModel` — a path (matching `ImportGraph`'s node key), the files under it, and its `SymbolNode`s. | Component-level unit. |
| `SymbolNode` | One extracted type/interface/function/method declaration: name, `SymbolKind`, file, line, `exported` flag, optional `parent` (for methods nested under a type), and a deterministic `id` — owner-qualified for methods (`{package_path}::{parent}.{name}`) so same-named methods on different types in one package don't collide; see the Pattern Decisions table. | Code-level unit. |
| `SymbolKind` | Sum type: `Type \| Interface \| Function \| Method`. | Exhaustively matched by every consumer (diagram renderer, LSP `SymbolKind` mapper, MCP `kind` filter) — no stringly-typed kind comparisons. |
| `ModelLevel` | Sum type: `Component \| Code`. Threaded through every consumer's "how deep" parameter (CLI `--level`, MCP `level` field, diagram renderer) instead of each interface inventing its own depth flag. | |
| `LangSymbolConfig` | Per-`Language` table of node-kind strings driving symbol extraction (type/interface/exported-function declaration kinds, export-modifier detection) — the type/interface sibling of `rules.rs`'s existing `LangRuleConfig`. | `src/symbol_extract.rs`. |
| `PruningSummary` | Struct recording what a `build_model` run excluded and why (`include_private`, `excluded_dirs`, `generated_files_skipped`, `private_symbols_skipped`, `files_with_parse_errors`, `unsupported_language_files`, `total_files_scanned`) — embedded in `ArchModel` so a consumer never mistakes "pruned" for "doesn't exist," and never mistakes "no supported language in this file" for "no code here." | Satisfies the UX research's "emotional job: confidence the answer is complete" finding. `unsupported_language_files`/`total_files_scanned` close pre-mortem P1 #1: a repo that's mostly Rust/C++/etc. (no in-scope language) must not produce a confident-looking-but-near-empty model with no signal that most of the repo wasn't represented. |
| `CachedModel` | The in-process cache entry: an `ArchModel` plus the `Vec<(PathBuf, Stamp)>` file-stamp set it was built from, used to decide whether a cache hit is still valid. | Owned by `KibitzerServer` and `Backend`, per ADR-002. Not persisted, not shared across processes. |
| `Stamp` | Existing `(mtime_secs, mtime_nanos, len)` fingerprint type from `src/cache.rs`, reused (not reimplemented) as the per-file invalidation unit for `CachedModel`. | |
| `build_model` | `pub fn build_model(repo_root: &Path, files: &[(PathBuf, String)], import_graph: &ImportGraph, config: &PruneConfig) -> Result<ArchModel>` — the pure orchestration function. No file walking, no disk reads, no config-file reading; callers pass already-collected paths paired with already-read source text (matching `extract_symbols_for_file`'s existing pure shape). | `src/arch_model.rs`. |
| `extract_symbols_for_file` | `pub fn extract_symbols_for_file(language: Language, source: &str, tree: &Tree) -> Vec<SymbolNode>` — the per-file entry point `build_model` and LSP's `document_symbol` both call, so a single-file request (LSP) doesn't need a whole-repo model built first. | `src/symbol_extract.rs`. |
| `PruneConfig` | Input to `build_model`: `include_private: bool` plus (future-extensible) exclusion knobs. Distinct from `PruningSummary`, which is the *output* record of what was excluded. | |
| `ArchitectureAction` | New `clap::Subcommand` enum: `Export { .. } \| Diagram { .. }` under a new `Command::Architecture` variant in `main.rs`. | Two CLI verbs, not one command with a mode flag, per UX research. |
| `NodeQuery` | The single `node: String` parameter on the MCP `get_architecture_node` tool — resolved first against `ArchModel::package`, then against symbol `id`s, mirroring `Grep`'s "one scoped lookup" semantics rather than a typed union the caller has to pick a variant for. | `src/mcp.rs`. |

---

## Pattern Decisions

| Component | Pattern Chosen | Source | Alternative Rejected | Reason |
|-----------|---------------|--------|---------------------|--------|
| Per-language symbol extraction (`symbol_extract.rs`) | Strategy (GoF), selected via a table-driven `LangSymbolConfig` dispatch — mirrors `rules.rs::lang_config` | GoF; this codebase's own precedent | A generic tree-sitter `.scm` Query DSL / `tree-sitter-graph` walker | Pitfalls research: node-shape divergence (field vs. positional children, Kotlin's missing field names, Go's grouped params) doesn't shrink under a query DSL, it relocates; introduces a tree-sitter API surface unused elsewhere in kibitzer |
| Shared model home (`arch_model.rs` + `symbol_extract.rs`) | Pure/I-O split, following `install.rs`'s `run_install`/`merge_hook` precedent | This codebase's own precedent | Fold model-building logic into each consumer (`mcp.rs`, `lsp.rs`) inline | Requirements.md's core risk is "4 divergent implementations instead of 4 views on one thing" — inlining reproduces exactly that risk |
| `ArchModel`'s serialized shape | Bespoke nested `serde` structs (`BTreeMap<String, PackageNode>`) | `stack.md` research | `petgraph` with `serde-1` feature | petgraph's serde output is a flat node-list/edge-list encoding, not the nested jq-friendly tree the requirements' "greppable/jq-able" success metric asks for; also a net-new dependency (confirmed absent from `Cargo.lock`) |
| Diagram renderer (`arch_diagram.rs`) | Hand-rolled Mermaid `graph TD` + `subgraph`-per-package text, with C4-*like* visual grouping — **not** real Mermaid `C4Component`/`C4Dynamic` notation | `build-vs-buy.md` research | Structurizr DSL / embedded Structurizr Lite renderer; real Mermaid `C4Component`/`C4Dynamic` notation | Structurizr models Container/Context levels this feature explicitly excludes from scope, and viewing it requires a Docker/JVM sidecar — breaks kibitzer's offline single-binary value prop. Real `C4Component` notation was rejected too: `build-vs-buy.md` found GitHub's built-in Mermaid renderer doesn't support Mermaid's C4 extension at all, so a real-C4-notation diagram pasted into a PR (the exact "shareable in a PR" purpose `ux.md`'s Social job names for this command) would render as raw unrendered text on GitHub specifically — `graph TD`/`subgraph` is the only choice that's actually shareable there. |
| Model caching (`KibitzerServer`, `Backend`) | In-process, in-memory-only **single-slot** cache (`Mutex<Option<(ModelCacheKey, CachedModel)>>`) keyed by `(repo_root, include_private)` — **not** `scope` — + a `Stamp`-set invalidation check | `architecture.md`/`pitfalls.md` research | Extend `daemon.rs`'s persisted per-file `Cache` schema; a `HashMap`-backed multi-entry cache keyed by `(repo_root, scope, include_private)` | `Cache` is per-file/mtime-keyed; a repo-scoped model needs either N entries + a merge step or one entry invalidated by any file touch — neither fits without new cache-schema work, and `lsp.rs` doesn't route through `daemon.rs` today anyway. Keying on `scope` was rejected because `build_model` itself is unscoped (Story 1.3.1) — the CLI export path already establishes "build once, filter many" via `.filtered()`; keying the cache on `scope` would fragment one repo's cache into a separate rebuild per distinct `scope` value a caller passes in a session, reintroducing the "recomputed every call" cost this cache exists to eliminate. A single slot (not a `HashMap`) needs no eviction policy: a key mismatch just replaces the one entry. See ADR-002. **Resolved, previously-open concern (flagged across two earlier review rounds): `ModelCache::get_or_build` still requires its caller to walk the directory tree and stat every file's `Stamp` on *every* call, cache hit or not — only the parse/extract step (the `build` closure itself) is skipped on a hit, not the walk.** This is an accepted, intentional v1 tradeoff, not an unresolved gap: a directory walk (`walk_and_collect_files`'s stat-only scan) is IO-cheap relative to per-file tree-sitter parsing, which *is* what this cache exists to skip and *is* cached via the file-stamp comparison. A walk-skipping mechanism (e.g. caching the file list itself between calls, or file-watcher-driven invalidation) is deferred as a natural v2 follow-up alongside ADR-002's daemon-RPC-sharing alternative, not built for v1 — see ADR-002's Consequences section. |
| MCP query tool response shape | Real structured JSON (`serde_json::to_string`), not the flat-`String` house convention | UX research | Match `list_checks`/`run_checks`/`architecture_assessment`'s flat-string convention | Defeats the "queryable, scoped answer" success metric — forces the agent to re-parse prose for fields it already has natively. See ADR-001. |
| LSP consistency model | Disk-snapshot (re-parse from disk on request/`did_save`), matching the existing diagnostics precedent (`src/lsp.rs:88-95`) | `architecture.md` research | Live-buffer via LSP incremental sync | Requirements.md scopes real-time incremental updates as explicitly out of scope; inventing a better consistency model for symbols alone than diagnostics already have leaves two staleness models in one file |
| Minimization/pruning default | Exported/public-only symbols by default, per-language export-rule table, opt-in `include_private` | `pitfalls.md` research (duplicate_code.rs `MIN_OCCURRENCES` precedent) | Show every symbol unfiltered by default | Matches `duplicate_code.rs`'s lesson (commit `c3e6719`: unfiltered output was too noisy in practice) and industry precedent (`go doc`, TS `--declaration`, Javadoc all default to exported-only) |
| Language coverage rollout | 3-phase order: Go/TS/Tsx/JS → Python → Java/Kotlin | Requirements.md Rabbit Holes + this plan | Full 7-language parity in one phase | Validates the shared-model design against the 4 languages `import_graph.rs` already partially covers before tackling Kotlin's highest-risk grammar (no field names) |
| Generic/templated symbol identity | Collapse to base name, drop type parameters (`F[T any]` → `F`) | `pitfalls.md` research | Full generic-instantiation-aware identity | Zero existing precedent in this codebase for type identity at all; full generic modeling is unbounded scope not requested by requirements.md |
| `SymbolKind`, `ModelLevel` | Sum type / sealed enum | type-driven-design | Raw `&str` "kind"/"level" fields | Compiler-enforced exhaustive handling across 4 consumers instead of stringly-typed comparisons that can silently miss a variant |
| Symbol node identity (`SymbolNode::id`) | Deterministic, owner-qualified string: `"{package_path}::{parent}.{name}"` when `parent.is_some()` (methods), else `"{package_path}::{name}"` (types, interfaces, free functions) — not a random UUID | type-driven-design | Random UUID per symbol; a flat `"{package_path}::{name}"` for every symbol kind regardless of owner | Requirements' "greppable/jq-able" success metric wants IDs re-derivable across runs without a lookup table; also sets up (without building) future drift-diffing noted as an unstated need in `features.md`. A flat scheme was rejected because it collides for two same-named methods on different types in one package — routine in Go/TS/Java (Story 1.2.2's uniqueness AC) — which broke `get_architecture_node`'s exact-lookup contract (Story 3.1.2). |
| CLI/MCP/LSP orchestration functions (`arch_export.rs`, `mcp.rs` handlers, `lsp.rs` handlers) | Service Layer (PoEAA): thin functions that assemble inputs and call the pure `build_model`/`extract_symbols_for_file` domain functions | PoEAA (Fowler) | Transaction Script per consumer (each interface inlines its own walk+parse+build) | Matches "one model, multiple views"; a Transaction Script per consumer is the exact divergence risk requirements.md flags |

---

## Observability Plan
- **Logs**: `kibitzer architecture export`/`diagram` print a one-line summary on success
  (package count, symbol count, files skipped as generated/vendored, files skipped due to
  parse errors) to stdout, matching `install.rs`'s existing `println!` convention — no new
  logging framework.
- **Metrics**: none — local dev tool, no metrics/alerts infra exists today (per
  requirements.md's Observability Requirements).
- **Alerts**: not applicable.

## Performance Target
requirements.md's NFR section asks Phase 2 research to set a concrete target; none of the
research docs did, so it's set here provisionally: **`kibitzer architecture export`
(no `--scope`) on a realistic mid-size multi-language repo completes in well under 5 seconds.**
Kibitzer's own repo is *not* the benchmark target — it's Rust, which isn't among this feature's
7 in-scope languages, and has no meaningful Go/TS/Tsx/JS/Python/Java/Kotlin source of its own
(the earlier "~90 source files" figure was wrong on both count and premise; see Task 1.3.1f).
The comparison to `architecture_assessment`'s "order of magnitude" is an unverified assumption,
not a measurement — no research doc records `architecture_assessment`'s actual runtime (Phase 4
cross-artifact consistency finding). **This 5-second figure must be validated against Task
1.3.1f's first real benchmark run before it's treated as a committed SLO rather than a working
assumption** — if the benchmark shows it's unrealistic, revise this section rather than the test.
This is also `design/ux.md`'s UX Acceptance Criterion 3.

## Risk Control
- **Feature flag**: not gated — purely additive (new command/tools/model), doesn't change
  behavior of any existing check, hook, or CLI command, matching requirements.md's Risk
  Control section.
- **Rollback procedure**: standard revert via PR close + revert commit.
- **Staged rollout**: full rollout on merge (solo-maintained OSS project, no user cohorts).
- **Superseded risk, now mitigated by design**: an earlier draft of this plan had the LSP
  `symbol` (workspace-symbol) handler build `ArchModel` synchronously, inline, on the first
  query per session — architecture/adversarial review flagged this as contradicting
  `pitfalls.md`/`research/ux.md`'s explicit "background index, never recomputed inline with
  the request" recommendation, and as risking blocking other concurrent LSP requests on the
  same tokio runtime (no handler used `spawn_blocking`). Epic 4.3 (Stories 4.3.0/4.3.1) now
  builds the index in a background `spawn_blocking` task starting at `initialized()`;
  `symbol` itself never triggers a build. Residual, accepted v1 limitation: on a very large
  repo, a `workspace/symbol` call that arrives before the background build finishes gets an
  explicit "still indexing" synthetic result (not real matches, not blocked, not an error) —
  see Story 4.3.1's AC. This is unrelated to requirements.md's "no real-time incremental
  updates" exclusion, which concerns live re-indexing on every file change, not the one-time
  startup index build.

## Unresolved Questions
- [ ] Barrel-file (`index.ts`-style) re-export resolution to the *originating* file,
      rather than the barrel's own directory — `import_graph.rs`'s existing JS/TS behavior
      (resolves to the barrel directory) is preserved as-is for parity in Phase 1/5; not a
      blocker for any v1 story, tracked as a follow-up if a user reports it as confusing
      in the symbol tree. Owner: revisit on user report, no story depends on it.

All other open questions from requirements.md (minimization rules, language coverage
order, model home, caching strategy, LSP capabilities, output bounding, MCP naming/schema,
CLI conventions) are resolved concretely above and in the Epics below — none are deferred
further.

## Dependency Visualization

```
Phase 1: Shared Model Foundation (Go/TS/Tsx/JS)
  Epic 1.1 Domain types ─┐
  Epic 1.2 Symbol extraction (Go/TS/Tsx/JS) ─┤
  Epic 1.3 build_model + pruning + query API ─┼─→ Epic 1.4 In-memory ModelCache
                                               │
        ┌──────────────────────────────────────┴───────────────────────────┐
        ▼                          ▼                          ▼            ▼
Phase 2: CLI              Phase 3: MCP tools        Phase 4: LSP symbols   (diagram
  Epic 2.1 export           Epic 3.1 list/get          Epic 4.1 capabilities shares
  Epic 2.2 diagram ◄────────────────────────────────── (reads ArchModel)   Phase 1's
        (arch_diagram.rs reused by Epic 2.2 only;                          model —
         Epics 3/4 consume ArchModel directly)                             no cross-
                                                                            phase edge
                                                                            beyond it)
                                                        Epic 4.2 document_symbol
                                                        Epic 4.3.0 background index
                                                          (spawn_blocking @ initialized(),
                                                           needs Epic 1.4's cache)
                                                        Epic 4.3.1 workspace symbol
                                                          (reads Epic 4.3.0's IndexState;
                                                           never builds inline itself)

Phase 5: Extended Language Coverage (independent per language, each mirrors Epic 1.2/1.3's
         shape; depends only on Phase 1, not on Phases 2-4)
  Epic 5.1 Python  ─┐
  Epic 5.2 Java     ─┼─→ (each extends symbol_extract.rs + import_graph.rs; Phases 2-4
  Epic 5.3 Kotlin   ─┘    automatically pick up new languages with no interface changes)
```

## MVP Cut Point

Phases 1–4 (Go/TS/Tsx/JS symbol+import coverage, plus all three consumer interfaces — CLI
export/diagram, MCP query tools, LSP document/workspace symbols) constitute a shippable v1
on their own: every success metric in requirements.md (queryable export, CLI artifact, LSP
browsing, one shared model across all views) is satisfied without Phase 5. Phase 5
(Python/Java/Kotlin symbol+import extraction) is structurally severable — the Dependency
Visualization diagram above shows it depends only on Phase 1's shared model, not on Phases
2–4 — and ships as a fast-follow with zero changes to `arch_model.rs`/`mcp.rs`/`lsp.rs`/the
CLI. If the Large (3–6 week) appetite runs short, Phase 5 is the first and only scope to
cut; Phases 1–4 are not further divisible without breaking the "one model, multiple views"
requirement.

---

## Phase 1: Shared Model Foundation

### Epic 1.1: Domain types
**Goal**: Define `ArchModel` and its constituent types as pure, serde-derived structs with
no I/O, giving every later phase a stable shape to build against.

#### Story 1.1.1: `ArchModel` and friends compile and round-trip through serde
**As a** kibitzer maintainer, **I want** `ArchModel`/`PackageNode`/`SymbolNode`/`SymbolKind`/
`ModelLevel`/`PruningSummary` defined in `src/arch_model.rs` with `Serialize`+`Deserialize`,
**so that** every consumer (CLI export, MCP tools, LSP mapper, diagram renderer) shares one
type instead of reimplementing it.

**Acceptance Criteria**:
- `ArchModel { repo_root: PathBuf, packages: BTreeMap<String, PackageNode>, import_edges: Vec<ImportEdge>, pruning: PruningSummary }` serializes to JSON with stable key ordering.
  - *Given* an `ArchModel` with two packages `"a"` and `"b"` inserted in reverse order, *When* it's serialized via `serde_json::to_string`, *Then* `"a"` appears before `"b"` in the output (BTreeMap ordering, not insertion order).
- `SymbolNode { id: String, name: String, kind: SymbolKind, file: PathBuf, line: usize, exported: bool, parent: Option<String> }` round-trips through `serde_json::to_string` + `from_str` without field loss.
  - *Given* a `SymbolNode` with `id: "pkg/foo::Bar"`, `kind: SymbolKind::Type`, `parent: None`, *When* serialized then deserialized, *Then* the result equals the original (`PartialEq` derived).
- `SymbolKind` is a 4-variant enum (`Type`, `Interface`, `Function`, `Method`) serialized as lowercase strings (`#[serde(rename_all = "lowercase")]`, matching `Severity`'s existing convention in `config.rs`).
  - *Given* `SymbolKind::Function`, *When* serialized, *Then* the JSON value is `"function"`.
- `ModelLevel` is a 2-variant enum (`Component`, `Code`), same rename convention.
- `ImportEdge` (`src/import_graph.rs`) gains `#[derive(Serialize, Deserialize)]` (currently only `Debug, Clone, PartialEq, Eq`) so `ArchModel::import_edges` can serialize without a parallel type.
  - *Given* the existing `ImportEdge { from: "a", to: "b", file: PathBuf::from("a.go"), line: 3 }`, *When* serialized, *Then* it produces `{"from":"a","to":"b","file":"a.go","line":3}` with no new fields.

**Files**: `src/arch_model.rs` (new), `src/import_graph.rs` (add derive), `src/main.rs` (add `mod arch_model;`)

##### Task 1.1.1a: Add `Serialize`/`Deserialize` derives to `ImportEdge` (~2 min)
- In `src/import_graph.rs`, change `#[derive(Debug, Clone, PartialEq, Eq)]` on `ImportEdge` to add `Serialize, Deserialize`; add `use serde::{Deserialize, Serialize};` at the top.
- Files: `src/import_graph.rs`

##### Task 1.1.1b: Create `src/arch_model.rs` with `SymbolKind` and `ModelLevel` (~3 min)
- Define both enums with `#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]` and `#[serde(rename_all = "lowercase")]`.
- Files: `src/arch_model.rs`

##### Task 1.1.1c: Add `SymbolNode`, `PackageNode`, `PruningSummary`, `ArchModel` structs (~5 min)
- `SymbolNode`: fields per Story 1.1.1's acceptance criteria; derive `Debug, Clone, PartialEq, Serialize, Deserialize`.
- `PackageNode { path: String, files: Vec<PathBuf>, symbols: Vec<SymbolNode> }`.
- `PruningSummary { include_private: bool, excluded_dirs: Vec<String>, generated_files_skipped: usize, private_symbols_skipped: usize, pruned_symbol_ids: Vec<String>, files_with_parse_errors: Vec<PathBuf>, unsupported_language_files: usize, total_files_scanned: usize }` — `files_with_parse_errors` is a path list (not just a count), so a consumer can identify *which* files didn't parse cleanly, not only how many. `unsupported_language_files` counts files under `--scope` that have no recognized extension at all (distinct from a parse error — the file was never even attempted); `total_files_scanned` is the denominator so a consumer can compute the unsupported fraction without a second pass over the repo. `pruned_symbol_ids` is the `SymbolNode::id` of every symbol excluded specifically by `include_private: false` (the ids counted in `private_symbols_skipped`) — captured at zero extra extraction cost since `build_model` already visits every extracted symbol before deciding whether to keep it (Task 1.3.1c just also pushes the id when it drops one). This is what closes pre-mortem P2 #2: Story 3.1.1's `possibly_pruned` and Story 3.1.2's `exists_but_pruned` checks read this field directly — no second build, no second `PruneConfig` pass, no re-run of extraction.
- `ArchModel` per Story 1.1.1.
- Files: `src/arch_model.rs`

##### Task 1.1.1d: Register the module and add round-trip serde tests (~4 min)
- Add `mod arch_model;` to `src/main.rs` (alphabetically, after `architecture_checks`).
- Add `#[cfg(test)] mod tests` in `arch_model.rs` covering the three GWT examples above.
- Files: `src/main.rs`, `src/arch_model.rs`

---

### Epic 1.2: Symbol extraction infrastructure (Go, TypeScript, Tsx, JavaScript)
**Goal**: Extract `SymbolNode`s (types, interfaces, exported functions, methods) from
already-parsed ASTs for the 4 languages `import_graph.rs` already partially covers,
following `rules.rs::LangRuleConfig`'s table-driven shape.

#### Story 1.2.1: `LangSymbolConfig` table exists for Go/TS/Tsx/JS
**As a** kibitzer maintainer, **I want** a per-language table of type/interface/function
declaration node kinds and an export-detection rule, **so that** symbol extraction doesn't
hardcode grammar-specific strings inline in the walker.

**Acceptance Criteria**:
- `LangSymbolConfig` has fields: `type_kinds: &'static [&'static str]`, `interface_kinds: &'static [&'static str]`, `function_kinds` (reuse `rules.rs`'s existing list per language, duplicated into this table since the two tables serve different rule sets and diverging later is expected), `name_finder: fn(Node) -> Option<Node>`, `is_exported: fn(Node, &str) -> bool` (second arg is source text, needed for Go's uppercase-identifier check).
  - *Given* `lang_symbol_config(Language::Go)`, *When* read, *Then* `type_kinds == &["type_declaration"]` and `is_exported` returns `true` for a `type_spec` named `Foo` and `false` for one named `foo`.
- Go: `type_declaration` wraps `type_spec` (struct, alias, or interface body) — `type_spec` with a `struct_type` child classifies as `SymbolKind::Type`; one whose child is `interface_type` classifies as `SymbolKind::Interface`; `function_declaration`/`method_declaration` classify as `Function`/`Method` respectively (method iff it has a `receiver` field).
  - *Given* Go source `type Reader interface { Read() }`, *When* extracted, *Then* one `SymbolNode` with `kind: Interface, name: "Reader", exported: true` is produced.
- TS/Tsx: `interface_declaration` → `Interface`; `type_alias_declaration`/`class_declaration` → `Type`; `function_declaration`/`method_definition` → `Function`/`Method` (method iff nested inside a `class_declaration`'s `class_body`); exported iff the declaration (or its enclosing `export_statement`) carries an `export` modifier.
  - *Given* TS source `export interface Shape { area(): number }`, *When* extracted, *Then* one `SymbolNode` with `kind: Interface, name: "Shape", exported: true`.
- JS: no `interface_declaration`/`type_alias_declaration` exist in the JS grammar (TS-only node kinds) — `type_kinds`/`interface_kinds` are empty for `Language::JavaScript`; only `class_declaration`/`function_declaration`/`method_definition` are extracted.
  - *Given* JS source `class Foo {}`, *When* extracted, *Then* one `SymbolNode` with `kind: Type, name: "Foo"` — no interface/type-alias symbols are ever produced for plain JS.
- Exported detection for JS/TS: a declaration is exported iff it (a) is a direct child of `export_statement`, or (b) `export_statement` has a `declaration` field pointing at it — verified against real `to_sexp()` output before finalizing, per the pitfalls research's "don't guess from grammar docs" lesson.

**Files**: `src/symbol_extract.rs` (new)

##### Task 1.2.1a: Dump real `to_sexp()` output for Go/TS/JS type & interface declarations (~5 min)
- Write a throwaway test (or reuse `cargo expand`-style scratch) parsing 4 fixtures (Go struct, Go interface, TS interface, TS exported class) and print `to_sexp()` + `field_name_for_child` for each — verify node kind names and field names match what Story 1.2.1's acceptance criteria assume before writing the extraction code, per the pitfalls research's explicit "verify, don't guess" lesson.
- Files: none committed (verification step; findings feed Task 1.2.1b's constants)

##### Task 1.2.1b: Define `LangSymbolConfig` struct and Go entry (~4 min)
- Struct definition + `lang_symbol_config(Language::Go) -> LangSymbolConfig` per Story 1.2.1.
- Files: `src/symbol_extract.rs`

##### Task 1.2.1c: Add TypeScript/Tsx/JavaScript entries (~5 min)
- `lang_symbol_config` match arms for `TypeScript`, `Tsx` (delegates to TypeScript's config with different `file_globs` — not needed here since this table has no `file_globs` field, unlike `LangRuleConfig`; Tsx and TypeScript can share the same config value directly), `JavaScript` (empty `type_kinds`/`interface_kinds`).
- Files: `src/symbol_extract.rs`

---

#### Story 1.2.2: `extract_symbols_for_file` walks the AST and produces `SymbolNode`s
**As a** kibitzer maintainer, **I want** one function that takes a parsed `Tree` and
returns every in-scope `SymbolNode`, **so that** both `build_model` (whole-repo) and the
LSP `document_symbol` handler (single-file) share the same extraction logic.

**Acceptance Criteria**:
- `extract_symbols_for_file(language: Language, source: &str, tree: &Tree, package_path: &str) -> Vec<SymbolNode>` walks `tree.root_node()` recursively, matching node kinds against `lang_symbol_config(language)`.
  - *Given* Go source with one exported function `func Do() {}` and one unexported `func do() {}`, *When* extracted with `include_private: true` semantics (both should appear at the extraction layer — pruning happens later in `build_model`, not here), *Then* both `SymbolNode`s are returned, `exported: true` and `false` respectively.
- Methods get `parent: Some(<owning type name>)` populated — Go via the `receiver` field's type name; TS/JS via the enclosing `class_declaration`'s name.
  - *Given* Go source `type T struct{}\nfunc (t T) M() {}`, *When* extracted, *Then* the `M` `SymbolNode` has `kind: Method, parent: Some("T".to_string())`.
- `SymbolNode::id` is owner-qualified and deterministic, per the Pattern Decisions table:
  `format!("{package_path}::{parent}.{name}")` when `parent.is_some()` (methods), else
  `format!("{package_path}::{name}")` (types, interfaces, free functions) — never random.
  - *Given* `package_path: "app/domain"`, a function named `Compute` with no parent, *When* extracted, *Then* `id == "app/domain::Compute"`.
  - *Given* `package_path: "app/domain"`, a method named `Compute` with `parent: Some("Widget")`, *When* extracted, *Then* `id == "app/domain::Widget.Compute"`.
- Two methods with the same name on different types in the same package produce distinct
  ids — the collision this owner-qualified scheme exists to prevent (a flat
  `"{package_path}::{name}"` id, as an earlier draft specified, collided here and broke
  `get_architecture_node`'s exact-lookup contract; see Story 3.1.2).
  - *Given* Go source in one package with `type A struct{}; func (a A) Close() {}` and `type B struct{}; func (b B) Close() {}`, *When* extracted, *Then* the two `SymbolNode`s have ids `"<pkg>::A.Close"` and `"<pkg>::B.Close"` — distinct, not colliding.
- Generic type parameters are stripped from the symbol's name for identity purposes (Go `func F[T any]()` → name `"F"`), per the Pattern Decisions table's generics decision.
  - *Given* Go source `func F[T any](x T) T { return x }`, *When* extracted, *Then* the resulting `SymbolNode.name == "F"` (no `[T any]` suffix).
- This function does **no** file I/O and takes no `GrammarCache` — the caller (Story 1.3.1's `build_model`, or the LSP handler) is responsible for parsing.

**Files**: `src/symbol_extract.rs`

##### Task 1.2.2a: Implement the generic recursive walk + Go extraction (~5 min)
- `extract_symbols_for_file` recurses `tree.root_node()`'s children, dispatching on `node.kind()` against the current language's `type_kinds`/`interface_kinds`/`function_kinds`; for Go, classify `type_spec` as `Type` vs. `Interface` by its child kind.
- Files: `src/symbol_extract.rs`

##### Task 1.2.2b: Method/parent detection for Go (receiver) (~4 min)
- Detect `method_declaration`'s `receiver` field, extract the receiver type's identifier (strip a leading `*` for pointer receivers), set `parent`.
- Files: `src/symbol_extract.rs`

##### Task 1.2.2c: TS/Tsx/JS extraction + method/parent detection (class body) (~5 min)
- Same walk for TS/Tsx/JS node kinds; a `method_definition` inside a `class_declaration`'s `class_body` gets `parent: Some(<class name>)`.
- Files: `src/symbol_extract.rs`

##### Task 1.2.2d: Generic-parameter stripping + owner-qualified `id` construction (~4 min)
- Strip a trailing `[...]`/`<...>` type-parameter list from the raw name text before constructing `SymbolNode::name`; build `id` as `package_path` + `::` + (if `parent.is_some()`, `parent` + `.`) + stripped name, per Story 1.2.2's id scheme.
- Files: `src/symbol_extract.rs`

##### Task 1.2.2e: Unit tests for all 6 acceptance criteria, including the same-name/different-owner uniqueness case (~6 min)
- One test per GWT example in Story 1.2.2, plus a TS/JS parity pair.
- Files: `src/symbol_extract.rs`

---

### Epic 1.3: `build_model` orchestration, pruning, and the query API
**Goal**: Assemble `PackageNode`s + apply pruning rules + provide the `package()`/
`find_symbol()`/`filtered()` query methods every consumer calls.

#### Story 1.3.1: `build_model` assembles a pruned `ArchModel` from files + an `ImportGraph`
**As a** kibitzer maintainer, **I want** one pure function that turns a file list and an
already-built `ImportGraph` into a complete `ArchModel`, **so that** CLI/MCP/LSP callers
share identical assembly logic and the double-import-graph-build bug in `mcp.rs` isn't
repeated.

**Acceptance Criteria**:
- `build_model(repo_root: &Path, files: &[(PathBuf, String)], import_graph: &ImportGraph, prune: &PruneConfig) -> Result<ArchModel>` takes each file's path paired with its already-read source text — **no disk I/O inside `build_model` itself**; the caller (`arch_export.rs`, the `ModelCache` build closures in `mcp.rs`/`lsp.rs`) reads files before calling, matching `extract_symbols_for_file`'s already-pure shape and the plan's own Service Layer pattern decision. It groups `files` by their package-directory key (same key convention `import_graph.rs` uses — Go's module-relative path, JS's `dir_key`), parses each with a fresh `GrammarCache` per file (matching `GrammarCache`'s documented one-cache-per-file contract), and calls `extract_symbols_for_file` per file.
  - *Given* two Go files under `app/domain/` and one under `app/handlers/` (as pre-read `(path, source)` pairs), plus an `ImportGraph` with a `domain → handlers` edge, *When* `build_model` runs, *Then* the resulting `ArchModel.packages` has exactly 2 keys (`"app/domain"`, `"app/handlers"`) and `import_edges` contains the one edge unchanged (not rebuilt).
- Files whose extension has no `Language` mapping (e.g. `.md`, `.json`) contribute no package entry, but are **not silently dropped**: each one increments `PruningSummary.unsupported_language_files`, and every file passed in (mapped or not) counts toward `PruningSummary.total_files_scanned`, so a caller can compute what fraction of the repo has no in-scope language (pre-mortem P1 #1 — a repo that's mostly an unsupported language must not look like a complete, confident export).
  - *Given* a directory with one `.go` file and one `README.md`, *When* `build_model` runs, *Then* the resulting `PackageNode.files` lists only the `.go` file, `PruningSummary.unsupported_language_files == 1`, and `PruningSummary.total_files_scanned == 2`.
- `PruneConfig.include_private: false` (the default) excludes `SymbolNode`s where `exported == false` from the final `PackageNode.symbols`, increments `PruningSummary.private_symbols_skipped` by the count excluded, and pushes each excluded symbol's `id` onto `PruningSummary.pruned_symbol_ids` — they are never silently dropped without a trace, and their ids stay cheaply queryable for the "exists but pruned" checks in Story 3.1.1/3.1.2.
  - *Given* a Go file with 3 exported and 2 unexported top-level functions, *When* `build_model` runs with `include_private: false`, *Then* the package's `symbols` has 3 entries, `ArchModel.pruning.private_symbols_skipped == 2`, and `ArchModel.pruning.pruned_symbol_ids` contains exactly the 2 excluded functions' ids.
- `PruneConfig.include_private: true` includes every extracted symbol regardless of `exported`.
- A file whose first two lines (case-insensitive) contain `"do not edit"` or `"code generated"` is skipped entirely (no symbols extracted, not counted in `PackageNode.files`), and `PruningSummary.generated_files_skipped` increments.
  - *Given* a Go file starting with `// Code generated by protoc-gen-go. DO NOT EDIT.`, *When* `build_model` runs, *Then* that file contributes zero symbols and `generated_files_skipped == 1`.
- A file whose tree-sitter parse tree has any error node (`tree.root_node().has_error()`,
  per `GrammarCache::parse`'s error-recovery behavior — `parse` (`src/checker.rs:157`)
  returns `Ok(Tree)` even for malformed source, never erroring on a syntactically broken
  file) contributes **zero** symbols to the model (skipped, same treatment as a generated
  file) and its path is recorded in `PruningSummary.files_with_parse_errors`; it is never
  partially/best-effort extracted, and `build_model` never fails or panics because of it.
  - *Given* one well-formed Go file and one Go file with a deliberately broken/truncated function body, *When* `build_model` runs, *Then* the resulting `ArchModel` contains only the well-formed file's symbols, `PruningSummary.files_with_parse_errors` has exactly one entry (the broken file's path), and `build_model` returns `Ok(_)`, not `Err(_)`.

**Files**: `src/arch_model.rs`

##### Task 1.3.1a: Define `PruneConfig` and the generated-file heuristic (~3 min)
- `PruneConfig { include_private: bool }`; `fn looks_generated(source: &str) -> bool` checking the first 2 lines case-insensitively.
- Files: `src/arch_model.rs`

##### Task 1.3.1b: Group `(path, source)` pairs by package key and skip unmapped extensions (~4 min)
- Reuse `import_graph.rs`'s extension-dispatch pattern (`has_ext`/`is_js_like`) to map each file's path to a `Language`; only files with a recognized `Language` are added to `PackageNode.files` and considered for extraction (keeps the artifact focused on source kibitzer actually parsed, not every non-code file in the directory). Every file the caller passed in is counted in `total_files_scanned`; a file whose extension maps to no recognized `Language` increments `unsupported_language_files` instead of being silently dropped with no trace (pre-mortem P1 #1).
- Files: `src/arch_model.rs`

##### Task 1.3.1c: Per-file parse-error check + extract + generated-file skip + private pruning (~7 min)
- For each recognized `(path, source)` pair — no `std::fs::read_to_string` here, `source` was already read by the caller: skip if `looks_generated(source)`, incrementing `generated_files_skipped`; else `GrammarCache::new()` + `cache.parse(language, source)`, then check `tree.root_node().has_error()` — if true, record `path` in `PruningSummary.files_with_parse_errors` and skip extraction for that file entirely (no partial/best-effort symbols); otherwise call `extract_symbols_for_file`, filter by `include_private` — for each symbol dropped because `!include_private && !exported`, increment `private_symbols_skipped` and push its `id` onto `pruned_symbol_ids` — accumulate the kept symbols into the owning `PackageNode`.
- Files: `src/arch_model.rs`

##### Task 1.3.1d: Assemble `ArchModel` and `PruningSummary`, including the unsupported-language fraction (~3 min)
- Wire `generated_files_skipped`/`private_symbols_skipped`/`files_with_parse_errors`/`unsupported_language_files`/`total_files_scanned` into `PruningSummary` (the last two per Story 1.3.1's updated "not silently dropped" AC above); attach `import_graph.edges` verbatim to `ArchModel.import_edges`.
- Files: `src/arch_model.rs`

##### Task 1.3.1e: Tests for the 6 acceptance criteria (~6 min)
- Files: `src/arch_model.rs`

##### Task 1.3.1f: Benchmark `build_model` against a realistic multi-language fixture + a synthetic larger fixture (~5 min)
- kibitzer's own repo is Rust (24 `.rs` files) — Rust isn't among this feature's 7 in-scope languages, and the repo has essentially zero real Go/TS/Tsx/JS/Python/Java/Kotlin source (only the 29 tiny `examples/` fixture files created for a separate feature), so it is not a meaningful performance benchmark for this task; correcting the earlier "~90 source files" claim, which was wrong on both the count and the premise. Instead: assemble or reuse a realistic external multi-language fixture (e.g. a mid-size open-source Go or TS project vendored into a test-fixtures directory, or a synthetic fixture generated to a representative file-count/size distribution) and assert it completes well under the 5-second Performance Target above, as a coarse regression guard (wall-clock assertion in a `#[test]`, not a micro-benchmark suite); also run against a synthetic ~2,000-file fixture to sanity-check large-repo scaling, no hard assertion required there beyond "completes, doesn't panic." The Performance Target section above should be revisited against this benchmark's first real run rather than treated as pre-confirmed (per the Phase 4 cross-artifact consistency finding).
- Files: `src/arch_model.rs`

---

#### Story 1.3.2: Query API — `package()`, `find_symbol()`, `filtered()`
**As a** consumer (MCP tool, LSP handler, diagram renderer), **I want** `ArchModel` methods
for scoped lookup instead of hand-rolling `BTreeMap`/`Vec` traversal in every call site,
**so that** filtering logic (scope glob, level) lives in one place.

**Acceptance Criteria**:
- `ArchModel::package(&self, path: &str) -> Option<&PackageNode>` is an exact-key `BTreeMap` lookup.
  - *Given* an `ArchModel` with a package at `"app/domain"`, *When* `.package("app/domain")` is called, *Then* `Some(&PackageNode)` is returned; `.package("app/other")` returns `None`.
- `ArchModel::find_symbol(&self, name: &str) -> Vec<(&str, &SymbolNode)>` returns every `(package_path, &SymbolNode)` pair whose `SymbolNode.name` exactly equals `name`, across all packages.
  - *Given* two packages each with a function named `Init`, *When* `.find_symbol("Init")` is called, *Then* 2 results are returned, one per package.
- `ArchModel::filtered(&self, scope: &[String], level: ModelLevel) -> ArchModel` reuses `crate::glob::matches_scope` (not a new glob dialect) to keep only packages whose path matches `scope` (empty `scope` keeps everything, matching `matches_scope`'s existing empty-means-all semantics); when `level == ModelLevel::Component`, every `PackageNode.symbols` is cleared (component view has no code-level detail) while `packages`/`import_edges` stay.
  - *Given* an `ArchModel` with packages `"web/ui"` and `"server/api"`, *When* `.filtered(&["web/**".to_string()], ModelLevel::Code)` is called, *Then* the result has only `"web/ui"`, with its symbols intact.
  - *Given* the same model, *When* `.filtered(&[], ModelLevel::Component)` is called, *Then* both packages remain but every `PackageNode.symbols` is empty.

**Files**: `src/arch_model.rs`

##### Task 1.3.2a: `package()` and `find_symbol()` (~3 min)
- Files: `src/arch_model.rs`

##### Task 1.3.2b: `filtered()` (~4 min)
- Files: `src/arch_model.rs`

##### Task 1.3.2c: Tests for all 4 acceptance criteria (~4 min)
- Files: `src/arch_model.rs`

---

### Epic 1.4: In-process `ModelCache`
**Goal**: Give `KibitzerServer` and `Backend` a shared, reusable cache implementation
(not duplicated logic in each) per ADR-002.

#### Story 1.4.1: `ModelCache` returns a cached `ArchModel` when the underlying files haven't changed
**As a** kibitzer maintainer, **I want** one small cache type both the MCP server and the
LSP backend can own, **so that** the invalidation logic (stamp-set comparison) isn't
written twice.

**Acceptance Criteria**:
- `ModelCache::get_or_build(&self, key: ModelCacheKey, files: &[PathBuf], build: impl FnOnce() -> Result<ArchModel>) -> Result<Arc<ArchModel>>` where `ModelCacheKey { repo_root: PathBuf, include_private: bool }` (derives `PartialEq`, `Eq`, `Hash`) — **no `scope` field**. `build_model` itself is unscoped (Story 1.3.1); `scope` (and `level`) are applied per-call via `ArchModel::filtered` against the cached, unscoped model, never part of the cache key — matching the CLI export path's already-correct "build once, filter many" pattern (Task 2.1.1b).
  - *Given* an empty `ModelCache` and a `key`/`files` pair, *When* `get_or_build` is called, *Then* `build` runs exactly once and the result is cached under `key`.
- A second call with the same `key` and unchanged file stamps returns the cached `Arc<ArchModel>` without invoking `build` again — including when the caller intends to apply a *different* `scope` afterward, since `scope` isn't part of `key`.
  - *Given* the state from the previous example, *When* `get_or_build` is called again with the same `key`/`files` and none of `files`' on-disk `Stamp`s have changed, *Then* `build` is not invoked a second time (verified via a call-counting closure in the test), regardless of what `scope` the caller subsequently applies via `.filtered()`.
- If any file in `files` has a different `Stamp` than what was cached (or a file was added/removed from the list), the cache is treated as stale: `build` runs again and replaces the entry.
  - *Given* the cached state from above, *When* one of `files`' mtimes changes on disk and `get_or_build` is called again, *Then* `build` runs a second time and the new result replaces the old cache entry.
- A call with a `key` that doesn't match the cache's current entry (different `repo_root`, or `include_private` flipped) replaces the single cached entry outright — the old `ArchModel` is dropped, not retained alongside the new one.
  - *Given* a cached entry for `include_private: false`, *When* `get_or_build` is called with `include_private: true`, *Then* `build` runs again and the cache now holds only the `include_private: true` result.
- The cache is a plain struct wrapping `Mutex<Option<(ModelCacheKey, CachedModel)>>` — a
  **single slot**, not a `HashMap` — so there's no eviction/size-bound policy to design (a
  key mismatch just replaces the one slot); no persistence, no daemon RPC, matching
  ADR-002.

**Files**: `src/arch_model.rs` (or a small new `src/arch_cache.rs` if `arch_model.rs` is getting large by this point — decide at implementation time based on line count; default to co-locating in `arch_model.rs` since `ModelCache` is a thin wrapper, not a new domain concept)

##### Task 1.4.1a: Define `ModelCacheKey { repo_root, include_private }`, `CachedModel`, `ModelCache` as a single-slot `Mutex<Option<(ModelCacheKey, CachedModel)>>` (~4 min)
- Files: `src/arch_model.rs`

##### Task 1.4.1b: Implement `get_or_build` with `Stamp`-set comparison (reuse `cache.rs`'s stamping, exposed as `pub(crate) fn stamp`) (~5 min)
- `src/cache.rs`'s `stamp()` function is currently private (`fn stamp`) — change to `pub(crate) fn stamp` so `arch_model.rs` can reuse it instead of reimplementing mtime/len fingerprinting.
- Files: `src/cache.rs` (visibility change only), `src/arch_model.rs`

##### Task 1.4.1c: Tests for the 3 acceptance criteria (~5 min)
- Files: `src/arch_model.rs`

---

## Phase 2: CLI Export & Diagram

### Epic 2.1: `kibitzer architecture export`
**Goal**: A CLI command writing `ArchModel` as pretty-printed JSON to a file, with
`--dry-run` matching `install.rs`'s convention.

#### Story 2.1.1: `kibitzer architecture export --path <p> --out <file>` writes a valid `ArchModel` JSON file
**As a** human developer or CI pipeline, **I want** a one-shot CLI command producing a
committable, diffable artifact, **so that** I can grep/jq/version the repo's architecture
without kibitzer re-parsing it every time.

**Acceptance Criteria**:
- Given a repo with a valid `.claude/inspect.json`, running `kibitzer architecture export --path . --out arch.json` writes `arch.json` containing `serde_json::to_string_pretty(&model)? + "\n"` (pretty-printed, trailing newline, stable key order via `preserve_order`), matching `install.rs:35`'s exact convention.
  - *Given* a small Go fixture repo with one package, *When* `kibitzer architecture export --path <fixture> --out arch.json` runs, *Then* `arch.json` exists, parses as valid JSON, and its top-level `"packages"` key lists the one package.
- `--dry-run` prints the would-be file contents to stdout and does not write `arch.json`, matching `install.rs`'s `--dry-run` flag name and "don't write the file" semantics. **Deviation from `install.rs`, confirmed correct at implementation time (spec-compliance sweep, post-Phase-2):** `install.rs` prefixes its dry-run output with `"[kibitzer] would write {path}:\n"`; `arch_export.rs` deliberately omits that prefix and prints pure `serde_json::to_string_pretty` output with nothing else on stdout, so `--dry-run` stays pipeable to `jq` — matching this feature's own "queryable, jq-able" success metric (requirements.md), which a leading non-JSON line would break. `run_export_dry_run_prints_json_and_writes_no_file` (`src/arch_export.rs`) asserts this by parsing all of stdout as JSON directly.
  - *Given* the same fixture, *When* run with `--dry-run`, *Then* stdout is valid JSON on its own (no prefix line) and no `arch.json` file is created on disk.
- `--scope <glob>` restricts the exported packages via `ArchModel::filtered`, reusing `matches_scope`.
  - *Given* a fixture with packages `web/ui` and `server/api`, *When* run with `--scope "web/**"`, *Then* the exported JSON's `"packages"` has only `web/ui`.
- `--include-private` includes unexported symbols (default: excluded, per Story 1.3.1).
- A repo with zero files kibitzer can parse (no `.go`/`.ts`/`.tsx`/`.js`/`.py`/`.java`/`.kt` files) prints `"no supported languages found under <path>; nothing to export"` and exits `ExitCode::SUCCESS` — not an error, per UX research.
  - *Given* a fixture directory containing only a `README.md`, *When* `kibitzer architecture export --path <fixture> --out arch.json` runs, *Then* stdout contains the exact message above, no `arch.json` is written, and the process exit code is 0.
- `--scope <glob>` that matches zero packages in an otherwise non-empty repo (distinct from
  the zero-supported-languages case above) prints `no packages matched scope "<glob>" under
  <path>; nothing to export`, writes no `arch.json`, and exits `ExitCode::SUCCESS` — per
  `design/ux.md`'s flagged "Open UX gap" (a scoped query with zero matches must name the
  scope so the caller knows *why* nothing matched, not just that nothing did).
  - *Given* a fixture with packages only under `web/` and `server/`, *When* run with `--scope "nonexistent/**"`, *Then* stdout contains exactly `no packages matched scope "nonexistent/**" under <path>; nothing to export`, no `arch.json` is written, and the process exit code is 0.
- Exit code is always `ExitCode::SUCCESS` on a successful write regardless of model contents (no pass/fail concept for export, per UX research); nonzero only on genuine I/O/config errors (propagated via `anyhow`/`Result`, same as every other kibitzer subcommand).
- When `PruningSummary.unsupported_language_files` is ≥50% of `PruningSummary.total_files_scanned` (and `total_files_scanned > 0`), stdout additionally prints a one-line warning *after* the normal write confirmation: `` warning: N/M files scanned have no supported language extension — this export may not represent most of the repo `` — surfaced per-run rather than silently discoverable only by inspecting `arch.json`'s `pruning` field (pre-mortem P1 #1). This is advisory only; it does not change the exit code or block the write.
  - *Given* a fixture of 10 files where 8 have no recognized extension, *When* `kibitzer architecture export --path <fixture> --out arch.json` runs, *Then* stdout contains a line matching `warning: 8/10 files scanned have no supported language extension`, `arch.json` is still written, and the exit code is 0.

**Files**: `src/arch_export.rs` (new), `src/main.rs`

##### Task 2.1.1a: Add `Command::Architecture { action: ArchitectureAction }` and `ArchitectureAction::Export { .. }` to `main.rs` (~4 min)
- Fields per the Domain Glossary's `ArchitectureAction` sketch: `path: PathBuf` (default `.`), `scope: Option<String>`, `out: PathBuf` (required), `dry_run: bool`, `include_private: bool`.
- Files: `src/main.rs`

##### Task 2.1.1b: Implement `run_export` — walk, read, build, prune, and the "nothing to export"/"no packages matched scope"/unsupported-language-fraction messages (~7 min)
- `find_config` + `walk_and_collect_files` + filter to recognized-language files (reuse the check from Task 1.3.1b) to detect the zero-supported-languages case up front; read each remaining file's contents into `(PathBuf, String)` pairs (this is the I/O `build_model` no longer does itself, per Story 1.3.1's pure signature); `import_graph::build` + `arch_model::build_model(repo_root, &files, &import_graph, &prune)` + optional `.filtered(scope, level)` — if `.filtered()` yields zero packages under a `--scope` filter on an otherwise non-empty model, emit the "no packages matched scope" message instead of writing an empty `arch.json`. After a successful write (or `--dry-run` print), check `model.pruning.unsupported_language_files as f64 / model.pruning.total_files_scanned as f64 >= 0.5` and print the warning line from Story 2.1.1's new AC if so.
- Files: `src/arch_export.rs`

##### Task 2.1.1c: Serialize + `--dry-run` branch + write (~3 min)
- Mirror `install.rs:35-45`'s pretty-print/trailing-newline/dry-run/write shape exactly.
- Files: `src/arch_export.rs`

##### Task 2.1.1d: Wire `main.rs`'s match arm to call `arch_export::run_export` (~2 min)
- Files: `src/main.rs`

##### Task 2.1.1e: Integration tests for all 8 acceptance criteria (~7 min)
- Files: `src/arch_export.rs`

---

### Epic 2.2: `kibitzer architecture diagram`
**Goal**: A distinct CLI verb rendering a C4-*like* Mermaid diagram plus an always-present
text-tree, with an explicit non-conformance disclaimer in its own `--help` text.

#### Story 2.2.1: `kibitzer architecture diagram` renders a labeled, non-C4-conformant Component/Code diagram
**As a** human developer, **I want** a shareable visual (Mermaid, pasteable into a PR) that
never overclaims standards conformance, **so that** a teammate doesn't mistake it for a
real C4 diagram.

**Acceptance Criteria**:
- The `ArchitectureAction::Diagram` clap doc comment (shown in `--help`) reads: "Render a Component/Code-level diagram in Mermaid notation *inspired by* C4 — not a standards-conformant C4 Context/Container diagram." — the disclaimer lives in the CLI's own text, not only this plan doc, per UX research.
  - *Given* the compiled binary, *When* `kibitzer architecture diagram --help` is run, *Then* its output contains the substring "not a standards-conformant C4".
- The disclaimer also travels inline in the rendered artifact itself, not only `--help` output — `render_text_tree` emits a leading comment line (`# Component/Code diagram — inspired by C4, not a standards-conformant C4 Context/Container diagram`) as the first line of its output, and `render_component_diagram` emits a Mermaid `%%` comment line (`%% inspired by C4 — not a standards-conformant C4 Context/Container diagram`) as the first line inside the fenced ` ```mermaid ` block — so a diagram copy-pasted into a PR/wiki/chat with no CLI context still carries the non-conformance disclaimer (closes pre-mortem Failure Mode #5, P3).
  - *Given* a small fixture, *When* `kibitzer architecture diagram --path <fixture>` runs, *Then* stdout's text-tree section's first line is the `# Component/Code diagram — inspired by C4...` comment, and the Mermaid fenced block's first line inside the fence is `%% inspired by C4 — not a standards-conformant C4 Context/Container diagram`.
- Output always includes a text-tree section (package → symbols, indented) *above* the Mermaid code fence — never Mermaid-only — per the UX accessibility finding that Mermaid has no reliable accessible-text equivalent.
  - *Given* a small fixture with one package and two symbols, *When* `kibitzer architecture diagram --path <fixture>` runs (no `--out`, so stdout), *Then* stdout contains both a text line per symbol and a fenced ` ```mermaid ` block.
- `--level component` (default) renders package-to-package boxes only (no symbol detail); `--level code` renders symbols nested inside their package's box.
  - *Given* the same fixture, *When* run with `--level code`, *Then* the Mermaid output contains a `subgraph` per package with the package's symbol names as nodes inside it; `--level component` output contains no symbol names at all.
- Reuses `MAX_NODES`-style degrade-gracefully behavior: past a node-count cap (component count for `--level component`, symbol count for `--level code`), the diagram falls back to the text-tree-only output with a note, mirroring `mermaid.rs`'s exact pattern (same message shape: "N nodes, over the M-node diagram cap — pass a narrower `--scope`").
  - *Given* a synthetic fixture with 200 packages, *When* run with default `--level component` (component cap reused from `mermaid.rs::MAX_NODES = 150`), *Then* stdout's Mermaid section is replaced by the over-cap text note and the text-tree section is still present in full.
- `--out <file>` writes the same combined text-tree + Mermaid output to a file instead of stdout (no `--dry-run` here — this command has no "would write" ambiguity since there's nothing to merge into, unlike `install.rs`'s settings.json case).

**Files**: `src/arch_diagram.rs` (new), `src/main.rs`

##### Task 2.2.1a: Add `ArchitectureAction::Diagram { .. }` with the disclaimer doc comment (~3 min)
- Fields: `path`, `scope`, `level: DiagramLevel` (clap `ValueEnum`, `Component`/`Code`, default `Component`), `out: Option<PathBuf>`.
- Files: `src/main.rs`

##### Task 2.2.1b: `render_text_tree(&ArchModel, ModelLevel) -> String` — first line of the returned string is the `# Component/Code diagram — inspired by C4, not a standards-conformant C4 Context/Container diagram` comment (Story 2.2.1's inline-disclaimer AC), then the package → symbols text tree (~5 min)
- Files: `src/arch_diagram.rs`

##### Task 2.2.1c: `render_component_diagram(&ArchModel, ModelLevel) -> String` — Mermaid `graph TD` with `subgraph` per package at `Code` level (C4-*like* visual grouping, **not** real Mermaid `C4Component`/`C4Dynamic` notation — GitHub's built-in Mermaid renderer doesn't support the C4 extension, which would defeat this diagram's PR-paste purpose, per the Pattern Decisions table), reusing `mermaid::slugify` (made `pub(crate)`); the first line emitted *inside* the fenced block (immediately after `graph TD`'s own line, before any node/subgraph lines) is the `%% inspired by C4 — not a standards-conformant C4 Context/Container diagram` comment (Story 2.2.1's inline-disclaimer AC) (~6 min)
- Files: `src/arch_diagram.rs`, `src/mermaid.rs` (visibility change on `slugify`)

##### Task 2.2.1d: Node-count cap + fallback note, mirroring `mermaid.rs::MAX_NODES` (~3 min)
- Files: `src/arch_diagram.rs`

##### Task 2.2.1e: Wire `main.rs`'s match arm — build model, call both render functions, write to `--out` or stdout (~4 min)
- Files: `src/main.rs`, `src/arch_diagram.rs`

##### Task 2.2.1f: Tests for all 5 acceptance criteria (~5 min)
- Files: `src/arch_diagram.rs`

---

## Phase 3: MCP Query Tools

### Epic 3.1: `list_architecture_symbols` and `get_architecture_node`
**Goal**: Two new MCP tools returning structured JSON (ADR-001), backed by the in-process
`ModelCache` (ADR-002), with pagination bounding output (resolving the "unbounded response"
pitfall).

#### Story 3.1.1: `list_architecture_symbols` returns a paginated, filtered slice of the model as JSON
**As an** AI agent, **I want** to query symbols by package/kind/name without re-parsing the
whole repo or receiving an unbounded response, **so that** I can look up "what does package
X export" cheaply mid-session.

**Acceptance Criteria**:
- Request struct `ListArchitectureSymbolsRequest { path: String, scope: Option<String>, package: Option<String>, kind: Option<String>, level: String (default "code"), include_private: bool (default false), limit: usize (default 200, max 1000), cursor: Option<String> }` — every field has a `///` doc comment stating its default, matching `ArchitectureAssessmentRequest`'s house style.
  - *Given* a request with only `path` set, *When* `list_architecture_symbols` runs, *Then* it behaves as `level: "code", include_private: false, limit: 200` (no `scope`/`package`/`kind` filter — everything in scope).
- Response is `serde_json::to_string(&response)` where `response = ListArchitectureSymbolsResponse { total_matched: usize, returned: usize, next_cursor: Option<String>, possibly_pruned: bool, symbols: Vec<SymbolListEntry> }` and `SymbolListEntry { package: String, symbol: SymbolNode }` — real JSON, not a formatted string, per ADR-001.
  - *Given* a fixture with 3 matching symbols and `limit: 200`, *When* the tool runs, *Then* the returned string parses as JSON with `total_matched: 3, returned: 3, next_cursor: null`, and `symbols` is a 3-element array.
- **Zero matches under the default `include_private: false` don't look identical to "truly nothing here"** — this closes pre-mortem P2 #2 (the exported-only default hiding the majority of real code with no signal that anything was hidden). When `total_matched == 0`, `request.include_private == false`, and `model.pruning.pruned_symbol_ids` is non-empty — filtered by package-id prefix (`"{package}::"`) when `request.package` is set, otherwise any non-empty `pruned_symbol_ids` counts — the response sets `possibly_pruned: true`; otherwise `possibly_pruned: false`. This is a cheap `Vec` scan against the already-computed `pruning.pruned_symbol_ids` field (Story 1.3.1) — no second build, no second `PruneConfig` pass.
  - *Given* a package whose only symbols are unexported (all pruned by the `include_private: false` default), *When* `list_architecture_symbols` is called with `package: Some("<that package>")` and default `include_private`, *Then* `total_matched: 0` and `possibly_pruned: true` — distinct from a truly-empty/nonexistent package, where `possibly_pruned: false`.
- `limit` + `cursor` pagination: `cursor` is the opaque string form of an offset; a response with more matches than `limit` sets `next_cursor` to a non-null value that, passed back as the next request's `cursor`, resumes from where the previous page left off.
  - *Given* a fixture with 5 matching symbols and `limit: 2`, *When* called with no `cursor`, *Then* `returned: 2, next_cursor: Some(_)`; *When* called again with that `next_cursor` as `cursor`, *Then* the next 2 symbols are returned and `next_cursor` is `Some(_)` again; a third call exhausts the remaining 1 and returns `next_cursor: null`.
- Querying a package/name with zero matches returns `total_matched: 0, symbols: []` (a normal, successful, empty result — not an MCP error), per UX research's Grep-parity finding.
  - *Given* `package: Some("does/not/exist")`, *When* the tool runs, *Then* the JSON response has `total_matched: 0` and no error is raised.
- The `#[tool(description = "...")]` string explicitly says "returns JSON" and contrasts with `architecture_assessment` ("scoped query, not a whole-repo report"), per ADR-001 and UX research.
- Backed by `ModelCache::get_or_build` (Epic 1.4) keyed on `(repo_root, include_private)` only — `scope` is applied via `.filtered()` to the cached model after retrieval, never part of the cache key. A second call in the same MCP session with the same key (even with a different `scope`) and unchanged files does not rebuild `ArchModel`.

**Files**: `src/mcp.rs`

##### Task 3.1.1a: Define request/response structs with doc comments, including `possibly_pruned: bool` on the response (~4 min)
- Files: `src/mcp.rs`

##### Task 3.1.1b: Implement the tool: resolve model via `ModelCache` (the build closure walks + reads files into `(PathBuf, String)` pairs, then calls `arch_model::build_model` per Story 1.3.1's pure signature), apply `scope`/`package`/`kind`/`level`/`include_private` filters via `.filtered()` on the cached model; when `total_matched == 0` and `!include_private`, set `possibly_pruned` by scanning `model.pruning.pruned_symbol_ids` (~7 min)
- Files: `src/mcp.rs`

##### Task 3.1.1c: Cursor-based pagination (offset-encoded cursor string) (~4 min)
- Files: `src/mcp.rs`

##### Task 3.1.1d: Add a `ModelCache` field (single-slot `Mutex<Option<(ModelCacheKey, CachedModel)>>`, Epic 1.4) to `KibitzerServer`, wire into `KibitzerServer::new()` (~3 min)
- Files: `src/mcp.rs`

##### Task 3.1.1e: Tests for all 7 acceptance criteria, including the `possibly_pruned` all-private-package case (~6 min)
- Files: `src/mcp.rs`

---

#### Story 3.1.2: `get_architecture_node` resolves one package or symbol by exact reference
**As an** AI agent, **I want** to fetch one specific node (a package, or a symbol by its
`id`) without listing/filtering, **so that** a follow-up lookup after `list_architecture_symbols`
is a single cheap call.

**Acceptance Criteria**:
- Request struct `GetArchitectureNodeRequest { path: String, node: String }` — `node` is tried first as an exact `ArchModel::package` key, then as a `SymbolNode::id`.
  - *Given* `node: "app/domain"` matching a package path, *When* the tool runs, *Then* the response's `"kind"` field is `"package"` and its body is that `PackageNode` (with only its direct `symbols`, not nested further — bounded by construction, no separate limit needed since one node's own children are the whole response).
  - *Given* `node: "app/domain::Compute"` matching a symbol id, *When* the tool runs, *Then* the response's `"kind"` field is `"symbol"` and its body is that `SymbolNode`.
- No match (neither a package path nor an exported/pruned-in symbol id) returns an explicit `{"kind": "not_found", "node": "<the query>", "exists_but_pruned": false}` JSON object — still a normal 200-equivalent MCP response, not an error, matching Story 3.1.1's Grep-parity finding.
  - *Given* `node: "does/not/exist"`, *When* the tool runs, *Then* the JSON response's `"kind"` field is `"not_found"` and `"exists_but_pruned"` is `false`.
- **"Truly absent" is distinguished from "exists but pruned by the default `include_private: false`"** — this closes pre-mortem P2 #2. Resolution order is: (1) `ArchModel::package`, (2) a `SymbolNode::id` in the (already-pruned) model's symbols, (3) if still unmatched, `model.pruning.pruned_symbol_ids.contains(node)` — a cheap scan against the field Story 1.3.1/1.1.1 already populates during the one `build_model` call, no second build, no second `PruneConfig` pass, no re-run of extraction. If step 3 matches, the response is `{"kind": "not_found", "node": "<query>", "exists_but_pruned": true, "hint": "retry with include_private: true"}` instead of the plain not-found shape above.
  - *Given* a package with one unexported function `doHelper` (pruned by the default `include_private: false`) and no exported symbol of that name, *When* the tool is called with `node: "<pkg>::doHelper"` and default `include_private`, *Then* the response is `{"kind": "not_found", "node": "<pkg>::doHelper", "exists_but_pruned": true, "hint": "retry with include_private: true"}` — distinct from a `node` that matches nothing at all, pruned or otherwise.
- An owner-qualified method id (`"<pkg>::Type.Method"`, per Story 1.2.2's id scheme) resolves
  to exactly that method, not to a same-named method on a different type in the same
  package — the concrete case the owner-qualified id scheme exists to make resolvable.
  - *Given* a package with two types `A`/`B` each defining a `Close` method, *When* the tool is called with `node: "<pkg>::A.Close"`, *Then* the response's `"kind"` is `"symbol"` and its body is the `Close` method whose `parent == "A"`, not `B`'s.
- Backed by the same `ModelCache` instance/key as `list_architecture_symbols` (not rebuilt separately) when called against the same repo within one MCP session.

**Files**: `src/mcp.rs`

##### Task 3.1.2a: Define request/response structs, including `exists_but_pruned: bool` and `hint: Option<String>` on the `not_found` shape (~3 min)
- Files: `src/mcp.rs`

##### Task 3.1.2b: Implement resolution order (package, then symbol id, then `pruned_symbol_ids` membership check, then plain not_found) (~5 min)
- Files: `src/mcp.rs`

##### Task 3.1.2c: Tests for the 5 acceptance criteria, including the owner-qualified same-name-different-type case and the exists-but-pruned case (~6 min)
- Files: `src/mcp.rs`

---

#### Story 3.1.3: `get_info()` and tool descriptions disambiguate the new tools from `architecture_assessment`
**As an** AI agent picking between kibitzer's 5 MCP tools, **I want** session-level guidance
on which one to use for a scoped query vs. a whole-repo report, **so that** I don't guess
wrong and re-run the expensive whole-repo assessment for a one-symbol lookup.

**Acceptance Criteria**:
- `KibitzerServer::get_info()`'s `instructions` string gains a clause naming `list_architecture_symbols`/`get_architecture_node` and stating they return JSON for scoped queries, distinct from `architecture_assessment`'s whole-repo prose report.
  - *Given* the compiled server, *When* `get_info()` is called, *Then* `instructions` contains both new tool names and the substring "JSON".

**Files**: `src/mcp.rs`

##### Task 3.1.3a: Update the `instructions` string (~2 min)
- Files: `src/mcp.rs`

##### Task 3.1.3b: Test asserting both tool names + "JSON" appear (~2 min)
- Files: `src/mcp.rs`

---

## Phase 4: LSP Integration

**Note: `document_symbol` vs `symbol` pruning asymmetry is intentional, not an oversight.**
Epic 4.2's `document_symbol` (`textDocument/documentSymbol`, an editor's file Outline)
includes **private** symbols; Epic 4.3's `symbol` (`workspace/symbol`, cross-repo search)
uses **pruned**, exported-only-by-default symbols, matching the whole-repo default
everywhere else in this feature. These are deliberately different rules for the same
underlying data, not an inconsistency to reconcile: an open file's Outline shows
everything you're editing, including private symbols, because you're already looking at
that exact file — there's no cross-repo noise to prune. Workspace-wide symbol search
defaults to the public surface because that's what's relevant when searching *across* the
repo for something to jump to, matching the "exported/public-only by default" pruning
rationale (Pattern Decisions table) applied at whole-repo scale. Task 4.2.1a and Task
4.3.1a should each carry a `///` doc comment on their handler stating this rationale
explicitly, so a future reader — or a user filing a "bug" that private symbols don't show
up in workspace search — finds a maintainer's explicit answer in the code, not silence.

### Epic 4.1: Declare `document_symbol_provider`/`workspace_symbol_provider` capabilities
**Goal**: Advertise the new capabilities so editors actually send the requests.

#### Story 4.1.1: `initialize()` advertises both symbol capabilities
**As an** editor connecting to `kibitzer lsp`, **I want** `ServerCapabilities` to declare
symbol support, **so that** my client enables "Go to Symbol"/"Workspace Symbol Search" UI.

**Acceptance Criteria**:
- `InitializeResult.capabilities` gains `document_symbol_provider: Some(OneOf::Left(true))` and `workspace_symbol_provider: Some(OneOf::Left(true))`, alongside the existing `text_document_sync`.
  - *Given* an `initialize` request, *When* `Backend::initialize` is called, *Then* the returned `ServerCapabilities.document_symbol_provider` and `.workspace_symbol_provider` are both `Some(OneOf::Left(true))`.

**Files**: `src/lsp.rs`

##### Task 4.1.1a: Add the two capability fields to the `ServerCapabilities` literal in `initialize()` (~2 min)
- Files: `src/lsp.rs`

##### Task 4.1.1b: Test asserting both fields on the returned `InitializeResult` (~2 min)
- Files: `src/lsp.rs`

---

### Epic 4.2: `document_symbol` — per-file, disk-based
**Goal**: `textDocument/documentSymbol` returns a nested `DocumentSymbol` tree for one file
without building a whole-repo `ArchModel`.

#### Story 4.2.1: `document_symbol` maps one file's `SymbolNode`s to a nested LSP tree
**As an** editor user, **I want** "Go to Symbol in File" to work, **so that** I can jump to
a type/function/method without scrolling.

**Acceptance Criteria**:
- `Backend::document_symbol` reads the file off disk (matching the existing diagnostics disk-snapshot precedent at `src/lsp.rs:88-95`, per the Pattern Decisions table), parses it with a fresh `GrammarCache`, calls `extract_symbols_for_file` directly (**not** `build_model` — no whole-repo walk for a single-file request), and returns `Ok(Some(DocumentSymbolResponse::Nested(...)))`.
  - *Given* a Go file on disk with one type `T` and one method `M` on `T`, *When* `document_symbol` is called for that file's URI, *Then* the response is `Nested` with one top-level `DocumentSymbol` named `"T"` whose `children` contains one `DocumentSymbol` named `"M"`.
- `SymbolKind` maps to `lsp_types::SymbolKind` (`Type`→`STRUCT`, `Interface`→`INTERFACE`, `Function`→`FUNCTION`, `Method`→`METHOD`).
- A file kibitzer has no `Language` mapping for (e.g. a `.md` file opened in an editor with kibitzer as its LSP — shouldn't normally happen, but must not panic) returns `Ok(None)`.
  - *Given* a `.md` file URI, *When* `document_symbol` is called, *Then* the result is `Ok(None)`, no panic.
- Unlike `did_open`/`did_change`, `document_symbol` includes **private** symbols (`include_private: true` semantics) — a file-scoped "jump to symbol" view has no noise problem the whole-repo pruning default exists to solve, so pruning is not applied here.
- If the file's parse tree has any error node (`has_error()`), `document_symbol` still returns `Ok(Some(...))` with whatever symbols extract cleanly — this is a deliberate divergence from `build_model`'s skip-the-file-entirely policy (Story 1.3.1), not an oversight: a single open file with one syntax typo shouldn't blank the whole Outline panel the way a whole-repo model export skips a broken file for correctness. `document_symbol` has no `PruningSummary` to record the error in (it's a single-file, no-model call), so this divergence is scoped to this handler only.

**Files**: `src/lsp.rs`

##### Task 4.2.1a: Implement `document_symbol`: read, parse, extract, map to `DocumentSymbol` tree; `///` doc comment on the function stating why this handler includes private symbols unlike `symbol` (Phase 4's pruning-asymmetry note) (~6 min)
- Files: `src/lsp.rs`

##### Task 4.2.1b: `SymbolKind` → `lsp_types::SymbolKind` mapping function (~2 min)
- Files: `src/lsp.rs`

##### Task 4.2.1c: Nest methods under their parent type via `SymbolNode.parent` (~4 min)
- Files: `src/lsp.rs`

##### Task 4.2.1d: Tests for all 4 acceptance criteria (~5 min)
- Files: `src/lsp.rs`

---

### Epic 4.3: Background-indexed `symbol` — workspace-wide, never built inline on a request
**Goal**: `workspace/symbol` searches against a background-built index that starts on
server startup, never a synchronous full-repo build triggered by the request itself. An
earlier draft of this epic had `symbol` build `ArchModel` synchronously, inline, on the
first request — architecture/adversarial review flagged this as contradicting
`pitfalls.md`'s explicit "background index, built at `initialize`/`initialized` time,
never recomputed from scratch inline with the request" recommendation and `ux.md`'s
"return what's indexed so far, or empty, never block" finding, and noted no `lsp.rs`
handler used `tokio::task::spawn_blocking` anywhere in the plan — meaning a slow synchronous
build could also block the tokio worker thread handling *other* concurrent LSP requests,
not just the calling client's own request. This redesign fixes both: the build runs in the
background starting at `initialized()`, and `symbol` reads from a shared `IndexState`
instead of ever building anything itself.

#### Story 4.3.0: Background index build kicks off on server start, tracked via `IndexState`
**As a** kibitzer maintainer, **I want** the whole-repo `ArchModel` build to start as soon
as the workspace root is known, off the request path entirely, **so that**
`workspace/symbol` never pays a synchronous cold-build cost inline.

**Acceptance Criteria**:
- `Backend` gains an `index_state: Arc<Mutex<IndexState>>` field, where `IndexState` is:
  `Building` (no ready snapshot yet — only true before the first background build
  completes), `Ready(Arc<ArchModel>)` (has a servable snapshot; a `did_save`-triggered
  rebuild may be running concurrently in the background without changing this variant until
  the rebuild completes and swaps it in), or `Failed(String)` (the first build errored, no
  snapshot exists). Initialized to `Building` at `Backend` construction. A separate
  `rebuilding: AtomicBool` (not a new `IndexState` variant) tracks whether a
  `did_save`-triggered background rebuild is currently in flight while `Ready` — this keeps
  `symbol`'s dispatch a plain 3-way match on `IndexState`. `Backend` also gains a
  `build_generation: AtomicU64` field, incremented (`fetch_add(1, Ordering::SeqCst)`) every
  time a background build — the initial one in `initialized()`, or any `did_save`-triggered
  rebuild — is spawned; the spawned task captures the generation value at spawn time and
  only swaps its result into `index_state` if that captured generation still equals
  `build_generation`'s current value when the task completes (see the two new ACs below —
  this resolves pre-mortem P2 #3's two unspecified concurrency behaviors).
- **`did_save` while `index_state == Building` is a no-op** — it does not spawn a second
  build. The in-flight initial build already reads whatever is on disk at whatever point
  in its walk it reaches each file, so a second concurrent build triggered by a save that
  lands mid-walk buys nothing but wasted work and a second race to swap into `index_state`;
  the existing in-flight build's own completion (per the `initialized()` AC above) is the
  only thing that transitions `index_state` out of `Building`.
  - *Given* `index_state == Building` (initial build still in flight), *When* `did_save`
    fires for an in-scope file, *Then* no second build is spawned (verified via the same
    call-counting build closure used elsewhere in this story) and `build_generation` is
    unchanged.
- **Out-of-order rebuild completion cannot clobber a newer result.** Under rapid-fire saves
  (e.g. format-on-save immediately followed by a manual save), two `did_save`-triggered
  rebuilds can be in flight at once; if the older one finishes *after* the newer one, its
  result must not overwrite the newer rebuild's already-swapped-in snapshot. Each rebuild
  is spawned with the `build_generation` value at the moment it was spawned; on completion,
  it swaps its result into `index_state` only if `build_generation`'s current value still
  equals its own captured generation — otherwise it discards its result and leaves
  `index_state` as the newer rebuild left it.
  - *Given* two `did_save` events fire in quick succession while `index_state == Ready`,
    spawning rebuild A (generation N) then rebuild B (generation N+1), and a test double
    makes A finish *after* B, *When* both complete, *Then* `index_state` ends up holding
    B's result, not A's — verified by giving A and B distinguishable synthetic `ArchModel`
    contents and asserting the final `index_state` snapshot matches B's, not A's.
- `Backend::initialized()` (the LSP `initialized` notification, sent once after the client
  processes `initialize`'s response — the point at which the workspace root is reliably
  known) spawns the whole-repo build via `tokio::task::spawn_blocking` (`build_model` +
  its callers' file walk/read are blocking work; running them inline on the async runtime's
  worker thread would block other concurrent LSP requests, per the adversarial review's
  explicit point) and returns immediately — it does **not** await the build. The build
  closure walks the workspace (`find_config` + `walk_and_collect_files`), reads each file
  into a `(PathBuf, String)` pair, and calls `import_graph::build` + `arch_model::build_model`
  (Story 1.3.1's pure signature) through the same `ModelCache::get_or_build`
  (`ModelCacheKey { repo_root, include_private: false }`, Epic 1.4) every other consumer
  uses. On completion, the spawned task locks `index_state` and transitions it to
  `Ready(Arc::new(model))` or `Failed(<error message>)`.
  - *Given* a workspace root is set at `initialize` time, *When* `initialized()` fires, *Then* the handler itself returns before the build completes (verified via a test double that observes the handler's return happening before the build-completion signal fires), and `index_state` eventually transitions away from `Building`.
- `did_save` for any in-scope file, when `index_state` is `Ready`, sets `rebuilding` and
  spawns another background `spawn_blocking` rebuild (same `ModelCache` path) rather than
  rebuilding inline — matching the Pattern Decisions table's disk-snapshot tolerance
  ("reflects last save, not live buffer") applied to the whole-repo index instead of one
  file. The existing `Ready` snapshot stays servable to concurrent `symbol` calls until the
  rebuild finishes and swaps it in.
  - *Given* `index_state == Ready(_)` and `rebuilding == false`, *When* `did_save` fires for an in-scope file, *Then* `rebuilding` becomes `true`, a new background build is spawned, and a `symbol` call made *during* that rebuild still returns results from the pre-rebuild `Ready` snapshot (verified via a call-counting/delay test double on the build step).

**Files**: `src/lsp.rs`

##### Task 4.3.0a: Define `IndexState` enum, `index_state`/`rebuilding`/`build_generation: AtomicU64` fields on `Backend` (~5 min)
- Files: `src/lsp.rs`

##### Task 4.3.0b: Spawn the background build in `initialized()` via `spawn_blocking` (walk + read files into `(PathBuf, String)` pairs + `import_graph::build` + `arch_model::build_model`, through `ModelCache::get_or_build`); increment `build_generation` and capture its value at spawn time, wire completion into `index_state` gated on that captured generation still being current (~7 min)
- Files: `src/lsp.rs`

##### Task 4.3.0c: `did_save` is a no-op while `index_state == Building`; while `Ready`, it sets `rebuilding`, increments `build_generation`, captures the new value, and re-spawns a background rebuild (never inline) gated the same way as Task 4.3.0b — swap `index_state` to the new `Ready` snapshot on completion only if the captured generation is still current (~7 min)
- Files: `src/lsp.rs`

##### Task 4.3.0d: Tests for the 5 acceptance criteria — handler-returns-before-build-completes, state transitions, stale-snapshot-served-during-rebuild, did_save-during-Building-is-a-no-op, out-of-order-rebuild-completion-keeps-the-newer-result (~9 min)
- Files: `src/lsp.rs`

---

#### Story 4.3.1: `symbol` returns matching `SymbolInformation` from the index, or an explicit still-indexing signal
**As an** editor user, **I want** "Go to Symbol in Workspace" to find a type/function
anywhere in the repo without ever hanging, **so that** I don't need to know which file it's
in and don't mistake a cold start for a broken picker.

**Acceptance Criteria**:
- While `index_state == Building` (no snapshot yet — only possible before the first
  background build finishes), `symbol` returns `Ok(Some(vec![<one synthetic
  SymbolInformation>]))` — a single entry whose `name` states indexing is in progress (e.g.
  `"⏳ kibitzer: still indexing this workspace — try again shortly"`) and whose `location`
  points at the workspace root — an explicit, visible "still indexing" state per `ux.md`'s
  recommendation, not a silent empty list a user could mistake for "no symbols in this
  repo." `query` is ignored while in this state (no filtering against a non-existent
  index).
  - *Given* `index_state == Building`, *When* `symbol` is called with any `query`, *Then* the single synthetic entry described above is returned, unfiltered.
- While `index_state == Ready(model)` (regardless of whether a background rebuild is
  concurrently in flight via `rebuilding`), `symbol` substring-filters
  `WorkspaceSymbolParams.query` against `model`'s **pruned** (exported-only by default)
  symbol names and maps matches to `SymbolInformation` — same filtering/mapping behavior as
  the original (pre-review) design, just never triggered by the request itself.
  - *Given* symbols `Reader`, `Writer`, `Closer` in the ready index, *When* `symbol` is called with `query: "Re"`, *Then* only `Reader` is returned.
- While `index_state == Failed(_)` (e.g. no `.claude/inspect.json` found for the workspace),
  `symbol` returns `Ok(None)` rather than an LSP error — an editor's symbol picker degrades
  to "no results" rather than showing an error toast.
  - *Given* `index_state == Failed(_)`, *When* `symbol` is called, *Then* the result is `Ok(None)`.
- Uses **pruned** (exported-only by default) symbols, matching the whole-repo default
  elsewhere — unlike `document_symbol`'s file-scoped exemption in Epic 4.2, a workspace-wide
  picker has the same noise concern the pruning default exists for.

**Files**: `src/lsp.rs`

##### Task 4.3.1a: Implement `symbol`: match on `index_state` (`Building` → synthetic still-indexing entry, `Ready` → substring filter + map, `Failed` → `Ok(None)`) — never builds or blocks itself; `///` doc comment on the function stating why this handler uses pruned (exported-only) symbols unlike `document_symbol` (Phase 4's pruning-asymmetry note) (~6 min)
- Files: `src/lsp.rs`

##### Task 4.3.1b: `SymbolNode`/package → `SymbolInformation` mapping helper (shared shape with Epic 4.2's `SymbolKind` mapper) (~3 min)
- Files: `src/lsp.rs`

##### Task 4.3.1c: Tests for all 4 acceptance criteria (~5 min)
- Files: `src/lsp.rs`

##### Task 4.3.1d: Document `workspace/symbol`'s cold-cache latency and pruning behavior in `docs/lsp.md` (~4 min)
- `docs/lsp.md` already exists and already has a `## Known limitation: diagnostics reflect
  disk, not the buffer` section (added for the diagnostics feature) — add a second section
  in the same style/heading level, `## Known limitation: cold-cache latency and pruning in
  workspace symbol search`, covering three points (each closes a specific Phase 4 UX gap
  flagged in the Phase 4 review round that followed this plan's first draft):
  1. **Cold-cache/first-call latency** (closes `design/ux.md`'s UX Acceptance Criterion 11):
     state that the first `workspace/symbol` call in a session may arrive before the
     background index (Story 4.3.0's `initialized()`-time build) finishes, in which case it
     returns the synthetic "still indexing" entry from Story 4.3.1's first AC (the literal
     `"⏳ kibitzer: still indexing this workspace — try again shortly"` text) instead of real
     matches — not a hang, not an error, not a sign the picker is broken. Subsequent searches
     in the same `kibitzer lsp` session are fast (index built once, reused per Story 4.3.0).
  2. **`textDocument/documentSymbol` vs. `workspace/symbol` pruning asymmetry** (the same
     rationale Task 4.2.1a/4.3.1a's `///` doc comments state in code, per the Phase 4 note
     above Epic 4.1 — restated here for an editor user who will never read the Rust source):
     "Symbol search in your editor's Outline (per-file) includes private/unexported symbols
     since you're already looking at that file; workspace-wide symbol search (Go to Symbol in
     Workspace) defaults to the public surface only."
  3. **No `possibly_pruned`/`exists_but_pruned` equivalent for `workspace/symbol`** (closes a
     UX review gap: unlike Surfaces 3/4's MCP tools, `workspace/symbol` has no field
     distinguishing "no results" from "results exist but were pruned"). State the limitation
     plainly: "`workspace/symbol` search defaults to public symbols only and returns no
     results for a private-only match, indistinguishable from a true non-match; use the MCP
     `list_architecture_symbols` tool with `include_private: true`, or `kibitzer architecture
     export --include-private`, for a definitive check." This is a documented limitation, not
     a bug to fix: verified against this repo's actual pinned dependency versions
     (`Cargo.toml`'s `tower-lsp = "0.20.0"`, `lsp-types 0.94.1` per `Cargo.lock`) that while
     `lsp-types` 0.94.1 does define a 3.17-spec `WorkspaceSymbol.data: Option<LSPAny>`
     extension field, `tower-lsp` 0.20.0's `LanguageServer::symbol` trait method signature is
     hardcoded to the legacy `Result<Option<Vec<SymbolInformation>>>` shape (`tower-lsp-0.20.0/
     src/lib.rs:1155-1162`), and `SymbolInformation` has no `data`/vendor-extension field — so
     there is no clean protocol-level signal available at this dependency version without a
     `tower-lsp` upgrade, which is out of scope for this feature.
- Also add one line to `README.md`'s existing `kibitzer lsp` bullet (currently just `kibitzer
  lsp                         # serve as an LSP server over stdio (diagnostics)`, with no
  pointer to `docs/lsp.md` anywhere in the repo today) so a reader lands on the fuller
  doc: append `(see docs/lsp.md)`; note in that same file's LSP section, near the existing
  `command reference`/"See `docs/checking-invocations.md`..." pointer style, that
  `docs/lsp.md` also documents document/workspace symbol support once Epic 4.1-4.3 ship.
- Files: `docs/lsp.md`, `README.md`

---

## Phase 5: Extended Language Coverage

Each epic below extends `symbol_extract.rs` (`LangSymbolConfig` entry + extraction cases)
and `import_graph.rs` (a new `build_<lang>` function, following `build_go`/`build_js`'s
existing shape) for one language. No changes to `arch_model.rs`, `mcp.rs`, `lsp.rs`, or the
CLI are needed per language — Phases 2–4's interfaces automatically pick up new languages
the moment `checker::Language` files are recognized, which is the direct payoff of the
"one model, multiple views" design from Phase 1.

Because each epic mirrors Epic 1.2/1.3's already-established shape (not novel design),
tasks below are grained slightly coarser (~5–10 min each) than Phase 1–4's 2–5 min
guideline — justified by repetition of a proven pattern, not by skipping verification.
Each epic still starts with a real `to_sexp()`/`field_name_for_child` dump before writing
extraction code, per the pitfalls research's core lesson.

### Epic 5.1: Python symbol + import extraction
**Goal**: `class_definition` → `Type`; `function_definition`/`decorated_definition`-wrapped
functions → `Function`/`Method` (method iff nested in a `class_definition`'s body); no
`Interface` kind (Python has no first-class interface node — `Protocol`-based structural
typing is a library convention, not a grammar-level construct, so `interface_kinds` is
empty for Python, matching JS's precedent from Epic 1.2). Exported iff the name doesn't
start with `_` (module-level PEP 8 convention) — classes/functions inside `if __name__ ==
"__main__":` blocks are still extracted (no special-casing; matches the "walk everything,
don't get clever" precedent from `rules.rs`).

#### Story 5.1.1: Python symbols extract correctly, including decorated definitions
**Acceptance Criteria**:
- *Given* `class Widget:\n    def render(self): pass`, *When* extracted, *Then* one `Type` symbol `Widget` and one `Method` symbol `render` with `parent: Some("Widget")`.
- *Given* `def _helper(): pass`, *When* extracted, *Then* `exported: false`.
- *Given* `@dataclass\nclass Point:\n    x: int`, *When* extracted, *Then* the `decorated_definition`-wrapped `class_definition` is still found as one `Type` symbol `Point` (recursion into every child, same as `rules.rs`'s existing handling of Python decorators for function-level rules).

**Files**: `src/symbol_extract.rs`

##### Task 5.1.1a: `to_sexp()` verification dump for Python class/decorated-class fixtures (~5 min)
##### Task 5.1.1b: `lang_symbol_config(Language::Python)` entry (~5 min)
##### Task 5.1.1c: Extraction match arms + underscore-prefix export rule (~6 min)
##### Task 5.1.1d: Tests for the 3 acceptance criteria (~5 min)
- Files: `src/symbol_extract.rs` (all tasks)

#### Story 5.1.2: Python import extraction (`import_graph.rs`)
**Acceptance Criteria**:
- *Given* `from .sibling import foo` inside `pkg/a.py`, *When* `build_python` runs, *Then* an edge from `pkg` to `pkg` (relative-import-within-same-package) is *not* added (matches `build_js`'s existing same-directory no-op rule), but `from ..other import bar` produces an edge from `pkg/sub` to `pkg`.
- *Given* `import os` (absolute, non-relative), *When* `build_python` runs, *Then* no edge is added (external/stdlib, matching Go's stdlib-skip and JS's bare-specifier-skip precedents).

**Files**: `src/import_graph.rs`

##### Task 5.1.2a: `to_sexp()` verification dump for Python `import_statement`/`import_from_statement` (~5 min)
##### Task 5.1.2b: `collect_python_imports` — walk `import_from_statement`'s `module_name` field, counting leading dots for relative depth (~7 min)
##### Task 5.1.2c: `build_python` wiring into `import_graph::build`'s extension dispatch (~4 min)
##### Task 5.1.2d: Tests for the 2 acceptance criteria (~5 min)
- Files: `src/import_graph.rs` (Tasks 5.1.2a-d)

##### Task 5.1.2e: Extend `SKIP_DIRS` (`check.rs`) with `__pycache__`, `.venv`, `venv`, `.tox` so Python virtualenv/bytecode-cache dirs don't leak into the file walk feeding this (~2 min)
- Mirrors Task 5.2.2d's Java equivalent — without this task the "Summary of new/changed
  files" table's claim that `check.rs` gains these four Python-specific entries in Phase 5
  has no implementing task (adversarial review finding).
- Files: `src/check.rs`

---

### Epic 5.2: Java symbol + import extraction
**Goal**: `interface_declaration` → `Interface`; `class_declaration`/`enum_declaration`/
`record_declaration` → `Type`; `method_declaration` → `Method` (Java has no free functions,
so `Function` is never produced for Java — every method has a `parent`). Exported iff a
`public` modifier is present (package-private/no-modifier is `exported: false`, matching
the Pattern Decisions table's default).

**Accepted v1 limitation — method overload id collisions**: the owner-qualified id scheme
(`"{package_path}::{parent}.{name}"`, Story 1.2.2/Pattern Decisions table) resolves same-
named methods on *different* types, but Java method **overloading** (same name, different
parameter lists, same class — e.g. two `save(String)`/`save(String, int)` methods on one
type) still collides, since arity/parameter types aren't part of the id. This is an
explicit v1 decision, not an undiscovered gap: overloaded methods are last-extraction-wins
in `PackageNode.symbols` (whichever overload `extract_symbols_for_file` visits last for that
`parent`/`name` pair is the one `get_architecture_node` resolves to), and this is documented
here rather than deferred. Revisit (e.g. append an arity or parameter-type-hash suffix to
the id) only if a user reports an ambiguous lookup in practice — not built speculatively for
v1.

#### Story 5.2.1: Java symbols extract correctly, including nested classes
**Acceptance Criteria**:
- *Given* `public interface Shape { double area(); }`, *When* extracted, *Then* one `Interface` symbol `Shape` with `exported: true`, and its abstract method `area` extracted as a `Method` with `parent: Some("Shape")`.
- *Given* `class Helper { void run() {} }` (no `public` modifier), *When* extracted, *Then* `Helper`'s `exported: false`.
- *Given* a `record Point(int x, int y) {}`, *When* extracted, *Then* one `Type` symbol `Point` (records classify as `Type`, not a new `SymbolKind` variant, per the "don't over-model" principle from the Pattern Decisions table's generics decision).

**Files**: `src/symbol_extract.rs`

##### Task 5.2.1a: `to_sexp()` verification dump for Java interface/class/record fixtures (~5 min)
##### Task 5.2.1b: `lang_symbol_config(Language::Java)` entry (~5 min)
##### Task 5.2.1c: Extraction match arms + `public`-modifier export rule (~6 min)
##### Task 5.2.1d: Tests for the 3 acceptance criteria (~5 min)
- Files: `src/symbol_extract.rs` (all tasks)

#### Story 5.2.2: Java import extraction (`import_graph.rs`)
**Acceptance Criteria**:
- *Given* a Maven/Gradle-style source tree (`src/main/java/com/acme/app/Foo.java` declaring `package com.acme.app;`), *When* `build_java` runs, *Then* the package node key is derived from the `package` declaration (not the directory path directly — Java's package name and directory layout are linked by convention, not identity, unlike Go), and an `import com.acme.domain.Bar;` produces an edge from `com.acme.app` to `com.acme.domain` iff `com.acme.domain` is one of the repo's own local packages (external/JDK imports like `java.util.List` are skipped, matching the stdlib-skip precedent).

**Files**: `src/import_graph.rs`

##### Task 5.2.2a: `to_sexp()` verification dump for Java `package_declaration`/`import_declaration` (~5 min)
##### Task 5.2.2b: `java_package_name` — extract every file's declared package (not directory-derived) (~6 min)
##### Task 5.2.2c: `collect_java_imports` + `build_java`, filtering to only-known-local packages (~7 min)
##### Task 5.2.2d: Extend `SKIP_DIRS` (`check.rs`) with `.gradle`, `.mvn`, `out` so Java build artifacts don't leak into the file walk feeding this (~2 min)
##### Task 5.2.2e: Tests for the acceptance criterion (~4 min)
- Files: `src/import_graph.rs`, `src/check.rs` (Task 5.2.2d only)

---

### Epic 5.3: Kotlin symbol + import extraction
**Goal**: Highest-risk grammar in the rollout (per Pattern Decisions #9) — Kotlin's
`function_declaration`/`class_declaration` expose no field names (confirmed precedent at
`rules.rs:157-173`'s `kotlin_body`/`kotlin_params` positional-child workaround). `interface`
is a modifier on `class_declaration` (Kotlin doesn't have a separate `interface_declaration`
node kind — verify this via `to_sexp()` before assuming it, per the pitfalls research's
explicit warning that Kotlin already broke one assumption in `rules.rs`). Exported iff
neither a `private` nor `internal` modifier is present (Kotlin's default visibility is
public, the inverse of Java's default).

#### Story 5.3.1: Kotlin symbols extract correctly using positional-child lookups
**Acceptance Criteria**:
- *Given* `interface Repository { fun save() }`, *When* extracted, *Then* one `Interface` symbol `Repository` (verified against the real grammar's node kind for an interface — whether that's a distinct kind or a modifier on `class_declaration`, resolved in Task 5.3.1a, not assumed here).
- *Given* `private class Internal`, *When* extracted, *Then* `exported: false`; *given* `class Public` (no modifier), *When* extracted, *Then* `exported: true`.
- *Given* `class Box { fun open(): Unit {} }`, *When* extracted, *Then* `open`'s `parent: Some("Box")`, found via the same positional-child pattern `kotlin_body`/`kotlin_params` already establish in `rules.rs` (reused, not reinvented, since the class/function containment shape is the same problem).

**Files**: `src/symbol_extract.rs`

##### Task 5.3.1a: `to_sexp()` + `field_name_for_child` verification dump for Kotlin interface/class/private fixtures — resolve whether `interface` is its own node kind or a `class_declaration` modifier before writing extraction code (~6 min)
##### Task 5.3.1b: `lang_symbol_config(Language::Kotlin)` entry, reusing `kotlin_body`/`kotlin_params`-style positional lookups from `rules.rs` (import or duplicate the pattern — duplicate, since `rules.rs`'s functions are private to that module and this is a distinct table; note as an accepted small duplication, not worth a shared-utility extraction for two call sites) (~6 min)
##### Task 5.3.1c: Extraction match arms + modifier-based export rule (~7 min)
##### Task 5.3.1d: Tests for the 3 acceptance criteria (~5 min)
- Files: `src/symbol_extract.rs` (all tasks)

#### Story 5.3.2: Kotlin import extraction (`import_graph.rs`)
**Acceptance Criteria**:
- *Given* a Kotlin file with `import com.acme.domain.Bar`, *When* `build_kotlin` runs, *Then* the same package-declaration-based resolution as Java (Kotlin also uses `package`/`import` declarations independent of directory layout, unlike Go/JS) produces an edge iff the target is a known local package.

**Files**: `src/import_graph.rs`

##### Task 5.3.2a: `to_sexp()` verification dump for Kotlin `package_header`/`import_header` (~5 min)
##### Task 5.3.2b: `collect_kotlin_imports` + `build_kotlin` (~7 min)
##### Task 5.3.2c: Extend `SKIP_DIRS` with any Kotlin-specific build dirs not already covered by Task 5.2.2d's Gradle additions (likely none — Kotlin/Gradle projects share `.gradle`/`build`) — verify, don't add speculatively (~2 min)
##### Task 5.3.2d: Test for the acceptance criterion (~3 min)
- Files: `src/import_graph.rs`

---

## Summary of new/changed files

| File | Change |
|------|--------|
| `src/arch_model.rs` | **New.** `ArchModel`, `PackageNode`, `SymbolNode`, `SymbolKind`, `ModelLevel`, `PruneConfig`, `PruningSummary`, `build_model`, query API, `ModelCache`. |
| `src/symbol_extract.rs` | **New.** `LangSymbolConfig`, `lang_symbol_config`, `extract_symbols_for_file`, per-language extraction (Go/TS/Tsx/JS in Phase 1; Python/Java/Kotlin in Phase 5). |
| `src/arch_export.rs` | **New.** CLI I/O for `kibitzer architecture export`. |
| `src/arch_diagram.rs` | **New.** C4-*like* Mermaid + text-tree rendering for `kibitzer architecture diagram`. |
| `src/import_graph.rs` | Add `Serialize`/`Deserialize` to `ImportEdge`; add `build_python`/`build_java`/`build_kotlin` (Phase 5). |
| `src/cache.rs` | `stamp()` visibility: `fn` → `pub(crate) fn`. |
| `src/check.rs` | Extend `SKIP_DIRS` with `__pycache__`, `.venv`, `venv`, `.tox`, `.gradle`, `.mvn`, `out` (Phase 5). |
| `src/mermaid.rs` | `slugify` visibility: private → `pub(crate)` (reused by `arch_diagram.rs`). |
| `src/mcp.rs` | Add `list_architecture_symbols`, `get_architecture_node` tools; `ModelCache` field on `KibitzerServer`; update `get_info()`. |
| `src/lsp.rs` | Add `document_symbol`, `symbol` handlers; `ServerCapabilities` fields; `ModelCache`/dirty-flag fields on `Backend`. |
| `docs/lsp.md` | Add "Known limitation: cold-cache latency and pruning in workspace symbol search" section (Task 4.3.1d). |
| `README.md` | Point the existing `kibitzer lsp` line at `docs/lsp.md` (Task 4.3.1d). |
| `src/main.rs` | Add `mod arch_model;`, `mod symbol_extract;`, `mod arch_export;`, `mod arch_diagram;`; add `Command::Architecture { action: ArchitectureAction }`. |

No `Cargo.toml` changes — confirmed by `stack.md` research: every dependency needed
(`serde`, `serde_json`, `tree-sitter` + all 7 grammars, `tower-lsp`, `rmcp`) is already
pinned.
