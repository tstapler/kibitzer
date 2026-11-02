# Research: Stack and Prior Art — pkg-split-sim

**Date**: 2026-11-01

## Existing kibitzer components reused

| Component | Reuse | Notes |
|---|---|---|
| `arch_model.rs` — `ArchModel`, `build_model` | Core data source | Must pass `PruneConfig { include_private: true }` |
| `import_graph.rs` — `ImportGraph`, `ImportEdge` | Import-edge enumeration | `edges_from()` walks forward; simulator builds reverse index in-memory |
| `affected.rs` — `compute_affected` | Reverse-dep BFS for before/after counts | Deduplicated by package key — no `fan_in_out` bug |
| `architecture_checks.rs` — `ArchitectureChecker` / `ArchModelChecker` traits | Dispatch pattern | `split_sim` is a new `ArchitectureAction` variant, not a check |
| `union_find.rs` | Optional: accelerate SCC | DFS cycle check may suffice for small projected graphs |
| `jaccard.rs` | Optional: cluster suggestion | Jaccard similarity over co-used symbols for advisory cluster leads |
| `symbol_extract.rs` | `init()` detection | Filter `Function` symbols by `name == "init"` |
| `mcp.rs` | MCP tool registration | New `split_sim` tool alongside `get_architecture_node` etc. |

## Rust cycle-detection options

- **DFS back-edge detection**: O(V + E) time, O(V) stack. Sufficient for the projected graph (typically a handful of packages). Already used in `import_graph.rs`'s `ImportCycleChecker` via `petgraph` crate.
- **Tarjan's SCC**: also O(V + E), gives full SCC decomposition, useful if multiple cycles exist. Same cost; cleaner output (SCC lists, not just one back edge).
- **`petgraph`**: already in `Cargo.toml` (`[dependencies]` visible from the codebase structure). `petgraph::algo::is_cyclic_directed` or `petgraph::algo::tarjan_scc` are drop-in. No new dependency.

## Output format

Matching `arch_export.rs`'s `--format json|text` precedent:

- `json`: a structured `SplitSimResult` (serializable via `serde_json`), machine-readable for agents.
- `text`: human-readable summary with section headers, bullet lists for blockers and caller sites.

Both are already used across the architecture subcommands; no new format infrastructure needed.

## MCP tool shape

Existing MCP tools (`list_architecture_symbols`, `get_architecture_node`, `list_callers`, `list_callees`, `list_subtypes`, `list_supertypes`) in `src/mcp.rs` all follow a "params struct → JSON result" pattern. The `split_sim` tool will follow the same shape:

```json
{
  "package": "github.com/tstapler/kibitzer/src/checker",
  "partitions": [
    {"name": "P1", "files": ["checker.rs", "checker_tests.rs"]},
    {"name": "P2", "files": ["grammar_cache.rs"]}
  ]
}
```

Result is a `SplitSimResult` JSON object.

## CLI shape

Nested under `ArchitectureAction` in `main.rs`, matching `Affected`'s recent precedent:

```
kibitzer architecture split-sim <PACKAGE> \
  --partition P1=file1.go,file2.go \
  --partition P2=file3.go \
  [--format json|text] \
  [--include-cluster-suggestion]
```

`--partition` may appear multiple times; remaining files not assigned to any named part automatically form an implicit residual part, or the user must assign all files explicitly (fail with a diagnostic otherwise).

## No new dependencies needed

All algorithm primitives (`petgraph` for cycle detection, `serde_json` for output, tree-sitter for `init()` detection) are already in `Cargo.toml`. The only new code is the simulation logic itself.
