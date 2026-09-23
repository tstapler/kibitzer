# ADR-001: `list_supertypes`/`list_subtypes` take a `depth` parameter (BFS, mirroring `list_callers`/`list_callees`), not a direct-only filter

**Status**: Accepted
**Date**: 2026-09-22

## Context

`requirements.md`'s Open Questions section flags an explicit, unresolved disagreement
between two Phase 2 research agents:

- `research/architecture.md` §3 recommends `list_supertypes`/`list_subtypes` be a simple
  direct-edges-only filter (`edge.to == node` / `edge.from == node` over
  `ArchModel.type_edges`) — no `depth` parameter, no BFS, no `truncated` flag. Its stated
  reasoning: "neither #59/#60/#61's stated needs nor the MCP tool naming... imply a
  multi-hop walk — this research found none," and that `traverse_call_edges` is
  "hard-typed to `&[CallEdge]` — not a reusable generic," so porting it would be
  over-fitting.
- `research/features.md` §2 and §4 directly contradicts this with more specific evidence,
  verified against `src/mcp.rs`: `list_callers`/`list_callees` are **not** one-hop-only
  today — `CallTraversalRequest` (`src/mcp.rs:169-180`) already takes a `depth` parameter
  (default `1`, clamped to `[1, MAX_CALL_DEPTH=10]`, `src/mcp.rs:188`), BFS-walked via
  `traverse_call_edges` (`src/mcp.rs:314-341`) with a visited-node set and a `truncated`
  response flag. `research/features.md` §4 also traces a concrete, catalog-documented
  consumer need: Extract Superclass (issue #61, per `docs/refactoring-catalog-analysis.md`)
  requires checking a type's **full ancestor chain** for "no existing hierarchy edge" — two
  types with no *direct* shared parent could still share a distant ancestor, which a
  direct-only tool cannot answer without the caller re-implementing BFS itself outside
  kibitzer.

Both research agents agree on one thing, which this ADR does not revisit:
`ArchModel.type_edges` itself stores only **direct** edges either way. This decision is
scoped entirely to the MCP query-tool surface (`list_supertypes`/`list_subtypes`), not the
model's storage shape.

## Decision

**Adopt `research/features.md`'s recommendation: `list_supertypes`/`list_subtypes` take a
`depth` parameter, defaulting to `1`, clamped to `[1, MAX_TYPE_HIERARCHY_DEPTH=10]`, walked
via a `TypeRelationEdge`-typed BFS helper with a visited-node set and a `truncated`
response flag** — the same shape `list_callers`/`list_callees` already establish for the
call graph.

Reasons, in order of weight:

1. **`research/features.md`'s evidence is more specific and independently verified against
   the actual code**, not just the tool-naming intuition `research/architecture.md` reasons
   from. `list_callers`/`list_callees` genuinely already support multi-hop traversal today
   — `research/architecture.md`'s premise that the precedent is "one-hop-only" is factually
   wrong (confirmed by reading `src/mcp.rs:169-188` directly, cited above). A design
   decision built on a mistaken premise about the existing precedent doesn't outweigh one
   built on the precedent as it actually is.
2. **A real, catalog-documented consumer need exists** (Extract Superclass, issue #61) that
   a direct-only tool cannot serve without the caller (a future checker) re-implementing
   BFS-with-cycle-safety outside kibitzer — exactly the kind of duplicated, easy-to-get-wrong
   logic (see `research/pitfalls.md` §3's note on cycle safety) a library-internal MCP tool
   should own once, not push onto every consumer.
3. **Consistency with an established MCP tool-family convention.** An agent already
   competent with `list_callers`/`list_callees`'s `{node, depth, truncated, edges}` envelope
   transfers that competence to `list_supertypes`/`list_subtypes` with zero new
   response-shape learning (per `research/ux.md` §3) — a `depth`-less pair would be the one
   inconsistent tool in the family, not the simpler one.
4. `research/architecture.md`'s objection that `traverse_call_edges` is "hard-typed to
   `&[CallEdge]`, not a reusable generic" is correct but doesn't argue against a `depth`
   parameter — it argues against literally reusing `traverse_call_edges`'s code. The
   resolution is a new, `TypeRelationEdge`-typed sibling (`traverse_type_edges`,
   `build_type_adjacency`, `expand_type_frontier`) structurally mirroring the call-graph
   trio, not a generic. This is what Phase 3's plan implements (see
   `implementation/plan.md`, Story 2.1.1) — a small amount of duplicated BFS scaffolding,
   in exchange for the tool actually answering #61's need.

**Reconciling with `requirements.md`'s separate, non-negotiable pagination constraint**:
neither research doc's recommendation, taken alone, satisfies `requirements.md`'s Non-
functional Requirements section, which states `list_supertypes`/`list_subtypes` "must
follow the same cursor/limit/next_cursor convention as `list_architecture_symbols`" —
`research/architecture.md`'s direct-only design has no depth field but does have
pagination; `research/features.md`'s BFS design has depth/truncated but doesn't specify
pagination. `implementation/plan.md`'s `TypeHierarchyRequest`/`TypeHierarchyResponse`
combine both: `depth`+`truncated` from the call-traversal precedent, `limit`/`cursor`/
`next_cursor`/`possibly_pruned` from the `list_architecture_symbols` precedent. Pagination
applies to the BFS's already-collected edge list (post-traversal), not to the walk itself
— `depth` bounds how far the walk goes; `limit`/`cursor` bound how many of the resulting
edges one response page returns.

## Consequences

- New scaffolding in `src/mcp.rs`: `TypeHierarchyDirection` enum, `build_type_adjacency`,
  `expand_type_frontier`, `traverse_type_edges`, `MAX_TYPE_HIERARCHY_DEPTH` — structurally
  parallel to the existing `CallDirection`/`build_call_adjacency`/`expand_call_frontier`/
  `traverse_call_edges`/`MAX_CALL_DEPTH` quintet, not a shared generic. Duplication is
  accepted here the same way `FieldAccessEdge`'s resolution machinery duplicates (rather
  than generalizes) `CallEdge`'s — see `research/architecture.md` §1's own citation of that
  precedent.
- `list_supertypes`/`list_subtypes`'s tool descriptions must state the `depth` default and
  clamp bound explicitly (per `research/ux.md` §1 point 4), the same way
  `CallTraversalRequest::depth`'s doc comment does.
- A hierarchy cycle (only reachable via a misresolved edge — Go/Kotlin `extends`/
  `implements` are compile-errors on genuine cycles) is defended against by the same
  visited-set mechanism `traverse_call_edges` already proved correct for the call graph's
  recursive-function case (`research/features.md` §3, citing
  `traverse_call_edges_self_loop_terminates_via_visited_set_not_depth`,
  `src/mcp.rs:1178-1186`).
