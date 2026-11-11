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
| Python | not run | — | — | — | unmeasured | default |

The TypeScript sample is small (n=6), so the 67% is a weak estimate; it exceeds the 40% bar,
so TypeScript and JavaScript are not in `default_checks()` and can be enabled in
`.kibitzer/inspect.json`. Python and Kotlin are advisory defaults per the spec without a
large-corpus measurement; re-run this backtest on a Python and a Kotlin repo before trusting
their rates.
