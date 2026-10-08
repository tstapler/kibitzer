# inline-ignore go/no-go gate (Phase 0, Tasks 0.1.1-0.1.3)

Plan: `project_plans/inline-ignore-syntax/implementation/plan.md`, Phase 0. Replay date 2026-10-07,
`kibitzer` built from `feat/inline-ignore-syntax` (master 3660e64 code), single-user sample.

## Method

```
./target/release/kibitzer check backtest all > /tmp/p0/all.txt     # NOT --only-new, so (pre-existing) is kept
python3 -I scripts/resurface-baseline.py /tmp/p0/all.txt --sample 20
```

Replay summary line: 1449 transcripts scanned, 31506 snapshots checked, 3677 edits unreconstructable.
A tuple is (transcript, rule, file); it re-surfaces when the same (rule, file, message) is reported at
a later seq tagged `(pre-existing)`.

## Numbers (VERIFIED, from the commands above)

| Quantity | Value |
|---|---|
| Findings parsed | 15875 |
| `F` (tuples with a finding) | 809 (gate floor: 100) |
| Re-surfaced tuples | 480 (gate floor: 10) |
| `R` (literal definition) | 59.33% |
| `R` strict (first sighting was agent-introduced, not pre-existing) | 359 / 809 = 44.38% |
| Median re-surfacing messages per re-surfaced tuple | 5 |
| `E` (replay edit events with at least one new finding) | 1389 |
| Distinct re-surfaced findings (rule, file, message) | 2153 |
| Re-report events (pre-existing sightings inside re-surfaced tuples) | 8085 |

Top rules by re-surfaced tuples: `syntax-rules` 86/145, `markdown-link-integrity` 50/67,
`em-dash-overuse` 47/82, `syntax-rules-rust` 46/59, `primitive-obsession` 41/63.
Sample size is sufficient (F >= 100, re-surfaced >= 10), so the STOP-on-sparse-data branch does not apply.

## Hand classification of 20 strict re-surfaced tuples (seed 7)

Classifier: one fresh subagent reading each transcript region, including whether the hook feedback
the agent saw actually contained the finding. Full table: `/tmp/p0/classification.md` (scratch, not committed).

| Class | Count |
|---|---|
| (a) addressed, regressed | 3 (#2, #9, #17; #9 and #17 weak) |
| (b) deliberately left | 4 (#7, #8, #10, #16) |
| (c) checker misfire, as judged by the agent | 0 |
| (d) unobservable: the agent was never shown the finding | 13 |

- `A` = (b)+(c) share = 4/20 = **20%**. Of the 7 tuples where the agent demonstrably saw the finding, 4 were (b).
- Stated reasons for (b): 2 gave one. #8 "throwaway /tmp recording script, not repo code" (specific); #16
  "pre-existing, not introduced by my edits" (boilerplate). So boilerplate = 1 of 2, a sample too small to apply the
  10% rule from the plan's Accepted risks.
- **Replay-vs-hook mismatch (important).** 13 of 20 re-surfacings never appear in any hook feedback in the
  transcript. The replay runs today's checkers over reconstructed file states and counts every
  `(pre-existing)` sighting, but the live hook downgrades pre-existing findings and diff-scopes. So `R` measures
  replay re-reports, not findings the agent was shown twice. This is the plan's own honest limit (1)
  and it bites hard here: `A = 20%` is the only visibility correction in the formula.

## Gate arithmetic

- `G = R x A`: 59.33% x 20% = **11.9%** (strict R: 44.38% x 20% = **8.9%**). Both >= 5%.
- `p* = (E x c_footer) / (S x c_re)` with `c_footer` = 54 and `c_re` = 150 tokens. The plan defines `S` as "addressable
  re-surfaced findings (classes b+c, summed over tuples)" without fixing the unit, and the verdict depends on it:

| Unit for S | S | p* | Verdict (G >= 5%) |
|---|---|---|---|
| re-report events x A: 8085 x 0.20 | 1617 | 0.31 | PROCEED (p* <= 0.5) |
| distinct re-surfaced findings x A: 2153 x 0.20 | 431 | 1.16 | SHRINK (p* > 0.5) |
| re-surfaced tuples x A: 480 x 0.20 | 96 | 5.21 | SHRINK (p* > 0.5) |

  Break-even `A` for PROCEED under the per-finding unit is about 0.46 (the observed 0.20 is far under it).

## Marker dry run (Task 0.1.2)

Not executed as an agent test: the decision rule needs agreement >= 70% **and** class (c) >= 10% of (b)+(c) to keep two markers.
Class (c) is 0 of 4, which fails the second condition on its own, so more agreement data cannot change the outcome.
**Result: collapse to one marker (`kibitzer:ignore`)**, drop Story 3.2.1 (-2.5h) and the marker clause from the footer.
Caveat: (c) = 0 reflects that the agent never called a finding a false positive in these transcripts; the checker-side
triage (`docs/backtest-triage/`) does record false positives, so this is a statement about agent behavior, not checker precision.

## Decision

The mechanical rule gives G >= 5% (PROCEED or SHRINK depending on the unit chosen for `S`). The plan states the
requester decides and that an unanswered note means nothing in Phase 1 starts, so no implementation has begun.
Recommended: SHRINK. Reasons: (1) under two of three readings of `S`, p* > 0.5; (2) 13 of 20 sampled re-surfacings
were never shown to the agent, so the true agent-visible `R` is far below the replay figure; (3) no marker split.

Limits: replay uses today's checkers; transcripts cannot show a dismissal; token costs are INFERRED; single-user sample;
n = 20 hand-classified.

## Human decision (Task 0.1.3), 2026-10-07

Decided by the requester (tstapler): **PROCEED with the FULL plan** (about 62h committed as planned; 59.5h after the
single-marker collapse below). Deferred items stay deferred: Task 2.2.2b, the `run` footer `false-positive` split,
and the success ack.

- Unit of `S`: **re-report events x A** (8085 x 0.20 = 1617), so `p* = 0.31`, under the 0.5 line. The other two readings
  of `S` (distinct findings, tuples) would have given SHRINK; they are not used.
- Caveats seen and accepted: (1) 13 of 20 sampled re-surfacings never appeared in any hook feedback, so the replay
  overstates `R`; `A = 20%` is the only visibility correction; (2) the marker split collapses to the single
  `kibitzer:ignore` marker (Task 0.1.2 result above), which drops Story 3.2.1 and the marker clause from the footer.
- Revised post-ship target (from the plan): re-surfacing rate of dismissed findings at most `R - 0.5 x G`, with
  `G = 11.9%` (literal `R`) so the replay-basis target is about `59.33% - 5.95% = 53.4%`; revised at the 30-day review.

## Post-ship review checklist (Task 4.1.1f)
Owner: tstapler. Due: release tag date + 30 calendar days, entered in the release PR when the tag is cut (the date is not
set yet). Hard stop: day 44, then run on whatever data exists and note the slip. Single marker only (Amendment 1): no
`list --inline`, no assumptions B or C, no metric 3.
- [ ] **Pool the data.** Every repo and session under `~/.claude/projects` where the hook was active. Count directives added
  in the window (`git log -S 'kibitzer:'` per repo).
- [ ] **Sample-size rule.** Under 20 directives or under 30 dismissed findings at day 30: kill criteria are INCONCLUSIVE, not
  triggered. Extend once to day 60 and re-run on pooled data; still short: record "insufficient use" and count it toward
  kill criterion 2.
- [ ] **Metric 1, re-surfacing of dismissed findings.** Run `scripts/resurface-baseline.py` on pooled post-ship replay output
  (as in Method above), dismissed findings only (fixed ones excluded). Compare with the target `R - 0.5 x G` (about 53.4% on
  the replay basis). A miss is kill criterion 1: revert the footer and hint teaching text first, keep `accepted/`.
- [ ] **Guardrail.** `[blocking-suppressed]` advisories per 100 directives reviewed, at most 10. Read every one and judge the
  reason.
- [ ] **Assumption A, reason quality.** Read a 20-directive sample of reasons and count boilerplate that passes the two-word
  floor. Over 10% triggers the ADR-002 stricter-reason lever.
- [ ] **Channel mix.** Inline share of new suppressions (`git log -S 'kibitzer:'` against new `accepted/` entries), cross-checked
  with the blocking count in the `kibitzer run` footer. Under 30% is kill criterion 2: fix the teaching surfaces (footer, MCP
  instructions) first.
- [ ] **Volume.** `[ignore-volume]` or weak-reason fires above 10% of directives is kill criterion 3: apply the ADR-002 levers.
- [ ] **Record the outcome** (kill criteria in `project_plans/inline-ignore-syntax/requirements.md`, Roadmap Fit) here, with
  the date, the counts, and whether the review was INCONCLUSIVE.
