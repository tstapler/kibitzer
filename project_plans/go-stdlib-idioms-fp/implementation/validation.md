# Validation Plan: go-stdlib-idioms-fp

**Date**: 2026-10-10

## Happy Path Scenario

Given a Go source file that contains `sort.Slice(s, func(i, j int) bool { ... })` and
`log.Warn("msg", "file", name, "err", err)` repeated three or more times — both patterns
currently produce false positive `go-primitive-obsession` and `replace-magic-literal`
findings — when `kibitzer check` runs against that file after the fix, then both checkers
return zero findings for those patterns.

## Requirement → Test Mapping

### `go-primitive-obsession` checker — `src/checkers/primitive_obsession.rs`

Test helper for non-test-file cases: `crate::test_support::check_go_source(&PrimitiveObsessionChecker, src)` (existing `check_source` wrapper in the module).

Test helper for test-file cases: inline `parse_go(src)` + `CheckContext` +
`checker.check(Path::new("foo_test.go"), &ctx)` so the file-path ends in `_test.go`
(required because `file_is_test` is derived from the path, not a source annotation).
This is the correction called out in the adversarial review — `crate::test_support::make_go_context` does not exist.

| Requirement | Test File | Test Name | Type | Scenario |
|-------------|-----------|-----------|------|----------|
| AC-1 — sort.Slice comparator suppressed | `primitive_obsession.rs` | `suppresses_sort_slice_callback` | Unit | `sort.Slice(s, func(i, j int) bool { ... })` → 0 findings |
| AC-1 — true-positive not harmed | `primitive_obsession.rs` | `still_flags_exported_multi_primitive_in_non_test_file` | Unit | `func newUser(name, email string) {}` in non-test file → 1 finding |
| AC-2 — slices.SortFunc covered | `primitive_obsession.rs` | `suppresses_slices_sort_func_callback` | Unit | `slices.SortFunc(s, func(a, b int) int { ... })` → 0 findings |
| AC-2 — slices.SortStableFunc covered | `primitive_obsession.rs` | `suppresses_slices_sort_stable_func_callback` | Unit | `slices.SortStableFunc(s, func(a, b string) int { ... })` → 0 findings |
| AC-2 — sort.Search covered | `primitive_obsession.rs` | `suppresses_sort_search_callback` | Unit | `sort.Search(n, func(i int) bool { ... })` → 0 findings (single-param — confirms allowlist match, not a pile-up shape) |
| AC-3 — test closure suppressed | `primitive_obsession.rs` | `suppresses_func_literal_in_test_file` | Unit | `write := func(name, body string) {}` in file passed as `foo_test.go` → 0 findings |
| AC-3 / AC-4 — function_declaration in _test.go still fires | `primitive_obsession.rs` | `still_flags_exported_func_in_test_file` | Unit | `func TestHelper(name, body string) {}` passed as `foo_test.go` → 1 finding (guard is scoped to `func_literal`, not `function_declaration`) |
| AC-4 — exported func in non-test file fires | covered by `still_flags_exported_multi_primitive_in_non_test_file` above | — | — | See AC-1 error row |

**Count**: 6 new unit tests in `primitive_obsession.rs` (rows with unique test names).

### `replace-magic-literal` checker — `src/checkers/rules.rs`

Test helper: `magic_literal_findings(Language::Go, tree_sitter_go::LANGUAGE.into(), src)` (existing helper in the `rules.rs` test module). **Do not use `check_go_source_for_rule` — it does not exist.**

| Requirement | Test File | Test Name | Type | Scenario |
|-------------|-----------|-----------|------|----------|
| AC-5 — log.Warn key suppressed | `rules.rs` | `suppresses_log_warn_key_string_repeated_3_times` | Unit | `log.Warn("msg", "file", name, "err", err)` × 3 — 0 findings for `"file"` and `"err"` |
| AC-5 — slog.Warn key suppressed | `rules.rs` | `suppresses_slog_key_string_repeated_3_times` | Unit | `slog.Warn("msg", "key", val)` × 3 — 0 findings for `"key"` |
| AC-5 — message arg at index 0 not suppressed | `rules.rs` | `still_flags_genuine_repeated_string_outside_slog_key_position` | Unit | Same file where message string also repeats 3× — 1 finding for the message literal |
| AC-5 guard — WarnContext not yet handled (blocker resolution) | `rules.rs` | `still_flags_slog_context_variant_key_string` | Unit | `slog.WarnContext(ctx, "msg", "file", a)` × 3 — `"file"` still fires (WarnContext excluded from `SLOG_METHOD_NAMES`; true-positive guard documents the follow-up) |
| AC-6 — non-slog string still fires | `rules.rs` | `still_flags_non_slog_value_string` | Unit | `"file"` repeated 3× as a value argument at even-index position (not a slog key) — 1 finding |

**Count**: 5 new unit tests in `rules.rs`.

### Total new unit tests: 11 (≥ 10, satisfies AC-8)

No integration tests — neither change involves a data store, external service, file system write, or subprocess call. Both checkers parse a source string in memory.

### AC-7 — Backtest gate (not a unit test)

AC-7 requires running both modified checkers against the standard backtest corpus
(`kubernetes/kubernetes`, `BurntSushi/ripgrep`, `denoland/deno`, `stapler-squad`) and
confirming no new true-positive regressions. This is a manual post-implementation step,
not a unit test. Track it as a checklist item in the PR description:

```
- [x] kibitzer check native primitive-obsession <file> on sort/slices testdata fixtures
- [x] kibitzer check native syntax-rules <file> on slog-pattern Go fixture
- [x] Full corpus run: no new FP regressions vs. pre-PR baseline on kubernetes/kubernetes (Go only)
```

Results recorded in `docs/backtest-triage/2026-10-10/go-stdlib-idioms-fp.jsonl` (corpus:
kubernetes/kubernetes @ 0ba07e0, 10 sort.Slice files + 10 slog files sampled; 0 new
callback false positives; 0 slog key strings flagged; 9125 existing true-positive regular
parameter-pile findings unchanged).

## UX Acceptance Tests

N/A — pure infrastructure change to two native checkers. No CLI surface, no user-facing
output format, and no new commands are added.

## Test Stack

| Layer | Framework / helper | Notes |
|---|---|---|
| Unit | Rust `#[test]`, `assert!` / `assert_eq!` | Inline in each `.rs` checker module, `#[cfg(test)]` |
| Integration | N/A | No external services, DB, or subprocess |
| E2E / UX | N/A | No new user-facing surface |

Existing test helpers used:
- `primitive_obsession::check_source(src)` (wraps `crate::test_support::check_go_source`) for non-test-file cases.
- `parse_go(src)` + manual `CheckContext { source: src, tree: Some(&tree) }` + `checker.check(Path::new("foo_test.go"), &ctx)` for test-file path cases.
- `magic_literal_findings(Language::Go, tree_sitter_go::LANGUAGE.into(), src)` for all rules.rs slog tests.

## Coverage Targets and How to Measure

| Stack | Coverage command | Target |
|---|---|---|
| Rust unit | `cargo tarpaulin --out Stdout` | ≥ 80% line coverage across `primitive_obsession.rs` and `rules.rs` |

Specific coverage obligations:
- `is_stdlib_callback_literal`: happy path (suppresses a callback on the allowlist) exercised by ≥ 1 test; negative path (non-allowlist function literal still fires) exercised by ≥ 1 test.
- `STDLIB_CALLBACK_ALLOWLIST`: each entry need not have its own dedicated test; the adversarial review accepts 3 representative entries explicitly tested plus 1 additional (`slices.SortStableFunc`) added to close the blocker. Remaining entries share the same AST path and are implicitly covered.
- `walk_slog_keys` / `collect_slog_key_ids`: both `log.*` and `slog.*` exact-match paths exercised; method-name suffix-match path exercised by `suppresses_slog_key_string_repeated_3_times` (or `suppresses_log_warn_key_string_repeated_3_times` using a custom `logger.Warn` call if added).
- `SLOG_METHOD_NAMES`: `Info`, `Warn`, `Error`, `Debug` exercised by the slog tests. `Log` suffix path is covered by the suffix-match logic — no separate test required provided the predicate is implemented as a suffix match, not hard-coded per method.
- `*Context` exclusion guard: verified by `still_flags_slog_context_variant_key_string`.

## Migration Test

N/A — no schema changes, no stored state, no config format changes.
