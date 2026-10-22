# Held-out evaluation 10: test-no-assertion-go rule and oracle tagger

Tenth held-out run on repos absent from held-out sets 1 to 9, from `docs/backtest-repos.md`, and from `~/code/github.com/` at the start of the run (checked by script: no repo of the 30 appears in any `heldout*-findings.tsv`, `*ground-truth.jsonl`, `HELDOUT*.md` or `RESULTS.md`; the first draft list contained `kubernetes-sigs/controller-runtime`, `kubernetes-sigs/kustomize` and `nats-io/nats.go`, which that check rejected, and they were replaced by `kubernetes-sigs/karpenter`, `argoproj/argo-rollouts` and `IBM/sarama` before any clone or count). Binary: `kibitzer-r11`, built from `7dd9c74` of `sdd/test-effectiveness-critique-impl`. No rule or tagger code changed. Method follows [HELDOUT9.md](HELDOUT9.md): ground truth ([heldout10-ground-truth.jsonl](heldout10-ground-truth.jsonl)) was committed as `4b77b23` before any `KIBITZER_ORACLE_TAGS=1` output was run or viewed; the findings list is [heldout10-findings.tsv](heldout10-findings.tsv). Scratch and scripts are under `~/.cache/kibitzer-eval/heldout10/`.

## Repos

The list of 30 was fixed before any finding count was seen; none was added or dropped afterwards, including the 8 that produced nothing. Shallow clones (`--depth 1`) under `~/code/github.com/<owner>/<repo>`. 18,247 `*_test.go` files in total (counted with `find`, excluding `vendor/`).

Two groups, fixed up front: 18 large or mid-large repos (large suites, to get usable n) and 12 small or medium well-regarded libraries and CLIs (the real-world mix). By findings, the 18 large contribute 125 of 148 (84.5%) and the 12 small/medium contribute 23 (15.5%). By test-file count (the better size axis, because the planned groups blur): 10 repos with 300 or more test files contribute 77 findings (52.0%), 9 with 100 to 299 contribute 51 (34.5%), 11 with fewer than 100 contribute 20 (13.5%).

| Repo | SHA (12) | Test files | Findings (all = labeled) | Group |
|---|---|---|---|---|
| GoogleContainerTools/skaffold | `035e3ddc3589` | 479 | 16 | large |
| redis/go-redis | `454251f54bc5` | 268 | 21 | large |
| siderolabs/talos | `d7637cc31a76` | 912 | 13 | large |
| googleapis/google-cloud-go | `9a2913189e55` | 3290 | 11 | large |
| kyverno/kyverno | `14096ce879e7` | 597 | 10 | large |
| openfga/openfga | `22c4d5ddf4d5` | 183 | 9 | large |
| argoproj/argo-rollouts | `37eae1d641c3` | 153 | 8 | large |
| open-telemetry/opentelemetry-collector-contrib | `0328aa347efd` | 3861 | 8 | large |
| vmware/govmomi | `465fded7cf3a` | 231 | 7 | large |
| cortexproject/cortex | `dd856d68163e` | 416 | 6 | large |
| rancher/rancher | `e8d3faea3cf1` | 532 | 6 | large |
| aws/aws-sdk-go-v2 | `0d2e6fa84ddf` | 4838 | 4 | large |
| grafana/k6 | `6028d79cb783` | 330 | 2 | large |
| quic-go/quic-go | `54d1c1bc626c` | 234 | 2 | large |
| kubernetes/ingress-nginx | `dbb11b92ddb7` | 89 | 1 | large |
| kubevirt/kubevirt | `de56e97e7462` | 871 | 1 | large |
| k3s-io/k3s | `65d71277db7a` | 96 | 0 | large |
| kubernetes-sigs/karpenter | `fff8e4dbef9d` | 177 | 0 | large |
| sirupsen/logrus | `6ef0748d1177` | 26 | 7 | small/medium |
| urfave/cli | `10f69fd3c38d` | 33 | 7 | small/medium |
| tidwall/gjson | `690362d6edf4` | 1 | 4 | small/medium |
| IBM/sarama | `f34447e311f8` | 163 | 3 | small/medium |
| derailed/k9s | `e51ac53cb4c3` | 215 | 1 | small/medium |
| uber-go/fx | `d5da5b04ac90` | 60 | 1 | small/medium |
| go-playground/validator | `a5843c6fbb59` | 26 | 0 | small/medium |
| google/go-cmp | `b133f1f1932e` | 10 | 0 | small/medium |
| gorilla/mux | `db9d1d0073d2` | 11 | 0 | small/medium |
| hashicorp/memberlist | `5cd2c4b0fb09` | 16 | 0 | small/medium |
| segmentio/kafka-go | `2e0b3968aa51` | 111 | 0 | small/medium |
| wagoodman/dive | `d6c691947f8f` | 18 | 0 | small/medium |

148 findings in 114 files across 22 repos with findings. Cap: 40 labeled per repo, with an even stride over findings sorted by file:line (`j = floor(i*n/40)+1`) had any repo exceeded it. None did (max 21, go-redis), so every finding was labeled: 148 of 148, 0% unlabeled. Unlike rounds 8 and 9 there is no cap-induced sampling bias.

Style mix actually present (observed, not assumed): gomock/hand-rolled fakes and the Kubernetes controller style (argo-rollouts, kyverno, karpenter, rancher, kubevirt), a custom test-wrapper framework with its own assertion methods (skaffold `testutil.Run`/`t.Check*`), Ginkgo/Gomega used from plain `Test*` functions (go-redis), heavy concurrency and `-race` tests (go-redis, cortex, logrus, sarama, kyverno), recent Go syntax (`for range N`, `wg.Go`, `t.Context()`, generics: all parsed), generated-client mega-suites (aws-sdk-go-v2, google-cloud-go), coverage-placeholder packages (talos). I did not verify which repos use testify suites, and no repo wraps `main()` in a flag-gated coverage harness this round; `govmomi/govc/main_test.go` has a `TestMain(t *testing.T)` that is an ordinary test, not the harness hook.

## Method

1. Listed findings with `check native test-no-assertion-go <file>` (no tag env) over every non-vendor `*_test.go`, sorted by repo, file, line.
2. Labeled before any tagger output: one label per finding after reading the function body (first 40 lines) and, where it depended on it, the tail or a helper (skaffold `status_check_test.go` and `verify_test.go` tails; cortex `testMemcachedStopping`; logrus `runEntryLoggerRace`; rancher `middleware_test.go:432` tail; k6 `collect_test.go:263` tail; go-redis `tryDial`).
3. Committed ground truth (`4b77b23`), then ran with `KIBITZER_ORACLE_TAGS=1`, parsed `[tag=...]`, joined with awk.

Labeling judgments, same as round 9 where they apply (all stated before tagger output):

- Gated `main()` wrapper = not a test; ungated `main()` call = no-panic-smoke; `_*` directories are fixtures. None of these occurred.
- A body that only exercises nil, zero, empty, edge or idempotent inputs and checks nothing = no-panic-smoke, with or without "panic" in the comment; an ordinary-path call with the result ignored, `t.Log`-only, or "for coverage" = untagged.
- `var _ I = x` (optionally followed by calling a method and discarding the result) = compile-check.
- Comment-only or empty bodies (including `t.Run` subtests holding only comments, and a `suite.Run` call that is commented out) = empty-stub.
- Goroutines plus a `Race`/`Concurrent`/`Parallel` name or comment = race-smoke; a test whose point is that a call returns rather than blocks = liveness-smoke; start/stop a controller or manager = lifecycle-smoke.
- Assertions through a wrapper type's own methods, or through gomega `Expect`, count as assertions (false-positive). A test whose only failure mode is a callback error that makes the code under test retry forever (hang then timeout) is labeled false-positive: a real, if indirect, oracle. That last one is a judgment call (see caveats).

## Ground-truth distribution

| Label | n |
|---|---|
| no-panic-smoke | 38 |
| race-smoke | 32 |
| untagged (log-only, coverage-only) | 21 |
| compile-check | 17 |
| empty-stub | 15 |
| false-positive | 14 |
| liveness-smoke | 6 |
| lifecycle-smoke | 5 |

## Rule precision

| Repo | n | FP | Loose | Strict TP |
|---|---|---|---|---|
| GoogleContainerTools/skaffold | 16 | 9 | 43.8% | 0 |
| redis/go-redis | 21 | 5 | 76.2% | 1 |
| all 20 other repos with findings | 111 | 0 | 100% | 14 |

Repos with findings and no FP: argo-rollouts 8, aws-sdk-go-v2 4, cortex 6, k9s 1, google-cloud-go 11, k6 2, sarama 3, ingress-nginx 1, kubevirt 1 (strict 1), kyverno 10, openfga 9, otel-contrib 8, quic-go 2, rancher 6 (strict 3), talos 13 (strict 10), logrus 7, gjson 4, fx 1, urfave/cli 7, govmomi 7.

| Slice | Value |
|---|---|
| Overall loose | 134/148 = 90.5% |
| Overall strict (empty-stub only; no `discarded-result` occurred) | 15/148 = 10.1% |
| Per-repo mean loose (22 repos with findings) | 96.4% |
| Per-repo median loose | 100% |
| Per-repo mean / median strict | 10.5% / 0% |
| Loose, same-file clusters counted once (114 files; 110 with a TP) | 110/114 = 96.5% |
| Strict, same-file clusters counted once | 15/114 = 13.2% |
| Loose excluding the 3 largest-FP repos (skaffold, go-redis, and the next, which has 0 FP) | 111/111 = 100% |
| Loose by size group: 10 repos with 300+ test files / 9 with 100-299 / 11 with under 100 | 68/77 = 88.3% / 46/51 = 90.2% / 20/20 = 100% |
| Loose, planned groups: 18 large / 12 small-medium | 111/125 = 88.8% / 23/23 = 100% |
| Loose if the 2 `try_dial` indirect-failure FPs counted as TP | 136/148 = 91.9% |

Only two repos have any FP, so "excluding the 3 largest-FP repos" removes two FP repos plus one arbitrary zero-FP repo and equals "excluding the two". The per-repo mean is high (96.4%) because 20 of 22 repos are at 100%. Strict is carried by talos: 10 of the 15 empty-stub findings are `TestEmpty` coverage placeholders in 10 different talos packages.

## False positives (14)

| Class | n | Where |
|---|---|---|
| Custom assertion methods on a wrapper `*testutil.T` (`t.CheckDeepEqual`, `t.CheckError`, `t.CheckErrorAndDeepEqual`) inside `testutil.Run(t, name, func(t *testutil.T){...})` | 9 | skaffold `pkg/skaffold/kubernetes/status/status_check_test.go:49,233,314,391,484,513,558`, `pkg/skaffold/verify/docker/verify_test.go:77,154` |
| Gomega `Expect(...)` assertions inside Ginkgo `It("...", func(){...})` called from a plain `Test*` function | 3 | go-redis `internal/util_test.go:28,44,57` |
| Failure signaled indirectly: the dialer callback returns an error when the context deadline is wrong, `tryDial` then retries forever (hang, test times out); no `t` assertion | 2 | go-redis `internal/pool/try_dial_test.go:11,30` |

No FP carries a tag (all 14 are untagged), so no tag-level damage from them.

Recurrence versus rounds 5-9:

- Callback or wrapper carries the assertion (round 2 hugo `t.Errorf` as argument; round 3 minio callback runners; round 9 delegation to a named helper and handle-holding collaborator): the skaffold wrapper-type assertion methods are the same underlying class in a new shape (assertions as methods on a custom test-context type). The round-2 fix covers `helper(t.Errorf, ...)`; it does not cover methods on a non-`*testing.T` receiver. 9 of the 14 FPs.
- New this round, no earlier counterpart found by grepping `HELDOUT*.md` and `RESULTS.md` for gomega, ginkgo, `t.Check`, testutil: Gomega `Expect` inside a Ginkgo `It` that is itself inside a plain `Test*` function (3). Whether that `It` actually executes under `go test` was not verified; the assertions are there by intent.
- Indirect failure signaled by hang or retry (2): new, and the weakest label of the 14 (see caveats).
- Did not recur: `main()` coverage and system-test harnesses, `benchdaily.Run` wrappers, `_sample`/`_*` fixture directories, helper-process re-exec stubs, gomock `Finish()` verification, `export_test.go` helpers named `Test*`, panic-as-assertion helpers, `t.Run`-passed generic helpers.

## Tagger results

| Tag | Predicted | Correct | Precision | Ground-truth count | Recall |
|---|---|---|---|---|---|
| empty-stub | 14 | 14 | 14/14 = 100% | 15 | 14/15 = 93.3% |
| race-smoke | 31 | 31 | 31/31 = 100% | 32 | 31/32 = 96.9% |
| compile-check | 8 | 8 | 8/8 = 100% | 17 | 8/17 = 47.1% |

No tag was wrong this round (the round-9 liveness-versus-race boundary, and empty-stub on fixtures, did not occur: there were no fixture directories and the liveness tests here, 6, carry no "race" wording).

Misses (11), causes not investigated (tagger code not read):

- compile-check (9): urfave/cli `flag_bool_with_inverse_test.go:601,607`, `flag_test.go:3703,3729,3735,3741,3747` (`var f Iface = &X{}` followed by calling a method and discarding the result with `_ =`; 7 findings, one idiom); go-redis `push/processor_unit_test.go:279` (`var _ I = (*T)(nil)` inside `t.Run` subtests); openfga `internal/listobjects/pipeline/internal/worker/receiver_test.go:282` (constructor call, `defer r.Close()`, then `var _ worker.Receiver[string] = r`). The 8 tagged ones were single-statement or all-`var _` bodies, and the kyverno `var _ ResourceSpec = result` after a call. Round 9 also had 5 compile-check misses on variants of the same family (grouped `var (...)`, `var _` plus a call).
- race-smoke (1): rancher `pkg/ext/stores/kubeconfig/store_test.go:2594` (`t.Parallel()` plus a `t.Run` subtest that calls `Stop` from two goroutines; the only goroutine is a `Stop` call, no `Race`/`Concurrent` wording; my label is itself borderline).
- empty-stub (1): go-redis `extra/redisotel-native/metrics_definitions_test.go:41` (five `t.Run` subtests whose bodies are only comments).

Share of true positives left untagged: 81 of 134 (60.4%): no-panic-smoke 38, untagged 21, liveness-smoke 6, lifecycle-smoke 5, compile-check 9, empty-stub 1, race-smoke 1. Of those, 11 are in categories the tagger claims; the other 70 are not. Round 9 was 63.2%.

## Comparison

| | Round 5 | Round 6 | Round 7 | Round 8 | Round 9 | Round 10 |
|---|---|---|---|---|---|---|
| Rule loose precision | 90.1% | 92.6% | 85.3% (96.0% by file) | 74.5% (77.8% by file) | 86.5% (86.5% by file) | 90.5% (96.5% by file) |
| empty-stub prec / recall | 5/5, 5/5 | 4/4, 4/6 | 19/19, 19/19 | 1/1, 1/1 | 12/15, 12/13 | 14/14, 14/15 |
| race-smoke prec / recall | 14/16, 14/14 | 10/11, 10/10 | 10/19, 10/13 | 21/21, 21/23 | 34/36, 34/34 | 31/31, 31/32 |
| compile-check prec / recall | 39/39, 39/43 | n/a | n/a, 0/2 | n/a, 0/2 | 30/30, 30/35 | 8/8, 8/17 |

## Caveats

- Same model family labeled and wrote the tagger; shared blind spots would hide at the boundaries. This round had no labeler-versus-tagger disagreement on a tag the tagger emitted, which is weaker evidence than a disagreement-free round with independent labelers would be.
- n is modest (148 findings, 22 repos, 114 files). Both false-positive classes that matter are concentrated: 9 of 14 FPs are one skaffold idiom (one repo, one test utility), and 5 are one go-redis package. Per-repo loose precision (mean 96.4%) and pooled loose precision (90.5%) differ because of that clustering, not because most repos are noisy.
- Concentration elsewhere: talos supplies 10 of the 15 empty-stub findings (identical `TestEmpty` placeholders); urfave/cli supplies 7 of the 17 compile-check findings and all but 2 of the 9 compile-check misses; go-redis supplies 21 findings, 12 of them race-smoke. Recall figures for compile-check (8/17) are therefore dominated by one repo's idiom.
- Judgment calls that move numbers: the 2 `try_dial` FPs (counting them TP gives 91.9% loose; the test does fail by timing out if the dialer sees the wrong deadline, so FP is defensible but it is not a conventional assertion); the 3 go-redis `It`-inside-`Test` FPs (assertions are present, execution under `go test` unverified); 38 no-panic-smoke labels include edge-input tests with no panic wording (my stated rule), and a stricter "only explicit wording" rule would move some of them to untagged without changing loose precision or any tagger metric.
- Labels were made by reading 40 lines of each function body, plus tails or helpers for the 7 cases listed in Method. One borderline classification (rancher `:2594`, race-smoke versus no-panic-smoke) was made without further checking. Files under `testdata/` and `_*` directories were not excluded from the scan, matching rounds 8 and 9; none contributed findings.
- Eight of 30 repos produced nothing (karpenter, k3s, validator, go-cmp, memberlist, kafka-go, mux, dive) and are reported as-is. By planned group, 6 of the 12 small/medium repos and 2 of the 18 large repos had no findings.
- The 30-repo list, not the yield, determined inclusion. The three replacements before cloning were forced by the novelty check, not by counts.
