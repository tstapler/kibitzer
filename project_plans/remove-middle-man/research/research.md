# Research: Remove Middle Man checker

## Literature

**Fowler, M., *Refactoring*, 2nd ed., Ch.7 "Encapsulation", p.192-193 (Remove Middle Man)**
The smell: a class has so many delegation methods that it's just passing everything
through to another object. The refactoring: instead of calling `department.manager`,
callers should call `person.department.manager` directly, removing the `Person`
class from the middle. The countervailing consideration: sometimes hiding the
delegate is worthwhile (Law of Demeter) — Remove Middle Man is the correction when
Hide Delegate is overdone.

**Lieberherr, K., Holland, I. & Riel, A., "Object-Oriented Programming: An Objective
Sense of Style," OOPSLA 1988 (Law of Demeter)**
The originating principle for both Hide Delegate and Remove Middle Man. A caller
should only talk to its "immediate friends" — but a class that exists only to relay
messages breaks the spirit of the law without providing its benefit.

**Sharma, T. & Spinellis, D., "A Survey on Software Smells," Journal of Systems and
Software, 2018**
Notes that delegation-ratio smells are mechanically detectable but rarely implemented
in commercial tools. The structural signal (fraction of methods that are pure
pass-throughs to one field) is clear and has low ambiguity compared to semantic
smells like Feature Envy.

## Codebase findings

### Existing infrastructure that directly applies

1. **`SymbolNode::is_pure_delegation`** (`src/arch_model.rs:88`,
   `src/symbol_extract.rs:524-528`)
   Already marks any Go/TS/JS function/method whose body is exactly one call
   statement or `return <call>`. The `is_pure_delegation_body` function (`symbol_extract.rs:583`)
   already parses the relevant AST; it just needs a sibling extraction to also record
   which field the call routes through.

2. **`methods_by_type()`** (`src/architecture_checks.rs:727`)
   Already groups a package's `Method` symbols by owning type name — the `LcomChecker`
   uses this to build LCOM4 components. The new checker can reuse this grouping directly.

3. **`ArchModelChecker` trait + `model_registry()`** (`src/architecture_checks.rs:505-531`)
   The registration point for `InstabilityChecker`, `LcomChecker`, `GodClassChecker`,
   `SingleCallSiteDelegationChecker`, etc. The new checker slots in here.

4. **`is_pure_delegation_body` AST analysis** (`src/symbol_extract.rs:583-608`)
   The current implementation returns `bool`. It already identifies the single
   `call_expression` node. To extract the delegation field, we need to inspect the
   call's `function` field child: for `s.inner.Foo()`, the `function` is a
   `selector_expression` (the `Foo` selector) whose `operand` is another
   `selector_expression` (`s.inner`). The field name is the `field` child of the
   inner `selector_expression`.

### AST shape (verified from `symbol_extract.rs` comments and tree-sitter grammar notes)

**Go** (`s.inner.Foo()`):
```
call_expression
  function: selector_expression
    operand: selector_expression   ← this is "s.inner"
      operand: identifier           ← "s" (receiver)
      field: field_identifier       ← "inner"  ← this is the delegation field
    field: field_identifier         ← "Foo"
  arguments: argument_list
```

**TypeScript** (`this.inner.foo()`):
```
call_expression
  function: member_expression
    object: member_expression      ← "this.inner"
      object: this                  ← "this" (receiver)
      property: property_identifier ← "inner"  ← this is the delegation field
    property: property_identifier   ← "foo"
```

The key difference: Go uses `selector_expression` + `operand`/`field`, while
TS/JS uses `member_expression` + `object`/`property`. Both have a two-level
chain: outer call → outer selector → inner selector/member containing the
field name.

### False-positive risks

1. **Intentional Adapter pattern**: A type might deliberately wrap another object
   with the same API (e.g., adding instrumentation, a null-safety layer, thread
   safety). The 80% threshold and 3-method minimum reduce but don't eliminate this.
   Advisory-severity is appropriate.

2. **Interface satisfaction**: An exported type implementing an interface may need
   to delegate all methods to a contained implementation — this is a common Go
   embedding substitute. Consider whether types that also own non-delegation methods
   should be treated differently.

3. **Method receiver type vs. pointer vs. value**: In Go, methods on `*S` and `S`
   are grouped as one type by `methods_by_type()` since both use the type name as
   `parent`. The checker should count all methods on a named type regardless of
   pointer/value receiver distinction.

4. **Empty `delegation_field`**: Methods that call a free function (not through a
   field) still have `is_pure_delegation = true` but `delegation_field = None`.
   The checker must exclude these from the "same field" computation.

### Comparable checkers as design reference

- `LcomChecker`: reuses `methods_by_type()`, iterates per-package, filters by
  minimum method count, computes a ratio-based metric, flags above threshold.
  The new checker follows an identical outer loop structure.
- `SingleCallSiteDelegationChecker`: uses `is_pure_delegation` on individual
  symbols. The new checker aggregates it to the type level.
- Backtesting requirement (`CLAUDE.md`): same as `LcomChecker` and every other
  `ArchModelChecker` — `kibitzer check backtest remove-middle-man` and
  real-world repo corpus run required before shipping.

## Open questions

1. **TS `member_expression` vs. `selector_expression`**: Go uses `selector_expression`
   (verified in `symbol_extract.rs`'s comments); TS uses `member_expression`
   (standard in the tree-sitter-typescript grammar). The extraction code must
   branch on language, or abstract over the two grammar differences. The existing
   `call_graph_supports`/`LangSymbolConfig` pattern provides the branching model.

2. **Delegation to method vs. field**: `s.inner.Foo()` goes through a *field* `inner`.
   `s.GetInner().Foo()` goes through a *method call* `GetInner()`. The proposal
   only targets field-based delegation (the classic Middle Man smell). Method-based
   delegation would require recognizing call chains, which is out of scope here.
   The AST check: the inner `selector_expression`'s operand must be an identifier
   (the receiver), not a call expression, to be a field-access chain.

3. **Multi-field types**: If a type has 5 methods, 4 delegating through `inner` and
   1 doing real work, it should still be flagged (80% threshold). The checker should
   compute the maximum fraction across all fields, not only fire when *all* methods
   delegate to the same field.
