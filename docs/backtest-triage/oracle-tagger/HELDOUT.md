# Oracle tagger: held-out evaluation (Go)

Tip under test: `7baf957` (`sdd/test-effectiveness-critique-impl`). No code was changed.

> **These 17 findings are now tuning data and no longer held out.** Round 3 (below) changed the rule and the tagger in response to them, so any further number measured on this set is in-sample. A new held-out set needs fresh repos.

## Method

1. Build `target/debug/kibitzer`; list `test-no-assertion-go` findings over every non-vendor `*_test.go` in three shallow clones: prometheus/prometheus `c052a79`, cli/cli `fa92838`, etcd-io/etcd `0155117` (`heldout-findings.tsv`).
2. Label each finding's ground truth from the test source (plus helpers found by grep) using only the tag definitions in the task brief. Labels are in `heldout-ground-truth.jsonl` and were committed (`7155dc9`) before any tagger output existed.
3. Run again with `KIBITZER_ORACLE_TAGS=1`, parse the `[tag=...]` line that follows each finding, join to ground truth with `jq`.

Counts: prometheus 2, cli/cli 7, etcd 8, **n = 17**.

## Blindness procedure and its limits

- Not opened before step 3: `src/test_code/oracle_tag.rs`, `docs/backtest-triage/oracle-tagger/*` (other than files I created), tagger fixtures, the oracle lines of `test_no_assertion.rs`. The env flag was not set until after the commit.
- Limits: the labeller is the same model family as the tagger's author, so shared habits in what counts as "smoke" are not independent. The tag definitions were supplied by the same people who wrote the tagger. n = 17 with several findings correlated (4 MoveLeader tests share one helper; 4 of 5 FPs are the same helper-process idiom), so the effective sample is closer to ~10 independent cases. One label is a judgement call (`goversion` Test: `compile-check` vs `empty-stub`) and one is borderline (`TestHistoryAppendSuccess`: `no-panic-smoke` vs `discarded-result`).
- Ground truth added an unlisted reason: `false-positive` was used for "subprocess re-exec stubs that are not tests", since the listed tags (all true-positive) do not fit and an assertion does exist in the parent test.

## Rule precision (test-no-assertion-go itself)

| | TP | n | precision |
|---|---|---|---|
| Loose (any true-positive tag) | 12 | 17 | 70.6% |
| Strict (discarded-result + empty-stub only) | 5 | 17 | 29.4% |

All 5 false positives are `TestHelperProcess`-style `os.Exit` re-exec entry points in cli/cli (`TestCommitsHelperProcess`, `TestCommandMocking`, `TestHelperProcess` x3). Cause: gated on `GH_WANT_HELPER_PROCESS`, end in `os.Exit`; the assertions live in the parent tests. Strict TPs: `TestFeatureGateMetrics` (TODO-only) and four `TestMoveLeaderWith*Auth` (delegate to a `t.Skip()` helper). No `discarded-result` cases occurred.

## Tagger vs ground truth (computed by `jq`)

Rows (truth -> predicted):

| truth \ predicted | empty-stub | no-panic-smoke | no-panic-smoke:uncued | race-smoke | untagged |
|---|---|---|---|---|---|
| compile-check (1) | 1 | | | | |
| empty-stub (5) | 1 | | 4 | | |
| no-panic-smoke (4) | | 2 | | | 2 |
| race-smoke (2) | | | | 2 | |
| false-positive (5) | | | | | 5 |

`no-panic-smoke:uncued` is emitted as a distinct tag string; it was not folded into `no-panic-smoke`, though that would not change any result (all four truths are empty-stub).

Per-tag (precision = TP/predicted, recall = TP/truth):

| tag | predicted | truth | TP | precision | recall |
|---|---|---|---|---|---|
| race-smoke | 2 | 2 | 2 | 100% | 100% |
| no-panic-smoke | 2 | 4 | 2 | 100% | 50% |
| empty-stub | 2 | 5 | 1 | 50% | 20% |
| compile-check | 0 | 1 | 0 | n/a | 0% |
| no-panic-smoke:uncued | 4 | 0 | 0 | 0% | n/a |
| untagged | 7 | 0 | 0 | n/a | n/a |

Headline numbers:

- Exact match, all 17: **5/17 = 29%** (strict: a tag must equal the label; `untagged` on a false positive counts as a miss because it was not a label).
- If `untagged` on a rule false positive is counted as correct (reasonable: "no oracle signal" on a non-test): 10/17 = 59%.
- Exact match on the 12 true positives: 5/12 = 42%.
- Coverage (any tag other than `untagged`): 10/17 = 59% of all findings; 10/12 = 83% of true positives.
- Of the 10 tagged findings, 5 are exactly right (50% tag precision).

## Every disagreement

1. **`TestMoveLeaderWith{No,Invalid,Root,User}Auth` (etcd, 4x): truth empty-stub, predicted `no-panic-smoke:uncued`.** The body is one call to a helper in the same file whose body is only `t.Skip()`. The tagger apparently sees "calls whose results are dropped" and does not follow the delegation to find the skip. A same-file helper-chasing step would fix it; this is a systematic miss, not noise.
2. **`goversion.Test` (prometheus): truth compile-check, predicted empty-stub.** Body is empty; a comment says the blank import is the test. The tagger reads the empty body (reasonable); my label used the comment. Either is defensible, so treat as ambiguous.
3. **`TestNoOpService` (cli): truth no-panic-smoke, predicted untagged.** Every call is a method on a local value with a comment "safe to call without panicking"; the tagger missed even this explicit cue (the comment phrase differs from `TestSpawnSendTelemetry...`'s "doesn't crash" which it did catch, so cue matching looks brittle, or the multi-call body falls out of its pattern).
4. **`TestHistoryAppendSuccess` (etcd): truth no-panic-smoke, predicted untagged.** No comment cue; oracle is that `append` does not panic. Mechanically this is a loop of dropped-result calls with no cue, so `untagged` is a defensible output; my label relied on reading `append`.
5. **5 helper-process stubs (cli): truth false-positive, predicted untagged.** Not a tagger error as such; the tagger cannot reclassify rule false positives and abstained, which is the right behaviour. Counted as misses only under the strict metric.

## Did the in-sample figures hold up?

The in-sample figures (92% coverage, 62/64 exact) cannot be re-derived from files I was barred from reading, so I compare against the numbers as quoted in the task.

- **Coverage 92%: not reproduced.** Held-out coverage is 59% over all findings, 83% over true positives. Part of the gap is the rule's false positives (5 of 17), which the tagger rightly leaves untagged.
- **Exact 62/64 (97%): not reproduced.** Held-out exact is 5/17 (29%) strict, 10/17 (59%) lenient, 5/12 (42%) over true positives. Even the best reading is far below 97%.
- The sole class that generalised cleanly is `race-smoke` (2/2, both with `Race` in the name and goroutines plus channels). `no-panic-smoke` was right whenever a comment cued it (2/2) and absent when it did not (0/2). The `empty-stub` rule failed in the one pattern (delegation to a skip-only helper) that the in-sample set evidently did not contain.
- Caveat on conclusions: with n = 17, correlated findings, and a single labeller, the intervals are wide (for 5/12 an exact 95% interval is roughly 15% to 72%). The data support "the in-sample numbers were optimistic, and delegation-to-skip and comment-cue variance are real gaps"; they do not support a precise held-out accuracy.

## After round 3 (the 17 findings are tuning data from here on)

Two changes, both made after reading this document:

1. **Rule (`test-no-assertion-go`)**: a Go test whose body (closures included) calls `os.Exit` is skipped. These are subprocess re-exec entry points (`TestHelperProcess`); the parent test asserts. Keyed on the `os.Exit` call only, not on the name, so `TestNamedHelperButNoExit` and an empty `TestEmptyHelperProcess` are still reported (`testdata/test-quality/go/helper_process_test.go`).
2. **Tagger**: if every top-level statement of the test is a call to a plain same-file function whose body is empty, TODO-only, or only `t.Skip*`, tag `empty-stub`. One level only (fixture `TestDelegates_twoLevels` is not tagged `empty-stub`). Still annotate-only and behind `KIBITZER_ORACLE_TAGS=1`; nothing is suppressed by a tag.

Not tuned: no name-specific cases. Change 1 is justified by the 5 cli/cli findings, change 2 by the 4 `TestMoveLeaderWith*Auth` findings (same helper) alone; each is one idiom seen in one repo, so in-sample evidence for both is thin even though the idioms are common Go.

Counts (scan = `find <repo> -name '*_test.go' -not -path '*/vendor/*' | xargs -P8 -n1 kibitzer check native test-no-assertion-go`):

| repo | findings before | after |
|---|---|---|
| prometheus/prometheus | 2 | 2 |
| cli/cli | 7 | 2 |
| etcd-io/etcd | 8 | 8 |
| total | 17 | 12 |

Rule precision on the 12 remaining: 12/12 loose true positives (was 12/17 = 70.6%); strict (discarded-result + empty-stub) still 5/12.

Tagger on the 12 reported findings: exact match 9/12 (was 5 of the 12 TPs). Changed: the 4 MoveLeader tests `no-panic-smoke:uncued` -> `empty-stub` (now correct). Still wrong: `TestNoOpService` (untagged, truth no-panic-smoke), `TestHistoryAppendSuccess` (untagged, truth no-panic-smoke), `goversion.Test` (empty-stub, truth compile-check; the ambiguous label). Per-class, both are in-sample now and say nothing about generalisation.

Guardrails:
- In-sample rule recall: kubernetes 26/26 and stapler-squad 38/38 triaged `true_positive` findings still reported (`comm -23 <triaged> <current>` empty); finding counts unchanged (27 and 38).
- Oracle tagger in-sample eval (`eval.sh`): exact-match 62/64 before and after (the two remaining misses are the known liveness-vs-race cases).
