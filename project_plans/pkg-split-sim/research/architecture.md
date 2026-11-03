# Research: Architecture — pkg-split-sim

**Date**: 2026-11-01

## What ArchModel already provides

`ArchModel` (`src/arch_model.rs:254–278`) carries everything the simulator needs:

- `packages: BTreeMap<String, PackageNode>` — keyed by module-qualified import path; each `PackageNode` carries `files: Vec<PathBuf>` and `symbols: Vec<SymbolNode>`.
- `import_edges: Vec<ImportEdge>` — directed, per-file (one edge per import statement), from `ImportGraph`. `edges_from()` on `ImportGraph` yields outward edges; the simulator needs reverse too (build a `HashMap<String, Vec<&ImportEdge>>` at simulation entry).
- `call_edges: Vec<CallEdge>` — `from` → `to` (both `SymbolNode::id`-qualified). Resolved edges have a stable `SymbolNode::id`; unresolved keep the raw callee text and `resolved: false`.
- `field_accesses: Vec<FieldAccessEdge>` — method → field (Go-only). Needed for detecting unexported-field sharing across a partition boundary.
- `type_edges: Vec<TypeRelationEdge>` — `extends`/`implements`. Needed for checking whether an unexported type used in both partition parts must be exported or moved.
- `file_import_aliases: BTreeMap<PathBuf, HashMap<String, String>>` — Go alias → package path. Needed to resolve qualified type references (`pkg.Type`) back to their source package.
- `pruning: PruningSummary` — tracks `private_symbols_skipped`, `generated_files_skipped`, `files_with_parse_errors`. These become coverage diagnostics in simulation output.

## What requires `include_private: true`

By default `build_model` runs with `PruneConfig { include_private: false }`, which excludes unexported symbols from `PackageNode.symbols` and from `call_edges`. The simulator's exported-surface delta and private-shared-state detection both require unexported symbols. **The simulation must request `PruneConfig { include_private: true }`.** The existing `ArchModelChecker` trait has a `needs_private_symbols() -> bool` hook that triggers this; the simulator will use the same mechanism.

Cost note: `PruningSummary.private_symbols_skipped` reports how many symbols were skipped in a normal run. On large packages this can be substantial (the Go stdlib alone has thousands of unexported functions). Benchmark on a representative package before committing to always-on.

## Cycle detection: do not reuse `ImportCycleChecker`

`ImportCycleChecker` (`src/architecture_checks.rs`) detects cycles in the **real** import graph. The simulator needs cycle detection in a **hypothetical** post-split graph:

1. Start from `ArchModel.import_edges`.
2. For each edge whose `from` package is the target package `P`, redirect it: if the source file belongs to part P1, the edge now originates from P1; if it belongs to P2, from P2. Similarly redirect `to` edges that terminate at P.
3. Add new intra-partition edges: if file `f1 ∈ P1` imports a symbol from file `f2 ∈ P2` (both previously in P), that creates a new cross-part edge P1 → P2.
4. Run Tarjan's SCC algorithm (or a simple DFS cycle check) on this projected graph. A back edge is a cycle; report the edge and the symbol that caused it.

The `union_find.rs` in the codebase provides a union-find structure that could accelerate "are these nodes in the same SCC" queries if needed.

## Reverse-dependency counts

`affected.rs` already implements reverse-BFS (transitive importers of a package set). The simulator reuses this logic:

- **Before**: run reverse-BFS from `{P}` → count of distinct packages that transitively import P.
- **After per part**: run reverse-BFS from `{P1}`, `{P2}` separately → count per part.

Important: count **distinct packages**, not `ImportEdge` count. `affected.rs`'s BFS already deduplicates by package key, so its output is already distinct-package counts. The `fan_in_out` bug (counting per-file edges rather than distinct package pairs in `architecture_checks.rs:180–187`) does not affect `affected.rs`'s BFS output — confirmed by reading `affected.rs:compute_affected`.

## Detecting `init()` registrations

`init()` functions in Go have no arguments and no return values; tree-sitter identifies them as `function_declaration` nodes with `name == "init"`. kibitzer's existing `symbol_extract.rs` extracts function declarations by kind; querying for `init` by name in each file of the target package is straightforward.

No ordering analysis: whether two `init()` calls across a split boundary have ordering constraints is undecidable without runtime tracing. The simulator flags any package that has one or more `init()` functions as a blocker for human review — conservative and correct.

## Build tags and cgo

Build tags are embedded in the first comment block of a `.go` file (`//go:build ...` or `// +build ...`). Files with build tags that affect which compilation units are included in a build create platform-variant considerations. The simulator should:

1. Detect files in the target package that have a `//go:build` or `// +build` comment.
2. Flag them as "build-tag variant — split may behave differently across platforms."

cgo files import `"C"` — detectable as a literal import in `ArchModel.import_edges` (an edge from the package to `"C"`). Flag any partition part that receives a cgo file as a blocker (cgo has additional toolchain constraints that go beyond import semantics).

## Generated files

`arch_model.rs:294` (the `is_generated` check) already identifies generated files via "do not edit" / "code generated" banner detection. `PruningSummary.generated_files_skipped` counts them. The simulator should report generated files as blockers requiring regeneration if they appear in the target package — they should not be part of a user-specified partition (their location is dictated by the generator, not the developer).

## Symbol visibility across partition boundary

For each call edge `from_file → to_symbol` where `from_file ∈ P1` and `to_symbol.file ∈ P2` (or vice versa), the simulator checks `to_symbol.exported`. If `!exported`, that symbol must either be exported (added to exported-surface delta) or the call site must be moved to the same part as the symbol. The same logic applies to `field_accesses` (unexported fields accessed across a boundary) and `type_edges` (unexported types used across a boundary).

## Coverage diagnostics (explicit abstention)

The simulation must report what it could not analyse:
- `pruning.private_symbols_skipped > 0` → "N private symbols were not analysed; unexported cross-boundary references may be undercounted."
- `pruning.generated_files_skipped > 0` → "N generated files were excluded; review manually."
- Unresolved call edges (`CallEdge.resolved == false`) in the target package → "K call sites have unresolved targets and could not be attributed to a partition part."
- `pruning.files_with_parse_errors` → "Files with parse errors; their symbols are absent from the model."

This "abstain rather than guess" contract matches the item description's requirement and kibitzer's general stance on incomplete evidence.
