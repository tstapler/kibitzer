# Validation: interface-impl-carveout-delegation

**Date**: 2026-10-10  
**Phase**: adversarial review of plan.md

## Finding 1: `is_interface_impl_suppressed` iterates `model.type_edges` twice

The plan builds `types_implementing_any_interface` (HashSet) then calls
`is_interface_impl_suppressed` which also iterates `model.type_edges` a third time per
candidate to collect the interfaces this type implements.  For most repos `type_edges`
is small, but the pattern is inconsistent: `caller_counts` is built once and consulted
O(1) per symbol; `is_interface_impl_suppressed` rebuilds per symbol.

**Resolution**: in Story 3, extend Story 1 to build a third index:
```rust
/// type_id -> Vec<interface_id> (resolved Implements edges only)
fn type_to_interfaces(model: &ArchModel) -> HashMap<String, Vec<String>> {
    let mut map: HashMap<String, Vec<String>> = HashMap::new();
    for edge in model.type_edges.iter()
        .filter(|e| e.resolved && matches!(e.kind, Some(TypeRelationKind::Implements)))
    {
        map.entry(edge.from.clone()).or_default().push(edge.to.clone());
    }
    map
}
```

Pass `&type_to_interfaces` alongside `implementing` and `implementers` to
`is_interface_impl_suppressed`.  Removes the per-symbol `type_edges` scan.

## Finding 2: `implementing` HashSet is redundant once `type_to_interfaces` exists

If Story 1 builds `type_to_interfaces`, the `implementing: HashSet<String>` is a
derived subset (`implementing = type_to_interfaces.keys().collect()`).  The check
`implementing.contains(&type_id)` becomes `type_to_interfaces.contains_key(&type_id)`.
The separate `HashSet` construction can be dropped: fewer allocations, less code.

**Resolution**: drop `types_implementing_any_interface` entirely; use
`type_to_interfaces.contains_key(…)` in `is_interface_impl_suppressed`.

## Finding 3: sole-implementer exemption logic has an edge case

The plan says: "if every interface it implements has exactly one implementer (this
type), don't suppress."  This means: if a type implements interfaces A and B, A has 1
implementer, but B has 3 implementers — suppress (because of B).

But the correct rule from #48's scope notes is about *this specific method*: a method
that both satisfies interface A (sole implementer) AND interface B (multiple implementers)
is part of a multi-implementer contract (B) — suppress is correct.  The plan's `any`
check (`interfaces.iter().any(|…| implementers > 1)`) is correct for this case.

The edge case: a type implements interface A (sole implementer).  The exemption fires.
Later, a second implementer is added to A in a different file.  The checker now
correctly flags again (because the count changes from 1 to 2 and `any` now returns
true again).  No stale-data concern — `type_edges` is rebuilt each `check` call.

**Status**: no change needed; logic is correct.

## Finding 4: test Story 6 item 1 requires a two-implementer model

The plan says "TS class with `implements`, interface has one other implementer too" but
`model_from` builds from a list of source strings.  Both the class and the other
implementer must be in the same model.  The test helper already supports multiple files
in one `model_from` call.

**Resolution**: confirm `model_from` in `single_call_site_delegation.rs::tests` accepts
multiple files (line 98-112 of the source shows it does: `files: Vec<(&str, &str)>`).
No plan change needed; just note in the test that both implementer files must be in the
same `model_from` call.

## Finding 5: `TypeRelationKind` import path

The plan's Story 4 says add `TypeRelationKind` to the `arch_model` import.  Verify the
current import line in `single_call_site_delegation.rs`:
```rust
use crate::arch_model::{ArchModel, SymbolKind, SymbolNode};
```
`TypeRelationKind` is defined in `arch_model.rs` (line ~213).  The plan's import
addition is correct.

## Finding 6: doc comment references #117 by number — number may drift

The plan proposes removing the "#117" issue reference and replacing with a self-contained
doc comment.  This is correct; issue numbers are ephemeral.  The replacement text in
Story 5 is self-contained (cites `arch_model.rs:226-230` for the scope note location),
which is better for long-term accuracy.  No change needed.

## Finding 7: backtest Story 7 names `kibitzer run` — confirm command exists

`kibitzer run` is mentioned in `CLAUDE.md` / `docs/backtesting.md`.  The corpus repos
are cloned via `scripts/clone-backtest-repos.sh` per `docs/backtest-repos.md`.  Command
confirmed; Story 7 is actionable.

## Summary

Revisions to plan.md from this review:
1. Drop `types_implementing_any_interface` helper; build `type_to_interfaces` index
   instead (Stories 1 and 2 refactored).
2. `is_interface_impl_suppressed` uses `type_to_interfaces` for O(1) per-symbol lookup
   with no per-symbol `type_edges` iteration.
3. All other stories unchanged.

Revised implementation is correct, low-risk, and backtest-complete.

## Verification results (post-implementation)

- AC6 (`kibitzer check backtest single-call-site-delegation`) is not runnable: the backtest only resolves
  per-file `checker::registry()` checkers; this is a whole-repo `ArchModelChecker`
  (`src/architecture_checks.rs:529`). Marked fail.
- Substitute evidence: `kibitzer check architecture single-call-site-delegation <vscode>`, HEAD~1 vs HEAD
  binary: 4059 -> 3222 findings, 0 added, 837 removed (all methods). 12/12 sampled removals are in
  classes with an explicit `implements` clause.
- Full suite: `cargo test --bin kibitzer` 1565 passed, 0 failed.
