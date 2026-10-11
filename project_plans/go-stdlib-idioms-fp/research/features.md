# Feature landscape: go-stdlib-idioms-fp

## 1. Existing false-positive suppression patterns

`docs/go-primitive-obsession-false-positives.md` catalogs four fixed FP categories for
`go-primitive-obsession`, all now part of the checker source (`src/checkers/primitive_obsession.rs`):

| Suppression | Mechanism | Source location |
|---|---|---|
| Named return-value list | `is_named_return_list` — skips `parameter_list` nodes reached via parent's `result` field | `primitive_obsession.rs:83` |
| Blank identifier `_` | `describe_param` counts `real_name_count` (blanks excluded); case (a) fires on `real_name_count >= 2` | `primitive_obsession.rs:161` |
| Deletion-only edit | `compute_changed_lines` returns `Some(Vec::new())` rather than `None` so the scoper suppresses everything | `hook.rs` (fixed 2026-08-10) |
| Ambiguous substring | `compute_changed_lines` unions all occurrences of a repeated `new_string` | `hook.rs` (fixed 2026-08-18) |

For `replace-magic-literal` (`src/checkers/rules.rs`), existing suppression mechanisms are:

| Suppression | Mechanism |
|---|---|
| Named constants | `resolve_excluded_constants` — any literal bound to a constant referenced ≥2 times in the file is excluded |
| Generated files | `is_generated` header check, applied before `walk_literals` |
| Universal values | `MAGIC_LITERAL_ALLOWLIST` — `"0"`, `"1"`, `"-1"`, `""` never flagged regardless of repetition |
| Threshold | `MAGIC_LITERAL_MIN_OCCURRENCES = 3` — a value appearing ≤2 times is never flagged |

Neither checker has any call-context or file-type suppression. That is the gap both issues exploit.

## 2. stdlib callback patterns that might trigger `go-primitive-obsession`

Issue 1 targets `sort.Slice(s, func(i, j int) bool { ... })`. The checker fires because the
`func_literal`'s `parameter_list` — `(i, j int)` — is two identifiers sharing one primitive
type (case a).

Other stdlib functions that pass comparator or predicate callbacks:

| Function | Callback signature | Fires? |
|---|---|---|
| `sort.Slice(s, func(i, j int) bool)` | `func(i, j int) bool` | **Yes** — case (a): `i, j int` |
| `sort.SliceStable(s, func(i, j int) bool)` | same | **Yes** |
| `slices.SortFunc(s, func(a, b T) int)` | `func(a, b T) int` where T is the element type | Only if T is a primitive (`string`, `int`, etc.) — e.g. `slices.SortFunc(ss, func(a, b string) int)` fires case (a) |
| `slices.SortStableFunc(s, func(a, b T) int)` | same as above | Same condition |
| `slices.BinarySearchFunc(s, t, func(a, b T) int)` | same | Same condition |
| `slices.IndexFunc(s, func(T) bool)` | `func(T) bool` — one parameter | Does **not** fire (single parameter) |
| `slices.ContainsFunc`, `slices.DeleteFunc`, `slices.CompactFunc`, etc. | `func(T) bool` or `func(T, T) bool` | `func(T, T) bool` fires when T is primitive |
| `maps.DeleteFunc(m, func(K, V) bool)` | `func(K, V) bool` with K, V being the map's key/value types | Fires when K and V are the same primitive (rare in practice) |
| `strings.IndexFunc(s, func(rune) bool)` | one-parameter `func(rune) bool` | Does not fire |
| `strings.Map(func(rune) rune, s)` | `func(rune) rune` — one parameter | Does not fire |
| `filepath.Walk(root, func(path string, info fs.FileInfo, err error) error)` | mixed types | Does not fire (distinct types) |
| `http.HandleFunc(pattern, func(w http.ResponseWriter, r *http.Request))` | mixed types | Does not fire |

Summary: the primary firing patterns for stdlib callbacks are `func(i, j int) bool` (sort),
`func(a, b string) int` / `func(a, b T) int` (slices comparators with primitive element
types), and `func(a, b T) bool` patterns where both parameters share a primitive type.

The suppression approach in `research.md` — detecting a `func_literal` that is a direct
argument of a `call_expression` and checking the callee against an allowlist — covers
`sort.Slice`/`sort.SliceStable` directly. For `slices.*` with primitive element types, the
same AST pattern applies (the `func_literal` is still a `call_expression` argument), so the
same allowlist mechanism suppresses those too when the callee name is added to the list.

## 3. slog-compatible logging library signatures

Go's `log/slog` package (stdlib since Go 1.21) and the common pre-1.21 structured-logging
libraries all use the same alternating key-value convention after a message argument.

**Stdlib `log/slog`** (`golang.org/x/exp/slog` before 1.21):
```go
slog.Info(msg string, args ...any)
slog.Warn(msg string, args ...any)
slog.Error(msg string, args ...any)
slog.Debug(msg string, args ...any)
slog.Log(ctx context.Context, level Level, msg string, args ...any)
slog.InfoContext(ctx, msg, args...)   // same pattern, context-aware variants
slog.WarnContext / slog.ErrorContext / slog.DebugContext
slog.With(args ...any) *Logger
// On a *slog.Logger:
logger.Info(msg, args...)  // same shape
```

**`log` package wrappers** — common in Go codebases (including `stapler-squad`) that wrap
stdlib `log` or a third-party logger under a `log` package alias:
```go
log.Info(msg, keyvals...)
log.Warn(msg, keyvals...)
log.Error(msg, keyvals...)
log.Debug(msg, keyvals...)
log.InfoContext(ctx, msg, keyvals...)  // etc.
```

**Other structured-logging libraries** with the same key-value convention:
- `go.uber.org/zap` `SugaredLogger`: `sugar.Infow(msg, key, val, ...)` — method names end in `w` (for "with fields"); the plain `sugar.Info(args...)` is printf-style, not key-value. The `Warnw`, `Errorw`, `Debugw` pattern is distinct from the slog shape.
- `github.com/hashicorp/go-hclog`: `logger.Info(msg, key, val, ...)` — same alternating shape, method names `Info`/`Warn`/`Error`/`Debug`.
- `github.com/go-logr/logr`: `log.Info(msg, key, val, ...)` — same shape; used by `controller-runtime` / Kubernetes controllers.
- `github.com/rs/zerolog`: uses builder pattern (`log.Warn().Str("key", val).Msg(msg)`) — no alternating string keys, so not affected.
- `github.com/sirupsen/logrus`: uses structured fields via `logrus.WithFields(logrus.Fields{...}).Warn(msg)` — no alternating string keys in the logging call itself, not affected.

For suppression purposes, the key observation is: **`log/slog`, `go-logr/logr`, and
`go-hclog` all share the same method-name suffix set** (`Info`, `Warn`, `Error`, `Debug`,
`Log`, `InfoContext`, `WarnContext`, `ErrorContext`, `DebugContext`, `With`) and the same
alternating position convention (odd-indexed args ≥1 are keys). Matching on method-name
suffix covers all three without requiring package import resolution.

## 4. Existing tests that need updating

**`go-primitive-obsession` tests** are in `src/checkers/primitive_obsession.rs`'s `tests`
module (lines 183–305). No test currently covers a `func_literal` passed as a call
argument, nor any `_test.go` file-path filtering. New tests to add:

- `allows_sort_slice_comparator` — `sort.Slice(s, func(i, j int) bool { return i < j })` → zero findings
- `allows_slices_sort_func_string_comparator` — `slices.SortFunc(ss, func(a, b string) int)` → zero findings
- `still_flags_sort_slice_if_not_in_call_arg` — a standalone `func(i, j int) bool {}` (not in a call) → flags
- `allows_func_literal_in_test_file` — same `func(name, body string)` shape in a `_test.go` path → zero findings

**`replace-magic-literal` tests** are in `src/checkers/rules.rs`'s `tests` module
(functions starting at line 2152). No test covers the slog key-position exclusion. New
tests to add:

- `go_allows_slog_key_string_in_odd_position` — `slog.Warn("msg", "file", name, "err", err)` with `"file"` and `"err"` each appearing ≥3 times → zero findings for those keys
- `go_still_flags_slog_value_in_even_position` — a repeated string at index 2 (value position) → still flags
- `go_allows_log_warn_key` — `log.Warn("msg", "file", f, "file", g, "file", h)` — `"file"` at odd positions, 3 occurrences → zero findings
- `go_still_flags_non_slog_repeated_string` — `"file"` repeated 3 times in non-logging calls → still flags
