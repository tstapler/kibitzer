# Research: Architecture — interface-impl-carveout-delegation

**Date**: 2026-10-10

## Data available in ArchModel

`ArchModel.type_edges: Vec<TypeRelationEdge>` exists since the `type-hierarchy-graph`
project landed.  Each edge has:
- `from: String` — the subtype's `SymbolNode::id` (`"{pkg}::{TypeName}"`)
- `to: String` — the interface's `SymbolNode::id` when `resolved == true`, raw text otherwise
- `kind: Option<TypeRelationKind>` — `Some(Implements)` for explicit `implements` syntax
- `resolved: bool` — `true` if `to` was matched to a known `SymbolNode` in the model

Languages with explicit `implements` in type_edges (captured in v1):
- TypeScript / TSX: `class Foo implements Bar`
- JavaScript: same (class syntax)
- Java / Kotlin: `class Foo implements/: Bar`

Go is explicit embedding only (`struct embeds interface → kind == Some(Implements)`
via `go_embedded_interface_relations`). Go structural satisfaction (method-set matching)
is **not** captured — scoped out in `project_plans/type-hierarchy-graph/requirements.md`.
Since `call_graph_supports` covers Go and TS/TSX/JS, and only TS/TSX/JS have explicit
`Implements` edges in type_edges, the carve-out is immediately useful for TS/TSX/JS
and must wait for Go structural satisfaction for Go.

## How to reconstruct a method's owning type id

`SymbolNode` for a method has:
- `kind == SymbolKind::Method`
- `parent: Some("TypeName")` — the owning type's *name* (not id)
- `id: "{pkg}::{TypeName}.{methodName}"` — from `build_id(package_path, Some(parent), name)`

To find the owning type's `SymbolNode::id`:
```
package = symbol.id.split_once("::").map_or("", |(pkg, _)| pkg)
type_id  = format!("{package}::{parent_name}")
```

This matches how `caller_package` extracts the package half (`arch_model.rs:545-547`).
The type id formula is `build_id(package_path, None, parent_name)` →
`"{package}::{TypeName}"`, which matches `TypeRelationEdge.from` for the type.

## Query pattern to detect "owning type implements any interface"

```rust
fn owning_type_implements_any_interface(method: &SymbolNode, type_edges: &[TypeRelationEdge]) -> bool {
    let Some(ref parent) = method.parent else { return false; };
    let package = method.id.split_once("::").map_or("", |(pkg, _)| pkg);
    let type_id = format!("{package}::{parent}");
    type_edges.iter().any(|e| {
        e.from == type_id
            && e.resolved
            && matches!(e.kind, Some(TypeRelationKind::Implements))
    })
}
```

Linear scan of `type_edges` per candidate method. `type_edges` is typically much
smaller than `call_edges` — O(types × interfaces), not O(calls). An upfront
`HashSet<String>` of "types that implement any interface" (built once per `check` call)
eliminates repeated scans: one pass over `type_edges` to build the set, then O(1) per
candidate.

## Counting implementers per interface (for "single implementer" exemption)

To answer "how many types implement this interface?":
```rust
fn implementers_of(interface_id: &str, type_edges: &[TypeRelationEdge]) -> usize {
    type_edges.iter()
        .filter(|e| e.to == interface_id
                 && e.resolved
                 && matches!(e.kind, Some(TypeRelationKind::Implements)))
        .count()
}
```

Build an inverted index `interface_id -> count` once per `check` call for efficiency.

## Where the logic lives

Two options:
1. **Private to `single_call_site_delegation.rs`**: fastest to ship, no API surface
   design risk.  Caller just passes `&model.type_edges` to the helper.
2. **`ArchModel` method** (e.g. `fn types_that_implement_interface(…)`): reusable by
   future checkers.  A bit of API design upfront; premature if only one checker needs it.

**Recommendation**: put the helper private to the checker for now.  When a second
checker needs the same query, extract to `ArchModel` then (YAGNI).

## Single-implementer exemption: conservative vs. nuanced

Conservative rule (always suppress interface methods):
- Simple to implement and test.
- Zero false-carve-outs: if a type ever starts implementing an interface, the method
  stops being a candidate.
- The downside: an interface with exactly one implementer, inlined elsewhere, would
  also be suppressed unnecessarily — but this is a small, acceptable loss.

Nuanced rule (suppress unless sole implementer):
- More precise: a type that is the only implementer of an interface is effectively a
  one-off; the interface might be deleted, not expanded.  The inlining suggestion is
  still valid.
- Requires the inverted implementer-count index above.
- Slightly more test surface needed.

**Recommendation**: implement the nuanced rule.  It matches #48's original scope notes
("unless the type has a single implementation and the interface has a single
implementer"), it has clear precedent in the codebase (checkers that check counts before
flagging — see `ISP_MIN_CONSUMERS`, `ISP_MIN_INTERFACE_METHODS`), and the additional
complexity is minimal (one HashMap build + one count check).  The conservative rule is
a valid fallback if the implementer counting proves tricky in testing.

## No ArchModel change needed

The existing `ArchModel.type_edges` field carries all required information.  No new
extraction in `symbol_extract.rs`, no new field in `ArchModel`, no schema change.
This is a pure query change in `single_call_site_delegation.rs`.

## Go: what to document

The residual false-positive for Go methods remains.  The doc comment in
`single_call_site_delegation.rs` should be updated from "tracked as #117" to
"Go structural satisfaction (method-set matching) is out of scope for type_edges v1 —
see `arch_model.rs:226-230`; Go methods that satisfy interfaces implicitly remain a
potential false positive."  This is accurate and self-sufficient without a GitHub issue
reference that may become stale.
