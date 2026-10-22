# Intentional-oracle tagger: feasibility results

Status: prototype. The tag annotates a `test-no-assertion-go` finding; it never changes whether the
finding is reported (ADR-004: no suppression).

## What exists

- `src/test_code/oracle_tag.rs`: `tag_go(&TestFn) -> OracleTag`, plus `OracleTag::annotation()`.
- Surface: with `KIBITZER_ORACLE_TAGS=1`, Go findings from `test-no-assertion-go` get one extra line, for
  example `Smoke-only: race (goroutines and sync primitives, results discarded) [tag=race-smoke]`.
  Without the variable, output is byte-for-byte unchanged (the call site in
  `src/checkers/test_no_assertion.rs` is gated on it), so existing UX tests are unaffected.
- Unit tests: `testdata/test-quality/go/oracle/oracle_cases_test.go` (snippets adapted from real findings),
  asserted in `oracle_tag::tests::fixture_cases_get_the_expected_tag`.
- Evaluation: `docs/backtest-triage/oracle-tagger/eval.sh` (run from the repo root after `cargo build`). It runs
  the real checker over every ground-truth file and computes the matrix and precision/recall with `jq`.
  Raw join: `joined.jsonl`; hand labels: `ground-truth.jsonl`.

## Method

1. Before writing any tagger code, hand-labeled each `true_positive` record of
   `tstapler-stapler-squad/test-no-assertion-go.jsonl` (38) and `kubernetes-kubernetes/test-no-assertion-go.jsonl`
   (26) by reading the test body in the repo clones at the commits recorded in the shards. That is 64 findings
   (the brief said ~66; the shards contain 38 + 26 true positives). The one false positive in the stapler-squad
   shard is a `testdata/` stub excluded by scope, not by intent, so it is not labeled.
2. Tagger written test-first against fixtures, then run over all 64 with the real checker.
3. Signals are text and statement shape only: comments, test name, top-level statement kinds (blank-identifier
   declaration, bare call, binding, `go`, `defer`, receive), `go` count and loop nesting, presence of
   `WaitGroup`/`chan`/`<-`/`Mutex`. No type information.
4. Tags are checked in this order: empty-stub, compile-check, race-smoke, discarded-result, lifecycle-smoke,
   no-panic-smoke (cued), no-panic-smoke:uncued, untagged.

## Results (64 findings, final run of `eval.sh`)

Round 1 (first implementation): 59/64 exact. Round 2 (after the three fixes below): 62/64 exact.

Confusion matrix, rows = hand label, columns = tagger:

```
want\got               compile-check  discarded-result  empty-stub  lifecycle-smoke  no-panic-smoke  no-panic-smoke:uncued  race-smoke  untagged
compile-check          7              0                 0           0                0               0                      0           0
discarded-result       0              5                 0           0                0               0                      0           0
empty-stub             0              0                 5           0                0               0                      0           0
lifecycle-smoke        0              0                 0           3                0               0                      0           0
no-panic-smoke         0              0                 0           0                18              0                      0           0
no-panic-smoke:uncued  0              0                 0           0                0               3                      0           0
race-smoke             0              0                 0           0                0               0                      16          0
untagged               0              0                 0           0                0               0                      2           5
```

| tag | TP | predicted | actual | precision | recall |
|---|---|---|---|---|---|
| compile-check | 7 | 7 | 7 | 100% | 100% |
| discarded-result | 5 | 5 | 5 | 100% | 100% |
| empty-stub | 5 | 5 | 5 | 100% | 100% |
| lifecycle-smoke | 3 | 3 | 3 | 100% | 100% |
| no-panic-smoke (cued) | 18 | 18 | 18 | 100% | 100% |
| no-panic-smoke:uncued | 3 | 3 | 3 | 100% | 100% |
| race-smoke | 16 | 18 | 16 | 89% | 100% |
| untagged | 5 | 5 | 7 | 100% | 71% |

Coverage: 59 of 64 findings (92%) received a non-`untagged` tag; 5 are `untagged` and all five were
hand-labeled `untagged`. The 2 errors are both `race-smoke` assigned to hand-labeled `untagged`
(`wait_test.go:664` and `informerwatcher_test.go:46`: goroutine plus channel handshakes whose oracle is
"completes without hanging").

## Rules added or loosened because of a specific finding

Honest list; these are the places the tagger was fitted to the corpus rather than derived from the brief.

1. `atomic` added to the race name/comment cue: needed for `user_pr_cache_test.go:21`
   (`TestUserPRCache_SetOnUpdated_Atomic`), a one-goroutine channel handoff.
2. A body that is only `t.*` calls plus a comment matching `compile-time|compiles` is `compile-check`: needed
   for `native_process_manager_test.go:39`.
3. The "result dropped" test for `discarded-result` first looked only at top-level statements and missed a
   `_ = sd.Detect(...)` inside a `for` (`asterism_test.go:151`); it now scans any depth. This one is a general
   correction, not a name special-case.

Choices made before the first run that came from reading the 64 bodies (so also corpus-informed): the no-panic cue
regex accepts "should not panic" and "should not cause panics" (the brief's list omitted `should`); lifecycle tests
yield to an explicit no-panic cue unless the name itself says Start/Stop/Close; race-smoke requires a cue, two or
more `go` statements, or a `go` inside a loop, which is a proxy for "results never read" (no data-flow analysis).
Not special-cased: the two `race-smoke` errors above (no rule distinguishes them without being a one-finding hack).

## What is not mechanically detectable

- Liveness versus race: `go` plus channels/WaitGroup where the oracle is "does not deadlock" (`wait_test.go`,
  `informerwatcher_test.go`, the workqueue tests) looks the same as a race smoke test. The three workqueue tests
  happened to stay untagged only because they have one goroutine and no race cue.
- Conversion versus call: `_ = I(x)` (compile check) and `_ = f(x)` (discarded result) differ only by type
  information. The tagger treats `_ =` as compile-check only for composite/`&T{}`/`(nil)` right-hand sides.
- Whether a name "promises" behaviour: the `No…/Returns/Empty/Error/Should/When` list is a word list, and it needed a
  panic/race exclusion (`NoPanic`, `NoCrash`, `NoDataRace`) to avoid mislabeling intentional smoke tests.
- Intent of a log-only or blocking demo (`TestKlog`, `TestTimeout`): both stay `untagged`, correctly, but the
  tagger cannot say they are intentional, only that no known pattern matches.
- Whether a `discarded-result` is a defect: the tag says "name promises, result dropped", which is a lead for a
  human, not a verdict.

## Sample size and circularity

- n = 64 findings from two repos; per-tag counts are 3 to 18, so a single finding moves a rate by 6 to 33 points.
  The percentages above are not estimates of real-world precision.
- The ground truth was labeled by the same agent that then built the tagger, from the same 64 findings, and the
  tagger was run on exactly those findings. The labels were written before the code, but the label definitions
  and the regexes were shaped by the same reading. Treat this as a feasibility check ("can these categories be
  separated by cheap signals at all"), not validation.
- Both repos are the ones that produced the triage set, so there is no held-out repo. Test names in stapler-squad
  carry unusually explicit cues (`should_..._When_...`, "Should not panic"), which likely inflates recall there.
- Next step if pursued: label a fresh shard from a repo not in this set (for example `servo/servo` or
  `apache/cassandra`) by someone other than the tagger's author, freeze the rules, and measure.

## After round 3

Held-out round (see `HELDOUT.md`) led to two changes: `os.Exit` re-exec stubs are no longer reported by the rule, and the tagger follows one level of same-file delegation to a stub helper (empty, TODO-only, `t.Skip*`) and tags `empty-stub`. In-sample result is unchanged: `eval.sh` exact-match 62/64 before and after; the same two `race-smoke` over-predictions remain. The held-out findings are now tuning data, so the 9/12 reported there is not a generalisation estimate. Tag stays annotate-only and env-gated.

## After round 4

Two tagger changes (commit b317e29): `race-smoke` recognises `<recv>.Go(...)` (`sync.WaitGroup.Go`, `errgroup`) and a bare `go f()` / `go func(){...}()` when the test name or a comment carries a race cue (a WaitGroup/channel/mutex is no longer required in that case); `lifecycle-smoke` now requires every effective statement to be a call whose name is `Start`, `Stop`, `Shutdown`, `Close` or `Run` (exactly or followed by an upper-case suffix, the documented `LIFECYCLE_VERBS`), a construction-style call or a `Set*` setter; `Init`/`Open`/`Cancel`/`Wait`/`Begin`/`End`, `time.Sleep` and any other call now yield `no-panic-smoke`. A `Run*`/`Start*` bind (`s := RunServer(&opts)`) counts as construction. Still annotate-only behind `KIBITZER_ORACLE_TAGS=1`.

Results by command (`docs/backtest-triage/oracle-tagger/eval.sh`): in-sample exact-match 62/64 before and 62/64 after (same two `race-smoke` over-predictions, `wait_test.go:664` and `informerwatcher_test.go:46`; an intermediate version without `Set*` in the construction set dropped `worktree_pr_poller_test.go:48` to `no-panic-smoke:uncued`, 61/64, which is why `Set*` is accepted). Held-out 1 (17 rows): 9/17 before and after. Held-out 2 (79 rows): 65/79 before, 68/79 after (65/72 -> 68/72 on true positives). See `HELDOUT2.md` "After round 4". The rule-side change (callback assertions, goroutine-guarded panic) is described there too; kubernetes (27 findings) and stapler-squad (38) report exactly the same `file:line` set before and after, so all 26 + 38 triaged true positives are still reported.

Both held-out sets are now tuning data and no longer held out: the caddy, lazygit and nats `gosrv` fixes and the `Set*` allowance were chosen from their failures. The gains above (+3 on held-out 2, 0 on held-out 1, 0 in-sample) therefore measure fit to those sets, not generalisation.

## After round 5

Held-out 3 (`HELDOUT3.md`) found two rule gaps and showed that most tags did not generalise. Held-out sets 1, 2 and 3 are now all tuning data; any new accuracy claim needs a fresh batch of repos.

Rule (commit `50a7bb9`): a same-file function or method value passed as a call argument or struct-field value is followed like a helper call (assertions in it count; a passed function with none keeps the test flagged), and an `if <cond> { panic(...) }` counts as an assertion in the body, a goroutine or a same-file helper. `if err != nil { panic(err) }` still counts only inside a goroutine, because counting it everywhere dropped a held-out 2 true positive (nats `TestSystemAccountWithBadRemoteLatencyUpdate`, whose helper panics on setup errors). rclone `TestTests` is not handled: it is a fixture run by another test and nothing mechanical separates it from a real assertion-free test.

Tagger (commit `5a34c8c`): only `empty-stub`, `race-smoke` and `compile-check` are emitted. `lifecycle-smoke`, `no-panic-smoke`, `discarded-result` and `liveness-smoke` emit no tag line, and a body matching none of the three gets no line instead of `untagged`. `eval.sh` now reads a missing tag line as `untagged`. Still annotate-only behind `KIBITZER_ORACLE_TAGS=1`.

Guardrails, whole-repo scans of `test-no-assertion-go`, `file:line` sets compared with `comm`:

| Set | Findings before | After | Ground-truth true positives still reported | False positives left |
|---|---|---|---|---|
| In-sample (kubernetes 27, stapler-squad 38) | 65 | 65 | 64/64 | 0 |
| Held-out 1 (cli, etcd, prometheus) | 12 | 12 | 12/12 | 0 |
| Held-out 2 (7 repos) | 72 | 72 | 72/72 | 0 |
| Held-out 3 (8 repos) | 54 | 37 | 36/36 | 1 (rclone `TestTests`) |

Counts are whole-repo scans at today's checkouts; the ground-truth files hold 64, 17, 79 and 54 rows because earlier rounds already removed some false-positive rows from the output.

Per kept tag, over all four ground-truth sets (214 rows; correct / predicted for precision, correct / truth for recall):

| Tag | Precision | Recall |
|---|---|---|
| empty-stub | 13/14 | 13/13 |
| race-smoke | 36/39 | 36/36 |
| compile-check | 51/51 | 51/54 |

These tags were kept because they matched in all three held-out rounds, so the table is a fit to that data, not a generalisation estimate.

## After round 6

Held-out 4 (`HELDOUT4.md`) found three false-positive classes (14 findings). Rule change only, no tagger change: an env-gated subprocess entrypoint (`if os.Getenv(..) == "" { return }` then a `Serve*`/`Run*`/`Main*`/`Start*`/`Listen*` call) is not reported; a struct literal carrying the testing handle into an unresolved call whose name contains check/verify/assert/require/expect/validate/contract/test counts as asserting; a mockery `.EXPECT()...Panic(..)` or testify `.On(..).Panic(..)` chain counts as a guarded panic. Details and the exact rules are in `HELDOUT4.md` "After round 6".

Guardrails (whole-repo scans, `comm` on `file:line`): in-sample 65 -> 65 (64/64 true positives), held-out 1 12 -> 12 (12/12), held-out 2 72 -> 72 (72/72), held-out 3 37 -> 37 (36/36, rclone `TestTests` still the one false positive), held-out 4 82 -> 68 (68/68). Only the 14 targeted false positives disappeared.

All four held-out sets (1, 2, 3, 4) are now tuning data. The 0 remaining false positives on held-out 4 is a fit, not a generalisation estimate; any accuracy claim needs a fresh batch of repos.
