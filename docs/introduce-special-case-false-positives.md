# `introduce-special-case` check — known false positives and tuning

`introduce-special-case` (`src/checkers/introduce_special_case.rs`) flags a Go type whose
values are nil-checked in the same `if x == nil { ... } else { x.M() }` shape (or the `!=`
form) 3+ times across 2+ files — a candidate for Fowler's *Introduce Special Case* (a
null-object implementation of the type). It runs by default for `**/*.go` at Advisory
severity. It surfaces candidates only; designing the null object is a human/LLM step.

It counts an `if` as an occurrence when the checked identifier's type resolves
syntactically (parameter, receiver, `var` declaration, or `x := &T{}`) and either an `else`
branch or a method call on `x` is present. A bare guard clause (`if x == nil { return }`)
is not counted. State lives in a per-repo JSON index under `special-case-index/` next to
the kibitzer cache, redirectable with `KIBITZER_SPECIAL_CASE_INDEX_DIR`.

## Known false-positive patterns

| Pattern | Why it fires | What the check does |
|---|---|---|
| `error` | Nil-checking an `error` is idiomatic Go; a null-object error is wrong. | Type excluded. |
| `context.Context` | Same: nil-checks on contexts are convention. | Type excluded. |
| Primitives (`string`, `int`, `bool`, ...) and `any` | A nil check on these says nothing about a missing domain object. | Types excluded. |
| Test files (`*_test.go`, and files under `test/`, `tests/`, `testing/`, `spec/`) | Tests repeat nil-checks on fixtures by design. | Skipped by default (the shared `checkers::is_test_file`, applied to the path below the repo root); set `exclude_tests: false` to include them. |
| `vendor/` (top-level or nested) | Third-party code you don't own. | Excluded from the default scope (`!**/vendor/**`). |
| Field access, not a call (`if x != nil { y = x.Name }`) | A selector on the checked identifier counts as a use, whether or not it is a method call. | Not filtered; known over-count. |
| Same type name in different packages | Types are keyed by their written name (`Config`), so unrelated `Config` types add up. | Not filtered; known over-count. |
| Parenthesized condition (`if (x == nil)`) | Not unwrapped. | Missed; known under-count. |
| Generated files | Generators emit repetitive nil-checks. | Skipped when the file is detected as generated. |
| Optional-field types (protobuf messages, config structs) | `nil` legitimately means "absent" and callers differ in what they do about it. | Not detected; suppress or raise the threshold. |
| Syntactic type resolution | Types introduced by `:=` from a function call or a field access are not resolved, so those checks are skipped; this under-counts rather than over-counts. | Unresolved identifiers are ignored. |

## Suppression

Add a directive above the flagged line, with a reason:

```go
// kibitzer:ignore introduce-special-case -- nil means "no override configured" here
if cfg == nil {
```

The directive silences that finding only. See `docs/suppressing-checks.md` for scoping or
disabling the check for a path or repo.

## Configuration

In `.kibitzer/inspect.json`:

```json
{ "checks": [ { "name": "introduce-special-case", "options": { "min_occurrences": 5 } } ] }
```

- `min_occurrences` (default `3`): occurrences per type needed to fire. The type must also
  appear in at least 2 distinct files regardless. Values below 2, or a non-integer, make
  `configure()` return an error.
- `exclude_tests` (default `true`): skip test files — `*_test.go` and anything under a
  `test/`, `tests/`, `testing/` or `spec/` directory (below the repo root).

## Backtest results

"Before" is zero everywhere: the checker is new, and the only edits to existing code
(shared index-path and atomic-save helpers in `duplicate_cross_file_checker.rs`, an
index-dir env list in `backtest.rs`) are behavior-preserving, covered by the existing
duplicate-code tests.

**Transcript backtest** (`kibitzer check backtest introduce-special-case`): 4799
transcripts, 97 Go snapshots checked, 0 findings. The whole run shares one scratch index,
so the threshold is reachable in principle; 97 snapshots of edited files is a small sample,
so this shows silence on that sample, not recall.

**Corpus run**, one `kibitzer check native introduce-special-case` per non-test `.go` file
from `git ls-files`, fresh index (debug build). This run predates two later changes — the
default `!vendor/**` scope and the broader `is_test_file` — so its vendored findings and
any in `test/` directories no longer appear in a default run; the counts below were not
re-run:

| Repo | Files | Findings | Types | Files with findings |
|---|---|---|---|---|
| kubernetes/kubernetes | 14361 | 124 | 30 | 80 |
| tstapler/stapler-squad | 746 | 3 | 2 | 2 |

- **kubernetes, vendored vs. first-party:** 64 findings (38 files) are under `vendor/`
  (4386 of the 14361 files), including all 22 `zap.Logger` findings (10 files); 60 (42 files) are first-party. First-party types are almost all
  Kubernetes API objects: `v1.Pod` 16, `v1.Node` 11, `resourceapi.DeviceClass` 9,
  `runtime.Object` 4, `egressselector.EgressSelector` 4, and 10 more types with 1-3 each.
- **stapler-squad:** `session.Instance` (3 occurrences, 2 files) and `time.Timer` (4, 3 files).

**Triage is a sample, not a full review.** Four kubernetes findings were read in source
(`zap.Logger`, `v1.Pod`, `resourceapi.DeviceClass`, `netlink.Link`): all four match the
checked shape, and all are an `if x != nil { x.M() }` guard with no `else` — the form the
checker counts deliberately. None was labelled a true or false positive. The remaining
findings are unreviewed.

Patterns this run shows that the table above does not yet filter:

- **Types the repo does not own** (stdlib, vendored dependencies, e.g. `time.Timer`,
  `zap.Logger`, `os.File`, `http.Request`): a null object cannot be added to them.
  Vendored ones are now out of the default scope; stdlib types such as `time.Timer` are
  still reported. Suppress with `kibitzer:ignore` (see `docs/suppressing-checks.md`).
- **API-object types where `nil` means "absent"** (`v1.Pod`, `v1.Node`): the nil-check is
  often the domain meaning, so whether a special case fits is a design call.
- **Logger-style optional dependencies** (`if lg != nil { lg.Warn(...) }`): the most
  common shape and the classic null-object use, but the checker cannot tell a worthwhile
  one from a one-off.
