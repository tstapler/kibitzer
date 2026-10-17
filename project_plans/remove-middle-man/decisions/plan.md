# Plan: Remove Middle Man checker

## Summary

Implement Fowler's **Remove Middle Man** as a new `ArchModelChecker` by:
1. Extending `SymbolNode` with `delegation_field: Option<String>` to record which
   field a pure-delegation method routes through.
2. Populating that field in `symbol_extract.rs` for Go and TS/JS.
3. Implementing `MiddleManChecker` in `src/architecture_checks.rs` (or a new
   `src/middle_man.rs`) that groups methods by type, computes per-field delegation
   fraction, and flags types above the 80%/3-method threshold.

## Pattern Decisions

| Decision | Choice | Rationale |
|---|---|---|
| Where to add checker | `src/architecture_checks.rs` or new `src/middle_man.rs` | Prefer new file (precedent: `src/single_call_site_delegation.rs`, `src/god_class.rs`) to keep `architecture_checks.rs` from growing further. Registers in `model_registry()` via `crate::middle_man::MiddleManChecker`. |
| `delegation_field` placement | `SymbolNode` field with `#[serde(default)]` | Same additive-field pattern as `is_pure_delegation`. Alternative (separate `HashMap` in `ArchModel`) adds indirection for no benefit. |
| Language scope | Go + TS/JS only | Matches `call_graph_supports` gate that `is_pure_delegation` already enforces; avoids false `None` silently being treated as a signal. |
| AST extraction approach | New `delegation_field_from_body` function alongside `is_pure_delegation_body` | Called only when `is_pure_delegation` is true; avoids re-parsing the body. |
| Threshold | `const MIDDLE_MAN_MIN_METHODS: usize = 3`, `const MIDDLE_MAN_DELEGATION_RATIO: f64 = 0.8` | Matches backlog proposal. Named constants mirroring `LCOM_MIN_METHODS`/`STABLE_MAX`. |
| Severity | `Advisory` | Same as `LcomChecker` — the finding is actionable but requires human judgment (Adapter/Decorator patterns are legitimate). |

## Implementation steps

### Step 1: Extend `SymbolNode` and `symbol_extract.rs`

**`src/arch_model.rs`**: Add field to `SymbolNode`:
```rust
/// Set when `is_pure_delegation` is `true` and the delegation call routes through a
/// named field on the receiver (e.g. `s.inner.Foo()` → `Some("inner")`). `None` when
/// the call goes through a free function, a direct method call on `self`/receiver, or
/// for languages `call_graph_supports` doesn't cover.
#[serde(default)]
pub delegation_field: Option<String>,
```

**`src/symbol_extract.rs`**: Add `delegation_field_from_body(body: Node, language: Language) -> Option<String>`:
- Returns `None` immediately for non-Go/non-TS/non-JS languages.
- For Go: look for `call_expression → function: selector_expression → operand: selector_expression → field: field_identifier`. Return the inner field identifier's text.
- For TS/JS: same pattern with `member_expression → object: member_expression → property: property_identifier`.
- Guard: the inner operand must be a bare `identifier` (Go) / `this` (TS/JS), not another call — this distinguishes `s.inner.Foo()` from `s.GetInner().Foo()`.

Update the `Some(SymbolNode {...})` construction in `classify_node` to include:
```rust
delegation_field: if is_pure_delegation {
    delegation_field_from_body(node.child_by_field_name("body")?, language)
} else {
    None
},
```

All existing `SymbolNode { ..., is_pure_delegation: false }` construction sites in
`arch_model.rs` must add `delegation_field: None` (compiler enforces exhaustiveness).

### Step 2: Implement `MiddleManChecker`

**`src/middle_man.rs`** (new file):
```rust
const MIDDLE_MAN_MIN_METHODS: usize = 3;
const MIDDLE_MAN_DELEGATION_RATIO: f64 = 0.8;

pub struct MiddleManChecker;

impl ArchModelChecker for MiddleManChecker {
    fn name(&self) -> &str { "remove-middle-man" }

    fn check(&self, model: &ArchModel, _config: &ArchitectureConfig) -> Vec<ArchFinding> {
        model.packages.values()
            .flat_map(|pkg| middle_man_findings_for_package(pkg))
            .collect()
    }
}
```

`middle_man_findings_for_package`:
1. Call `methods_by_type(pkg)` to group method `SymbolNode::id`s by type name.
2. For each type, look up the actual `SymbolNode` objects from `pkg.symbols`.
3. Count total methods and methods per `delegation_field` value (ignoring `None`).
4. Find the field with the highest count. If count/total >= `MIDDLE_MAN_DELEGATION_RATIO`
   and total >= `MIDDLE_MAN_MIN_METHODS`, emit a finding.

Finding message:
```
[remove-middle-man] pkg::TypeName delegates N/M methods through field 'inner' — consider
removing this middle man and letting callers talk to 'inner' directly
(Fowler, Remove Middle Man)
```

### Step 3: Register in `model_registry()`

**`src/architecture_checks.rs`**:
```rust
pub fn model_registry() -> Vec<Box<dyn ArchModelChecker>> {
    vec![
        // ... existing entries ...
        Box::new(crate::middle_man::MiddleManChecker),
    ]
}
```

Add `pub mod middle_man;` to `src/main.rs` (or wherever the module tree is declared).

### Step 4: Tests

In `src/middle_man.rs` `#[cfg(test)]` module, using `model_from()` (same helper as
`single_call_site_delegation`'s tests, via `arch_model::build_model`):

- `flags_go_type_with_majority_same_field_delegation`: 4 methods, 3 delegating through `inner` → flagged.
- `does_not_flag_type_with_mixed_delegation_targets`: 4 methods delegating to 2 different fields → not flagged.
- `does_not_flag_type_below_min_method_threshold`: 2 methods, both delegating through `inner` → not flagged.
- `does_not_flag_type_at_or_below_ratio_threshold`: 5 methods, 3 delegating through `inner` (60%) → not flagged.
- `flags_typescript_type_with_majority_delegation`: TS version of the positive case.
- `does_not_flag_non_delegating_methods_through_none`: Methods with `delegation_field: None` don't count toward same-field delegation.

Also add `symbol_extract` tests for `delegation_field` population:
- `delegation_field_set_for_go_field_delegation`
- `delegation_field_none_for_go_direct_method_call` (no intermediate field)
- `delegation_field_set_for_ts_field_delegation`

### Step 5: Backtesting

Per `CLAUDE.md`'s "Writing a new check" section:
1. `kibitzer check backtest remove-middle-man` against session transcripts.
2. Run against all repos in `docs/backtest-repos.md`.
3. Record findings in `project_plans/remove-middle-man/validation.md`.

## Adversarial review

**Pre-mortem risks:**

1. **`selector_expression` depth ambiguity in Go**: `a.b.c.d()` chains to depth 3+.
   The extraction must only capture the *immediately enclosing* field (one level below
   the call's function selector), not the deepest. This is correct behavior — we want
   to know which field the delegation routes through, not the full chain.

2. **TS `member_expression` vs. `selector_expression` naming**: Go calls the node
   `selector_expression`; TS/JS calls it `member_expression`. The extraction function
   must branch correctly. Risk: `delegation_field_from_body` uses Go-specific node kind
   for TS code → always returns `None` → no false positives but also no true positives
   for TS. Mitigation: add `flags_typescript_type_with_majority_delegation` test to
   CI — it will fail if the TS branch is broken.

3. **`methods_by_type` returns ids, not nodes**: The LCOM checker uses ids to build
   call/field graphs; the Middle Man checker needs the actual `delegation_field` field
   from the `SymbolNode`. Must resolve ids back to `SymbolNode`s via `pkg.symbols`. The
   lookup is O(N) per method; for typical type sizes (< 50 methods) this is fine.

4. **Cached `ArchModel` compatibility**: Existing `cache.json` files won't have
   `delegation_field` in their `SymbolNode` JSON. `#[serde(default)]` makes this
   backward-compatible (deserializes to `None`), which is safe: the first re-run after
   the update will rebuild the cache. No migration needed.

5. **`model_registry()` needs `needs_private_symbols`**: The default is `false` (exported
   only). For Remove Middle Man, both exported and unexported methods matter for counting
   the ratio. Override `needs_private_symbols()` to `true` — same as
   `SingleCallSiteDelegationChecker`. Without this, `build_arch_model_for_check` prunes
   unexported methods before the checker sees them, breaking the ratio computation.

6. **Go pointer receiver duplication**: `func (s *S) Foo()` and `func (s S) Bar()` both
   have `parent = Some("S")`. `methods_by_type` already groups both under `"S"`, so this
   is handled correctly.
