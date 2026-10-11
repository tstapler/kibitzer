# Validation: go-stdlib-idioms-fp

## Plan completeness check

### Does the plan address every AC?

| AC | Addressed? | Where |
|----|-----------|-------|
| AC-1 — sort.Slice comparator suppressed | ✅ | plan.md Change 1, §1.1 + §1.5 test `ignores_sort_slice_comparator` |
| AC-2 — full slices/sort stdlib coverage | ✅ | plan.md §1.1 allowlist includes `sort.Slice`, `sort.SliceStable`, `sort.Search`, all `slices.*` variants |
| AC-3 — test closure suppressed | ✅ | plan.md §1.2 + §1.5 test `ignores_test_file_closure` |
| AC-4 — exported function true-positive preserved | ✅ | plan.md §1.5 test `still_flags_exported_func_in_test_file` scoped to `func_literal` only |
| AC-5 — slog key string excluded from magic-literal | ✅ | plan.md Change 2, §2.1–§2.3 + §2.5 test `suppresses_slog_key_string_in_log_warn` |
| AC-6 — non-slog true-positive preserved | ✅ | plan.md §2.5 test `does_not_suppress_non_slog_string` |
| AC-7 — backtest gate | ✅ | plan.md §Backtest tasks items 1–3 |
| AC-8 — regression tests | ✅ | plan.md §1.5 (5 tests) + §2.5 (5 tests) = 10 new unit tests |

All 8 ACs are addressed.

## Implementation risk review

### Risk 1: `argument_list` child indexing

**Plan says:** Use `named_children()` for index computation.

**Verification needed:** Confirm tree-sitter-go's `argument_list` children include comma
tokens as unnamed children. The existing `go_param_identifier_count` in `rules.rs` uses
`children(cursor).filter(...)`, not `named_children()`. The `walk_literals` function
also uses `node.children(&mut cursor)`.

**Action before coding:** Run `to_sexp()` on a real `argument_list` node with commas to
confirm unnamed comma tokens are present. Use `named_children()` in the new code to get
a clean index. This is a one-line verification — file a CLAUDE.md-style note in the
implementation PR if the answer is non-obvious.

### Risk 2: `func_literal` parent path in `is_stdlib_callback_literal`

**Plan says:** Walk up `parameter_list → func_literal → argument_list → call_expression`.

**Potential issue:** If a `func_literal` is passed as an argument via a variable
(`less := func(i, j int) bool { ... }; sort.Slice(s, less)`), the node is NOT in
`argument_list` — it's in a `var_declaration` / `short_var_declaration`. The suppression
would correctly NOT apply in that case (the variable-declared closure has the same
shape as a named function declaration; flagging it is correct behavior).

**No action needed** — this is the right behavior, not a gap.

### Risk 3: Slog message at index 0 also being a string

**Plan says:** Only suppress at odd indices ≥ 1; index 0 is the message.

**Potential issue:** The message at index 0 might repeat 3+ times across calls
(e.g. a common log prefix). That should still fire.

**Correctly handled:** Index 0 is not tagged as a slog key (only indices 1, 3, 5, …).
AC-5's test `does_not_suppress_slog_message_arg` covers this.

### Risk 4: Allowlist function text matching

**Plan says:** Match `call_expression.function` text against a string list.

**Potential issue:** `sort.Slice` in source text is a `selector_expression` node
whose `utf8_text(src)` returns the full `"sort.Slice"` string — verified by how the
existing `go_panic_detector` uses `func.utf8_text(src) == Ok("panic")` for a plain
identifier. For a `selector_expression`, the same call returns the full selector text
including the package prefix.

**Action:** Confirm `selector_expression.utf8_text(src)` returns `"sort.Slice"` (not
just `"Slice"`) in a test fixture before landing. Add a test `ignores_sort_slice_comparator`
that will fail if the text-match is wrong.

### Risk 5: slog `With` call semantics

`slog.With(key1, val1, ...)` is a constructor-like call that takes only key-value pairs
(no leading message argument). At index 0 the first argument is already a key, not a
message.

**Gap in plan:** The plan suppresses indices 1, 3, 5, … but `slog.With` needs indices
0, 2, 4, … to be suppressed instead.

**Resolution options:**
1. Omit `slog.With` from the method-name list (conservatively don't suppress it;
   `"file"` appearing in `slog.With` calls can use `kibitzer:ignore`).
2. Add a separate `slog_with_functions` set that suppresses even indices.

**Recommendation:** Option 1 — omit `With` from the initial list. This is the safer
and more conservative fix. It can be added in a follow-up once the AST shape of
`slog.With` is verified. Note the gap in the implementation PR.

### Risk 6: Incomplete test coverage for the `slices.*` callback family

The plan tests `slices.SortFunc` in `ignores_slices_sort_func_comparator`. Some
`slices.*` functions take `func(T) bool` (single-parameter, not a pile-up) and wouldn't
fire regardless. The ones that could fire are `slices.SortFunc`/`slices.SortStableFunc`
(take `func(a, b T) int`) — only when `T` is a primitive type (e.g. `int`).

**Action:** The test fixture for `ignores_slices_sort_func_comparator` should use
`slices.SortFunc(s, func(a, b int) int { ... })` to exercise the int pile-up.

## Tests coverage summary

10 new unit tests cover:
- 2 stdlib callback suppression cases (sort, slices)
- 1 standalone func_literal true-positive guard (non-test, non-callback closure)
- 2 test-file suppression cases (func_literal, exported function not suppressed)
- 2 slog key suppression cases (log.Warn, slog.Warn)
- 1 slog message non-suppression guard
- 1 non-slog string true-positive guard
- 1 method-style logger suppression case

This matches the 10-test estimate from the plan.

## Final verdict

The plan is implementable as written with the following amendments:
1. **Drop `slog.With` from the method list** — add as a follow-up once its AST shape is verified.
2. **Verify `argument_list` named-children indexing** — confirm commas are unnamed children before coding.
3. **Verify `selector_expression.utf8_text()` includes package prefix** — confirm before the allowlist text-match code.

No architectural issues. Both changes are narrowly scoped to their respective checkers,
follow the repo's existing two-pass architecture and suppression patterns, and have clear
regression-test coverage.
