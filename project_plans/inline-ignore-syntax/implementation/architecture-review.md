# Architecture Review: inline-ignore-syntax
**Date**: 2026-10-07
**Verdict**: CONCERNS (re-review 2026-10-07; original verdict BLOCKED, see Re-review below)

Reviewed `plan.md` (master @ 3660e64) against the source. kibitzer is on `PATH` and `.claude/inspect.json` exists, but this review verified claims by reading source directly rather than running `kibitzer run`. No `docs/adr/ADR-000-architecture-constitution.md` exists (`docs/adr` is absent), so there are no constitution violations. The Tech Debt Disposition table covers `check.rs`, `accepted_findings.rs`, `comment_quality.rs`, `mcp.rs`, and `hook.rs`. No Lens 4 blocker is raised on the dispositions themselves.

## Constitution Violations
None (no constitution file).

## Blockers
- [ ] Task 2.1.1c (`native_check("inline-ignore", ..., &["**/*"])` in `default_checks()`) — The plan would add the first default native checker scoped to every file, but the per-file pipeline has no binary or non-UTF-8 guard.
  - `walk_and_collect_files` (`src/check.rs:1491-1499`) returns every file not under `SKIP_DIRS`.
  - `run_checks_for_trigger` (`src/check.rs:1453-1479`) filters only by `matches_scope`.
  - `run_checker_against_file` (`src/check.rs:576-589`) calls `read_to_string`.
  - `run_native_check` turns a read error into a failed `CheckResult` whose output is the error text (`src/check.rs:456-470`).
  - Today every default native check is extension-scoped. `grep '"\*\*/\*"' src/config.rs` shows only test fixtures use that scope.
  - This repo has `uploads/*.png`, so `kibitzer run .` would report a failing check for each binary file, and a Blocking/exit-code consequence is possible via `has_blocking_finding`.
  - Remediation, either of:
    - Add a task before 2.1.1c that makes `run_checker_against_file` return `(String::new(), true)` for non-UTF-8 content, with a regression test using a PNG fixture. Do this for this checker only, so existing failure semantics for other checkers do not change.
    - Scope `inline-ignore` to the extensions that have a scanner (the 8 `Language::ALL` grammars, `.md`, and the known fallback types) instead of `**/*`. This also resolves Unresolved Question 2, which the plan currently leaves open.
- [ ] Story 1.2.2 / Task 1.2.2a (rule resolution via `extract_rule`) and Story 2.1.1 (drift guard) — `markdown-link-integrity`, the one default Blocking check and the motivating case for ADR-002, self-prefixes findings with the reference id, not the rule id.
  - `src/checkers/markdown_link_integrity.rs:264` emits `[{id}] used but never defined`.
  - `:295` emits `[{ref_id}] defined but never used`.
  - `:397` and `:400` emit `[{ref_id}]: ... -> file does not exist` and `[{ref_id}]: ... -> no such heading in ...`.
  - `:311` emits `#{slug} -> no such heading in this doc` (no bracket prefix, so it falls back to the checker name).
  - `extract_rule` (`src/accepted_findings.rs:105-110`) reads the leading `[...]` as the rule. For the reference-style findings the "rule" is therefore the link label, such as `foo`. A directive `kibitzer:ignore markdown-link-integrity -- ...` would not suppress them.
  - Consequences:
    - The directive parses as `Valid` and raises no `[ignore-syntax]`, so the suppression fails silently.
    - `[unused-ignore]` (Task 2.2.2a) would then wrongly report it as suppressing nothing.
    - The agent loops on the Blocking exit-2 path (`src/hook.rs:195-203`), which is the failure mode ADR-002 says it avoids.
    - The Task 2.1.1c drift guard greps `"[<id>]` literals. It would pick up `[{id}]` and `[{ref_id}]` as bogus ids, or require special-casing.
  - Remediation, either of:
    - Add a task (before 1.2.2a) that changes those four `markdown-link-integrity` messages to carry a stable `[markdown-link-integrity]` prefix, e.g. `[markdown-link-integrity] [foo] used but never defined`. This is a small, in-repo fix with its own backtest-able test.
    - Introduce a per-checker rule resolver in `inline_ignores` where `markdown-link-integrity` findings always resolve to the checker name.
  - Whichever is chosen, add an AC and test for suppressing a reference-style `markdown-link-integrity` finding, and exclude dynamic `[{...}]` prefixes from the drift guard explicitly.

## Concerns
- [ ] Task 1.2.2a / 2.2.1b — `SUPPRESSED_TOTAL: AtomicUsize` is process-wide state with three problems:
  - The 2.2.1 AC ("3 covered findings -> footer says 3") cannot be asserted deterministically under `cargo test`'s default parallelism, because other tests call `apply_inline_ignores`.
  - The git-HEAD baseline path (`check_native_against_git_head`, `src/check.rs:594-630`) runs `run_checker_against_source` with `Apply` and would double-count.
  - Daemon (`src/daemon.rs:71` spawns threads) and LSP calls never reset it.
  - Recommendation: have `apply_inline_ignores` return `(kept, suppressed_count)`. Carry the count only to `run.rs`, e.g. via a field on `CheckResult` set at the 23 construction sites (`grep 'CheckResult {'` finds 23) with `..Default`-style helper, or via a `SuppressionSink` passed alongside `accepted`. Alternatively, count in `run.rs` by diffing raw and applied runs when it already reruns with `Disabled` for unused-ignore.
- [ ] Task 1.2.2a — `apply_inline_ignores` runs once per checker per file (about 30 default native checks), and each call with a marker present builds its own `GrammarCache` and parses.
  - The plan's "parsed once per call" cost claim understates this.
  - Recommendation: return early when `findings.is_empty()` (the common case for most checkers), and share the directive scan per `(file, source)` through the existing `GrammarCache` or a small per-file memo. This also helps the 4.1.1c timing test.
- [ ] Task 1.2.2b — `inline_mode` on `AcceptedFindings` (`src/accepted_findings.rs:37`) conflates two independent suppressors.
  - It is a `Deserialize`d struct holding `.kibitzer/accepted/` data, so a CLI-mode flag does not belong on it.
  - `AcceptedFindings { accepted }` is constructed by literal at `accepted_findings.rs:100`, so the new field needs `..Default::default()` there.
  - Task 2.2.2b needs to mutate it, but `run_batch_collect` holds `&AcceptedFindings` and shares it across files; it would need a clone or a second instance.
  - Recommendation: introduce a small `SuppressionContext { accepted: &AcceptedFindings, inline: InlineIgnoreMode }` threaded through the same `run_check`/`run_checks_for_trigger` signatures (they already change in 23 test call sites with `&AcceptedFindings::default()`). Or keep the flag field but document the compromise and add the `Default` construction to Task 1.2.2b.
- [ ] Story 1.2.1 / Unresolved Question 1 — The plan treats `file-complexity` as a file-scope rule anchored away from statements. The source disagrees.
  - `aggregate_findings` (`src/checkers/complexity.rs:78-96`) emits one `Finding` per complex function at that function's own line. The comment explains why: diff-scoping would otherwise swallow it.
  - So an ignore above one function suppresses only that function's finding, while the other findings carrying the same aggregate message stay.
  - A directive in the first 10 lines (`FILE_HEAD_LINES`) would suppress all of them.
  - Recommendation: close Unresolved Question 1 now. Document that `file-complexity` is per-function anchored plus the head-of-file whole-suppress convention, and fix the anchor table in Story 1.2.1 and `docs/suppressing-checks.md`.
- [ ] Story 1.2.1 `covers` — A trailing comment (`x := f() // kibitzer:ignore ...`) is defined to cover its own row and the next row, so it also silently suppresses a same-rule finding on the following line.
  - Task 3.2.1a already distinguishes "code precedes the comment".
  - Recommendation: apply that rule in `covers`. A comment with code before it on its row covers only that row; a whole-line comment covers its rows plus the next row.
- [ ] Domain Glossary / Task 1.1.1c — `start_row`/`end_row` and `Finding.line` mix bases.
  - Tree-sitter's `start_position().row` is 0-based, `Finding.line` is 1-based (`src/checker.rs:12-15`), and the plan's ACs write 1-based rows (e.g. `end_row: 9` covers line 10).
  - Recommendation: a `Line(NonZeroUsize)` newtype (1-based) is the only type `Directive` and `covers` accept, with conversion from the tree-sitter row at the single scan boundary. The "line 0 normalized to 1" rule then lives in one constructor.
- [ ] Task 1.1.1b — The near-miss regex `kibitzer\s*:\s*(ignore|disable|allow|suppress|false[-_ ]positive)` is described without an anchor.
  - Applied to any comment text it would emit `[ignore-syntax]` on prose such as `// see kibitzer: allow list in docs`.
  - Recommendation: anchor near-miss detection to the start of the stripped comment text (same position as the exact form), and add negative tests for mid-sentence mentions. The corpus run in 4.1.1b would otherwise be the first to find this.
- [ ] Task 1.1.1a / 1.3.1a — This creates a mutual dependency, `inline_ignores` -> `checkers::comment_quality::comment_kinds` and `comment_quality` -> `inline_ignores::is_directive_comment`. Core filter logic then depends on one checker's private helper.
  - Recommendation: move `comment_kinds` next to `Language` in `src/checker.rs` or `src/tree_walk.rs` instead of widening `comment_quality.rs` (1456 lines) visibility. `inline_ignores` and `comment_quality` then both depend downward.
- [ ] Task 2.1.1a / 2.1.1c — `KNOWN_RULES` duplicates rule ids owned by checkers.
  - `known_rule` calls `crate::checker::registry()`, which rebuilds every checker on each call (`src/checker.rs:~178`). Do it once per scan.
  - The literal-grep drift guard misses ids built dynamically (e.g. `rules.rs` ids interpolated into `format!`).
  - Recommendation: have each checker expose its rule ids (an optional `Checker::rule_ids()` defaulting to `&[]`) so unknown-rule detection and unused-ignore ownership derive from the registry, and keep the grep test only as a backstop.
- [ ] Task 2.2.2b — `[unused-ignore]` is emitted from `run.rs` while `[ignore-syntax]`/`[ignore-volume]` come from the `inline-ignore` checker, so the meta rules live in two places and `unused-ignore` is judged only for checkers that actually ran.
  - A file over `MAX_NATIVE_CHECK_BYTES` (`src/check.rs:572`), a check excluded by trigger, or a plugin-owned rule would produce false `unused-ignore`.
  - Rerunning every file check with `Disabled` also doubles `kibitzer run` cost for marker files.
  - Recommendation: emit it from one place, and have the unused judgement take the set of checks that actually ran for that file from the first pass. Skip the second pass by having the first run collect raw findings alongside applied ones.
- [ ] Story 2.1.2 — `[ignore-volume]` is anchored at line 1, so under PostToolUse diff-scoping (`src/check.rs:473`) it is dropped unless the edit touches line 1.
  - ADR-002 relies on it as a guardrail. In practice it surfaces only on unscoped `batch`/`Stop` runs.
  - Recommendation: state this in ADR-002 and the docs, or anchor it at the most recently added directive's row so it is in scope when the agent adds the fifth ignore.

## Nitpicks
- `Scanned` (Task 1.1.2b) is used but not in the glossary. Define it next to `Directive`.
- `Directive.rules: Vec<RuleId>` can be empty by construction. A non-empty type (or constructor check) makes that unrepresentable.
- The plan cites `check_native_against_git_head` as `:622`; the `run_checker_against_source` call is at `src/check.rs:623`, and the function itself starts at `:594`.
- Verified references, all matching the source: `run_checker_against_source` at `src/check.rs:549`, `run_checker_against_file` at `:576`, `Finding` at `src/checker.rs:12-15`, `file-size` anchor at `src/checkers/file_size.rs:120-126`, `duplicate-code` anchor at `src/checkers/duplicate_code.rs:115`, cross-file anchor at `src/checkers/duplicate_cross_file_checker.rs:261-269`, hook footer at `src/hook.rs:207-219`, blocking stderr at `src/hook.rs:195-203`, MCP instructions at `src/mcp.rs:1429-1440`, `run_batch_collect` accepted load at `src/run.rs:129`, `backtest.rs` calling `run_checker_with_cache` directly (`src/backtest.rs:344`).

---

## Re-review (updated plan.md)
**Verdict**: CONCERNS (no remaining blockers)

Scope: the two original blockers plus the structures the fix introduced. Checked against source on master @ 3660e64.

### Blocker 1 (binary / non-UTF-8 vs `**/*`): RESOLVED
- Design Decision 11 and Task 1.2.2b make `run_checker_against_file` return `(String::new(), true)` for `inline-ignore` on read or UTF-8 error. That is the right seam: the function already receives `checker_name` and does the `read_to_string` (`src/check.rs:576-589`), and the error-to-failed-`CheckResult` path it bypasses is `src/check.rs:456-470`. Other checkers keep today's semantics.
- Globs are narrowed to `Language::ALL` extensions plus `*.md` (Story 2.1.1 "Scope is narrow", Task 2.1.1c), so most binaries are never read. Regression tests (PNG bytes, invalid-UTF-8 `.go`, unchanged read-error behavior for another checker) are specified.
- Small gap: `Language::extensions` is private (`src/checker.rs:82`, no `pub`), and `config.rs` needs it to build the glob list. Task 2.1.1c should say "make `extensions` `pub(crate)`" (or add a `Language::all_globs()` helper). Not a blocker.

### Blocker 2 (`markdown-link-integrity` rule resolution): RESOLVED
- Plan now adds `DYNAMIC_PREFIX_CHECKERS = ["markdown-link-integrity"]` and `rule_matches` (Design Decision 10, Task 1.2.1a). Cited lines match source: `[{id}]` at `markdown_link_integrity.rs:264`, `[{ref_id}]` at `:295`, `:379`, `:397`, `:400`. `:311` (`#{slug} -> ...`) has no bracket prefix and falls back to the checker name, so it needs no special case.
- `accepted_findings::extract_rule` (`src/accepted_findings.rs:105`) stays untouched, so `accepted/` behavior cannot regress.
- AC, real-checker integration test (Task 1.2.1b), `[unused-ignore]` ownership AC (Story 2.2.2) and the drift-guard exclusion of `[{` prefixes are all present. `KNOWN_RULES` is advisory only, so a stale table cannot fail suppression closed.

### New structures
- `comment_kinds` to `src/tree_walk.rs`: OK. `tree_walk.rs` imports only `tree_sitter::Node` today and `checker.rs` does not import `tree_walk`, so adding `use crate::checker::Language` creates no cycle. Source `comment_kinds` is at `src/checkers/comment_quality.rs:173`, with other uses at `:229`; the move is mechanical. `comment_quality` -> `inline_ignores::is_directive_comment` is now one-way. Mild cohesion note: a language-specific table in a generic walker module; acceptable.
- `Line` newtype: OK. Defined in `inline_ignores.rs`, single 0-to-1-based conversion at the scan boundary, line-0 normalization in its constructor. Consumers outside the module (checker output) stay `usize`, so no layering impact.
- `InlineIgnoreContext` on `AcceptedFindings`: acceptable as a documented compromise. `AcceptedFindings` derives `Debug, Clone, Default, Deserialize` (`src/accepted_findings.rs:~35`), so `#[serde(skip)]` plus `Default` works, and `Arc<AtomicUsize>` is `Clone + Debug`. Dependency direction is `accepted_findings -> inline_ignores` (type only); `inline_ignores` takes the context by parameter and does not import `accepted_findings`, so no cycle. `run_checker_against_source` itself takes `&InlineIgnoreContext`, not `AcceptedFindings`, which keeps `check.rs` helpers and `backtest.rs:344` (calls `run_checker_with_cache` directly) unaffected. The per-run `Arc` fixes the earlier global-state concern; `Clone` shares the counter, which is why the baseline path must call `without_counter()` as planned.

### Remaining concerns (non-blocking, fix in the plan before or during implementation)
- [ ] Memo scope is not implementable as written. Pattern Decisions and Task 1.1.2b say the scan is "memoized ... for one `run_native_check` file pass" so ~30 checks share one parse. But each check is a separate `run_native_check` call (`src/check.rs:422`), driven from the loop in `run_checks_for_trigger` (`src/check.rs:1453-1479`); no object spans one file's checks. State where the memo lives (e.g. a `RefCell<Option<(PathBuf, u64, Vec<Scanned>)>>` single-entry cache in `InlineIgnoreContext`, or a thread-local) or drop the claim and rely on the empty-findings early return plus the substring fast path. Story 4.1.1's "parsed once per file pass" AC depends on this.
- [ ] Unused-ignore rerun (Task 2.2.2b) must use an empty `AcceptedFindings` with mode `Disabled`, not the run's `accepted`, because `run_native_check` applies `drop_accepted_findings` (`src/check.rs:482-489`) and would otherwise hide findings an `accepted/` entry shadows, breaking the "shadowed by `accepted/` is not unused" AC. Say so in the task.
- [ ] `ran_checkers` for unused-ignore "comes from the first pass" but `run_checks_for_trigger` returns `Vec<CheckResult>` keyed by `check_name`, so Task 2.2.2b should state that it derives the set from those results (that works, just unstated).
- [ ] Carry-over, unchanged and accepted by the plan: `KNOWN_RULES` duplicates checker-owned ids (advisory only now, so low risk).
