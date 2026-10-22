# test-no-assertion-go on tstapler/stapler-squad: triage summary

Repo commit `d320c038ceea6eb1ec6e193f9611f1540a49f25f` (a live repo, not a frozen clone; see the staleness
caveat in `../README.md`). Checker: tip `28492c7` of `sdd/test-effectiveness-critique-impl`, i.e. with the
receiver/chain-helper, generic-instantiation and `t.Run(name, method)` improvements already in. Verdicts live in
`test-no-assertion-go.jsonl` (39 records; counts below derived with `jq` over that shard). The corpus has 602
`*_test.go` files; 32 of them produce findings.

## Counts

| Verdict | Count |
|---|---|
| true_positive, smoke-only (no-panic / `-race` / compile-time interface check / Start-Stop oracle) | 33 |
| true_positive, real (name promises a behavior, body discards the result) | 5 |
| false_positive | 1 |
| **Total** | **39** |

Smoke-only oracle split (hand-tallied from the notes): `-race` 12, no-panic 12, compile-time interface
satisfaction 6, Start/Stop 3.

Precision (loose) = TP / (TP + FP) = 38 / 39 = **97.4%**. Strict (real TP only) = 5 / 39 = **12.8%**.

## The one false positive is a scope bug, not a missed assertion

`tools/scanner/backend/testdata/handler_test.go:5` is `func TestCreateSession(t *testing.T) {}` in package
`services`. It sits under `testdata/`, which `go build`/`go test`/`go vet` ignore. It is a fixture input for the
`tools/scanner` HTTP/proto scanner (siblings: `handler.go`, `handler.pb.go`, `test.proto`), not a runnable
test, so "empty test" is meaningless there. The rule should skip any path with a `testdata` component. No
finding in this corpus is an "assertion exists but the rule missed it" false positive.

## False-positive causes, ranked

| Rank | Cause | Count |
|---|---|---|
| 1 | File under `testdata/` is a fixture, not a test | 1 |
| - | Missed assertion (helper/receiver/generic/cross-file) | 0 |

## True positives worth noting

The 5 "real" ones have a name that promises a check and a body that throws the answer away, so they pass
vacuously:

- `TestClaude_AsterismPattern_NoAiderFalsePositive` (`session/detection/asterism_test.go:151`): `_ = sd.Detect(...)`
- `TestApprovalDetector_NoDetection` (`session/detection/approval_test.go:184`): `_ = requests`
- `TestNewIdleDetectorWithDetector_should_acceptInjectedDetector` / `_should_createOwn_When_nilInjected`
  (`session/detection/idle_test.go:694`, `:719`): `_ = state`
- `TestProcessInspector_PermissionDenied_ReturnsEmptyNotError` (`session/procinfo/inspector_test.go:79`): `_ = err; _ = files`

Of the 33 smoke-only, several are cheap to upgrade: `TestWorktreePRPoller_SkipsEmptyRepoPath` and
`TestPool_SameKey_ConcurrentCalls_Serialized` (names claim skipping / serialization that nothing checks), and
`TestNativeProcessManager_ImplementsProcessManager` (body is only `t.Log`; the real check is the package-level
`var _`). Six tests are compile-time `var _ I = (*T)(nil)` inside a test function.

## Comparison to kubernetes

| | kubernetes (before) | kubernetes (after improvements) | stapler-squad (improved checker) |
|---|---|---|---|
| Findings | 47 | 27 | 39 |
| Precision (loose) | 55.3% | 96.3% | 97.4% |
| Strict (real TP only) | 10.6% | 18.5% | 12.8% |

The improvements appear to generalize: none of the four kubernetes FP causes (receiver/chain helpers, generic
instantiation, `t.Run` method values, cross-file helper) shows up here, on a codebase with a different style
(testify, `-race`-oracle concurrency tests, `var _ I = ...` interface checks). UNVERIFIED: how much of the
97.4% is the improvements versus stapler-squad being a lower-FP corpus, because the pre-improvement binary was
not run on it. The sample is 39 findings from a single author.

The remaining kubernetes miss (cross-file helper `assertFailures`) did not occur here; a cross-file helper
would still be missed, since the checker parses one file per run.

## Suggested rule improvements (rule code not changed)

1. Skip paths with a `testdata` (and `vendor`, `node_modules`) component. Removes the only FP here.
2. Split smoke-only from defect. 33 of 38 TPs are deliberate oracles. Recognise (a) `var _ I = (*T)(nil)` as the
   sole statement of a test, (b) doc/name/body-comment markers such as `-race`, `must not panic`,
   `no crashes`, `Concurrent`, and (c) `Start`/`Stop` bodies, and report them at lower severity or under a
   separate rule id.
3. Promote the "name promises, body discards" shape to a higher-confidence finding: test name contains an
   outcome word (`No...`, `Returns...`, `Skips...`, `Accept...`) and the only use of the result is `_ = x`.
   That is the 5 real TPs and the actionable class.
4. Keep the empty-body stub (`func TestX(t) {}`) as its own category, as suggested for kubernetes. Its only
   instance here was the `testdata` fixture, so this corpus is no evidence either way.
5. `t.Log(...)`-only bodies are correctly flagged; keep `t.Log` from counting as an assertion.

## After round 2

The `testdata` path skip removes the single FP (`tools/scanner/backend/testdata/handler_test.go:5`; regression fixture
`testdata/test-quality/go/scanner_handler_test.go`, paths tested in `test_no_assertion_should_SkipNonTestDirs_When_PathHasTestdataVendorOrThirdParty`).
Same repo commit `d320c038`; `find` + `xargs -P8` over `kibitzer check native <rule> <file>`, debug builds before/after, compared by `file:line` against the shard with `comm`.

| | Before | After |
|---|---|---|
| Findings | 39 | 38 |
| true_positive records still reported | 38 of 38 | 38 of 38 |
| false_positive records still reported | 1 | 0 |
| Findings not in the shard | 0 | 0 |

Precision = 38 / 38 = **100%** (was 97.4%); strict (real TP only) 5 / 38 = 13.2%.
