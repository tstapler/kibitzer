# Implementation Plan: inline-ignore-syntax

**Feature**: Inline `kibitzer:ignore` comments that suppress a native per-file finding on the same line or the line below, with mandatory rule and reason.
**Date**: 2026-10-07
**Status**: Phase 0 gate decided 2026-10-07: PROCEED with the full plan (59.5h committed after the single-marker collapse; `docs/backtest-triage/inline-ignore-gate.md`). Phases 1-4 in implementation.

## Amendment 1 (2026-10-07): single marker, supersedes any text below that says otherwise

The Task 0.1.2 marker test failed (class (c) 0 of 4) and the requester accepted the collapse. Wherever this plan or `validation.md` mentions a second marker, read it as follows:
- Only `kibitzer:ignore` exists. `kibitzer:false-positive` is not a directive; it parses as `Malformed(NearMissMarker)` (add `false-positive`/`false_positive` to the near-miss set; the message echoes the text found: `'kibitzer:false-positive' not recognized; use 'kibitzer:ignore'`). `DirectiveKind` and `Directive.kind`/`DroppedFinding.kind` do not exist.
- Story 3.2.1, Epic 3.2 and Tasks 3.2.1a/b are dropped (there is no `list --inline`; `kibitzer check false-positives list` is unchanged). Remove `list --inline` wording from docs, `CLAUDE.md` and Task 4.1.1f's checklist; `[ignore-volume]` message drops the `(N false-positive)` part.
- The hook footer carries no marker clause; the syntax line is `Dismiss a judged finding: // kibitzer:ignore <rule> -- <why>, on its own line directly above the flagged line (above line 20 for the first). Rules: ...`. The 640/215 character caps stay as ceilings.
- Hours (recounted with the command in "Effort and appetite": 61.5 sum of per-task figures): Story 3.2.1 (2.5h) leaves the committed slice, so Phases 1-4 are 55.5h and the committed total is 59.5h (the table below shows the pre-collapse 58.0/62.0/64.0/49.0 figures; subtract 2.5 from the first three, and the SHRINK slice no longer applies).
- Validation rows for `list_false_positive_markers_*`, `false_positives_list_*inline*`, `*FalsePositiveKind*`, `*AcceptBothMarkers*`, `*SuppressIdentically*` are dropped; the near-miss row also covers `kibitzer:false-positive`.
**ADRs**: [ADR-001](../decisions/ADR-001-adopt-inline-ignore-comments.md), [ADR-002](../decisions/ADR-002-blocking-checks-suppressible-with-guardrails.md)

Line refs are against master @ 3660e64 and were re-checked in this phase.

## Effort and appetite

The earlier 3-8 minute per-task figures (about 2.5 hours total) were wrong: they were agent keystroke times, not engineering effort. Task estimates below are hours (AIC 1-4h unit). **Totals are the sum of the per-task figures, recounted with a command** (INFERRED from task sizes, not measured):

```
perl -ne 'print "$1\n" if /^#{4,5} Task .*?\(~([0-9.]+)h/' plan.md | paste -sd+ - | bc
```

Round-3 re-estimates: 1.2.2b1b 2.5h to 4.5h (thread context through 4 callers, `SourceCheck` refactor with the raw-findings field, `ScanMemo` with test counters, 8 AC tests), 2.2.3a 3.5h to 5h (rerun orchestration plus the raw-findings path), 1.2.2b1c 1h to 1.5h (cache version stamp). Added: Phase 0 (4h, replaces Task 4.1.1e), Task 2.2.0a post-pass module (1h), Task 2.2.1c `run` syntax hint (0.5h), Task 2.2.2e run-path unknown-rule audit (1h); 4.1.1c grows 0.5h (net-token check), 4.1.1f grows 0.5h (pooled review). Cut: the success ack (1h, see Scope disposition) and the `false-positive` split in the `run` footer.

| Slice | Hours |
|---|---|
| Phase 0 go/no-go gate (Tasks 0.1.1-0.1.3) | 4.0 |
| Phases 1-4 committed slice (everything except Task 2.2.2b) | 58.0 |
| **Committed total (Phase 0 + implementation)** | **62.0** |
| Task 2.2.2b (`[unused-ignore]` in `kibitzer run`, deferred to a follow-up PR) | 2.0 |
| Sum of every per-task figure | 64.0 |
| **SHRINK slice** (Phase 0 result 2% <= G < 5%, see Phase 0): drops Tasks 2.2.2a, 2.2.2e, 2.2.3a-d, and Story 3.2.1 (single marker, no listing), 13.0h together; keeps the post-pass module (3.1.2a needs it) | 49.0 |

Appetite is Medium (1-2 weeks, roughly 40-80 focused hours for one person). 62h is upper-middle of that range with no slack, up from the earlier claim of about 52.5h because round 3 found three light estimates and four missing pieces; the earlier figure was an under-count, not a saving that was lost. One person on a part-time week does not absorb an overrun, which is why Phase 0 comes first: a STOP costs 4h, not 62h.

### Scope disposition (beyond-ask features)

The requirements ask for: syntax, shared-path filtering, malformed error, false-positive listing, docs. Each addition beyond that, with its decision and hours (the hours are the per-task figures; guardrails marked KEEP are the P1 mitigations from `pre-mortem.md`):

| Feature | Decision | One-line justification | Hours |
|---|---|---|---|
| `[ignore-volume]` (Story 2.1.2) | KEEP | P1-2: only repo-visible signal against blanket silencing; 0.5h | 0.5 |
| `[blocking-suppressed]` hook advisory (Story 3.1.2) | KEEP | ADR-002 makes the one default blocking check suppressible; this is the price | 1.5 |
| Weak-reason rule (Task 1.1.1d) | KEEP | P1-2: stops `-- needed`; 0.5h; two-word floor is deliberately minimal (see Accepted risks) | 0.5 |
| `run` footer total + blocking count (Story 2.2.1) | KEEP | P1-2: makes 1-4 ignores per file across many files visible | 0.5 |
| `run` footer `false-positive` split | DEFER (cut) | the `list --inline` count at the 30-day review gives the same number; removes a counter field and a test | 0 |
| `--no-inline-ignores` (Task 2.2.1a) | KEEP | `Disabled` mode must exist anyway for the raw rerun; the flag is the only way a developer sees what is hidden (reviewability constraint); 0.5h | 0.5 |
| `[unused-ignore]` hook advisory (Story 2.2.3 family) | KEEP (dropped only in the SHRINK slice) | P1-1 wrong-row loop; the correction fires where the mistake happens | 10.5 (2.2.0a 1, 2.2.2a 1.5, 2.2.3a 5, 2.2.3d 1, 2.2.3c 1, 2.2.3b 1) |
| `[unused-ignore]` in `kibitzer run` (Task 2.2.2b) | DEFER to follow-up PR | nothing depends on it; the unknown-rule typo case is covered without it by Task 2.2.2e | 2.0 (not in committed total) |
| Unknown-rule audit in `kibitzer run` (Task 2.2.2e) | KEEP | UX: a typo'd rule must not be silent in the CLI even with 2.2.2b deferred; reuses the Task 2.2.3c first-pass logic, no rerun | 1.0 |
| Marker split (`false-positive` vs `ignore`, Story 3.2.1) | DROPPED 2026-10-07 (Amendment 1) | Task 0.1.2 collapsed to one marker (class (c) 0 of 4) | 0 (was 2.5) |
| Success ack line (old Task 3.1.2 ack) | DEFER (cut) | a working ignore makes the finding vanish; `kibitzer run` confirms; ~40 tokens per edit for an ambiguity that costs one command; revisit if the 30-day sample shows agents re-adding directives that already work | 0 (was folded in, unbudgeted) |
| `run` one-line syntax hint (Task 2.2.1c) | KEEP | UX gap: developers otherwise find the syntax only in docs; 0.5h | 0.5 |

**If it overruns, cut in this order** (never the P1 guardrails: `[ignore-volume]`, blocking count, `[blocking-suppressed]`, `Reason` rule): (1) the long tail of Task 1.2.1b fixtures (about 3h): ship fixtures for the checkers with the most findings in the Phase 0 replay and list the rest in `ANCHOR_PENDING`, each with a tracking note; (2) Task 2.2.2e (1h); (3) Task 2.2.1c (0.5h); (4) stop and ask the requester before cutting anything else. Cuts (1)-(3) free 4.5h, enough to absorb the top of the honest range (about 62-68h); beyond that, move to the SHRINK slice rather than cutting guardrails.

---

## Domain Glossary

| Term | Definition | Notes |
|------|-----------|-------|
| `Line` | 1-based line number newtype (`NonZeroUsize`); the only row type `Directive` and `covers` accept | Converted from tree-sitter's 0-based `start_position().row` at the single scan boundary; `Finding.line == 0` is normalized to `Line(1)` in its one constructor |
| `Directive` | One parsed ignore comment: kind, non-empty rules, reason, `start_line`/`end_line` (`Line`), `whole_line: bool` (no code precedes the comment on its start row) | Struct in `src/inline_ignores.rs` |
| `Scanned` | `(Line, DirectiveParse)` pair returned by `scan_directives` | Defined next to `Directive` |
| `DirectiveKind` | REMOVED by Amendment 1 (single marker) | Do not create the type; drop `kind` from `Directive` and `DroppedFinding` |
| `RuleId` | Rule a directive names; must match `[a-z0-9-]+` (so a doc-comment placeholder like `<rule>` is not a directive). Matches a finding when it equals the finding's checker name, or equals the leading `[x]` prefix and the checker is not in `DYNAMIC_PREFIX_CHECKERS` (see Story 1.2.2) | Newtype over `String`; matching lives in `inline_ignores`, `accepted_findings::extract_rule` (`src/accepted_findings.rs:105`) is not changed |
| `DYNAMIC_PREFIX_CHECKERS` | Checkers whose `[...]` prefix is data, not a rule id: `markdown-link-integrity` (ref id, `src/checkers/markdown_link_integrity.rs:264,295,379,397,400`) | Their findings only match a directive naming the checker |
| `Reason` | Trimmed text after ` -- ` meeting a minimum-quality rule: at least 2 whitespace-separated words (a mechanical floor, not the target the messages state), and not equal (case-insensitive, hyphens/spaces folded) to any rule id in the directive | Newtype; constructor rejects empty, one-word, and rule-id-echo reasons (parse, don't validate). A rejected reason yields `Malformed(WeakReason(kind))` with `kind` = `TooShort` or `RuleEcho` (separate repair messages, Task 1.1.1d) and the directive does not suppress. Limit: this stops lazy boilerplate like `-- needed` or `-- flag-argument`, not a determined two-word lie (`-- legacy code`); review and the footer counts cover that |
| `DirectiveParse` | Result of parsing one comment line: `Valid(Directive)` / `Malformed(MalformedReason)` / `NotADirective` | Sum type; malformed never suppresses |
| `MalformedReason` | `MissingRule` / `MissingReason` / `WeakReason(TooShort \| RuleEcho)` / `NearMissMarker` / `NotAtCommentStart` / `EmDashSeparator` / `BadRuleList` / `UnknownRule{suggestion}` | Sum type; each maps to one exact-fix message |
| `InlineIgnoreContext` | `{ mode: InlineIgnoreMode (Apply default / Disabled raw), counter: Option<Arc<SuppressionCounts>>, scan_memo: Arc<ScanMemo> }`; counter and memo are per-run instances created with the context, never a `static` or `thread_local` | Field on `AcceptedFindings`, `#[serde(skip)]`, `Default` = `Apply`, no counter, fresh empty memo |
| `SuppressionCounts` | `{ total: AtomicUsize, blocking: AtomicUsize }`: findings dropped inline, of which suppressed from a `Severity::Blocking` check | Feeds the `run` footer (Story 2.2.1). The earlier `false_positive` field was cut (scope disposition): `list --inline` gives that count at review |
| `ScanMemo` | Single-entry cache `Mutex<Option<(PathBuf, u64 /*content hash*/, Arc<Vec<Scanned>>)>>` plus a `scans: AtomicUsize` test counter. A lookup with a different path or hash replaces the entry | Owned by `InlineIgnoreContext`, so it lives exactly as long as one run/daemon request context and holds at most one file. Content-hash keying means an edited file can never return stale directives |
| `AppliedIgnores` | `{ kept: Vec<Finding>, dropped: Vec<DroppedFinding> }`, the return type of `apply_inline_ignores` | `DroppedFinding { directive_start: Line, directive_end: Line, rule: RuleId, kind: DirectiveKind, reason: Reason, finding_line: Line, severity: Severity }`; always populated (cheap, only the dropped findings), so the blocking-suppressed advisory (Story 3.1.2) and the `SuppressionCounts` both read one source |
| `InlineOutcome` | `{ shown: Vec<(RuleId, Line)>, kept: Vec<(RuleId, Line)>, dropped: Vec<DroppedFinding> }`, carried on `CheckResult`; `kept` = every finding that survived inline filtering, before scoping and `accepted/` (whole file, capped at 20; the Task 2.2.3c fallback needs rows for findings that diff-scoping hides); methods `first_anchor() -> Option<(&RuleId, Line)>` (= `shown.first()`) and `rule_ids() -> Vec<&RuleId>` (distinct, finding order) | `shown` (finding order, capped at 20 entries) lists the **shown** findings only: they are computed in `run_native_check` **after** diff-scoping (`src/check.rs:473`) and `drop_accepted_findings` (`:482`), from the structured `Vec<Finding>` that `SourceCheck` carries back, by keeping the findings whose rendered `{file}:{line}: {message}` line still appears in the final `combined` (set membership on text the same function just produced; no rule or line is ever parsed out of text) and resolving each rule with `anchor_rule(checker_name, &Finding)`. So the hook hint never names a row for a finding that was scoped out or accepted-dropped. `dropped` is built before scoping (it covers the whole file, which Task 2.2.3c relies on). `CheckResult` is `Serialize`/`Deserialize` and cached in `cache.json` (`src/check.rs:31`; the daemon returns cached results at `src/daemon.rs:155-165`). The field is `#[serde(default)]` and **is serialized** (decision 12): old caches still load (empty outcome) and a cache hit keeps the hint data and dropped list. `RuleId`, `Line`, `Reason`, `DirectiveKind`, `Severity` and `DroppedFinding` must therefore derive `Serialize`/`Deserialize`. `CheckResult` is built with a struct literal at 16 sites in 4 files (`grep -nE 'CheckResult\s*\{' src` minus the struct/impl/fn lines: `src/check.rs` 10 (9 production at :216,:235,:261,:329,:439,:460,:512,:1167,:1237, 1 test helper at :101), `src/cache.rs` 4 (test, :195,:284,:360,:368), `src/hook.rs` 1 (test, :250), `src/lsp.rs` 1 (test, :580)); each gets `inline: InlineOutcome::default()` (Task 1.2.2b1a). Note `CheckResult` does not derive `Default`. Verified against master 3660e64: `CheckResult.findings` is `Vec<ArchFinding>` (architecture checks) and `output` is flattened text, so neither can carry native `Finding`s |
| `SourceCheck` | `{ combined: String, passed: bool, findings: Vec<Finding> /* kept, after inline filtering */, inline: InlineOutcome /* dropped only; shown fields are filled later */ }`, the return of `run_checker_against_source` (replaces the `(String, bool)` tuple, 4 callers) | The one structured data path: `run_native_check` uses `findings` to fill the shown fields of `InlineOutcome`, and `raw_findings_for_check` (Task 2.2.3a) returns `findings` from a `Disabled` call. Not serialized, so `Finding` needs no `Serialize` |
| `RawFinding` | `{ line: Line, checker: String, rule: RuleId, message: String }` | Built from a `Disabled` `SourceCheck.findings` by `raw_findings_for_check`; the input type of `unused_ignores`. The rule is resolved by `anchor_rule`, not parsed from text |
| `inline_post_pass` | New module `src/inline_post_pass.rs`: hook-side logic that runs once after the per-check loop in `run_checks_for_trigger` (wrong-row `[unused-ignore]`, `[blocking-suppressed]`, unowned-rule fallback). `run_checks_for_trigger` gains a single call | Keeps Stories 2.2.3 and 3.1.2 out of `check.rs` / `run_checks_for_trigger` (3043-line file; see Tech Debt Disposition) |
| `anchor line` | The `Finding.line` a checker reports (`src/checker.rs:12-15`) | Ignore matches it by position only |
| `covers` | A directive covers a finding when the rule matches and the finding line is within the comment's rows, or exactly one past its last row **only if the directive is `whole_line`**. A trailing comment (`x := f() // kibitzer:ignore ...`) covers only its own row | "same line or line above" |
| `file-scope rule` | Rule whose anchor is not a statement (`file-size`; `file-complexity` emits one finding per complex function, see Story 1.2.1) | Also covered by a directive in the first `FILE_HEAD_LINES` (10) lines |
| `meta rule` | `ignore-syntax`, `unused-ignore`, `ignore-volume`, `blocking-suppressed` | Never suppressible inline |
| `inline-ignore` | Name of the new native checker emitting `[ignore-syntax]` and `[ignore-volume]` | Registered via `inventory::submit!`, in `default_checks()`; scoped to grammar extensions plus `*.md`; returns no findings on non-UTF-8 or unreadable files |
| `KNOWN_RULES` | Static table of bracket-prefixed rule ids to owning checker-name prefix | **Advisory only**: used for "did you mean" and unknown-rule reporting. It never gates suppression, so a stale table cannot make a legitimate ignore fail closed. Drift-guarded by a test |

---

## Step 0.5: Alternatives considered

| Approach | Strength | Weakness |
|----------|----------|----------|
| A. Extend text-based `filter_accepted` | Reuses existing path; one filter site | Has no source or tree; would re-read the file and regex-scan raw text (string-literal false matches); finds are already flattened |
| B. New pure module `inline_ignores.rs` at `run_checker_against_source`, over structured `Vec<Finding>` | Source and findings both in hand; tree-sitter comment nodes defeat string matches; one call site covers hook/MCP/run/daemon/LSP and the HEAD baseline | Needs its own (second) parse when a marker is present |
| C. Each checker filters its own findings | Anchors exact per checker | ~30 checkers touched; every new checker must remember |

**Chosen: B.** The second parse only happens when `source.contains("kibitzer")` is true, so hook-path cost is a substring search.

---

## Pattern Decisions

| Component | Pattern Chosen | Source | Alternative Rejected | Reason |
|-----------|---------------|--------|---------------------|--------|
| `inline_ignores::apply_inline_ignores` | Pure function, Transaction Script | PoEAA | Domain Model / stateful filter object | One input to one output, no identity or lifecycle |
| Seam in `run_checker_against_source` | Adapter/facade seam over `check.rs` | GoF / Feathers | Inline logic in `check.rs` | `check.rs` is 3043 lines; keep logic out of it |
| Hook-side post-pass (Stories 2.2.3, 3.1.2, Task 2.2.3c) | New module `src/inline_post_pass.rs`, one call after the loop in `run_checks_for_trigger` | Transaction Script, same seam principle as above | Growing `run_checks_for_trigger` (7 args, in a 3043-line file) with rerun orchestration, advisory building and fallback logic | Round-3 engineering: three tasks (2.2.3a, 3.1.2a, 3.1.1b) would otherwise add logic to `check.rs` against the seam decision. `check.rs` gains: one call, the `SourceCheck`/shown-findings code in `run_native_check`, and `raw_findings_for_check` (a thin wrapper) |
| `RuleId`, `Reason` | Newtype with validating constructor | type-driven-design | raw `String` | Empty reason / blank rule unrepresentable |
| `DirectiveParse`, `MalformedReason`, `DirectiveKind` | Sum types, exhaustive match | type-driven-design | `Option<Directive>` + error strings | Malformed vs absent must be distinguishable so it can be reported |
| `inline-ignore` checker | Existing `Checker` plugin pattern (`inventory::submit!`) | repo convention (`src/checker.rs:123`) | New hook in `check.rs` for diagnostics | Gets every entry point and `default_checks()` for free |
| Comment scanning | Strategy by file type: tree-sitter / pulldown-cmark / regex | GoF | One regex over raw text | Regex matches inside string literals and fences (pitfalls 7) |
| Raw mode | `InlineIgnoreContext` on already-threaded `AcceptedFindings` | pragmatic | New `SuppressionContext` param on 10+ signatures | `run_check`/`run_checks_for_trigger` already take `&AcceptedFindings` (`src/check.rs:166,1460`); avoids a wide signature change. Known compromise: a CLI-mode field on a `Deserialize` struct, kept `#[serde(skip)]` + `Default`; refactor to a separate context only if a third suppressor appears (declined now: 23+ call-site churn) |
| Suppressed-count footer | Per-run `Arc<SuppressionCounts>` in `InlineIgnoreContext`, created by `run.rs`, incremented by `apply_inline_ignores` (total and blocking-severity counts) | pragmatic | Process-wide `static AtomicUsize`; new field on `CheckResult` | No process-wide state (parallel `cargo test`, daemon, LSP can not interfere); `CheckResult` is built at 16 sites in 4 files. HEAD-baseline replays use a context with no counter, so they are not counted. `apply_inline_ignores` receives the checker's `Severity` to classify blocking suppressions |
| Directive scan sharing | `ScanMemo`: single-entry cache inside `InlineIgnoreContext`, keyed by `(path, content hash)`; `apply_inline_ignores` also returns early when `findings.is_empty()`, and the `contains("kibitzer")` fast path runs before any hash or lock | pragmatic | Re-scan per checker; `static`/`thread_local` cache (unbounded in daemon/LSP, cross-test interference) | Each check is a separate `run_native_check` call (`src/check.rs:422`) driven from the loop in `run_checks_for_trigger` (`src/check.rs:1453-1479`), so no object spans one file's checks except the context they all already receive. Checks for one file run consecutively, so a one-entry cache gives about one scan per marker file. Ownership: the context (dropped with the run or request). Forbid `static`/`thread_local` in review. Fallback if the Mutex shows in profiles: drop the memo and rely on the early return plus fast path (most checkers return empty, so rescans are rare) |

---

## Tech Debt Disposition

| Area | Existing Issue | Disposition | Justification |
|------|----------------|--------------|----------------|
| `src/check.rs` (3043 lines, ~25 fns) | Large; `run_native_check`/`run_checker_against_source` is the natural but crowded filter site | Isolate via seam | Pure logic lives in `src/inline_ignores.rs`; hook-side post-pass logic lives in `src/inline_post_pass.rs` (Task 2.2.0a). `check.rs` gains one call in `run_checks_for_trigger`, a context argument, a severity argument, a `SourceCheck` return struct, the shown-findings computation in `run_native_check`, and `raw_findings_for_check` (Tasks 1.2.2b1a-b3, 2.2.0a, 2.2.3a) |
| `src/accepted_findings.rs` `extract_rule` (:105, private) | Rule namespace logic private to one module | Leave unchanged | Inline matching has its own `rule_matches` (it must treat `markdown-link-integrity` ref-id prefixes differently); `accepted/` behavior is untouched |
| `src/checkers/comment_quality.rs` `comment_kinds` (:173, private) | Needed by a second consumer; widening it would make `inline_ignores` depend on one checker while `comment_quality` depends back on `inline_ignores::is_directive_comment` | Move | Move `comment_kinds` to `src/tree_walk.rs` (shared, next to `walk_preorder`); `comment_quality` and `inline_ignores` both depend downward on it. `comment_quality` gains only a skip for `kibitzer:` comments |
| `src/mcp.rs` (3350 lines), `src/hook.rs` | Large | Extend as-is | Only a footer string and one instruction sentence (text edits, no new logic) |

---

## Migration Plan
Omitted: no schema or data changes. `.kibitzer/accepted/` format and behavior are untouched.

## Observability Plan
- **Logs**: none new (CLI/hook tool; findings are the output). `kibitzer run` footer prints `N findings suppressed inline (B from blocking checks) (rerun with --no-inline-ignores to see them)`, counted per run (HEAD-baseline replays excluded). The blocking count is repo-level, so per-file `[ignore-volume]` cannot hide a spread of 1-4 ignores per file.
- **Metrics**: none; the NFR is checked by structural counters plus a relative-ratio timing test, and the hook-path rerun latency budget below is recorded in the PR (Task 4.1.1c).
- **Alerts**: no new alerts required.

## Non-functional budgets
- **No-marker fast path** (the common hook case): `apply_inline_ignores` does only an emptiness check, a mode check, and `source.contains("kibitzer")`; no hashing, locking, parsing, or allocation beyond the early return. Gated by structural counters (`scan_memo.scans == 0`, `hash_calls == 0`), not an absolute time.
- **Marker file, one checker**: one extra tree-sitter parse per file per context (memoized across the ~30 checkers), only when a covering candidate exists.
- **Hook-path raw rerun** (Story 2.2.3): runs only when a directive's own rows intersect `changed_lines`, and reruns only the checkers that own the directive's rules (not the full check set), through `raw_findings_for_check`, which bypasses `run_native_check` entirely (no diff-scoping, no `accepted/`, no baseline). Budget: added work at most equal to one more run of those owning checkers on that file (at most +100% of that file's native-check time), no added work for edits that touch no directive row. I/O: the post-pass makes one extra read of the edited file (page-cache hot) per hook call when `changed_lines` is `Some`, gated by `source.contains("kibitzer")` before any scan; it does not reuse a checker's in-memory source because `run_checks_for_trigger` does not hold one. The earlier "no I/O beyond the source already in memory" claim was wrong and is withdrawn. Absolute target, INFERRED until Task 4.1.1c measures it: under 150 ms added on this repo's largest source file with a marker. Enforcement: the rerun counter test (zero reruns when no directive row changed) plus the ratio assertion in Task 2.2.3b; the number is first measured by the Task 2.2.3d spike (with a decision rule if it misses) and re-measured at Task 4.1.1c, and goes in the PR body.
- **Footer tokens** (Story 3.1.1): the hook footer is paid on every failing hook call, so it is a budget, not a free teaching surface. Caps: whole footer at most 640 characters (about 160 tokens), net added over today's measured 423-character footer text at most 215 characters (about 54 tokens; research/ux.md targeted about 45, the 9-token gap buys the three clauses that close P1/P2 failures: anchor row, marker choice, rule ids). INFERRED at 4 characters per token; the net-token break-even is computed in Phase 0 Task 0.1.1 and re-checked at Task 4.1.1c.
- **Daemon memo contention**: the single-entry `ScanMemo` Mutex can thrash between two files under concurrent daemon requests (correctness is unaffected: a different path or hash simply rescans). Perf is INFERRED; Task 4.1.1c measures two alternating marker files; the documented fallback (drop the memo, keep the early return and fast path) applies if it shows.
- **Requirements NFR deviation**: the requirements say "single pass over already-read text"; the marker path adds a second parse, gated by the substring fast path and the empty-findings early return. Stated here so it is not a surprise at review.

## Risk Control
- **Feature flag**: not gated; an opt-out exists per repo through the existing `.kibitzer/inspect.json` check disabling for `inline-ignore`, and `--no-inline-ignores` on `kibitzer run`.
- **Rollback procedure**: standard revert via PR close + revert commit; no persisted state.
- **Staged rollout**: full rollout on merge; release via the CLAUDE.md "Cutting a release" flow.

## Unresolved Questions
One gate, not a question about the design: whether to build at all depends on the Phase 0 baseline (decision rule in Task 0.1.3). Everything below is resolved given a PROCEED or SHRINK result.
- RESOLVED (was Q1): `file-complexity` emits one `Finding` per complex function, at that function's own line, all with the same aggregate message, and only when at least `MIN_COMPLEX_FUNCTIONS` functions exceed the threshold (`src/checkers/complexity.rs:55-96`). A directive above one function suppresses only that function's finding; the others stay. A directive in the first `FILE_HEAD_LINES` lines suppresses all of them (the whole-file convention). The anchor table and fixtures reflect this.
- RESOLVED (was Q2): the `inline-ignore` checker's globs are the `Language::ALL` extensions plus `*.md` (the file types with a real comment scanner). The leading-comment regex fallback still applies to suppression for any other file a native checker covers, but malformed directives in fallback-only files are not reported (documented limit). No file-type survey is needed.

Research correction: `research/features.md` §2 says `god_class.rs:520` emits `line: 0`. That line is a test fixture (`FieldAccessEdge { line: 0 }`), not a `Finding`, and god-class is an architecture check (out of scope). Line-0 handling is therefore only a defensive normalization (0 treated as 1), not a design driver.

## Accepted risks

- **Marker adoption (pre-mortem #5)**: resolved by Amendment 1 (single marker); no steering clause, no worklist.
- **`KNOWN_RULES` staleness**: advisory only, never gates suppression; see Task 2.1.1c.
- **Two-word lie in `Reason`**: the quality rule rejects boilerplate, not intent; the repo-level blocking count and the hook advisory (Story 3.1.2) are the remaining checks. The two-word floor is kept, not raised (round-3 UX gap 8): a mechanical floor cannot tell `-- legacy code` from a real constraint, and a boilerplate denylist is whack-a-mole that grows the message surface. The control is data: Phase 0 Task 0.1.1 reads the reasons agents gave in prose for the 20 classified cases, and the 30-day review reads a 20-directive sample; if more than 10% of sampled reasons are boilerplate that passes the floor, apply the ADR-002 lever (stricter reason check) before anything else.
- **`[ignore-volume]` fires once per file** (round-3 UX gap 6): anchored at the 5th directive's row only. A 6th directive added in a later edit is not re-flagged in the hook, because its row is the only changed row and no finding sits there. ACCEPTED, not fixed: firing at every directive from the 5th on would make `kibitzer run` print N-4 lines for a file the maintainer already knows is heavy, and the visibility that matters for later additions is the repo-level `run` footer count and the diff itself, which the maintainer reviews. If the 30-day sample shows files growing past 5 without review, lower the threshold or re-fire at every 5th (ADR-002 lever).

## Design Decisions (resolved)

1. **Syntax**: `<leader> kibitzer:ignore <rule>[,<rule>...] -- <reason>` and `kibitzer:false-positive ...`. ASCII `--` (`em-dash-overuse` is a default check). One reason is shared across a comma list.
2. **Scope**: comment on the finding's line (any row the comment spans) or on the row directly above the finding (physical, no skipping decorators or attributes). Rationale: `Finding` carries one line only (`src/checker.rs:12-15`), so the mechanical rule is "match the reported line"; the `file:line:` prefix in every finding tells the author which line.
3. **Multi-line findings**: anchor is the checker's reported line; the ignore goes there or one above. Per-checker convention table in Story 1.2.1. A trailing comment on a code line covers only its own row; the "one above" rule applies only to whole-line comments.
4. **Markers**: one (`ignore`) after Amendment 1; `false-positive` is a near-miss, not a directive.
5. **Blocking checks**: suppressible (ADR-002).
6. **Malformed**: reported by checker `inline-ignore` as `[ignore-syntax]` at the comment's row, with the exact fix; does not suppress; the original finding stays visible.
7. **Unused**: `[unused-ignore]` in two places with different status. (a) Hook path, Story 2.2.3 (**core**, in the committed slice): for a directive whose own rows fall inside `changed_lines` (the agent just added or edited it) that covers no raw finding, emit `[unused-ignore]` with the nearest raw finding row. This is the P1 wrong-row mitigation (pre-mortem #1) and the key correction loop, because the hook is where a misplaced directive happens; the footer hint alone teaches only the first finding's row. Never emitted for untouched directives. (b) `kibitzer run`, Story 2.2.2 (**pre-declared first cut**, may ship as a follow-up PR): unscoped (`changed_lines == None`), every directive. Judged against raw (ignores-disabled, `accepted/`-free) findings so an ignore shadowed by `accepted/` is still "used". The plan stays correct if 2.2.2 ships later: 2.2.3 reuses `unused_ignores` (Task 2.2.2a, the pure `unused_ignores` function, is shared and belongs to the committed slice, so 2.2.3 does not depend on any `run.rs` work). The hook variant is bounded: it reruns the owning checkers raw only for files whose changed lines contain a directive, which is rare (latency budget in Non-functional budgets).
8. **Precedence with `accepted/`**: inline runs first (inside `run_checker_against_source`), `drop_accepted_findings` second (`src/check.rs:482-489`); both are independent suppressors.
9. **HEAD baseline**: `check_native_against_git_head` (`src/check.rs:594`; its `run_checker_against_source` call is at :623) receives the run's `InlineIgnoreMode` but never the counter. In `Apply` an ignore present at HEAD is honored when judging "predates your edits"; under `--no-inline-ignores` the baseline is raw too, so raw current findings are compared with raw HEAD findings and none is misreported as new.
10. **Rule resolution** (BLOCKER fix): a directive rule matches a finding when it equals the checker name, or equals the finding's leading `[x]` prefix and the checker is not in `DYNAMIC_PREFIX_CHECKERS`. `markdown-link-integrity` messages start with a ref id (`[{ref_id}] used but never defined`, `src/checkers/markdown_link_integrity.rs:264,295,379,397,400`), so `accepted_findings::extract_rule` would return the ref id; that checker is therefore matched by checker name only. Matching does not consult `KNOWN_RULES`, so checkers outside `src/checkers/` (e.g. `src/single_call_site_delegation.rs:69`) or with built ids never fail closed. `accepted_findings::extract_rule` and `accepted/` behavior are untouched. Edge: a finding from a non-dynamic checker with a data-valued bracket prefix could only be matched by naming that prefix, which is harmless.
11. **Non-UTF-8 files** (BLOCKER fix): `run_checker_against_file` (`src/check.rs:576-589`) turns a read error into a failed `CheckResult` (:456-470), and `kibitzer run` walks every non-skipped file (`walk_and_collect_files`, :1491-1499). For `inline-ignore` only, a read or UTF-8 error returns `(String::new(), true)` (no findings, no failure); other checkers keep their current failure semantics. Globs are also narrowed (Q2 above) so most binaries are never read.

12. **Cache hits keep the inline outcome** (round-2 engineering fix): `InlineOutcome` is serialized (`#[serde(default)]`), not `skip_serializing`, so a daemon cache hit (`src/daemon.rs:155-165`) still carries the footer anchor and dropped list; see Task 1.2.2b1c for the rejected alternatives (bypass when the file contains `kibitzer`; recompute on hit).

13. **Cache is invalidated on binary version** (round-3 engineering fix): cache entries are keyed on file, config and registry stamps and the trigger only (`src/cache.rs:84-103`), so a result cached by a pre-feature binary would persist across an upgrade with unsuppressed findings until the file is touched. `Cache` gains a `#[serde(default)] kibitzer_version: String`; `Cache::load` returns an empty cache when it differs from `env!("CARGO_PKG_VERSION")` (an old `cache.json` has no key, so it defaults to empty and is discarded too), and `save` stamps the current version. Chosen over per-entry keys because one comparison covers every entry and it needs no change to `get`'s 4-argument signature. Limit: dev builds that share a `Cargo.toml` version share a cache; document "delete `cache.json` or restart the daemon after rebuilding" (Task 1.2.2b1c).
14. **Raw findings use one structured path, never text** (round-3 engineering fix): `run_checker_against_source` returns `SourceCheck { combined, passed, findings, inline }`; `raw_findings_for_check(check, file_path, source) -> Result<Vec<RawFinding>>` calls it with a `Disabled` context and maps `findings`. It does not call `run_native_check`, so the `:473` diff-scoping and `:482` `accepted/` drop cannot hide the raw finding at row 20 when `changed_lines = Some(&[(12,12)])`, and the plan's own rule (never parse rendered text) holds. `CheckResult` is not extended with raw findings: it is the cached, serialized type and `Finding` is not `Serialize`.

## Scope cut order

See "Scope disposition" in Effort and appetite for the keep/defer decision on every beyond-ask feature. Never cut the core seam (Epics 1.1-1.3, Story 2.1.1).

Never cut (guardrails and correction loops from the pre-mortem, P1-1 and P1-2): Story 2.1.2 (`[ignore-volume]`, anchored at the 5th directive's row so it survives diff-scoping at `src/check.rs:473`); the blocking count in the `run` footer (Story 2.2.1); `[blocking-suppressed]` (Story 3.1.2); the `Reason` quality rule (Task 1.1.1d); Story 2.2.3 (wrong-row advisory), except in the SHRINK slice chosen at the Phase 0 gate.

Cut order if the appetite slips (committed work only; Task 2.2.2b and the success ack are already deferred):
1. The long tail of Task 1.2.1b fixtures (about 3h, `ANCHOR_PENDING`).
2. Task 2.2.2e (1h), then Task 2.2.1c (0.5h).
3. Stop and ask the requester.
Deferred already (follow-up PRs, not in the committed total): Task 2.2.2b; the success ack line; the `run` footer `false-positive` split. Docs and Story 3.3.1 text must not promise `[unused-ignore]` in `kibitzer run` unless 2.2.2b shipped in the same PR (Task 3.3.1b checks this).
Hook footer and MCP hints (Story 3.1.1) are kept: requirements say agents must learn the syntax without a doc fetch.

## Dependency Visualization

Edges are `A -> B` meaning B cannot start until A is done. The round-3 review found omitted and circular edges; this list replaces the earlier picture.

```
Phase 0 (gate):  0.1.1 -> 0.1.2 -> 0.1.3 -> {everything in Phases 1-4}      (0.1.3 records PROCEED / SHRINK / STOP)

Parsing:         1.1.1a -> 1.1.1b -> 1.1.1c;  1.1.1b <-> 1.1.1d (built together)
                 1.1.1a -> 1.1.2a -> 1.1.2b
                 1.1.1c, 1.1.2b -> 1.2.1a -> 1.2.1b
                 1.1.1b -> 1.3.1a -> 1.3.1b                    (needs only is_directive_comment; independent of 1.2.2*)

Seam:            1.2.1a -> 1.2.2a -> 1.2.2b1a -> 1.2.2b1b -> 1.2.2b2
                 1.2.2b1b -> 1.2.2b1c                            (cache hit handling and version stamp)
                 1.2.2b1b -> 2.1.1a -> 1.2.2b3 -> 2.1.1b -> 2.1.1c -> 2.1.2a
                 (1.2.2b3's regression test runs the inline-ignore checker, so it follows 2.1.1a; 2.1.1c's binary-file test then
                 reuses it. This removes the earlier 2.1.1c <-> 1.2.2b3 cycle.)

Post-pass:       1.2.2b2 -> 2.2.0a                               (post-pass module: severity is threaded, SourceCheck exists)
                 2.2.0a, 1.2.2b1c -> 2.2.3a -> 2.2.3d -> 2.2.3c -> 2.2.3b
                 1.2.2a (unused_ignores types) -> 2.2.2a -> 2.2.3a
                 2.2.0a, 1.2.2b2 -> 3.1.2a                       (3.1.2a needs the post-pass module and DroppedFinding.severity, NOT 2.2.3a)

Footer/run:      1.2.2b2 -> 2.2.1a -> 2.2.1b -> 2.2.1c
                 2.2.3c -> 2.2.2e                                (2.2.2e reuses the unowned-rule judgement)
                 2.2.2a, 2.2.1b -> 2.2.2b                        (deferred follow-up)

Teaching:        1.2.2a -> 3.1.1a -> 3.1.1b -> 3.1.1c -> 3.1.1d
                 1.2.2b1b, 1.2.2b1c, 2.2.0a -> 3.1.1b            (shown-findings anchor and rule ids on CheckResult, cache-surviving)

Listing:         (dropped, Amendment 1)

Docs:            {2.1.2a, 2.2.1b, 2.2.1c, 2.2.2e, 2.2.3b, 3.1.1d, 3.1.2a} -> 3.3.1a -> 3.3.1b

Validation:      3.3.1b -> 4.1.1a -> 4.1.1b
                 {1.2.2b1b, 2.2.3d, 3.1.1b} -> 4.1.1c            (scan counters, rerun re-measure, footer length)
                 {4.1.1b, 4.1.1c} -> 4.1.1d -> 4.1.1f
```

---

## Phase 0: Baseline replay and go/no-go gate (before any implementation)

**Goal**: Turn the unmeasured problem statement into a measured decision before spending the implementation hours. Round 3 (product) found that the re-surfacing baseline was measured only at the END of the plan, so kill criterion 1 would be evaluated after about 52h were spent. This phase moves it to the front. It needs no code from this feature: `kibitzer check backtest` already exists (`docs/backtesting.md`).

**Phase 0 does not edit `src/`.** It produces `scripts/resurface-baseline.py`, results under `docs/backtest-triage/`, and a one-page gate note in the PR.

#### Task 0.1.1: Baseline replay, classification, and net-token computation (~2.5h; moved from the old Task 4.1.1e, plus the net-token computation)
- Method (unchanged from the old 4.1.1e): reuse `kibitzer check backtest all --only-new` over `~/.claude/projects` (`docs/backtesting.md`; output is `<transcript>#<seq> <file>:<line>: [<checker>] <message>`, tagged `(pre-existing)` when the finding also fired before the edit). `scripts/resurface-baseline.py` (same style as `scripts/backtest-triage.py`) groups per transcript by `(rule, file)` and counts a re-surfacing when the same `(rule, file, message)` is reported at an earlier sequence number, the agent made at least one further edit to that file in between, and the finding is `(pre-existing)` at the later sequence number. Report: `R` = tuples with at least one re-surfacing over tuples with at least one finding, `F` = number of tuples with at least one finding, per-rule split, median re-reports per re-surfaced tuple.
- Hand-classify 20 re-surfaced tuples (all of them if fewer than 20) into: (a) agent addressed it and it regressed, (b) agent deliberately left it, (c) checker misfire. `A` = share of (b)+(c). Read the reason the agent gave in prose for each (b)/(c) case (assumption A input; boilerplate count recorded for the two-word-floor decision).
- Gate metric: `G = R x A`, the share of tuples whose re-surfacing an inline directive could address. This is the number the decision rule uses; `R` alone over-counts (class (a) is excluded because a re-report after a fix may be a real regression).
- Net-token computation (new; UX gap 1): from the same replay count `E` = failing hook-visible events (edit events with at least one new finding) and `S` = addressable re-surfaced findings (classes b+c, summed over tuples). Inferred unit costs, both labelled INFERRED: `c_footer` about 54 tokens (the footer cap, Non-functional budgets) paid on each of the `E` events, and `c_re` about 150 tokens per re-surfaced finding (rendered line about 40 tokens plus a re-triage turn). Feature is token-positive only if `S x c_re x p > E x c_footer`, where `p` is the share of addressable re-surfacings the directive actually prevents (starts at the 50% adoption assumption). Record the break-even `p* = (E x c_footer) / (S x c_re)`. If `p* > 0.5`, the footer cap is too large for this repo's mix: SHRINK and cut the footer to the generic syntax line (about 235 characters before trimming, no anchor-row clause) or stop.
- Honest limits, stated in the gate note: (1) the replay runs today's checkers, not what the hook showed the agent then; (2) a transcript cannot show a dismissal, only that the agent edited and the finding persisted, which is why classification is by hand; (3) token costs are inferred from counts, not measured; (4) the maintainer's transcripts are a single-user sample.
- Files: `scripts/resurface-baseline.py` (new), `docs/backtest-triage/` (results)

#### Task 0.1.2: Marker-choice dry run (~1h)
- Why now: assumption C (agents choose `false-positive` vs `ignore` correctly from one footer clause) and assumption B (the worklist gets triaged) were tested only after ship. A wrong answer collapses Story 3.2.1 and half of Story 3.1.1's text, so test it before building.
- Method: take the 20 classified cases from Task 0.1.1 (rewrite each as a short finding plus the surrounding edit context) and the draft clause `false-positive: the checker is wrong here; ignore: the code is right for a stated reason`. Have a fresh agent session with no other context choose a marker for each; compare with the hand label (class (c) checker misfire = `false-positive`, class (b) deliberately left = `ignore`). Also read the class (c) share: if (c) is under 10% of (b)+(c), the worklist the marker split feeds would be nearly empty.
- Decision: agreement at least 70% **and** class (c) at least 10%: keep two markers. Otherwise collapse to one marker (`kibitzer:ignore`), drop Story 3.2.1 (-2.5h) and the marker clause from the footer, and record it in `requirements.md`. One reword of the clause and one re-run are allowed before collapsing.
- Files: none (results in the gate note)

#### Task 0.1.3: Gate decision (~0.5h)
- Write the gate note (PR body or `docs/backtest-triage/inline-ignore-gate.md`): `R`, `F`, `A`, `G`, `p*`, the marker result, and the decision. The requester (tstapler) decides; if the note is not answered, nothing in Phase 1 starts (the default is not "proceed").
- **Decision rule** (all thresholds PROVISIONAL judgement calls, flagged as such: 2% is "less than about one re-surfaced finding in 50", 5% is "about one in 20"; they are revised only with this task's data in hand, never after the fact):
  - Sample sufficiency first: need `F` at least 100 tuples and at least 10 re-surfaced tuples to classify. If short, widen once to every transcript under `~/.claude/projects`; if still short, the problem is too rare to measure: **STOP**.
  - `G >= 5%` and `p* <= 0.5`: **PROCEED** with the full committed slice (62h).
  - `2% <= G < 5%`, or `G >= 5%` with `p* > 0.5`: **SHRINK**: the SHRINK slice (49.0h): no Tasks 2.2.2a, 2.2.2e or 2.2.3a-d, single marker and no Story 3.2.1, footer cut to the generic syntax line if `p* > 0.5`. Keep the seam, malformed errors, `Reason` rule, `[ignore-volume]`, blocking count, `[blocking-suppressed]`, hook/MCP teaching, docs.
  - `G < 2%`: **STOP**. Revert to the pure `accepted/` path and reopen the agent-writable accept tool as the alternative (`requirements.md` Roadmap Fit). Archive this plan.
- **Revised target** (replaces the placeholder "at least 50% lower than baseline"): the post-ship re-surfacing rate of dismissed findings should be at most `R - 0.5 x G`, that is, remove half of the addressable portion. The 50% factor is itself an adoption assumption (agents place the directive correctly for half the addressable cases, backed by the anchor-conformance test and the exact-row hint); it is recorded in the gate note and revised at the 30-day review using observed adoption. Pre-ship the outcome cannot be observed (transcripts predate the feature), so Task 4.1.1d adds a counterfactual coverage check on the classified cases instead (see there); this bounds the achievable reduction, it does not prove it.
- Files: none (gate note)

---

## Phase 1: Core parse, match, apply

### Epic 1.1: Directive parsing
**Goal**: Turn a file's real comments into `Directive`s with no string-literal false matches.

#### Story 1.1.1: Parse directives from code-language comments
**As a** kibitzer maintainer, **I want** directives read only from tree-sitter comment nodes, **so that** a `kibitzer:ignore` inside a string literal never suppresses anything.
**Acceptance Criteria**:
- A valid directive in a comment of each of the 7 grammars (Go, TS/TSX, JS, Python, Java, Kotlin, Rust) parses to a `Directive`.
  - *Given* Go source `// kibitzer:ignore flag-argument -- legacy API, callers pinned` at 1-based line 7, *When* `scan_directives` runs, *Then* one `Directive{kind: Ignore, rules: [RuleId("flag-argument")], reason: Reason("legacy API, callers pinned"), start_line: Line(7), end_line: Line(7), whole_line: true}`.
- (Amendment 1) Python `# kibitzer:false-positive primitive-obsession -- id is opaque` is `Malformed(NearMissMarker)` and does not suppress; `# kibitzer:ignore primitive-obsession -- id is opaque` parses as `Valid`.
- A directive inside a string literal is ignored.
  - *Given* Rust `let s = "// kibitzer:ignore x -- y";`, *When* scanned, *Then* zero directives.
- Missing rule or reason is `Malformed`, never `Valid`.
  - *Given* `// kibitzer:ignore flag-argument` , *When* parsed, *Then* `Malformed(MissingReason)`; *Given* `// kibitzer:ignore -- because`, *Then* `Malformed(MissingRule)`.
- A low-effort reason is `Malformed(WeakReason)` and never suppresses; too-short and rule-echo are distinct kinds with distinct messages (Story 2.1.1).
  - *Given* `// kibitzer:ignore flag-argument -- needed`, *Then* `Malformed(WeakReason(TooShort))`; *Given* `// kibitzer:ignore flag-argument -- flag-argument` or `-- Flag Argument`, *Then* `Malformed(WeakReason(RuleEcho))` (checked first, so a two-word echo is `RuleEcho`, not accepted); *Given* `-- legacy API, callers pinned`, *Then* `Valid`.
- An em or en dash separator gets its own repair message, not the missing-reason one.
  - *Given* `// kibitzer:ignore flag-argument — legacy API, callers pinned` (U+2014; also U+2013), *When* parsed, *Then* `Malformed(EmDashSeparator)`, never `MissingReason`.
- Rule lists are strict: comma-separated, no spaces, no empty elements.
  - *Given* `// kibitzer:ignore a,b -- legacy API, callers pinned`, *Then* `Valid` with rules `[a, b]` and one shared reason; *Given* `a,a`, *Then* `Valid` with `[a]` (deduplicated); *Given* `a, b -- why not` (space after comma), `a,,b`, `a,` or `,a`, *Then* `Malformed(BadRuleList)`; *Given* `a,<rule> -- why not`, *Then* `NotADirective` (a placeholder element makes the whole line a doc placeholder, same as the single-rule case).
- Near miss spellings are `Malformed(NearMissMarker)`, but only when the stripped comment text starts with the marker (same position as the exact form).
  - *Given* `// kibitzer: ignore foo -- bar`, *When* parsed, *Then* `Malformed(NearMissMarker)`.
  - *Given* prose `// see kibitzer: allow list in docs` or `// the kibitzer:ignore syntax is documented`, *When* parsed, *Then* `NotADirective`.
- A complete, well-formed directive that appears after other comment text is `Malformed(NotAtCommentStart)`, never a silent no-op ("malformed is not silent"). Detection requires the whole exact grammar later in the stripped text (`kibitzer:ignore` + a valid `[a-z0-9-]+` rule list + ` -- ` + a non-empty reason), so prose and placeholders stay `NotADirective`.
  - *Given* `// TODO kibitzer:ignore flag-argument -- legacy API, callers pinned` or `// legacy: kibitzer:ignore flag-argument -- legacy API, callers pinned`, *When* parsed, *Then* `Malformed(NotAtCommentStart)` and it does not suppress (the repair text is in Story 2.1.1).
  - *Given* `// the kibitzer:ignore syntax is documented` (no rule list or ` -- `) or `// see kibitzer:ignore <rule> -- <why>` (placeholder), *Then* `NotADirective`.
- A rustdoc-style placeholder is not a directive.
  - *Given* `/// kibitzer:ignore <rule> -- <why>`, *When* parsed, *Then* `NotADirective` (rule must match `[a-z0-9-]+`).
- A block comment may carry the directive on any of its lines, reported at that line's row.
  - *Given* Java `/*\n * kibitzer:ignore long-method -- generated\n */` starting at 1-based line 10, *When* scanned, *Then* `start_line: 11`.
- Rows are 1-based `Line` values everywhere; tree-sitter rows are converted once at the scan boundary.
  - *Given* a comment on tree-sitter row 6 (0-based), *When* scanned, *Then* `start_line == Line(7)`.
- `whole_line` is recorded.
  - *Given* `x := f() // kibitzer:ignore a -- b`, *When* scanned, *Then* `whole_line == false`; for a comment alone on its line, `true`.
**Files**: `src/inline_ignores.rs`, `src/tree_walk.rs`, `src/checkers/comment_quality.rs`, `src/main.rs`

##### Task 1.1.1a: Module skeleton and visibility (~0.5h)
- Create `src/inline_ignores.rs` with `Line` (1-based newtype), `Directive` (non-empty `rules`, constructor-checked), `DirectiveKind`, `RuleId` (`[a-z0-9-]+`), `Reason` (validating `new`), `DirectiveParse`, `MalformedReason`, `Scanned`.
- Add `mod inline_ignores;` where sibling modules are declared (`src/main.rs`).
- Move `fn comment_kinds` (`src/checkers/comment_quality.rs:173`) to `src/tree_walk.rs` as `pub(crate)` and update `comment_quality` to call it there (pure move, no behavior change); this avoids an `inline_ignores` <-> `comment_quality` cycle.
- Files: `src/inline_ignores.rs`, `src/main.rs`, `src/tree_walk.rs`, `src/checkers/comment_quality.rs`

##### Task 1.1.1b: Directive line grammar (~1.5h)
- `parse_comment_line(text: &str) -> DirectiveParse`: strip leading `//`, `///`, `//!`, `#`, `/*`, `/**`, `*`, `<!--` and trailing `*/`, `-->`; match `^kibitzer:ignore\s+RULES\s+--\s+REASON$` with `regex` (already a dep); detect near-miss `^kibitzer\s*:\s*(ignore|disable|allow|suppress|false[-_ ]positive)` (anchored to the start of the stripped text, same position as the exact form) that is not exact. Detect `NotAtCommentStart` with the same exact-grammar regex unanchored (`\bkibitzer:(ignore|false-positive)\s+RULES\s+--\s+\S`) run only when the anchored forms did not match, so a prose mention without a rule list and ` -- ` stays `NotADirective`. Trim CR and tabs.
- Unit tests: valid, comma list, missing rule, missing reason, empty reason after `--`, near-miss, CRLF, tabs, em dash and en dash separators are `Malformed(EmDashSeparator)` (never `MissingReason`), rule-list edge cases (`a,b`, `a,a`, `a, b`, `a,,b`, `a,`), negative prose mentions (`// see kibitzer: allow list`, `// the kibitzer:ignore syntax ...`), rustdoc placeholder `/// kibitzer:ignore <rule> -- <why>`, and the not-at-start positives (`// TODO kibitzer:ignore x -- why here`, `// legacy: kibitzer:ignore x -- why here`) which return `Malformed(NotAtCommentStart)`.
- Files: `src/inline_ignores.rs`

##### Task 1.1.1c: Tree-sitter comment scan (~1.5h)
- `scan_code_comments(lang: Language, tree: &Tree, source: &str) -> Vec<(row, DirectiveParse)>` using `crate::tree_walk::comment_kinds(lang)` and `crate::tree_walk::walk_preorder`; split block-comment text per line and compute each line from `comment.start_position().row + 1` (single 0-based to `Line` conversion). Set `whole_line` by checking that only whitespace precedes the comment on its start row.
- Table-driven test, one fixture per `Language::ALL` (`src/checker.rs:42`), plus the string-literal negative case. Use `GrammarCache::new().parse`.
- Files: `src/inline_ignores.rs`

##### Task 1.1.1d: `Reason` minimum-quality rule (~0.5h)
- `Reason::new(text, rules: &[RuleId]) -> Result<Reason, WeakReason>` rejects: empty/blank (that is `MissingReason`), text equal to any listed rule id after lowercasing and folding `-`/`_`/space (`WeakReason::RuleEcho`, tested first), and a single whitespace-separated word (`WeakReason::TooShort`). `parse_comment_line` maps each kind to its own message: `TooShort` -> `[ignore-syntax] reason 'needed' is too short to explain the code. Write: kibitzer:ignore flag-argument -- <the concrete constraint that makes this code acceptable>`; `RuleEcho` -> `[ignore-syntax] reason repeats the rule id instead of saying why the code is acceptable. Write: kibitzer:ignore flag-argument -- <the concrete constraint that makes this code acceptable>`. Neither message states a word count: the two-word floor is a mechanical check, and the message asks for the constraint (what pins the code, e.g. "callers pinned by public API") so an agent does not satisfy the floor with `-- legacy code`. A weak-reason directive never suppresses.
- Tests: `-- needed` is `TooShort`; `-- flag-argument`, `-- Flag Argument` and a two-word echo are `RuleEcho`; `-- legacy API, callers pinned` accepted; the two messages differ, neither contains `at least two words`, both contain `concrete constraint`, no em dash. Acceptance: Story 1.1.1 AC above.
- Files: `src/inline_ignores.rs`

#### Story 1.1.2: Markdown and fallback comment scanning
**As a** doc author, **I want** `<!-- kibitzer:ignore ... -->` honored in Markdown but not in code fences, **so that** `docs/suppressing-checks.md` showing the syntax suppresses nothing.
**Acceptance Criteria**:
- An HTML comment directive in Markdown parses.
  - *Given* `.md` line 5 `<!-- kibitzer:ignore em-dash-overuse -- quoted source -->`, *When* scanned, *Then* one `Directive` at row 5 with rule `em-dash-overuse`.
- A directive inside a fenced code block is ignored.
  - *Given* a ```` ```md ```` fence containing `<!-- kibitzer:ignore a -- b -->`, *When* scanned, *Then* zero directives.
- Files with no grammar and no Markdown use a leading-comment regex (whole-line comments only).
  - *Given* a `.sh` file line `# kibitzer:ignore file-size -- vendored`, *When* scanned, *Then* one `Directive`; *Given* `echo "# kibitzer:ignore a -- b"`, *Then* zero.
- Files without the substring `kibitzer` skip all parsing.
  - *Given* source of 10,000 lines with no `kibitzer`, *When* `scan_directives` is called, *Then* it returns empty without constructing a parser (unit test asserts via a `GrammarCache` that was never used, or timing under the Task 4.1.1c budget).
**Files**: `src/inline_ignores.rs`

##### Task 1.1.2a: Markdown scan (~1.5h)
- `scan_markdown(source)` with `pulldown_cmark::Parser::new_ext(..).into_offset_iter()`; consider `Event::Html`/`InlineHtml` whose text is an HTML comment; skip content inside `Tag::CodeBlock`; row from byte offset. Reuse the options the existing markdown checker uses (`src/checkers/markdown_link_integrity.rs:5`).
- Tests: top-level comment, inline trailing comment in a table row, fenced negative, indented code block negative.
- Files: `src/inline_ignores.rs`

##### Task 1.1.2b: Fallback and dispatcher (~1.5h)
- `scan_leading_comments(source)` regex `^\s*(?://|#|;|--)\s*kibitzer:`; `scan_directives(path, source) -> Vec<Scanned>` dispatching by `Language::for_path` (`src/checker.rs:104`) / `.md` / fallback, with the `source.contains("kibitzer")` fast path first. `scan_directives_memoized(ctx, path, source) -> Arc<Vec<Scanned>>` wraps it with the `ScanMemo` owned by `InlineIgnoreContext` (single entry keyed by `(path, content hash)`; a different path or hash replaces it; increments `scans` only on a real scan). The ~30 per-checker calls on one file share one parse because they run consecutively against the same context. No `static`/`thread_local`.
- Files: `src/inline_ignores.rs`

### Epic 1.2: Matching and application at the seam
**Goal**: Drop covered findings before they are flattened to text, for every entry point.

#### Story 1.2.1: Anchor convention
**As an** agent, **I want** a predictable placement rule for every checker, **so that** I put the comment where it will match on the first try.
**Acceptance Criteria** (convention, one row per awkward checker; each backed by an integration test running the real checker):
- Standard statement-anchored finding: same line or line above.
  - *Given* a whole-line `Directive{rules:[flag-argument], end_line: 9}` and a finding `[flag-argument]` at line 10, *When* `covers`, *Then* true; at line 11, false.
- A trailing comment on a code line covers only its own row.
  - *Given* `x := f() // kibitzer:ignore flag-argument -- legacy` at line 9 and `[flag-argument]` findings at lines 9 and 10, *When* applied, *Then* line 9 is dropped and line 10 stays.
- Reference-style `markdown-link-integrity` findings resolve by checker name (their `[x]` prefix is a link ref id).
  - *Given* `.md` with `[foo]` used but never defined (finding `[foo] used but never defined`) and `<!-- kibitzer:ignore markdown-link-integrity -- placeholder -->` on the line above, *When* the real checker runs then `apply_inline_ignores`, *Then* the finding is dropped; a directive naming the ref id `foo` does not match it; and the same holds for the "defined but never used" and "-> file does not exist" variants.
- `file-size` (reports `line: lines`, last line, `src/checkers/file_size.rs:120-126`): a directive in the first 10 lines also covers.
  - *Given* a 900-line Go file with `// kibitzer:ignore file-size -- generated tables` on row 1 and a `[file-size]` finding at line 900, *When* applied, *Then* the finding is dropped.
- `file-complexity` emits one finding per complex function at that function's own line, all with the same aggregate message (`src/checkers/complexity.rs:55-96`): a directive above one function drops only that function's finding; a directive in the first 10 lines drops all of them.
  - *Given* a Go file with 3 complex functions (at or above `MIN_COMPLEX_FUNCTIONS`) and an ignore above the first, *When* applied, *Then* 2 findings remain; with the ignore on line 1, *Then* 0 remain.
- `duplicate-code` anchors at the last occurrence (`src/checkers/duplicate_code.rs:115`): the ignore goes on the row directly above the last block's first line, as a whole-line comment. A trailing comment on the block's first line becomes part of the duplicated block text and re-anchors the finding.
  - *Given* duplicates at lines 5 and 40 with the ignore above line 40, *When* applied, *Then* the finding is dropped; with the ignore above line 5, *Then* it stays.
- `duplicate-code-cross-file` is reported per file at `start + 1` (`src/checkers/duplicate_cross_file_checker.rs:261-269`); the ignore covers only the file it is in, and goes on the row directly above the block's first line (a trailing comment changes the block text and re-anchors the finding, same as `duplicate-code`).
  - *Given* files `a.go` and `b.go` each flagged and an ignore only in `a.go`, *When* both are checked, *Then* `a.go` is clean and `b.go` still reports.
- `over-commented` anchors at the first leading comment row; `commented-out-code` at each commented-out row: ignore directly above that row (an ignore comment is exempt from both, Story 1.3.1). For a run of dead-code lines, delete the code instead.
  - *Given* an ignore above a leading comment block starting row 20 and a `[over-commented]` finding at line 20 after the ignore row is exempt, *When* applied, *Then* dropped.
- Meta rules are never covered.
  - *Given* `// kibitzer:ignore ignore-syntax -- x` and an `[ignore-syntax]` finding on the next line, *When* applied, *Then* the finding stays.
- Finding line 0 is normalized to 1 (in the `Line` constructor).
  - *Given* a finding at line 0 and a directive on line 1, *When* `covers`, *Then* true.
**Files**: `src/inline_ignores.rs`, `src/checkers/complexity.rs` (read only), `src/checkers/markdown_link_integrity.rs` (read only)

##### Task 1.2.1a: `covers` and `FILE_SCOPE_RULES` (~1h)
- `fn rule_matches(directive_rule: &RuleId, checker_name: &str, message: &str) -> bool` per Design Decision 10 (checker name, or leading `[x]` prefix unless `checker_name` is in `DYNAMIC_PREFIX_CHECKERS`); `covers(d: &Directive, finding_line: Line, rule_match: bool) -> bool` with the `whole_line` +1 rule; consts `FILE_SCOPE_RULES = ["file-size", "file-complexity"]`, `FILE_HEAD_LINES = 10`, `META_RULES`, `DYNAMIC_PREFIX_CHECKERS = ["markdown-link-integrity"]`.
- Unit tests per bullet above, including the trailing-comment and `markdown-link-integrity` cases.
- Files: `src/inline_ignores.rs`

##### Task 1.2.1b: Anchor conformance test over every default checker (~7h, re-estimated from 3h: about 30 fixtures that make the real checker fire, each needing trial and error, plus a completeness guard)
- Replaces the hand-picked set. A table-driven test iterates every native per-file checker in `config::default_checks()` (excluding `inline-ignore` itself, which emits only meta rules). For each it needs a fixture source that makes the real checker fire (`run_checker_configured`, real output, not a hand-built `Finding`); fixtures live in one table `ANCHOR_FIXTURES: &[(checker_name, path, source)]`. For each finding the checker reports at line N, the test builds two variants of the source, a whole-line directive on row N-1 and a trailing/same-row directive on row N (head-of-file variants for `FILE_SCOPE_RULES`), runs `apply_inline_ignores`, and asserts that finding is dropped.
- Completeness guard: a second test asserts every checker name in `default_checks()` either has an `ANCHOR_FIXTURES` entry or is listed in `ANCHOR_EXEMPT` with a reason string (meta checker, no per-file anchor). Adding a default checker without a fixture or an exemption fails the build, which is the point (agents must be able to place the comment on the first try for every checker).
- A checker whose finding cannot be covered by either variant fails the test; the fix is to document the exception in the Story 1.2.1 anchor table and the head-of-file convention, not to skip it.
- Keep the awkward-anchor cases from Story 1.2.1 as explicit extra rows (`file-size`, `file-complexity` per-function, `duplicate-code` last occurrence, `duplicate-code-cross-file` per file, `comment-quality-go`, reference-style `markdown-link-integrity`). Use `src/test_support.rs` helpers where they fit.
- Files: `src/inline_ignores.rs`, `src/test_support.rs` (read)

#### Story 1.2.2: Apply at `run_checker_against_source`
**As an** agent, **I want** one comment to dismiss a finding in hook, MCP, `run`, daemon, and LSP output, **so that** I stop re-triaging it.
**Acceptance Criteria**:
- A covered finding disappears from `run_checker_against_source` output; uncovered findings remain.
  - *Given* Go source with two `[flag-argument]` findings at lines 10 and 30 and an ignore above line 10, *When* `run_checker_against_source` runs with `Apply`, *Then* `combined` contains only `...:30: [flag-argument]...` and `passed == false`.
- When every finding is covered, the check passes.
  - *Given* the same file with ignores above both, *When* run, *Then* `passed == true` and `combined` is empty.
- The ignore survives edits elsewhere.
  - *Given* the file above with 5 lines inserted at line 1 above the ignore (ignore now row 15, finding line 16), *When* run, *Then* still suppressed (no line-number or hash key).
- Findings without a `[rule]` prefix use the checker name.
  - *Given* a `primitive-obsession` finding (no prefix) and `kibitzer:ignore primitive-obsession -- ...` above it, *When* run, *Then* dropped.
- A stale rule table cannot make a legitimate ignore fail.
  - *Given* a finding `[some-new-rule] ...` from a checker absent from `KNOWN_RULES` and a directive naming `some-new-rule`, *When* run, *Then* dropped.
- `Disabled` mode returns raw findings.
  - *Given* `mode: Disabled` and the covered file, *When* run, *Then* the finding is present.
- No findings means no scan.
  - *Given* a checker returns zero findings for a file that contains `kibitzer:ignore`, *When* `apply_inline_ignores` runs, *Then* it returns without scanning (test via the memo/scan counter on the context).
- Directive scan is memoized per context, not per checker.
  - *Given* one `InlineIgnoreContext` and a Go file containing a valid directive, *When* 30 checkers each return at least one finding and call `apply_inline_ignores` in sequence, *Then* `scan_memo.scans == 1`; *Given* the same context and the file's source edited (ignore removed) before the next call, *Then* the new call rescans (`scans == 2`) and no longer suppresses; *Given* a different path, *Then* the single entry is replaced (never more than one entry held).
- Suppressed count is per run and excludes baseline replays.
  - *Given* two `AcceptedFindings` contexts with separate counters running concurrently, each covering 2 and 3 findings, *When* both finish, *Then* `total` reads 2 and 3; *Given* one covered finding from a `Severity::Blocking` check and one from an advisory check, *Then* `total == 2` and `blocking == 1`; and *Given* a `check_native_against_git_head` replay of a file with an ignore at HEAD, *Then* no counter changes.
- The apply step reports what it dropped, not only what it kept.
  - *Given* a blocking-check finding covered by a valid directive at rows 4-4, *When* `apply_inline_ignores` runs, *Then* `AppliedIgnores.dropped` has one `DroppedFinding` with `directive_start == directive_end == Line(4)`, the rule, the kind, the reason, and `severity == Blocking`; `kept` excludes the finding. Story 3.1.2 and the counters consume this; no later signature change.
- Zero regression for `accepted/`.
  - *Given* the existing `accepted_findings.rs` tests and an `accepted/` entry matching a finding with no inline directive, *When* `cargo test accepted`, *Then* all pass unchanged.
**Files**: `src/inline_ignores.rs`, `src/check.rs`, `src/accepted_findings.rs`

##### Task 1.2.2a: `apply_inline_ignores` (~1h)
- `pub(crate) fn apply_inline_ignores(findings: Vec<Finding>, file: &Path, source: &str, checker_name: &str, severity: Severity, ctx: &InlineIgnoreContext) -> AppliedIgnores`: return `AppliedIgnores { kept: findings, dropped: vec![] }` immediately when `findings` is empty, when `ctx.mode == Disabled`, or when `source` lacks `kibitzer` (all three before any hashing or locking); otherwise `scan_directives_memoized(ctx, ..)` and, for each `Valid` directive, move findings where `rule_matches` and `covers` into `dropped` as `DroppedFinding`; add the number dropped to `ctx.counter` if present (`total`; also `blocking` when `severity == Blocking`). The return type is fixed here so Tasks 1.2.2b1b/b2 and 3.1.2a do not re-edit the signature.
- Also here: `fn anchor_rule(checker_name: &str, finding: &Finding) -> RuleId` (the leading `[x]` prefix unless `checker_name` is in `DYNAMIC_PREFIX_CHECKERS`, else the checker name), shared by `rule_matches` and the hook hint (Task 3.1.1b).
- No change to `accepted_findings::extract_rule` (`src/accepted_findings.rs:105`) or `accepted/` behavior.
- Files: `src/inline_ignores.rs`

##### Task 1.2.2b1a: Add `inline` to `CheckResult` and fix every literal (~1h)
- Add `pub inline: InlineOutcome` to `CheckResult` (`src/check.rs:31`) with `#[serde(default)]` (serialized; see Task 1.2.2b1c) and the `InlineOutcome`/`DroppedFinding` types (derive `Serialize`/`Deserialize`, plus `RuleId`, `Line`, `Reason`, `DirectiveKind`). `CheckResult` has no `Default`, so every struct literal gets `inline: InlineOutcome::default()`. Recounted with `grep -nE 'CheckResult\s*\{' src` (minus struct/impl/fn signature lines): 16 literals in 4 files: `src/check.rs` 10 (:101 test helper, :216, :235, :261, :329, :439, :460, :512, :1167, :1237), `src/cache.rs` 4 (:195, :284, :360, :368, all tests), `src/hook.rs` 1 (:250, test), `src/lsp.rs` 1 (:580, test). The earlier "23 sites in check.rs" claim was wrong.
- Mechanical, compiles on its own; no behavior change. Test: the existing suite passes unchanged.
- Files: `src/check.rs`, `src/cache.rs`, `src/hook.rs`, `src/lsp.rs`, `src/inline_ignores.rs` (5 files, at the limit; the three non-check files are one-line test-literal edits)

##### Task 1.2.2b1b: Context, seam, and shown-findings outcome (~4.5h, re-estimated from 2.5h; Tasks 1.2.2b1a + 1.2.2b1b together are about 5.5h)
- Add `InlineIgnoreContext` (default `Apply`, no counter, fresh `ScanMemo`) with `#[serde(skip)]` field `inline` on `AcceptedFindings` (`src/accepted_findings.rs:37`); the struct literal `Ok(AcceptedFindings { accepted })` at `:100` becomes `..Default::default()`. Extend `run_checker_against_source` (`src/check.rs:549`, returns `anyhow::Result<(String, bool)>` today) and `run_checker_against_file` (:576) with a `&InlineIgnoreContext` argument and a `severity` argument (placeholder passed through; real threading is Task 1.2.2b2); call `apply_inline_ignores` between `run_checker_configured` and the text flatten. The return becomes `SourceCheck { combined, passed, findings, inline }` (glossary; 4 callers), where `findings` is the kept `Vec<Finding>` (the structured data path; nothing downstream parses text).
- In `run_native_check` (:422): populate `CheckResult.inline.dropped` from `SourceCheck.inline`; **after** the diff-scoping block (:473) and `drop_accepted_findings` (:482) compute the **shown** findings as those in `SourceCheck.findings` whose `format!("{}:{}: {}", file, f.line, f.message)` is a line of the final `combined`, and fill `inline.shown` (rule via `anchor_rule`, line from the structured finding, capped at 20) from them, and `inline.kept` from `SourceCheck.findings` directly; `first_anchor()` and `rule_ids()` read it. A finding that was scoped out or accepted-dropped therefore never supplies an anchor. Pass `&accepted.inline` from `run_native_check` (call to `run_checker_against_file` at :457); `check_native_against_git_head` (:594; call at :623) passes the run's mode with the counter stripped (`ctx.without_counter()`).
- Tests for the Story 1.2.2 AC bullets (covered finding disappears, all covered passes, survives edits, no-prefix finding, stale table, `Disabled`, no-scan on empty), plus: (a) with `changed_lines = Some(&[(5,5)])`, a finding at line 20 is scoped out and `inline.shown` does not contain it, while a finding at line 5 is present; (b) a finding removed by an `accepted/` entry never appears in `inline.shown`; (c) `SourceCheck.findings` equals the kept findings and `Disabled` returns every raw finding. Follow the existing test style in `check.rs`.
- Files: `src/check.rs`, `src/accepted_findings.rs`

##### Task 1.2.2b1c: Cache-hit behavior for `InlineOutcome` and binary-version stamp (~1.5h)
- Problem (round-2 review): with `#[serde(skip_serializing)]` a cache hit returns a `CheckResult` whose `inline` is empty, so the footer anchor hint (Story 3.1.1) and the never-cut `[blocking-suppressed]` data vanish silently. The cache is real on the hot path: `handle_run_checks` returns cached results when `changed_lines.is_none()` (`src/daemon.rs:155-165`), keyed on file, config and registry stamps and the trigger (`src/cache.rs:84-103`).
- Decision: **serialize `InlineOutcome`** (`#[serde(default)]`, drop `skip_serializing`). Rejected: (a) bypass the cache when the file contains `kibitzer`: the anchor hint matters most for files that have no directive yet, so the bypass would not cover the main case, and it adds a content read to the cache fast path; (b) recompute on hit: needs the rule/finding structure the cache no longer has, i.e. a full rerun, which is what the cache exists to avoid. Serializing is safe because a cache entry is keyed on the file stamp, so a hit means the file is unchanged and the stored outcome is still exact; the cost is a few hundred bytes only on results that have a failing finding. An entry written by an older binary has no `inline` and deserializes to the empty outcome, so the hint degrades to the generic form (no first-finding row) until the next edit refreshes the entry; with the version stamp such an entry is discarded on load instead.
- `[blocking-suppressed]` (Story 3.1.2) needs `changed_lines = Some`, a path that never reads the cache, so it is unaffected on the hook path; the serialized `dropped` list matters only for the batch trigger and for consistency.
- Binary-version staleness (round-3 engineering gap 7; Design Decision 13): `Cache` gains `#[serde(default)] kibitzer_version: String` (`src/cache.rs`, struct at :60); `load` returns an empty cache when the stored value differs from `env!("CARGO_PKG_VERSION")`, `save` stamps the current version. Without this, a result cached by a pre-feature binary keeps serving unsuppressed findings after an upgrade until the file's stamp changes, because `get` (`src/cache.rs:84-103`) checks only file, config and registry stamps and the trigger. Documented limit: same-version dev builds share a cache.
- Tests: `cache.rs` round trip of a `CheckResult` with a populated `inline` (`first_anchor`, one `DroppedFinding`) through `put`/`get`/`save`/`load`; a `cache.json` string without the `inline` key deserializes with the empty outcome (same style as `src/check.rs:3029`); a `cache.json` written with `kibitzer_version` "0.0.0" (and one with no version key) loads as an empty cache, and one stamped with the current version loads intact; a `daemon.rs` test that a cache hit returns the first-finding anchor (run once, hit second time, compare `inline.first_anchor`).
- Files: `src/check.rs`, `src/cache.rs`, `src/daemon.rs` (test)

##### Task 1.2.2b2: Thread `Severity` (~1h)
- Pass the check's real `Severity` (known in `run_native_check`) down to `run_checker_against_source` and on to `apply_inline_ignores`, replacing the placeholder from 1.2.2b1b; `check_native_against_git_head` passes it too. Tests: the counter AC (`total == 2`, `blocking == 1`, baseline replay leaves the counter unchanged, two concurrent contexts do not share counts) and `DroppedFinding.severity` for a blocking check.
- Files: `src/check.rs`

##### Task 1.2.2b3: Non-UTF-8 and unreadable-file guard (~0.5h; runs after Task 2.1.1a because its regression test drives the registered `inline-ignore` checker)
- In `run_checker_against_file`, when `checker_name == "inline-ignore"` a read error or non-UTF-8 content returns an empty passing result instead of the failed-`CheckResult` path (:456-470). Other checkers keep their current read-error behavior.
- Regression tests: a PNG-like binary file (bytes `\x89PNG\r\n\x1a\n\xff\xfe`) in a temp dir, `run_native_check` for `inline-ignore` yields a passing, empty result; `kibitzer run` over a dir containing that file prints no `inline-ignore` output; an existing checker's read-error behavior is unchanged.
- Files: `src/check.rs`

### Epic 1.3: Ignore comments are invisible to comment-quality
**Goal**: The fix an agent adds never triggers a new finding.

#### Story 1.3.1: Exempt `kibitzer:` comments from comment and prose checks
**As an** agent, **I want** my ignore comment not to produce a new finding, **so that** I do not loop.
**Acceptance Criteria**:
- No `commented-out-code` / `verbose-comment` on a directive comment.
  - *Given* Go `// kibitzer:ignore flag-argument -- see foo(bar) and x = y` , *When* `comment-quality-go` runs, *Then* zero findings.
- The directive does not inflate `over-commented` counts.
  - *Given* a function with exactly the comment-line count at the `over-commented` threshold minus one plus one `kibitzer:` comment, *When* run, *Then* no `[over-commented]`.
- Markdown prose checks ignore the HTML comment.
  - *Given* a `.md` paragraph plus `<!-- kibitzer:ignore a -- b -->` line, *When* `repetitive-sentence-structure` and `missing-paragraph-break` run, *Then* findings are identical to the file without the comment.
- Other line- and comment-reading checkers are not perturbed.
  - *Given* an otherwise unchanged Go file with and without an added valid ignore comment, *When* `duplicate-code`, `syntax-rules-go`, and `em-dash-overuse` run, *Then* their findings are identical apart from line shifts (a regression test; `file-size` legitimately counts the added line).
**Files**: `src/checkers/comment_quality.rs`, `src/markdown_text.rs`

##### Task 1.3.1a: Skip in comment-quality (~1h)
- Add `fn is_directive_comment(text) -> bool` (stripped text starts with `kibitzer:`) in `inline_ignores.rs` (`comment_quality` depends on it; `inline_ignores` no longer depends on `comment_quality` since `comment_kinds` moved to `tree_walk.rs`); skip such nodes in the `check` loop (`comment_quality.rs:~233-244`), `collect_comment_rows`/`leading_comment_rows` used by `check_proportionality` (`:643-700`). Tests per the three comment-quality ACs.
- Files: `src/checkers/comment_quality.rs`, `src/inline_ignores.rs`

##### Task 1.3.1b: Markdown prose exemption (~0.5h)
- Read how `src/markdown_text.rs` handles HTML; strip directive HTML comments if it does not. Add a before/after-equality test.
- Files: `src/markdown_text.rs`, tests

---

## Phase 2: Diagnostics and guardrails

### Epic 2.1: The `inline-ignore` checker
**Goal**: Malformed ignores are loud, self-repairing findings.

#### Story 2.1.1: Malformed and unknown-rule findings
**As an** agent, **I want** a malformed ignore to tell me exactly what to write, **so that** I fix it in one edit.
**Acceptance Criteria**:
- Missing reason.
  - *Given* `// kibitzer:ignore flag-argument` at row 8, *When* `inline-ignore` runs, *Then* `8: [ignore-syntax] kibitzer:ignore flag-argument has no reason. Write: kibitzer:ignore flag-argument -- <why this is acceptable>`.
- Missing rule: `// kibitzer:ignore -- why` yields `[ignore-syntax] kibitzer:ignore needs a rule id. Write: kibitzer:ignore <rule> -- <why>`.
- Unknown rule with suggestion (advisory; reported at the directive row only when the directive neither names a known rule nor a registered checker, and it does not stop the directive from suppressing a finding whose rule it equals).
  - *Given* `// kibitzer:ignore flag-arg -- x`, *When* run, *Then* `[ignore-syntax] unknown rule 'flag-arg' - did you mean 'flag-argument'?` (ASCII hyphen; no em dash).
- Em dash separator: `// kibitzer:ignore flag-argument — legacy API` (U+2014 or U+2013) yields `[ignore-syntax] use ASCII '--' (two hyphens) between the rule and the reason, not an em dash. Write: kibitzer:ignore flag-argument -- <why>`; the message is ASCII only and is not the missing-reason text.
- Bad rule list: `// kibitzer:ignore a, b -- legacy API` (and `a,,b`, `a,`) yields `[ignore-syntax] rule list must be comma-separated with no spaces. Write: kibitzer:ignore a,b -- <why>`.
- Unknown-rule semantics are pinned (all advisory, never an error, never blocking).
  - *Severity*: `inline-ignore` is `Severity::Advisory`, so `[ignore-syntax]` never exits 2 and never turns a passing hook into a block, even in a file whose other checks are blocking.
  - *Unknown rule with a near match* (edit distance <= 2 to a `KNOWN_RULES` id or registered checker name): `[ignore-syntax] unknown rule 'flag-arg' - did you mean 'flag-argument'?`.
  - *Unknown rule with no near match*: no `[ignore-syntax]` finding. A rule naming something that cannot match anything (`kibitzer:ignore made-up-rule -- legacy API`) is surfaced by `[unused-ignore]` (Story 2.2.3 when just added in the hook; **Task 2.2.2e in `kibitzer run`, which ships even though the full run-path audit, Task 2.2.2b, is deferred**) with the not-a-known-rule wording `[unused-ignore] 'made-up-rule' is not a known rule or checker; run 'kibitzer check list' to see valid names`, not the "remove it" wording (the intent behind a typo is usually valid, so deleting it is the wrong repair), and not by a possibly stale-table guess.
  - *Suppression is independent of the table*: a directive naming a rule the table does not know still suppresses a finding whose own rule equals it (Story 1.2.2 stale-table AC).
  - *Comma list*: each unknown element is reported separately; known elements in the same list still suppress. *Given* `// kibitzer:ignore flag-argument,flag-arg -- legacy API` above a `[flag-argument]` finding, *Then* the finding is dropped and one `[ignore-syntax] unknown rule 'flag-arg' - did you mean 'flag-argument'?` is emitted.
- Near miss: `// kibitzer: ignore foo -- bar` yields `[ignore-syntax] 'kibitzer: ignore' not recognized; use 'kibitzer:ignore'`.
- Directive not at the start of the comment: `// TODO kibitzer:ignore flag-argument -- legacy API, callers pinned` at row 8 yields `8: [ignore-syntax] kibitzer:ignore must start the comment; it was found after other text and suppresses nothing. Write it as its own comment: kibitzer:ignore flag-argument -- <why>` (rule echoed from the parse; ASCII only; the original finding stays visible).
- Weak reasons, two messages (Task 1.1.1d): `-- needed` yields `reason 'needed' is too short to explain the code. Write: kibitzer:ignore flag-argument -- <the concrete constraint that makes this code acceptable>`; `-- flag-argument` yields `reason repeats the rule id instead of saying why the code is acceptable. Write: ...` (same repair form).
- Volume and blocking advisories address the agent and name the person to tell (Story 2.1.2, Story 3.1.2).
- The original finding stays visible alongside it.
  - *Given* a malformed ignore above a real `[flag-argument]` finding, *When* `kibitzer run` on the file, *Then* both lines print.
- Known-rule table cannot silently drift.
  - *Given* every static `"[<id>]` message prefix literal in `src/checkers/*.rs` and top-level checker modules (e.g. `src/single_call_site_delegation.rs`) outside `#[cfg(test)]`, *When* the drift-guard test runs, *Then* each id is in `KNOWN_RULES` or is a registered checker name. Dynamic prefixes (`"[{id}]"`, `"[{ref_id}]"`, any `[{`) are ignored by the guard, since they are data not rule ids.
- Non-UTF-8 and unreadable files yield nothing.
  - *Given* a binary file (PNG header bytes) and an invalid-UTF-8 `.go` file in a walked directory, *When* `kibitzer run <dir>` runs, *Then* no `inline-ignore` failure or output line appears for either, exit status is unaffected, and no other checker's read-error behavior changed.
- Scope is narrow.
  - *Given* `default_checks()`, *When* the `inline-ignore` entry is read, *Then* its globs are the `Language::ALL` extensions plus `*.md`, not `**/*`.
**Files**: `src/checkers/inline_ignore.rs`, `src/checkers/mod.rs`, `src/config.rs`, `src/check.rs`, `src/inline_ignores.rs`

##### Task 2.1.1a: Checker skeleton and rule table (~1.5h)
- New `src/checkers/inline_ignore.rs` implementing `Checker` (`name() == "inline-ignore"`, `language() == None`, globs = `Language::ALL` extensions plus `*.md`, advisory), registered like its siblings with `inventory::submit!`; `pub mod inline_ignore;` in `src/checkers/mod.rs`. The checker body works on the `source` it is handed; the non-UTF-8 guard is in `run_checker_against_file` (Task 1.2.2b3).
- In `inline_ignores.rs`: `KNOWN_RULES` (comment-quality ids, syntax-rules ids from `src/checkers/rules.rs`, `file-size`, etc.), `known_rule(rule) -> bool` = `KNOWN_RULES` plus `crate::checker::registry()` names (`src/checker.rs:178`; build the registry name set once per scan, not per call); small in-file Levenshtein for `did_you_mean`. Used only for unknown-rule/suggestion reporting, never to decide suppression. Optional later refinement (not in scope): a `Checker::rule_ids()` method so the table derives from the registry.
- Files: `src/checkers/inline_ignore.rs`, `src/checkers/mod.rs`, `src/inline_ignores.rs`

##### Task 2.1.1b: Messages (~0.5h)
- Render each `MalformedReason` per the ACs, including the dedicated `EmDashSeparator` and `BadRuleList` messages and the near-match-only `UnknownRule` rule; tests for each, including that no message contains an em dash and that `EmDashSeparator` never renders the missing-reason text.
- Files: `src/checkers/inline_ignore.rs`

##### Task 2.1.1c: Default-on and drift guard (~1h)
- Make `Language::extensions` (`src/checker.rs:82`, currently private) `pub(crate)` so `config.rs` can build the glob list (or add a `Language::all_globs()` helper in `checker.rs`). Add `native_check("inline-ignore", Severity::Advisory, <Language::ALL extension globs + "**/*.md">)` to `config::default_checks()` (pattern at `src/config.rs:~782`); add the drift-guard test described above and the binary-file test (Task 1.2.2b3); update the config default-catalog test if one enumerates names. Docs note that `kibitzer check native <name> <file>` (`src/main.rs:384`) runs a checker directly and so bypasses inline ignores (intended for the corpus workflow).
- `KNOWN_RULES` staleness mitigation: it is advisory only (never gates suppression), so drift costs at most a spurious or missing "did you mean"; the drift-guard test is the backstop, and the unknown-rule finding is emitted only when a near match (edit distance <= 2) exists (pre-mortem #3).
- Files: `src/config.rs`, `src/checker.rs`, `src/inline_ignores.rs`

#### Story 2.1.2: Volume guardrail (not cuttable, see Scope cut order)
**As a** maintainer, **I want** heavy ignore use in one file to be visible, **so that** blanket silencing shows up.
**Acceptance Criteria**:
- `[ignore-volume]` fires at 5 or more valid directives in a file, anchored at the row of the 5th directive so it is inside the changed lines when the agent adds that directive (a line-1 anchor would be dropped by diff-scoping, `src/check.rs:473`).
  - *Given* a Go file with 5 valid directives, the 5th at line 40, *When* `inline-ignore` runs, *Then* one finding at line 40: `[ignore-volume] 5 inline ignores in this file; tell the user you are silencing this many checks here, and either fix the code or ask the user whether a check is wrong`.
  - Emitted **once per file**, at the 5th directive's row only; a 6th, 7th, ... directive adds no further finding (no repeat noise on every added directive). Known limit: a 6th directive added in a later edit is not re-flagged in the hook (the 5th's row is outside `changed_lines`); the `kibitzer run` footer's repo-level counts and the `[blocking-suppressed]` advisory remain the visibility for that case.
  - *Given* the same file run with `changed_lines = Some(&[(40,40)])`, *Then* the finding survives scoping.
- 4 directives produce no volume finding.
  - *Given* 4 valid directives, *When* run, *Then* zero `[ignore-volume]`.
**Files**: `src/checkers/inline_ignore.rs`

##### Task 2.1.2a: Volume finding (~0.5h)
- `const IGNORE_VOLUME_THRESHOLD: usize = 5;` emit one finding from the checker at the 5th directive's row only (not at each later one) with the AC message and tests, including a 7-directive file yielding exactly one `[ignore-volume]`. Update ADR-002 item (3) to state the anchor.
- Files: `src/checkers/inline_ignore.rs`

### Epic 2.2: Raw mode and unused-ignore reporting
**Goal**: Maintainers see raw findings; stale ignores are reported without hook noise.

##### Task 2.2.0a: Hook post-pass module skeleton (~1h; round-3 engineering gap 6)
- Create `src/inline_post_pass.rs` (`mod` in `src/main.rs`) with one entry point `pub(crate) fn run(ctx: PostPassInput) -> Vec<CheckResult>`, where `PostPassInput { checks, repo_root, file_path, changed_lines: Option<&[(usize, usize)]>, results: &[CheckResult], accepted: &AcceptedFindings, registry }`, and call it exactly once in `run_checks_for_trigger` (`src/check.rs:1453`) after the per-check loop (`results.extend(post_pass::run(..))`). The module reads the file once (only when `changed_lines` is `Some` and the substring `kibitzer` is present), takes directives from the shared `ScanMemo`, and returns synthesized `inline-ignore` results (advisory severity). Stories 2.2.3 and 3.1.2 and Task 2.2.3c add their logic here, not in `check.rs`. Skeleton returns an empty vec; test: zero results when `changed_lines` is `None` or the file lacks the substring.
- Files: `src/inline_post_pass.rs` (new), `src/main.rs`, `src/check.rs`


#### Story 2.2.1: `--no-inline-ignores` and suppressed-count footer
**As a** maintainer running backtests, **I want** raw checker output, **so that** ignores in the corpus do not hide the false-positive rate.
**Acceptance Criteria**:
- `kibitzer run --no-inline-ignores` prints covered findings.
  - *Given* a file with a covered `[flag-argument]` finding, *When* `kibitzer run --no-inline-ignores <file>`, *Then* the finding line prints; without the flag it does not.
- Footer reports the count, per run, repo-wide, with the blocking share. (The `false-positive` split was cut, see Scope disposition.)
  - *Given* 3 covered findings in a normal run, 1 of them from a `Severity::Blocking` check, *When* `kibitzer run`, *Then* the last line is `[kibitzer] 3 findings suppressed inline (1 from blocking checks) (rerun with --no-inline-ignores to see them)`; with 0 suppressed no footer line; the blocking parenthetical is omitted when its count is zero. The counts span all files of the run, so 1-4 ignores per file across many files still add up. HEAD-baseline replays are not counted, and two runs in one process (or parallel tests) do not share a count.
- `--no-inline-ignores` also governs the HEAD baseline.
  - *Given* a blocking finding covered by an ignore that exists unchanged at HEAD, *When* `kibitzer run --no-inline-ignores`, *Then* the finding is reported as pre-existing, not new (current and HEAD are both raw).
- `kibitzer run` teaches the syntax in one line (round-3 UX gap 4: developers otherwise find it only in the docs).
  - *Given* a `kibitzer run` that reports at least one finding, *When* it finishes, *Then* it prints once, before the suppressed-count footer, `[kibitzer] to dismiss a finding you judged acceptable: <comment> kibitzer:ignore <rule> -- <why> (docs/suppressing-checks.md)`; with zero findings the line is absent.
- `kibitzer check backtest` is unaffected.
  - *Given* `src/backtest.rs` calls `run_checker_with_cache` directly, *When* a backtest runs over a transcript with ignores, *Then* findings are raw (asserted by a test calling the backtest path on a covered fixture).
**Files**: `src/run.rs`, `src/main.rs`, `src/backtest.rs` (read)

##### Task 2.2.1a: Flag plumbing (~0.5h)
- Add `--no-inline-ignores` to the `Run` subcommand args in `src/main.rs`; in `src/run.rs:129` set `accepted.inline.mode = Disabled` when passed, and always create `accepted.inline.counter = Some(Arc::new(SuppressionCounts::default()))` per run. The baseline path inherits the mode via Task 1.2.2b1b.
- Files: `src/main.rs`, `src/run.rs`

##### Task 2.2.1b: Footer (~0.5h)
- After the report lines print the footer (total and blocking count per the AC) when the run's own `total` is `> 0` (no global to reset). Verify `backtest.rs` really bypasses the filter and add the test.
- Files: `src/run.rs`, `src/backtest.rs`

##### Task 2.2.1c: `kibitzer run` syntax hint (~0.5h)
- Print the one-line hint from the Story 2.2.1 AC once per run that reports at least one finding, in `src/run.rs` next to the footer. Test: a run with a finding ends with the hint line; a clean run does not print it; the hint contains `kibitzer:ignore <rule> -- <why>`.
- Files: `src/run.rs`

#### Story 2.2.2: Unused-ignore audit in `kibitzer run` (DEFERRED to a follow-up PR except Tasks 2.2.2a and 2.2.2e; the hook variant is Story 2.2.3, which is core)
**As a** maintainer, **I want** ignores that suppress nothing reported, **so that** fixed code does not leave dead ignores.
**Acceptance Criteria**:
- Unused ignore reported by `kibitzer run`.
  - *Given* `// kibitzer:ignore flag-argument -- legacy` above a line with no `[flag-argument]` finding and `flag-argument`'s checker enabled, *When* `kibitzer run <file>`, *Then* `<file>:<row>: [unused-ignore] kibitzer:ignore flag-argument suppresses nothing - remove it`.
- Used ignore not reported.
  - *Given* the same ignore above a line that does fire `[flag-argument]`, *When* `kibitzer run`, *Then* no `[unused-ignore]`.
- A rule that is neither a `KNOWN_RULES` id nor a registered checker name gets the not-a-known-rule wording, not "remove it".
  - *Given* `// kibitzer:ignore made-up-rule -- legacy API` with no near match, *When* `kibitzer run <file>`, *Then* `<file>:<row>: [unused-ignore] 'made-up-rule' is not a known rule or checker; run 'kibitzer check list' to see valid names`.
- Ignore shadowed by `accepted/` is not unused.
  - *Given* a covering ignore plus a matching `.kibitzer/accepted/` entry for the same finding, *When* `kibitzer run`, *Then* no `[unused-ignore]` (judged against raw findings).
- Hook diff-scoped mode never emits it for directives outside the changed lines (Story 2.2.3 covers the ones inside, and its Task 2.2.3b owns the negative test).
  - *Given* `changed_lines = Some(&[(1,3)])` and an unused ignore at row 50, *When* `run_checks_for_trigger`, *Then* no `[unused-ignore]` in any result.
- Ignores whose rule's checker is disabled are not judged.
  - *Given* an ignore for `em-dash-overuse` with that check disabled in `.kibitzer/inspect.json`, *When* `kibitzer run`, *Then* no `[unused-ignore]`.
- Ignores whose checker did not run on this file are not judged.
  - *Given* a file over `MAX_NATIVE_CHECK_BYTES` (`src/check.rs:572`) or a check excluded by trigger, *When* `kibitzer run`, *Then* no `[unused-ignore]` for rules that checker owns. The "ran" set comes from the first (applied) pass, not from config alone.
- `markdown-link-integrity` ignores are judged by checker name.
  - *Given* a used `markdown-link-integrity` ignore (reference-style finding), *When* `kibitzer run`, *Then* no `[unused-ignore]`.
**Files**: `src/inline_ignores.rs`, `src/run.rs`

##### Task 2.2.2a: `unused_ignores` pure function (shared by Stories 2.2.2 and 2.2.3; stays in the committed slice) (~1.5h)
- Pure `unused_ignores(directives: &[Directive], raw: &[RawFinding], ran_checkers: &[&str], only_rows: Option<&[(usize, usize)]>) -> Vec<Finding>`. `RawFinding { line: Line, checker, rule: RuleId, message }` (glossary) is built from structured findings by `raw_findings_for_check` (Task 2.2.3a); the function never sees or parses rendered text. Rule-to-checker ownership by `KNOWN_RULES` prefix plus exact checker names; rules with unknown ownership are not judged for "matches nothing" by this function (fail open here), but a rule that fails `known_rule` (neither a `KNOWN_RULES` id nor a registered checker name) is reported with the not-a-known-rule message pointing at `kibitzer check list`. The hook path covers unowned rules with the first-pass fallback in Task 2.2.3c, and `kibitzer run` covers them with Task 2.2.2e. Unit tests per AC, including `only_rows` filtering.
- Files: `src/inline_ignores.rs`

##### Task 2.2.2b: Run-path wiring (DEFERRED, follow-up PR) (~2h)
- In `src/run.rs` per file with a `kibitzer` marker: `ran_checkers` is the set of `CheckResult.check_name` values from the first pass's `Vec<CheckResult>` (so a check skipped by size, trigger, or config never appears and its rules are not judged). Then call `raw_findings_for_check` (Task 2.2.3a) for exactly those checks: it returns structured `RawFinding`s from a `Disabled` context and never goes through `run_native_check`, so neither diff-scoping nor `drop_accepted_findings` (`src/check.rs:482-489`) can hide a finding that `accepted/` shadows, which keeps the "shadowed by `accepted/` is not unused" AC true without constructing a second `AcceptedFindings`. Judge with `unused_ignores` (same `rule_matches`/`covers` as the apply path), emit `[unused-ignore]` lines under check name `inline-ignore`. Only for `kibitzer run` (`changed_lines` is `None`, `src/run.rs:147`).
- Test: a file with a covering ignore plus a matching `accepted/` entry yields no `[unused-ignore]`.
- Files: `src/run.rs`

##### Task 2.2.2e: Unknown-rule audit in `kibitzer run`, first-pass data only (~1h; round-3 UX gap 5)
- Why: Story 2.1.1 deliberately emits no `[ignore-syntax]` for an unknown rule with no near match (a stale `KNOWN_RULES` must not produce false alarms), and Task 2.2.2b, which would have reported it in `run`, is deferred. Without this task the CLI is silent on a typo like `made-up-rule`.
- Reuse the Task 2.2.3c first-pass judgement from `src/inline_post_pass.rs` unscoped: in `src/run.rs`, for a file with the `kibitzer` marker, take directives from the shared `ScanMemo`, collect `inline.dropped` from the run's `CheckResult`s, and emit `[unused-ignore] 'x' is not a known rule or checker; run 'kibitzer check list' to see valid names` for each directive whose rule fails `known_rule`, is not in any `dropped` entry (so a legitimate built id that did match stays silent), and equals no `ran_checkers` name. No rerun, no extra parse.
- Test (`tests/inline_ignore_cli.rs`): `kibitzer run` on a file with `// kibitzer:ignore made-up-rule -- legacy API` prints the line; a directive for a rule absent from `KNOWN_RULES` that really suppresses a finding prints nothing.
- Files: `src/run.rs`, `src/inline_post_pass.rs`

#### Story 2.2.3: Hook advisory for an added directive that matches nothing (core, not cuttable: the P1 wrong-row correction loop)
**As an** agent, **I want** to be told right after the edit when the directive I just added is on the wrong row, **so that** I fix the placement instead of piling on more ignores.
**Acceptance Criteria**:
- A directive inside `changed_lines` that covers no raw finding yields an advisory with the exact row.
  - *Given* a Go file where the agent adds `// kibitzer:ignore flag-argument -- legacy API, callers pinned` at row 12 while the `[flag-argument]` finding is at row 20, `changed_lines = Some(&[(12,12)])`, *When* the PostToolUse hook runs, *Then* the output contains `12: [unused-ignore] kibitzer:ignore flag-argument matches no finding at line 12 or 13; the finding is at line 20. Move the comment to the line directly above line 20 (or the end of line 20)` (the nearest raw finding for that rule, when one exists; otherwise `suppresses nothing - remove it`, except for a rule that is not a known rule or checker, which gets `'made-up-rule' is not a known rule or checker; run 'kibitzer check list' to see valid names`). "Move" is deliberate and absolute ("directly above line 20", not "line 19"): row numbers shift on insertion, and an agent that already inserted at row 12 must not read the hint as "add another".
- A directive that does match is silent.
  - *Given* the same edit with the comment at row 19, *Then* no `[unused-ignore]`.
- Directives outside `changed_lines` are never judged.
  - *Given* an unused ignore at row 50 and `changed_lines = Some(&[(1,3)])`, *Then* nothing.
- A rule with no `KNOWN_RULES` owner is still judged, from first-pass data (Task 2.2.3c).
  - *Given* a directive for an unowned rule at row 12 and a surviving finding of that rule at row 20 with `changed_lines = Some(&[(12,12)])`, *Then* `[unused-ignore]` naming row 20; *Given* an unowned rule with no surviving finding and no `ran_checkers` name match, *Then* `[unused-ignore] 'x' is not a known rule or checker; run 'kibitzer check list' to see valid names` (not silent, not "remove it": an unowned rule fails `known_rule`, so this is the typo case; a legitimate unowned rule that was used appears in `dropped` and stays silent).
- The raw rerun is not diff-scoped (round-3 engineering gap 2): the nearest raw finding is found even when it lies outside `changed_lines`.
  - *Given* the first AC's setup (directive at row 12, `changed_lines = Some(&[(12,12)])`, finding at row 20), *When* the post-pass runs, *Then* the advisory names row 20, which `scope_output_to_changed_lines` (`src/check.rs:473`) would have removed from any text-level result. Asserted by a test that also asserts the first-pass `combined` for the same call does not contain the row-20 finding.
- Only owning checkers that ran are judged (same `ran_checkers` rule as Story 2.2.2); the raw rerun happens only when a directive row lies inside `changed_lines`.
  - *Given* an edit that touches no directive row, *Then* no raw rerun is performed (asserted via a rerun counter).
**Files**: `src/inline_post_pass.rs`, `src/check.rs`, `src/inline_ignores.rs`

##### Task 2.2.3a: Hook-scoped unused judgement (~5h, re-estimated from 3.5h: raw-rerun orchestration, `ran_checkers`, and the structured raw path)
- In `src/check.rs` add `pub(crate) fn raw_findings_for_check(check: &Check, file_path: &Path, source: &str) -> anyhow::Result<Vec<RawFinding>>`: it calls `run_checker_against_source` with `InlineIgnoreContext` in `Disabled` mode and no counter and returns `SourceCheck.findings` mapped to `RawFinding` (rule via `anchor_rule`). It **does not call `run_native_check`**, so the `changed_lines` scoping at `src/check.rs:473`, the `accepted/` drop at `:482`, and the HEAD baseline never apply: this is the "changed_lines = None" behavior, achieved by construction rather than by passing `None`. No text is parsed anywhere on this path (round-3 engineering gaps 1 and 2).
- In `src/inline_post_pass.rs`: when `changed_lines` is `Some` and a directive from the shared `ScanMemo` has rows intersecting it, call `raw_findings_for_check` for the checks that own the directive's rules (owner by Task 2.2.2a; only checks present in the first pass's `CheckResult.check_name` set), pass the result to `unused_ignores(.., only_rows = changed_lines)`, add `nearest_finding_line` for the hint, and return the advisory as an `inline-ignore` result. Skipped entirely when no directive row is in `changed_lines`. This task also provides the changed-lines plumbing in the post-pass that Task 3.1.2a reuses (3.1.2a depends on Task 2.2.0a, not on this task).
- Tests: the Story 2.2.3 ACs, the unscoped-raw-path assertion (row 20 found although `changed_lines = (12,12)`), rerun counter zero when no directive row changed.
- Files: `src/check.rs`, `src/inline_post_pass.rs`, `src/inline_ignores.rs`

##### Task 2.2.3d: Hook-latency measurement spike (~1h; runs right after 2.2.3a, before 2.2.3c and 2.2.3b build further)
- Why here: the 150 ms and 2x budgets are INFERRED and were first measured at Task 4.1.1c, after all of Story 2.2.3 was built; a miss would force rework of a core story late. Measure now, while only 2.2.3a exists.
- Measure: this repo's largest source file with a marker placed inside `changed_lines`; hook-path raw rerun wall time, median of 5, absolute ms and ratio to the same owning checkers' first-pass time. Record both in the task notes (they are copied into the PR body at Task 4.1.1c).
- Decision rule: **pass** if added time is at most 150 ms and the ratio is at most 2x: continue to 2.2.3c and 2.2.3b unchanged. **Miss**: (1) narrow the rerun to the owning checkers whose first pass returned a non-empty or relevant result and re-measure once; (2) if still over, drop the raw rerun for the hook path entirely and judge from first-pass data (`dropped` plus the surviving findings), i.e. the Task 2.2.3c fallback becomes the only mode, and the advisory's nearest-row hint is given only when a surviving finding of the same rule exists (otherwise `suppresses nothing - remove it or move it`). Record which tier shipped in the PR. Time-box the spike to 1h; do not optimize the checkers themselves.
- **Spike result (release build, macOS Darwin 25.6, throwaway `#[ignore]` test, not committed; median of 5 after one discarded cold run; marker is `kibitzer:ignore long-function` inserted at row 5, `changed_lines = (5,5)`, owning check = `syntax-rules-rust`)**: `src/checkers/rules.rs` (155,748 bytes, largest source file by size): first pass of the owner 35.3 ms, raw rerun 26.8 ms (ratio 0.76), whole post-pass 26.9 ms added; `src/check.rs` (139,062 bytes, largest by lines): first pass 41.6 ms, rerun 29.5 ms (ratio 0.71), post-pass 25.9 ms. Whole default-check hook call with vs without the marker: 110.0 vs 86.2 ms and 147.5 vs 102.0 ms (the difference also includes the directive scan the first pass pays). Both files pass (at most 150 ms added, at most 2x): **tier shipped = full owning-check rerun** (no narrowing, no first-pass-only mode). Task 2.2.3c stays a fallback for unowned rules only, and Task 2.2.3b keeps the 2x ratio test.
- Files: none (measurement; a throwaway test or `cargo bench` style harness is fine and need not be committed)

##### Task 2.2.3c: Fallback for rules with no `KNOWN_RULES` owner (~1h)
- Problem (round-2 review): `unused_ignores` finds a directive's owning checker through `KNOWN_RULES` prefixes plus exact checker names (Task 2.2.2a) and treats unowned rules as "not judged (fail open)". A misplaced directive for a rule outside `KNOWN_RULES` (a checker outside `src/checkers/`, a plugin or built id, which Design Decision 10 deliberately keeps matchable) therefore gets no advisory, weakening the P1 wrong-row mitigation for exactly the rules the table does not cover.
- Fallback, no extra rerun: for a directive in `changed_lines` whose rule has no owner, use the first pass. (1) The directive is **used** if any `CheckResult.inline.dropped` entry has `directive_start` equal to its row (`dropped` is built before diff-scoping, `src/check.rs:473`, so it covers the whole file). (2) If unused, emit `[unused-ignore]` only when there is evidence the rule's checker ran: the rule equals a `ran_checkers` check name, or some kept finding in the first pass (`inline.kept`, whole file, so a finding outside `changed_lines` still counts) has that rule. Otherwise (rule fails `known_rule`, no evidence any checker owns it) emit the not-a-known-rule wording pointing at `kibitzer check list` instead of staying silent: the directive was just added in `changed_lines`, and an unused ignore on an unrecognized rule is almost always a typo (a checker skipped by size, trigger or config has an owned, known rule and is never judged here, so it cannot produce a false advisory). (3) The nearest-row hint comes from kept findings of that rule in the first pass (`inline.kept` carries rule and row, unscoped), if any.
- Documented limit: an unowned rule whose checker ran, found nothing anywhere in the file, and is not named by a `ran_checkers` check name cannot be told apart from a typo, so it gets the not-a-known-rule wording (which names `kibitzer check list`, so a real id outside `KNOWN_RULES` is quickly recognized); the same case for an owned rule is judged via the rerun.
- Logic lives in `src/inline_post_pass.rs` as `judge_unowned(directives, dropped, ran_checkers, kept, rows: Option<&[(usize, usize)]>)`; Task 2.2.2e reuses it with `rows = None`.
- Tests (`src/inline_post_pass.rs`): (a) unowned rule, directive on the wrong row, a kept finding of that rule elsewhere in the file, including one outside `changed_lines`: advisory with the nearest row; (b) unowned rule, directive on the right row (appears in `dropped`): silent; (c) unowned rule, no surviving finding and no matching `ran_checkers` name: the not-a-known-rule advisory pointing at `kibitzer check list` (the documented limit, pinned so it cannot change unnoticed; asserts the text contains `kibitzer check list` and not `remove it`); (d) unowned rule equal to a ran check's name, nothing matched: advisory `suppresses nothing - remove it` (the rule is a real checker name, so "remove it" is the right repair).
- Files: `src/inline_post_pass.rs`, `src/inline_ignores.rs`

##### Task 2.2.3b: Hook-path negative, rerun-counter, and latency-ratio tests (~1h; encode the Task 2.2.3d decision, so it is written after the spike)
- Negative test (moved from the former Task 2.2.2c): diff-scoped runs produce no `[unused-ignore]` for directives outside `changed_lines`. Rerun counter: an edit touching no directive row performs zero raw reruns. Latency: assert the raw rerun's wall time for a marker file is at most 2x the same owning checkers' first-pass wall time (median of 5 runs, relative, not an absolute number; see Non-functional budgets).
- Files: `src/check.rs`

---

## Phase 3: Surfaces and docs

### Epic 3.1: Teach the syntax once
**Goal**: Agents learn the syntax from the hook and MCP output without per-finding tokens.

#### Story 3.1.1: Footer and server instructions
**As an** agent, **I want** a copy-pasteable example in the hook footer, **so that** I need no doc fetch.
**Acceptance Criteria**:
- Hook advisory footer carries the syntax with the edited file's comment leader.
  - *Given* a failed check on `src/foo.go`, *When* the PostToolUse hook renders (`src/hook.rs:207-219`), *Then* the context ends with text containing `// kibitzer:ignore <rule> -- <why>` (and no `false-positive` marker, Amendment 1); for `notes.md` the leader is `<!-- ... -->`; for `x.py` it is `#`.
- The hint names the exact anchor row for the first **shown** finding, from structured data.
  - *Given* a failed `[flag-argument]` finding at `src/foo.go:20` (a `CheckResult` whose `inline.first_anchor() == Some((RuleId("flag-argument"), Line(20)))`), *When* the hook renders, *Then* the hint contains `above line 20` and the example `// kibitzer:ignore flag-argument -- <why>`, worded as "on its own line directly above the flagged line (or at its end)" ("directly above line 20", not "line 19": unambiguous when an agent has already inserted lines). The rule and line come from `InlineOutcome.shown`, never from parsing the rendered `file:line: [rule]` text. For a `FILE_SCOPE_RULES` finding the clause says `in the first 10 lines or on the anchor line`.
  - *Given* a finding at line 20 that is outside `changed_lines` (scoped out) or removed by an `accepted/` entry, plus a shown finding at line 5, *When* the hook renders, *Then* the anchor row is 5, never 20 (round-3 engineering gap 3: the anchor is computed after diff-scoping and `accepted/`, Task 1.2.2b1b).
  - *Given* a `markdown-link-integrity` finding, *Then* the hint names `markdown-link-integrity`, not the link ref id.
  - *Given* several failing results, *Then* the anchor comes from the first shown finding of the first failing result, and the hint is rendered once.
- Rule ids are listed, not inferred (round-3 UX gap 2). A finding with no `[rule]` prefix shows no id in its own line, and the finding line itself is **not** changed: its rendered form is the input to `accepted_findings::extract_rule` (`src/accepted_findings.rs:105`), to `scripts/backtest-triage.py`, and to the text-membership step of Task 1.2.2b1b, so prepending an id would change `accepted/` matching and every consumer. Instead the footer ends with `Rules: <id>, <id>` built from `InlineOutcome::rule_ids()` of every failing result (distinct, in order, at most 6 then `...`), where an unprefixed checker contributes its checker name via `anchor_rule`. This replaces the earlier fixed clause "the rule id is the [x] in the finding, or the checker name": the agent copies a listed id and needs no rule to recall.
  - *Given* two failing results, `flag-argument` (prefixed) then `primitive-obsession` (no prefix), *When* the hook renders, *Then* the footer contains `Rules: flag-argument, primitive-obsession`; *Given* a cached result from an older binary with an empty `inline`, *Then* the `Rules:` line is omitted and the generic form `<rule>` is used (no crash, no stale ids).
- Marker steering: none. Task 0.1.2 collapsed to one marker, so the clause is dropped (Amendment 1).
- Footer growth is bounded with a fixed drop order, and measured (round-3 UX gap 1). The footer is paid on every failing hook call and the feature exists to save tokens, so the budget is small: whole footer at most 640 characters (about 160 tokens), **net added over today's footer text at most 215 characters (about 54 tokens)**. Today's footer text is 423 characters (counted: `len()` of the string in `src/hook.rs:207-219` including both URLs; the earlier "about 480" was wrong). The compact syntax block measured 235 characters in the research draft: `Dismiss a judged finding: // kibitzer:ignore <rule> -- <why>, on its own line directly above the flagged line (above line 20 for the first), or kibitzer:false-positive if the checker is wrong. Rules: flag-argument, primitive-obsession.` Fitting it inside the 215-character net means trimming about 20 characters of the existing prose. The research target was about 45 tokens (180 characters); the extra 9 tokens carry the three clauses that close P1/P2 failures (anchor row, marker choice, rule ids), so the cap is not lowered further. No once-per-session decay: the hook is a one-shot process and decay needs a per-session state file; deferred, and revisited only if the Task 0.1.1 / 4.1.1c net-token check fails.
  - *Given* the pre-change footer length recorded in Task 3.1.1b, *When* the hook renders for the longest case (long path, long rule id, `.md` leader, 6 listed rules), *Then* the whole footer is at most 640 characters and the net added text at most 215. Drop order when over budget (first dropped first): (1) the `reporting-false-positives.md` link and its sentence; (2) trim the existing turn-off prose; (3) the `suppressing-checks.md` link; (4) the `Rules:` list is truncated, not dropped. The syntax line and the first-finding anchor are never dropped. The longest-case test asserts the anchor is present at the limit, and a second test with the budget artificially exceeded asserts the drop order.
  - Net-token check: Task 0.1.1 computes the break-even (`p*`) from replay data before build; Task 4.1.1c re-checks it with the final measured footer length.
- The footer's own example parses as a valid directive.
  - *Given* the rendered hint for each leader (`//`, `#`, `<!-- -->`) and the generic form, *When* its example is stripped of the leader, `<rule>`/`<why>` are replaced by a real rule id and a two-word reason, and the result is fed to `parse_comment_line`, *Then* it returns `Valid` with the expected kind (Task 3.1.1d).
- Existing links (subject to the drop order above) and "no findings emits nothing" stay.
  - *Given* a passing run, *When* the hook renders, *Then* no `additionalContext` is emitted (existing behavior preserved; the success ack that once made an exception was cut, see Scope disposition).
- Blocking stderr line mentions the syntax **and carries the repair text** (round-3 UX gap 3). Verified gap: the exit-2 path (`src/hook.rs:192-204`) prints only blocking results (`result.describe()`), and `inline-ignore` is advisory, so a malformed ignore on a blocking finding would have been invisible exactly when the agent is stuck.
  - *Given* a blocking failure, *When* hook exits 2, *Then* stderr includes `kibitzer:ignore`.
  - *Given* a blocking `markdown-link-integrity` finding in a file that also has a malformed `// kibitzer:ignore ...` directive (an `[ignore-syntax]` result), a misplaced added directive (`[unused-ignore]`), or a suppressed blocking finding (`[blocking-suppressed]`), *When* hook exits 2, *Then* stderr also contains each of those lines (every failing `inline-ignore` result is printed after the blocking results, whatever its severity), so the agent sees the repair in the same output. Test: `tests/hook_contract.rs`.
- MCP instructions and `run_checks` output.
  - *Given* the MCP `get_info`, *When* instructions are read (`src/mcp.rs:1429-1440`), *Then* they mention `kibitzer:ignore`; *Given* `run_checks` returns >0 findings, *Then* the last line is the one-line syntax with the file's leader; with 0 findings it is absent.
**Files**: `src/hook.rs`, `src/mcp.rs`, `src/inline_ignores.rs`, `src/check.rs`, `tests/hook_contract.rs`

##### Task 3.1.1a: `comment_leader(path)` and `syntax_hint(path)` (~0.5h)
- In `inline_ignores.rs`: leader by `Language::for_path` (`src/checker.rs:104`) / `.md` / fallback `#`; `syntax_hint(path, anchor: Option<(&RuleId, Line)>, rule_ids: &[&RuleId])` renders the generic form when `anchor` is `None` and the compact exact-row form plus the `Rules:` list otherwise, per the Story 3.1.1 AC; unit tests, including a prefix-less first finding and a second prefix-less finding after a prefixed one. (Rule-id visibility is folded into this task and 3.1.1b; its hours are inside their figures.)
- Files: `src/inline_ignores.rs`

##### Task 3.1.1b: Hook footer and blocking stderr (~2.5h)
- Replace the footer text at `src/hook.rs:207-219` to include `syntax_hint` for the edited file. The anchor is `failures.first().inline.first_anchor()` and the rule list is the union of `rule_ids()` over `failures` (structured `InlineOutcome.shown`, populated in Task 1.2.2b1b after scoping and `accepted/`); `hook.rs` does not parse `output`. First step: assert the current footer text length is 423 characters in a test (the previous "about 480" was an estimate), then apply the Story 3.1.1 budget (whole footer at most 640, net added at most 215) and drop order. In the exit-2 branch (`src/hook.rs:192-204`) also print every failing result whose `check_name == "inline-ignore"` after the blocking results, and keep the `kibitzer:ignore` mention in the stderr line. Update existing footer tests (`src/hook.rs:~245`). Tests: footer length in the longest case, the budget drop order, and a `tests/hook_contract.rs` case that an `[ignore-syntax]` result appears in exit-2 stderr.
- Files: `src/hook.rs`, `src/check.rs` (read), `tests/hook_contract.rs`

##### Task 3.1.1c: MCP (~1h)
- Add one sentence to `instructions` (`src/mcp.rs:1429`) and a trailing hint line in the `run_checks` renderer (`src/mcp.rs:~840-852`) when `finding_count > 0`; update `get_info_instructions_*` test (`:3055`).
- Files: `src/mcp.rs`

##### Task 3.1.1d: Footer example parses as a directive (~0.5h)
- Test `hook_footer_example_should_ParseAsValidDirective_When_PlaceholdersFilled` in `src/hook.rs` (or `inline_ignores.rs`): for each of Go, Python, Markdown, and Rust leaders, extract the example line from the rendered footer, fill `<rule>` and `<why>`, strip the leader, and assert `parse_comment_line` returns `Valid` of the right kind. This is the only automated check that the teaching surface teaches a form the parser accepts; adoption by real agents is measured separately (Task 4.1.1f).
- Files: `src/hook.rs`, `src/inline_ignores.rs`

#### Story 3.1.2: Hook advisory when an added directive suppresses a blocking finding
**As a** reviewer, **I want** the agent and the transcript to show each time a directive silences a blocking check, **so that** blocking checks cannot be quietly defeated (ADR-002).
**Acceptance Criteria**:
- Advisory fires for a directive inside `changed_lines` that suppressed a `Severity::Blocking` finding.
  - *Given* a `.md` edit adding `<!-- kibitzer:ignore markdown-link-integrity -- placeholder for later -->` at row 4 above a reference-style finding, `changed_lines = Some(&[(4,4)])`, *When* the PostToolUse hook runs, *Then* the hook output contains `4: [blocking-suppressed] markdown-link-integrity finding silenced inline (reason: placeholder for later); tell the user you silenced a blocking check and why, so they can confirm it`, as an advisory (does not set exit code 2). The text addresses the agent and names the person to tell ("the user"); "confirm this is intended" had no named confirmer and was a no-op instruction.
- Directives outside `changed_lines` and suppressions of advisory-severity findings emit nothing.
  - *Given* an ignore at row 50 outside `changed_lines`, or an `[flag-argument]` suppression (advisory), *Then* nothing.
- The advisory is not itself suppressible.
  - *Given* `kibitzer:ignore blocking-suppressed -- x y`, *Then* it stays (add `blocking-suppressed` to `META_RULES`).
- No success acknowledgement (DEFERRED, round-3 UX gap 7). The hook stays silent on a passing run; an agent that wants to confirm an ignore works runs `kibitzer run <file>`. The ack line (about 40 tokens per edit, 1h) was cut as beyond the ask; revisit if the 30-day sample shows agents re-adding directives that already work.
**Files**: `src/inline_post_pass.rs`, `src/inline_ignores.rs`

##### Task 3.1.2a: Surface blocking suppressions (~1.5h)
- Reads `CheckResult.inline.dropped` (the `DroppedFinding` list returned by `apply_inline_ignores` since Task 1.2.2a, filtered to `severity == Blocking`; no signature change). In `src/inline_post_pass.rs` (Task 2.2.0a; **not** `check.rs`), when `changed_lines` is `Some`, emit one `[blocking-suppressed]` advisory result (check name `inline-ignore`, severity Advisory) per such directive whose rows intersect `changed_lines`. `kibitzer run` already carries the repo-level count in its footer (Story 2.2.1). `META_RULES` gains `blocking-suppressed`. Depends on Tasks 1.2.2b2 and 2.2.0a only; it does not need Task 2.2.3a (the changed-lines plumbing lives in the post-pass module from 2.2.0a).
- Files: `src/inline_post_pass.rs`, `src/inline_ignores.rs`

### Epic 3.2: False-positive worklist (DROPPED 2026-10-07)
Dropped with the single-marker collapse (gate note `docs/backtest-triage/inline-ignore-gate.md`, Task 0.1.2: class (c) 0 of 4). Story 3.2.1 and Tasks 3.2.1a/3.2.1b (2.5h) are removed; there is no `kibitzer check false-positives list --inline`, and `list` is unchanged. Misfires keep flowing through `report_false_positive`.

### Epic 3.3: Docs and catalog
**Goal**: The reversed stance is documented consistently in one change.

#### Story 3.3.1: Update docs
**As a** reader, **I want** the docs to match behavior, **so that** I am not told "no inline suppression".
**Acceptance Criteria**:
- Old stance removed.
  - *Given* the repo after the change, *When* `rg "no inline|no inline/per-line|still no inline" docs/ CLAUDE.md`, *Then* zero matches.
- Syntax and anchor table documented.
  - *Given* `docs/suppressing-checks.md`, *When* read, *Then* it has a section with the grammar, both markers, same-or-above scope, the per-checker anchor table from Story 1.2.1, and the meta-rule exclusion.
- `accepted/` positioned as the fallback.
  - *Given* `docs/accepting-findings.md`, *When* read near the former line 80, *Then* it states inline ignores are the default and `accepted/` is for locations that cannot hold a comment.
- Docs state the hook's stale-ignore gap.
  - *Given* `docs/suppressing-checks.md`, *When* read, *Then* it says that in the PostToolUse hook an unused (stale) ignore is reported only when the directive is inside the lines just edited (Story 2.2.3), and that stale ignores elsewhere in a file are not reported by the hook until Story 2.2.2 (`kibitzer run` audit) ships; if 2.2.2 shipped in the same PR the sentence instead names `kibitzer run` as the audit.
- Examples in docs do not self-suppress.
  - *Given* the changed docs run through `kibitzer run docs/`, *When* checked, *Then* no `[unused-ignore]` and no `[ignore-syntax]` from the doc examples (they are in code fences, including the examples in the anchor table).
**Files**: `docs/suppressing-checks.md`, `docs/accepting-findings.md`, `docs/reporting-false-positives.md`, `CLAUDE.md`

##### Task 3.3.1a: Replace the stance (~1h)
- Edit `docs/suppressing-checks.md:11` and `docs/accepting-findings.md:80` and add the syntax/anchor section to the former (no new doc file).
- Files: `docs/suppressing-checks.md`, `docs/accepting-findings.md`

##### Task 3.3.1b: Cross-references and catalog (~0.5h)
- `docs/reporting-false-positives.md`: mention that a judged finding can be dismissed inline with `kibitzer:ignore` and that a checker misfire should still be reported through `report_false_positive` (no `list --inline`, Amendment 1). Repo `CLAUDE.md` "Default check catalog": add `inline-ignore`. Docs must not promise `[unused-ignore]` in `kibitzer run` unless Story 2.2.2 shipped in the same PR; if it is deferred, document only the hook advisory. In `docs/suppressing-checks.md` also document: stale ignores outside the edited lines are unreported in the hook until Story 2.2.2 ships (see the AC); `kibitzer check native` bypasses inline ignores; `markdown-link-integrity` ignores name the checker, not the link label; trailing comments cover only their own line; fallback-only (non-grammar) files honor ignores but do not report malformed ones.
- Files: `docs/reporting-false-positives.md`, `CLAUDE.md`

---

## Phase 4: Validation (per CLAUDE.md "Writing a new check")

### Epic 4.1: Backtest and corpus
**Goal**: Show the new checker and filter are quiet at scale and cheap.

#### Story 4.1.1: Backtest the `inline-ignore` checker and the filter
**As a** maintainer, **I want** the checker backtested on real edits and the public corpus, **so that** it does not ship noisy.
**Acceptance Criteria**:
- Transcript backtest run.
  - *Given* `~/.claude/projects/*/*.jsonl`, *When* `kibitzer check backtest inline-ignore`, *Then* the output is recorded in the PR with the count of `[ignore-syntax]` fires (expected near zero; any fire is triaged).
- Corpus run is clean.
  - *Given* the corpus from `scripts/clone-backtest-repos.sh` (`docs/backtest-repos.md`), *When* `kibitzer run <repo>` for each, *Then* zero `[ignore-syntax]` / `[unused-ignore]` false positives on prose or string mentions of `kibitzer`, triaged via `scripts/backtest-triage.py`.
- Self-run on this repo.
  - *Given* this repo (sources and tests embed `kibitzer:ignore` in string literals), *When* `kibitzer run .`, *Then* no finding is suppressed by or reported against a string-literal marker.
- Hook-path cost (no absolute-time gate; CI-stable).
  - *Given* a 5,000-line Go file with no `kibitzer` substring, *When* `apply_inline_ignores` runs 1,000 times, *Then* the structural counters stay at zero (`scan_memo.scans == 0`, `hash_calls == 0`, no parser constructed) and, as a coarse sanity bound only, the median-of-5 time of the 1,000 calls is at most 20x a baseline loop of `source.contains("kibitzer")` over the same input (relative, not an absolute microsecond figure, so a slow CI machine does not flake it). The same file with one marker is scanned once per `InlineIgnoreContext` while consecutive checkers share the single-entry `ScanMemo` (asserted via `scan_memo.scans == 1`, not a "file pass" object, which does not exist), rescanned when the content hash changes, and not scanned at all when the checker returned no findings.
- Baseline measured and the gate recorded before implementation (Phase 0, Tasks 0.1.1-0.1.3; moved from the old Task 4.1.1e).
  - *Given* the PR, *When* reviewed, *Then* it links the Phase 0 gate note (R, F, A, G, p*, marker result, PROCEED/SHRINK decision) and states plainly that token cost is inferred from counts, not measured. A PR with no gate note is not mergeable.
- Footer net-token re-check (round-3 UX gap 1).
  - *Given* the final footer for the longest case, *When* its length is measured (Task 3.1.1b test) and plugged into the Task 0.1.1 break-even formula with the measured `E` and `S`, *Then* `p*` is recomputed and recorded in the PR; if it is now above 0.5, the footer is cut to the generic line before merge.
- Hook-path rerun latency is recorded against the budget.
  - *Given* the largest source file in this repo with a marker in changed rows, *When* the hook-path raw rerun runs, *Then* the added wall time is recorded in the PR and compared with the Non-functional budgets target (under 150 ms, INFERRED).
- Binary files in the corpus walk.
  - *Given* a corpus repo containing images or jars, *When* `kibitzer run <repo>`, *Then* zero `inline-ignore` read-error lines.
**Files**: `docs/backtest-triage/` (results), `src/inline_ignores.rs` (timing test)

##### Task 4.1.1a: Transcript backtest (~0.5h)
- Run `kibitzer check backtest inline-ignore`; record counts in the PR body.
- Files: none (command output)

##### Task 4.1.1b: Corpus and self run (~2h)
- Clone corpus, run, triage with `scripts/backtest-triage.py`; fix any noise (tighten near-miss regex first). Run `kibitzer run .` on this repo.
- Files: `docs/backtest-triage/` (as needed)

##### Task 4.1.1c: Fast-path, latency, and footer net-token checks (~1h; the hook-path measurement moved earlier to Task 2.2.3d)
- Add the structural-counter fast-path test and the relative-ratio sanity check to `src/inline_ignores.rs` (no absolute microsecond threshold). Re-measure the hook-path raw rerun once more on the final code (this repo's largest source file with a marker), compare with the Task 2.2.3d number, and record both in the PR. Measure two alternating marker files through the daemon to see whether the single-entry `ScanMemo` Mutex thrashes (Non-functional budgets). Recompute the footer break-even `p*` with the final measured footer length (Story 4.1.1 AC).
- Files: `src/inline_ignores.rs`

##### Task 4.1.1d: End-to-end ship gate (~1h)
- Before sign-off confirm the Phase 0 gate note is recorded and links the PROCEED/SHRINK decision. Counterfactual coverage check (the pre-ship stand-in for the outcome metric, which transcripts that predate the feature cannot show): for each Phase 0 classified case in classes (b) and (c) whose file content the backtest harness can reconstruct, insert a synthetic directive at the finding's reported row and confirm the finding is dropped; record coverage as n of N. It bounds the achievable reduction, it does not prove adoption. `cargo test`, `cargo clippy`, and a manual end-to-end: add a covering ignore to a fixture Go file, confirm hook (`kibitzer hook` with a PostToolUse payload), `mcp run_checks`, and `kibitzer run` all omit the finding; remove it, confirm it returns.
- Files: none (verification)

##### Task 4.1.1f: Post-ship review checklist (30 days after release) (~1h)
- Not code; a dated checklist recorded in the PR body so the learning loop is not forgotten. Owner: the maintainer (tstapler); due date = release tag date + 30 calendar days, written into the release PR when the tag is cut (hard stop day 44, see `requirements.md` Roadmap Fit).
- **Sample-size rule** (round-3 product gap 6: one maintainer plus kibitzer's own repo is a small sample). Pool every repo and session whose transcripts live under `~/.claude/projects` and where the hook was active, not only this repo; count directives added in the window. If fewer than 20 directives or fewer than 30 dismissed findings exist at day 30, the kill criteria are **INCONCLUSIVE, not triggered**: extend once to day 60 and re-run on pooled data; if still short at day 60, record "insufficient use" and treat that as evidence for kill criterion 2 (agents are not using it), not as a pass.
- Compute metric 1 on dismissed findings only (fixed ones excluded) against the revised target recorded in the Phase 0 gate note (`R - 0.5 x G`), using Task 0.1.1's script on pooled post-ship transcripts; compute the standalone guardrail metric (`[blocking-suppressed]` advisories per 100 directives reviewed); the channel mix from the `kibitzer run` footer counts and `git log -S 'kibitzer:'` vs. new `accepted/` entries. Test the risky assumption that remains: read every `[blocking-suppressed]` advisory and a 20-directive sample of reasons, counting boilerplate that passes the two-word floor (assumption A; more than 10% boilerplate triggers the ADR-002 stricter-reason lever). Assumptions B and C and metric 3 were dropped with the single-marker collapse (Amendment 1). Apply the kill criteria in `requirements.md` Roadmap Fit and record the outcome.
- Files: none
