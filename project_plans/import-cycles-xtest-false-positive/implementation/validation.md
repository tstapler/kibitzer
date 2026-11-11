# Validation Plan: import-cycles-xtest-false-positive

**Date**: 2026-11-10  
**Complexity**: 1 — 1 test per requirement; unit+error+integration triad, pre-mortem, and Product Triad Review are all skipped per calibration.

## Happy Path Scenario

Given a Go module containing `package widget`, a sibling `package widgettest` test-helper (which imports `widget`), and an external test file (`package widget_test`) that imports `widgettest`, when `kibitzer check architecture import-cycles` is run, then no findings are reported — the external test file does not appear in the import graph at all.

---

## Requirement → Test Mapping

| Requirement | Test File | Test Name | Type | Scenario |
|---|---|---|---|---|
| AC-1: no false cycle on X/Xtest/X_test triple | `src/import_graph.rs` | `go_external_test_package_does_not_create_false_cycle` | Unit | Three-package layout: widget / widgettest (imports widget) / widget_test.go (package widget_test, imports widgettest). Assert zero cycle findings. |
| AC-2: real cycles still reported | `src/import_graph.rs` | `go_import_graph_finds_a_two_package_cycle` (existing) | Unit | Two non-test packages that mutually import each other. Assert both directed edges exist in the graph. |
| AC-3: external test file edges excluded from graph | `src/import_graph.rs` | `go_external_test_package_does_not_create_false_cycle` | Unit | Assert `graph.nodes` does not contain `example.com/m/widget_test` and `graph.edges` has no edge from `widget` to `widgettest`. |
| AC-4: internal test files unaffected | `src/import_graph.rs` | `go_internal_test_file_still_appears_in_graph` | Unit | A `widget_test.go` file declaring `package widget` (internal). Assert node and `file_packages` entry for `example.com/m/widget` still present. |
| AC-5: coupling and layering checkers unaffected | `src/import_graph.rs` | `go_internal_test_file_still_appears_in_graph` + existing coupling/layering tests | Unit | Existing tests for `LayeringChecker` and `CouplingChecker` pass without modification. The upstream graph fix is the only change; checker logic is untouched. |
| AC-6: regression test added | `src/import_graph.rs` | `go_external_test_package_does_not_create_false_cycle` | Unit | Test exists and passes after the patch (`cargo test go_external_test_package_does_not_create_false_cycle`). |
| AC-7: backtest passes on corpus | Manual / CI | Corpus backtest (`kibitzer check architecture import-cycles` on `kubernetes/kubernetes` or `servo/servo`) | Integration | Before/after finding counts documented in PR. Count must not increase; a decrease is expected. |

---

## UX Acceptance Tests

N/A — pure checker fix; no user-facing surface changes.

---

## Test Stack

- **Unit**: Rust standard `#[test]` with `assert!` / `assert_eq!` — the existing pattern throughout `src/import_graph.rs::tests`.
- **Integration**: Manual corpus backtest using `kibitzer check architecture import-cycles <repo>` CLI.
- **E2E / UX**: N/A.

---

## Coverage Targets and How to Measure

| Stack | Coverage command | Target |
|---|---|---|
| Rust | `cargo test` | All new and existing `import_graph` tests green |
| Corpus backtest | `kibitzer check architecture import-cycles <repo> \| wc -l` (before vs. after) | Count must not increase |

---

## Readiness Gate (self-check)

| # | Criterion | Status |
|---|-----------|--------|
| 1 | Every requirement in requirements.md has ≥1 test case above | ✅ All 7 ACs mapped |
| 2 | plan.md has no TODO/TBD placeholders | ✅ None found |
| 3 | All ADRs referenced in plan.md exist on disk | ✅ No ADRs referenced |
| 4 | No BLOCKER items in adversarial-review.md | ✅ Verdict: CLEAN |
| 5 | No BLOCKER items in architecture-review.md | ✅ File absent (Complexity 1) |
| 6 | Schema changes: N/A | ✅ No schema changes |
| 7 | No P1 items in pre-mortem.md | ✅ File absent (Complexity 1, skipped) |
| 8 | No planned package creates a new import cycle or cross-layer dependency | ✅ Change is entirely within `src/import_graph.rs`; no new package dependencies |

**Verdict: PASS** — implementation ready to proceed.
