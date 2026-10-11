# Pitfalls: go-stdlib-idioms-fp fixes

Covers Fix 1 (`primitive_obsession.rs` — skip `parameter_list` in stdlib
callback `func_literal`s and `_test.go` files) and Fix 2 (`rules.rs` — skip
`replace-magic-literal` for slog/log key arguments at odd positions).

---

## 1. `named_children` skips commas — argument position ≠ child index only when `children()` is used

**Risk level: HIGH for Fix 2.**

In tree-sitter, `named_children()` filters out anonymous nodes (commas, parens,
`...`). For `argument_list` in tree-sitter-go:

```
argument_list: ( expr , expr , expr )
children():        0   1   2   3   4   5   6   ← includes parens and commas
named_children():  0       1       2           ← only the argument expressions
```

`named_children()` gives you the zero-based argument position directly — index 0
is the first argument, index 1 is the second, and so on. The "odd = key" rule for
slog (`slog.Info(msg, key1, val1, key2, val2, ...)`) means:
- index 0 → `msg` (exempt from the key/value alternation — never a repeated-string key)
- index 1, 3, 5, … → keys (the positions to suppress)
- index 2, 4, 6, … → values (should still be flagged)

Using `children()` instead of `named_children()` shifts every real argument by at
least one due to the opening paren, making odd-index key detection wrong by 1. The
existing codebase uses `named_children()` everywhere (confirmed by grepping
`rules.rs`), so this is only a risk if the new code deviates. Stick with
`named_children()`.

---

## 2. `slog.With()` has no leading message argument — the odd-index rule breaks it

**Risk level: HIGH for Fix 2.**

The slog functions split into two calling conventions:

| Function | Signature | Index 0 |
|---|---|---|
| `slog.Info`, `slog.Warn`, `slog.Error`, `slog.Debug` | `(msg string, args ...any)` | message string — *not* a key |
| `slog.With` | `(args ...any)` | a key — *is* subject to odd-index rule |
| `slog.Log` | `(ctx, level, msg string, args ...any)` | `ctx` — non-string |
| `logger.With` (method) | `(args ...any)` | a key |
| `logger.Info` (method) | `(msg string, args ...any)` | message |

Applying a blanket "suppress odd indices" rule to all slog calls silently
misses the key at index 0 for `slog.With` / `logger.With`. The fix must either:

(a) Not add `slog.With` to the allowlist (accept the false positive), OR  
(b) Track per-function "key-index offset" — 0 for `With`-family, 1 for
`Info`/`Warn`/`Error`/`Debug`-family.

Option (a) is safer to ship first. Option (b) requires a per-name dispatch table
and is more complex.

---

## 3. Parent-traversal lifetimes in tree-sitter: two levels is safe, but borrow rules bite

**Risk level: MEDIUM for Fix 1.**

`Node<'_>` in tree-sitter 0.20+ carries a lifetime tied to the `Tree` (not the
cursor). `node.parent()` returns `Option<Node<'_>>` with the *same* lifetime as
`node` — it does not borrow from an intermediate. Traversing two levels up
(`node.parent()?.parent()`) is therefore safe as long as both nodes are used
within the same borrow of the tree, which is always true in a `check_parameter_list`
call (the tree is alive for the checker's entire run).

The real risk is a match-arm that borrows from a temporary:

```rust
// WRONG — `parent` is a temporary, inner borrow doesn't outlive the let binding
let kind = node.parent().map(|p| p.parent()).flatten().map(|gp| GoKind::of(gp));
```

The safe pattern already used in `is_named_return_list` (`src/checkers/primitive_obsession.rs:83`):

```rust
node.parent()
    .and_then(|parent| parent.child_by_field_name("result"))
    .is_some_and(|result| result == node)
```

Chaining `.and_then()` keeps each intermediate node alive within its closure.
Assign to a named local only when you need to inspect the intermediate node at
multiple sites; otherwise chain.

At the tree root, `parent()` returns `None` — the chain short-circuits cleanly.
No explicit root check is needed.

---

## 4. `_test.go` boundary is file-suffix only — correct, but must avoid path false negatives

**Risk level: LOW for Fix 1.**

In Go, test files must be named `*_test.go` (the `go` tool enforces this). The
suffix `_test.go` is the *only* distinguishing marker the toolchain uses — there
is no separate directory convention like `src/test/`. External test packages
(`package foo_test`) live in the same directory and same `*_test.go` files, so a
file-path suffix check covers all Go test code.

The pitfall is in how the file path reaches the checker. If kibitzer normalizes
paths (strips a prefix, converts separators), verify that the suffix check sees
the final file name, not a truncated version. The existing checker infrastructure
passes the raw file path (confirmed in `check_go_source` test harness) — a suffix
check on the raw path is reliable.

One edge case: generated test helpers with names like `testing_helpers.go`
(no `_test` suffix) are *not* test files by Go's definition and should not be
suppressed by this guard.

---

## 5. Stdlib callback allowlist silently misses user-defined wrappers — expected false-negative category

**Risk level: LOW (accepted limitation), document it.**

A hard-coded allowlist (`sort.Slice`, `slices.SortFunc`, `slices.SortStableFunc`,
`sort.SliceStable`, `sort.Search`, `strings.IndexFunc`, `strings.Map`, etc.) will
never fire on:

```go
func mySorted(s []int, less func(i, j int) bool) { sort.Slice(s, less) }
// usage:
mySorted(items, func(i, j int) bool { return items[i] < items[j] })
```

This is an inherent false-negative of the name-only approach. The alternative —
full type-inference to detect any `func(i, j int) bool` parameter — is out of
scope for a tree-sitter checker. Document the known-miss category in a comment
near the allowlist; do not try to close it without a type-system integration.

---

## 6. Interaction with existing suppressions in `primitive_obsession.rs`

**Risk level: LOW.**

The existing suppression (`is_named_return_list`) operates at the
`parameter_list` level: it returns early before `check_parameter_list` runs.
The proposed Fix 1 also guards at the `parameter_list` level (skip when the list
belongs to a `func_literal` that is a callback argument, or when the file is
`_test.go`). These guards are additive and do not overlap:

- Named-return guard: triggered when `parameter_list` is the `result` field of a
  signature node.
- Callback guard: triggered when the `func_literal` parent is an argument inside
  a known stdlib call.
- Test-file guard: triggered by file path.

No case triggers two guards simultaneously on the same node (a named-return list
inside a stdlib callback inside a test file would pass all three, but that is
fine — the first matching guard short-circuits the rest). No regression risk
here.
