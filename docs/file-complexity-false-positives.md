# `file-complexity` check — known false positives

Tracks confirmed false-positive firings of the `file-complexity` kibitzer check
(`src/complexity.rs`), wired up via `kibitzer check native file-complexity
{file}`. Check new occurrences against this list before re-investigating a
firing from scratch.

## Mechanism (confirmed by reading the source)

The check counts decision points (`if`/`for`/`case`/`&&`/`||`) per function via
`count_decision_points`, recursing into every descendant node unconditionally —
including `func_literal` bodies, so a closure's branching is deliberately
summed into its enclosing function's complexity (by design, per the checker's
own doc comment and test coverage). Unlike `src/file_size.rs` (`is_generated`)
and `src/arch_model.rs` (`looks_generated`), `complexity.rs` has no check for
the `// Code generated ... DO NOT EDIT.` marker.

## Fixed

### 2026-09-12 — kubernetes/kubernetes corpus backtest — no generated-file exclusion

- **Symptom**: `staging/src/k8s.io/api/scheduling/v1alpha3/zz_generated.validations.go` (kubernetes/kubernetes, a
  giant generated validation-gen switch statement) flagged — genuinely branchy by any McCabe count, but nobody
  can or would refactor generated code to reduce its complexity. A sibling instance,
  `staging/src/k8s.io/api/scheduling/v1beta1/generated.pb.go` (protoc-gen-gogo), showed the same gap.
- **Mechanism**: `complexity.rs` never called `file_size::is_generated`/`arch_model::looks_generated` the way
  sibling checks do — no generated-code exclusion at all.
- **Fixed by**: an early return via `file_size::is_generated` (reused). Verified directly: both cited files now
  produce no findings, while a genuine true positive (`cmd/kubelet/app/server.go`, kubernetes/kubernetes, still
  flags 5 functions) is unaffected.

### 2026-09-12 — stapler-squad corpus backtest — independent `t.Run` subtests summed into one complexity figure

- **Symptom**: `session/vc/git_provider_test.go` (stapler-squad) — a `TestXxx` function containing several
  hand-written `t.Run(name, func(t *testing.T) {...})` calls, each an independent test case, had their branching
  summed into one figure. `TestGitProviderGetChangedFiles` reported complexity 33 from 5 independent subtests'
  ~32 total `if` statements.
- **Mechanism**: `count_decision_points` recursed into every descendant node unconditionally, including
  `func_literal` bodies — correct for an ordinary closure whose branching affects the enclosing function's
  control flow, but Go's idiomatic multi-`t.Run` subtest pattern isn't that: each closure is its own independent
  test case, not entangled control flow.
- **Fixed by**: `is_run_subtest_closure` detects a `func_literal` whose parent is an `argument_list` whose parent
  is a `call_expression` with a `selector_expression` callee field named `Run` (the `t.Run`/`b.Run`/`m.Run`
  convention), and excludes its contribution from the enclosing function's sum. Scoped narrowly, beyond the
  minimum ask, to bound false-negative risk: the exclusion only applies when the file is `_test.go` **and** the
  enclosing function's name starts with `Test`/`Benchmark`/`Fuzz` — a `.Run(closure)` in production code, or in a
  test file's non-`Test`-named helper, is never affected and still sums normally. Regression-guarded by unit- and
  end-to-end-level tests plus two true-positive guards (`.Run(closure)` outside a test file; inside a test file
  but in a non-`Test`-named function) in `src/complexity.rs`'s test module. Verified directly:
  `TestGitProviderGetChangedFiles` drops from 33 to 1, `TestGitProviderGetBranch` from 17 to 1, while
  `cmd/kubelet/app/server.go`'s 5 true positives are unaffected.

## Log

No open entries.
