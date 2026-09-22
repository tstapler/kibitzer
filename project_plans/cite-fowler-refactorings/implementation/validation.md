# Validation Plan: cite-fowler-refactorings

**Date**: 2026-09-22

## Happy Path Scenario
Given a Go function body that exceeds `LONG_FUNCTION_LINES` (40 lines), when `check_source`
runs `check_declaration` over it, then the resulting `[long-function]` `Finding.message`
contains both "Extract Function" and
`https://refactoring.com/catalog/extractFunction.html` — telling the user which Fowler
refactoring to apply without them already knowing the catalog.

## Requirement → Test Mapping

| Requirement | Test File | Test Name | Type | Scenario |
|-------------|-----------|-----------|------|----------|
| AC#1 — `long-function` message names "Extract Function" and links the catalog URL | `src/rules.rs` (`mod tests`) | `long_function_message_cites_extract_function` | Unit (happy path) | 45-line Go function body (reuses `flags_long_function`'s fixture, `src/rules.rs:1230-1242`) → `Finding.message` contains `"Extract Function"` and `"https://refactoring.com/catalog/extractFunction.html"` |
| AC#1 — message does *not* regress when body is under the threshold | `src/rules.rs` (`mod tests`) | `allows_short_function` (existing, unmodified) | Unit (error/negative path) | Short function body → `findings.is_empty()`, so no citation text can appear — confirms the citation addition didn't loosen the trigger condition |
| AC#2 — `deep-nesting` message names "Replace Nested Conditional with Guard Clauses" and links the catalog URL | `src/rules.rs` (`mod tests`) | `deep_nesting_message_cites_guard_clauses` | Unit (happy path) | 5-level-nested Go function body (reuses `flags_deep_nesting`'s fixture, `src/rules.rs:1257-1278`) → `Finding.message` contains `"Guard Clauses"` and `"https://refactoring.com/catalog/replaceNestedConditionalWithGuardClauses.html"` |
| AC#2 — message does not regress for shallow nesting | `src/rules.rs` (`mod tests`) | `allows_shallow_nesting` (existing, unmodified) | Unit (error/negative path) | Shallowly nested function → no `[deep-nesting]` finding at all, so no citation text can leak in incorrectly |
| AC#3 — both catalog URLs return 2xx | N/A (manual/shell, not a `cargo test`) | `curl -sI` against both URLs (Task 1.2.1a) | Integration (external call) | `curl -sI https://refactoring.com/catalog/extractFunction.html` and `curl -sI https://refactoring.com/catalog/replaceNestedConditionalWithGuardClauses.html` — status line is `HTTP/1.1 2xx`, run once immediately before opening the PR per `research/build-vs-buy.md` §2 (no automated linkcheck; `markdown-link-integrity` deliberately treats `http(s)://` targets as always-valid, `src/markdown_link_integrity.rs:357`) |
| AC#4 — `LONG_FUNCTION_LINES`, `MAX_NESTING_DEPTH`, trigger conditions, `Finding.line`/severity stay byte-identical | N/A (diff inspection, not a runtime test) | `git diff src/rules.rs` reviewed by hand/PR reviewer | N/A — structural invariant, not testable via `cargo test` | Diff touches only the string-literal contents inside the two `format!(...)` calls (`src/rules.rs:826-828`, `:836-838`); constants at `:11`/`:15` and the `Finding { line, .. }` construction are unchanged |
| AC#5 — all 29 existing test call sites pass unmodified | `src/rules.rs` (`mod tests`, all existing tests) | `cargo test rules::` (full existing suite, no edits to assertions) | Unit regression (happy path, run as a batch) | Existing tests assert `.message.contains("[long-function]")` / `.contains("[deep-nesting]")` — substring match on the bracketed prefix, unaffected by appended trailing text |
| AC#6 — two new unit tests exist, compile, and pass | `src/rules.rs` (`mod tests`) | `long_function_message_cites_extract_function`, `deep_nesting_message_cites_guard_clauses` | Unit (happy path) | `cargo test rules::long_function_message_cites_extract_function rules::deep_nesting_message_cites_guard_clauses` — both exist and pass |
| AC#7 (committed in scope, see plan.md Story 1.1.3) — `CATALOG` descriptions for `long-function`/`deep-nesting` name the refactoring, matching the `flag-argument`/`unreachable-code` style | N/A — no dedicated test; `CATALOG` is `#[allow(dead_code)]` self-documentation, not machine-read (per plan.md's Pattern Decisions table) | N/A | N/A | Verified by reading `CATALOG[0].description` / `CATALOG[1].description` (`src/rules.rs:38`, `:44`) against the pattern at `:56`/`:62` — a compile-time string literal, not exercised by any runtime path |

## UX Acceptance Tests
N/A — no user-facing UI surface. This change edits two `Finding.message` string literals and
(optionally) two `RuleMeta.description` string literals consumed as CLI/JSON output text; there
is no `design/ux.md` for this project and none is needed at this scope.

## Test Stack
- **Unit**: cargo test's built-in `#[test]` + `assert!`/`assert_eq!`, matching src/rules.rs's existing `mod tests` convention. New tests follow the existing snake_case naming style seen in `flags_long_function`, `flags_deep_nesting`, `allows_shallow_nesting` — no camelCase-derived `methodName_should_X_When_Y` scheme is used, since it doesn't match this codebase's convention.
- **Integration**: N/A — no data store, no internal service call. The one external call (catalog URL liveness) is verified manually with `curl -sI`, not via an automated integration test (see AC#3 row and `research/build-vs-buy.md` §2 for why: this repo deliberately excludes `http(s)://` targets from its own `markdown-link-integrity` check to avoid flaky, network-dependent CI).
- **E2E / UX**: N/A — no user-facing UI surface.

## Coverage Targets and How to Measure

| Stack | Coverage command | Target |
|---|---|---|
| Rust | `cargo test rules::` | All 29 existing + 2 new = 31 tests in `rules::tests` pass |
| Rust (full gate, per plan.md Task 1.2.1a) | `cargo test && cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings` | All three exit 0 |
| External URLs (manual, not `cargo test`) | `curl -sI <url>` for both catalog URLs, re-run immediately before shipping | Both status lines are `HTTP/1.1 2xx` |
