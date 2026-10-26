# Requirements: Native check — Remove Middle Man (delegation-ratio-per-type)

item_id: 6c24cbab-0421-4b6c-87dc-26a1a8732c2b

## Background

The inverse of Hide Delegate: a type whose methods are almost all pure pass-through
to a single field (`func (s *S) Foo() { return s.inner.Foo() }` repeated across most
of its methods) adds an indirection layer without adding behavior. Fowler calls this
**Remove Middle Man** (*Refactoring*, 2nd ed., Chapter 7, p.193): the caller should
just talk to the underlying object directly.

`docs/refactoring-catalog-analysis.md` classifies this as issue
[#53](https://github.com/tstapler/kibitzer/issues/53) with verdict 🧩 (new capability
needed), noting "No hard capability dependency" — meaning it extends `ArchModel`
directly rather than requiring the call-graph or field-tracking capabilities.

## Proposal

A new `ArchModelChecker` registered in `architecture_checks::model_registry()`: for
each type, compute the fraction of its methods whose body is a single delegating call
routed through the *same* field on the receiver, and flag types where that fraction
exceeds a threshold (80%) and the type has 3+ methods.

## Key capability gap

`SymbolNode::is_pure_delegation` already marks methods whose body is a single
delegating call, but does **not** record *which field* that delegation routes through.
To answer "do most methods delegate to the same field?", `symbol_extract.rs` must
be extended to extract and record the delegation receiver field name when
`is_pure_delegation` is true.

The AST pattern in Go for `func (s *S) Foo() { return s.inner.Foo() }`:
- The call body contains one `call_expression`
- The `function` field of that call is a `selector_expression`
- The `operand` of that `selector_expression` is another `selector_expression`
  (`s.inner`) — the field access
- The field name is the `field` child of the outer selector's operand

The same structural pattern applies in TS/JS (`this.inner.foo()`).

## Scope notes

- Exclude types with fewer than 3 methods (intentional Adapter/Decorator wrappers
  are typically small; a 2-method type that purely delegates is ambiguous).
- Threshold: ≥80% of methods delegating to the same field. Configurable as a named
  `const` mirroring `LCOM_MIN_METHODS`/`MAX_FAN_OUT`.
- Language scope: Go and TS/JS, matching `call_graph_supports` coverage (same
  restriction `is_pure_delegation` already applies).
- Exported types are candidates (unlike `single_call_site_delegation`, which only
  targets unexported symbols) — an exported type can still be a middle man.

## Repo conventions this check must follow

- Registers in `model_registry()` (`architecture_checks.rs`) like every other
  `ArchModelChecker` — not in `default_checks()`, which is for per-file native
  checkers via `Checker`, not whole-repo `ArchModelChecker`s.
- `[remove-middle-man]` bracket prefix on finding messages, consistent with
  `[lcom]`, `[instability]` etc.
- Must be backtested per `CLAUDE.md`'s "Writing a new check" section:
  - `kibitzer check backtest remove-middle-man` against session transcripts.
  - Full real-world repo corpus (`docs/backtest-repos.md`).
- No `.kibitzer/inspect.json` required — runs automatically like every other
  `ArchModelChecker` in `model_registry()`.

## Acceptance criteria

1. `SymbolNode` (or a companion structure in `ArchModel`) carries a new optional
   `delegation_field: Option<String>` field capturing the receiver-field name when
   `is_pure_delegation` is `true` and the delegation routes through a named field
   (`None` otherwise — e.g. plain function calls, `self.method()` without an
   intermediate field).

2. `symbol_extract.rs` populates `delegation_field` for Go and TS/JS via AST
   analysis of the delegation call expression, with no change needed for unsupported
   languages (`None` unconditionally, same `call_graph_supports` gate as
   `is_pure_delegation`).

3. A new `MiddleManChecker` implementing `ArchModelChecker` is registered in
   `model_registry()` with name `"remove-middle-man"`.

4. The checker groups each type's methods, filters to those with
   `is_pure_delegation == true && delegation_field == Some(field_name)` for the
   same field name, and flags the type when:
   - total exported+unexported method count ≥ `MIDDLE_MAN_MIN_METHODS` (3), and
   - the fraction of methods delegating to a single shared field ≥
     `MIDDLE_MAN_DELEGATION_RATIO` (0.8).

5. Both thresholds are named `const`s in the source (mirroring `LCOM_MIN_METHODS`,
   `STABLE_MAX`), not magic numbers.

6. Finding message follows the `[remove-middle-man] pkg::Type delegates N/M methods
   through field 'inner' — consider removing this middle man and letting callers talk
   to 'inner' directly (Fowler, Remove Middle Man)` format.

7. Unit tests in `architecture_checks.rs` (or a new sibling test module) cover:
   - A type with ≥80% of 3+ methods delegating through the same field → flagged.
   - A type with mixed delegation (multiple fields) → not flagged.
   - A type with fewer than 3 methods → not flagged even if all delegate to same field.
   - A type with ≥3 methods but delegation ratio below threshold → not flagged.
   - A non-Go/non-TS file → `delegation_field` is `None`, type not flagged.

8. The check is backtested against session transcripts and the public repo corpus
   before being marked done; false-positive rate recorded in
   `project_plans/remove-middle-man/validation.md`.

9. `#[serde(default)]` on `delegation_field` in `SymbolNode` so an `ArchModel`
   cached before this field existed still deserializes without error (same additive-
   field precedent as `ArchFinding::severity_override`, `SymbolNode::is_pure_delegation`).

## Non-goals

- No configuration via `.kibitzer/inspect.json` in v1 — the thresholds are global
  `const`s, per-project overrides can follow later if defaults prove noisy.
- No Rust/Python/Java/Kotlin support in v1 — `is_pure_delegation` is gated on
  `call_graph_supports` and this feature extends that same set.
- No distinction between exported and unexported methods in the ratio computation —
  all methods count toward the threshold.
