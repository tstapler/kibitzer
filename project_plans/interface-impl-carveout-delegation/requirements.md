# Requirements: interface-impl-carveout-delegation

**Date**: 2026-10-10
**Type**: feature / checker improvement
**Complexity**: 2 — consuming an existing model field; no new model infrastructure needed
**Source**: backlog item `7600247a-5e5f-4d4c-b1d3-e740cddc6a53` / GitHub issue #117 (follow-up to #48 / PR #116)

## Problem Statement

`single-call-site-delegation` (`src/single_call_site_delegation.rs`) flags a private
function/method whose entire body is one delegating call and which has exactly one
resolved caller — Fowler's *Inline Function* smell.  Issue #48's original scope notes
explicitly requested a carve-out for methods that implement an interface, because a
single call site *today* does not mean the interface will not gain a second
implementer/caller tomorrow.

That carve-out was deferred in PR #116 (issue #117 tracks it) because `ArchModel` had
no `type-implements-interface` edge at the time.  The `type-hierarchy-graph` project
(PR that landed `TypeRelationEdge` / `ArchModel.type_edges`) has since shipped those
edges.  `type_edges` with `kind == Some(TypeRelationKind::Implements)` now exist for
TypeScript, TSX, JavaScript, Java, and Kotlin — languages whose source uses explicit
`implements` syntax.

Go is the primary language for `is_pure_delegation` (Go and TS/TSX/JS are the only
`call_graph_supports` languages), but Go uses structural interface satisfaction (method
set, no keyword) — not captured in `type_edges` v1 (see arch_model.rs:226-230, scoped
out explicitly in `project_plans/type-hierarchy-graph/requirements.md`).  That remains
a known gap and does not block wiring in the carve-out for languages that already have
explicit `Implements` edges.

## Baseline

- `single_call_site_delegation.rs::is_candidate` checks: `!exported && is Function/Method && is_pure_delegation`.
- No interface check: the comment in `single_call_site_delegation.rs:10-12` explicitly
  notes "No interface-implementation carve-out yet — tracked as #117."
- `ArchModel.type_edges: Vec<TypeRelationEdge>` is populated (landed in type-hierarchy-graph).
- `TypeRelationKind::Implements` edges exist for TS/TSX/JS/Java/Kotlin explicit `implements`.
- Go structural satisfaction is **not** in `type_edges`; methods on types that satisfy Go
  interfaces implicitly remain a documented false-positive source until a future item
  adds method-set matching.

## Users / Consumers

- Any project using kibitzer on a TypeScript, TSX, or JavaScript codebase that has interface
  implementations — currently those projects see false-positive `single-call-site-delegation`
  findings on every private method that implements an interface.
- Future: Go codebases once Go structural satisfaction is added.

## Success Metrics

- A private TypeScript/TSX/JS method whose owning type has at least one `Implements`
  `TypeRelationEdge` is **not** flagged by `single-call-site-delegation`, regardless of
  how many callers it has.
- Nuanced guard: if the implementing type is the **sole** implementer of the interface
  and the interface has exactly one implementer, the method **may** still be flagged
  (it is as safe to inline as a non-interface method under those conditions).  This is
  an optional refinement; the simpler conservative rule (always suppress for any
  interface method) is also acceptable as v1.
- A private Go method that happens to have one caller is still reported (no change —
  Go structural satisfaction is out of scope; no regression).
- The doc comment on line 10-12 of `single_call_site_delegation.rs` is updated to
  remove the "tracked as #117" note, replaced by the language-scoped caveat for Go
  structural satisfaction.
- All existing unit tests pass unchanged; new tests cover the TS/Go cases.

## Appetite

Small (1-2 days). No new model infrastructure; the consuming query is a linear scan of
`model.type_edges` that mirrors patterns already in `isp_fat_interface.rs` and
`architecture_checks.rs`.

## Constraints

- Must not modify the `ArchModel` data itself — only add query logic in
  `single_call_site_delegation.rs`.
- Must not break existing unit tests.
- Go false-positive note in docs or comments must remain accurate.

## Non-functional Requirements

- **Performance**: `caller_counts` already does a linear scan of `call_edges`; the
  interface check adds one more linear scan of `type_edges` (typically much smaller —
  O(types × interfaces), not O(calls)).  Acceptable for an `ArchModelChecker`.
- **Correctness over completeness**: if `type_edges` has no `Implements` edge for a
  given method's owning type, the existing behavior is unchanged (flag if single caller).
  Unknown ≠ "no interface."

## Scope

### In Scope

- `is_candidate` in `single_call_site_delegation.rs`: add interface-implements check
  against `model.type_edges`.
- Helper: a function (private to the checker or exposed in `arch_model.rs`) that answers
  "does type_id implement any interface in this model?" — built from `type_edges`.
- Optional: count implementers per interface and apply the "single implementer exemption."
- Unit tests:
  - TS method with `implements` → not flagged even with one caller.
  - TS method with `implements` on an interface with only one implementer → flagged (if
    the single-implementer exemption is implemented) or not flagged (conservative v1).
  - Go method (no interface edges) → existing behavior unchanged (flagged if single caller).
- Update `single_call_site_delegation.rs` doc comment to reflect Go-only residual gap.

### Out of Scope

- Go structural interface satisfaction (method-set matching against all interfaces in the
  model) — explicitly deferred; not in `type_edges` v1.
- Any change to `ArchModel`, `TypeRelationEdge`, or `symbol_extract.rs`.
- Java/Kotlin method carve-out (those languages' `is_pure_delegation` is always `false`
  since they are not `call_graph_supports` languages — the carve-out would be a no-op
  today; include in tests for completeness only if convenient).

## Rabbit Holes

- Go method `parent` is a type name (string), not a `SymbolNode::id`. To look up a
  method's owning type in `type_edges`, need to reconstruct the type's id from its
  package path + name.  Pattern already established in `isp_fat_interface.rs::
  collect_interface_usage` (same package/name → id reconstruction).
- `type_edges` stores `from` as the subtype's `SymbolNode::id` and `to` as the
  interface's id (when resolved) or raw text (when unresolved).  Use only resolved edges
  (`resolved == true`) to avoid false carve-outs on unresolved external targets.
- If `SymbolNode.parent` is `None` (a free function, not a method), there is no owning
  type to check — skip the interface query for non-methods immediately.

## Feasibility Risks

- Low. The data is already in the model; the query pattern is straightforward.
- The only ambiguity is whether to implement the "single implementer" exemption now or
  treat the simple conservative rule as v1.  Research and planning phases should confirm
  which is simpler and correct to ship first.

## Observability Requirements

Standard — no new logging needed.  The existing `ArchFinding` output is unchanged for
genuine single-call-site-delegation findings.

## Open Questions

1. **Implement the "single implementer" exemption in v1 or defer?** The description
   says "unless the type has a single implementation and the interface has a single
   implementer" — this is more nuanced than simply "skip all interface methods."  The
   conservative rule (skip all) is safe; the exemption requires counting implementers
   per interface from `type_edges`.  Phase 2 research should confirm whether the
   conservative rule produces any real false-negatives on known codebases, or whether
   the exemption is needed from day one.

2. **Should the helper that builds a "type → implements interfaces" index live in
   `arch_model.rs` (as a shared query API) or stay private to
   `single_call_site_delegation.rs`?** The same query will eventually be needed for
   other checkers (once Go structural satisfaction lands, for example).  Phase 3 plan
   should decide.
