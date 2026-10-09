# Adversarial Review: inline-ignore-syntax

**Date**: 2026-10-07
**Verdict**: BLOCKED (initial review; see Re-review below, verdict CLEAN)

Plan claims were checked against source at master @ 3660e64. Verified claims are listed at the end.

## Blockers

- [ ] **B1. Rule matching via `extract_rule` cannot work for `markdown-link-integrity`, the only default-blocking check.** The plan keys on `extract_rule` (`src/accepted_findings.rs:105`), which returns whatever is inside the leading `[...]`. `markdown-link-integrity` messages start with the link ref id: `format!("[{id}] used but never defined")` (`src/checkers/markdown_link_integrity.rs:264`), `"[{ref_id}] defined but never used"` (:295), `"[{ref_id}]: {file_part} -> file does not exist"` (:397), and the two heading variants (:379, :400). A directive `kibitzer:ignore markdown-link-integrity -- ...` therefore never covers these findings, because the extracted rule is the ref id. A directive naming the ref id is flagged as an unknown rule (Story 2.1.1), and the unused-ignore pass (Task 2.2.2a) would misjudge it. ADR-002's decision to keep Blocking checks suppressible, and the success metric "one comment dismisses the finding", both fail on this check. Neither the plan nor the research mentions it, and no AC covers a markdown-link-integrity finding. The same hazard applies to any checker whose message begins with `[` followed by dynamic text. Recommendation: before implementation, define rule resolution as "message `[x]` prefix only if `x` is in the known-rule set, else the checker name". Then add an AC and a real-checker anchor test (Task 1.2.1b) for `markdown-link-integrity`. Decide whether `accepted_findings::extract_rule` is shared as-is or whether the change must not alter `accepted/` behavior (Story 1.2.2 "zero regression").

- [ ] **B2. Default-on `inline-ignore` with globs `**/*` fails on every non-UTF-8 file.** Task 2.1.1a registers the checker with `file_globs` `**/*` and `native_check("inline-ignore", Advisory, &["**/*"])`. `run_native_check` scope-matches, then `run_checker_against_file` calls `std::fs::read_to_string` (`src/check.rs:585-587`). Any error becomes a failed `CheckResult` with the error text as output (`src/check.rs:458-470`). `kibitzer run` collects every file via `walk_and_collect_files` (`src/check.rs:1491-1499`), which has no binary or extension filter, only directory `SKIP_DIRS`. Every PNG, jar, or font in a repo would therefore print `reading <file>: stream did not contain valid UTF-8` as an `inline-ignore` failure. Every existing native checker is protected by narrow language globs, so this is a new failure class introduced by the plan. It also contradicts Task 4.1.1b ("corpus run is clean") and the NFR (no slowdown), because every file is read once per check. Recommendation: restrict the checker's globs to the extensions that have a scanner (the 7 grammars, `*.md`, and the chosen fallback set from Unresolved Question 2). Alternatively, make a non-UTF-8 read return empty findings for this checker. Add an AC that a binary file in the walk yields no `inline-ignore` output.

## Concerns

- [ ] **C1. A trailing comment on a code line also suppresses the next line.** `covers` is defined as "comment rows or exactly one past its last row" (Glossary). So `x := f() // kibitzer:ignore flag-argument -- ...` at row 9 silently suppresses a different `[flag-argument]` finding at line 10. Task 3.2.1a already distinguishes "code precedes the comment", but `covers` does not. Recommendation: the +1 rule applies only to whole-line comments (no code before the comment on its start row), and add a unit test.

- [ ] **C2. `UnknownRule` is a parse-time Malformed, so a stale `KNOWN_RULES` table makes real suppression fail closed.** The drift guard (Story 2.1.1 last AC) greps `"[<id>]` string literals in `src/checkers/*.rs`. It misses three cases. (1) Dynamic prefixes, as in B1. (2) Checkers outside that directory: `src/single_call_site_delegation.rs:69` emits `[single-call-site-delegation]` and is a top-level module. (3) Any rule table in `rules.rs`, if ids are built rather than literal. An agent that writes a legitimate ignore for such a rule gets `[ignore-syntax] unknown rule` and an unsuppressed finding, with no recourse but `accepted/`. Recommendation: make the unknown-rule check advisory only. Apply a directive when the rule matches a finding's resolved rule, regardless of the table. Use the table only to produce the "did you mean" finding when a directive covers nothing. This also removes the need for the table to be complete.

- [ ] **C3. The process-wide `AtomicUsize` suppressed counter is racy and over-counts.** It is incremented in `apply_inline_ignores`, which Task 1.2.2b also reaches from `check_native_against_git_head` (HEAD baseline, `src/check.rs:594-623`, runs with `Apply`). So the footer "N findings suppressed" counts baseline replays as well as real ones. Parallel `cargo test` threads and the long-lived daemon share the counter, so Story 2.2.1's "last line is exactly 3 findings suppressed" test is flaky. Recommendation: return the suppressed count from `apply_inline_ignores` and carry it on the result path. Alternatively, make the HEAD-baseline call non-counting and use a thread-local reset in the test.

- [ ] **C4. `--no-inline-ignores` makes the baseline inconsistent.** The current run uses raw findings, but `check_native_against_git_head` is hard-wired to `Apply` (Decision 9). A raw blocking finding whose HEAD copy had an ignore is then reported as "new" when the file did not change that line. Recommendation: pass `accepted.inline_mode` into the baseline call too. Decision 9's rationale (honor ignores at HEAD) still holds in `Apply` mode.

- [ ] **C5. Near-miss regex is unanchored in the plan text and its tightening is deferred to the backtest.** `kibitzer\s*:\s*(ignore|disable|allow|suppress|...)` (Task 1.1.1b) would fire `[ignore-syntax]` on ordinary prose comments mentioning kibitzer's own syntax (this repo's docs and tests do). That is a default-on advisory noisy by design. Recommendation: anchor to the start of the stripped comment text now, and add negative tests for prose mentions. Do not wait for Task 4.1.1b.

- [ ] **C6. Both Unresolved Questions are answerable now and gate fixtures.** (1) `file-complexity` emits one finding per complex function at that function's line, all with the same aggregate message, and only when at least `MIN_COMPLEX_FUNCTIONS` functions exceed the threshold (`src/checkers/complexity.rs:55-96`). It is not a single file-level anchor. Ignoring one function leaves the others, so a head-of-file directive covering all of them (the plan's `FILE_HEAD_LINES` rule) is the only way to silence the whole finding. Document that in the anchor table and test it. (2) Fallback-scanner scope: B2's glob restriction needs this answered before Task 2.1.1a, not at 1.1.2b. Recommendation: resolve both in the plan.

- [ ] **C7. The scope exceeds the requirements document in several places.** The requirements ask for syntax, shared-path filtering, malformed reporting, a false-positive listing, and docs. `[ignore-volume]`, `[unused-ignore]` (with a second full rerun of file checks per marker file in `run.rs`), the suppressed-count footer, `--no-inline-ignores`, hook footer syntax hints, and MCP output hints come from ADR-002 and the research, not the requirements. They are defensible, but the appetite is "medium, 1-2 weeks" with 4 phases and about 40 tasks. Recommendation: mark Phase 2.2.2 (`unused-ignore`) and Story 2.1.2 (volume) as cuttable, so a shortfall drops them rather than the core seam.

- [ ] **C8. Directive comments can perturb other checkers that read comments or lines.** Only `comment-quality` and the markdown prose checks are handled (Story 1.3.1). `file-size` counts raw lines, `duplicate-code` and `duplicate-code-cross-file` compare line windows, and `syntax-rules` and `em-dash-overuse` read comment text. An added ignore line can shift or create duplicate-block matches, and a reason with `--` is fine but one with an em dash is not (handled by the parser as malformed, good). Recommendation: add one regression test that an ignore comment does not change `duplicate-code` and `syntax-rules` results for an otherwise unchanged file.

## Minors

- Several line references in the plan are approximate or off by one: `Language::for_path` is at `src/checker.rs:104` (plan `~62`) and `Language::ALL` at :42 (plan `~48`). The `run_checker_against_file` call inside `run_native_check` is at `src/check.rs:457` (plan :456), and the HEAD baseline call at :623 (plan :622). Anchor on function names in the tasks, since the tasks themselves are correct.
- `Ok(AcceptedFindings { accepted })` at `src/accepted_findings.rs:100` is a struct literal. Adding the `inline_mode` field requires touching it (use `..Default::default()`). Plan names only the struct at :37.
- Putting `InlineIgnoreMode` on `AcceptedFindings` mixes two unrelated concerns. The plan acknowledges it as pragmatic. A `#[serde(skip)]` field on a `Deserialize` struct is fine.
- `kibitzer check native <name> <file>` (`src/main.rs:384`) runs the checker directly and therefore bypasses inline ignores. This is the right behavior for the corpus workflow, but the docs should say so.
- `[ignore-syntax]` findings for a malformed directive in a doc comment that merely illustrates the syntax (for example `/// kibitzer:ignore <rule> -- <why>` in this repo's own source) would be parsed as a directive with rule `<rule>`. Add a rustdoc-style negative test, or require the rule to match `[a-z0-9-]+`.
- Story 3.3.1's last AC says doc examples are "in code fences", but Task 3.3.1a adds the anchor table; make sure the examples there are fenced as well.

## Verified claims (against source)

- `run_checker_against_source` (`src/check.rs:549`), `run_checker_against_file` (:576), `check_native_against_git_head` (:594), `run_native_check` (:422), `drop_accepted_findings` (:528) exist as described. The accepted filter runs after scope filtering (:473-489).
- All entry points funnel through `run_check`/`run_checks_for_trigger` (`src/lsp.rs:86`, `src/mcp.rs:886,971`, `src/daemon.rs:171,351`, `src/run.rs:138,147`), so the single-seam claim holds. `src/backtest.rs:344,351` call `run_checker_with_cache` directly, so backtests stay raw.
- `extract_rule` is private at `src/accepted_findings.rs:105`; `comment_kinds` is private at `src/checkers/comment_quality.rs:173`.
- `file-size` anchors at the last line (`src/checkers/file_size.rs:120-126`); `duplicate-code` at the last occurrence (`src/checkers/duplicate_code.rs:115`); `duplicate-code-cross-file` at `start + 1` (`src/checkers/duplicate_cross_file_checker.rs:261-269`).
- `Finding` has only `line` and `message` (`src/checker.rs:12-15`). `CheckResult {` appears 23 times in `src/` (cache 6, lsp 2, check 13, hook 2). `check.rs` is 3043 lines and `mcp.rs` is 3350.
- The Research correction on `god_class.rs` was not re-verified and is not load-bearing.

## Re-review (blockers B1 and B2 only)

**Verdict: CLEAN**

Checked plan.md (Design Decisions 10 and 11, Stories 1.2.1, 1.2.2, 2.1.1, 2.2.2, Tasks 1.2.1a, 1.2.1b, 1.2.2b, 2.1.1a, 2.1.1c) against source at master @ 3660e64.

### B1 (markdown-link-integrity rule resolution): resolved

- Rule: a directive matches when it equals the checker name, or equals the leading `[x]` prefix and the checker is not in `DYNAMIC_PREFIX_CHECKERS = ["markdown-link-integrity"]`. `extract_rule` (`src/accepted_findings.rs:105`) stays private and untouched, so `accepted/` behavior is unchanged.
- VERIFIED the five dynamic-prefix sites: `format!("[{id}] used but never defined")` (`src/checkers/markdown_link_integrity.rs:264`), `[{ref_id}] defined but never used` (:295), and the heading/file variants (:379, :397, :400). The checker's `name()` is `"markdown-link-integrity"` (:53-55), which equals the string the plan puts in `DYNAMIC_PREFIX_CHECKERS`.
- VERIFIED by grep over `src/**/*.rs` for `"[{...}]` message prefixes that `markdown_link_integrity.rs` is the only checker with a data-valued prefix (other hits are `mcp.rs`/`run.rs`/`check.rs` output formatting, not finding messages). The one-entry list is complete today. The Story 2.1.1 drift guard also ignores `[{` prefixes, so a future dynamic checker is not caught by it. That is a minor gap, not a blocker (a new dynamic checker still matches by checker name; only `[x]` matching would be wrong).
- The matcher takes `checker_name`, and the seam has it: `run_checker_against_source(checker_name, ...)` (`src/check.rs:549-563`) calls `run_checker_configured`, which resolves the name through `lookup`, so the registry name is the one the plan compares against.
- Test coverage now exists: AC at plan.md:211-212 and a real-checker (not hand-built `Finding`) anchor test in Task 1.2.1b, plus a `markdown-link-integrity` AC in Story 2.2.2.
- Decision 10 also removes C2's fail-closed hazard: matching never consults `KNOWN_RULES` (Task 2.1.1a: "never to decide suppression"), and Story 1.2.2 has an AC for a rule absent from the table.

### B2 (non-UTF-8 files vs default `inline-ignore`): resolved

- Root cause confirmed in source: `run_checker_against_file` (`src/check.rs:576-589`) does `read_to_string(...).with_context(...)?`, and `run_native_check` turns the `Err` into a failed `CheckResult` (:457-470). `walk_and_collect_files` (:1491-1499) has no binary filter.
- Fix works as designed: `run_checker_against_file` already receives `checker_name` (:577), so a branch `checker_name == "inline-ignore"` that returns `Ok((String::new(), true))` on a read error gives a passing empty result and leaves every other checker's behavior as is. The existing size guard (:581-585, `MAX_NATIVE_CHECK_BYTES`) already uses the same `Ok((String::new(), true))` shape, so the pattern is consistent.
- Second layer: globs narrowed from `**/*` to `Language::ALL` extensions plus `*.md` (Task 2.1.1a/2.1.1c, with an AC asserting it), so PNG/jar/font files are never read by this checker. The guard covers invalid-UTF-8 files that do match a glob (for example a `.go` file); Story 2.1.1 has an AC for both cases and Task 1.2.2b has the regression test.
- Other paths: the HEAD baseline already degrades safely (`String::from_utf8(...).ok()?`, `src/check.rs:~618`), and hook/MCP/LSP paths hand the checker an in-memory `&str`.

### New problems introduced by the fixes

None blocking. Minor notes:

- The guard keys on the string `"inline-ignore"` inside `check.rs`, a small coupling. Acceptable; a `Checker` trait flag (for example `tolerates_non_utf8`) would be cleaner if a second checker needs it.
- The drift guard ignores dynamic prefixes by design, so a future dynamic-prefix checker must be added to `DYNAMIC_PREFIX_CHECKERS` by hand. Worth one line in the doc comment of that const.
- Concerns C1 (trailing-comment +1), C2 (advisory unknown rule), C3 (per-run counter, baseline excluded), C4 (baseline inherits mode), C5 (anchor near-miss), C6, C7 (cut order) and the rustdoc `<rule>` placeholder minor are all visibly addressed in the updated plan text; they were not re-audited in depth, as scoped.

Remaining blockers: none.
