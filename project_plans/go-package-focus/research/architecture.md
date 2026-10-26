# go-package-focus: Architecture Research

## 1. fan_in_out bug — exact defect and fix

### What `fan_in_out` does

`fan_in_out` at `src/architecture_checks.rs:180-187` iterates the raw
`&[ImportEdge]` slice and increments a counter for each edge:

```rust
pub fn fan_in_out(edges: &[ImportEdge]) -> HashMap<&str, (usize, usize)> {
    let mut counts: HashMap<&str, (usize, usize)> = HashMap::new();
    for edge in edges {
        counts.entry(edge.from.as_str()).or_default().0 += 1;  // Ce
        counts.entry(edge.to.as_str()).or_default().1 += 1;    // Ca
    }
    counts
}
```

### The bug

`ImportEdge` records **one row per import statement** (`file` + `line` are
stored so findings can cite a concrete location). A package that imports
another package from multiple files within the same package emits multiple
`ImportEdge` entries with the same `(from, to)` pair — one per file. Each
such duplicate increments Ce/Ca independently, inflating both metrics.

Concretely: if `pkg/server` imports `pkg/store` in `server.go`, `grpc.go`,
and `http.go`, that produces three `ImportEdge` entries for the same
`(pkg/server, pkg/store)` pair. `fan_in_out` counts each one, so
`pkg/server.fan_out += 3` instead of `+= 1`, and `pkg/store.fan_in += 3`
instead of `+= 1`. The resulting Ce, Ca, and `I = Ce/(Ca+Ce)` values are all
wrong in proportion to per-package import-statement density.

### Downstream consumers

`fan_in_out` is called by:

- `InstabilityChecker::check` (`architecture_checks.rs:614`) — uses Ce/Ca to
  compute Martin's Instability metric `I = Ce/(Ca+Ce)`; inflated values push
  packages toward `I≈1.0` (unstable) regardless of their actual coupling.
- `DipConcreteCouplingChecker::check` (`architecture_checks.rs:680`) — uses
  `fan_in_out` to determine whether the *from* package is "stable" before
  flagging it. It already deduplicates the edge pairs itself (via
  `BTreeSet<(&str, &str)>`) for its own `pairs` iteration, but it still passes
  the raw `model.import_edges` to `fan_in_out`, so its stability guard uses
  inflated numbers.
- `fan_out_findings` and `fan_in_findings` (the `coupling` checker, called
  from the `CouplingChecker` path) — use the returned counts to flag packages
  above `MAX_FAN_OUT`/`MAX_FAN_IN`.

### The fix

Deduplicate `(from, to)` pairs before counting by building a `HashSet` of
the distinct pairs first:

```rust
pub fn fan_in_out(edges: &[ImportEdge]) -> HashMap<&str, (usize, usize)> {
    let distinct: HashSet<(&str, &str)> = edges
        .iter()
        .map(|e| (e.from.as_str(), e.to.as_str()))
        .collect();
    let mut counts: HashMap<&str, (usize, usize)> = HashMap::new();
    for (from, to) in distinct {
        counts.entry(from).or_default().0 += 1;
        counts.entry(to).or_default().1 += 1;
    }
    counts
}
```

This matches what `DipConcreteCouplingChecker` already does for its own pairs
set — a precedent already in the file. No callers need to change; the returned
`HashMap<&str, (usize, usize)>` type is unchanged.

**Disposition: Refactor-first.** The bug is a correctness issue that
inflates every Ce/Ca/instability metric in the system; any `go-package-focus`
checker that calls `fan_in_out` to characterize cross-boundary edges would
inherit corrupt numbers. Fix it as the first commit of this work so all
metrics, new and old, are built on a correct baseline.

---

## 2. ArchModel and private-symbol scope

### ArchModel fields

`ArchModel` (`src/arch_model.rs:255-278`) carries:

| Field | Type | Content |
|---|---|---|
| `packages` | `BTreeMap<String, PackageNode>` | One node per package-directory; each holds `files` and `symbols: Vec<SymbolNode>` |
| `import_edges` | `Vec<ImportEdge>` | Cross-package import statements (one row per file×import) |
| `call_edges` | `Vec<CallEdge>` | Function/method call edges; `resolved` flag; unresolved kept as raw callee text |
| `field_accesses` | `Vec<FieldAccessEdge>` | Method→field accesses (read/write), Go-only |
| `type_edges` | `Vec<TypeRelationEdge>` | Extends/implements hierarchy edges |
| `file_import_aliases` | `BTreeMap<PathBuf, HashMap<String, String>>` | Per-file `alias → package path` maps (Go) |
| `pruning` | `PruningSummary` | Provenance: counts of skipped symbols, parse errors, `pruned_symbol_ids` |

### What `needs_private_symbols() = true` provides

`PruningSummary` (`src/arch_model.rs:153-170`) records:

- `include_private: bool` — set from `PruneConfig::include_private`
- `private_symbols_skipped: usize` — unexported symbols excluded when `include_private = false`
- `pruned_symbol_ids: Vec<String>` — the exact `SymbolNode::id`s of every pruned symbol

When `needs_private_symbols()` returns `true`, `build_arch_model_for_check`
passes `PruneConfig { include_private: true }` to `build_model`. Every
unexported symbol appears in `PackageNode::symbols` — `SymbolNode::exported`
is `false` for them. The `pruning.private_symbols_skipped` counter will be 0,
and `pruning.pruned_symbol_ids` will be empty.

For `go-package-focus`, private symbols are essential: Go's idiomatic design
concentrates logic in unexported helpers, and a responsibility hypothesis built
only from exported symbols would miss most of a package's internal structure.
The checker must set `needs_private_symbols() = true`.

### Coverage diagnostics from PruningSummary

The checker should surface the following from `model.pruning` in its output:

- `pruning.private_symbols_skipped` — if nonzero, coverage is incomplete
  (model was built without `include_private = true`, despite the checker's
  request — shouldn't happen if wired correctly, but worth asserting defensively)
- `pruning.pruned_symbol_ids` — can enumerate specifically what was excluded
- `pruning.files_with_parse_errors` — files whose symbols are missing from the model
- `pruning.unsupported_language_files` / `total_files_scanned` — fraction
  coverage for mixed-language repos

A finding's output should include a `[coverage]` diagnostic block citing these
counts so the caller can tell "no responsibilities found" from "most of the
repo was unparseable."

---

## 3. extract_class vs. go-package-focus — keeping them distinct

### extract_class scope

`src/extract_class.rs` operates at the **intra-type** level: given one Go
struct/type, it clusters that type's methods by their shared field-access
overlap (Jaccard similarity over `ArchModel::field_accesses`), then ranks
dendrogram cuts by an Entity Placement cohesion score. The unit of output is
an `ExtractClassCandidate`: which methods of *one* type should move to a *new
type inside the same package*.

Key signals used: `ArchModel::field_accesses` (method→field), `SymbolKind::Method`,
`SymbolNode::parent` (owning type), file extension `.go`.

### go-package-focus scope

`go-package-focus` operates at the **intra-package** level: given the full
symbol set of one Go package, it clusters symbols (types + functions + methods
as a group) by their cross-symbol call edges, import edges, and field-access
state groupings to ask "do these symbols form one coherent responsibility, or
two?"

The unit of output is a responsibility hypothesis: which symbols cluster
together, which cross-boundary call edges and import directions stitch them,
and what state (struct fields) each cluster owns.

### How to avoid labeling Extract Class findings as package decomposition

The two checkers should be treated as complementary, not contradictory:

- **extract_class finds**: "methods `A`, `B`, `C` on type `Foo` share no
  fields with methods `D`, `E`, `F` — split `Foo` into two types."
- **go-package-focus finds**: "symbols in group 1 (`TypeX`, `funcA`, `funcB`)
  never call symbols in group 2 (`TypeY`, `funcC`, `funcD`) and import
  different packages — split this package into two."

A finding from `go-package-focus` that proposes splitting a package should
**not** list the individual method groups of a single type as responsibilities
— that would overlap with `extract_class`. The guard is straightforward:
`go-package-focus` works at `SymbolNode` granularity where each node is a
type, free function, or method, and it **groups by top-level type boundary**
first (all methods on `TypeX` belong to `TypeX`'s cluster before cross-type
edges are considered). Only if two types' entire method sets are disconnected
from each other does the checker propose a package split, not if two methods
on the *same* type are disconnected (that's `extract_class`'s domain).

Concretely: when building the call-graph adjacency for `go-package-focus`,
treat all methods on a type as a single super-node for the purposes of
package-level clustering. This collapses intra-type structure (handled by
`extract_class`) out of the package-level signal.

---

## 4. ArchModelChecker integration

### Trait signature

```rust
pub trait ArchModelChecker {
    fn name(&self) -> &str;
    fn check(&self, model: &ArchModel, config: &ArchitectureConfig) -> Vec<ArchFinding>;
    fn needs_private_symbols(&self) -> bool { false }
}
```

`ArchFinding` carries `file: Option<PathBuf>`, `line: Option<usize>`,
`message: String`, `severity_override: Option<Severity>`.

### Registration

New checkers are added to `model_registry()` in `src/architecture_checks.rs:567`.
`lookup_model` resolves by `name()` for the CLI `kibitzer check native <name>`
path. The checker should also be added to `default_checks()` in `src/config.rs`
once it passes backtest bars — same convention as every other native checker.

### Data available for cross-package edge analysis

For a `go-package-focus` checker operating on a single package:

1. **Intra-package call edges**: filter `model.call_edges` where both `from`
   and `to` resolve to symbols within the target package (cross-package calls
   are where `to`'s package prefix differs from `from`'s).

2. **Cross-package call edges**: `model.call_edges` where `to`'s package prefix
   is outside the target package. These are the "import direction" signals.

3. **Import edges**: `model.import_edges` filtered to `edge.from == pkg.path`.
   After the `fan_in_out` dedup fix, these give the correct distinct-package
   fan-out.

4. **Field accesses**: `model.field_accesses` to see which methods in the
   package touch which struct fields — used to identify state ownership groups.

5. **Type hierarchy edges**: `model.type_edges` for interface satisfaction
   relationships between types in the package.

### Output shape for go-package-focus

The checker produces `ArchFinding` entries with a structured message format:

```
[go-package-focus] {package}: responsibility cluster {N}/{total}
  symbols: {sym1}, {sym2}, {sym3}
  state: {field_id1}, {field_id2}
  cross-boundary calls: {sym4} → {sym5} ({N} edges)
  import direction: → {dep_pkg1}, → {dep_pkg2}
[go-package-focus] {package}: {N} unresolved cross-package call edges excluded
[go-package-focus] {package}: {N} symbols with no field access or call data (clustering blind spots)
```

One finding per candidate split (only emitted when `LCOM4-style component
count > 1` at the package level), plus explicit coverage diagnostics (unresolved
edges, parse-error files).

---

## 5. Build-cost collector — placement

### Requirement

A "Go build-cost collector" that reads `go build -debug-actiongraph` output to
profile which packages are expensive to compile. This is explicitly described as
"separate from per-edit lint."

### Options considered

| Option | Fit |
|---|---|
| New native per-file check | Wrong: build cost is a build-time, whole-repo signal; not a per-file diff check |
| Plugin (`kibitzer plugin install`) | Possible but heavyweight; plugins are for third-party checkers, not first-party prototypes |
| `ArchModelChecker` / `ArchitectureChecker` | Wrong shape: build cost needs to invoke `go build`; no check trait does I/O |
| New CLI subcommand | Best fit: matches the `kibitzer architecture assessment` pattern for whole-repo, separate-invocation analysis |
| Separate binary | Overkill for a prototype; increases release surface |

### Recommendation: new `kibitzer go build-cost <dir>` subcommand

Under `src/main.rs`, add a `go build-cost` subcommand (within the existing `go`
command group if one exists, or as a new top-level `go` group). The
subcommand:

1. Runs `go build -debug-actiongraph /tmp/actiongraph.json ./...` in `<dir>`.
2. Parses the actiongraph JSON (a list of `{ID, Package, Deps, TimedOut, ...}`)
   to extract per-package compile time.
3. Emits a ranked table of packages by build time, annotated with fan-out from
   `ArchModel::import_edges` (correlates "slow to build" with "many dependents").
4. Version-gates via `go version` check before invoking `-debug-actiongraph`
   (the flag was added in Go 1.19; earlier toolchains silently ignore it and
   produce no output).

This is a prototype: no `ArchFinding`s, no check registration. Output is
human-readable text to stdout. The version-gate and the actiongraph JSON
parsing are the two integration risks to prototype and validate before
committing to a full check-based design.

The subcommand lives in its own file (`src/cmd/go_build_cost.rs` or inline in
`main.rs`'s command dispatch) and does not touch the check pipeline at all
— it's an advisory diagnostic tool, not a lint.
