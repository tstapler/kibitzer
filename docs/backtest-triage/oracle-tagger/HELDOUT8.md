# Held-out evaluation 8: test-no-assertion-go rule and oracle tagger

Eighth held-out run on repos absent from held-out sets 1 to 7 and from `docs/backtest-repos.md`. Binary: `kibitzer-r9`, built from `12024aa` of `sdd/test-effectiveness-critique-impl` (adds compile-check after setup, race-smoke requires race intent, table-loop skip stubs). No rule or tagger code changed. Method follows [HELDOUT7.md](HELDOUT7.md): ground truth ([heldout8-ground-truth.jsonl](heldout8-ground-truth.jsonl)) was committed as `f5a1cb6` before any `KIBITZER_ORACLE_TAGS=1` output was run or viewed.

## Repos

Eighteen repos were fixed up front (style diversity: testify, gomock, table-driven stdlib, generics, helper-heavy, integration/e2e) before any finding count was seen; none was added or dropped afterwards. Shallow clones (`--depth 1`) under `~/code/github.com/<owner>/<repo>`. No repo reached the 40-finding cap (largest is opentelemetry-go at 32), so no sampling was applied and every finding was labeled. (The sampler, had it been needed, was an even stride over findings sorted by file:line.)

| Repo | SHA (12) | Findings |
|---|---|---|
| open-telemetry/opentelemetry-go | `5968638e2868` | 32 |
| casbin/casbin | `c35c93d29063` | 4 |
| grafana/mimir | `cb3c9c94fcb4` | 3 |
| prometheus/client_golang | `cc3d26c3366a` | 3 |
| jackc/pgx | `0dd05bbb0340` | 2 |
| rs/zerolog | `20ca4734f29c` | 2 |
| docker/compose | `20ca59586f67` | 1 |
| etcd-io/bbolt | `a4ef19710eb7` | 1 |
| go-gorm/gorm | `b3d3bf219f02` | 1 |
| hashicorp/hcl | `4a347c448ee9` | 1 |
| samber/lo | `5c6ddcb7063c` | 1 |
| spf13/viper | `528f7416c4b5` | 0 |
| go-chi/chi | `9f8adb1aabaa` | 0 |
| nats-io/nats.go | `94863ca27b3a` | 0 |
| uber-go/mock | `d8c3a2a4df48` | 0 |
| testcontainers/testcontainers-go | `7c8ce2644943` | 0 |
| kubernetes/minikube | `fb73a73e8ead` | 0 |
| sqlc-dev/sqlc | `1911a764fc85` | 0 |

51 findings in 36 distinct test files across 11 repos with findings. Yield was low (seven repos produced nothing); per the protocol the list was not extended, so n is small and a single repo (opentelemetry-go, 63% of findings) dominates.

## Method

1. Listed findings with `check native test-no-assertion-go <file>` over every non-vendor `*_test.go` ([heldout8-findings.tsv](heldout8-findings.tsv)).
2. Labeled before any tagger output: one label per finding after reading the function body and, where it delegated, the helper (`testFloat64ConcurrentSafe`, `testAggregationConcurrentSafe`, `validateGauge`/`validateHistogram`, and each `t.Run`-passed generic helper). Scratch under `~/.cache/kibitzer-eval/heldout8/`.
3. Re-ran with `KIBITZER_ORACLE_TAGS=1`, parsed `[tag=...]`, joined with awk.

Blindness: no tagger output or tagger/rule source was seen before the ground-truth commit. Loose precision = TP/(TP+FP); strict counts only `discarded-result` and `empty-stub`.

## Ground-truth distribution

| Label | n |
|---|---|
| race-smoke | 23 |
| false-positive | 13 |
| no-panic-smoke | 9 |
| compile-check | 2 |
| untagged (log-only, coverage-only) | 2 |
| empty-stub | 1 |
| lifecycle-smoke | 1 |

## Rule precision

| Repo | n | FP | Loose | Strict TP |
|---|---|---|---|---|
| open-telemetry/opentelemetry-go | 32 | 13 | 59.4% | 0 |
| casbin/casbin | 4 | 0 | 100% | 1 |
| every other repo with findings (9 repos, 15 findings) | 15 | 0 | 100% | 0 |

| Slice | Value |
|---|---|
| Overall loose | 38/51 = 74.5% |
| Overall strict | 1/51 = 2.0% |
| Per-repo mean loose (11 repos with findings) | 96.3% |
| Per-repo median loose | 100% |
| Per-repo mean / median strict | 2.3% / 0% |
| Loose, same-file clusters counted once (36 files) | 28/36 = 77.8% |
| Strict, same-file clusters counted once | 1/36 = 2.8% |
| Loose excluding opentelemetry-go (19 findings) | 19/19 = 100% |

All 13 false positives are in opentelemetry-go, in 8 files (`sdk/metric/exemplar/filter_test.go` 2, `sdk/metric/internal/aggregate/atomic_test.go` 3, `drop_test.go` 1, `exponential_histogram_test.go` 1, `finish_lastvalue_test.go` 3, `histogram_test.go` 1, `lastvalue_test.go` 1, and `sdk/metric/pipeline_registry_test.go` 1). No file mixes FP and TP findings. Strict is tiny because the only empty stub is `casbin persist/persist_test.go:29` (`// No tests yet`).

## False positives (13)

| # | Where (opentelemetry-go) | Class |
|---|---|---|
| 13 | `sdk/metric/exemplar/filter_test.go:15,36`; `sdk/metric/internal/aggregate/atomic_test.go:161,176,242`; `drop_test.go:15`; `exponential_histogram_test.go:1013`; `finish_lastvalue_test.go:209,249,353`; `histogram_test.go:295`; `lastvalue_test.go:509`; `sdk/metric/pipeline_registry_test.go:409` | helper function passed to `t.Run`: the test body is only `t.Run("Int64", helper[int64])` / `t.Run("Float64", helper[float64])` (generic instantiation of a named `func(*testing.T)`), or `t.Run(name, helper[N]())` where the helper returns `testAggregationConcurrentSafe(...)`'s closure. The helpers assert (`assert.Equal`, `require.Len`, `assert.Empty`, and `validateHistogram`/`validateGauge`/`validateExponentialHistogram` callbacks; e.g. `atomic_test.go:166-174`, `drop_test.go:20-27`). |

Class recurrence versus earlier rounds:

- Helper-returned or helper-passed test function given to `t.Run` (round 7 cluster-api `conversionutil.FuzzTestFunc(...)`): recurred, in two new shapes (a bare generic function value `helper[int64]`, and a call returning a closure built from a shared harness). It is the only FP class this round and accounts for 13/13. VERIFIED by reading each helper; NOT determined why the rule does not follow these (rule code not read, per instructions).
- Handle-holding collaborator (round 6 coredns, round 7 vitess `ts.Run()`): did not recur. This sample has no such shape, which is not proof it is fixed.
- testify `suite.Run`, Go 1.26 `new(expr)` parse failure: did not recur (no such findings; the sample is thin on testify). Recent Go syntax (`wg.Go`, `for range N`, `t.Context()`, generics) parsed without incident.

No false positive carries a tag.

## Tagger results

| Tag | Predicted | Correct | Precision | Ground-truth count | Recall |
|---|---|---|---|---|---|
| empty-stub | 1 | 1 | 1/1 | 1 | 1/1 |
| race-smoke | 21 | 21 | 21/21 | 23 | 21/23 |
| compile-check | 0 | 0 | n/a | 2 | 0/2 |

- No wrong tags. The round 5-7 liveness-versus-race boundary did not come up: no finding was a goroutine-plus-WaitGroup "does not deadlock" test labeled liveness-smoke, so this round says nothing new about that boundary.
- race-smoke misses (2): opentelemetry-go `internal/global/instruments_test.go:55` and `:100`. The goroutine and `setDelegate` race live in a shared helper (`testFloat64ConcurrentSafe`, `testInt64ConcurrentSafe`); the test body only builds instruments and calls the helper from `t.Run` closures. (Observation about the test shape; tagger code not read.)
- compile-check misses (2): mimir `pkg/mimirpb/compat_test.go:690` (`_ = labels.Label(LabelAdapter{})`) and `:696` (`_ = histogram.Span(BucketSpan{})`), a type conversion of a composite literal to another package's type. Not investigated further.
- empty-stub was exercised once (one finding), so it says little.

Share of true positives left untagged: 16 of 38 (42.1%): no-panic-smoke 9, race-smoke 2, compile-check 2, untagged 2, lifecycle-smoke 1. Of those, 4 are in categories the tagger claims; the other 12 are not. No `discarded-result` occurred.

## Comparison

| | Round 5 | Round 6 | Round 7 | Round 8 |
|---|---|---|---|---|
| Rule loose precision | 90.1% | 92.6% | 85.3% (96.0% by file) | 74.5% (77.8% by file; 100% without opentelemetry-go) |
| empty-stub prec / recall | 5/5, 5/5 | 4/4, 4/6 | 19/19, 19/19 | 1/1, 1/1 |
| race-smoke prec / recall | 14/16, 14/14 | 10/11, 10/10 | 10/19, 10/13 | 21/21, 21/23 |
| compile-check prec / recall | 39/39, 39/43 | n/a | n/a, 0/2 | n/a, 0/2 |

## Caveats

- Same model family labeled and wrote the tagger; shared blind spots would hide at the boundaries (here, liveness versus race-smoke did not even arise).
- n is tiny: 51 findings, 11 repos, 36 files. One repo contributes 63% of findings and all 13 FPs, and those 13 are one class in 8 files; the overall 74.5% is an artifact of that clustering. Per-repo mean/median (96.3% / 100%) and by-file figures (77.8%) bracket it; none is a stable population estimate.
- 23 of the 51 findings are race-smoke, 21 of them from opentelemetry-go's `*ConcurrentSafe` naming convention, so race-smoke precision 21/21 mostly measures that one codebase's idiom.
- Low yield (7 of 18 repos gave 0 findings, including testify- and gomock-centred ones such as uber-go/mock and testcontainers-go) is reported as-is; the list was not extended. Testify-suite coverage is therefore thinner than in round 7.
- Label judgment calls: `hcl TestRoundTripSafeConcurrent` (fire-and-forget goroutines, no wait) and `pgx TestContextWatcherUnwatchIsConcurrencySafe` were labeled race-smoke on name and shape; the tagger agreed. `samber/lo TestDebounce` (println-only subtests) was labeled log-only/untagged.
