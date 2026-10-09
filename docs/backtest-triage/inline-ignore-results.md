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

Test added in `src/inline_ignores.rs` (commit d6f08cd, formatted in 251d427):
`apply_inline_ignores_should_UseNoScanOrHash_When_NoKibitzerSubstring` (5,000-line Go source, 1,000 calls, `scans == 0`,
`hash_calls == 0`). That commit also added a wall-clock ratio test (at most 20x a `contains("kibitzer")` baseline) and a
rerun-latency ratio test; both were removed in the post-verify cleanup as flaky on loaded machines, and the `ScanMemo`
counters they sat beside are now `#[cfg(test)]`. The manual timings below remain the latency evidence.

Hook-path raw rerun on the final code. Method: `kibitzer hook` subprocess, fresh cache per call, no daemon, Edit payload
inserting `// kibitzer:ignore long-function -- ...` (marker) or `// plain comment` (no marker) above a unique line near the top;
median of 5 after one discarded run; added = marker minus no marker (whole-process wall time, so noisier than the spike's
in-process timing). Two batches per file.

| File | No marker | Marker | Added |
|---|---|---|---|
| `src/checkers/rules.rs` (largest by bytes) | 215 / 206 ms | 242 / 240 ms | **27 / 34 ms** |
| `src/check.rs` (largest by lines) | 194 / 178 ms | 217 / 199 ms | **23 / 21 ms** |
| Task 2.2.3d spike (plan.md), rules.rs / check.rs | n/a | n/a | 26.9 / 25.9 ms (rerun alone 26.8 / 29.5 ms) |

Result: at most 34 ms added, well under the 150 ms target; consistent with the spike. The rerun-to-first-pass ratio was not re-measured by this run.

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
- A full-tree `kibitzer run` still did not finish for eight repos (deno, k8s, cassandra, vscode, servo, mdn, gitlab, stapler-squad); see "Follow-up" for what the per-file and per-directory reruns did cover and the 24 directories that still timed out.
- Hook and daemon timings are single-machine, whole-process wall time with 20 to 40 ms of run-to-run noise.
- The counterfactual coverage check is a rule-level proxy.

## Follow-up: the eight timed-out repos

Measured 2026-10-08, release build of this branch (`target/release/kibitzer`), scripts kept outside the repo. Whole-tree
`kibitzer run` is infeasible for these repos, so two narrower runs replaced it.

**Per-file sweep of the inline-ignore checker.** `rg --files` (gitignore respected) over the in-scope extensions
(`go ts tsx js jsx mjs cjs py java kt kts rs md`), then `xargs -n1 -P8 kibitzer check native inline-ignore <file>`. This
exercises exactly the new checker on every in-scope file, with no time cap hit.

| Repo | In-scope files | Output lines | Error lines | Wall time |
|---|---|---|---|---|
| denoland/deno | 6,227 | **0** | 24 (6 non-UTF-8 files, 4 lines each) | 14 s |
| kubernetes/website | 8,316 | **0** | 0 | 60 s |
| apache/cassandra | 6,480 | **0** | 0 | 33 s |
| microsoft/vscode | 14,968 | **0** | 0 | 52 s |
| servo/servo | 68,719 | **0** | 36 (9 non-UTF-8 files, 4 lines each) | 245 s |
| mdn/content | 14,651 | **0** | 0 | 28 s |
| gitlabhq/gitlabhq | 15,366 | **0** | 0 | 44 s |
| tstapler/stapler-squad (`~/code` checkout) | 7,475 | **0** | 0 | 17 s |
| kubernetes/kubernetes (not asked for, run for completeness) | 18,003 | **0** | 0 | 46 s |

The error lines are the known single-file-CLI UTF-8 read failure described above, not inline-ignore output.

**Per-directory `kibitzer run`.** Every top-level directory of the same repos (`.git`, `node_modules`, `vendor`, `target`
skipped; files directly in a repo root were not run), `kibitzer run <dir>` with a 180 s cap, 6 at a time: 135 directories.
**111 finished, 24 still timed out.** Across the 111 finished runs: **0** `: [ignore-syntax]` lines, **0** `: [unused-ignore]`
lines, **0** `suppressed inline` footers.

| Repo | Directories finished | Directories timed out at 180 s |
|---|---|---|
| deno | 3 | `cli`, `libs`, `ext`, `tests` |
| kubernetes/website | 10 | `content` |
| cassandra | 13 | `src`, `test` |
| vscode | 6 | `extensions`, `src` |
| servo | 8 | `components`, `tests` |
| mdn | 2 | `files` |
| gitlabhq | 29 | `app`, `doc`, `doc-locale`, `spec` |
| stapler-squad | 30 | `server`, `session`, `tests`, `web-app` |
| kubernetes/kubernetes | 10 | `cmd`, `pkg`, `staging`, `test` |

What is still not shown: a complete `kibitzer run` (all default checkers) over those 24 directories. For them the only
evidence is the per-file sweep above, which covers the `inline-ignore` checker but not the suppression path inside `run`
(inline filtering of other checkers' findings). The public corpus has no `kibitzer` strings (see Limits), so a suppression
there could only come from a false directive match, and none of the 111 finished directories showed one.

## Deferred after the verify review (not done in this pass)

- Module cycle between `check` and `inline_post_pass` (review item C1).
- Splitting `src/inline_ignores.rs` (C2).
- Options struct for the `check.rs` signatures (C7).
- A single `RuleInfo` table in place of `KNOWN_RULES` plus the ownership table.
- Index-based `shown` tracking in `InlineOutcome`.
- From the plan's Amendment 1: Task 2.2.2b (full `kibitzer run` unused-ignore audit), the run-footer false-positive split,
  the success acknowledgement, and Story 3.2.1.

## Phase 6 round 2

Findings from the idioms, architecture and refactor reviews were addressed in small commits on this branch. Not done as asked:

- `debug_assert` on `Line::new(0)`: skipped. Line 0 ("the checker reports no line") is a documented input, treated as line 1, and `covers_should_TreatLineZeroAsLineOne` exercises it; the assert made that test panic.
- `passed_raw: bool` in `scope_line_verdicts` as an enum: skipped. It has 18 references across production and tests, so it was not trivial.
- Trimming the `inline_ignores/mod.rs` re-export facade: skipped. Every non-test re-export has a caller outside the module; the test-only block is already separated.
- `daemon.rs` double registry-path load: skipped as pre-existing and unrelated to this branch.
- `mcp.rs` `run_checks` 185-line extraction: skipped; out of the budget for this pass.
- Cache old-tuple compatibility test: kept on purpose.


## Phase 6 round 3

Fresh-reviewer findings were fixed test-first, one commit per group. Not done as asked, or done differently:

- Tier 1 sanitizer (`echo`) is an allow-list built from printable ASCII, `char::is_alphanumeric` and explicit symbol ranges, not a Unicode general-category lookup (`std` has none and no new dependency was added). Decomposed combining marks outside U+0300-036F (for example Zalgo stacks on other scripts) are dropped.
- Paths in lenient (Tier 2) output are escaped only where the checker prints the path as `Path::display` of the edited file, its canonical form, or its base name. A checker that prints some other derived form of a hostile name is covered by the control-character strip but not the newline escape.
- `kibitzer check native` and the other direct-print subcommands in `src/main.rs` are not sanitized.
- Planted-directive advisory: a pure deletion reports every blocking drop in the file, which can repeat on unrelated deletions. The alternative was silence.
- Config-read ordering (K5): the stamp is now taken before `find_effective_config`, but no test can inject an edit between those two steps, so only the stamp-before-run ordering is mutation-tested (`run_uncached_should_NotCacheStaleResult_...` kills it).
- Daemon: `retire_stale_daemon` no longer clears the spawn debounce, so after an upgrade the new daemon can take up to 10 seconds to appear (hooks run uncached meanwhile). A daemon that predates the owner lock and ignores `shutdown` is left in place rather than displaced.
- Surviving mutant kept on purpose: the `take(MAX_RULES_PER_DIRECTIVE + 1)` guard in `parse.rs` is equivalent to its absence for correctness (it only bounds work), so no test distinguishes it.
- Footer comma/em-dash wording (O4): left as is.


## Phase 6 round 4

Fresh-reviewer findings F1-F6b, G1-G3 were fixed test-first (a few advisory-logic tests were written alongside the change, then checked to fail against the old rule), one commit per group. Not done as asked, or done differently:

- F1: the fix escapes other files' paths at the source (`duplicate-code-cross-file`, architecture finding locations) plus a backstop that escapes every sibling entry of the edited file's directory. A hostile name in a different directory, printed by an external command check, is still only control-stripped, not newline-escaped.
- F3/F5: the "was this suppression already reported" memory is a per-file fingerprint store in the cache dir (rule, reason, text of the silenced line), not a comparison against the pre-edit file or git HEAD. Without a stored baseline a scoped edit reports only file-scope findings and deletions; a finding that appears under a pre-planted directive elsewhere in a never-before-seen file is not reported until the second edit. Two silenced lines with identical text under the same reason share one fingerprint, so the second is not reported.
- F4: a deletion cannot be placed in the post-edit file, so the hook sends a "removed lines" flag (any edit whose `old_string` has more lines than `new_string`) instead of a row; the post-pass treats it like a pure deletion for rows outside the edit.
- F6: ZWSP/word joiner/BOM survive only alone between two letters or digits; variation selectors survive one per base character with the base-class rules in `docs/suppressing-checks.md`. Smuggling through one selector per base character is bounded (about 4 bits per character) but not closed.
- F6b: `kibitzer check architecture`, `check duplicates` and similar subcommands in `main.rs` still print checker text unfiltered.
- G1: the displacement of a wedged holder shells out to `ps` and uses `libc::kill` (new `libc` dependency, already in the lock file). The identity check (user's `kibitzer ... daemon start`, process older than the lock file's last write) is not proof against a hostile same-user process, which can already signal the daemon anyway. Each hook while a daemon is wedged costs one 750 ms probe, then a 10 second skip window.
- G2: unit tests never contact or spawn a real daemon (`cfg!(test)` guards in `run_checks_smart`, `maybe_spawn_daemon`, `spawn_detached_daemon`), rather than setting `KIBITZER_NO_AUTO_DAEMON`; integration tests still exercise real spawns.
- G3: moved the default socket location on a machine with no `XDG_RUNTIME_DIR` to `<tmp>/kibitzer-<uid>/`; a daemon started by an older version at the old path is not found and is left running until it exits.
