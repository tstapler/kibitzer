# Validation Plan: go-package-focus

**Date**: 2026-10-23

## Happy Path Scenario

Given a Go module at `testdata/dogfood-architecture/mixed_concerns/` whose symbols form two
disconnected type-level clusters (decoder symbols vs. storage symbols with no shared field
accesses), when `kibitzer architecture check --checker go-package-focus` runs, then the
command exits 0 and emits exactly one finding whose message begins with
`[go-package-focus] Candidate split` and includes a coverage diagnostic line.

---

## Requirement → Test Mapping

| Requirement | Test File | Test Name | Type | Scenario |
|-------------|-----------|-----------|------|----------|
| AC-1 happy | `src/architecture_checks.rs` | `fan_in_out_should_return_ce_1_when_same_package_imported_from_multiple_files` | Unit | Two `ImportEdge`s with identical `(from="a", to="b")` but `file="a/f1.go"` and `file="a/f2.go"` → `fan_in_out` returns Ce=1 for `"a"`, Ca=1 for `"b"` |
| AC-1 error | `src/architecture_checks.rs` | `fan_in_out_should_return_ce_2_when_source_imports_two_distinct_packages` | Unit | Two `ImportEdge`s with `from="a"`, `to="b"` and `from="a"`, `to="c"` → Ce=2 for `"a"` (pairs are not collapsed) |
| AC-1 integration | `tests/go_package_focus_integration.rs` | `multi_file_same_import_fixture_should_yield_correct_instability_under_architecture_check` | Integration | `testdata/dogfood-architecture/multi_import/` fixture (two `.go` files both importing the same package) is processed by `InstabilityChecker`; Ce for the multi-file package equals 1, not 2 |
| AC-2 happy | `src/go_package_focus.rs` | `clustering_should_merge_methods_into_type_super_nodes_before_building_components` | Unit | Construct an `ArchModel` with 4 methods on one type and 4 methods on another type; after super-node collapse the Union-Find has 2 type nodes, not 8 method nodes; both types end up in separate components only when their field-access sets are disjoint |
| AC-2 error | `src/go_package_focus.rs` | `go_package_focus_checker_should_not_emit_split_when_only_one_type_has_disconnected_methods` | Unit | `ArchModel` with one type whose methods share no field accesses (extract-class territory); checker returns no findings — single type-node cannot form two components each with ≥ 2 type nodes |
| AC-3 happy | `src/go_package_focus.rs` | `go_package_focus_checker_should_emit_candidate_split_finding_when_package_has_mixed_concerns` | Unit | Minimal `ArchModel` stub for `mixed_concerns` package (two type nodes, zero cross-type field accesses, each type with ≥ 2 methods); `check()` returns exactly one `ArchFinding` whose message starts with `[go-package-focus] Candidate split` |
| AC-3 error | `src/go_package_focus.rs` | `go_package_focus_checker_should_emit_no_finding_when_package_is_cohesive` | Unit | `ArchModel` stub for `cohesive_parser` package (all method nodes connected via shared `ParseContext` field accesses → single cluster); `check()` returns empty vec |
| AC-3 integration | `tests/go_package_focus_integration.rs` | `go_package_focus_checker_should_run_without_panic_on_dogfood_architecture` | Integration | Run `GoPackageFocusChecker::check()` with the model built from `testdata/dogfood-architecture/`; assert no panic and exit code 0 |
| AC-4 happy | `src/go_package_focus.rs` | `finding_should_include_private_symbol_count_when_model_has_pruned_symbols` | Unit | `ArchModel` with `pruning.private_symbols_skipped = 3`; finding message contains the substring `"3 private symbols not modeled"` |
| AC-4 error | `src/go_package_focus.rs` | `finding_should_show_zero_private_symbols_when_all_symbols_are_public` | Unit | `ArchModel` with `pruning.private_symbols_skipped = 0`; finding message contains `"0 private symbols"` (diagnostic present, count is zero) |
| AC-5 unit — generated | `src/go_package_focus.rs` | `go_package_focus_checker_should_skip_package_when_all_files_have_generated_banner` | Unit | `ArchModel` package whose sole file has the `// Code generated … DO NOT EDIT.` banner (propagated via `PackageNode::is_generated` or equivalent flag); `check()` returns no finding for that package |
| AC-5 unit — test cycles | `src/go_package_focus.rs` | `cycle_risk_should_exclude_import_edges_whose_file_ends_with_test_go` | Unit | Two packages whose only mutual import edges have `file` suffixes `_test.go`; cycle-risk section of the finding is absent (no cycle risk reported) |
| AC-5 unit — init | `src/go_package_focus.rs` | `clustering_should_not_count_init_call_edges_when_from_symbol_name_is_init` | Unit | Call edge with `from = "pkg::init"` connecting two otherwise-unconnected type groups; after filtering, the types remain in separate components (init edge excluded from adjacency graph) |
| AC-5 integration | `tests/go_package_focus_integration.rs` | `shared_lock_fixture_should_produce_no_split_finding` | Integration | `testdata/dogfood-architecture/shared_lock/` fixture (shared `sync.Mutex` across function groups); `GoPackageFocusChecker::check()` returns no split finding (shared state is contrary evidence) |
| AC-6 integration | `tests/go_package_focus_integration.rs` | `size_only_mode_should_produce_informational_finding_on_large_unfocused_package` | Integration | `GoPackageFocusChecker` in size-only mode (graph edges suppressed) against a package large by LOC/file count; finding message contains `[informational]`, not `[advisory]` |
| AC-6 integration | `tests/go_package_focus_integration.rs` | `combined_mode_should_produce_no_finding_when_cohesive_parser_fixture_is_used` | Integration | `GoPackageFocusChecker` in default combined mode against `testdata/dogfood-architecture/cohesive_parser/`; returns no findings — graph connectivity overrides size signal |
| AC-7 happy | `src/go_build_cost.rs` | `go_version_should_return_major_minor_tuple_when_binary_reports_go1_21` | Unit | Mock `go version` stdout as `"go version go1.21.3 linux/amd64"`; `go_version()` returns `Ok((1, 21))` |
| AC-7 error | `src/go_build_cost.rs` | `go_version_should_return_diagnostic_string_when_version_is_below_1_19` | Unit | Mock returns `"go version go1.18.0 linux/amd64"`; `go_version()` returns the human-readable diagnostic string (not `Err`), confirming graceful degradation |
| AC-7 integration | `tests/go_package_focus_integration.rs` | `run_build_cost_should_parse_actiongraph_json_without_panic_on_dogfood_architecture` | Integration | Machine has Go ≥ 1.19; `run_build_cost(Path::new("testdata/dogfood-architecture"))` parses the `-debug-actiongraph` JSON and returns a `BuildCostReport` with `action_count > 0`; no panic, no `GOCACHE` mutation |
| AC-8 doc | `tests/go_package_focus_integration.rs` | `docs_go_package_focus_should_contain_split_decision_checklist_section` | Doc/Integration | `docs/go-package-focus.md` exists and `read_to_string` contains the substring `"split decision checklist"` (case-insensitive) |
| AC-8 doc | `tests/go_package_focus_integration.rs` | `docs_go_package_focus_should_contain_ci_gateable_vs_advisory_section` | Doc/Integration | Same file contains both `"CI-gateable"` and `"advisory"` as sibling headings or in the same section — verifies the distinction is explicit, not just implied |

---

## UX Acceptance Tests

N/A — no user-facing browser surface.

---

## Test Stack

- **Unit**: Rust / `cargo test` / `#[cfg(test)]` inline modules in `src/architecture_checks.rs`, `src/go_package_focus.rs`, `src/go_build_cost.rs`
- **Integration**: Rust / `cargo test` / `tests/go_package_focus_integration.rs` (new file); uses real fixtures under `testdata/dogfood-architecture/` and (for AC-7) the system Go binary
- **Doc check**: Rust / `cargo test` — two `#[test]` functions in `tests/go_package_focus_integration.rs` that open `docs/go-package-focus.md` and assert required section strings are present; these are also tagged `#[ignore]` under a `doc_check` feature so they can be skipped in environments where the doc file has not yet been written
- **E2E / UX**: N/A

---

## Fixtures Required

| Fixture path | Purpose | Expected checker outcome |
|---|---|---|
| `testdata/dogfood-architecture/multi_import/` | Two `.go` files each importing the same package; triggers AC-1 regression | Architecture check passes with Ce=1 |
| `testdata/dogfood-architecture/mixed_concerns/` | Two independent function/type groups; no shared field accesses | `go-package-focus` emits one split finding |
| `testdata/dogfood-architecture/cohesive_parser/` | All symbols share a `ParseContext` struct field access | `go-package-focus` emits no finding |
| `testdata/dogfood-architecture/shared_lock/` | `sync.Mutex` shared across function groups | `go-package-focus` emits no finding (contrary evidence) |
| `docs/go-package-focus.md` | Split/keep decision procedure (Phase 4 output) | Doc check tests pass |

---

## Coverage Targets and How to Measure

| Stack | Coverage command | Target |
|---|---|---|
| Rust unit + integration | `cargo tarpaulin --out Stdout` | ≥ 80% line coverage across `src/go_package_focus.rs` and `src/go_build_cost.rs` |
| `fan_in_out` fix | Covered by two focused unit tests + one integration test | 100% branch coverage of the dedup path (`HashSet` insertion + count) |

- All public checker methods (`name`, `check`, `needs_private_symbols`) on `GoPackageFocusChecker`: happy path + error paths covered by unit tests.
- `go_version()` and `run_build_cost()`: happy path + version-gate error path covered; external `go` binary calls are tested via integration test (real binary) and unit test (mocked stdout).
- Coverage diagnostics code path: exercised by two dedicated unit tests (pruned vs. clean model).
- Edge-case guards (generated, test-only, init): one unit test each.
- Mode selection (size-only vs. combined): two integration tests.

---

## Summary

| Test type | Count |
|---|---|
| Unit | 13 |
| Integration | 6 |
| Doc/text assertion (in integration suite) | 2 |
| **Total** | **21** |

Requirements coverage: **8/8 ACs** (100%).

Migration tests: N/A — no database or schema involved.
