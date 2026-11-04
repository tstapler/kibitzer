# Requirements: pkg-split-sim

**Date**: 2026-11-01
**Type**: feature addition
**Complexity**: 4 — new analysis subsystem (read-only simulation layer on top of `ArchModel`, with deterministic output and a fixture-backed acceptance suite)

## Problem Statement

Splitting a Go package is a common structural refactor but a high-risk one: import cycles, unexported-symbol boundaries, and `init()`-registration invariants don't surface until code is actually moved, at which point the work has already started and the simplest exit is to revert. Agents encounter this routinely — they propose a partition, start moving files, and discover infeasibility mid-flight.

kibitzer already builds the data needed to check a proposed partition *before* touching code: `ArchModel` has package-level import edges, per-file symbol declarations (including exported/unexported), call edges, field-access edges, and a file-to-package mapping. What's missing is the simulation layer: given a target package and a proposed partition of its files (or symbols) into two or more new packages, walk the model and answer "is this legal, and what would it change?" without writing a line of refactored code.

This is the read-only, pre-flight foundation of the mechanical-refactor umbrella. It ships before any auto-apply work because it is safe (no mutations), immediately useful in review (agents can check feasibility before proposing a refactor), and provides the ground-truth fixture suite the mechanical-refactor work will eventually need to validate against.

## Baseline

- **`ArchModel`** (`src/arch_model.rs`) carries `packages` (keyed by path), `import_edges` (per `ImportEdge.from`/`ImportEdge.to`), `call_edges`, `field_accesses`, `type_edges`, `file_import_aliases`, and `pruning`. It is already the shared data source for CLI export, diagram, MCP tools, and the `affected` subcommand.
- **`ImportGraph`** (`src/import_graph.rs`) has directed `ImportEdge`s and `file_packages` (path → package key). Its `edges_from()` walks forward (outward imports). No reverse-index or cycle-detection exists today outside `ImportCycleChecker` (`src/architecture_checks.rs`), which detects existing cycles in the *real* graph — not simulated ones in a hypothetical post-split graph.
- **`affected.rs`** already computes transitive reverse-dependencies (reverse BFS from a changed-file set) and should be reused here for the "reverse-dep count before/after" report section.
- **`go-package-focus`** (`project_plans/go-package-focus/`) answers "should this package be split?" (advisory, evidence-gathering). This project answers "can it be split *this way*, and what changes?" — the two should share evidence and coverage diagnostics (private symbols, generated code, unresolved symbols from `PruningSummary`) rather than duplicate them.
- **`fan_in_out` deduplication** (`src/architecture_checks.rs:180–187`) is a known bug (counts per-file import edges, not distinct package pairs). The reverse-dependency counts in this simulation *must* count distinct package pairs from the start — do not inherit that bug.

## Users / Consumers

- **Primary**: AI agents (e.g., Claude Code running a refactor) invoking `kibitzer architecture split-sim <package> --partition <file1,file2,...>=<P1> [--partition ...]` before proposing or executing a split.
- **Secondary**: human developer running the same command interactively to pre-flight a proposed split before a large refactor PR.
- **Tertiary**: CI: a pre-refactor gate that blocks a split PR if the simulation reports blockers (cycles, unexported cross-boundary references).

## Success Metrics

- For a legal split, the simulation reports zero cycle violations and zero blocker-class issues; for a cycle-creating split, it names the exact import edge and the symbols that force it.
- The reported "reverse-dependency before/after" per-part counts are verified against ground truth (i.e., match what `kibitzer architecture affected` computes after the real split is performed on at least two fixtures).
- The full fixture suite (legal, cycle-creating, private-shared-state, `init()` registration, build-tag variant, test-only cycle, generated code) all produce the expected simulation outputs.
- Available through both the CLI (`kibitzer architecture split-sim`) and the MCP server (`split_sim` tool).

## Appetite

TBD — Phase 3 planning sizes this after research. Baseline estimate: new `split_sim.rs` (~400–600 lines), fixture testdata (~7 test trees), CLI wiring (~50 lines), MCP wiring (~80 lines), fixture verification doc (before/after reverse-dep comparison on ≥2 fixtures).

## Constraints

- **Read-only**: no source files are written or moved. The simulation works entirely on the in-memory `ArchModel` — same "one model, many views" pattern the architecture-export project established.
- **Go-only v1**: `ArchModel`'s call graph and symbol detail is richest for Go today; other language support is a future extension, matching the `affected.rs` precedent.
- **Deterministic output**: same tree + same partition → same output. No randomized heuristics, no LLM calls in the simulation core.
- **Distinct-package-pair reverse-dep counts**: the `fan_in_out` per-file-edge counting bug must not carry over here.
- **Abstain rather than guess**: when `PruningSummary` indicates private symbols, generated files, or parse errors that make the simulation incomplete, emit an explicit coverage diagnostic rather than suppressing it.
- **No mechanical-refactor apply logic in this project**: the simulation's job is analysis and reporting only; the apply step (file moves, import-path rewrites, export promotions) is a separate, later project.

## Non-functional Requirements

- **Performance**: simulation over a realistic package (≤200 files, ≤2000 symbols) must complete in under 1 second on the local machine; the model itself is already built by the caller.
- **Output formats**: JSON (machine-readable, for agent consumption) and plain-text human summary; matching `arch_export.rs`'s `--format json|text` convention.
- **Security**: reads only the already-built `ArchModel`; no new trust boundaries, no git ops.

## Scope

### In Scope

1. A `kibitzer architecture split-sim <package> --partition ...` CLI subcommand nested under the existing `ArchitectureAction` enum, alongside `Export`/`Diagram`/`Affected`.
2. A `split_sim` MCP tool (parallel to the existing `list_architecture_symbols`/`get_architecture_node` tools in `src/mcp.rs`).
3. Simulation output sections:
   - **Cycle check**: new import edges that would create a cycle (test-only and production, named with the offending symbols).
   - **Exported-surface delta**: identifiers that must be promoted to exported across the new boundary; identifiers currently used only within one part (can stay unexported there).
   - **Callers to rewrite**: distinct importing packages and reference sites per moved symbol (production, test, generated — split into three buckets per the `PruningSummary`'s generated-file tracking).
   - **Blockers**: `init()`-time registration, build tags/platform variants, cgo files, files sharing unexported state or a lock, generated files needing regeneration.
   - **Dependency-graph impact**: per-part before/after distinct-package reverse-dependency counts (fan-in narrowing, or not).
   - **Coverage diagnostics**: what the model could not analyse (private symbols pruned, generated files skipped, unresolved call targets, parse errors).
   - **Optional cluster suggestion**: candidate partitions derived from co-usage of identifiers and imports, clearly labelled advisory (see `go-package-focus`'s AC-3 precedent for the uncertainty qualifier).
4. A fixture test suite covering the seven scenarios from the acceptance criteria.
5. Ground-truth verification on at least two fixtures: perform the real split, run `kibitzer architecture affected` before/after, confirm the simulation's reverse-dep counts match.

### Out of Scope

- Automatic file moves or import-path rewrites.
- Multi-language support beyond Go in v1.
- Function/symbol-level granularity partition input (v1 accepts file-set partitions; symbol-set input is a future extension).
- Daemon-cache-backed incremental recomputation (same decision as `affected.rs`).

## Rabbit Holes

- **Cycle detection in the simulated graph** must run on the post-split hypothetical edge set, not the real one. Don't adapt `ImportCycleChecker` in-place (it checks the real graph) — the simulator builds a temporary projected graph, runs a DFS/Tarjan, and discards it.
- **"Private shared state" detection** is inherently under-approximate: if two files both use an unexported `var mu sync.Mutex` at package scope, the simulator can flag it, but it cannot see a mutex *field* shared indirectly through an unexported type. Be explicit about this ceiling in coverage diagnostics.
- **`init()` registration**: `init()` functions are typically found by name matching; tree-sitter can find them, but whether two `init()` calls across a split boundary have ordering constraints (e.g., one registers a codec the other uses) is not statically determinable. Flag all `init()` presences as a blocker for human review; don't try to prove ordering safety.
- **Partition input format**: `--partition file1.go,file2.go=P1` is a natural CLI shape, but an agent will submit a JSON body to the MCP tool. The two surfaces need a shared internal `Partition` type; don't let CLI parsing details leak into the simulation core.
- **Test packages as a separate partition dimension**: Go test files (`_test.go`) may import packages internal to the module (`package foo_test` external tests). A split that moves files into a new internal package could break test-only imports — this is the "test-only cycle" fixture scenario; don't ignore it just because `_test.go` is not production code.

## Alternatives Considered

- **Build on `go-package-focus`'s evidence report**: `go-package-focus` (AC-3) produces an advisory evidence report for *whether* to split; this project produces a deterministic feasibility report for *how* to split a specific partition. The two answer different questions, but they share the `ArchModel` and the same coverage-diagnostic contract. Considered merging them into one command; rejected because the inputs differ (go-package-focus takes a package, the simulator takes a package + explicit partition), and conflating advisory evidence with deterministic feasibility would muddy both outputs. They remain separate commands that share model data.
- **Shell out to `go build` after a scratch copy of the files**: deterministic and accurate, but requires writing files to disk, which violates the read-only constraint and adds a temp-dir management surface. Rejected in favor of the in-model simulation.
- **Only expose via MCP, no CLI**: the existing pattern (every architecture subcommand has both a CLI surface and, for interactive use, an MCP tool) strongly argues for both. CLI is the test harness; MCP is the agent surface.

## Feasibility Risks

- **Private-symbol coverage gap**: `PruningSummary.private_symbols_skipped` is non-zero on any real repo (kibitzer prunes unexported symbols by default). The simulation's exported-surface delta and private-shared-state detection both depend on having private symbols in the model; this requires passing `PruneConfig { include_private: true }` to `build_model`, which is more expensive. The CLI and MCP tool must document this and the performance cost.
- **`init()` ordering analysis**: statically undecidable in general. The fixture must include a case where two `init()` functions have an ordering constraint so the "flag all `init()` as blocker" policy is exercised and documented as intentionally conservative.
- **Ground-truth verification of reverse-dep counts**: performing the real split on a fixture, running `kibitzer architecture affected`, and comparing counts requires a working `kibitzer architecture affected` implementation. This project is therefore softly sequenced after `affected.rs` lands (per the item description's "builds on #36, #37" note) — if `affected.rs` is not yet in place, the ground-truth verification step falls back to a manual `go list -test -deps` comparison documented in the fixture PR.

## Open Questions

- **Partition input via MCP**: should the MCP `split_sim` tool accept a flat JSON array of `{files: [...], name: "P1"}` objects, or a map `{P1: [...], P2: [...]}`? The array form is more consistent with the CLI's multi-`--partition` flag; the map form is more natural for small ad-hoc splits. Resolve in Phase 3.
- **`include_private` cost**: does forcing `PruneConfig { include_private: true }` for the simulation make model build time noticeably worse on a large repo (e.g., ripgrep, vscode)? Benchmark during research before committing to always-on.
- **Cluster suggestion threshold**: the advisory cluster suggestion (optional AC) needs a similarity threshold (Jaccard over shared call targets? co-usage pairs?). `jaccard.rs` already exists in the codebase. What minimum Jaccard score justifies surfacing a cluster lead? Defer to Phase 3 after corpus experiments.
