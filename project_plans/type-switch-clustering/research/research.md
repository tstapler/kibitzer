# Research: type-switch-clustering

**Date**: 2026-11-04

## Dependency status

Both upstream dependencies are already landed — no blocked waiting:

| Dependency | Status | Evidence |
|---|---|---|
| #38 `go-type-switch-density` | **Landed** | `src/checkers/go_type_switch_density.rs` exists; registered at `src/config.rs:883` |
| #40 `type_edges` / type-hierarchy-graph | **Landed** | `ArchModel.type_edges: Vec<TypeRelationEdge>` at `src/arch_model.rs:266`; `list_supertypes`/`list_subtypes` MCP tools at `src/mcp.rs:1366/1389` |

## Existing infrastructure survey

### go_type_switch_density.rs

- `TypeSwitchDensityChecker` is a per-file `Checker` (not `ArchModelChecker`).
- Internal helpers: `collect_dense_type_switches(node, src, out)`, `type_case_count(type_switch)`,
  `node_text(node, src)`.
- These are `fn`s in the same file, not `pub` — the clustering checker will need them either
  moved to a `pub(crate)` location or duplicated minimally.
- The checker extracts: `line`, `subject` (the switch subject expression text), `case_count`.
  It does NOT extract the owning method name or the owning type. That's what the clustering
  layer needs to add.

### ArchModel / ArchModelChecker integration point

- `src/architecture_checks.rs:~597` introduces a second, `ArchModel`-based checker trait
  ("a third checker shape for a third graph"). The clustering checker is exactly this shape:
  it needs the full `ArchModel` (for `type_edges` and the full `SymbolNode` table with
  `parent`) rather than just the per-file AST.
- `ArchModel.symbols: BTreeMap<String, SymbolNode>` — keyed by id (`pkg::Type::method`).
  `SymbolNode.parent` holds the owning type name (string). `SymbolNode.kind == Method`
  filters to methods.
- `ArchModel.type_edges: Vec<TypeRelationEdge>` — each edge has `from`, `to`, `kind`
  (Some(Extends) / Some(Implements) / None).

### Two-pass design required

The per-file `Checker` interface only sees one file at a time — it cannot cluster across
files. The `ArchModelChecker` runs once per batch against the whole model. A pure
`ArchModelChecker` approach requires the type-switch site data (line, subject, method) to
be re-extracted from source during the architecture check, or pre-collected and stored in
`ArchModel`.

**Option A: Extract type-switch sites during `build_model`** — add a
`type_switch_sites: Vec<TypeSwitchSite>` to `ArchModel` (similar to how `field_accesses`
was added). Site struct: `{file, method_id, subject: String, case_count: usize, line}`.
Pro: single parse pass; clustering can then run as a pure `ArchModelChecker` with zero
re-parsing. Con: adds to `ArchModel` (serialized JSON), potentially noisy for non-Go repos.

**Option B: Re-walk source files within the ArchModelChecker** — the clustering checker
walks each `.go` file, extracts type-switch sites with method context (by traversing up
from `type_switch_statement` to enclosing `function_declaration`/`method_declaration`),
and clusters them. Pro: no `ArchModel` schema change. Con: second parse pass for Go files;
but the checker runs only at `batch` granularity, not on every edit, so the wall-clock
cost is acceptable.

**Recommendation: Option B** for v1. No `ArchModel` schema change keeps the blast
radius small and matches the precedent set by `change_coupling.rs`
(`src/change_coupling.rs`), which does its own git-log walk rather than pre-storing
coupling data in the model. The `type_edges` lookup (to decide Polymorphism vs. Subclasses)
still uses `ArchModel` (read-only). A second tree-sitter walk over Go files is fast
(Go repos typically have fewer source files than the total corpus size; `checkers/` has
precedent for multi-file walkers).

### Literature backing

- Fowler, *Refactoring* 2nd ed., "Replace Conditional with Polymorphism":
  trigger = same conditional chain over a type tag repeated in multiple methods of the
  same class. The clustering threshold (2+ methods) matches Fowler's own description.
- Fowler, "Replace Type Code with Subclasses": used when the type code affects behavior
  *and* no suitable hierarchy exists yet — identical trigger, different target.
- Tsantalis & Chatzigeorgiou (JDeodorant, IEEE TSE 2009) — not directly applicable here
  (JDeodorant's focus is Move Method / Extract Class), but the principle of clustering
  code smells by shared structural anchor (here: the switch subject and owning type)
  mirrors JDeodorant's grouping methodology.

## Go AST shape for the clustering walk

A `method_declaration` in Go's tree-sitter grammar has:
- `receiver` field — the receiver parameter, e.g. `(r *MyType)`
- `name` field — the method name identifier
- Body: `block` → may contain one or more `type_switch_statement` nodes

A `type_switch_statement` has a `value` field (the switched-on expression, e.g. `r.kind`)
and `type_case` children.

The clustering walk:
1. Walk all `.go` files in the repo.
2. For each `method_declaration` node: extract receiver type name, method name.
3. For each `type_switch_statement` within that method: extract subject text, case count.
4. Accumulate `(owner_type, subject) → [(method_name, file, line, case_count)]`.
5. Filter to clusters where len ≥ 2.
6. For each cluster: look up `ArchModel.type_edges` for edges where `from` or `to`
   contains the switched-on type name (the interface the switch is dispatching on).
   If any edge exists → "Replace Conditional with Polymorphism".
   Otherwise → "Replace Type Code with Subclasses".

## Open questions

1. **Switch subject normalization**: `r.kind` vs. `s.kind` for two methods with different
   receiver names but the same field. Conservative (exact string) match is safest for v1;
   document the limitation.
2. **Switched-on type vs. switch-subject**: the switch's subject (`r.kind`) is the
   expression, but the `type_edges` lookup needs the *interface type* being switched on
   (`r.kind` might be of type `Shape` — that's what needs the hierarchy check). Resolving
   that type name from the subject expression requires a type annotation or heuristic. For
   v1: scan `ArchModel.type_edges` for any edge whose `from` or `to` name appears as a
   substring of the case type names in the cluster's switches (the case type names are
   already available from the `TypeCase` nodes). If none match, default to "Subclasses."
3. **Threshold**: 2 methods minimum for a cluster (Fowler's "repeated" framing). Whether
   to also require a minimum `case_count` per method (e.g. ≥ 3, below `go-type-switch-density`'s
   threshold of 4) needs backtest calibration.
