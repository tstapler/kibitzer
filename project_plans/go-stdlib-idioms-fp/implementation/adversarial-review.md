# Adversarial Review: go-stdlib-idioms-fp

**Date**: 2026-10-10
**Verdict**: BLOCKED

## Blockers

- [ ] **`*Context` variant position arithmetic is wrong in `go_is_slog_key_literal`** — The `idx % 2 == 1` rule in `go_is_slog_key_literal` correctly identifies keys for `slog.Warn(msg, key, val, ...)` (keys at odd positions 1, 3, 5) but breaks for `WarnContext`, `InfoContext`, `ErrorContext`, `DebugContext`. Those signatures are `(ctx context.Context, msg string, args ...any)`, so physical positions are: ctx(0), msg(1), key(2), val(3), key(4)... Keys land at even indices (2, 4, 6), not odd. With `idx % 2 == 1`, "key" at position 2 is EVEN and is NOT added to `slog_key_ids`, so it still fires as a magic literal. Conversely, "msg" at position 1 is wrongly added to `slog_key_ids` and silently suppressed. The plan's own test `suppresses_slog_context_variant` would FAIL as written: `slog.WarnContext(ctx, "msg1", "file", a)` puts "file" at position 2 (even), so "file" would not be suppressed and the `assert!(findings.is_empty())` assertion would fail. Neither pitfalls.md nor the plan's logic section addresses this case — the research only analyzed `slog.Log`'s offset, not the `*Context` family. Resolution: either (a) remove `InfoContext`/`WarnContext`/`ErrorContext`/`DebugContext` from `SLOG_METHOD_NAMES` and update the test expectations accordingly (AC5 becomes weaker), or (b) add a per-method key-start-offset dispatch table so `*Context` methods use `(idx - 1) % 2 == 1` (keys start at index 2 rather than 1). Without this fix the plan ships a failing test and silently broken behavior for a quarter of the slog method set it claims to cover.

- [ ] **AC8 test count is manufactured — actual new test count is 8, not 10** — Story 3.1.2 claims "4 (Story 1.1.3) + 4 (Story 2.1.4) = 8 new tests … plus 2 existing tests that directly exercise the new suppression paths via regression-guard assertions = 10." AC8 requires "at least 10 new unit tests covering each suppression case and each true-positive guard." The 2 existing tests are pre-existing; they cannot count toward "new unit tests." The actual deliverable is 8 new tests. The plan acknowledges the shortfall ("If the count falls short, add edge-case tests"), but does not specify which 2 additional tests close the gap, leaving the AC8 gate undefined at implementation time. Resolution: add 2 explicitly named new tests to the plan — e.g., one for `slices.EqualFunc` (AC2 coverage for a less-common allowlist entry) and one for `slog.ErrorContext` with the corrected position logic once the Blocker above is resolved — so the gate is deterministic before implementation starts.

## Concerns

- [ ] **`git stash pop` in Task 3.1.1 violates repo conventions** — The backtest step says `git stash -u -m "go-stdlib-idioms-fp-baseline"` then `git stash pop`. The repo's CLAUDE.md explicitly forbids bare `git stash pop` because the stash stack is shared across Claude sessions and a concurrent session can push entries that pop underneath the wrong stash. The correct pattern is: capture the SHA immediately after push (`git stash list --format='%H %gs' | grep go-stdlib-idioms-fp-baseline | head -1`), restore with `git stash apply <sha>`, then drop with `git stash drop stash@{n}` (finding `n` by tag). Resolution: update Task 3.1.1's step 2 with the safe stash apply/drop pattern. Low probability of a concurrent push during a local manual backtest run, but following the documented convention avoids a latent trap.

- [ ] **Test-file early return is broader than AC3 states** — AC3 asks to suppress findings for "a func_literal with a same-typed multi-parameter list" in `_test.go` files. The plan implements a blanket early return at the top of `check()` for any file ending in `_test.go`, which suppresses ALL `go-primitive-obsession` findings in that file — including exported function declarations that happen to live in test helper files (e.g., `export_test.go` files used to expose internal symbols, or `testutil/helpers_test.go` with exported test helpers). The AC4 true-positive guard only tests non-test files, so this over-suppression is invisible to the test suite as written. This is broader than the stated requirement and could mask legitimate findings in test-support APIs. Resolution: either accept the known over-suppression and document it in the guard comment, or scope the guard to `func_literal` nodes only (adding the file-path check inside `check_parameter_list` when the node is a `FuncLiteral` parent) — this is more complex but matches AC3 precisely. If the blanket return is kept, add an explicit comment stating that ALL primitive-obsession findings in test files are suppressed by design, not just func-literal parameter lists.

- [ ] **Two test helpers referenced in the plan do not exist** — Story 1.1.3's AC3 test calls `crate::test_support::make_go_context(src)`, which does not exist in `src/test_support.rs`. The module has `parse_go(src) -> Result<Tree>` and `check_go_source(checker, src)` but no `make_go_context`. Story 2.1.4's tests call `check_go_source_for_rule(src, "replace-magic-literal")`, which also does not exist; the existing rules.rs test infrastructure uses `magic_literal_findings(Language::Go, tree_sitter_go::LANGUAGE.into(), src)`. Both references would produce compile errors if copied verbatim. Resolution: update Story 1.1.3 to inline the parse + CheckContext construction (call `parse_go(src).unwrap()` and build the `CheckContext` manually in the test body), and update Story 2.1.4 to use the existing `magic_literal_findings` helper or inline `check_full`. These are mechanical corrections, but leaving them as-is wastes implementation time on a compile failure.

## Minors

- The `slog.With()` exclusion is documented as a deliberate "follow-up after verifying AST shape," but no follow-up issue is created anywhere in the plan. Repeated slog.With keys (e.g., `slog.With("service", name)` in multiple call sites) will continue to fire as magic literals after this PR. Low impact since slog.With is less common than the Info/Warn/Error/Debug family, but a linked issue or a note in the unresolved-questions section with a concrete next step would prevent this from being forgotten.

- Story 1.1.3 tests only 3 of the 10 allowlist entries (Slice, SortFunc, Search). This is reasonable coverage at the unit level, but there is no statement in the plan about which entries share the same structural pattern and are therefore covered by the three representative tests, versus entries that have a different parameter shape and should have their own test. A brief comment in Story 1.1.3 mapping "these three cover the single-closure shape; the multi-return shape (BinarySearchFunc) is implicitly covered by the predicate being field-name-only" would make the intentional coverage gap explicit.

- The `STDLIB_CALLBACK_ALLOWLIST` uses field-name-only matching, so any user-defined function named `Slice`, `SortFunc`, etc. is also exempted. This is acknowledged in ADR-001 and documented in the code comment, so it is not a new finding — noted here only to confirm that the backtest in Phase 3 should specifically include repos with user-defined higher-order functions (e.g., a custom `Slice(s, less)` wrapper) to quantify the false-negative rate, which the plan does not call out explicitly.

---

## Re-review Round 1

**Date**: 2026-10-10
**Scope**: Targeted re-check of the two original blockers only.

### Blocker 1: `*Context` variant position arithmetic — RESOLVED

All three required changes are present:

1. `SLOG_METHOD_NAMES` (Story 2.1.1) no longer includes `WarnContext`, `InfoContext`, `ErrorContext`, or `DebugContext` — confirmed at the constant definition, which lists only `Info`, `Warn`, `Error`, `Debug`, `Log` and explicitly comments that `*Context` variants are excluded.

2. Unresolved Question #3 in the plan documents the exclusion and the required follow-up (per-method key-start-offset table), satisfying the requirement to add a new Unresolved Question.

3. The original `suppresses_slog_context_variant` test (which would have asserted `findings.is_empty()` and failed) is replaced by `still_flags_slog_context_variant_key_string` — a true-positive guard that asserts findings are expected when `slog.WarnContext` is used, correctly capturing the not-yet-implemented case.

### Blocker 2: AC8 test count miscounted — RESOLVED

All three required changes are present:

1. `suppresses_slices_sort_stable_func_callback` is present in Story 1.1.3 (covers `slices.SortStableFunc` with string element type, a genuine second allowlist shape beyond `sort.Slice`).

2. `still_flags_non_slog_value_string` is present in Story 2.1.4 (verifies that a string at value position, even index 2, still fires as a magic literal — an AC6 guard that was missing before).

3. Story 3.1.2 tallies 6 (Story 1.1.3) + 5 (Story 2.1.4) = 11 new tests and explicitly states "Existing tests are not counted toward this bar." Total new count is 11 ≥ 10 (AC8).

**Overall verdict: CLEAN** — both prior blockers are resolved; no new showstoppers observed.
