# Held-out evaluation 2: `test-no-assertion-go` and the oracle tagger

Second held-out run on repos never used to build or tune the rule or tagger.
Code under test: `origin/sdd/test-effectiveness-critique-impl` @ b5bab70, no code changed.
Inputs: `heldout2-findings.tsv` (79 rows), `heldout2-ground-truth.jsonl` (labels, committed in a53b6d3 before any tagger output).

## Method and blindness

1. Step 1: `kibitzer check native test-no-assertion-go <file>` over every non-vendor `*_test.go` of 8 shallow clones (`find | xargs -P8`). Recount: 79 findings, matching the expected figure per repo.
2. Step 2: for each finding I read the test body (and same-package helpers by grep where an assertion might hide) and assigned one ground-truth label. The labels were committed (a53b6d3) before step 3.
3. Step 3: the same files were re-run with `KIBITZER_ORACLE_TAGS=1`; the `[tag=...]` suffix was joined to the labels by `repo/file:line` and all metrics were computed with awk/jq (`/tmp/claude-1000/h2/metrics.sh`, not committed; scripts reproduce from the two committed data files plus a tagged run).

Blindness limits, stated plainly:

- I did not open `oracle_tag.rs`, the tagger docs, fixtures, or the tagger lines of the checker, and did not set the env flag before the labels were committed. That is verifiable from `git log` order only for the data files; the "did not read" part is a self-report.
- The labeller is the same model family that wrote the tagger. The tag vocabulary came from the task brief, so shared priors about what "race-smoke" looks like are likely and would inflate agreement.
- Labels are judgement calls. Two are borderline (see below) and both directions are reported.
- n = 79, but findings cluster hard: 46 are terraform, 42 of those are the same `var _ I = new(T)` idiom, 6 hugo findings are one file. Effective independent units are far fewer than 79. Treat every percentage as indicative, not an estimate with a tight interval.
- grpc-go produced 0 findings, so it contributes nothing to precision or the tagger.

## Findings and labels

| repo | findings | TP | FP | rule precision (loose) |
|---|---:|---:|---:|---:|
| hashicorp/terraform | 46 | 46 | 0 | 100% |
| gohugoio/hugo | 14 | 7 | 7 | 50% |
| nats-io/nats-server | 14 | 14 | 0 | 100% |
| caddyserver/caddy | 2 | 2 | 0 | 100% |
| traefik/traefik | 1 | 1 | 0 | 100% |
| jesseduffield/lazygit | 1 | 1 | 0 | 100% |
| spf13/cobra | 1 | 1 | 0 | 100% |
| grpc/grpc-go | 0 | 0 | 0 | n/a |
| **all** | **79** | **72** | **7** | **91.1%** |
| **excluding terraform** | **33** | **26** | **7** | **78.8%** |

Strict precision (true positive tagged `discarded-result` or `empty-stub` only): 1/79 = 1.3% overall, 1/33 = 3.0% excluding terraform. The one case is nats `TestSubjectTreeMatchNoCallbackDupe` (detects a duplicate callback and only `t.Logf`s it). No `empty-stub` occurred. In other words, on these repos nearly every finding is a smoke test, not a likely defect.

Ground-truth distribution: compile-check 43, race-smoke 15, no-panic-smoke 8, lifecycle-smoke 5, discarded-result 1, false-positive 7, empty-stub 0, liveness-smoke 0, untagged 0.

Borderline label: hugo `htmltemplate/clone_test.go:204 TestTemplateCloneExecuteRace` checks `Execute` errors and calls `panic(err)` inside goroutines. I labelled it false-positive (an error assertion exists, expressed as a panic). If you treat it as a true `race-smoke`, rule loose precision becomes 73/79 = 92.4% and the tagger exact-on-TP figure becomes 66/73 = 90.4%.

## False positives and their causes

| class | count | where | cause |
|---|---:|---|---|
| FP-callback-assertion | 6 | hugo `transform/urlreplacers/absurlreplacer_test.go` (TestAbsURL, TestAbsURLUnquoted, TestRelativeURL, TestAbsURLSrcSet, TestAbsXMLURLSrcSet, TestXMLAbsURL) | The test passes `t.Errorf` as a function value into a same-file helper (`apply(t.Errorf, ...)` -> `applyWithPath(ef errorf, ...)`) which compares expected to actual and calls `ef(...)`. The assertion exists, but it is reached through a callback parameter rather than a `t.*` call or a recognised helper receiving `t`. |
| FP-panic-as-assertion | 1 | hugo `htmltemplate/clone_test.go:204` | Error checked with `if err != nil { panic(err) }` inside goroutines; no `t.*` call. Borderline (see above). |

This is the key new result: one genuinely new FP class (the `t.Errorf`-as-argument callback), which accounts for 6 of 7 FPs but is a single file and a single idiom, so it says little about its frequency. Hugo's `go_templates` directory is vendored Go standard-library test code, which is also where several smoke tests live.

Not FP classes seen here: helpers in other files (no case needed a cross-file lookup to find a missed assertion; the six callback FPs were same-file).

## Tagger results

Tag matching treats `no-panic-smoke:uncued` as `no-panic-smoke`; 12 of the 14 no-panic predictions were `:uncued`, 2 cued. The tagger has no way to say false-positive, so every FP row is counted as a miss in "exact-match overall".

| measure | all (n=79) | excluding terraform (n=33) |
|---|---:|---:|
| exact match, overall (FPs count as miss) | 65/79 = 82.3% | 19/33 = 57.6% |
| exact match, true positives only | 65/72 = 90.3% | 19/26 = 73.1% |
| coverage on true positives (tagged at all) | 67/72 = 93.1% | 21/26 = 80.8% |
| tagged-and-correct among tagged TPs | 65/67 = 97.0% | 19/21 = 90.5% |

Per-repo exact match on true positives (clusters matter): terraform 46/46, hugo 7/7, nats 10/14, caddy 1/2, traefik 1/1, lazygit 0/1, cobra 0/1.

Per-tag precision and recall (precision counts FP rows tagged as that tag as wrong):

| tag | predicted | correct | precision | actual | recall |
|---|---:|---:|---:|---:|---:|
| compile-check | 42 | 42 | 100% | 43 | 98% |
| race-smoke | 14 | 13 | 93% | 15 | 87% |
| no-panic-smoke | 14 | 7 | 50% | 8 | 88% |
| lifecycle-smoke | 4 | 3 | 75% | 5 | 60% |
| discarded-result | 0 | 0 | n/a | 1 | 0% |
| empty-stub, liveness-smoke | 0 | 0 | n/a | 0 | n/a |

Confusion matrix (rows = ground truth, columns = tagger):

| truth \ tagger | compile-check | race-smoke | no-panic-smoke | lifecycle-smoke | untagged |
|---|---:|---:|---:|---:|---:|
| compile-check (43) | 42 | 0 | 0 | 1 | 0 |
| race-smoke (15) | 0 | 13 | 0 | 0 | 2 |
| no-panic-smoke (8) | 0 | 0 | 7 | 0 | 1 |
| lifecycle-smoke (5) | 0 | 0 | 1 | 3 | 1 |
| discarded-result (1) | 0 | 0 | 0 | 0 | 1 |
| false-positive (7) | 0 | 1 | 6 | 0 | 0 |

### Every disagreement (14)

| finding | truth | tagger | likely reason (my inference from the test body only; not checked against tagger source) |
|---|---|---|---|
| caddy `admin_test.go:163` TestLoadConcurrent | race-smoke | untagged | goroutines via `wg.Go(...)` (new Go 1.25 API) with no explicit `go` keyword |
| lazygit `thread_safe_map_test.go:47` | race-smoke | untagged | bare `go func` with no WaitGroup/channel sync; main goroutine loops |
| nats `events_test.go:1645` | no-panic-smoke | untagged | server setup plus one dropped-result call; no cue |
| nats `server_test.go:162` TestStartProfiler | lifecycle-smoke | no-panic-smoke (uncued) | Start then Close under a mutex; only the label differs |
| nats `test/gosrv_test.go:59` TestGoServerMultiShutdown | lifecycle-smoke | untagged | `RunServer` + double `Shutdown` |
| nats `stree_test.go:963` | discarded-result | untagged | `t.Logf` present, no assertion; the one real defect candidate is missed |
| cobra `completions_test.go:2896` | compile-check | lifecycle-smoke | blank-identifier assignments mixed with ignored `RegisterFlagCompletionFunc` calls |
| hugo `absurlreplacer_test.go` x6 | false-positive | no-panic-smoke (uncued) | rule FP; tagger cannot flag it |
| hugo `clone_test.go:204` | false-positive (borderline) | race-smoke | rule FP / borderline |

## Do the round-3 figures generalize?

Round-3 reference points: rule precision 70.6% on the first 17 findings; tagger 9/12 exact after tuning.

- Rule precision: only partially. Overall precision here is 91.1%, higher than 70.6%, but 42 of the 46 terraform findings are one trivially true `_impl` idiom and terraform is 58% of the sample. Excluding terraform it is 78.8% (26/33), and hugo alone is 50%. The honest summary is that precision is repo-dependent, the 70.6% figure was neither a ceiling nor a floor, and there is a new FP class (assertion through a `t.Errorf` function-value callback) the rule does not handle. Strict precision (a likely real defect) is 1.3%; the rule finds smoke tests, almost never defects.
- Tagger accuracy: the 9/12 (75%) in-sample figure is consistent with 19/26 (73.1%) on non-terraform true positives, so it does not obviously overfit, but the 90.3% overall figure is inflated by the terraform compile-check cluster (46/46) and should not be quoted. Coverage on non-terraform true positives is 80.8%; the tagger leaves `wg.Go`/bare-goroutine race tests, double-shutdown lifecycle tests, and the one discarded-result case untagged. The tagger has no channel for "this finding is a rule FP", so FP-callback tests receive a confident `no-panic-smoke:uncued`.
- Caveats: same-model labeller, n=33 outside terraform with heavy within-file clustering, and a self-reported blindness procedure. These data support "no evidence of overfitting; failure modes are concurrency-idiom coverage and no FP escape hatch", not a precise accuracy estimate.
