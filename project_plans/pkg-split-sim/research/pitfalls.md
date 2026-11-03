# Research: Pitfalls — pkg-split-sim

**Date**: 2026-11-01

## Pitfall 1: Inheriting the `fan_in_out` per-file-edge counting bug

`architecture_checks.rs:180–187` iterates `ImportEdge` slices and counts every edge, not distinct `(from, to)` package pairs. If the reverse-dependency count computation naively sums `import_edges.len()` where `to == target_package`, it inherits this inflation. **Always deduplicate by `(from, to)` pair before counting.**

`affected.rs`'s BFS deduplicates by inserting package keys into a `BTreeSet` — reuse that pattern exactly.

## Pitfall 2: Forgetting test packages in the projected graph

Go test files (`_test.go` ending, or `package foo_test`) can import internal packages not available to production importers. An `external_test` package (e.g., `package checker_test`) that imports a symbol from the target package `P` is a separate package node in `ImportGraph`, but its test files' `file_packages` entries point to a distinct package key (Go convention: `<import-path>_test`). The simulator must:

1. Treat `_test` packages as legitimate package nodes in the projected graph.
2. Check for test-only cycles separately (a production code cycle is a hard blocker; a test-only cycle may be acceptable depending on project policy, but must still be reported).

Existing kibitzer precedent: `affected.rs` does not distinguish test vs. production packages today — they are all just package keys in `ImportGraph.nodes`. The simulator follows the same approach but labels findings as "test-only cycle" vs. "production cycle" based on whether the edge originates from a file whose path ends in `_test.go` or whose `PackageNode` key ends in `_test`.

## Pitfall 3: Treating `PruningSummary.private_symbols_skipped > 0` as "no private symbols"

When `include_private: false` (the default), private symbols are absent from `PackageNode.symbols` and from `call_edges`. If the simulator runs with `include_private: false`, it will silently miss all unexported cross-boundary references and report a false "no exported-surface delta needed" conclusion. **Always run with `include_private: true` for split-sim.** Document the performance cost in the CLI help text.

## Pitfall 4: Confusing package-level import edges with file-level call edges

`ArchModel.import_edges` are package-level edges (one per `import` statement, keyed by package path). `ArchModel.call_edges` are function-level call edges (one per call site, keyed by `SymbolNode::id`). For cycle detection in the projected graph, use import edges (package-level). For exported-surface delta (which specific symbols cross the boundary), use call edges. Mixing the two produces incorrect results.

## Pitfall 5: Partition coverage — unassigned files

If the caller provides a partition `{P1: [f1.go, f2.go], P2: [f3.go]}` but the target package has four files (`f4.go` is missing from the partition), the simulator has two reasonable choices:

1. Fail with "unassigned files: f4.go — all files must be assigned to a partition part."
2. Implicitly assign unassigned files to a residual part `P_residual`.

The fail-early approach is safer: unassigned files mean the caller hasn't fully specified the partition, and the simulation result would be misleading (it covers only the provided subset). **Default: fail with a diagnostic listing unassigned files.** A `--allow-residual` flag can relax this for exploratory use.

## Pitfall 6: `init()` ordering — don't try to be clever

Go's spec guarantees `init()` functions in a single file run in source order, and between files according to the build-dependency partial order. For two `init()` functions in different files that will land in different parts after a split, whether their ordering invariant is preserved depends on the full build graph — not just the files in the target package. **Flag all `init()` presences as a blocker for human review; do not attempt to prove ordering safety statically.** The fixture should include a case where two `init()` functions have an ordering constraint to exercise this conservative policy.

## Pitfall 7: Cycle check performance on large graphs

The projected graph is small (typically O(10s) of packages), so Tarjan's SCC is fast. However, building the projected graph from `ArchModel.import_edges` (which may contain thousands of edges across the whole repo) requires filtering to the edges relevant to the split. Filter upfront: collect only edges where `from == P` or `to == P`, redirect them per the partition assignment, then run cycle detection on the resulting small subgraph. Do not run cycle detection on the full `import_edges` list.

## Pitfall 8: MCP partition input — ambiguous file path format

Files in a partition must be specified relative to the target package directory (e.g., `checker.rs`) or as repo-relative paths (e.g., `src/checker/checker.rs`). The MCP tool and CLI must agree on which format is authoritative, and the simulator must resolve both to the same `PathBuf` keys used in `ImportGraph.file_packages`. **Use repo-relative paths as the canonical format**, matching `import_graph.rs`'s own `file_packages` key convention.

## Pitfall 9: Generated files may not be in `ArchModel` at all

`build_model` skips generated files entirely (`generated_files_skipped`). If a user includes a generated file in their partition, the simulator will not find it in `PackageNode.files` and may silently ignore it. **Check all user-specified partition files against `PackageNode.files`; warn about any specified file that is absent (it may be generated, missing, or outside the scanned root).**
