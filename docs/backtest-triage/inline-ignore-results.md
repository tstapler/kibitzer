# inline-ignore Phase 4 results (Story 4.1.1)

Measured 2026-10-07 on branch `feat/inline-ignore-syntax`, release build, macOS (Darwin 25.6, arm64).
Plan: `project_plans/inline-ignore-syntax/implementation/plan.md`, Epic 4.1. Gate note:
`docs/backtest-triage/inline-ignore-gate.md`. Token costs below are inferred from character counts, not measured.

## 4.1.1a Transcript backtest

| Command | Result |
|---|---|
| `kibitzer check backtest inline-ignore` | 1471 transcripts scanned, 3551 snapshots checked, 3705 edits unreconstructable; **0** `[ignore-syntax]` fires, 0 `[unused-ignore]` fires, 6.3 s wall |

Nothing to triage. The count is zero partly because the transcripts predate the feature, so few snapshots
contain a `kibitzer:` comment; it shows the checker is quiet on historical code, not that agents use it.

## 4.1.1b Corpus and self run

The corpus clones from `scripts/clone-backtest-repos.sh` already existed under `~/code/github.com/` (the script skipped
all of them). `tstapler/consolette` and `tstapler/docspan` were added because they mention `kibitzer`.
`kibitzer run <repo>` prints nothing until it finishes, so a timeout yields no output. Caps: 900 s for deno, 300 s for the rest.

| Repo | `kibitzer run` | `[ignore-syntax]` | `[unused-ignore]` | other `inline-ignore` lines |
|---|---|---|---|---|
| BurntSushi/ripgrep | done, 15 s | 0 | 0 | 0 |
| android/nowinandroid | done, 13 s | 0 | 0 | 0 |
| rust-lang/rustc-dev-guide | done, 13 s | 0 | 0 | 0 |
| rust-lang/book | done, 37 s | 0 | 0 | 0 |
| microsoft/vscode-docs | done, 63 s | 0 | 0 | 0 |
| docker/docs | done, 76 s | 0 | 0 | 0 |
| kubernetes/website | done, 242 s | 0 | 0 | 0 |
| tstapler/consolette (mentions kibitzer in 10 files) | done, 20 s | 0 | 0 | 0 |
| tstapler/docspan (mentions kibitzer in 1 file) | done, 10 s | 0 | 0 | 0 |
| denoland/deno | **timed out at 900 s** | n/a | n/a | n/a |
| kubernetes/kubernetes, apache/cassandra, microsoft/vscode, servo/servo, mdn/content, gitlabhq/gitlabhq, tstapler/stapler-squad | **timed out at 300 s** | n/a | n/a | n/a |

Why the timeouts are not a skip: every default checker runs on every file, and these repos have 9k to 660k files
(servo 196,731; stapler-squad 663,312 including vendored trees), so a full `run` takes longer than the time box. They were
not retried with a longer cap. The inline-ignore-specific risks were covered separately:

| Check | Repos | Result |
|---|---|---|
| Files containing the string `kibitzer` (`rg -l -F kibitzer`, gitignore respected) | the 14 public corpus repos | **0 files**. The public corpus never mentions the tool, so the "prose or string mention" false-positive condition is vacuous there (see Limits) |
| `kibitzer check native inline-ignore` on every file that mentions `kibitzer` | tstapler/{consolette,docspan,kibitzer,stapler-squad}, 147 files | **0** output lines |
| In-scope (`go ts tsx js jsx mjs cjs py java kt kts rs md`) non-UTF-8 files in 10 corpus repos (k8s, cassandra, vscode, servo, deno, mdn, gitlab, nowinandroid, k8s website, docker/docs), copied to a scratch dir, `kibitzer run` | 15 files (servo 9, deno 6; the other eight 0) | 0 `inline-ignore` lines; 76 read-error lines from other checkers (unchanged behavior, plan Design Decision 11) |
| `kibitzer run` on a scratch dir with a non-UTF-8 `.go` and `.md` that contain `kibitzer` | 2 files | 0 `inline-ignore` lines |

Finding, not a defect of this feature: `kibitzer check native <name> <binary-file>` exits non-zero with `reading <file>: stream did
not contain valid UTF-8` for any checker, because the single-file CLI reads the file itself. The whole-tree `find ... | xargs`
workflow in `docs/backtest-repos.md` only hits it for binaries with an in-scope extension (the 15 files above). Not changed.

Self-run, `kibitzer run .` on this repo: 19.8 s, 1517 output lines, **0** lines tagged `[ignore-syntax]`, `[unused-ignore]`,
`[ignore-volume]` or `[blocking-suppressed]`, and no `N finding(s) suppressed inline` line, so nothing was suppressed by or
reported against the string-literal markers in sources and tests. Four output lines mention inline-ignore only because
`replace-magic-literal` quotes literals such as `"inline-ignore"` (`src/inline_ignores.rs:435-436`,
`tests/inline_ignore_cli.rs:75,121`); they are unrelated advisories.

`scripts/backtest-triage.py` was not used: it records verdicts for findings, and there were no inline-ignore findings to judge.

## 4.1.1c Fast path, latency, footer

Tests added in `src/inline_ignores.rs` (commit d6f08cd, formatted in 251d427), both passing:
`apply_inline_ignores_should_UseNoScanOrHash_When_NoKibitzerSubstring` (5,000-line Go source, 1,000 calls, `scans == 0`,
`hash_calls == 0`) and `apply_inline_ignores_should_StayWithinRatioOfSubstringSearch_When_NoKibitzerSubstring` (median of 5 of
1,000 calls at most 20x a `contains("kibitzer")` baseline plus 5 ms).

Hook-path raw rerun on the final code. Method: `kibitzer hook` subprocess, fresh cache per call, no daemon, Edit payload
inserting `// kibitzer:ignore long-function -- ...` (marker) or `// plain comment` (no marker) above a unique line near the top;
median of 5 after one discarded run; added = marker minus no marker (whole-process wall time, so noisier than the spike's
in-process timing). Two batches per file.

| File | No marker | Marker | Added |
|---|---|---|---|
| `src/checkers/rules.rs` (largest by bytes) | 215 / 206 ms | 242 / 240 ms | **27 / 34 ms** |
| `src/check.rs` (largest by lines) | 194 / 178 ms | 217 / 199 ms | **23 / 21 ms** |
| Task 2.2.3d spike (plan.md), rules.rs / check.rs | n/a | n/a | 26.9 / 25.9 ms (rerun alone 26.8 / 29.5 ms) |

Result: at most 34 ms added, well under the 150 ms target; consistent with the spike. The 2x ratio guard is the Task 2.2.3b
test; this run did not re-measure the rerun-to-first-pass ratio separately.

`ScanMemo` thrash: daemon started with an isolated socket, hook calls (Edit payload, marker) against two copies of the
largest files, median per call, n=10 each. Run 2 of 2 shown; in run 1 the alternating medians were about 20 ms slower than repeated (206 and 212 vs 184 ms), so the sign flips between runs and the difference is noise.

| Pattern | a.rs | b.rs |
|---|---|---|
| sequential, same file repeated | 196 ms | 156 ms |
| sequential, alternating a, b | 163 ms | 169 ms |
| two concurrent streams, different files | 203 ms | 203 ms |
| two concurrent streams, same file (n=20) | 216 ms | |

No consistent penalty from alternation or from two files in flight (the sign of the alternating-vs-repeated difference flips between runs; different-file concurrency was no slower than same-file). Per-call time is dominated by process start and the other checkers. **Decision: keep the memo;
the plan's drop-the-memo fallback does not apply.** Limit: the daemon exposes no scan counter, so this is latency evidence, not
a count of rescans, and the run-to-run noise is about 20 to 40 ms.

Footer break-even, `p* = (E x c_footer) / (S x c_re)` with E = 1389, S = 1617 (8085 re-report events x 0.20), c_re = 150
tokens, characters / 4 = tokens. Footer lengths measured with a temporary test in `src/hook.rs` (removed): typical Go case
590 characters (423 baseline + 167), longest case 450 (+27).

| Case | Net added | c_footer | p* | vs 0.5 line |
|---|---|---|---|---|
| Typical Go (2 rules, anchor at line 20) | +167 chars | 41.75 tokens | **0.239** | under |
| Cap (215 chars) | +215 chars | 54 tokens | **0.309** | under |
| Longest case (long markdown path, 7 rules) | +27 chars | 6.75 tokens | **0.039** | under |

The longest case is short because the footer sheds its two link sentences to fit the cap, so more rules costs fewer characters
there. `p*` stays at or below the 0.31 recorded in the gate note; the footer is not cut.

## 4.1.1d End-to-end ship gate

Counterfactual coverage (Phase 0 classes b and c): the class (b) cases are #7, #8, #10, #16 and class (c) is 0 of 4
(`inline-ignore-gate.md`). The classified file contents cannot be reconstructed: `/tmp/p0` holds only the replay output and
classification, and the backtest harness has no snapshot dump. So the literal "insert a directive into the reconstructed
file" check was not run. Proxy instead: all four rules (`duplicate-code-cross-file`, `syntax-rules-javascript` long-function,
`go-error-context`, `primitive-obsession`) have an anchor fixture in `src/inline_ignores_anchor_tests.rs`
(`ANCHOR_FIXTURES`, lines 20, 23, 25, 55; `ANCHOR_PENDING` is empty), which asserts a directive at the reported row drops the
finding. Coverage: **4 of 4 cases by rule, 0 of 4 on real file content**. It bounds achievable reduction, not adoption.

Manual end-to-end on a fixture Go file (`func render(page string, verbose bool)`, `flag-argument` at row 3), release binary,
isolated cache:

| Step | `kibitzer hook` PostToolUse | MCP `run_checks` (`kibitzer mcp`, stdio JSON-RPC) | `kibitzer run` |
|---|---|---|---|
| no ignore | finding + footer with `kibitzer:ignore flag-argument -- <why>`, "above line 3" | finding + same hint | finding + dismiss hint line |
| covering ignore above | no output (Write and Edit payloads) | `all checks passed` | `1 finding suppressed inline (rerun with --no-inline-ignores ...)`; `--no-inline-ignores` shows it |
| ignore removed | finding returns | finding returns | finding returns |

Malformed ignores (hook, Write payload): no reason gives `[ignore-syntax] kibitzer:ignore flag-argument has no reason. Write:
kibitzer:ignore flag-argument -- <why this is acceptable>`; `kibitzer:false-positive` gives `'kibitzer:false-positive' not
recognized; use 'kibitzer:ignore'`, and the finding is still reported. A directive two rows above, sent as an Edit touching it,
gives `[unused-ignore] ... matches no finding at line 2 or 3; the finding is at line 4. Move the comment to the line directly
above line 4`. As planned (Task 2.2.2b deferred), `kibitzer run` does not emit `[unused-ignore]`: the same misplaced directive
there reports the finding only.

Final gates: `cargo build` ok; `cargo test` 1426 passed, 0 failed (1381 unit, 9 + 2 + 8 + 10 + 16 integration);
`cargo clippy --all-targets` no warnings; `cargo fmt --check` clean.

## Limits

- The public corpus has no `kibitzer` strings, so it proves quiet-at-scale and binary safety, not precision on prose mentions;
  the precision evidence is the 147 mention files in the tstapler repos, this repo's self-run, and the unit tests.
- Eight repos (deno, k8s, cassandra, vscode, servo, mdn, gitlab, stapler-squad) did not finish a full `kibitzer run` inside the time box.
- Hook and daemon timings are single-machine, whole-process wall time with 20 to 40 ms of run-to-run noise.
- The counterfactual coverage check is a rule-level proxy.
