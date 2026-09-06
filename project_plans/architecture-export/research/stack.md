# Research: Stack

**Question**: Which specific libraries, frameworks, versions, and patterns apply to `architecture-export`? What new dependencies (if any) are needed, and what are current community-recommended versions?

## Existing codebase context (VERIFIED — read directly)

- `Cargo.toml` (repo root): kibitzer 0.1.10, edition 2024, MIT license, standalone package (opted out of parent workspace). Relevant existing deps: `tree-sitter = "0.26"` + `tree-sitter-go 0.25` / `tree-sitter-typescript 0.23` / `tree-sitter-javascript 0.23` / `tree-sitter-python 0.23` / `tree-sitter-java 0.23.5` / `tree-sitter-kotlin-ng 1.1.0`, `serde 1` (derive), `serde_json 1` (`preserve_order`), `clap 4` (derive), `anyhow 1`, `rmcp 0.2` (features `server`, `transport-io`), `schemars 0.8`, `tokio 1` (`rt-multi-thread`, `macros`, `io-std`), `tower-lsp 0.20.0`.
- `src/import_graph.rs` (379 lines): `ImportGraph { nodes: BTreeSet<String>, edges: Vec<ImportEdge> }`, package/module-directory granularity, Go + TS/JS extraction only today (comment at line ~33 explicitly notes Python/Kotlin/Java can follow the same per-language dispatch pattern). No graph crate — plain `BTreeSet`/`Vec`.
- `src/architecture_checks.rs`: `find_cycles(graph: &ImportGraph) -> Vec<Vec<String>>` at line 186 is a **hand-rolled DFS** over `ImportGraph`, using only `std::collections::HashMap`. No `petgraph` or any graph-algorithm crate involved anywhere in the current implementation.
- `src/mermaid.rs` (162 lines): hand-rolls Mermaid `graph TD` text generation directly (string formatting + a `slugify` helper for node IDs), capped at `MAX_NODES = 150` with a text-fallback message beyond that. No templating or diagram crate.
- `src/mcp.rs` (441 lines): `rmcp` `#[tool_router]`/`#[tool]` macros; `architecture_assessment` (line 130) is the existing one-shot tool.
- `src/lsp.rs` (231 lines): `impl LanguageServer for Backend` (line 116) currently implements only `initialize` and diagnostics-publishing paths — no symbol-related methods exist yet.
- `Cargo.lock`: **`petgraph` does not appear anywhere** (`grep -c '^name = "petgraph"' Cargo.lock` → 0 matches). It is not a transitive dependency of `tree-sitter`, `rmcp`, `tower-lsp`, or anything else currently pulled in — confirming the requirements doc's assumption should be checked, and it checks out: petgraph is **not** already present, adding it would be a genuinely new dependency.

## 1. tower-lsp 0.20 — workspace/document symbol support

**VERIFIED** via `docs.rs/tower-lsp/latest/tower_lsp/trait.LanguageServer.html` (crates.io confirms 0.20.0 is both `newest_version` and `max_stable_version`, license `MIT OR Apache-2.0`).

`LanguageServer` trait already ships three symbol-related **provided methods** (default no-op impls, so kibitzer must override them to do anything — nothing needs adding to `Cargo.toml`, this is pure `src/lsp.rs` implementation work):

- `async fn document_symbol(&self, params: DocumentSymbolParams) -> Result<Option<DocumentSymbolResponse>>` — `textDocument/documentSymbol`. `DocumentSymbolResponse` is an enum with two variants: `Flat(Vec<SymbolInformation>)` (legacy, flat, deprecated `location`-only) and **`Nested(Vec<DocumentSymbol>)`** (hierarchical — a `DocumentSymbol` has a `children: Option<Vec<DocumentSymbol>>` field). Nested is the modern shape and is what a C4 Code-level "types/functions within a file" view maps onto naturally.
- `async fn symbol(&self, params: WorkspaceSymbolParams) -> Result<Option<Vec<SymbolInformation>>>` — `workspace/symbol`, project-wide, **flat only** (`SymbolInformation`, not `DocumentSymbol`) — this is the entry point for "where is type Y defined" queries across the repo.
- `async fn symbol_resolve(&self, params: WorkspaceSymbol) -> Result<WorkspaceSymbol>` — optional resolve step for partial `workspace/symbol` results (lazy `location`/`data` population); only needed if `symbol` returns partial results, which kibitzer likely won't need given it computes everything up front.

Implication for the plan phase: `src/lsp.rs`'s `impl LanguageServer for Backend` needs `document_symbol` and `symbol` added (both currently unimplemented, inheriting the trait's default `Ok(None)`), and `initialize`'s `ServerCapabilities` response needs `document_symbol_provider: Some(...)` / `workspace_symbol_provider: Some(...)` set (currently presumably absent/`None`, not directly confirmed by line-range read but standard tower-lsp capability-negotiation requirement — flag as a plan-phase check against the actual `InitializeResult` construction at `src/lsp.rs:118`).

**Maintenance note (flag, not a blocker)**: `ebkalderon/tower-lsp`'s last published version is 0.20.0, roughly 2+ years old per crates.io download/version metadata — the upstream repo shows reduced release cadence. A community fork, **`tower-lsp-server`** (`tower-lsp-community/tower-lsp-server` on GitHub, published as `tower-lsp-server` on crates.io), exists specifically because of this: it replaced the unmaintained `gluon-lang/lsp-types` dependency with `tower-lsp-community/lsp-types` and bumped MSRV 1.64→1.77. **Recommendation**: stay on `tower-lsp 0.20.0` for this feature (no API-breaking reason to migrate, and workspace/document symbol support already exists in 0.20), but note the fork as a future-migration option if upstream `tower-lsp` goes fully dormant — out of scope to switch as part of `architecture-export`.

## 2. Mermaid/C4 diagram generation crate vs. hand-rolling

**Claim in the requirements doc — "no mainstream C4-in-Rust crate" — VERIFIED as essentially correct**, with nuance:

- Searched crates.io for a Rust C4-DSL crate: no established, actively-maintained crate dedicated to C4 model diagram generation exists. (`c4` on crates.io is unrelated — a graph-clustering-algorithm crate, not architecture diagrams. `C4lc` is unrelated.)
- Rust *does* have general Mermaid-emission crates (`mermaid-builder` — type-checked builder pattern for Mermaid diagram strings; `simple-mermaid` — a macro for embedding Mermaid in rustdoc, not for programmatic generation; `mermaid-rs-renderer` — renders Mermaid *to SVG*, i.e. solves the opposite problem from what kibitzer needs since kibitzer only needs to emit Mermaid *text*, same as `src/mermaid.rs` already does for `graph TD`). None of these are anywhere near as established/widely-depended-on as e.g. `serde` or `clap` — they're small, single-maintainer crates.
- **Mermaid.js itself DOES support C4 notation directly** (VERIFIED via mermaid.js.org/syntax/c4.html): diagram types `C4Context`, `C4Container`, `C4Component`, `C4Dynamic`, `C4Deployment` — syntax is explicitly designed to be PlantUML-C4-compatible. Marked **experimental** by the Mermaid project (syntax/properties may still change). One caveat: **GitHub's built-in Mermaid renderer does not support the C4 extension** — a `C4Component` block won't render inline in a GitHub PR/issue/markdown preview, only in tools with full Mermaid.js (e.g. Mermaid Live Editor, VS Code Mermaid extensions, mermaid-cli). Since kibitzer already emits Mermaid text by hand (not by rendering it), this doesn't block anything — it's a caveat to document for end users, not an implementation blocker.
- **Recommendation**: continue the `src/mermaid.rs` pattern — hand-roll `C4Component`/`C4Context`-notation text generation as a sibling function (e.g. `render_c4_component` alongside `render_dependency_graph`), reusing the existing `slugify`/`MAX_NODES`-style guardrails. No new dependency needed for this. If a PlantUML-C4 or Structurizr-DSL output format is later wanted as an *additional* export target, that's plain string templating too — no crate exists for either in Rust, and pulling in an external DSL library isn't warranted for text generation this simple.

## 3. In-memory queryable tree/graph model + serde JSON — petgraph vs. plain structs

**VERIFIED**: `petgraph` is not currently in the dependency tree at all (checked `Cargo.lock` directly, 0 matches — not pulled transitively by `tree-sitter`, `rmcp`, `tower-lsp`, or anything else). Adding it would be a net-new dependency, contradicting the (unverified-in-requirements) assumption it might already be present.

- Current version: **petgraph 0.8.3** (crates.io `newest_version` == `max_stable_version`, license `MIT OR Apache-2.0` — compatible with kibitzer's MIT license).
- Serde support is feature-gated: `features = ["serde-1"]` enables serialization for `Graph`, `StableGraph`, and `GraphMap` — but petgraph's own serde output shape is a **node-list + edge-list encoding** (adjacency-list internals), not a nested tree. That's a reasonable shape for round-tripping petgraph's own structures, but it is *not* the "tree-shaped JSON, greppable/jq-able" artifact the requirements doc asks for (Success Metrics: "a persisted, greppable/jq-able artifact"). A jq-friendly export wants nested objects keyed by package/symbol path, not a flat node/edge array pair.
- The existing codebase already proves the "plain struct + serde_json" approach scales to this problem: `ImportGraph` is exactly that (a `BTreeSet` of nodes + `Vec` of edges), and `find_cycles` does its own DFS over it with zero graph-crate dependency, at whatever scale kibitzer's own repo and larger currently exercise it at (no evidence of performance problems reported in the codebase or CHANGELOG-equivalent commit history).
- **Recommendation**: **don't add petgraph.** Model the new architecture tree as plain `serde`-derived nested structs (e.g. a `Package { path, symbols: Vec<Symbol>, imports: Vec<String> }` tree, or similar, shaped for direct `serde_json::to_writer_pretty` export and `jq` navigation) exactly as `ImportGraph`/`CheckResult` already do elsewhere in the codebase. Reserve `petgraph` as a fallback only if the plan phase's query/traversal algorithms (e.g. C4-level filtering, transitive-dependency queries) turn out to need graph algorithms (shortest-path, topological sort, strongly-connected-components at symbol granularity) that would be meaningfully harder to hand-roll than `find_cycles`' existing ~untitled DFS was. Given cycle-detection was already hand-rolled successfully at package granularity, the same pattern likely extends to symbol granularity without new algorithmic complexity — but flag this as a decision point for the Phase 3 plan, not settled by this research alone.

## 4. Version/license summary for anything recommended

No new runtime dependencies are recommended by this research. Everything proposed reuses:

| Crate | Version already pinned | Confirmed current-latest | License | Action |
|---|---|---|---|---|
| tower-lsp | 0.20.0 | 0.20.0 (crates.io) | MIT OR Apache-2.0 | keep; implement `document_symbol`/`symbol` on existing trait |
| serde / serde_json | 1 / 1 (`preserve_order`) | — (not re-verified, unchanged) | MIT OR Apache-2.0 | keep; model new tree as serde-derived structs |
| tree-sitter + grammars | 0.26 / per-language | — (unchanged) | MIT (tree-sitter core) | keep; extend per-language dispatch for symbol-level AST walks (Go/TS/TSX/JS done for imports; Python/Java/Kotlin grammars already pinned but unused for extraction yet — per `import_graph.rs` comment) |

If petgraph is later pulled in during Phase 3 planning: **petgraph 0.8.3, MIT OR Apache-2.0** — license-compatible with kibitzer's MIT.

## Sources

- [LanguageServer in tower_lsp - docs.rs](https://docs.rs/tower-lsp/latest/tower_lsp/trait.LanguageServer.html)
- [tower-lsp - crates.io](https://crates.io/crates/tower-lsp) / [tower-lsp API](https://crates.io/api/v1/crates/tower-lsp)
- [tower-lsp-server - crates.io](https://crates.io/crates/tower-lsp-server) / [tower-lsp-community/tower-lsp-server](https://github.com/tower-lsp-community/tower-lsp-server)
- [C4 Diagrams | Mermaid](https://mermaid.js.org/syntax/c4.html)
- [Support C4 Model diagram syntax in Mermaid renderer — github.com/orgs/community discussion #197898](https://github.com/orgs/community/discussions/197898) (GitHub's own renderer doesn't support the C4 extension)
- [mermaid-builder — earth-metabolome-initiative/mermaid-builder](https://github.com/earth-metabolome-initiative/mermaid-builder)
- [petgraph - crates.io](https://crates.io/crates/petgraph) / [petgraph API](https://crates.io/api/v1/crates/petgraph)
- [petgraph docs.rs](https://docs.rs/petgraph/latest/petgraph/)
- Repo files read directly: `Cargo.toml`, `Cargo.lock` (grep for petgraph), `src/import_graph.rs`, `src/mermaid.rs`, `src/mcp.rs`, `src/lsp.rs`, `src/architecture_checks.rs`
