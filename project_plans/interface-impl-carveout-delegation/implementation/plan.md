# Implementation Plan: interface-impl-carveout-delegation

**Date**: 2026-10-10  
**Status**: planning

## Summary

Wire the existing `ArchModel.type_edges` into `single_call_site_delegation.rs::is_candidate`
to suppress findings for methods whose owning type explicitly implements an interface,
with a nuanced exemption when the type is the sole implementer of that interface.

## Pattern Decisions

| Decision | Choice | Rationale |
|---|---|---|
| Conservative vs. nuanced suppression | **Nuanced** (suppress unless sole implementer) | Matches #48's scope notes; minimal extra complexity |
| Helper location | Private to `single_call_site_delegation.rs` | YAGNI — no other checker needs this today |
| Go residual gap | Leave as-is; update doc comment | Go structural satisfaction not in `type_edges` v1 |
| Index shape | Two HashMaps built once per `check` call | Same pattern as `caller_counts` |

## Stories

### Story 1: Build interface-suppression indexes

**File**: `src/single_call_site_delegation.rs`

Add two private helpers analogous to `caller_counts`:

```rust
/// Returns the set of type `SymbolNode::id`s that have at least one resolved
/// `Implements` edge in `type_edges` — used to skip methods whose owning type
/// satisfies any interface.
fn types_implementing_any_interface(model: &ArchModel) -> HashSet<String> {
    model.type_edges.iter()
        .filter(|e| e.resolved && matches!(e.kind, Some(TypeRelationKind::Implements)))
        .map(|e| e.from.clone())
        .collect()
}

/// Number of distinct resolved implementers per interface id — used for the
/// "sole implementer" exemption: a type that is the only implementer of an
/// interface may still be flagged (the interface is effectively private to it).
fn implementers_per_interface<'a>(model: &'a ArchModel) -> HashMap<&'a str, usize> {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for edge in model.type_edges.iter()
        .filter(|e| e.resolved && matches!(e.kind, Some(TypeRelationKind::Implements)))
    {
        *counts.entry(edge.to.as_str()).or_default() += 1;
    }
    counts
}
```

### Story 2: Widen `single_call_site_delegation_findings` to pass indexes

The existing `is_candidate` predicate takes `&SymbolNode` only. To avoid two separate
passes, compute both indexes upfront and pass them to a new `is_candidate` variant (or
use a closure).

Updated `single_call_site_delegation_findings`:

```rust
fn single_call_site_delegation_findings(model: &ArchModel) -> Vec<ArchFinding> {
    let counts = caller_counts(model);
    let implementing = types_implementing_any_interface(model);
    let implementers = implementers_per_interface(model);

    let mut findings: Vec<ArchFinding> = model
        .packages
        .values()
        .flat_map(|pkg| pkg.symbols.iter())
        .filter(|s| is_candidate(s))
        .filter(|s| !is_interface_impl_suppressed(s, model, &implementing, &implementers))
        .filter(|s| counts.get(s.id.as_str()).copied().unwrap_or(0) == 1)
        .map(finding_for)
        .collect();
    findings.sort_by(|a, b| (&a.file, a.line).cmp(&(&b.file, b.line)));
    findings
}
```

### Story 3: Implement `is_interface_impl_suppressed`

```rust
/// Returns `true` if this method should be excluded from single-call-site-delegation
/// findings because its owning type implements an interface — suppressed to avoid
/// false positives on polymorphic surfaces (a second implementer could appear later).
///
/// Exemption: if the owning type is the *sole* implementer of every interface it
/// declares, the method is still a valid inlining candidate (per #48's scope note).
/// Uses only resolved `Implements` edges (explicit `implements` keyword — TypeScript,
/// TSX, JavaScript, Java, Kotlin). Go structural satisfaction is not captured in
/// `ArchModel.type_edges` v1 (see `arch_model.rs:226-230`); Go methods that satisfy
/// interfaces implicitly may still produce false positives until that gap is filled.
fn is_interface_impl_suppressed(
    symbol: &SymbolNode,
    model: &ArchModel,
    implementing: &HashSet<String>,
    implementers: &HashMap<&str, usize>,
) -> bool {
    let Some(ref parent) = symbol.parent else { return false; };
    let pkg = symbol.id.split_once("::").map_or("", |(p, _)| p);
    let type_id = format!("{pkg}::{parent}");

    if !implementing.contains(&type_id) {
        return false; // owning type implements no interface — don't suppress
    }

    // Collect interfaces this type implements.
    let interfaces: Vec<&str> = model.type_edges.iter()
        .filter(|e| {
            e.from == type_id
                && e.resolved
                && matches!(e.kind, Some(TypeRelationKind::Implements))
        })
        .map(|e| e.to.as_str())
        .collect();

    // Exemption: suppress only if the type has a non-sole-implementer interface.
    // If every interface it implements has exactly one implementer (this type),
    // don't suppress — the interface is effectively private.
    interfaces
        .iter()
        .any(|iface_id| implementers.get(iface_id).copied().unwrap_or(0) > 1)
}
```

### Story 4: Add imports in single_call_site_delegation.rs

Add `HashSet` to the `use std::collections::HashMap` line:
```rust
use std::collections::{HashMap, HashSet};
```

Add `TypeRelationKind` to the arch_model import:
```rust
use crate::arch_model::{ArchModel, SymbolKind, SymbolNode, TypeRelationKind};
```

### Story 5: Update doc comment

Replace lines 10-12 of `single_call_site_delegation.rs`:
```
//! No interface-implementation carve-out yet (a single call site today may not be the
//! only one once a second implementer/caller appears) — `ArchModel` has no
//! type-implements-interface edge to detect it with; tracked as #117.
```
With:
```
//! Interface-method carve-out: a method whose owning type has a resolved `Implements`
//! edge in `ArchModel.type_edges` is suppressed unless the type is the sole
//! implementer of every interface it declares (see `is_interface_impl_suppressed`).
//! Scope limitation: only explicit `implements` syntax (TypeScript/TSX/JS/Java/Kotlin)
//! is covered. Go structural interface satisfaction (method-set matching) is not
//! captured in `type_edges` v1 — see `arch_model.rs:226-230`; Go methods that implicitly
//! satisfy interfaces remain a potential false positive until that gap is filled.
```

### Story 6: Unit tests

Add to the `#[cfg(test)]` block in `single_call_site_delegation.rs`:

1. `does_not_flag_ts_method_implementing_multi_implementer_interface` — TS class with
   `implements`, interface has one other implementer too → not flagged.
2. `flags_ts_method_implementing_sole_implementer_interface` — TS class is the only
   implementer → still flagged (single-caller + sole-implementer exemption).
3. `does_not_flag_ts_method_implementing_interface_with_one_implementer_two_callers` —
   sole implementer but two callers → not flagged by call-count rule (existing behavior,
   confirm unchanged).
4. `go_method_no_type_edges_behavior_unchanged` — Go method, no `Implements` edges in
   `type_edges` → flagged as before.

### Story 7: Backtest

- `kibitzer check backtest single-call-site-delegation` — transcript backtest per
  `CLAUDE.md`.
- `kibitzer run ~/code/github.com/microsoft/vscode` — TS real-world corpus; count
  findings before and after; confirm suppressed methods look like genuine interface
  implementations, not arbitrary wrappers.
- `kibitzer run ~/code/github.com/kubernetes/kubernetes` — Go corpus; confirm no
  regression (same finding count before and after).

## Open Questions Resolved

- **Conservative vs. nuanced**: nuanced rule (Story 3 `is_interface_impl_suppressed`
  with sole-implementer exemption).
- **Helper location**: private to checker (revisit if a second consumer appears).

## Estimate

| Story | Estimate |
|---|---|
| Story 1: indexes | 30m |
| Story 2: threading indexes through findings fn | 30m |
| Story 3: `is_interface_impl_suppressed` | 1h |
| Story 4: imports | 5m |
| Story 5: doc comment | 15m |
| Story 6: unit tests | 2h |
| Story 7: backtest | 1h |
| **Total** | **~5.5h** |
