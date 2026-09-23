# Adversarial Review: type-hierarchy-graph

**Date**: 2026-09-22
**Verdict**: CONCERNS

## Blockers
(none — the prior blocker is resolved, see below)

## Concerns
- [ ] **Asymmetric backtest rigor across languages.** Epic 3.2 (plan.md:584-613) still only adds a dedicated corpus repo (`android/nowinandroid`) and a 20-sample manual spot-check for Kotlin's three-way `delegation_specifier` split. No equivalent validation task exists for Go embedding or TS/Java `extends`/`implements`, even though the existing 14-repo corpus (`docs/backtest-repos.md`) already contains large amounts of Go struct embedding and Java inheritance at zero marginal clone cost. Untouched by the repair pass. — **Recommendation**: add a lightweight task alongside 3.2.1b that spot-checks `type_edges` against 1-2 already-cloned Go/Java/TS repos, or state explicitly why those languages are lower-risk and don't need it.
- [ ] **The `file_import_aliases`-ordering dependency in `build_model` (Task 1.6.2c, plan.md:447-450) still has no regression test through the real orchestration path.** Task 1.6.2c's text still flags the risk ("must come *after* `file_import_aliases` is computed... reorder `build_model`'s post-loop block accordingly if needed") but Task 1.6.2d (plan.md:452-454) is unchanged from the prior version: its two integration tests are `build_model_populates_type_edges_for_go_embedding` and `..._across_go_and_typescript_in_one_call` — neither is a cross-package *qualified* Go embed (the case that actually exercises `file_import_aliases` ordering) run through `build_model`. Story 1.6.1's qualified-embed AC (line 400-401) still only tests `resolve_type_edges` directly against a hand-built alias map, bypassing `build_model`'s real computation order. Untouched by the repair pass. — **Recommendation**: add one Story 1.6.2 integration test using a two-package Go fixture with an aliased cross-package embed, run through the actual `build_model` entry point.

## Minors
- Task 1.2.1b's Go embed-unwrapping still reads as flat, non-recursive matching; a combined shape like `*pkg.Base[T]` still isn't covered by any AC or the Epic 3.1 perf fixture. Untouched — low real-world likelihood, still worth a fixture case or an explicit out-of-scope note.
- Story 1.5.1's ACs (plan.md:328-338 covers Java's parallel case, not Kotlin's) still don't include a Kotlin `interface Foo : Bar, Baz` (interface-extends-interface) fixture test, unlike Java which gets its own explicit AC. Untouched.
- The `node_kind`/`hint` response fields (Pattern Decisions table, plan.md:43) still don't consult `model.pruning.pruned_symbol_ids` the way `get_architecture_node` does. Untouched.

## Verification notes

**Blocker resolution — VERIFIED RESOLVED.** Independently re-ran `grep -rn "ArchModel {" src/` from the working directory (not trusting the prior review's count) and confirmed 18 literal-construction sites total: `build_model` (arch_model.rs:423, Task 1.1.1b's own site) plus 17 others — 7 more inside `arch_model.rs`'s own `#[cfg(test)]` module (`filtered` at 676, plus test literals at 967, 984, 1711, 1752, 1786, 1820, all confirmed by reading the surrounding code), and 10 spread across `arch_diagram.rs` (2), `architecture_checks.rs` (2), `isp_fat_interface.rs` (1), `god_class.rs` (3), `extract_class.rs` (1), `lsp.rs` (1).

This exactly matches the edited plan.md:
- Story 1.1.1's AC (plan.md:141-142) now states a concrete "`cargo build --tests` succeeds repo-wide" acceptance criterion with a Given-When-Then citing the same 18-site count.
- New Task 1.1.1c (plan.md:185-196) explicitly enumerates all 17 non-`build_model` sites by file and approximate line number, matching my independent grep site-for-site, and makes `cargo build --tests` passing the task's own closing condition.
- The "Summary of new files touched" section (plan.md:616-618) lists all 6 additional files (`arch_diagram.rs`, `architecture_checks.rs`, `isp_fat_interface.rs`, `god_class.rs`, `extract_class.rs`, `lsp.rs`) plus `arch_model.rs`'s own 7 other sites.

No gaps found — every site my independent grep found is named in the plan. The fix is complete, not partial.

**Other concerns from the prior review:**
- `include_private` justification — **resolved**. The Pattern Decisions table (plan.md:60) now has a dedicated row explaining why `TypeHierarchyRequest.include_private` deviates from `CallTraversalRequest`'s precedent (an unexported `Type`/`Interface` in a hierarchy is a normal case, unlike a call-graph query).
- Asymmetric backtest rigor and the `file_import_aliases` integration-test gap — **not touched** by the repair pass (see Concerns above); neither was required to unblock, but both remain open.
