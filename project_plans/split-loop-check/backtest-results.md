# split-loop-go backtest results

Build: release binary at commit `61b414c`. Findings below are as-run; labels are one
reviewer's reading of the loop body (not a second opinion).

## 1. Transcript backtest (AC 7)

```sh
kibitzer check backtest split-loop-go --only-new   # and again without --only-new
# [kibitzer] scanned 4749 transcript(s), checked 97 snapshot(s), 1136 edit(s) unreconstructable
```

**0 findings** in both runs, so the false-positive rate is 0/0 — below the 40% bar only
vacuously. Only 97 Go snapshots were replayable, so this says the checker is quiet on
recent edits, not that it is precise. The corpus run below is the stronger evidence.

## 2. Real-world corpus (AC 8)

Command, per file (non-test `*.go`, excluding `vendor/`):
`kibitzer check native split-loop-go <file>`.

| Repo | Go files | Findings |
|------|---------:|---------:|
| `kubernetes/kubernetes` | 9,975 | 1 |
| `tstapler/stapler-squad` | 941 | 0 |
| supplementary: etcd, prometheus, cobra, cli, terraform, gitea, helm | 5,520 | 8 |
| supplementary: moby, grafana, containerd, istio, cockroach | 13,080 | 25 |

kubernetes and stapler-squad alone yield 1 finding, so the other repos from
`docs/backtest-repos.md`'s locally cloned set were added to reach ≥10 labeled samples.

### Labeled sample (12 of 34)

TP = loop really does two separable jobs; FP = splitting is wrong or pointless.

| # | Location | Accumulators | Label | Why |
|--:|----------|--------------|:-----:|-----|
| 1 | kubernetes `pkg/scheduler/schedule_one_podgroup.go:1144` | placementPodGroupAssignments, placementStates | FP | Parallel slices indexed together, built by ranging a map; two loops would give two different map orders and break the index pairing. |
| 2 | prometheus `web/api/v1/api.go:1310` | pools, n | FP | Collect keys + count (sum+count). |
| 3 | cli `api/queries_projects_v2.go:45` | inputs, mutations, variables | FP | Three builders in lockstep sharing the counter `i`. |
| 4 | terraform `configs/parser_config_dir.go:246` | diags, files | FP | Value + diagnostics from one call. |
| 5 | terraform `legacy/helper/schema/schema.go:1420` | es, ws | FP | Errors + warnings from one validation call. |
| 6 | gitea `models/unittest/fixtures_loader.go:96` | sqlArguments, sqlBuf | FP | SQL text and its args; reset together each row. |
| 7 | grafana `annotationsimpl/xorm_store.go:288` | args, valuesClause | FP | Placeholders and their args must stay aligned. |
| 8 | grafana `accesscontrol/database/seeder.go:456` | args, where | FP | Same placeholder/args pairing. |
| 9 | grafana `grafana-testdata-datasource/csv_data.go:135` | fieldNames, fieldRawValues | FP | Parallel per-column slices initialized together. |
| 10 | cockroach `pkg/cmd/vecbench/main.go:258` | sumElapsed, sumRecall, … | FP | Sums share one expensive per-iteration search; a split reruns it. |
| 11 | cockroach `pkg/sql/colexec/colexechash/hash.go:106` | v1, v2, v3, v4 | FP | Hash lanes fused deliberately for speed. |
| 12 | istio `operator/pkg/util/progress/progress.go:64` | comps, wait | TP (weak) | Two unrelated lists from one map walk; trivially splittable, low value. |

**Sample precision: 1 of 12 TP (~8%); ~92% FP.** This misses the item's own bar of
<40% FP (that bar is stated for the transcript run; the corpus run is the honest
measurement). Reason: the required behavior ("two `append` accumulators flagged",
AC 11) flags parallel-slice and lockstep-builder code, which dominates real Go. See
`docs/split-loop-false-positives.md`. Not tuned further here because the dominant
pattern is exactly what the acceptance tests require to be flagged.

Raw outputs: not committed (re-run the commands above).
