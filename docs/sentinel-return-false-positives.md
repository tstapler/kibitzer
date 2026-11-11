# sentinel-return: false positives and backtest

`sentinel-return-<lang>` (java, python, kotlin, typescript, javascript) flags `return -1` /
`return null` / `return None` inside a `catch`/`except` block — Fowler's Replace Error Code
with Exception. Go is excluded on purpose (returned errors are idiomatic there). v1 flags the
pattern alone; call-site cross-referencing is not implemented. Engine: `src/checkers/sentinel_return.rs`.

## Known false-positive patterns and mitigations

| Pattern | Handling |
|---|---|
| Search-style functions (`indexOf`, `find*`, `search*`, `lookup*`) where `-1`/`null` means "not found" | Suppressed by function-name substring |
| `tryX` and InputStream-style `read()` returning `-1` at EOF | Suppressed by name (`try` prefix, exact `read`) |
| Factory methods (`create*`, `make*`, `build*`, `new*`, `of`, `from*`, `valueOf`) returning `null` | Suppressed by name prefix, `null`/`None` only |
| Kotlin function declared `T?` (also Python `Optional`/`None`, TypeScript `null`/`undefined`) | `null`/`None` suppressed: the nullable type is the contract |
| Test files | Java and Kotlin default scope excludes `**/test/**` |
| Lambdas/nested functions inside a catch | Not attributed to the outer catch |

Still flagged (unfixed): `null` returned deliberately as "absent" from an un-annotated
function, e.g. a cache that stores `null` for "does not exist".

## Backtest (2026-11-10, debug build, `kibitzer check native` per file)

Triage is by reading the enclosing code of a sample, not exhaustive; a hit counts as a true
positive when a failure is swallowed into a sentinel callers must check.

| Language | Corpus | Hits | Reviewed | False positives | Rate | Decision |
|---|---|---|---|---|---|---|
| Java | `apache/cassandra` `src/` (3266 files) | 52 before the `try`/`read` suppressions, 46 after | every 4th hit (13) | 2 (`tryComponentFromFilename`, `RebufferingInputStream.read` → both now suppressed) | 2/13 ≈ 15% before, 0/11 reviewed-equivalent after | default |
| TypeScript | `microsoft/vscode` `src/` | 6 | all 6 | 4 (three `terminalLinkResolver.ts` "does not exist" cache nulls, one `launchMainService.ts` map-then-`coalesce`) | 4/6 ≈ 67% | **opt-in** |
| JavaScript | not run (shares the TypeScript engine; no corpus checked) | — | — | — | unmeasured | **opt-in** |
| Kotlin | `android/nowinandroid` | 0 | — | — | unmeasured at this size | default |
| Python | `servo/servo` (3266 `.py` files, mostly vendored WPT/third-party) | ≥40 (output truncated at 40; milvus not run) | 7 picked from the first 40 | 4 (`manifestexpected.py:43`, `wpttest.py:443`: KeyError→None for an optional key; `run_tc.py:385`: missing env var; `_inputstream.py:900`: iterator end). TPs: `browser.py:636`, `runner.py:121`, `manifest_build.py:83` | 4/7 ≈ 57% | **opt-in** |

The TypeScript sample is small (n=6), so the 67% is a weak estimate; it exceeds the 40% bar,
so TypeScript and JavaScript are not in `default_checks()` and can be enabled in
`.kibitzer/inspect.json`. Python also exceeds the bar (small sample, weak estimate) and is opt-in. Kotlin is an advisory default
with no hits on the only local corpus (`nowinandroid`); re-run on a larger Kotlin repo before trusting its rate. Python files have no
test-path exclusion in the default scope when enabled; add one in `.kibitzer/inspect.json`.
