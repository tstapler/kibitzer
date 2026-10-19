# Held-out evaluation 6: test-no-assertion-go rule and oracle tagger

Sixth held-out run on repos absent from held-out sets 1 to 5 and from `docs/backtest-repos.md`. Binary: `kibitzer-r7`, built from `2dd2a05` of `sdd/test-effectiveness-critique-impl` (the round-7 rule fixes). No code changed. Method follows [HELDOUT5.md](HELDOUT5.md).

## Repos

Fifteen repos were chosen for diversity (CLIs, databases, networking, infra, web frameworks, libraries) before any finding counts were seen; two produced zero findings. Shallow clones (`--depth 1`) under `~/code/github.com/<owner>/<repo>`. No repo reached the 40-finding cap (largest is 14), so no sampling was applied and every finding was labeled.

| Repo | Findings |
|---|---|
| temporalio/temporal | 14 |
| VictoriaMetrics/VictoriaMetrics | 10 |
| dolthub/dolt | 8 |
| coredns/coredns | 7 |
| gofiber/fiber | 4 |
| gin-gonic/gin | 2 |
| moby/buildkit | 2 |
| nektos/act | 2 |
| charmbracelet/bubbletea | 1 |
| containers/podman | 1 |
| go-git/go-git | 1 |
| junegunn/fzf | 1 |
| labstack/echo | 1 |
| hashicorp/raft | 0 |
| prometheus/node_exporter | 0 |

Commit SHAs (default-branch tips cloned 2026-10-19, read from each clone's `.git`): coredns `1550b867f762`, gin `0f09c3a9b462`, fzf `55c122f1a0e1`, bubbletea `f9df43c4f2c0`, go-git `3b930b688adc`, VictoriaMetrics `b7583b63122d`, act `4f411281417e`, raft `a5b469295ddc`, node_exporter `37ec35f6981b`, dolt `48829716caa0`, buildkit `64aefaef4d7d`, echo `8fd0e184dd39`, fiber `45156761f9c6`, podman `316518e4a753`, temporal `27897c115926`.

## Method

1. Listed findings with `check native test-no-assertion-go <file>` over every non-vendor `*_test.go`: 54 findings ([heldout6-findings.tsv](heldout6-findings.tsv)).
2. Labeled ground truth before any tagger output was seen (no `KIBITZER_ORACLE_TAGS=1` output, no tagger source). Each finding got one label after reading the function and same-file helpers ([heldout6-ground-truth.jsonl](heldout6-ground-truth.jsonl), committed as `fc99dea` before step 3).
3. Re-ran with `KIBITZER_ORACLE_TAGS=1`, parsed `[tag=...]`, joined with shell/awk. Scratch: `~/.cache/kibitzer-eval/heldout6/` (`tagrun.sh`, `joined.tsv`).

One caveat on blindness: while labeling the podman finding I ran the plain (untagged) rule on a modified copy of the file to find why it fired. That used rule output only, not tagger output.

Loose precision = TP/(TP+FP), every non-false-positive label counted TP. Strict counts only `discarded-result` and `empty-stub`.

## Ground-truth distribution

| Label | n |
|---|---|
| no-panic-smoke | 17 |
| race-smoke | 10 |
| empty-stub | 6 |
| liveness-smoke | 6 |
| lifecycle-smoke | 5 |
| untagged (log-only, construction-only) | 5 |
| false-positive | 4 |
| discarded-result | 1 |
| compile-check | 0 |

## Rule precision

| Repo | n | FP | Loose | Strict TP |
|---|---|---|---|---|
| temporalio/temporal | 14 | 2 | 85.7% | 1 |
| VictoriaMetrics/VictoriaMetrics | 10 | 0 | 100% | 0 |
| dolthub/dolt | 8 | 0 | 100% | 5 |
| coredns/coredns | 7 | 1 | 85.7% | 1 |
| gofiber/fiber | 4 | 0 | 100% | 0 |
| gin-gonic/gin | 2 | 0 | 100% | 0 |
| moby/buildkit | 2 | 0 | 100% | 0 |
| nektos/act | 2 | 0 | 100% | 0 |
| bubbletea, go-git, fzf, echo | 1 each | 0 | 100% | 0 |
| containers/podman | 1 | 1 | 0% | 0 |

| Slice | Value |
|---|---|
| Overall loose | 50/54 = 92.6% |
| Overall strict | 7/54 = 13.0% |
| Per-repo mean loose (13 repos with findings) | 90.7% |
| Per-repo median loose | 100% |
| Per-repo mean / median strict | 6.5% / 0% |
| Loose, temporal suite pair counted once | 50/53 = 94.3% |

Strict is dominated by dolt (5 of the 7 strict TPs: one dropped `Exec` result, four stub tests). Median-of-repos strict is 0: most repos' findings are real but benign smoke tests.

## False positives by cause

| Cause | n | Where |
|---|---|---|
| Go 1.26 `new(expr)` (e.g. `new("profiles/pod.json")`) in the test: the function body does not parse, so assertions in it are not seen. Confirmed by replacing `new("` with `ptr("` in a scratch copy: finding disappears. | 1 | podman `pkg/specgen/generate/kube/kube_test.go:191` |
| testify `suite.Run(t, s)` entry point; the assertions live in the suite's methods (77 and 3 `s.*` assertion calls respectively) | 2 | temporal `service/frontend/operator_handler_test.go:59`, `service/worker/scanner/executor/executor_test.go:24` |
| Assertions via an injected collaborator that holds `t` (`writer{t: t}`; `writer.Dnstap` calls `w.t.Error`) | 1 | coredns `plugin/dnstap/handler_test.go:132` |

Whether each class is new relative to rounds 1 to 5 was only checked by grepping `docs/` for `suite.Run` (no hits in earlier round docs); the other two were not diffed against earlier FP lists.

## Tagger results

| Tag | Predicted | Correct | Precision | Ground-truth count | Recall |
|---|---|---|---|---|---|
| empty-stub | 4 | 4 | 4/4 | 6 | 4/6 |
| race-smoke | 11 | 10 | 10/11 | 10 | 10/10 |
| compile-check | 0 | 0 | n/a | 0 | n/a |

- Wrong race-smoke tag (1): dolt `go/store/nbs/store_test.go:671` `TestWaitForGC`, labeled liveness-smoke (comment: wait must always return when the context is canceled) but it is 32 goroutines plus a WaitGroup, so the tag is borderline. This is the same shape as pebble `TestFlusherCond` in round 5.
- empty-stub misses (2): dolt `go/libraries/doltcore/diff/column_identity_test.go:64` and `:193`. The test builds a table and dispatches via `t.Run` to a helper whose body is `t.Skip("implement me")`.
- compile-check had no instances: no `var _ I = ...` test appeared in this sample, so that tag was not exercised.

Share of true positives left untagged: 35 of 50 (70.0%): no-panic-smoke 17, liveness-smoke 5, lifecycle-smoke 5, untagged 5, empty-stub 2, discarded-result 1. The tagger does not claim those categories. The one `discarded-result` (dolt `commands_test.go:52`, `Exec` return dropped) is untagged.

## Comparison

| | Round 4 | Round 5 | Round 6 |
|---|---|---|---|
| Rule loose precision | 82.9% | 90.1% | 92.6% |
| empty-stub prec / recall | 8/8, 8/9 | 5/5, 5/5 | 4/4, 4/6 |
| race-smoke prec / recall | 12/13, 12/13 | 14/16, 14/14 | 10/11, 10/10 |
| compile-check prec / recall | 14/14, 14/15 | 39/39, 39/43 | n/a |

## Caveats

- Same model family labeled and wrote the tagger; shared blind spots are possible (the liveness-smoke versus race-smoke boundary is the visible one).
- Per-tag n is tiny (4, 11, 0); do not read 100% or 90.9% as stable.
- Repos were chosen for diversity but after seeing the first batch's count (raft and node_exporter at 0, then a second batch was added to reach a usable n), so the sample size was influenced by yield even though no individual repo was picked for expected count.
- Median-of-repos is over 13 repos, 10 of which have 1 to 10 findings.
- Several repos use `sync.WaitGroup.Go` and `new(expr)`, i.e. very recent Go; the podman FP shows the parser lags there.
