# test-no-assertion-kotlin on android/nowinandroid

Corpus: `android/nowinandroid` at `a49ed25`; the only repo in the backtest corpus with Kotlin tests (52 test files, 240 `@Test` functions). Verdicts: `test-no-assertion.jsonl` (the triage script names the shard by rule id).

| Run | Findings | TP | FP | Precision |
|---|---|---|---|---|
| 1 (before fix) | 45 | 1 | 44 | 1/45 = 2.2% |
| 2 (after golden-image fix) | 1 | 1 | 0 | 1/1 |

## False-positive cause (44 of 44)

Roborazzi screenshot tests. The oracle is `captureRoboImage`, called directly, through a same-file helper, or through `captureMultiTheme` / `captureMultiDevice` in a helper file. Fix: in a Kotlin file that mentions "screenshot", any `capture*` call, `verifyRoboImage` or `compareRoboImage` counts as an assertion. Regression fixtures: `ScreenshotTest.kt` (negative), `CaptureNonGoldenTest.kt` (positive).

The 44 FP records stay in the shard as history; they no longer reproduce, so `run` reports them stale.

## True positive

`TopicScreenTest.kt:121` (`news_whenSuccessAndTopicIsSuccess_isShown`) only calls `performScrollToNode`, which throws if the node is missing: a weak implicit oracle, not an empty test.

## Limits

- The wrapper heuristic (`capture*` plus the file-level "screenshot" mention) is name-based. A real fix needs a cross-file helper index; `assertion_helpers` is the escape hatch until then.
- Paparazzi and Shot idioms are not covered.
- One corpus is thin evidence: 240 tests, 1 finding after the fix. Re-run on a second Kotlin repo (JUnit/MockK/Turbine, no screenshot tests) once one is added to `docs/backtest-repos.md`.

## After round 2

Round 2 adds the shared path skip (`testdata`/`vendor`/`node_modules`/`third_party`) for Kotlin as well. Same commit
`a49ed25`; `find` + `xargs -P8` over `kibitzer check native <rule> <file>`, debug builds before/after, compared by `file:line` against the shard with `comm`.

| | Before | After |
|---|---|---|
| Findings | 1 | 1 |
| true_positive records still reported | 1 of 1 | 1 of 1 |

Precision unchanged at 1 / 1; the 44 historical FP records still do not reproduce. No Kotlin path under these
directory names exists in this corpus, so the skip is covered only by the unit test
(`test_no_assertion_should_SkipNonTestDirs_When_PathHasTestdataVendorOrThirdParty`).
