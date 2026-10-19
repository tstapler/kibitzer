# Held-out set 4: final Go rule and three-tag oracle tagger

Fourth held-out evaluation of `test-no-assertion-go` and its oracle tagger on 13 repos never used for
tuning. No code was changed. Binary: `kibitzer-r5`, built from tip 06c6123
(`sdd/test-effectiveness-critique-impl`). Data: [heldout4-findings.tsv](heldout4-findings.tsv),
[heldout4-ground-truth.jsonl](heldout4-ground-truth.jsonl) (committed in ce08f5a, before any tagged run).

## Method and blindness

1. Listed findings with `check native test-no-assertion-go` over every `*_test.go` outside `vendor/`
   (`find | xargs -P8`). Recount: 82 findings, identical per repo to the stored `<owner>-<repo>.txt` files.
2. Labelled each function (and cross-file helpers, by grep) from the Go source only, one tag each. The
   tagger source, its fixtures, `RESULTS.md`, `HELDOUT*.md`, earlier ground-truth files, `eval.sh` and the
   tagger code in `test_no_assertion.rs` were not opened, and `KIBITZER_ORACLE_TAGS` was unset until the
   ground truth was committed.
3. Ran with `KIBITZER_ORACLE_TAGS=1`, parsed `[tag=...]` (absent means untagged), and computed every
   figure with awk/jq (`metrics.sh`, scratch under `~/.cache/kibitzer-eval/heldout4/`).

Limits:

- The labeller is the same model family as the one that designed the tags, so the label vocabulary
  (`no-panic-smoke`, `liveness-smoke`, ...) is not independent of the tagger's.
- n = 82, and 10 of the 13 repos produced findings. Findings cluster by repo: vault (21) and
  opentelemetry-collector (8) carry 13 of the 14 false positives, and each cluster is one
  project-specific idiom. Treat the precision figures as having wide intervals.
- "True positive" means "the rule's claim of no assertion is right". The reasoning behind tag labels such
  as `no-panic-smoke` is judgment, and two cases (`TestHandleModEvent_HasChanges`, `TestAssertState`)
  fit no tag (`untagged`).
- Labelling compile-check/empty-stub stays mechanical; labelling race-smoke vs liveness-smoke does not
  (see `TestGrabLockOrStop`).

## Findings per repo and rule precision

Loose = TP/(TP+FP). Strict counts only `discarded-result` + `empty-stub` as true positives (the likely-defect
subset), over all findings.

| repo | n | TP | FP | loose | strict TP | strict |
|---|---|---|---|---|---|---|
| hashicorp/vault | 21 | 14 | 7 | 66.7% | 2 | 9.5% |
| argoproj/argo-cd | 12 | 11 | 1 | 91.7% | 2 | 16.7% |
| istio/istio | 12 | 12 | 0 | 100% | 1 | 8.3% |
| hashicorp/nomad | 8 | 8 | 0 | 100% | 4 | 50.0% |
| open-telemetry/opentelemetry-collector | 8 | 2 | 6 | 25.0% | 0 | 0% |
| grafana/loki | 6 | 6 | 0 | 100% | 3 | 50.0% |
| hashicorp/consul | 5 | 5 | 0 | 100% | 2 | 40.0% |
| uber-go/zap | 5 | 5 | 0 | 100% | 0 | 0% |
| containerd/containerd | 3 | 3 | 0 | 100% | 0 | 0% |
| docker/cli | 2 | 2 | 0 | 100% | 0 | 0% |
| flux2, badger, kind | 0 | | | | | |
| **all** | 82 | 68 | 14 | **82.9%** | 14 | **17.1%** |
| **excluding vault** | 61 | 54 | 7 | **88.5%** | 12 | **19.7%** |
| excluding vault and otel-collector | 53 | 52 | 1 | 98.1% | 12 | 22.6% |

## False positives by cause (14)

| cause | n | repos | new class? |
|---|---|---|---|
| Not a real test: `TestXxx_PluginMain_*` / `TestPlugin_GRPC_Main` are env-gated subprocess entrypoints (`if os.Getenv(...) == "" { return }`, then serve a plugin when the test binary is re-exec'd) | 7 | vault | yes |
| Assertions live in a cross-file helper that receives `t` through a params struct (`CheckConsumeContract(Params{T: t, ...})`; 36 and 22 assert/require/Fatal calls in the two helper files) | 6 | otel-collector | yes |
| Negative expectation through a mock: `clusterCache.EXPECT().Invalidate(...).Panic("should not invalidate")` makes an unexpected call fail the test | 1 | argo-cd | yes |

All 14 are untagged (no FP carries a tag), so the tagger never decorates a false positive with a wrong
oracle in this set.

## Tag precision and recall (three emitted tags)

Precision = ground-truth equals the tag among findings that carry it. Recall = tagged correctly among all
ground-truth items of that tag.

| tag | emitted | correct | precision | gt count | recall | in-sample (rounds 2-3) precision / recall |
|---|---|---|---|---|---|---|
| empty-stub | 8 | 8 | 100% | 9 | 88.9% | 13/14, 13/13 |
| race-smoke | 13 | 12 | 92.3% | 13 | 92.3% | 36/39, 36/36 |
| compile-check | 14 | 14 | 100% | 15 | 93.3% | 51/51, 51/54 |

Misses and the one wrong tag:

- `TestStructs_Implements` (consul): 17 `var _ I = &T{}` declarations in one block, compile-check, untagged.
- `TestExternalTokenHelperPath` (vault): builds maps, then the assertion loop is commented out, empty-stub, untagged.
- `TestParallelLabels` (istio): fire-and-forget goroutines with no WaitGroup, race-smoke, untagged.
- `TestGrabLockOrStop` (vault): tagged race-smoke, labelled liveness-smoke (a deadlock watchdog; it also
  has goroutines and sync, so the tag is defensible).

## True positives left untagged

68 true positives, 34 tagged, so **34 (50.0%) are untagged**. Composition of those 34:

| ground-truth tag | n |
|---|---|
| no-panic-smoke | 15 (most carry a "should not panic"-style comment or name cue) |
| liveness-smoke | 6 |
| discarded-result | 5 |
| lifecycle-smoke | 2 |
| no tag fits (`untagged`) | 2 |
| compile-check / empty-stub / race-smoke misses | 1 each |

This is the cost of pruning down to three tags: all 5 `discarded-result` true positives (the likely-real
defects, e.g. `TestCMS` builds a sketch and never queries it, `TestDefaultConfig` drops nine `hcl.Decode`
results 500 times, `TestOCSPRetry` only prints the result) print no tag, as do 15 `no-panic-smoke` and
6 `liveness-smoke` tests. The tagged tags cover the three cheap, mechanical oracles; the untagged half is
the part that needs a human decision.

## Comparison with rounds 2 and 3

- Rule loose precision: 82.9% overall here (88.5% without vault) against 91.1% (round 2) and 66.7%
  (round 3) before the final rule changes. The final rule lands between the two earlier sets and is not
  clearly better or worse; the set-level swing (about 25 points between rounds) is within what repo
  clustering alone produces here.
- The false positives are new classes the rule does not yet handle (subprocess-entrypoint "tests",
  assertions behind a `t`-carrying params struct, mock `.Panic` expectations). None repeats a class tuned
  on in earlier rounds as far as can be told without opening the earlier files, which this evaluation
  deliberately did not do.
- Tags hold up on new code: empty-stub 8/8 precision (in-sample 13/14), race-smoke 12/13 (36/39),
  compile-check 14/14 (51/51). Recall 88.9% to 93.3% (in-sample 100%, 100%, 94.4%), a modest drop for
  empty-stub and race-smoke driven by one miss each. No tag precision fell below its in-sample value by
  more than the one-case granularity of n.
- Strict precision (17.1%, 19.7% without vault) is low because most findings in mature repos are
  `*_impl` compile checks and smoke tests, not defects. Rounds 2 and 3 strict numbers were not part of
  the brief, so no comparison is made.

Answer: the three kept tags generalise (no wrong tag on a false positive, precision at or near 100%, recall
near 90%); the final rule does not clearly generalise, because 13 of its 14 false positives come from
two repos through three idioms it has no handling for.

## After round 6

Commit (this round's rule change): three rule changes aimed at the 14 false positives above, no
tagger change. All four held-out sets (1, 2, 3 and 4) are now tuning data. Any accuracy claim needs a
fresh batch of repos; the numbers below measure fit, not generalisation.

Rules (`src/checkers/test_no_assertion.rs`, fixtures `testdata/test-quality/go/{env_entrypoint,params_struct_handle,mock_panic_expectation}_test.go`):

1. Env-gated subprocess entrypoint (7, vault). The test is skipped from reporting when its first
   statement is an `if` (no `else`) whose condition calls `os.Getenv(` or `os.LookupEnv(`, and either the
   remaining body or the `if` body (when the `if` is the only statement) ends with a call whose last
   segment starts with `Serve`, `Run`, `Main`, `Start` or `Listen`. An early `return` gate followed by
   an empty body, or by an ordinary call such as `doExpensiveThing()`, is still reported, as is a bare
   `Serve(...)` with no gate.
2. Params-struct `t` (6, otel-collector). A call counts as asserting when an argument is a struct
   literal (or `&literal`, or a variable bound to one in the same function) with a field set to the
   testing handle (`Params{T: t}`), the callee is not resolvable in the file, and its last segment
   contains `check`, `verify`, `assert`, `require`, `expect`, `validate`, `contract` or `test`
   (any case). The existing rule that a bare `t` argument to an unresolved call asserts is unchanged and
   already covered `Foo(t)`; a literal carrying `t` into `newServer(Params{T: t})` still gets reported.
3. Mock `.Panic` expectation (1, argo-cd). A call whose callee text ends in `.Panic` and contains
   `.EXPECT()` (mockery) or `.On(` (testify) counts as a guarded panic. `logger.Panic(...)` does not.
   The narrower form "only when the mock is later asserted" was not feasible: the argo-cd test never
   asserts the mock, the `Panic` is the oracle. The sibling `TestHandleModEvent_HasChanges` (`Once()`
   expectations that nothing verifies) is still reported, correctly.

Guardrail: whole-repo scans of `test-no-assertion-go` over 29 repos, `file:line` sets compared with
`comm`, base = tip 48a563f binary, after = this commit.

| Set | Findings before | After | Ground-truth true positives still reported | False positives left |
|---|---|---|---|---|
| In-sample (kubernetes 27, stapler-squad 38) | 65 | 65 | 64/64 | 0 |
| Held-out 1 (17 rows) | 12 | 12 | 12/12 | 0 |
| Held-out 2 (79 rows) | 72 | 72 | 72/72 | 0 |
| Held-out 3 (54 rows) | 37 | 37 | 36/36 | 1 (rclone `TestTests`, unchanged) |
| Held-out 4 (82 rows) | 82 | 68 | 68/68 | 0 |

The only findings removed anywhere are the 14 held-out 4 false positives (vault 7, otel-collector 6,
argo-cd 1); no finding was added and no true positive was lost. Loose precision on this set is now
68/68 by construction, because the rules were written from these 14 cases: that figure is a fit, not
evidence of generalisation. Rule 2's name filter and rule 1's serve-prefix list are judgment calls
taken from two repos each.
