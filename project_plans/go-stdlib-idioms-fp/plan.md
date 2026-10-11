# Plan: go-stdlib-idioms-fp

Two independent changes — one in `src/checkers/primitive_obsession.rs`, one in
`src/checkers/rules.rs` — each narrowly scoped to the false-positive pattern found.

## Change 1: `go-primitive-obsession` — suppress stdlib callbacks and test closures

**File:** `src/checkers/primitive_obsession.rs`

### 1.1 Stdlib callback suppression

Add a helper `is_stdlib_callback_literal(node: Node, src: &[u8]) -> bool` that, given
a `parameter_list` node, walks up the AST to determine whether it belongs to a
`func_literal` that is:
1. Passed as a direct argument (child of `argument_list`)
2. Whose enclosing `call_expression`'s `function` text matches a known stdlib allowlist

Return `true` (suppress) when all three conditions hold.

**Allowlist (text-match on the `function` field):**
```
sort.Slice, sort.SliceStable, sort.Search,
slices.SortFunc, slices.SortStableFunc,
slices.BinarySearchFunc, slices.IndexFunc,
slices.ContainsFunc, slices.EqualFunc, slices.CompareFunc
```

Match by `node.utf8_text(src)` on the `function` field of the `call_expression`.

**AST traversal:**
```
parameter_list → parent → func_literal (kind check)
func_literal   → parent → argument_list (kind check)
argument_list  → parent → call_expression (kind check)
call_expression.child_by_field_name("function") → text match
```

### 1.2 Test file suppression

Add a `file_is_test: bool` flag derived from the file path (`path.to_string_lossy().ends_with("_test.go")`).

In `walk()`, skip any `parameter_list` node when `file_is_test` is true **and** the
node belongs to a `func_literal` (i.e. the parent node's kind is `func_literal`).

Exported function declarations in `_test.go` files are rare but theoretically possible
(exported test helpers). The guard is scoped to `func_literal` only, not all parameter
lists, so `func TestFoo(t *testing.T)` and `func BenchmarkBar(b *testing.B)` still pass
through unaffected (they are `function_declaration` nodes, not `func_literal`).

### 1.3 Update `walk()`

```rust
fn walk(node: Node, src: &[u8], findings: &mut Vec<Finding>, file_is_test: bool) {
    crate::tree_walk::walk_preorder(node, &mut |n| {
        if GoKind::of(n) == GoKind::ParameterList && !is_named_return_list(n) {
            // New guard A: skip test-file func_literal parameters
            if file_is_test && parent_is_func_literal(n) {
                return true;
            }
            // New guard B: skip stdlib-callback func_literal parameters
            if is_stdlib_callback_literal(n, src) {
                return true;
            }
            check_parameter_list(n, src, findings);
        }
        true
    });
}
```

`parent_is_func_literal(n)` → `n.parent().is_some_and(|p| GoKind::of(p) == GoKind::FuncLiteral)`.

### 1.4 Update `check()` in `PrimitiveObsessionChecker`

Thread `file_is_test` into `walk()` from the checker's `check()` method:
```rust
let file_is_test = file.to_string_lossy().ends_with("_test.go");
walk(tree.root_node(), ctx.source.as_bytes(), &mut findings, file_is_test);
```

### 1.5 Unit tests to add

| Test name | Shape | Expected |
|-----------|-------|----------|
| `ignores_sort_slice_comparator` | `sort.Slice(s, func(i, j int) bool { ... })` | 0 findings |
| `ignores_slices_sort_func_comparator` | `slices.SortFunc(s, func(a, b int) int { ... })` | 0 findings |
| `still_flags_standalone_func_literal_pileup` | `f := func(a, b string) {}` in non-test file | 1 finding |
| `ignores_test_file_closure` | `write := func(name, body string)` in path ending `_test.go` | 0 findings |
| `still_flags_exported_func_in_test_file` | `func TestHelper(name, body string) {}` in `_test.go` | 1 finding |

The `check_go_source` helper used by existing tests doesn't pass a file path — add a
`check_go_source_as_test_file` variant (or an extra parameter) for the two test-file cases.

## Change 2: `replace-magic-literal` — suppress slog key positions

**File:** `src/checkers/rules.rs`

### 2.1 New type: `SlogKeyCollector` pass

Add a separate first-pass `walk_slog_keys(root, src) -> HashSet<usize>` that returns the
`Node::id()`s of string literals in slog key position. This follows the repo's existing
two-pass architecture (collect → emit) and keeps `walk_literals` focused.

```rust
fn walk_slog_keys(root: Node, src: &[u8]) -> HashSet<usize> {
    let mut ids = HashSet::new();
    crate::tree_walk::walk_preorder(root, &mut |n| {
        if GoKind::of(n) == GoKind::CallExpression {
            collect_slog_key_ids(n, src, &mut ids);
        }
        true
    });
    ids
}
```

### 2.2 `collect_slog_key_ids`

```
fn collect_slog_key_ids(call: Node, src: &[u8], ids: &mut HashSet<usize>) {
    1. Retrieve call.child_by_field_name("function") → get text
    2. If text doesn't end with a recognized slog method name, return
    3. Retrieve call.child_by_field_name("arguments") → argument_list
    4. Collect named children of argument_list
    5. For each child at index 1, 3, 5, ... (odd, 0-based):
       if GoKind::of(child) == GoKind::InterpretedStringLiteral
       or GoKind::of(child) == GoKind::RawStringLiteral:
           ids.insert(child.id())
}
```

**Recognized slog method name suffixes (match against the full function field text):**

Exact matches (package-qualified):
`slog.Info`, `slog.Warn`, `slog.Error`, `slog.Debug`, `slog.Log`,
`slog.InfoContext`, `slog.WarnContext`, `slog.ErrorContext`, `slog.DebugContext`,
`slog.With`,
`log.Info`, `log.Warn`, `log.Error`, `log.Debug`,
`log.InfoContext`, `log.WarnContext`, `log.ErrorContext`, `log.DebugContext`

Method-name suffix match (for `logger.Warn(…)` etc. — match the identifier after the
last `.`): `Info`, `Warn`, `Error`, `Debug`, `Log`, `InfoContext`, `WarnContext`,
`ErrorContext`, `DebugContext`, `With`.

For the suffix match, the function field's text is split on `.` and the last segment is
matched; this avoids false suppression from non-logging selectors.

### 2.3 Integrate into `check()`

In `SyntaxRulesChecker::check()` for Go only (check `self.lang == Language::Go`):
```rust
// After walk_literals, before emit_literal_findings:
let slog_keys = if self.lang == Language::Go {
    walk_slog_keys(tree.root_node(), src)
} else {
    HashSet::new()
};
// Pass slog_keys into emit_literal_findings for filtering:
emit_literal_findings(&collector, &cfg, &excluded, &slog_keys, &mut findings);
```

Alternatively, `slog_keys` can be merged into `excluded` before passing (both are
`HashSet<usize>` node ID sets) — simpler, avoids a new parameter.

### 2.4 `argument_list` child indexing note

tree-sitter-go's `argument_list` nodes: `named_children()` returns only non-anonymous
children (no commas). Use `named_children()` count for the index — verified by the
existing `go_param_identifier_count` pattern in the same file.

### 2.5 Unit tests to add

All in the `syntax-rules` test module (or a new `slog_suppression_tests` submodule):

| Test name | Input | Expected |
|-----------|-------|----------|
| `suppresses_slog_key_string_in_log_warn` | `log.Warn("msg", "file", name, "err", err)` with `"file"` / `"err"` appearing 3+ times | 0 findings for those keys |
| `suppresses_slog_dot_warn_call` | `slog.Warn("msg", "key", val)` 3x | 0 findings |
| `does_not_suppress_slog_message_arg` | same file where the message string also repeats 3x | 1 finding for the message |
| `does_not_suppress_non_slog_string` | `"file"` repeated 3x in non-log call | 1 finding |
| `suppresses_logger_method_call` | `logger.Warn("msg", "key", v)` 3x | 0 findings |

## Backtest tasks

After implementation:
1. Run `kibitzer check native primitive-obsession {file}` against the sort/slices examples in `testdata/`.
2. Run `kibitzer check native syntax-rules {file}` against a Go file with slog patterns.
3. Run the checker against `kubernetes/kubernetes` (Go only) to confirm no new FP regressions.
4. Add entries to `docs/go-primitive-obsession-false-positives.md` for the two new fixed cases.
5. If `docs/syntax-rules-go-false-positives.md` is created, add the slog-key entry there.

## Order of implementation

These two changes are independent — either can land first. They touch different checkers
and different files, so they can be developed concurrently and landed as one PR or two.

A single PR is cleaner: both fix Go-stdlib-idiom false positives and can be described
together.

## Adversarial review

**Question A: Is the stdlib-callback allowlist the right mechanism?**

*Against:* The allowlist will grow stale as new stdlib packages add callback-taking
functions. It could miss user-defined functions with the same callback contract
(`mySort(s []T, less func(i, j int) bool)`).

*For:* An automatic detection (e.g. "skip any `func_literal` argument whose signature
matches exactly `func(int, int) bool`") would also suppress real primitive obsession
in user code with that exact shape. The allowlist is explicit, auditable, and
matches how the repo already handles similar suppressions (the `MAGIC_LITERAL_ALLOWLIST`
is also a hard-coded list). The stdlib list is stable; user code can use
`kibitzer:ignore` for their own cases.

**Decision:** Keep the explicit allowlist. Note in the false-positive doc that user
callback-wrappers can be suppressed with `kibitzer:ignore primitive-obsession`.

**Question B: Is skipping all `func_literal` parameters in `_test.go` too broad?**

*Against:* A test helper with genuinely confused argument types (e.g.
`process := func(id, name, role string)`) would be silently skipped.

*For:* The existing blank-identifier suppression already accepts false negatives for
"values that can't be confused." Test closures are internal and ephemeral. Prior art:
the `_test.go` boundary is used elsewhere in Go tooling (vet, staticcheck) to apply
different rules.

**Decision:** Accept. Gate on `func_literal` specifically (not all parameter lists in
`_test.go`) so `func TestXxx(t *testing.T)` and actual exported test helpers are
unaffected. Add a doc note in `docs/go-primitive-obsession-false-positives.md`.

**Question C: Slog method-name suffix match — could it suppress non-logging code?**

*Against:* Any method named `Warn`, `Error`, etc. that takes alternating key/value
pairs would have its key strings suppressed.

*For:* The methods named `Warn`/`Error`/`Info`/`Debug` that *also* take slog-style
alternating arguments are overwhelmingly logging calls. A non-logging `Warn` that
takes key/value pairs would itself be following a slog-like convention. False-negative
cost is low (one un-fired magic-literal finding per non-logging method named `Warn`).

**Decision:** Accept the suffix match. The exact-match list (`slog.Warn`, `log.Warn`)
covers the stdlib and the most common wrapper packages; the suffix-based fallback
covers custom loggers.
