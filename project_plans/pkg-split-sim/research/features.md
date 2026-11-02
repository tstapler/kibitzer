# Research: Feature Breakdown — pkg-split-sim

**Date**: 2026-11-01

## Feature 1: Partition input validation

Validate the user-specified partition before running any simulation:
- All specified files exist in `PackageNode.files` (warn on missing/generated).
- Every file in the target package is assigned to exactly one part (fail or warn on unassigned).
- Partition names are distinct.
- At least two parts are specified (a one-part "partition" is trivially a no-op).

Output: structured `PartitionValidationResult` with errors and warnings.

## Feature 2: Projected import graph construction

From `ArchModel.import_edges`:
1. Build a `HashMap<PathBuf, &str>` from file path to partition-part name (for files in the target package) or to the package key (for files outside).
2. For each `ImportEdge` touching the target package `P`:
   - Edge `A → P` where file `f ∈ P1`: becomes `A → P1`.
   - Edge `P → B` where the edge originates from a file `f ∈ P1`: becomes `P1 → B`.
   - An edge where both source and target are in the target package (intra-package import, Go does not have these — skip) — note: Go packages don't import themselves, so intra-package edges are impossible.
   - An edge from `f1 ∈ P1` to a symbol in `f2 ∈ P2` (detected via `call_edges`, not `import_edges`) creates a new edge `P1 → P2`.

Output: `ProjectedGraph { nodes: HashSet<String>, edges: Vec<(String, String)> }`.

## Feature 3: Cycle detection

Run Tarjan's SCC on `ProjectedGraph`. For each SCC of size > 1 (or any back edge in DFS), report:
- The cycle as a list of package nodes: `[P1, X, P1]`.
- The import edge (or call edge) that closes the cycle, including file and line from the originating `ImportEdge` or `CallEdge`.
- Whether the cycle is production-only, test-only, or mixed.

Output: `Vec<CycleFinding>`.

## Feature 4: Exported-surface delta

For each `CallEdge` where `from_file ∈ Pi` and `to_symbol.file ∈ Pj` (i ≠ j):
- If `to_symbol.exported == false`: add to "must export" list (symbol name, current file, which parts reference it).
- If `to_symbol.exported == true`: add to "already exported" list (still relevant — these are callers to update in `import` statements).

Same logic for `field_accesses` (unexported fields crossing the boundary) and `type_edges` (unexported types used across the boundary).

Output: `ExportedSurfaceDelta { must_export: Vec<SymbolRef>, already_exported: Vec<SymbolRef>, unexported_field_refs: Vec<FieldRef> }`.

## Feature 5: Callers to rewrite

For each symbol that moves between parts (any symbol in `Pi` referenced from `Pj` or from external packages):
- Collect all `CallEdge`s where `to == symbol.id`.
- Bucket into: production (file doesn't end in `_test.go`), test (`_test.go`), generated (file matches generated-file detection).
- Report the distinct importing package count and the specific file:line reference sites (capped at a configurable limit, default 20 per symbol).

Output: `CallersToRewrite { symbol: SymbolRef, production: Vec<RefSite>, test: Vec<RefSite>, generated: Vec<RefSite> }`.

## Feature 6: Blocker detection

Check each file in the target package for:
- `init()` function: `symbol_extract` finds `Function` symbols named `init`.
- Build tags: scan first 20 lines for `//go:build` or `// +build`.
- cgo: check `import_edges` for an edge to `"C"` from the file's package.
- Generated: already covered by `pruning.generated_files_skipped` and `build_model`'s skip logic.
- Unexported state shared across parts: `FieldAccessEdge`s where both accessor and accessed field's owner type are in different parts but the field is unexported — a proxy for "shared state." (Underapproximation: indirect sharing through an unexported type is not detected.)

Output: `Vec<Blocker>` with kind (`InitFunc | BuildTag | Cgo | GeneratedFile | SharedUnexportedState`), file, and a human-readable explanation.

## Feature 7: Reverse-dependency impact

Using `affected.rs`'s reverse-BFS logic:
- **Before**: count of distinct packages that transitively import `P` (the unsplit package).
- **After P1**: count of distinct packages that would transitively import `P1`.
- **After P2**: count of distinct packages that would transitively import `P2`.

A split that results in a narrow-fan-out part (P2 has few reverse deps) and a wide-fan-out part (P1 has many reverse deps) confirms that the split isolates a stable, widely-depended-on interface in P1 from a volatile implementation in P2. Report both counts and the delta.

Output: `ReverseDependencyImpact { before: usize, per_part: BTreeMap<String, usize> }`.

## Feature 8 (optional): Cluster suggestion

Using `jaccard.rs`'s existing similarity functions:
- Build a co-usage matrix: for each pair of files `(f1, f2)` in the target package, compute the Jaccard similarity of their outbound call targets (the set of `SymbolNode::id`s they reference).
- Files with high co-usage similarity belong in the same partition part.
- Use a simple greedy clustering (threshold: Jaccard ≥ 0.4, tunable) to suggest candidate partitions.
- Label explicitly as "advisory cluster suggestion — not a recommendation" per the item description's evidence standards.

Output: `Option<ClusterSuggestion>` with named candidate parts, similarity scores, and the confidence qualifier.

## Feature 9: Coverage diagnostics

Assemble from `PruningSummary` + unresolved call-edge count:
- Private symbols skipped (count + note that unexported cross-boundary refs may be undercounted).
- Generated files excluded (count).
- Files with parse errors (list of paths).
- Unresolved call edges in the target package (count of `CallEdge.resolved == false` where `from` is in the target package).
- Unsupported-language files (count — relevant if the package has non-Go files).

Output: `CoverageDiagnostics`.

## Feature 10: CLI and MCP wiring

- **CLI**: new `SplitSim` variant of `ArchitectureAction` in `main.rs`. Parse `--partition P1=f1.go,f2.go` flags (multiple allowed). Pass to `split_sim::run(model, package, partitions, opts)`.
- **MCP**: new `split_sim` tool in `src/mcp.rs`, parameter schema matches the JSON shape in `research/stack.md`. Returns `SplitSimResult` as JSON.
- **Output formats**: `--format json` emits `SplitSimResult` as JSON; `--format text` (default) emits a structured human-readable summary.
