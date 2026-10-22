# Held-out evaluation 7: test-no-assertion-go rule and oracle tagger

Seventh held-out run on repos absent from held-out sets 1 to 6 and from `docs/backtest-repos.md`. Binary: `kibitzer-r8`, built from `bef7943` of `sdd/test-effectiveness-critique-impl` (the round-7/8 rule fixes: skip unparsed Go bodies, accept testify `suite.Run`, handle-holding collaborators). No rule or tagger code changed. Method follows [HELDOUT6.md](HELDOUT6.md): ground truth ([heldout7-ground-truth.jsonl](heldout7-ground-truth.jsonl)) was committed as `22fae6c` before any `KIBITZER_ORACLE_TAGS=1` output was run or viewed.

## Repos

Eighteen repos were fixed up front for diversity (testify-suite-heavy, mock-heavy, CLIs, databases, infra, libraries) before any finding count was seen; none was added or dropped afterwards. Shallow clones (`--depth 1`) under `~/code/github.com/<owner>/<repo>`. No repo reached the 40-finding cap (largest is vitess at 39), so no sampling was applied and every finding was labeled.

| Repo | SHA (12) | Findings |
|---|---|---|
| vitessio/vitess | `479625a5aacf` | 39 |
| ethereum/go-ethereum | `7c5069001149` | 23 |
| mattermost/mattermost | `f292f95d5afa` | 16 |
| aws/amazon-ecs-agent | `e5e43bd984cd` | 11 |
| jaegertracing/jaeger | `ca047c72f34c` | 10 |
| golang-migrate/migrate | `504568a3cbd2` | 8 |
| thanos-io/thanos | `777ac4173c8c` | 8 |
| cosmos/cosmos-sdk | `f0fcee24d938` | 6 |
| argoproj/argo-workflows | `00850865d409` | 4 |
| cloudflare/cloudflared | `002a5ccaf75a` | 4 |
| projectcalico/calico | `0be3617ccada` | 4 |
| kubernetes-sigs/cluster-api | `c26c33a827b4` | 3 |
| cert-manager/cert-manager | `9267bbc3a7ae` | 2 |
| go-delve/delve | `a439ae64ea3c` | 2 |
| open-policy-agent/opa | `869cccac6712` | 1 |
| pocketbase/pocketbase | `6f370ea741f3` | 1 |
| sigstore/cosign | `8effbeac60ec` | 1 |
| ent/ent | `8037bc89bcab` | 0 |

143 findings in 101 distinct test files. Testify-suite and gomock/mockery-heavy repos (cosmos-sdk, mattermost, thanos, ecs-agent, vitess, cert-manager, calico) were included deliberately; none produced a `suite.Run` false positive.

## Method

1. Listed findings with `check native test-no-assertion-go <file>` over every non-vendor `*_test.go` ([heldout7-findings.tsv](heldout7-findings.tsv)).
2. Labeled before any tagger output: one label per finding after reading the function body and, where the body delegated, the helper it delegated to. Scratch under `~/.cache/kibitzer-eval/heldout7/` (`list.sh`, `dump.sh`, `labels.tsv`, `tagrun.sh`, `stats.sh`).
3. Re-ran with `KIBITZER_ORACLE_TAGS=1`, parsed `[tag=...]`, joined with awk.

Blindness: no tagger output or tagger source was seen before the commit. To confirm candidate false positives I read test-helper source (`helper_event_test.go`, cluster-api `util/conversion/conversion.go`, thanos `testMemcachedStopping`); that is helper code, not rule or tagger output.

Loose precision = TP/(TP+FP), every non-false-positive label counted TP. Strict counts only `discarded-result` and `empty-stub`. Empty `TestDummy` placeholders (jaeger, 3) and `TestNothing` (vitess) are labeled `empty-stub` mechanically, though they are intentional placeholders; they raise strict precision.

## Ground-truth distribution

| Label | n |
|---|---|
| no-panic-smoke | 51 |
| false-positive | 21 |
| empty-stub | 19 |
| liveness-smoke | 18 |
| untagged (log-only, construction-only, coverage-only) | 16 |
| race-smoke | 13 |
| compile-check | 2 |
| lifecycle-smoke | 2 |
| discarded-result | 1 |

## Rule precision

| Repo | n | FP | Loose | Strict TP |
|---|---|---|---|---|
| vitessio/vitess | 39 | 18 | 53.8% | 1 |
| ethereum/go-ethereum | 23 | 0 | 100% | 2 |
| mattermost/mattermost | 16 | 0 | 100% | 0 |
| aws/amazon-ecs-agent | 11 | 0 | 100% | 1 |
| jaegertracing/jaeger | 10 | 0 | 100% | 3 |
| golang-migrate/migrate | 8 | 0 | 100% | 8 |
| thanos-io/thanos | 8 | 0 | 100% | 2 |
| cosmos-sdk, argo-workflows, cloudflared, calico | 6/4/4/4 | 0 | 100% | 0/0/0/1 |
| kubernetes-sigs/cluster-api | 3 | 3 | 0% | 0 |
| cert-manager | 2 | 0 | 100% | 2 |
| delve, opa, pocketbase, cosign | 2/1/1/1 | 0 | 100% | 0 |

| Slice | Value |
|---|---|
| Overall loose | 122/143 = 85.3% |
| Overall strict | 20/143 = 14.0% |
| Per-repo mean loose (17 repos with findings) | 91.4% |
| Per-repo median loose | 100% |
| Per-repo mean / median strict | 17.7% / 0% |
| Loose, same-file clusters counted once (101 files) | 97/101 = 96.0% |
| Strict, same-file clusters counted once | 14/101 = 13.9% |
| Loose without the one vitess `vstreamer_test.go` file (18 FP) | 122/125 = 97.6% |

The overall figure is dominated by a single file: 18 of 21 false positives are in `vitess/.../vstreamer/vstreamer_test.go`. Counted per file, there are 4 FP files out of 101 and no file mixes FP and TP findings. Strict is dominated by golang-migrate (8 TODO stubs in 2 files) and the placeholders noted above.

## False positives (21)

| # | Where | Class |
|---|---|---|
| 18 | vitess `go/vt/vttablet/tabletserver/vstreamer/vstreamer_test.go` lines 412, 454, 490, 554, 578, 1336, 1381, 1423, 1469, 1495, 1694, 1901, 2136, 2437, 2798, 2880, 2910, 3031 | handle-holding collaborator: `ts := &TestSpec{t: t, ...}` then `ts.Run()`; `Run` asserts with `require.NoError(ts.t, err)` (`helper_event_test.go:393-432`). Lines 2910 and 3031 call `ts.Run()` inside a per-case `t.Run` closure. |
| 3 | cluster-api `bootstrap/kubeadm/webhooks/conversion/conversion_test.go:44`, `internal/topology/upgrade/test/t2/v1beta1/conversion_test.go:37`, `test/infrastructure/docker/api/v1beta1/conversion_test.go:37` | helper-returned test function: `t.Run(name, conversionutil.FuzzTestFunc(...))` / `SpokeConverterFuzzTestFunc(...)`; the returned `func(*testing.T)` builds `gomega.NewWithT(t)` and asserts (`util/conversion/conversion.go:195-203`, `:298-310`) |

Class recurrence versus earlier rounds:

- Handle-holding collaborator (round 6, coredns `writer{t: t}`): recurred in a new shape, a multi-field struct literal with the `t` field and a `Run()` method, after the `bef7943` fix. VERIFIED that the helper asserts through `ts.t`; NOT determined why the fix did not cover it (rule code not read, per instructions).
- Helper-returned test function passed to `t.Run`: a class I did not find in `docs/` (grep for "returned closure", "returns func(*testing", "fuzz" in `docs/backtest-triage/oracle-tagger/*.md` and `docs/*false-positives*.md` returned nothing relevant). Treat as new on the strength of that grep only; earlier FP lists were not diffed line by line.
- Go 1.26 `new(expr)` parse failure and testify `suite.Run`: did not recur (no such findings in this sample, which is not proof they are fixed).

No false positive carries a tag.

## Tagger results

| Tag | Predicted | Correct | Precision | Ground-truth count | Recall |
|---|---|---|---|---|---|
| empty-stub | 19 | 19 | 19/19 | 19 | 19/19 |
| race-smoke | 19 | 10 | 10/19 | 13 | 10/13 |
| compile-check | 0 | 0 | n/a | 2 | 0/2 |

- Wrong race-smoke tags (9, all labeled liveness-smoke): ecs-agent `TestProduceConsume`; go-ethereum `TestFeedOfSubscribeBlockedPost`, `TestFeedOfUnsubscribeBlockedPost`, `TestFeedOfUnsubscribeSentChan`, `TestFeedSubscribeBlockedPost`, `TestFeedUnsubscribeBlockedPost`, `TestFeedUnsubscribeSentChan`; vitess `TestDeadlockBwCloseAndReload`, `TestDeadlockBwCloseAndSchemaChange`. These are goroutines plus WaitGroup/channel where the oracle is "does not deadlock". This is the same liveness-versus-race boundary as pebble `TestFlusherCond` (round 5) and dolt `TestWaitForGC` (round 6). It is the tagger's only precision problem here and my labels are judgment calls on it (see caveats); 6 of the 9 come from the two near-identical `Feed`/`FeedOf` files.
- race-smoke misses (3): thanos `TestProxyDataRace` in `pkg/metadata`, `pkg/rules`, `pkg/targets` (same body shape): the test creates 100 stub endpoints and calls the proxy once with the result discarded; no goroutine is in the test body, so the race is internal to the code under test.
- compile-check misses (2): cosmos-sdk `internal/blockstm/txnrunner_test.go:502` (`var _ sdk.TxRunner = NewSTMRunner(...)`, a function-call right-hand side inside a test body) and mattermost `.../poster/default_poster_test.go:22` (`_ = NewPoster(&client.Post, botID)` inside `t.Run`). Not investigated further.
- empty-stub was exercised well (19): 7 TODO stubs and 1 empty body in golang-migrate, cert-manager `TestSync` x2, commented-out bodies, empty placeholders.

Share of true positives left untagged: 84 of 122 (68.9%): no-panic-smoke 51, untagged 16, liveness-smoke 9, race-smoke 3, compile-check 2, lifecycle-smoke 2, discarded-result 1. Of those, 5 are in categories the tagger does claim (3 race-smoke, 2 compile-check); the other 79 are in categories it does not claim. The one `discarded-result` (thanos `label_zlabel_test.go:474`, `reflect.DeepEqual(expectedResult, list)` result dropped after `sort.Sort`) is untagged.

## Comparison

| | Round 4 | Round 5 | Round 6 | Round 7 |
|---|---|---|---|---|
| Rule loose precision | 82.9% | 90.1% | 92.6% | 85.3% (96.0% by file; 97.6% without one vitess file) |
| empty-stub prec / recall | 8/8, 8/9 | 5/5, 5/5 | 4/4, 4/6 | 19/19, 19/19 |
| race-smoke prec / recall | 12/13, 12/13 | 14/16, 14/14 | 10/11, 10/10 | 10/19, 10/13 |
| compile-check prec / recall | 14/14, 14/15 | 39/39, 39/43 | n/a | n/a, 0/2 |

## Caveats

- Same model family labeled and wrote the tagger; the liveness-smoke versus race-smoke boundary, where all 9 tagger errors fall, is exactly where a shared blind spot would hide. A different labeler could reasonably call the `Feed` blocked-post tests race-smoke (2000 goroutines) and flip up to 9 errors, which would take race-smoke precision to 19/19.
- Per-tag n is small (19, 19, 2); do not read 19/19 or 0/2 as stable. Findings cluster heavily: 18 FP in one file, 6 near-duplicate `Feed`/`FeedOf` tests, 8 stubs in two golang-migrate files. File-level and per-repo figures above are the less inflated views.
- The loose precision drop from round 6 is one helper-pattern in one file; with it excluded the rule is at 97.6%. Neither number is a stable population estimate from 17 repos.
- Repos were fixed before counts were seen and not changed, but vitess happened to contribute 27% of findings; mocking-heavy suites (cosmos, mattermost, thanos) otherwise yielded few FPs.
- Some repos are very recent Go (`wg.Go`, `for range N`, `t.Context()`); those parsed without incident in this sample.
- Placeholder tests (`TestDummy`, `TestNothing`) count as strict true positives although they are intentional; excluding the 4 would give strict 16/143 = 11.2%.
