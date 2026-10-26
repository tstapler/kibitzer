# Validation: interface-impl-carveout-delegation

**Date**: 2026-10-10
**Phase**: sdd:4-validate (retroactive; run after implementation at `c1a8017`/`fdfb3b6`/`dc7f40a`)
**Inputs**: `requirements.md`, `implementation/plan.md`, `decisions/validation.md` (adversarial plan review), `git diff 67885ac~1..HEAD -- src`
**Scope of the change**: `src/single_call_site_delegation.rs` only (111 lines changed: +105/-6).

## Test run (evidence)

Command: `cargo test --bin kibitzer single_call_site`

```
running 13 tests
test single_call_site_delegation::tests::does_not_flag_a_delegating_function_with_two_callers ... ok
test single_call_site_delegation::tests::does_not_flag_a_delegating_function_with_zero_callers ... ok
test single_call_site_delegation::tests::does_not_flag_a_function_with_an_empty_body ... ok
test single_call_site_delegation::tests::does_not_flag_a_function_with_a_multi_statement_body ... ok
test single_call_site_delegation::tests::does_not_flag_an_exported_delegating_function ... ok
test single_call_site_delegation::tests::does_not_flag_an_unsupported_language_even_with_one_caller ... ok
test single_call_site_delegation::tests::flags_a_go_method_delegating_to_a_call_with_one_caller ... ok
test single_call_site_delegation::tests::flags_a_return_call_delegating_function_with_one_caller ... ok
test single_call_site_delegation::tests::does_not_flag_ts_method_on_type_sharing_an_interface_with_another_implementer ... ok
test single_call_site_delegation::tests::flags_a_typescript_function_delegating_to_a_call_with_one_caller ... ok
test single_call_site_delegation::tests::flags_a_bare_call_delegating_function_with_one_caller ... ok
test single_call_site_delegation::tests::flags_ts_method_on_sole_implementer_of_its_interface ... ok
test single_call_site_delegation::tests::go_method_without_implements_edges_is_still_flagged ... ok

test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 1552 filtered out
```

10 tests pre-existed; 3 are new (the last three in the list order of the source file).

## Acceptance criteria (backlog item `7600247a-...`, issue #117)

| # | Criterion | Evidence | Class | Status |
|---|---|---|---|---|
| AC1 | TS multi-implementer method suppressed | `does_not_flag_ts_method_on_type_sharing_an_interface_with_another_implementer` (Sq and Circ both `implements Shape`; private `thin` has one caller, is not flagged) | unit (model built from real TS source via `model_from`; exercises extractor + checker) | PASS |
| AC2 | Sole-implementer method still flagged | `flags_ts_method_on_sole_implementer_of_its_interface` (Sq is the only implementer; `thin` flagged) | unit | PASS |
| AC3 | Existing tests unchanged | `git diff 67885ac~1..HEAD -U0 -- src/single_call_site_delegation.rs` removed lines are only the 3 doc-comment lines, the two `use` lines, and the `std::collections::HashSet` path in `caller_counts`; no test line removed or edited. All 10 pre-existing tests pass (run above). | regression (existing unit) | PASS |
| AC4 | Go no-regression test | `go_method_without_implements_edges_is_still_flagged` (asserts the model has zero `Implements` edges for structural Go satisfaction, then asserts exactly one finding on `method thin`). Pre-existing `flags_a_go_method_delegating_to_a_call_with_one_caller` also covers Go. | unit | PASS |
| AC5 | Doc comment drops #117, cites `arch_model.rs:226-230` | `grep -rn '#117' src` returns nothing. Header (lines 10-16) now cites `arch_model.rs:226-230`; that range is the "v1 scope note: Go ... structural satisfaction ... out of scope" paragraph on `TypeRelationEdge`, so the citation is accurate. | static / doc inspection | PASS (no automated test; doc content is not testable) |
| AC6 | `kibitzer check backtest` passes | NOT RUNNABLE. `./target/debug/kibitzer check backtest single-call-site-delegation` -> `Error: no checker named 'single-call-site-delegation' registered`. The backtest only resolves per-file `checker::registry()` checkers; this is a whole-repo `ArchModelChecker` (`src/architecture_checks.rs:529`). Substitute: AC7 corpus run below. | integration (substitute only) | FAIL as written; criterion is not satisfiable for this checker type |
| AC7 | vscode spot-check | Re-run on current binary: `kibitzer check architecture single-call-site-delegation ~/code/github.com/microsoft/vscode` -> 3222 findings, matching the 3222 recorded in `decisions/validation.md` post-change. The recorded before figure (4059, 837 removed, 0 added, 12/12 sampled removals in classes with an explicit `implements`) is taken from that doc; the HEAD~1 binary was not rebuilt in this run (UNVERIFIED here, VERIFIED only for the after count). | integration (manual corpus run, not an automated test) | PASS (manual evidence) |

## Requirement traceability

| Requirement (requirements.md) | Evidence | Class | Status |
|---|---|---|---|
| R1: private TS/TSX/JS method whose owning type implements an interface is not flagged | AC1 test | unit | Covered (TS only; TSX/JS share the same extractor path but have no dedicated test) |
| R2: sole-implementer refinement (still flagged) | AC2 test | unit | Covered |
| R3: Go method with one caller still reported, no regression | AC4 tests | unit | Covered |
| R4: doc comment updated, #117 removed, Go caveat kept accurate | AC5 inspection | static | Covered (no test) |
| R5: all existing unit tests pass unchanged | AC3 | regression | Covered |
| R6: new tests cover TS/Go cases | 3 new tests | unit | Covered |
| C1: do not modify `ArchModel`/`TypeRelationEdge`/`symbol_extract.rs` | `git diff --stat` shows only `src/single_call_site_delegation.rs` under `src/` | static | Covered |
| C2: use only `resolved == true` edges (rabbit-hole note) | Implemented in `implements_index` filter (`e.resolved && e.kind == Some(Implements)`); no test with an unresolved/external `implements` target | - | NO TEST |
| C3: skip non-methods (no owning type) | `implements_shared_interface` returns false when the id has no `.name` suffix; exercised only indirectly by pre-existing free-function tests (`flags_a_typescript_function_...`, `flags_a_bare_call_...`), which would break if free functions were wrongly suppressed | unit (indirect) | Covered indirectly, no dedicated test |
| NFR1: correctness over completeness (no Implements edge -> unchanged behaviour) | `go_method_without_implements_edges_is_still_flagged`, plus all pre-existing flag tests | unit | Covered |
| NFR2: performance (one extra linear scan, O(1) per-symbol lookups) | `implements_index` built once per `check`; no benchmark or test | - | NO TEST (design-level; low risk) |
| Plan finding 1/2 (index built once, no per-symbol `type_edges` scan) | Implemented as `ImplementsIndex` (`interfaces_of`, `implementer_counts`) | static | Covered by inspection |
| Plan finding 3 (`any` semantics: type with sole interface A and shared interface B is suppressed) | Implemented as `.any(|iface| count > 1)` | - | NO TEST |
| Plan Story 7: kubernetes Go corpus no-regression (same finding count) | Not recorded in `decisions/validation.md`; not run here | integration | NO EVIDENCE |

## Gaps (requirements with no test)

1. **C2 unresolved edges**: no test that an `implements` target which does not resolve (external/vendored interface) fails to suppress. Behaviour is implemented, untested.
2. **Plan finding 3**: no test for a type implementing both a sole-implementer interface and a shared interface (should suppress).
3. **Deduplication by (type, interface) pair**: `implements_index` dedups via `HashSet<(&str,&str)>`; no test with duplicate edges (e.g. same `implements` listed across declaration-merged or duplicated classes) guarding the implementer count.
4. **Cross-file implementers**: both new tests put all classes in one file; no test where the second implementer lives in another file/package.
5. **TSX/JS/Java/Kotlin**: only `.ts` is tested; Java/Kotlin are explicitly out of scope (no `call_graph_supports`).
6. **Go kubernetes corpus run** (plan Story 7) has no recorded evidence.
7. **AC6** cannot pass as worded; the backlog criterion should be amended to the architecture-checker corpus run.

## Verdict

Requirements covered by a named passing test or recorded evidence: 7 of 7 acceptance criteria addressed (AC1-AC5 and AC7 pass; AC6 fails as worded and is replaced by AC7's corpus run). Of the 14 traceability rows, 9 are covered, 1 is covered indirectly, and 4 have no test or evidence (C2, NFR2, plan finding 3, Go corpus run). No source was modified by this validation.

## Implementation readiness gate (run retroactively, after implementation)

| # | Criterion | Result |
|---|-----------|--------|
| 1 | Every requirement has >=1 test | PASS with gaps: 4 traceability rows have no test (unresolved edges, mixed sole/shared interface, dedup, cross-file implementers) |
| 2 | No TODO/TBD in plan.md | PASS |
| 3 | ADRs exist | PASS (none referenced) |
| 4 | No BLOCKER in adversarial review | PASS (decisions/validation.md findings were resolved in plan) |
| 5 | No architecture-review BLOCKER | PASS (file absent) |
| 6 | Schema migration plan | N/A |
| 7 | No open P1 in pre-mortem.md | **FAIL** — P1-1 open |
| 8 | No import cycle / layering violation | PASS |

Verdict: **FAIL on criterion 7.** Disposition of P1-1 (carve-out is per type, not per method; suppresses
private/protected/`#` methods unreachable through an interface): this is a defect in the spec, not a deviation
from it. Backlog AC1 states "a private ... method whose owning type has at least one resolved Implements edge is
not flagged", and the unit test and requirements.md success metric encode that. Making the carve-out
method-level would contradict AC1 as written, so it was not changed unilaterally. Escalated to the reviewer in
the review summary; recommended follow-up is a method-level carve-out (or at minimum skipping
private/protected/`#` methods) plus a language gate for Go struct embeds (P2-1), then re-run both corpus diffs.
