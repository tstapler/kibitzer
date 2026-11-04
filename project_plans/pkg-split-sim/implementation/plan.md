# Implementation Plan: pkg-split-sim

**Date**: 2026-11-01

## Overview

The simulator lives in a new `src/split_sim.rs` module. It is a pure function: `run(model: &ArchModel, package: &str, partitions: &[PartitionSpec], opts: SplitSimOpts) -> Result<SplitSimResult>`. No I/O, no git ops, no mutations.

CLI wiring adds a `SplitSim` variant to `ArchitectureAction` in `main.rs`. MCP wiring adds a `split_sim` tool in `mcp.rs`.

## Story 1: Core data types (`src/split_sim.rs`)

**Estimate**: ~2h

Define the input and output types:

```rust
pub struct PartitionSpec { pub name: String, pub files: Vec<PathBuf> }
pub struct SplitSimOpts { pub include_cluster_suggestion: bool }

pub struct SplitSimResult {
    pub package: String,
    pub parts: Vec<String>,
    pub cycle_findings: Vec<CycleFinding>,
    pub exported_surface_delta: ExportedSurfaceDelta,
    pub callers_to_rewrite: Vec<CallersToRewrite>,
    pub blockers: Vec<Blocker>,
    pub reverse_dep_impact: ReverseDependencyImpact,
    pub cluster_suggestion: Option<ClusterSuggestion>,
    pub coverage: CoverageDiagnostics,
}
```

All types derive `Debug`, `Clone`, `Serialize`, `Deserialize`. All fields use `BTreeMap`/`BTreeSet` for deterministic serialization order.

## Story 2: Partition validation (`split_sim::validate_partition`)

**Estimate**: ~1.5h

Given `PackageNode` and `Vec<PartitionSpec>`:
1. Verify all specified files are in `PackageNode.files`. Warn on any absent (may be generated or misspelled).
2. Verify all `PackageNode.files` appear in exactly one `PartitionSpec`. Return `Err` on unassigned files (configurable with `--allow-residual`).
3. Verify ≥ 2 parts, distinct names.

Returns `ValidationResult { errors: Vec<String>, warnings: Vec<String> }`.

## Story 3: Projected graph construction + cycle detection

**Estimate**: ~3h

1. Build `file_to_part: HashMap<PathBuf, String>` from all `PartitionSpec` files.
2. Filter `ArchModel.import_edges` to those touching the target package.
3. Redirect edges per `file_to_part` assignment:
   - External A → P with file `f ∈ Pi`: becomes `A → Pi`.
   - P → external B with source file `f ∈ Pi`: becomes `Pi → B`.
4. Detect intra-package cross-part references via `call_edges` (file `f1 ∈ Pi` calling symbol in file `f2 ∈ Pj` creates edge `Pi → Pj`).
5. Build `petgraph::Graph<String, ()>` from projected edges.
6. Run `petgraph::algo::tarjan_scc` — any SCC with size > 1 is a cycle.
7. For each cycle, find the `ImportEdge` or `CallEdge` that closes it (smallest set of edges that form the cycle, via DFS back-edge trace).
8. Label each cycle finding as production/test/mixed based on originating file suffix.

## Story 4: Exported-surface delta

**Estimate**: ~2h

For each `CallEdge` in `ArchModel.call_edges`:
- Resolve `from` file → part and `to` symbol's file → part (use `file_to_part` or external).
- If parts differ and `to_symbol.exported == false`: add to `must_export`.
- If parts differ and `to_symbol.exported == true`: add to `already_exported`.

Same pass for `field_accesses` (unexported field crossing boundary) and `type_edges` (unexported type used across boundary).

This requires `include_private: true` — the caller (`main.rs`/`mcp.rs`) must build the model with `PruneConfig { include_private: true }`.

## Story 5: Callers-to-rewrite aggregation

**Estimate**: ~1.5h

For each symbol that moves (in any part, exported or not, referenced from outside that part):
1. Collect all `CallEdge`s where `to == symbol.id`.
2. Bucket by file suffix (`_test.go` → test, generated-file detection → generated, else production).
3. Group by importing package key (from `ImportGraph.file_packages`).
4. Report distinct package count and up to `opts.max_ref_sites` (default 20) specific `file:line` sites.

## Story 6: Blocker detection

**Estimate**: ~2h

Per file in the target package:
- `init()`: `PackageNode.symbols.iter().filter(|s| s.name == "init" && s.kind == Function)`.
- Build tags: read first 20 lines of each file; scan for `//go:build` or `// +build` prefix.
- cgo: check `import_edges` for edge from `file`'s package to the special `"C"` import path.
- Unexported shared state: `field_accesses` where accessor's file and field-owner type's file are in different parts and the field is unexported — flag as potential shared state (underapproximation noted in coverage diagnostics).

## Story 7: Reverse-dependency impact

**Estimate**: ~1.5h

Reuse `affected.rs`'s reverse-BFS (`compute_reverse_reachable(graph, seed_packages) -> BTreeSet<String>`). The function doesn't exist by that name yet (it's inlined in `compute_affected`) — extract it into a standalone helper function in `affected.rs`.

Run three times: seed `{P}`, seed `{P1}`, seed `{P2}` (for a two-part split). Report distinct-package counts (not edge counts).

## Story 8: Fixture test suite (`testdata/split-sim/`)

**Estimate**: ~3h

Seven fixture packages (minimal Go trees, each with `go.mod` + a few `.go` files):

1. `legal-split/` — two-part split, no cycles, one symbol needs export promotion.
2. `cycle-creating/` — split that creates a direct import cycle between P1 and P2.
3. `private-shared-state/` — two files share an unexported `var mu sync.Mutex` at package scope.
4. `init-registration/` — one file has an `init()` that registers a codec used by another file.
5. `build-tag-variant/` — one file has `//go:build linux`.
6. `test-only-cycle/` — `_test.go` external test package creates a cycle via the split.
7. `generated-file/` — one file has a "Code generated" banner and should not appear in partitions.

Each fixture has a `expected.json` (the expected `SplitSimResult`, committed, used for snapshot testing). Unit tests in `src/check/` or a new `src/split_sim_tests.rs` drive each fixture and assert against the expected JSON.

## Story 9: Ground-truth reverse-dep verification

**Estimate**: ~1h (documentation + script)

On two fixtures (at minimum: `legal-split/` and `cycle-creating/`):
1. Perform the real file split (in a temporary directory, not in the repo).
2. Run `kibitzer architecture affected --base <pre-split-sha>` on the split result.
3. Compare the affected package counts against the simulation's `reverse_dep_impact` output.

Document the comparison in `project_plans/pkg-split-sim/implementation/verification.md`. This is the item's "verified against ground-truth recomputation" acceptance criterion.

## Story 10: CLI wiring

**Estimate**: ~1.5h

Add to `main.rs`:
```rust
ArchitectureAction::SplitSim { package, partitions, format, allow_residual, include_cluster_suggestion }
```

Parse `--partition NAME=file1.go,file2.go` (multi-flag). Build model with `PruneConfig { include_private: true }`. Call `split_sim::run(...)`. Emit result per `--format`.

## Story 11: MCP wiring

**Estimate**: ~1.5h

Add `split_sim` tool to `mcp.rs` following the existing `get_architecture_node` parameter pattern. Accept JSON input matching the schema in `research/stack.md`. Return `SplitSimResult` as JSON. Register in `list_tools` and dispatch in `call_tool`.

## Story 12 (optional): Advisory cluster suggestion

**Estimate**: ~2h

Using `jaccard.rs`:
- Build call-target sets per file in the target package.
- Compute pairwise Jaccard similarity.
- Greedy cluster: threshold 0.4 (configurable via `--cluster-threshold`).
- Label output as advisory.

Only emitted when `opts.include_cluster_suggestion == true` (or `--include-cluster-suggestion` CLI flag).

## Sequencing

```
Story 1 (types)
  → Story 2 (validation)
  → Story 3 (projected graph + cycle detection)
  → Story 4 (exported-surface delta)
  → Story 5 (callers to rewrite)
  → Story 6 (blocker detection)
  → Story 7 (reverse-dep impact)   [needs affected.rs helper extraction]
  → Story 8 (fixtures)             [drives Stories 3–7]
  → Story 9 (verification)         [needs affected CLI]
  → Story 10 + 11 (CLI/MCP wiring) [independent of 8/9]
  → Story 12 (cluster, optional)
```

Stories 3–6 can be implemented in parallel by separate agents; each reads `ArchModel` and `file_to_part` (built in Story 3's setup phase) independently.

## Risk notes

- **`include_private: true` performance**: benchmark on kibitzer's own codebase and one backtest-corpus repo before committing to always-on. If cost is >500ms on a mid-size package, add a `--skip-private-analysis` flag that disables Stories 4/6 and emits an explicit coverage warning.
- **`affected.rs` helper extraction** (Story 7): a small refactor to `affected.rs` to extract the reverse-BFS. This is a contained change to an already-tested file — low risk, but needs its own unit test to confirm the extracted function behaves identically to the inlined version.
