# test-no-assertion-go on kubernetes/kubernetes: triage summary

Repo commit `0ba07e0866bb1e3d67464b2d3e01f6bde6fe9963`. Verdicts live in
`test-no-assertion-go.jsonl` (47 records, one per finding; counts below
derived with `jq` over that shard).

## Counts

| Verdict | Count |
|---|---|
| true_positive, smoke-only (no-panic / race-detector / timeout oracle) | 21 |
| true_positive, real (empty disabled stubs, `cacher_test.go`) | 5 |
| false_positive | 21 |
| **Total** | **47** |

Precision = TP / (TP + FP) = 26 / 47 = **55.3%** (strict "real" TP only: 5 / 47 = 10.6%).

`scripts/backtest-triage.py list --repo-slug kubernetes-kubernetes --verdict false_positive`
reports 21 `test-no-assertion-go` rows, matching the shard (the script has no
`report`/`summary` subcommand; only `run`, `mark`, `list`).

## False-positive causes (21)

| Cause | Count | Tests |
|---|---|---|
| Assertion in a method on a receiver/returned struct (`env.expectPath`, chained `tester.when...().expect()` -> `doExpect` -> `s.t.Errorf`) | 13 | openapi controller_test x5, trigger_time_tracker_test x8 |
| Generic helper called with explicit instantiation, `helper[T](t)` | 5 | `TestMonotonic`, `TestMinimum`, `TestMaximum`, `TestRoundTripJSONCBOR*` x2 |
| `t.Run(name, tc.runner)` method value (assertions inside `runner`) | 2 | `TestPatch` in client-go metadata/dynamic fake |
| Cross-file helper (`assertFailures` in `validation_common_test.go`) inside a `t.Run` closure | 1 | `TestValidateResourceSlice` |

## True positives worth noting

- 5 real: `TestListInconsistentContinuation`, `TestListResourceVersionMatch`, `TestGuaranteedUpdate`, `TestGuaranteedUpdateChecksStoredData`, `TestTransformationFailure` in `cacher_test.go` have empty bodies (`TODO(#109831)`). They pass vacuously.
- 21 smoke-only: oracles are no-panic (e.g. `TestContextNoPanic`), `-race` (`TestCounterConcurrentWithContextRace`), or go-test timeout/hang (`TestPollUntil`, workqueue drain tests, `TestEventProcessorExit`). `TestTimeout` in `ktesting/examples` is a deliberate demo that blocks 1000h.

## Suggested rule improvements (rule code not changed)

1. Resolve calls through receiver-method helpers: when the test body calls `x.m(...)` or a chain `x.m(...).n(...)` and a same-file method `m`/`n` (on any receiver type) contains `t.Error*/Fatal*/Fail*` (including via a stored `t` field like `t.t.Errorf`, `s.t.Errorf`), count it. Covers 13 of 21 FPs.
2. Handle explicit generic instantiation (`helper[int](t)`, `helper[T](t)`) as a call to `helper` in the same-file helper lookup. Covers 5.
3. Treat method values passed to `t.Run` (`t.Run(name, tc.runner)`) like a closure: analyze the named method body. Covers 2.
4. Look up helpers declared in other files of the same package (`*_test.go` siblings), at least those taking `testing.TB`/`*testing.T` and calling `Errorf/Fatalf/Error/Fatal`; also recognise helpers that forward `t` to a known assertion-bearing API (`field.ErrorMatcher.Test(t, ...)`, `assertFailures(t, ...)`). Covers 1 here, more on repos with shared helper files.
5. Add a lower-severity / separate category for the smoke-only pattern: bodies with no assertion whose name matches `NoPanic|NoCrash|DoesNotPanic|Race|Smoke`, or whose doc comment says "must not panic"/"smoke"/"expected to timeout", so these are not reported as defects. Would reclassify most of the 21 smoke-only TPs.
6. Split out the empty-body stub (`func TestX(t) { /* TODO */ }`) as its own higher-confidence finding; it is the only class here a reviewer would call a defect (5 of 5 real TPs).

## After improvements

Improvements 1-3 are implemented in `src/checkers/test_no_assertion.rs`; the shard above is left unchanged as the
pre-change history (`scripts/backtest-triage.py mark` has no "fixed" verdict). Re-run on the same commit
(`0ba07e08`) with a debug build over every `*_test.go` (`find` + `xargs -P8`), compared by `file:line`
against the shard with `jq`/`comm`:

| | Before | After |
|---|---|---|
| Findings | 47 | 27 |
| true_positive records still reported | 26 of 26 | 26 of 26 |
| false_positive records still reported | 21 | 1 |
| Findings not in the shard | 0 | 0 |

Precision = 26 / 27 = **96.3%** (was 55.3%). Strict "real" TP only: 5 / 27 = 18.5%.

- Fixed (20 of 21): receiver and chained method helpers (13, including a `// comment` line between chain links),
  explicit generic instantiation `helper[T](t)` (5; tree-sitter-go parses the one-argument form as a
  type-conversion expression, so it was never seen as a call), and method values passed to `t.Run` (2).
- Remaining (1): `TestValidateResourceSlice` (`validation_resourceslice_test.go:210`) calls `assertFailures`, declared in a
  sibling file (`validation_common_test.go`). The checker parses one file per run, so cross-file lookup is not done;
  list the helper under the `assertion_helpers` option (tested by
  `test_no_assertion_should_ReportZero_When_CrossFileHelperListedInAssertionHelpers`).
- Not attempted: improvements 5 and 6 (separate smoke-only / empty-stub categories); they change classification, not precision.

## After round 2

Round 2 adds the `testdata`/`vendor`/`node_modules`/`third_party` path skip (no Go oracle change). Same commit
`0ba07e08`; `find` + `xargs -P8` over `kibitzer check native <rule> <file>`, debug builds before/after, compared by `file:line` against the shard with `comm`.

| | After round 1 | After round 2 |
|---|---|---|
| Findings | 27 | 27 |
| true_positive records still reported | 26 of 26 | 26 of 26 |
| false_positive records still reported | 1 | 1 |

Precision unchanged at 26 / 27 = **96.3%**. The remaining FP is the cross-file `assertFailures` helper
(`validation_resourceslice_test.go:210`); fix via `assertion_helpers`.
