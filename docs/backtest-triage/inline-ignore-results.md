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
above line 4`. At the time of this backtest `kibitzer run` did not emit `[unused-ignore]` (Task 2.2.2b, since shipped in a follow-up PR):
the same misplaced directive there reported the finding only.

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
- From the plan's Amendment 1: Task 2.2.2b (full `kibitzer run` unused-ignore audit; later shipped), the run-footer false-positive split,
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

## Phase 6 round 5

Round-4 reviewer findings were fixed test-first where the old code could still be run: the 7 new `hook_contract` repros failed against the round-4 binary, the 8 new sanitizer tests failed against the round-4 `sanitize.rs`, and the 2 new CLI path tests failed before their fix. The daemon tests (untrusted dir, re-probe, backlog) were written together with the fix and not replayed against the old code.

- **A (done)**: the per-file fingerprint store and the "removed lines" daemon flag are gone (the request field is dropped; an older client that still sends it is ignored by serde). `[blocking-suppressed]` now compares the blocking findings directives drop now with the ones they dropped at git HEAD, as a multiset keyed by rule and the trimmed text of the silenced line (file-scope rules: the rule alone, so 403 -> 603 lines is quiet when HEAD was already over). A file git does not know has an empty baseline; with no git at all a `Write` reports everything and an `Edit` only the rows it touched. The round-4 `MultiEdit` test assumed comment lines between a directive and its call break coverage; they do not (verified with `kibitzer run` on the HEAD content), so the test now deletes two statements instead.
  - Hook latency, debug build, 30 hooks per case on a ~400-line markdown file with three suppressed links: `Edit` 154 ms median without the marker, 165 ms with it (one `git show`, one extra checker run over the HEAD content); `Write` 210 ms vs 153 ms with the marker (the no-marker file also carries live blocking findings, so the pair is not a clean A/B). The extra work runs only when the file carries the marker and a blocking drop exists, and the HEAD read is skipped when every drop was touched by the edit. Treat the numbers as noise-bounded, not a benchmark.
  - Leftovers: the baseline is HEAD, not the previous edit, so until the agent commits, later edits to the same file report the earlier new suppressions again. Identity is the silenced line's text, so a finding that moves between two identical lines is invisible. `markdown-link-integrity` emits two findings per dangling link (6 drops for 3 pairs), noticed and not investigated.
- **B (done)**: `kibitzer check duplicates` and `kibitzer status` (repo paths, check names, log path) escape paths with `display_path`/`escape_path`. Other direct-print subcommands were not audited again.
- **C (done)**: an untrusted runtime directory makes `daemon start` exit 1 naming the directory and why; `daemon status` and `daemon stop` print `runtime dir untrusted: <dir>` and exit 1; hooks append one `daemon_degraded` note to the hook log (marker `daemon-degraded` beside it dedupes by reason) and `kibitzer status` shows it. The per-pid `kibitzer-untrusted-*` socket path no longer exists; tests assert none appears in the scratch temp dir, runtime dir or `/tmp`.
- **D (done)**: (1) a refused connect while the owner lock is held classifies as wedged (verified on this macOS build: a saturated backlog refuses, so the same probe reads dead without the lock and wedged with it). Side effect: during the instant a daemon holds the lock but has not bound its socket, a probe reads wedged and a hook may skip the daemon for the 10 s window. (2) `is_our_daemon` reads `ps -o comm=` and `-o args=` separately and requires the command line to end in exactly ` daemon start` with a kibitzer-named executable that agrees with `comm`, so a path with spaces verifies; a daemon started with extra arguments is no longer displaceable. (3) An `XDG_RUNTIME_DIR` that is not already 0700 is refused and never chmodded; only the `kibitzer-<uid>` directory kibitzer makes is tightened. Test scratch runtime dirs now need mode 0700. (4) The holder is pinged again and its recorded pid re-read right before SIGTERM, and the process is re-verified before SIGKILL; `pid_t::try_from` guards the cast. A window of milliseconds between the re-probe and `kill(2)` remains.
- **E (done, with choices)**: Tier 2 strips U+00AD, U+034F, the Hangul and Khmer fillers and U+2000-U+200A except U+2009; LRM, RLM and ALM survive alone beside an RTL letter; ZWSP, word joiner and BOM survive between letters or digits unless both are ASCII, and after a Thai, Lao, Khmer or Myanmar mark that follows a non-ASCII letter; variation selectors need a base from the emoji, symbol (arrows, math, shapes, dingbats) or CJK blocks. Tag runs survive only as the three England, Scotland and Wales flags (`gbeng`, `gbsct`, `gbwls` after U+1F3F4, ended by U+E007F), narrower than "any bounded run" so a flag cannot carry other text. `echo_path` keeps ZWJ/ZWNJ between two non-ASCII graphic characters and VS16 after an emoji base, and `escape_path` keeps that VS16. Limits: the symbol-block set approximates `StandardizedVariants.txt` rather than reproducing it; Mongolian free variation selectors are not handled; `escape_path` now also escapes LRM/RLM/ALM, so an RTL file name containing one shows `\u{200f}`.

## Phase 6 round 6

Round-5 reviewer findings were fixed test-first, one commit per group. New regression tests were replayed against the pre-fix code: the 8 new `hook_contract` repros fail on the round-5 binary, 6 of the 7 new `daemon_spawn` tests fail on the pre-fix daemon, 7 of the 8 new sanitizer tests fail against the round-5 `sanitize.rs` (the eighth, an exhaustive per-code-point test, uses the new internals), and the `check architecture` CLI test failed before its fix. Not done as asked, or done differently:

- **S (done)**: Tier 2, `escape_path` and `echo_path` share one allow-list (`is_graphic_base`) plus context rules judged from raw neighbors (`invisible_fits`, `mark_fits`); the new `unicode-ident` dependency (already in the lock file via `proc-macro2`) supplies "is a combining mark" (`XID_Continue` minus `XID_Start`, digits, connector punctuation and format characters). An exhaustive test runs every code point `a<c><c>b` and asserts nothing invisible survives; every reviewer-listed code point is stripped in runs of 1, 2 and 200. Residual covert bandwidth is quantified in `docs/suppressing-checks.md`: about 8 bits per CJK ideograph through variation selectors (257 states), 1.6-2.3 bits per emoji, joining-script letter pair, right-to-left letter or space, and visible accent stacks of up to three generic marks per base. Limits: unassigned code points inside the symbol and emoji ranges inherited from Tier 1 (U+1F000-1FAFF and similar) still pass, since `std` has no category query and they render as visible tofu; Mongolian free variation selectors and Arabic Cf signs are dropped; a generic diacritic is accepted after any visible base, so accent stacks over ASCII remain (capped at three).
- **A (done)**: `kibitzer check architecture` filters messages through the lenient tier with sibling-name escaping for the directory scanned (test: Go packages named `<ESC>]0;PWNED<BEL>a` plus a newline, in an import cycle with `b`). `check duplicates`, `check native` and `status` were already covered; other `main.rs` diagnostics were not audited.
- **B1 (done)**: the removed-lines flag is back (hook to daemon request, `serde(default)`, so old clients still work) and is used only when the baseline is unavailable: report when the edit removed lines or left no changed range, or the rule is file-scope. Non-UTF-8 HEAD blobs decode lossily. With no baseline an `Edit` that slides a finding now reports every untouched blocking drop, not just the slid one (2 of 2 in the reproductions), because without HEAD there is nothing to tell them apart.
- **B2 (done)**: baseline and `check_native_against_git_head` git calls remove `GIT_DIR`, `GIT_WORK_TREE`, `GIT_INDEX_FILE`, `GIT_OBJECT_DIRECTORY`, `GIT_ALTERNATE_OBJECT_DIRECTORIES`, `GIT_COMMON_DIR`, `GIT_NAMESPACE`, `GIT_CEILING_DIRECTORIES` and `GIT_PREFIX`, and run with `core.fsmonitor=false` and `--no-ext-diff`. `check_against_git_head`-style shell-command checks were not touched.
- **B3 (done)**: a staged rename compares against the renamed-from HEAD blob (`git diff --cached -M -l200 --name-status`, only after `git show HEAD:path` fails); no commits, untracked, ignored, newly added and submodule paths read as unavailable, not as empty. An unstaged rename (`mv` then edit) has no rename source and falls back to the conservative rule.
- **B4 (declined)**: the hook-dedup directory is a claim-by-filename store for `tool_use_id`s with a one-hour sweep; a per-file multiset count that also tracks HEAD would be a second fingerprint store under another name, with the races and test isolation the round-5 redesign removed, and would hide an advisory from an agent that lost it to context compaction. The noise stays bounded (10 lines plus a count) and is documented.
- **B5 (done)**: baseline, rename, `check_native_against_git_head` and hook-log git calls time out after 2 seconds (`git_cmd::bounded_output`) and read as unavailable; test with a PATH shim `git` that sleeps. `git archive` calls are still unbounded.
- **C (done)**: `acquire_owner_lock` and `shutdown_with_lock` take the displacement as a parameter, which made M04/M05 observable. Mutant replay in a scratch copy: M04 and M05 (`&|| true` re-probe) fail their wiring tests; M07 (second `verify` removed) fails `displace_with_should_NotSigkill_When_PidFailsReverifyAfterTermGrace`; M15 as a regression of `lock_is_held` to a creating open fails two tests, and M15 as `exists()` removed from `lock_is_live` fails one when `lock_is_held` is the old creating version. Because `lock_is_held` now never creates, removing the `exists()` guard alone is an equivalent mutant and survives; the guard stays as defense in depth. `hook_should_FallBackWithinBound_And_ReplaceDaemon_When_HolderIsStopped` was stable alone (6 of 6 runs); the race found by reading was that `start_daemon` waited for the socket file to exist, not for the daemon to answer, and the STOP was not confirmed; it now waits on a ping and on process state `T`, with 20 s per-step and 90 s replacement bounds.
- **D1 (done)**: the degrade marker holds per-note lines (kind, time, text); it is cleared when a hook finds a usable runtime directory, deduped per note per hour, and capped at six notes an hour; `status` prints only notes still in force with their age. A degrade that is never followed by a hook stays listed with a growing age.
- **D2 (done, with a decision)**: 0755-but-own is accepted for a user-provided `XDG_RUNTIME_DIR`: the threat is group or other write (they could replace entries), and the 0600 socket and lock inside stay unreachable. A symlinked `XDG_RUNTIME_DIR` is accepted when its resolved target is a private own directory; the resolved path is used. A refused directory falls back to `<tmp>/kibitzer-<uid>` and is noted once. `daemon start` no longer refuses on a loose `XDG_RUNTIME_DIR` alone.
- **D3 (done)**: `daemon status` on a stopped daemon prints `daemon not responding (pid N)` and exits 1; `daemon stop` names the pid when a holder cannot be verified; the untrusted-directory message says a daemon started earlier may still be running and gives the pid and lock path when the lock is readable.

Pre-existing flake seen once, unrelated: `backtest::tests::backtest_isolates_the_duplicate_cross_file_index_and_still_finds_duplicates` asserts on a process-global environment variable and a scratch directory and failed in one full run of the suite (passes alone, 3 of 3). A scratch `src/daemon.rs.rej` from a patch attempt could not be removed from this sandbox (the tool refused `rm`) and is untracked and unstaged.

## Phase 6 round 7

Round-6 convergence-reviewer findings were fixed test-first, one commit per group. The new tests were not replayed against the round-6 binary; the exhaustive sanitizer scan and the hung-`git archive` hook test were checked another way (the hook test hung 63 s on round-6 `check.rs` and passes in 8 s now).

- **1 (done)**: Tier 1 and Tier 2 no longer allow whole blocks. `is_graphic_base` (Tier 2) and `is_echo_safe` (Tier 1) both consult range tables generated from Unicode 18.0.0 (`scripts/gen-unicode-tables.py` -> `src/inline_ignores/unicode_tables.rs`: assigned L/N/P/S, assigned marks, Default_Ignorable, standardized variants). A code point newer than the tables is dropped until they are regenerated. The tautological scan test is replaced by three exhaustive tests (all of planes 0-3 and 14-16 plus a stride through planes 4-13; 1/2/3-runs over ASCII, Latin, RTL, emoji, Thai, CJK, space and newline contexts, every selector/joiner after every base, and Tier 1 / path output) plus a test that pins the reviewer's 22 unassigned ranges and one that cross-checks the tables against `char::is_alphanumeric`.
- **2 (done)**: `is_mark` is now the category (Mn, Mc, Me), so enclosing marks (U+20DD-20E0 and so on) count against the cap.
- **3 (done)**: U+303F, U+13441, U+13442 (plus U+2800, U+1D159, U+FFFC) are excluded in the generator as assigned-but-blank.
- **4 (done)**: a bidi mark is kept only when the raw previous character is not default-ignorable, so a dropped character between two marks no longer resets the allowance.
- **5 (done)**: emoji and symbol bases take only VS15/VS16 (3 states, about 1.6 bits); other FE00-FE0D pairs must be in `StandardizedVariants.txt` (generated); CJK keeps the full range. A selector also requires its base to survive (an unassigned base plus VS16 used to leave the VS16).
- **6 (done)**: all assigned L/N/P/S characters survive (katakana middle dot, brackets, musical symbols, math alphanumerics, currency signs, ...); arrows U+2190-21FF join the emoji ZWJ members; marks cap at 5 on Tibetan, Myanmar, Khmer and Indic bases (generic accents stay at 3 within them, about 37 bits per base worst case); a joiner after a script mark ending a word (Malayalam chillu) and ZWNJ between Arabic-Indic digits and letters survive. Trailing RLM before Latin after Hebrew still drops (nit).
- **7 (done)**: both `git archive HEAD` calls use `bounded_output` with a 10 s bound. Remaining unbounded git calls: `affected.rs`, `hotspots.rs`, `change_coupling.rs` and `root_cause_clusters.rs`, reached only from the `kibitzer affected|hotspots|coupling|root-causes` CLI subcommands (`src/main.rs`), never from a hook, daemon, MCP or LSP request; a source-scan test (`git_should_NeverRunUnbounded_When_OutsideCliOnlyModules`) fails the build on a new unbounded git call anywhere else. The scan is textual (statement containing `Command::new("git")` or `git_command(` and `.output()`/`.status()`/`.spawn()`), so a call built through a helper would slip past it.
- **8 (done)**: bounded children lead their own process group and the group is killed on timeout and after a normal exit (a background grandchild holding the pipes no longer hangs the reader joins). Left: `tar -x` for the snapshot and the repo-wide check's `sh -c` are not bounded by this runner, and the post-exit group kill has a theoretical pid-reuse window.
- **9 (done)**: `with_git_budget` (5 s per hook, daemon, MCP, LSP or `run` request, entered in `run_check`, `run_checks_for_trigger`, `check_predates_git_head` and the hook log) shares one clock across calls; once spent, calls return unavailable without spawning.
- **10 (done)**: a nested "refused too" XDG reason prints the directory and reason instead of the whole nested message, so "untrusted" appears once.
