# Held-out evaluation 3: `test-no-assertion-go` and the oracle tagger

Third held-out run on Go repos never used for tuning. Binary built from tip `ed84e64`
(`oracle-heldout3` branch, prebuilt `kibitzer-r4`). Nothing in the code was changed.

## Method

Repos (shallow clones): go-gitea/gitea `31626de`, syncthing/syncthing `d70d210`,
restic/restic `e40a4a1`, rclone/rclone `35a03a4`, goreleaser/goreleaser `4a71e1e`,
helm/helm `9b88efd`, influxdata/telegraf `2bac12a`, minio/minio `7aac2a2`.

1. Step 1: `kibitzer check native test-no-assertion-go <file>` over every `*_test.go`
   outside `vendor/`. Recounted: **54** findings (minio 26, rclone 11, telegraf 8,
   restic 3, syncthing 2, helm 2, gitea 1, goreleaser 1), matching the expected count.
   Listed in `heldout3-findings.tsv`.
2. Step 2: each finding's test function was opened, plus same-file and cross-file helpers
   and callbacks by grep, and labelled with one ground-truth tag. Labels are in
   `heldout3-ground-truth.jsonl` and were committed (`7de3514`) before any tag output was
   produced.
3. Step 3: re-run with `KIBITZER_ORACLE_TAGS=1`; the `[tag=...]` line was parsed
   (`no-panic-smoke:uncued` / `:cued` suffix collapsed to the base tag for scoring) and
   compared with the labels.

Ground-truth tags: `false-positive`, or a true-positive tag from `compile-check`,
`race-smoke`, `lifecycle-smoke`, `no-panic-smoke`, `discarded-result`, `empty-stub`,
`liveness-smoke`, `untagged`. Note `untagged` is a legitimate truth label (3 cases:
diagnostic-logging tests with no smoke class), so a tagger "untagged" on those is a match.

### Blindness procedure and limits

Not read before labelling: `src/test_code/oracle_tag.rs`, anything under
`docs/backtest-triage/oracle-tagger/` other than what this run wrote, tagger fixtures,
and the oracle-related code in `src/checkers/test_no_assertion.rs`. Tagging was not run
until the labels were committed.

Limits:
- Labeller and tagger author are the same model family, so shared blind spots (what
  counts as "smoke") are not independent. Several tag boundaries are judgement calls:
  `liveness-smoke` vs `race-smoke` for goroutine+channel handshakes (dsync/lsync
  `TestParallelReaders`, `TestMutex`), `lifecycle-smoke` vs `no-panic-smoke` for
  Lock/Unlock or Shutdown-twice tests.
- `discarded-result` was assigned when a comparison or result existed but was only
  `t.Log`'d (never fails); a stricter labeller could call these `untagged`.
- n=54 and findings cluster per repo: minio supplies 26 and one cause supplies 13 of
  its 16 false positives. Confidence intervals are wide; per-repo rates are not independent.
- The helper-following depth of the labeller (cross-file grep) is deeper than the rule's
  (same-file helpers), by design of the question.
- Ground truth for the 13 minio wrapper tests was verified by counting `t.Fatal*/t.Error*`
  in each callback (or the same-file helper it delegates to): all 13 > 0.

## Rule precision

Loose = TP/(TP+FP). Strict counts only `discarded-result` + `empty-stub` as true positives
(likely real defects).

| Scope | n | FP | Loose | Strict TP | Strict |
|---|---|---|---|---|---|
| Overall | 54 | 18 | 36/54 = 66.7% | 9 | 16.7% |
| Excluding minio | 28 | 2 | 26/28 = 92.9% | 9 | 32.1% |
| minio | 26 | 16 | 10/26 = 38.5% | 0 | 0% |
| rclone | 11 | 1 | 90.9% | 3 | 27.3% |
| telegraf | 8 | 0 | 100% | 6 | 75.0% |
| restic | 3 | 1 | 66.7% | 0 | 0% |
| syncthing | 2 | 0 | 100% | 0 | 0% |
| helm | 2 | 0 | 100% | 0 | 0% |
| gitea | 1 | 0 | 100% | 0 | 0% |
| goreleaser | 1 | 0 | 100% | 0 | 0% |

Ground-truth distribution: false-positive 18, no-panic-smoke 10, discarded-result 6,
liveness-smoke 4, lifecycle-smoke 4, empty-stub 3, compile-check 3, race-smoke 3,
untagged 3.

### False positives by cause (18)

| Cause | n | Where | Notes |
|---|---|---|---|
| Assertions live in a callback passed as a function value to a runner (`ExecObjectLayerAPITest(Args{objAPITest: testX})`), or that callback delegates to a helper that asserts | 13 | minio `cmd/bucket-*-handlers_test.go`, `object-handlers_test.go` | New class. The rule follows direct same-file helper calls but not a function identifier passed as an argument or struct field. All 13 callbacks have 2-18 `t.Fatal*/t.Error*`. A single repo-wide pattern; this alone is 72% of the FPs. |
| `panic(...)` used as the failure signal, in the test body | 2 | restic `TestNoopCounter`; minio `TestDynamicTimeoutConcurrent` (panic inside the goroutine body) | New class: `panic` on a violated invariant is an assertion. |
| `panic(...)` as the failure signal in a same-file helper the test calls | 2 | minio dsync and lsync `TestRWMutex` (`hammerRWMutex`/`HammerRWMutex` -> reader/writer `panic("wlock(%d)")`) | Same class as above, reached through a helper. |
| Not a real test: nested no-op subtests run as a fixture by another test | 1 | rclone `fstest/runs` `TestTests` ("Nested tests for TestTestsToRegexpLive to run") | One-off; name/comment identify it. |

So 4 of the 5 non-callback FPs share one theme (panic-as-assertion); the callback class
is the dominant one. Excluding the callback class (13 findings removed from the denominator), loose precision
would be 36/41 = 87.8%, versus 66.7% as measured. This is a post-hoc projection, not a
measured number.

### True positives by tag (36)

`no-panic-smoke` 10 (4 with a comment or name cue: gitea, telegraf phpfpm, restic updater,
syncthing semaphore; helm x2 cued by name), `discarded-result` 6 (telegraf 3, rclone b2 x2,
swift; all log-only or result-dropping), `liveness-smoke` 4, `lifecycle-smoke` 4,
`empty-stub` 3 (telegraf x3), `compile-check` 3, `race-smoke` 3, `untagged` 3.

The six `discarded-result` cases are the likely real defects: tests that compare against an
expected value and only `t.Logf` the mismatch so they can never fail (rclone b2
`TestTimeString`, `TestParseTimeString`, swift `TestInternalUrlEncode`), telegraf tests that
run the code and drop or only log the outcome (`TestRunSetUpClient`, `TestExtractFields`,
`TestMachineProcstat`). Plus three telegraf empty stubs.

## Tagger accuracy

Tagger output on all 54 findings: no-panic-smoke 26 (22 `:uncued`, 4 cued), untagged 16,
race-smoke 5, empty-stub 3, compile-check 2, lifecycle-smoke 2.

| Measure | Result |
|---|---|
| Exact match, all 54 (the tagger has no false-positive class, so every FP row counts as a miss) | 20/54 = 37.0% |
| Exact match on the 36 true positives | 20/36 = 55.6% |
| Exact match, excluding minio, all 28 | 18/28 = 64.3% |
| Exact match on true positives excluding minio | 18/26 = 69.2% |
| Coverage on TPs (tagged anything but `untagged`) | 23/36 = 63.9% (excl. minio 16/26 = 61.5%) |

Note on coverage: 3 of the 36 TPs are truly `untagged`, so the tagger leaving them
`untagged` is correct and counted as an exact match, not as coverage.

### Confusion matrix (rows = ground truth, columns = tagger)

| truth \ tagger | compile-check | empty-stub | lifecycle-smoke | no-panic-smoke | race-smoke | untagged | total |
|---|---|---|---|---|---|---|---|
| compile-check | 2 | 0 | 0 | 0 | 0 | 1 | 3 |
| discarded-result | 0 | 0 | 0 | 0 | 0 | 6 | 6 |
| empty-stub | 0 | 3 | 0 | 0 | 0 | 0 | 3 |
| false-positive | 0 | 0 | 1 | 13 | 1 | 3 | 18 |
| lifecycle-smoke | 0 | 0 | 1 | 3 | 0 | 0 | 4 |
| liveness-smoke | 0 | 0 | 0 | 2 | 1 | 1 | 4 |
| no-panic-smoke | 0 | 0 | 0 | 8 | 0 | 2 | 10 |
| race-smoke | 0 | 0 | 0 | 0 | 3 | 0 | 3 |
| untagged | 0 | 0 | 0 | 0 | 0 | 3 | 3 |

### Per-tag precision and recall (true positives only)

| Tag | Predicted | Truth | Correct | Precision | Recall |
|---|---|---|---|---|---|
| empty-stub | 3 | 3 | 3 | 100% | 100% |
| race-smoke | 4 | 3 | 3 | 75% | 100% |
| compile-check | 2 | 3 | 2 | 100% | 67% |
| no-panic-smoke | 13 | 10 | 8 | 62% | 80% |
| lifecycle-smoke | 1 | 4 | 1 | 100% | 25% |
| discarded-result | 0 | 6 | 0 | n/a | 0% |
| liveness-smoke | 0 | 4 | 0 | n/a | 0% |

(Predicted counts here are over the 36 TPs only. Over all 54 findings, `no-panic-smoke` is
predicted 26 times with 8 correct, because 13 FP rows get it.)

### Tagger disagreements on true positives (16)

| Cause | n | Cases |
|---|---|---|
| `discarded-result` never produced: tagger says `untagged` | 6 | telegraf `TestRunSetUpClient`, `TestExtractFields`, `TestMachineProcstat`; rclone b2 `TestTimeString`, `TestParseTimeString`, swift `TestInternalUrlEncode` |
| Lock/Unlock or goroutine-handshake tests over-tagged `no-panic-smoke:uncued` instead of `lifecycle-smoke` / `liveness-smoke` | 5 | minio dsync `TestSimpleLock`, `TestSimpleLockUnlockMultipleTimes`, `TestTwoSimultaneousLocksForDifferentResources` (lifecycle); dsync and lsync `TestParallelReaders` (liveness) |
| Liveness not recognized | 2 | minio `TestTwoSimultaneousLocksForSameResource` (untagged), `TestRingBufferCloseErrorUnblocks` (race-smoke) |
| Smoke test left `untagged` | 2 | restic `TestBlobsSortNilSlice`, syncthing `TestIssue1507` (single call on nil/edge input, no cue) |
| `compile-check` missed | 1 | minio `TestRingBuffer_interface` (blank decls plus one commented-out line and a `New(1)` constructor call) |

(Counts sum to 16; 36 TPs - 20 exact = 16.)

### Tagger on false-positive rows

The tagger has no way to say "assertion exists". On the 18 FPs it emitted
`no-panic-smoke:uncued` 13 times, `untagged` 3, `race-smoke` 1, `lifecycle-smoke` 1. The
13 are the minio callback wrappers (`ExecObjectLayerAPITest(...)` one-liners looking like
call-only bodies). The `:uncued` suffix ("lower confidence") is therefore the only signal
distinguishing a hollow smoke test from a delegating wrapper, but it did not separate them
here: of the 22 `:uncued` predictions, 13 are FP rows, 4 are correct `no-panic-smoke`, 3 are
`lifecycle-smoke` and 2 are `liveness-smoke` truths. The 4 cued predictions were all correct.

## Comparison with rounds 1 and 2

| | Round 1 | Round 2 | Round 3 (this) |
|---|---|---|---|
| Rule loose precision | 70.6% | 91.1% | 66.7% overall, 92.9% excl. minio |
| Tagger exact on TPs | 5/12 | 9/12; then 65/72 (19/26 excl. terraform) | 20/36 = 55.6%; 18/26 = 69.2% excl. minio |

Answer to "do post-tuning rules and tagger hold up on genuinely new code":

- Rule: partly. On the 28 findings outside minio the loose precision (92.9%) matches round
  2 (91.1%), so the tuned rule transfers on typical code. The overall 66.7% is dominated by
  one structural gap the earlier rounds did not exercise: minio's callback-runner idiom
  (13 FPs from 13 tests). Because findings cluster per repo, the effective sample is
  closer to 8 repos than 54 findings; the honest reading is "92% on 7 repos, 38% on the one
  repo that uses a table-of-callbacks harness", not a stable 67%.
- Tagger: no. Exact match on TPs excluding minio (69.2%) is below round 2's
  figures (65/72 = 90% overall, 19/26 = 73% excl. terraform) and the all-TP figure is lower still (55.6%).
  The tagger never emits `discarded-result` or `liveness-smoke` here (0/10 recall
  combined), and sends all 6 `discarded-result` truths to `untagged`; those are the
  highest-value labels for a reader. Lifecycle recall is 25%. What does generalize:
  `empty-stub` (3/3), `race-smoke` (3/3), `compile-check` (2/3).
- Strict precision is low (16.7% overall; 32.1% excl. minio): about a third of the
  non-minio findings are likely real defects, driven by telegraf (6 of 8).

## Biggest failure modes (for a later tuning round, not acted on here)

1. Rule: function value passed as an argument/struct field to a runner (13 FPs).
2. Rule: `panic(...)` as assertion, in the body, a goroutine, or a same-file helper (4).
3. Tagger: no `discarded-result` detection when the body computes a result then only
   `t.Log`s it or drops it (6 of 6 missed).
4. Tagger: `no-panic-smoke:uncued` swallows lifecycle and liveness tests, and also tags
   delegating wrappers (13 FP rows).
5. Tagger: liveness (goroutine + channel/timeout) tests recognized as `race-smoke` or
   nothing (0/4).

## Files

- `heldout3-findings.tsv`: the 54 findings.
- `heldout3-ground-truth.jsonl`: labels `{repo,file,line,func,tag,reason}`.

## After round 5

Two changes, both chosen from this set's failures, so **held-out sets 1, 2 and 3 are now all
tuning data. A fresh batch of unseen repos is needed for any new accuracy claim**; the numbers
below measure fit to these sets, not generalisation.

Rule (commit `50a7bb9`):

- A same-file function or method value passed as a call argument or struct-field value
  (`ExecObjectLayerAPITest(Args{objAPITest: testX})`) is followed like a helper call, one
  helper hop deeper than the passed function. A passed function with no assertion keeps the
  test flagged, as does a value not defined in the file.
- An `if <cond> { panic(...) }` counts as an assertion in the test body, a goroutine or a
  same-file helper. Deviation from the brief: `if err != nil { panic(err) }` still counts only
  inside a `go` statement. Counting it everywhere dropped nats
  `TestSystemAccountWithBadRemoteLatencyUpdate` (held-out 2 true positive), whose helper
  `createAccount` panics on a setup error; that panic propagates a setup failure and asserts
  nothing about the code under test. The minio and restic panics are value checks, so they count.
- Not handled: rclone `TestTests` (`fstest/runs`). It is a fixture run by another test, and
  nothing in its body distinguishes it from a real assertion-free test short of its name or
  comment.

Tagger (commit `5a34c8c`): only `empty-stub`, `race-smoke` and `compile-check` are emitted
(the three that matched in all three rounds). `lifecycle-smoke`, `no-panic-smoke` (cued and
uncued), `discarded-result` and `liveness-smoke` no longer produce a tag line; a body matching
none of the three gets no line (previously `untagged`). Still annotate-only behind
`KIBITZER_ORACLE_TAGS=1`.

### Rule on this set

Findings 54 -> 37 (minio 26 -> 10, restic 3 -> 2). Removed: the 13 callback wrappers plus
minio `TestDynamicTimeoutConcurrent`, dsync `TestRWMutex` and lsync `TestRWMutex` (16), and
restic `TestNoopCounter` (1). All 36 ground-truth true positives are still reported.

| | Before | After |
|---|---|---|
| Findings | 54 | 37 |
| False positives | 18 | 1 (rclone `TestTests`) |
| Loose precision | 36/54 = 66.7% | 36/37 = 97.3% |
| Excluding minio | 26/28 = 92.9% | 26/27 = 96.3% |

The fix targets the causes found on this set, so 97.3% is a fit figure, not an estimate.

### Kept tags on this set (after, annotation run over the 37 findings)

| Tag | Predicted | Truth | Correct | Precision | Recall |
|---|---|---|---|---|---|
| empty-stub | 3 | 3 | 3 | 100% | 100% |
| race-smoke | 4 | 3 | 3 | 75% | 100% |
| compile-check | 2 | 3 | 2 | 100% | 67% |

Across all four ground-truth sets (in-sample, held-out 1, 2, 3; 214 rows): `empty-stub`
13/14 precision, 13/13 recall; `race-smoke` 36/39, 36/36; `compile-check` 51/51, 51/54. These
are the tags that survived three rounds, so selection is itself a fit to the data.
