# ADR-001: Hard-code the stdlib callback allowlist in primitive_obsession.rs

**Status**: Accepted
**Date**: 2026-10-10

## Context

`go-primitive-obsession` fires on `sort.Slice(s, func(i, j int) bool { ... })` because the
closure's `(i, j int)` parameter list contains two identifiers of the same primitive type.
The developer cannot change these names without breaking the stdlib contract — the closure
signature is mandated by the calling function.

We need a way to recognize that a closure is in a stdlib callback position and suppress the
finding for it.

## Decision

Hard-code a `const STDLIB_CALLBACK_ALLOWLIST: &[&str]` in `src/checkers/primitive_obsession.rs`
containing the field names of the stdlib and `slices` package functions whose callbacks carry
mandated parameter signatures. Walk the parent chain from the `parameter_list` node upward
through `func_literal → argument_list → call_expression`, then check the call's `function`
field selector against this list.

The list is a `&[&str]` of field names (not fully-qualified names), so any package naming
its function `Slice` is treated the same as `sort.Slice`. This is an accepted false-negative
boundary: user-defined wrappers with the same names as stdlib callbacks will not fire. The
comment in the code documents this.

Initial list: `Slice`, `SliceStable`, `Search`, `SortFunc`, `SortStableFunc`,
`BinarySearchFunc`, `IndexFunc`, `ContainsFunc`, `EqualFunc`, `CompareFunc`.

## Alternatives considered

**A. kibitzer:ignore directive per call site** — rejected. Requires developers to annotate
every `sort.Slice` in their codebase. That is the exact problem this feature is supposed to
fix; using a workaround as the solution is not acceptable.

**B. Config file / `.kibitzer/inspect.json` extension point** — rejected. No evidence of
demand for user-extension at this stage; the stdlib function set is stable and small.
Adding a config mechanism now front-loads complexity without evidence it is needed.

**C. Fully-qualified name matching (e.g. `sort.Slice`)** — rejected. Requires resolving
the package import alias, which is beyond the scope of this checker's AST-only analysis.
Field-name matching is consistent with `is_run_subtest_closure` in `complexity.rs`, which
also checks only the field name (`.Run`) without resolving the receiver type.

## Consequences

- Any user-defined function named `Slice`, `SortFunc`, etc. that takes a closure with
  same-type primitive parameters will be silently exempted. This is documented in the code.
- Adding a new stdlib function to the exemption list requires only a string added to the
  constant — no schema changes, no config migration.
