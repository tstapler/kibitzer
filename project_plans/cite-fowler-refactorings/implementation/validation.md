# Validation Plan: cite-fowler-refactorings

**Date**: 2026-09-22

## Happy Path Scenario
Given a Go function whose body exceeds 40 lines and nests conditionals more than 4 levels deep, when `kibitzer` runs the `long-function`/`deep-nesting` checks against it, then the resulting finding messages name the corresponding Fowler *Refactoring* catalog entry ("Extract Function" / "Replace Nested Conditional with Guard Clauses") and link its `refactoring.com/catalog/...` URL, while thresholds, trigger conditions, and `line`/severity stay exactly as before.

## Requirement → Test Mapping

| Requirement | Test File | Test Name | Type | Scenario |
|-------------|-----------|-----------|------|----------|
| AC-1: `long-function` message names "Extract Function" and links `extractFunction.html` | src/rules.rs (`mod tests`, alongside `flags_long_function` at line 1230) | `long_function_message_cites_extract_function` | Unit | Reuse the 45-line-body Go source from `flags_long_function`; assert the matched finding's `message` contains both `"Extract Function"` and `"https://refactoring.com/catalog/extractFunction.html"`. |
| AC-2: `deep-nesting` message names "Replace Nested Conditional with Guard Clauses" and links `replaceNestedConditionalWithGuardClauses.html` | src/rules.rs (`mod tests`, alongside `flags_deep_nesting` at line 1258) | `deep_nesting_message_cites_guard_clauses` | Unit | Reuse the 5-level-nested Go source from `flags_deep_nesting`; assert the matched finding's `message` contains both `"Replace Nested Conditional with Guard Clauses"` and `"https://refactoring.com/catalog/replaceNestedConditionalWithGuardClauses.html"`. |
| AC-3: both URLs return a successful (non-404/non-error) response | N/A (manual, not automated) | N/A | Manual | `curl -sI https://refactoring.com/catalog/extractFunction.html` and `curl -sI https://refactoring.com/catalog/replaceNestedConditionalWithGuardClauses.html`, run immediately before ship (plan Story 1.1.4); confirm `HTTP/2 200` (or other 2xx/3xx) on both. Not an automated test — a live network dependency in `cargo test` is undesirable and the content is static, so a point-in-time manual check per the plan is sufficient. |
| AC-4: no detection-logic changes — thresholds, trigger conditions, finding `line`/severity unchanged | src/rules.rs (`mod tests`, existing) | *(no new test — covered by existing tests)* | Unit (existing) | `allows_short_function`, `allows_shallow_nesting` (negative/boundary cases) and every existing positive case (`flags_long_function`, `flags_deep_nesting`, `rules_fire_independently_on_one_function`, etc.) already assert on trigger conditions and firing behavior around `LONG_FUNCTION_LINES`/`MAX_NESTING_DEPTH`. Since the plan only appends text inside the two `format!` strings (`src/rules.rs:826-828`, `:836-838`) and does not touch the constants or the `if` conditions that gate them, these pre-existing tests re-passing unchanged is the evidence for AC-4 — no new test adds coverage here. |
| AC-5: all 29 existing `.contains("[long-function]")` / `.contains("[deep-nesting]")` call sites still pass | N/A (verification step, not a new test) | N/A | Unit (existing, run as a gate) | `cargo test rules::` (plan Story 1.1.3). A substring `.contains(...)` on the rule-id prefix (`"[long-function]"`, `"[deep-nesting]"`) is unaffected by appending more text after it, so this is a regression gate on the existing suite rather than something a new test could cover better. |
| AC-6 (optional, open question): `CATALOG` descriptions for `long-function`/`deep-nesting` get the same treatment | src/rules.rs (`mod tests`) | *(no test required — out of scope unless implemented)* | N/A | `CATALOG` (`src/rules.rs:34-46`) is documentation metadata (`#[allow(dead_code)]`, not read by checker logic — see doc comment at `src/rules.rs:31-32`) with no existing test asserting on its `description` strings. If Epic 1.2 is implemented, a one-line manual read-back of `CATALOG[0].description`/`CATALOG[1].description` is sufficient; if skipped, no coverage gap since AC-6 is explicitly an open question, not a firm requirement. |

## UX Acceptance Tests
N/A — no user-facing surface (CLI/library finding-message change).

## Test Stack
- **Unit**: cargo test (Rust built-in test framework)
- **Integration**: N/A
- **E2E / UX**: N/A

## Coverage Targets and How to Measure
| Stack | Coverage command | Target |
|---|---|---|
| Rust | `cargo test rules::` | All existing + 2 new assertions pass |

- All public service methods: N/A for this change
- All external integrations: the two refactoring.com URLs — verified via manual `curl -sI` per AC #3, not an automated test (network dependency in CI is undesirable for a static-content link check)
- UX acceptance criteria: N/A
