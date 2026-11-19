# Validation Plan: feature-envy-detection

**Date**: 2026-11-03

## Happy Path Scenario

Given a Go method with 5 foreign field accesses to `pkg::Customer` and 1 local field access, when `FeatureEnvyChecker.check(model, config)` runs, then it emits a single `[feature-envy:move-function]` finding naming `pkg::Customer` as the dominant foreign type.

---

## Requirement → Test Mapping

| Requirement | Test File | Test Name | Type | Scenario |
|-------------|-----------|-----------|------|----------|
| AC-1: `feature-envy` registered in `model_registry()` | `src/feature_envy.rs` | `test_feature_envy_checker_registered_in_model_registry` | Unit | `model_registry()` returns a slice containing an entry whose `name()` == `"feature-envy"` |
| AC-1: no duplicate registration | `src/feature_envy.rs` | `test_feature_envy_checker_registration_not_duplicated` | Unit | Filter `model_registry()` by name `"feature-envy"`; count == 1 |
| AC-2: flags when foreign_ratio >= threshold and foreign_count >= minimum | `src/feature_envy.rs` | `test_foreign_ratio_flags_when_exceeds_threshold_and_minimum` | Unit | Method with 5 foreign field accesses to `pkg::Customer`, 1 local → `foreign_ratio` = 5/6 ≥ 0.33 and foreign ≥ 3 → one finding emitted |
| AC-2: does not flag when foreign_count below minimum | `src/feature_envy.rs` | `test_foreign_ratio_does_not_flag_when_below_min_foreign_count` | Unit | Method with 2 foreign field accesses (ratio = 1.0 but foreign < 3) → no finding |
| AC-3: finding message names dominant foreign type | `src/feature_envy.rs` | `test_finding_message_names_dominant_foreign_type` | Unit | Method accesses `pkg::Customer` 4 times and `pkg::Pricer` 1 time → finding message contains `"pkg::Customer"` |
| AC-3: dominant type is highest-count type among multiple foreign types | `src/feature_envy.rs` | `test_dominant_type_returns_highest_count_type` | Unit | `dominant_type` called with `{"pkg::A": 3, "pkg::B": 5}` → returns `Some("pkg::B")` |
| AC-4: `[feature-envy:move-field]` fires on field-accesses-only method | `src/feature_envy.rs` | `test_move_field_candidate_fires_when_only_field_accesses_dominate` | Unit | Method with 4 foreign field accesses and 0 foreign calls; 1 local → emits `[feature-envy:move-field]` label (not `move-function`) |
| AC-4: `[feature-envy:move-function]` fires when calls included alongside fields | `src/feature_envy.rs` | `test_move_function_candidate_fires_when_calls_included` | Unit | Method with 2 foreign field accesses + 2 foreign call edges to same type (total foreign = 4) and 1 local → emits `[feature-envy:move-function]` label |
| AC-5: test-file methods suppressed | `src/feature_envy.rs` | `test_test_file_method_is_suppressed` | Unit | Method whose `SymbolNode.file` ends with `_test.go` and would otherwise be flagged → no finding |
| AC-5: adapter-type methods suppressed | `src/feature_envy.rs` | `test_adapter_type_method_is_suppressed` | Unit | Type named `OrderAdapter` with an envious method → no finding (adapter suppression) |
| AC-5: low-total methods suppressed by min_total_guard | `src/feature_envy.rs` | `test_low_total_method_suppressed_when_below_min_total` | Unit | Method with 3 foreign + 0 local = total 3 < `FEATURE_ENVY_MIN_TOTAL` (4) → no finding |
| AC-6: flagged Move Function candidate | `src/feature_envy.rs` | `test_move_function_candidate_is_flagged` | Unit | Same as AC-2 happy path: 5 foreign (3 field + 2 call), 1 local → `[feature-envy:move-function]` finding |
| AC-6: flagged Move Field candidate | `src/feature_envy.rs` | `test_move_field_candidate_is_flagged` | Unit | 4 foreign field accesses, 0 foreign calls, 1 local → `[feature-envy:move-field]` finding |
| AC-6: suppressed test-file method (true negative) | `src/feature_envy.rs` | `test_suppressed_test_file_produces_no_findings` | Unit | Envious method in `_test.go` → empty findings vec |
| AC-6: below-threshold true negative | `src/feature_envy.rs` | `test_below_threshold_method_produces_no_findings` | Unit | Method with 1 foreign field access, 5 local → `foreign_ratio` = 1/6 < 0.33 → no finding |
| AC-6 / plan key fix: bare key normalization in per_type_access_map merge | `src/feature_envy.rs` | `test_per_type_access_map_normalizes_bare_keys_to_qualified_names` | Unit | `field_counts` has key `"Customer"` (bare); `call_counts` has key `"pkg::Customer"`; after normalization and merge both map to `"pkg::Customer"` with combined count — dominant type is `"pkg::Customer"`, not split across two keys |
| AC-7: session-transcript backtest completes without error | `scripts/` or manual step | `backtest_session_transcripts_complete_without_error` | Integration | `kibitzer check backtest feature-envy` runs to completion with no panic; TP/FP rates recorded in `decisions/validation.md` |
| AC-7: public repo corpus backtest produces acceptable false-positive rate | `scripts/` or manual step | `public_repo_corpus_acceptable_false_positive_rate` | Integration | `kibitzer run <repo>` across each corpus repo (`kubernetes/kubernetes`, `BurntSushi/ripgrep`, `denoland/deno`, `servo/servo`, `apache/cassandra`, `microsoft/vscode`, `stapler-squad`); FP rate recorded; qualitatively reviewed to confirm no systematic noise |
| AC-8: `needs_private_symbols` returns `true` | `src/feature_envy.rs` | `test_needs_private_symbols_returns_true` | Unit | `FeatureEnvyChecker.needs_private_symbols()` == `true` |

---

## UX Acceptance Tests

N/A — pure infrastructure checker, no user-facing surface.

---

## Test Stack

- **Unit**: Rust `#[cfg(test)] mod tests` inside `src/feature_envy.rs`; assertions via `assert_eq!` / `assert!`; run with `cargo test`.
- **Integration**: Corpus backtests via `kibitzer check backtest feature-envy` (session-transcript corpus) and `kibitzer run <repo>` (public-repo corpus per `docs/backtest-repos.md`). These are manual Phase 5 steps; results recorded in `project_plans/feature-envy-detection/decisions/validation.md`.
- **E2E / UX**: N/A

---

## Test Construction Notes

### Unit test helpers

Each unit test that exercises `feature_envy_findings_for_package` or higher-level checker invocation should construct a minimal `ArchModel` in-memory (no filesystem). The key builders:

- `ArchModel` with synthetic `packages`, `call_edges`, and `field_accesses` fields — no real Go parsing needed for most tests.
- For tests that exercise the ATFD walker (`foreign_field_accesses_per_type`), pass a small inline Go snippet as `source: &str` and parse it via `tree_sitter_go` directly, same pattern as `god_class.rs` tests.
- Suppression tests rely only on `SymbolNode.file` values and type-name strings — no AST required.

### Key edge cases wired to acceptance criteria

| AC | Edge case | Why it matters |
|----|-----------|---------------|
| AC-2 | `own = 0` (zero local field accesses) | `foreign_ratio` = 1.0 by definition; `min_total_guard` must catch this before the ratio check |
| AC-3 | Tie between two foreign types at equal count | `dominant_type` must be deterministic; use `max_by_key` with a secondary lexicographic key to avoid non-deterministic test failures |
| AC-4 | Method has foreign calls but zero foreign field accesses | Should emit `move-function` only (not `move-field`) since `move-field` requires `foreign_fld >= FEATURE_ENVY_MIN_FOREIGN` |
| AC-5 | Type suffix `*Wrapper`, `*Proxy`, `*Decorator` (not just `*Adapter`) | All four suffixes in `INTENTIONAL_DELEGATION_SUFFIXES` need coverage; at minimum one non-Adapter suffix tested |
| Plan fix | Same foreign type appears as `"Customer"` in `field_counts` and `"pkg::Customer"` in `call_counts` | Without normalization, merged total is split across two keys, dropping the type below `FEATURE_ENVY_MIN_FOREIGN` and silencing the finding |

---

## Coverage Targets and How to Measure

| Stack | Coverage command | Target |
|---|---|---|
| Rust unit | `cargo tarpaulin --out Stdout -- --test-threads=1` | ≥ 80% line coverage on `src/feature_envy.rs` |
| Integration backtests | Manual: run and record TP/FP table in `decisions/validation.md` | Zero panics; FP rate qualitatively acceptable (< ~20% across corpus) |
