# Requirements: inline-ignore-syntax

**Date**: 2026-10-07
**Type**: feature addition
**Complexity**: 2 — focused feature

## Problem Statement
An agent or developer who has already judged a kibitzer finding (a false positive, or a real hit accepted as a tradeoff) has no cheap way to say so next to the code. The only per-finding lever is `.kibitzer/accepted/*.json` (`docs/accepting-findings.md`): a separate hand-written file keyed on `(rule, file, line, content)`. It is awkward to author and breaks when the line changes. The finding then reappears on the next edit, the hook feeds it to the agent again, and tokens are spent re-triaging something already decided. There is also no cheap, greppable signal of which suppressions are really checker misfires, and those are the best candidates for new regression test cases.

## Baseline
- Per-finding suppression: hand-write a JSON file in `.kibitzer/accepted/` (`src/accepted_findings.rs`); `docs/suppressing-checks.md` and `docs/accepting-findings.md` state "no inline suppression comment" as a deliberate stance.
- Suppressed findings cost nothing once an entry matches; the cost is the authoring friction and the stale-on-drift re-surfacing.
- Misfires: `report_false_positive` MCP tool queues prose to `~/.local/share/kibitzer/false-positive-reports.jsonl`, drained manually into `docs/<checker>-false-positives.md`. No fixture or test is produced.

## Users / Consumers
- AI agents receiving findings via the PostToolUse hook / MCP `run_checks`.
- Developers and CI running `kibitzer run`, the daemon and the LSP.
- Kibitzer maintainers triaging false positives into checker tests.

## Success Metrics
Outcomes, not capabilities. Capability checks (finding disappears, survives edits, zero `accepted/` regressions) stay as acceptance tests in `implementation/plan.md`; they are not the success bar.

Token cost is **inferred, not measured**: the problem claim is that a re-surfaced, already-judged finding costs the agent a re-triage turn. Counting re-surfaced findings is the measurable proxy; token measurement stays out of scope (see Out of Scope).

| Outcome | Baseline | Target | How measured |
|---|---|---|---|
| Re-surfacing rate of **dismissed** findings: share of findings re-reported for the same `(rule, file, message)` after the agent deliberately left them (a dismissal) earlier in the same session. Findings the agent fixed are excluded: a re-report after a fix may be a real regression, and inline directives cannot help with it | Measured **before any implementation** by plan Phase 0 Task 0.1.1 (transcript replay via `kibitzer check backtest`): `R` (re-surfacing rate) and `G = R x A`, the addressable share after hand classification. Until Phase 0 records a gate decision, the problem is a hypothesis, not a fact | **Set from the Phase 0 data, not a placeholder**: post-ship rate at most `R - 0.5 x G`, i.e. remove half of the addressable portion (classes "agent deliberately left it" and "checker misfire"). The 0.5 is an adoption assumption (agents place the directive correctly for half the addressable cases), recorded in the gate note and revised at the review with observed adoption. The replay runs on transcripts that predate the feature, so no reduction can be observed before ship; pre-ship, plan Task 4.1.1d runs a counterfactual coverage check on the classified cases (it bounds the achievable reduction, it does not prove it). Gate: `G < 2%` stops the project, `2% <= G < 5%` shrinks it, `G >= 5%` proceeds (kill criterion 1; thresholds provisional, see Roadmap Fit) | `kibitzer check backtest all --only-new` over `~/.claude/projects`, counted by Task 0.1.1's script, rerun at the 30-day review on pooled repos and sessions |
| Suppression channel mix: inline directives vs. `.kibitzer/accepted/*.json` entries for newly dismissed findings | 0% inline (not possible today); `accepted/` entry count at ship | At least 70% of suppressions created in the first 30 days are inline (provisional: inline is meant to be the default, and the remaining 30% leaves room for the cases it cannot serve, files the agent must not edit, generated files and non-comment formats; revise after the first review) | Count of `kibitzer:` directives added (`git log -S` / `kibitzer run` footer `findings suppressed inline`) vs. new `.kibitzer/accepted/*.json` entries over the same window |
| ~~Checker improvements driven by inline misfire markers~~ **DROPPED 2026-10-07** (single-marker collapse: no `false-positive` marker, no `list --inline`) | n/a | n/a | n/a. Misfires continue through the `report_false_positive` queue (`docs/reporting-false-positives.md`) |
| Marker honesty (guardrail, not a goal): weak or blanket suppression | n/a | `[ignore-syntax]` weak-reason fires and `[ignore-volume]` fires stay near zero in transcripts | Task 4.1.1a backtest count |
| Blocking-suppression guardrail (standalone): `[blocking-suppressed]` advisories per 100 directives reviewed at the 30-day review | n/a (the advisory does not exist today) | At most 10 per 100 directives reviewed (provisional; matches the 10% line in kill criterion 3), and every one read and judged for an honest reason | Count of `[blocking-suppressed]` advisories in hook output and transcripts, divided by the number of directives added in the window (`git log -S 'kibitzer:'`), times 100; the `run` footer's blocking count is the cross-check |

## Risky Assumptions
Each is a bet the feature loses on if wrong; each gets a cheap test before or soon after ship.

| Assumption | Cheap test | If it fails |
|---|---|---|
| A. Agents write honest reasons rather than boilerplate or blanket ignores, especially on blocking checks (ADR-002 bounds this with guardrails but does not remove the bet) | Before build: read the reasons the agent gave in prose for the 20 classified dismissals (Phase 0 Task 0.1.1) and count boilerplate that would pass the two-word floor; after ship: read every `[blocking-suppressed]` advisory and a 20-directive random sample of added directives at the 30-day review | Tighten the `Reason` rule or lower the `[ignore-volume]` threshold (ADR-002 Consequences); if blocking checks are routinely defeated, revisit excluding them |
| ~~B. The `false-positive` list is actually triaged~~ | DROPPED 2026-10-07 with the single-marker collapse | n/a |
| ~~C. Agents choose the right marker from one footer clause~~ | RESOLVED before build by plan Task 0.1.2: class (c) was 0 of 4, so the split collapsed to one marker. Task 3.1.1d still checks the footer text parses via `parse_comment_line` | n/a |

## Roadmap Fit
- **Relation to `.kibitzer/accepted/*.json`**: coexist. Inline is the default for anything that can hold a comment; `accepted/` stays for locations that cannot (generated files, files the agent must not edit, non-comment formats). No deprecation in this project. **Precedence**: inline runs first (inside `run_checker_against_source`), `drop_accepted_findings` second; both are independent suppressors, so a finding is dropped if either matches. `accepted/` behavior is unchanged.
- **`report_false_positive` queue** (`~/.local/share/kibitzer/false-positive-reports.jsonl`): complementary. The queue captures prose reports from anywhere; `kibitzer:false-positive` captures the same signal at the exact line, in the diff. `list --inline` prints a nudge to also file the general report. Neither generates fixtures (still out of scope).
- **Deferred agent-writable accept tool / `kibitzer accept` CLI**: not built here. If inline directives reach the channel-mix target (metric 2), the tool is not needed; if agents cannot edit the file in question, it becomes the follow-up for exactly the `accepted/` residue. Revisit at the 30-day review.
- **Owner and date of the post-ship review**: the maintainer (tstapler) owns it. Date rule: 30 calendar days after the release tag's date, entered in the release PR when the tag is cut (Task 4.1.1f). If it slips, run it on whatever data exists no later than day 44 and note the slip; do not skip it. **Sample size**: the maintainer is the only known user, so the review pools every repo and session under `~/.claude/projects` where the hook was active, not just this repo. With fewer than 20 directives or fewer than 30 dismissed findings at day 30, the kill criteria are INCONCLUSIVE, not triggered: extend once to day 60; if still short, record "insufficient use" and count it toward kill criterion 2 (plan Task 4.1.1f). The `false-positive` worklist (assumption B) has the same owner.
- **Provisional thresholds**: the gate thresholds (2% stop, 5% proceed, on `G = R x A` from plan Task 0.1.1) are judgement calls, not measured figures: below 2% a re-surfaced, addressable finding occurs less than about once per 50 findings, which does not justify a new syntax, checker and docs surface; 5% is about one in 20. The 30% and 70% inline-share lines (criterion 2, metric 2) rest on "inline should be the default; `accepted/` covers the residue". All are provisional; the gate thresholds are fixed before Phase 0 reads its result and revised only with that data in hand, never after the fact.
- **Post-ship learning and kill criteria** (review 30 days after release, using the metrics above):
  0. **Phase 0 go/no-go (before any implementation)**. Result `G = R x A` (plan Task 0.1.1): `G < 2%` (or too few re-surfaced tuples to classify after widening once): STOP, keep the pure `accepted/` path, reopen the agent-writable accept tool; `2% <= G < 5%`, or `G >= 5%` with a footer net-token break-even above 50%: SHRINK to the reduced slice (plan Effort and appetite); `G >= 5%`: PROCEED. The requester decides on the gate note; no answer means nothing starts.
  1. After ship, the post-ship rate misses the target `R - 0.5 x G` at the pooled 30-day review (INCONCLUSIVE below the sample-size floor above): the feature is not paying for its surface. Revert the footer/hint teaching text first, keep the pure `accepted/` path, and reopen the agent-writable accept tool as the alternative.
  2. Inline share of new suppressions under 30%: agents are not using it; fix teaching surfaces (footer, MCP instructions) before anything else.
  3. `[ignore-volume]` or weak-reason fires above 10% of directives, or `[blocking-suppressed]` appearing on most blocking failures: ADR-002 guardrails are insufficient; apply the ADR's stricter-reason levers, then revisit excluding blocking checks.
  4. ~~Fewer than 5 `false-positive` markers after a month~~: already triggered by Phase 0 Task 0.1.2 (marker-choice test failed); the markers collapsed to one before build. No post-ship check.

## Appetite
Medium (1–2 weeks). Re-estimated in `implementation/plan.md` (Effort and appetite), with totals recounted by command from the per-task figures: 4.0 hours of Phase 0 (baseline replay and go/no-go gate) plus 58.0 hours of implementation, 62.0 hours committed (59.5 after the 2026-10-07 single-marker collapse dropped Story 3.2.1), 64.0 (61.5) including the deferred `kibitzer run` unused-ignore audit (Task 2.2.2b, follow-up PR). That is upper-middle of Medium with no slack; the earlier 52.5-hour figure under-counted (round 3 re-estimated three tasks and found four missing pieces). If Phase 0 says SHRINK, the slice is 49.0 hours. A STOP costs 4 hours, not 62.

## Constraints
- Must work across every language kibitzer supports with its own comment syntax (`//`, `#`, `<!-- -->`, etc.).
- A reason must be required, consistent with `accepted/`'s non-empty `reason` rule (the "written-down tradeoff" principle).
- Applies to native per-file checkers only, the same scope as `accepted/` (shell-out `command` checks and whole-repo architecture checks stay out).
- Must not weaken the stance that suppressions stay reviewable: an ignore comment is visible in the diff.

## Non-functional Requirements
- **Performance SLO**: no measurable slowdown to hook-path checks (comment scan is a single pass over already-read file text).
- **Scalability**: not applicable
- **Security classification**: internal
- **Data residency**: no special requirements

## Scope
### In Scope
- Inline ignore comment syntax (rule id + required reason; line-scoped on the same or next line; optionally a distinct marker to flag a misfire vs. accept a tradeoff).
- Filtering in the same shared path `accepted_findings::filter_accepted` already uses, so all entry points (hook, MCP, run, daemon, LSP) pick it up.
- Malformed ignore (missing rule or reason) reported as a clear error/finding, not a silent no-op.
- ~~A listing command for ignores marked as false positives.~~ Cut 2026-10-07 (single marker, see Open Questions).
- Docs updates: `docs/suppressing-checks.md`, `docs/accepting-findings.md` (the "no inline comment" statements).

### Out of Scope
- Agent-writable accept tool / `kibitzer accept` CLI (not selected).
- Auto-generating fixtures or draft regression tests from false-positive reports.
- Token-burn measurement/audit.
- Block/file-level ignore ranges, unless Phase 2 research shows line-scoped is insufficient.
- Shell-out `command` checks and whole-repo architecture checks.

## Rabbit Holes
- Finding line vs. comment placement: checkers report on different lines than the "flagged statement" for multi-line constructs (functions, duplicates spanning files).
- Cross-file checks (`duplicate-code-cross-file`) report at two locations; which one carries the ignore?
- Language comment-syntax detection and comment-in-string false matches.
- Markdown checks (`markdown-link-integrity`, prose checks) where inline comments are `<!-- -->` and may render or interfere.
- Checkers that don't self-prefix a `[rule-id]` (single-rule checkers use the checker name).
- Interaction and precedence with `.kibitzer/accepted/` and the `comment-quality` checks themselves (an ignore comment must not trigger `commented-out-code`/`verbose-comment`).

## Alternatives Considered
- Keep config-only (status quo) and add only an agent-writable accept tool — rejected by the user for this project; a possible follow-up.
- Reuse existing linter directives (`# noqa`, `// nolint`) — would couple to other tools' semantics and lack a mandatory reason.
- Content-hash-anchored entries (self-healing `accepted/`) — keeps the file-based stance but still needs a separate file per finding.

## Feasibility Risks
- A uniform "line the finding is on" is not guaranteed across all native checkers; some may need a normalized anchor line.
- Reversing the documented "no inline suppression" design decision needs an ADR.

## Observability Requirements
Not applicable (complexity 2).

## Risk Control
Not applicable (complexity 2).

## Open Questions
The four design questions are resolved in `implementation/plan.md` (Design Decisions). **One question does block building**: whether the problem is big enough to build for.
- OPEN, BLOCKING, measured baseline: the re-surfacing baseline (Success Metrics) is unmeasured until plan Phase 0 Task 0.1.1 runs, and its result gates the whole project through kill criterion 0 (Roadmap Fit): STOP, SHRINK or PROCEED. No Phase 1 task starts before the gate note is recorded. (Earlier drafts said "none block building" while the baseline gated the kill criteria; that was contradictory.)
- RESOLVED, syntax: `<leader> kibitzer:ignore <rule>[,<rule>...] -- <reason>` with ASCII `--` (Design Decision 1). **Single marker (2026-10-07)**: the planned second marker `kibitzer:false-positive` is removed from scope. Plan Task 0.1.2 found class (c) checker misfire at 0 of 4 addressable cases (needs at least 10%), so the marker-choice test fails on its own (`docs/backtest-triage/inline-ignore-gate.md`); the requester accepted the collapse with the PROCEED decision. `kibitzer:false-positive` is reported as a near-miss marker, never as a directive. Consequences: Story 3.2.1 and Epic 3.2 are dropped; the marker clause leaves the footer; Success Metric 3, Risky Assumptions B and C and kill criterion 4 no longer apply (see below).
- RESOLVED, same line vs. next line: both. A whole-line comment covers its own rows and the row directly below; a trailing comment covers only its own row (Design Decision 2 and 3). The anchor is the checker's reported `Finding.line`.
- RESOLVED, unused/stale ignores: reported as `[unused-ignore]`. The hook-path advisory for a directive just added on the wrong row (Story 2.2.3) is core (dropped only in the SHRINK slice); the `kibitzer run` rerun (Task 2.2.2b) is deferred to a follow-up PR, with the unknown-rule typo case covered in `run` by Task 2.2.2e (Design Decision 7).
- RESOLVED (cut), listing command: `kibitzer check false-positives list --inline` (old Story 3.2.1) is **cut** with the single-marker collapse. With one marker the listing could only enumerate every `kibitzer:ignore`, which `git grep 'kibitzer:ignore'` already does; misfires keep flowing through the existing `report_false_positive` queue.

## Scope Additions Beyond the Original Ask
The original ask: syntax, shared-path filtering, malformed error, false-positive listing, docs. Everything else was added by ADR-002 and the pre-mortem. Round-3 disposition (full table with hours in `implementation/plan.md`, "Scope disposition"):

| Addition | Decision | Why |
|---|---|---|
| `[ignore-volume]` | keep | P1-2 guardrail, 0.5h |
| `[blocking-suppressed]` | keep | price of ADR-002 (blocking checks stay suppressible) |
| weak-reason rule | keep | P1-2, 0.5h; floor stays two words (accepted risk, measured) |
| `run` footer total + blocking count | keep | P1-2 visibility across files |
| `run` footer `false-positive` split | defer | `list --inline` gives the same number at review |
| `--no-inline-ignores` | keep | the raw mode must exist for the hook rerun; the flag shows what is hidden |
| `[unused-ignore]` in the hook | keep (dropped if SHRINK) | P1-1 wrong-row loop |
| `[unused-ignore]` in `kibitzer run` | defer | nothing depends on it; unknown-rule typos covered by Task 2.2.2e |
| marker split and `list --inline` (Story 3.2.1) | **dropped 2026-10-07** | Task 0.1.2: class (c) 0 of 4; single `kibitzer:ignore` marker; requester accepted |
| success ack line | defer | `kibitzer run` confirms; revisit on 30-day evidence |
| `run` one-line syntax hint | keep | developer discoverability, 0.5h |
