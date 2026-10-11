# Validation Plan: long-function-flat-literal

**Date**: 2026-10-11

## Happy Path Scenario

Given a Go file with a function `func cases() []tc { return []tc{ {name: "a", input: 1, want: 2}, ... } }`
spanning 49 lines (47 entries), when `kibitzer check native syntax-rules <file>` runs, then
no `[long-function]` finding is emitted for that function.

## Requirement → Test Mapping

| Requirement | Test Name | Type | Scenario |
|-------------|-----------|------|----------|
| AC1: flat Go composite literal exempted | `long_function_exempts_flat_go_composite_literal` | Unit, Happy path | Go source: `func cases() []tc { return []tc{ ... } }` with 45 entries (no calls, no control flow). Assert zero `[long-function]` findings. |
| AC2: literal with call expression still flagged | `long_function_does_not_exempt_literal_with_call` | Unit, Error path | Same function but one entry: `{name: fmt.Sprintf("case %d", i), input: 1, want: 2}`. Assert `[long-function]` IS emitted. Guards `has_logic_descendants`'s call-kinds check. |
| AC3: literal with `if` still flagged | `long_function_does_not_exempt_literal_with_if` | Unit, Error path | Same shape but one entry's value is `func() int { if x > 0 { return 1 }; return 0 }()`. Assert `[long-function]` IS emitted. (Alternatively: a `composite_literal` that cannot syntactically contain an `if` — use an `if_statement` check on the containing function instead.) |
| AC4: multi-statement body not exempted | `long_function_does_not_exempt_multi_statement_body` | Unit, Error path | Go function: `var x []tc = nil; return x` (two statements), with enough added lines to exceed 40. Assert `[long-function]` IS emitted. |
| AC5: TypeScript flat array exempted | `ts_long_function_exempts_flat_array_literal` | Unit, Happy path | TypeScript: `function cases(): TC[] { return [{name: "a", input: 1}, ...] }` with 45 entries. Assert no `[long-function]`. |
| AC6: Python flat list exempted | `py_long_function_exempts_flat_list_literal` | Unit, Happy path | Python: `def cases() -> list[TC]: return [TC("a", 1), ...]` with 45 entries (no calls — TC is a dataclass/NamedTuple constructor, which IS a call; verify whether Python test cases are feasible without calls, or adjust to use tuple literals `("a", 1)` instead). |
| AC6 (existing regression) | `flags_long_function` (existing, unchanged) | Unit, Regression | Existing test that builds a Go function with 45 `println(...)` calls. Must still emit `[long-function]`. |
| AC7: `docs/syntax-rules.md` updated | Manual doc review | Docs | Confirm `long-function` row mentions the flat-literal exemption. |
| AC8: backtest run, result noted | `kibitzer check backtest syntax-rules` | Process | See Epic 3. Result in PR description. |
| AC9: corpus sample confirms noise | `syntax-rules` on `kubernetes/kubernetes` test packages | Process | See Epic 3. Any newly-silenced findings are flat fixture helpers. |

## Notes on AC6 (Python)

Python's `list` literal `[TC("a", 1), ...]` contains `call` nodes (the `TC(...)` constructor
calls). The predicate would correctly NOT exempt this — it contains logic. The Python happy
path test must use a shape without calls, e.g. `return [("a", 1, 2), ...]` (tuples of raw
literals). If Python's `tuple` node kind is verified as a `collection_literal_kinds` entry
(Task 1.1.1f grammar check), the test works. Otherwise, Python's `list` of tuples-of-literals
also works since the inner tuples pass `has_logic_descendants`.

Alternatively, the Python test can use a dictionary: `return {"a": 1, "b": 2, ...}` where
every value is a plain literal — verifiable with `to_sexp()`.

## Grammar verification requirement

Before writing any test, verify with `to_sexp()` or the tree-sitter playground that:
- Go `return []tc{{...}}` → the return's named child is `composite_literal` ✓ (already
  verified: `density_boundary_kinds` lists it, and the filed item's reproducer confirms
  the finding fires at the composite literal's line)
- TypeScript `return [{...}]` → the return's named child is `array` (verify via `to_sexp`)
- Python `return [("a", 1), ...]` → the return's named child is `list` (verify via `to_sexp`)
- Python `for i in ...` inside a list comprehension → `for_in_clause` kind (verify; if not
  in `nesting_kinds`, add Python-specific comprehension test and handle accordingly)
