# Architecture Review: feature-envy-detection
**Date**: 2026-11-03
**Verdict**: CONCERNS (blocker resolved; 4 concerns remain — see Re-review below)

## Blockers

- [ ] **Phase 2, Task 2.2.1c** — Same-package type key format mismatch breaks Move Function dominant-type identification and threshold check — `foreign_field_accesses_per_type` returns bare type names for same-package foreign types (e.g., `"Customer"`) because `foreign_type_of` in `src/god_class.rs` lines 406–418 returns `local_type.to_string()` when the name contains no `.`. `foreign_call_counts_per_type` always produces fully-qualified keys (e.g., `"pkg::Customer"`) because `edge.to.rsplit_once('.')` on a `"pkg::Customer.Method"` value yields `"pkg::Customer"`. When the dominant foreign type is in the same package, the merge step in `feature_envy_findings_for_package` produces two separate map entries — `"Customer" → 2` and `"pkg::Customer" → 3` — instead of one `"pkg::Customer" → 5` entry. Consequences: (a) `dominant_type` returns `"pkg::Customer"` with count 3 instead of 5, so the finding message reports a wrong access count; (b) the threshold check for Move Function uses split counts, so a method with 2 field + 3 call accesses to the same intra-package type only sees each side independently, not the combined 5. Cross-package type keys resolve correctly on both sides, so this silently affects only intra-package feature envy — the most common and most actionable case. Remediation: normalize `field_counts` keys before merging by prepending `"{pkg.path}::"` to bare names (the package path is available from `PackageNode.path` in `feature_envy_findings_for_package`), confirming the fix against both Story 2.1.1's GWT example (`{"Customer": 2}` → `{"pkg::Customer": 2}`) and Story 2.1.2's GWT example (`{"pkg::Pricer": 2}`).

## Concerns

- [ ] **Phase 2, Task 2.2.1c** — `SymbolNode.is_pure_delegation` suppression omitted despite explicit pitfall recommendation — `pitfalls.md` action-item table row 1 says: "Suppress by name suffix *and* by detecting single-expression delegation (reuse `SymbolNode.is_pure_delegation`)." The plan implements only the name-suffix half (`INTENTIONAL_DELEGATION_SUFFIXES`). A wrapper type named `httpRequestHandler`, `grpcTransport`, or anything else that delegates to a single field without advertising it in its type name will fire at ratio 1.0 on every method body. `SymbolNode.is_pure_delegation` is already pre-computed by `symbol_extract.rs` during model build at zero extra cost — skipping a method where `sym.is_pure_delegation` is `true` requires one boolean check. Recommendation: add `if sym.is_pure_delegation { continue; }` in the per-method loop in `feature_envy_findings_for_package`.

- [ ] **Phase 2, Task 2.2.1c** — Zero-own-access pure utility helpers pass `MIN_TOTAL` and fire — `FEATURE_ENVY_MIN_TOTAL = 4` does not prevent a method with 6 foreign accesses and 0 own accesses (total = 6 ≥ 4) from firing at ratio = 1.0. `pitfalls.md` §3 explicitly recommends the `methods_with_own_signal` pre-filter from `architecture_checks.rs` line 867 — skip a method if `local_field_access_count` returns 0 and `call_edges` has no resolved outgoing call to its own type. The plan's Risk Control table acknowledges zero-own-access ratio inflation but claims `MIN_TOTAL` guards against it; `MIN_TOTAL` only guards against very-low-total methods, not utility helpers with 6+ foreign accesses and zero own. Recommendation: add a pre-filter in `feature_envy_findings_for_package` that skips methods where `local_field_access_count` returns 0 and there are no resolved outgoing own-type calls, mirroring `LcomChecker`'s `methods_with_own_signal` pattern at `src/architecture_checks.rs:867`.

- [ ] **Phase 4, Task 4.1.1a** — Missing FileCache stale-tree regression test — `pitfalls.md` §4 says explicitly: "The regression test at `god_class.rs` lines 529–562 is the reference fixture. Feature Envy should have an identical regression test." The four proposed test cases (threshold logic, adapter suppression, test-file suppression, below-threshold negative) do not exercise `FileCache` with two files where the second is shorter than the first. The stale-tree panic (`byte-index-out-of-bounds`) is easy to silently introduce and hard to catch without this specific regression. Recommendation: add a fifth test case modeled on `file_cache_parses_a_second_shorter_file_correctly_after_a_longer_first_one` in `src/god_class.rs` tests.

- [ ] **Phase 2, Task 2.2.1c** — `New*` constructor false-positive class accepted without documentation — the plan's Risk Control table claims "`New*` functions caught by `FEATURE_ENVY_MIN_FOREIGN` ≥ 3 (constructors rarely have 3+ distinct foreign field accesses)." This assumption does not hold for `New*` functions with 3+ parameters: `NewOrder(customer *Customer, pricer *Pricer, logger *Logger)` that sets each field in the initializer body will access 3+ distinct foreign field pairs and fire. `MIN_FOREIGN = 3` is below PMD's `ATFD > 5` calibrated against this pattern. Recommendation: either add `New*` prefix suppression (a one-line `sym.name.starts_with("New")` check), or explicitly document the known FP class in the checker's module-level doc comment and in a Non-goals section so the Phase 5 backtest triage team knows to expect it.

## Nitpicks

- **Phase 1, Task 1.1.1a** — `walk_selectors_on_typed_locals` is promoted to `pub(crate)` unnecessarily. `feature_envy.rs` calls only `collect_foreign_field_accesses`, which calls `walk_selectors_on_typed_locals` internally. Promoting the recursive implementation widens the crate's internal API beyond what's needed; only `collect_foreign_field_accesses` requires promotion. The precedent cited in the plan — `FileCache` and `find_method_node` — are both primary entry points, not implementation details of a higher-level function.

- **Research and plan docs** — `FEATURE_ENVY_MIN_FOREIGN = 3` is described as "conservative relative to PMD's `ATFD > 5`." The relationship is backwards: 3 < 5, so this threshold fires more than PMD does, not less. The threshold value itself is defensible, but the stated rationale misrepresents it.

- **Phase 2, Task 2.1.3a** — `dominant_type` is nondeterministic on tied counts. `counts.iter().max_by_key(|(_, &v)| v)` over a `HashMap` has unspecified iteration order; two types with equal counts produce a non-deterministic dominant type between runs. If tie-breaking matters for finding stability, switch to a deterministic traversal (e.g., iterate a sorted key list or use `BTreeMap` for counts).

- **Phases 2–5** — Test-helper packages (`testutil/`, `testing/`, `fake/` subdirectories) that are not `_test.go` files are not suppressed. The `is_test_file` guard fires only on `_test.go` files; a shared test-helper package at `pkg/testutil/*.go` that reaches into production structs will fire. This is a known FP class distinct from the `is_test_file` guard the plan implements. Record it in the Phase 5 validation artifact rather than blocking.

## Kibitzer run findings (pre-existing, not new)

`kibitzer run src/ --trigger batch` returned advisory-level findings only — no blocking violations in the existing codebase that the new checker would inherit or worsen. No `ADR-000-architecture-constitution.md` found in `docs/adr/`.

---

## Re-review: Blocker Resolution Check (2026-11-03)

**Iteration**: 2 (re-check of the single BLOCKED item from initial review)

**Item checked**: Phase 2 / Task 2.2.1c — same-package type key format mismatch.

**Finding**: RESOLVED. The updated `plan.md` Task 2.2.1c algorithm (step 3b) now contains an explicit **"Normalize `field_counts` keys (required)"** bullet that:
1. Identifies that `foreign_field_accesses_per_type` returns bare keys (e.g., `"Customer"`) for same-package types while `foreign_call_counts_per_type` always returns qualified keys (e.g., `"pkg::Customer"`).
2. Specifies the fix: qualify any key with no `::` to `"{pkg}::{key}"` using the method's own package name before merging; cross-package keys pass through unchanged.
3. States the reason: prevents the same foreign type from splitting across two map entries, which would break dominant-type identification and threshold evaluation.

Additionally, Task 4.1.1a test case 5 ("Same-package foreign type key normalization") specifically verifies that without the normalization the finding is absent (counts remain split 3+2 and never reach `MIN_FOREIGN` from either key alone), and that with normalization the combined count of 5 produces the expected `[feature-envy:move-function]` finding — making the normalization step load-bearing and testable.

**Overall verdict**: CONCERNS — the blocker is resolved; the 4 concerns from the initial review (is_pure_delegation suppression omission, zero-own-access pre-filter, FileCache stale-tree regression test, New* constructor FP documentation) and the nitpicks are unchanged and carry forward.
