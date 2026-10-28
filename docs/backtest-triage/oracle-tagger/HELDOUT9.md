# Held-out evaluation 9: test-no-assertion-go rule and oracle tagger

Ninth held-out run on repos absent from held-out sets 1 to 8, from `docs/backtest-repos.md`, and from `~/code/github.com/` at the start of the run. Binary: `kibitzer-r10`, built from `3815595` of `sdd/test-effectiveness-critique-impl` (adds one normalizer for generic and parenthesized `t.Run` bodies). No rule or tagger code changed. Method follows [HELDOUT8.md](HELDOUT8.md): ground truth ([heldout9-ground-truth.jsonl](heldout9-ground-truth.jsonl)) was committed as `9fc6167` before any `KIBITZER_ORACLE_TAGS=1` output was run or viewed; the findings list is [heldout9-findings.tsv](heldout9-findings.tsv).

## Repos

Round 8 yielded only 51 findings from 18 repos, so this round deliberately favored large codebases with large test suites (hundreds to thousands of test files; 21,954 `*_test.go` files across the 26 repos, counted with `find` excluding `vendor/`). The list of 26 was fixed before any finding count was seen; none was added or dropped afterwards, including the five that produced nothing. Shallow clones (`--depth 1`) under `~/code/github.com/<owner>/<repo>`. Style mix: stdlib table-driven (golang/go), testify itself, gomock (tidb, kops), generics and recent Go syntax (dapr, autoscaler), helper-heavy (cockroach, milvus), e2e/integration (knative, boundary, teleport, tidb realtikvtest), Kubernetes controllers (external-dns, velero, crossplane, tekton, operator-sdk).

| Repo | SHA (12) | Test files | Findings (all) | Labeled |
|---|---|---|---|---|
| golang/go | `d8b7594888df` | 1987 | 207 | 40 (sampled) |
| milvus-io/milvus | `a62b1319ceff` | 1994 | 87 | 40 (sampled) |
| dapr/dapr | `1efeb205e0f5` | 490 | 34 | 34 |
| pingcap/tidb | `18ec04a2d8fb` | 2028 | 26 | 26 |
| elastic/beats | `ff3bd99adf67` | 1919 | 24 | 24 |
| cockroachdb/cockroach | `d30c905fff79` | 3183 | 17 | 17 |
| kubernetes/autoscaler | `1a2d57590368` | 228 | 14 | 14 |
| grafana/grafana | `e40de7898e1c` | 2454 | 11 | 11 |
| gravitational/teleport | `1283425b60ec` | 2135 | 6 | 6 |
| vmware-tanzu/velero | `6093ea050cc4` | 305 | 6 | 6 |
| grafana/alloy | `b76089e32e25` | 737 | 4 | 4 |
| golang/tools | `b65ac2ee2200` | 510 | 3 | 3 |
| kubernetes-sigs/external-dns | `db120154ddab` | 184 | 3 | 3 |
| libp2p/go-libp2p | `3c2c902122cb` | 200 | 3 | 3 |
| tektoncd/pipeline | `775821351e55` | 299 | 3 | 3 |
| crossplane/crossplane | `6dbabf1e17b5` | 127 | 2 | 2 |
| envoyproxy/gateway | `d4961845fa9b` | 212 | 2 | 2 |
| hashicorp/boundary | `eeb38a1a84eb` | 787 | 2 | 2 |
| knative/serving | `c4c11cb789b6` | 272 | 2 | 2 |
| stretchr/testify | `87a7b9d57689` | 21 | 2 | 2 |
| ipfs/kubo | `833e04a240d8` | 186 | 1 | 1 |
| kubernetes/kops | `0481af6c533b` | 301 | 0 | 0 |
| operator-framework/operator-sdk | `299157a814e6` | 116 | 0 | 0 |
| aquasecurity/trivy | `2f4aaf7e7caa` | 598 | 0 | 0 |
| anchore/syft | `b55707f60fdd` | 509 | 0 | 0 |
| buildpacks/pack | `9b483c2428d8` | 172 | 0 | 0 |

459 findings in 301 files across 21 repos with findings. Cap: 40 labeled per repo. Two repos exceeded it (golang/go 207, milvus 87); for those the sample is a deterministic even stride over findings sorted by file:line (`j = floor(i*n/40)+1` for `i` in `0..39`), so it spans the whole tree rather than the first 40. 245 findings in 208 files were labeled (53% of all findings). Because the cap bites only on go and milvus, the other 19 repos are complete.

## Method

1. Listed findings with `check native test-no-assertion-go <file>` over every non-vendor `*_test.go` (`raw.tsv` -> sorted -> capped).
2. Labeled before any tagger output: one label per finding after reading the function body (first 40 lines) and, where it delegated, the helper (golang/go `testStopResetResult`, `RunGetgThreadSwitchTest`, `testZeroRange*`; tidb `tearDownTest`; the truncated tails of `runner_race_test.go` and `task_search_test.go`). Scratch under `~/.cache/kibitzer-eval/heldout9/`.
3. Committed ground truth, then re-ran with `KIBITZER_ORACLE_TAGS=1`, parsed `[tag=...]`, joined with awk.

Label vocabulary as in earlier rounds: empty-stub, race-smoke, compile-check, no-panic-smoke, lifecycle-smoke, liveness-smoke, untagged (log-only or coverage-only), false-positive. Loose precision = TP/(TP+FP); strict counts only empty-stub (and discarded-result, of which there was none).

## Ground-truth distribution

| Label | n |
|---|---|
| no-panic-smoke | 79 |
| compile-check | 35 |
| race-smoke | 34 |
| false-positive | 33 |
| untagged (log-only, coverage-only) | 22 |
| liveness-smoke | 21 |
| empty-stub | 13 |
| lifecycle-smoke | 8 |

## Rule precision

| Repo | n | FP | Loose | Strict TP |
|---|---|---|---|---|
| cockroachdb/cockroach | 17 | 0 | 100% | 2 |
| crossplane/crossplane | 2 | 0 | 100% | 0 |
| dapr/dapr | 34 | 0 | 100% | 0 |
| elastic/beats | 24 | 12 | 50.0% | 2 |
| envoyproxy/gateway | 2 | 0 | 100% | 0 |
| golang/go | 40 | 2 | 95.0% | 1 |
| golang/tools | 3 | 0 | 100% | 0 |
| grafana/alloy | 4 | 0 | 100% | 1 |
| grafana/grafana | 11 | 0 | 100% | 3 |
| gravitational/teleport | 6 | 5 | 16.7% | 0 |
| hashicorp/boundary | 2 | 1 | 50.0% | 1 |
| ipfs/kubo | 1 | 1 | 0% | 0 |
| knative/serving | 2 | 1 | 50.0% | 0 |
| kubernetes/autoscaler | 14 | 0 | 100% | 0 |
| kubernetes-sigs/external-dns | 3 | 0 | 100% | 0 |
| libp2p/go-libp2p | 3 | 0 | 100% | 0 |
| milvus-io/milvus | 40 | 0 | 100% | 1 |
| pingcap/tidb | 26 | 11 | 57.7% | 1 |
| stretchr/testify | 2 | 0 | 100% | 0 |
| tektoncd/pipeline | 3 | 0 | 100% | 0 |
| vmware-tanzu/velero | 6 | 0 | 100% | 1 |

| Slice | Value |
|---|---|
| Overall loose | 212/245 = 86.5% |
| Overall strict | 13/245 = 5.3% |
| Per-repo mean loose (21 repos with findings) | 81.9% |
| Per-repo median loose | 100% |
| Per-repo mean / median strict | 7.0% / 0% |
| Loose, same-file clusters counted once (208 files; 180 with a TP) | 180/208 = 86.5% |
| Strict, same-file clusters counted once | 11/208 = 5.3% |
| Loose excluding beats, tidb, teleport (189 findings, 5 FP) | 184/189 = 97.4% |
| Loose if the 14 main()-harness FPs were instead counted as TP smoke tests (see caveats) | 226/245 = 92.2% |

The per-repo mean is dragged down by three repos where one repeated shape (10 near-identical `TestSystem` wrappers in beats, 7 `TestBenchDaily` in tidb, 5 `_sample` fixtures in teleport) produces 28 of the 33 FPs. Two files contain both an FP and a TP finding.

## False positives (33)

| Class | n | Where |
|---|---|---|
| Coverage/system-test harness that calls `main()` (gated by a flag or env, or unconditional `TestRunMain`) | 14 | beats `auditbeat/main_test.go:41`, `heartbeat/main_test.go:40`, `libbeat/libbeat_test.go:38`, `packetbeat/main_test.go:40`, `x-pack/auditbeat/main_test.go:28`, `x-pack/heartbeat/main_test.go:26`, `x-pack/libbeat/libbeat_test.go:25`, `x-pack/osquerybeat/main_test.go:27`, `x-pack/packetbeat/main_test.go:25`, `x-pack/winlogbeat/main_test.go:25`; kubo `cmd/ipfs/runmain_test.go:18`; tidb `br/cmd/br/main_test.go:71`, `cmd/tidb-server/main_test.go:57`, `lightning/cmd/tidb-lightning/main_test.go:25` |
| `benchdaily.Run(...)` benchmark-runner wrapper (not a test) | 7 | tidb `pkg/distsql/bench_test.go:69`, `pkg/executor/test/splittest/split_table_test.go:644`, `pkg/planner/core/casetest/partition/list_partition_integration_test.go:266`, `pkg/planner/core/casetest/tpcds/tpcds_test.go:225`, `pkg/planner/core/tests/partition/bench_test.go:1482`, `pkg/statistics/bench_daily_test.go:23`, `pkg/util/ranger/bench_test.go:264` |
| Fixture sample tests under a `_`-prefixed directory (ignored by the go tool; inputs to the render-tests tool) | 5 | teleport `build.assets/tooling/cmd/render-tests/_sample/package1/foo_test.go:23,29`, `package2/bar_test.go:23`, `package3/baz_test.go:23,27` |
| Helper-process re-exec stub (gated mode, not a test) | 2 | beats `dev-tools/mage/gotest_test.go:147,344` |
| Direct delegation to a named helper that asserts or panics on failure | 2 | go `src/time/sleep_test.go:892` (`testStopResetResult(t, true)` asserts via `t.Error`), `src/runtime/proc_test.go:1115` (`RunGetgThreadSwitchTest` in `export_test.go` panics if `g1 != g2`) |
| Handle-holding collaborator (`suite.Execute(Configuration{T: t})`) | 1 | knative `test/upgrade/upgrade_test.go:30` |
| `export_test.go` helper that happens to be named `Test*` | 1 | boundary `internal/db/schema/export_test.go:15` |
| gomock controller `Finish()` in the teardown verifies the `EXPECT()` | 1 | tidb `pkg/lightning/backend/backend_test.go:331` |

Recurrence versus rounds 5-8:

- Handle-holding collaborator (round 6 coredns, round 7 vitess `ts.Run()`): recurred once (knative). Round 8 had none.
- Helper-process re-exec stub (an earlier round, cli/cli): recurred twice (beats).
- Delegation to a named helper (round 7 cluster-api, round 8 opentelemetry `t.Run(name, helper[N])`): the `t.Run`-passed generic-helper shape from round 8 did **not** recur (it is the case the `3815595` normalizer targets, and none of its shapes appeared here), but direct delegation `helper(t, ...)` and a panic-on-failure helper in `export_test.go` are new variants of the same underlying class.
- Mock-expectation assertion (argo-cd, earlier round): recurred once as gomock `Finish()`.
- Not-a-real-test fixture (rclone `fstest/runs`, earlier round): recurred as teleport `_sample` (5).
- New this round: `main()` coverage/system harness (14) and `benchdaily.Run` wrapper (7) are not in any earlier FP table I could find (grep over rounds 1-8 ground truth). Note that held-out 4 labeled mattermost `TestRunMain` (calls `main()`, nothing asserted) as no-panic-smoke, a TP; see caveats.
- testify `suite.Run`, Go 1.26 `new(expr)` parse failure: did not recur. Recent syntax (`wg.Go`, `for range N`, `t.Context()`, `t.Output()`, generics, `synctest`) parsed without incident.

Three FPs carry a tag: teleport `_sample/package2/bar_test.go:23`, `package3/baz_test.go:23,27` were tagged empty-stub (comment-only bodies in fixture files; the shape is right, the target is not a real test). No other FP is tagged.

## Tagger results

| Tag | Predicted | Correct | Precision | Ground-truth count | Recall |
|---|---|---|---|---|---|
| empty-stub | 15 | 12 | 12/15 = 80.0% | 13 | 12/13 = 92.3% |
| race-smoke | 36 | 34 | 34/36 = 94.4% | 34 | 34/34 = 100% |
| compile-check | 30 | 30 | 30/30 = 100% | 35 | 30/35 = 85.7% |

Errors:

- empty-stub wrong (3): the teleport fixture FPs above.
- race-smoke wrong (2): `golang/go src/time/sleep_test.go:94 TestTickerStress` and `libp2p p2p/net/swarm/dial_sync_test.go:198 TestStressActiveDial`, both labeled liveness-smoke by me (the oracle is receiving N ticks / all dials completing, with no race wording in the body). The round 5-7 liveness-versus-race boundary therefore recurred; both are name-driven ("Stress") judgment calls where either label is defensible. Tagger code not read.
- compile-check misses (5): cockroach `pkg/kv/kvserver/protectedts/protectedts_test.go:11` (a `var ( _ Provider; _ = ErrNotExists ... )` block), dapr `pkg/actors/targets/fake/fake_test.go:23` (`var _ targets.Factory = ...` plus `_ = fake.New("")`), dapr `pkg/runtime/hotreload/loader/fake/fake_test.go:23` (grouped `var (...)` with a generic instantiation), go `src/cmd/compile/internal/test/zerorange_test.go:25` (compile-only per its comment; helpers take `t` but never use it), go `src/runtime/slice_test.go:832` (comment "just checking that this compiles"). The other 30 compile-check findings (single-line `var _ I = x`, mostly dapr `fake` packages) were all tagged. Cause for the misses not investigated.
- empty-stub miss (1): grafana `apps/dashboard/pkg/migration/schemaversion/v36_test.go:7`, whose only statement is `t.Log("... to be implemented")`.

Share of true positives left untagged: 134 of 212 (63.2%): no-panic-smoke 79, untagged 22, liveness-smoke 19, lifecycle-smoke 8, compile-check 5, empty-stub 1. Of those, 6 are in categories the tagger claims; the other 128 are not. No `discarded-result` occurred.

## Comparison

| | Round 5 | Round 6 | Round 7 | Round 8 | Round 9 |
|---|---|---|---|---|---|
| Rule loose precision | 90.1% | 92.6% | 85.3% (96.0% by file) | 74.5% (77.8% by file) | 86.5% (86.5% by file) |
| empty-stub prec / recall | 5/5, 5/5 | 4/4, 4/6 | 19/19, 19/19 | 1/1, 1/1 | 12/15, 12/13 |
| race-smoke prec / recall | 14/16, 14/14 | 10/11, 10/10 | 10/19, 10/13 | 21/21, 21/23 | 34/36, 34/34 |
| compile-check prec / recall | 39/39, 39/43 | n/a | n/a, 0/2 | n/a, 0/2 | 30/30, 30/35 |

## Caveats

- Same model family labeled and wrote the tagger; shared blind spots would hide at the boundaries. The liveness-versus-race labels (2 disagreements) are exactly such a boundary.
- The FP class "calls `main()` from a test" is a labeling judgment: I treated flag- or env-gated coverage/system wrappers as not-a-test, while held-out 4 labeled an unconditional `TestRunMain` in mattermost as no-panic-smoke. Counting those 14 as TP smoke tests lifts loose precision from 86.5% to 92.2%. `benchdaily.Run` wrappers (7) and teleport `_sample` fixtures (5) are likewise judgment calls about what counts as a "test"; the tidb, beats and teleport clusters are 28 of the 33 FPs, so overall precision is dominated by three repos' conventions.
- Sampling: golang/go and milvus were capped at 40 of 207 and 87; the stride sample is unbiased with respect to line order but not to directory, and go's 40 are 19% of its findings. 214 of 459 findings (47%) were not labeled.
- n is modest (245 findings, 21 repos, 208 files) and 67% of the labeled findings (164 of 245) come from five repos (go, milvus, dapr, tidb, beats). Five of 26 repos (kops, operator-sdk, trivy, syft, pack) produced no findings and are reported as-is.
- Of the 34 race-smoke findings, 10 are in autoscaler, 4 in dapr and 4 in tidb (13 repos in all), so the 34/34 recall partly measures a few `Concurrent*`/`*Race` naming conventions.
- Empty-stub label judgment: placeholder bodies with only `t.Log("to be implemented")` (grafana v36) and the intentionally empty `go TestInit` (comment says "init must not panic") were labeled empty-stub.
- Labels were made by reading 40 lines of each function body; truncated tails were checked only where the label depended on them (envoy `runner_race_test.go`, milvus `task_search_test.go`). Files under `testdata/` and `_sample/` directories were not excluded from the scan, matching round 8.
