# `introduce-special-case` check — known false positives and tuning

`introduce-special-case` (`src/checkers/introduce_special_case.rs`) flags a Go type whose
values are nil-checked in the same `if x == nil { ... } else { x.M() }` shape (or the `!=`
form) 3+ times across 2+ files — a candidate for Fowler's *Introduce Special Case* (a
null-object implementation of the type). It runs by default for `**/*.go` at Advisory
severity. It surfaces candidates only; designing the null object is a human/LLM step.

It counts an `if` as an occurrence when the checked identifier's type resolves
syntactically (a parameter, a `var` declaration, or `x := &T{}`) and either an `else`
branch or a method call `x.M(...)` is present. A bare guard clause
(`if x == nil { return }`) is not counted. Declarations are scoped like Go: visible from
their statement to the end of their block, and an untyped redeclaration such as
`a, err := f()` clears any type the name had. A method's receiver is never typed. State
lives in a per-repo JSON index under `special-case-index/` next to the kibitzer cache,
redirectable with `KIBITZER_SPECIAL_CASE_INDEX_DIR`.

## Known false-positive patterns

| Pattern | Why it fires | What the check does |
|---|---|---|
| `error` | Nil-checking an `error` is idiomatic Go; a null-object error is wrong. | Type excluded. |
| `context.Context` | Same: nil-checks on contexts are convention. | Type excluded. |
| Primitives (`string`, `int`, `bool`, ...) and `any` | A nil check on these says nothing about a missing domain object. | Types excluded. |
| Test files (`*_test.go`, and files under `test/`, `tests/`, `testing/`, `spec/`) | Tests repeat nil-checks on fixtures by design. | Skipped by default (the shared `checkers::is_test_file`, applied to the path below the repo root); set `exclude_tests: false` to include them. |
| `vendor/` (top-level or nested) | Third-party code you don't own. | Excluded from the default scope (`!**/vendor/**`). |
| Field read, not a call (`if x != nil { y = x.Name }`) | Not a use of the type's behavior. | Not counted unless the `if` has an `else`. |
| Nil-safe receiver (`func (f *T) Set() { if f != nil { ... } }`) | The type guards itself; there are no callers to simplify. | The receiver is not typed, so it is not counted. |
| Shadowed name (`a, err := f()` after an outer typed `err`) | A typed name must not survive a redeclaration. | Scoped per block; an untyped `:=` clears the name. |
| Generated files | Generators emit repetitive nil-checks. | Skipped when the file is detected as generated. |
| Same code forked across versioned directories | One logical site copied into several packages counts as several occurrences (seen: the three `allocator_*.go` copies in kubernetes). | Not filtered. |
| Nil as a domain or API sentinel (`prev == nil` = "first registration", `into != nil` = "decode into this object") | The nil-check is the meaning, so there is no null object to introduce. | Not filtered; seen on `v1.Pod` and `runtime.Object`. |
| Types the repo does not own (`time.Timer`, `zap.Logger`, `os.File`) | A null object cannot be added to a stdlib or third-party type. | Not filtered outside `vendor/`; suppress with `kibitzer:ignore`. |
| `else` branch that never uses `x` (`if class != nil { log(class) } else { log(claim) }`) | The `else` alone qualifies the `if`. | Not filtered. |
| Same type name in different packages | Types are keyed by their written name (`Config`), so unrelated `Config` types add up. | Not filtered; known over-count. |
| Parenthesized condition (`if (x == nil)`) | Not unwrapped. | Missed; known under-count. |
| Syntactic type resolution | Types introduced by `:=` from a function call or a field access are not resolved, so those checks are skipped. | Unresolved identifiers are ignored; under-counts. |

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

"Before" is zero: the checker is new, and the only edits to existing code (shared
index-path and atomic-save helpers in `duplicate_cross_file_checker.rs`, per-checker
scratch directories in `backtest.rs`) are behavior-preserving, covered by the existing
duplicate-code tests.

**Transcript backtest**, `kibitzer check backtest introduce-special-case` (final build):
4805 transcripts, 97 Go snapshots checked, 0 findings. The whole run shares one scratch
index, so the threshold is reachable in principle; 97 snapshots of edited files is a small
sample, so this shows silence on that sample, not recall.

**Corpus runs.** Per repo: list the in-scope files, then run the checker on each against a
fresh index.

```sh
export KIBITZER_SPECIAL_CASE_INDEX_DIR=$(mktemp -d)
cd ~/code/github.com/kubernetes/kubernetes   # or tstapler/stapler-squad
git ls-files '*.go' | grep -vE '(^|/)vendor/' | grep -v '_test\.go$' > files
while read -r f; do kibitzer check native introduce-special-case "$PWD/$f"; done < files
```

The first run listed every `.go` file (vendor included); later runs apply the default
scope. Three builds, same repos, same checkout:

| Build | kubernetes: files → findings (types) | stapler-squad: files → findings (types) |
|---|---|---|
| 1. first implementation (flat per-function declarations, any `x.Field` counts, no vendor or `test/` scope) | 14361 → 124 (30) | 746 → 3 (2) |
| 2. `2eaf6cd` (+ vendor scope, `test/` dirs treated as tests) | 9975 → 46 (13) | 746 → 3 (2) |
| 3. `96e8a8b` (+ block scoping, method calls only, receiver untyped); `c899421` (+ range/switch/variadic shadowing) produces identical output | 9975 → 9 (4) | 746 → 3 (2) |

What removed findings, by type, between builds 2 and 3 (every build-2 finding was read in
source): `v1.Pod` 11 → 1 and `v1.Node` 6 → 0 and `api.Pod` 1, `api.PodTemplateSpec` 3,
`volume.Spec` 1, `v1.NodeSelector` 1, `metainternalversion.ListOptions` 2 → 0 and
`resourceapi.DeviceClass` 9 → 1 were mostly field reads (`node.Spec.Taints`,
`pod.UID`); `FieldsV1` 2 → 0 were nil-safe receiver guards in two build-tag variants of the
same setters; `apierrors.StatusError` 1 → 0 was an `error` variable mistyped by a later
nested `var err *StatusError` (`policy_dispatcher.go:141`, `:155`). Between builds 1 and 2,
the 64 vendor findings (including all 22 `zap.Logger`) were not individually read; that
code is now out of scope. One recall cost of the change: `leadermigration.LeaderMigrator`
(2 findings in build 2) is a genuine optional-feature guard, but it reads fields and a
channel rather than calling methods, so it no longer fires.

**Verdicts for every build-3 finding.** One reader's judgement from the source, not
independently confirmed. "Candidate" means the shape is right and a null object is a
plausible design; whether to do it is a human call.

| Repo | Type (findings) | Verdict | Why |
|---|---|---|---|
| kubernetes | `egressselector.EgressSelector` (4) | Candidate | `if egressSelector != nil { egressSelector.Lookup(...) }` repeated in 5 files (`config.go:265`, `peer.go:75`, `authentication.go:737`, `tracing.go:101`, `webhook/authentication.go:62`, `:85`) |
| kubernetes | `resourceapi.DeviceClass` (1) | False positive | three versioned copies of one function (`allocator_{experimental,incubating,stable}.go`), not three sites |
| kubernetes | `v1.Pod` (1) | False positive | `prev == nil` is "first registration" (`cache_based_manager.go:234`); the other two are unrelated sites |
| kubernetes | `runtime.Object` (3) | False positive | `into`/`obj == nil` selects "decode into" vs "allocate" (`json.go:165`, `versioning.go:133`, `helpers.go:378`); `fake.go:78` and `etcd3/store.go:501` are one-off guards |
| stapler-squad | `session.Instance` (1) | Candidate | `if inst != nil { inst.M() }` at `tools_goal.go:160`, `:170` and `review_queue_manager.go:409` |
| stapler-squad | `time.Timer` (2) | False positive | stdlib type; `nil` means "not created yet" for four lazily created timers (`claude_settings_watcher.go:230`, `tmux_unix.go:53`, `mmapwatch.go:76`, `:89`) |

So build 3 reports 12 findings: 5 candidates and 7 false positives by this reading. The
false positives are the patterns in the table above that the checker does not filter
(versioned forks, nil sentinels, stdlib types).
