# Research: Features (prior art, edge cases, unstated needs)

*(Written by the orchestrator from the research subagent's returned findings — the subagent
could not write this file itself because the host root filesystem was 100% full at the time
it ran. The user has since freed space and this content was recorded verbatim from the
subagent's returned summary.)*

## 1. Existing patterns to reuse

- `checker.rs`'s `Language` enum (7 langs) + `registry()`/`lookup()` trait-object pattern is used identically by both `Checker` and `ArchitectureChecker` — a new `SymbolExtractor` should follow the same shape.
- `rules.rs`'s `lang_config(Language) -> LangRuleConfig` table (`src/rules.rs:175-320`) already catalogs per-language "function-like" tree-sitter node kinds (Go's `function_declaration`/`method_declaration`, Kotlin's positional-child quirks needing `kotlin_body`/`kotlin_params`, Python's `decorated_definition` wrapping, etc.) — this is 90% of the node-kind knowledge symbol extraction needs; it's missing only type/interface/class kinds. Should extend this table, not build a parallel one.
- `checker::GrammarCache` (`src/checker.rs:145-171`) parses each language once per cache instance — should be reused by symbol extraction. Note: `import_graph.rs`'s `build_go`/`build_js` currently construct their own `tree_sitter::Parser` inline rather than using `GrammarCache` — a pre-existing inconsistency worth reconciling.
- `crate::glob::matches_scope` is the existing scope-glob convention (used by `mcp.rs`'s `architecture_assessment`, `src/mcp.rs:145-155`) — CLI export/MCP query tools should reuse this exact glob syntax.
- `mermaid.rs`'s `MAX_NODES = 150` fallback (`src/mermaid.rs:10-40`, returns a text note + "pass a narrower scope" instead of a diagram) is the pattern the requirements' NFRs ask to mirror for C4 diagrams.
- `architecture_checks.rs`'s `find_cycles` (Tarjan SCC, `src/architecture_checks.rs:186-258`) is already `pub` and reused by `mermaid.rs` — could genericize for type-reference-cycle detection at symbol level.
- `mcp.rs`'s `architecture_assessment` returns a flat annotated `String`, not JSON — a new query tool returning structured JSON (which the requirements' "queryable, scoped answer" metric implies) would be a shape deviation from every other MCP tool today; call this out explicitly in the plan.
- `docs/output-formats.md`'s SARIF handling is the only existing "structured model, multiple renderers" precedent, but it's check-output-level, not export-level — no existing `--format` flag on any subcommand.

## 2. Industry prior art

ctags/gopls workspace-symbols validate a flat searchable name→location index but lack containment/dependency edges; LSP `documentSymbol`'s nested tree is a good target shape for the symbol side. `go doc`/TS `--declaration`/Javadoc all default to exported-only surface (informs the "minimized" pruning rabbit hole) but are single-language and single-level. Sourcegraph/CodeSee/Sourcetrail/`cq`-style tools all invest in clustering/collapsing past a few hundred nodes — validates kibitzer's 150-node Mermaid cap and suggests C4 Component-level should default to package granularity, symbol-level only on drill-down. tree-sitter-graph confirms the general tree-sitter-per-language approach but offers no ready-made schema.

## 3. Edge cases

Generated code (no existing marker-detection anywhere in the codebase — net new); vendored code (exclusion today depends entirely on upstream file-list/`.gitignore` handling, not verified to exist); symlinks (`import_graph.rs:190-213` already canonicalizes for JS import resolution — symbol-extraction file walking needs the same discipline); very large files (no size/count guard exists anywhere today); circular type references (distinct from import cycles — no type-reference edge extraction exists yet, would be net-new graph-building); anonymous/inline types (no naming policy exists — `rules.rs` never names anything); monorepo multi-language (import_graph already dispatches per-extension over one file list and merges graphs — pattern generalizes, but cross-language edges are unaddressed).

## 4. Unstated needs

Architecture drift diffing between two exports (kibitzer's whole identity is diff-aware; a first persisted model without diff support is a conspicuous gap, though not explicitly requested); "changed since last commit" filtering (parallels kibitzer's diff-aware theme elsewhere, not in requirements); multiple output shapes — JSON is explicit in scope but a human `tree`-style renderer is implied by the Users/Consumers section and not listed in Scope; stable symbol IDs across runs (needed for any drift/incremental use case, not mentioned, affects JSON schema design); performance risk of full whole-repo/whole-language re-parse on every export given `GrammarCache`'s per-run cache-from-scratch design.

## Files read by the research agent

`src/architecture_checks.rs`, `src/import_graph.rs`, `src/mermaid.rs`, `src/checker.rs`, `src/mcp.rs`, `src/lsp.rs`, `src/rules.rs`, `docs/syntax-rules.md`, `docs/output-formats.md`.
