# Held-out evaluation 5: test-no-assertion-go rule and oracle tagger

Fifth held-out run on nine repos never used for tuning. Binary: `kibitzer-r6`, built from tip `0ecc44e` of `sdd/test-effectiveness-critique-impl`. No code changed.

## Method

1. Listed findings with `check native test-no-assertion-go <file>` over every non-vendor `*_test.go` in nine shallow clones. Total: 131 findings ([heldout5-findings.tsv](heldout5-findings.tsv)).
2. Labeled ground truth blind to the tagger (no tagger source, fixtures, earlier result files, or `KIBITZER_ORACLE_TAGS=1` output seen). Each finding got one label after reading the function and any same-package helper it calls ([heldout5-ground-truth.jsonl](heldout5-ground-truth.jsonl), committed as b861059 before step 3).
3. Re-ran with `KIBITZER_ORACLE_TAGS=1`, parsed `[tag=...]`, and joined to ground truth with shell/awk. Scratch scripts and outputs: `~/.cache/kibitzer-eval/heldout5/` (`tagrun.sh`, `joined.tsv`).

Rule precision loose = TP/(TP+FP) with every non-false-positive label counted as TP. Strict counts only `discarded-result` and `empty-stub` as TP.

## Blindness limits

- The labeler is the same model family as the tagger's author, so shared blind spots are possible (for example what counts as "smoke").
- n is small. Clusters dominate: packer 43 (38 are `var _ I = new(T)` declarations), tailscale 40, and one ollama file supplies 11 of the 13 false positives. Treat per-tag figures outside packer/tailscale as anecdotes (n of 1 to 11).
- Labels are judgment calls where definitions overlap: `TestFlusherCond` (pebble) is liveness-smoke by its comment but also goroutines plus sync, so the race-smoke tag is arguably fine. Log-only tests (`t.Logf` of a result) are `untagged`, except two that compare to a want value and only log the mismatch (`discarded-result`).

## Ground-truth distribution

| Label | n |
|---|---|
| compile-check | 43 |
| no-panic-smoke | 23 |
| liveness-smoke | 14 |
| race-smoke | 14 |
| untagged (log-only, allocation-only, panic-as-assertion) | 13 |
| false-positive | 13 |
| empty-stub | 5 |
| discarded-result | 4 |
| lifecycle-smoke | 2 |

## Rule precision

| Repo | n | FP | Loose | Strict TP |
|---|---|---|---|---|
| hashicorp/packer | 43 | 0 | 100% | 3 |
| tailscale/tailscale | 40 | 1 | 97.5% | 3 |
| ollama/ollama | 16 | 12 | 25.0% | 0 |
| cilium/cilium | 12 | 0 | 100% | 1 |
| cockroachdb/pebble | 8 | 0 | 100% | 2 |
| grafana/tempo | 6 | 0 | 100% | 0 |
| controller-runtime | 3 | 0 | 100% | 0 |
| prometheus/alertmanager | 2 | 0 | 100% | 0 |
| kustomize | 1 | 0 | 100% | 0 |

| Slice | n | FP | Loose | Strict |
|---|---|---|---|---|
| All | 131 | 13 | 90.1% | 9/131 = 6.9% |
| Excluding packer and tailscale | 48 | 12 | 75.0% | 3/48 = 6.3% |
| All, ollama `reg_groups_test.go` cluster counted once | 121 | 3 | 97.5% | 9/121 = 7.4% |
| Excl. packer+tailscale, same collapse | 38 | 2 | 94.7% | 3/38 = 7.9% |

Strict precision is low by construction: most findings here are real but benign smoke tests (compile-check, race, no-panic), not defects.

## False positives by cause

| Cause | n | Where |
|---|---|---|
| New: test body only calls a same-file helper that dispatches to subtests via `t.Run(name, c.Run)` (method value taken from a registry of cases), so the assertions live in the registered cases; helper also `t.Skip`s when nothing selected | 11 | ollama `integration/reg_groups_test.go` (one file) |
| New: not a real test; subprocess helper that re-execs itself guarded by an env var (`TestRunnerJobHelper`, Windows-only file) | 1 | ollama `llm/runner_job_windows_test.go` |
| New: assertion reached through a function-value alias in a same-file helper (`f := tb.Errorf; if fatal { f = tb.Fatalf }; f(...)`) | 1 | tailscale `types/lazy/deferred_test.go:159` (`checkError`) |

All three classes are new relative to the classes this task named; I did not diff them against the earlier rounds' false-positive lists.

## Tagger results (tags emitted: compile-check, race-smoke, empty-stub)

| Tag | Predicted | Correct | Precision | Ground-truth count | Recall |
|---|---|---|---|---|---|
| empty-stub | 5 | 5 | 5/5 | 5 | 5/5 |
| race-smoke | 16 | 14 | 14/16 | 14 | 14/14 |
| compile-check | 39 | 39 | 39/39 | 43 | 39/43 |

Excluding packer and tailscale: empty-stub 1/1 and 1/1; race-smoke 10/11 and 10/10; compile-check 1/1 and 1/3.

Wrong race-smoke tags (2):
- tailscale `deferred_test.go:159`: a rule false positive (assertion via alias), so the tag is moot.
- pebble `TestFlusherCond`: labeled liveness-smoke (comment: a lost signal "causing the test to wedge"), borderline.

Compile-check misses (4): ollama `TestCodexIntegration` and `TestHermesIntegration` (the `var _ I = x` lines are inside `t.Run` subtests), tailscale `TestClientImplementsDialContext` (declares a local interface type and has two `var _` lines), tailscale `TestImplementsChonk` (slice literal of interface values plus `t.Logf`, labeled compile-check by intent).

Share of true positives left untagged: 59 of 118 (50.0%). By ground-truth tag: no-panic-smoke 23, untagged 13, liveness-smoke 13, compile-check 4, discarded-result 4, lifecycle-smoke 2. The tagger does not claim those categories, so this is the expected cost of emitting only three tags. The four `discarded-result` findings (pebble `TestDeck`/`TestWeighted`, tailscale `TestRemoveDatePrefix`/`TestExtractScopeAndParams`) are the likely real defects and none is tagged.

## Comparison with rounds 2 to 4

| | Round 2 | Round 3 | Round 4 | Round 5 |
|---|---|---|---|---|
| Rule loose precision | 91.1% | 66.7% | 82.9% (before last fixes) | 90.1% (97.5% with the ollama cluster counted once) |
| empty-stub prec / recall | | | 8/8, 8/9 | 5/5, 5/5 |
| race-smoke prec / recall | | | 12/13, 12/13 | 14/16, 14/14 |
| compile-check prec / recall | | | 14/14, 14/15 | 39/39, 39/43 |

## Do the final rule and tags hold up on new code?

- Rule: yes for the common patterns. Excluding one ollama file, 3 false positives in 121, each a new class (subtest-dispatch helper, subprocess helper, function-value alias of `t.Errorf`). The headline 90.1% (75.0% excluding packer and tailscale) is driven by one 11-finding file, so per-file clustering, not breadth of failure, explains it.
- Tags: precision holds (empty-stub 100%, compile-check 100%, race-smoke 87.5% with one borderline and one moot). Recall is high for race-smoke and empty-stub. compile-check recall dips (39/43) on `var _` inside subtests and on slice-literal or multi-declaration forms; packer's 38 trivial `var _` cases inflate the figure.
- Caveats: the sample is dominated by two repos, labeling was by one model family, and the tagger covers only half of true positives by design.
