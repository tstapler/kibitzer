# G1 decision: DRAFT (file name is deliberately not `G1-decision.md`)

Status: **draft prepared by Claude from the held-out backtest rounds; nothing below is recorded as decided until the owner signs the lines marked `OWNER`.** Story 1.3.1 governs this file (`implementation/plan.md`, Epic 1.3).

## Preconditions the plan requires, and their state

| Precondition | State |
|---|---|
| `G0-decision.md` Part A committed (H4 floor, cap percentile) | **Missing.** No H4 floor exists, so no promote verdict can be mechanically gated. |
| Budget commit (absolute hook latency, line C1) precedes the Story 1.2.3 measurement | **Not done.** Story 1.2.3 has not run. The first commit of the final file must be the budget commit, so this draft must not be committed as `G1-decision.md` in that role. |
| Owner-vs-agent triage agreement | **Not measured.** Every label below was assigned by an agent of the same model family that wrote the rule and tagger. |
| Goodhart measurements from Story 1.2.1 | Not run. |

Because of these, the draft can state evidence and a recommended verdict, but the verdict cannot be recorded as G1-compliant yet.

## Evidence: `test-no-assertion-go`, held-out rounds 5-10

Each round is a blind label of findings from repos not used for tuning the rule version measured. Ground truth was committed before tagger output was viewed (`docs/backtest-triage/oracle-tagger/heldout{5..10}-ground-truth.jsonl`, write-ups `HELDOUT{5..10}.md`). Rounds 1-4 are excluded because their rules were tuned on the same repos. Rounds 5-9 are tuning data for the rule as it stands now, and only round 10 is an unbiased read of the current build.

Loose = finding is a test with no real oracle (true positive of any kind). Strict = `empty-stub` or `discarded-result` only. The strict set is the only one a reader would call "a likely defect".

| Round | n | Loose TP | Loose % | Wilson 95% lower | Strict TP | Strict % |
|---|---|---|---|---|---|---|
| 5 | 131 | 118 | 90.1 | 83.8 | 9 | 6.9 |
| 6 | 54 | 50 | 92.6 | 82.4 | 7 | 13.0 |
| 7 | 143 | 122 | 85.3 | 78.6 | 20 | 14.0 |
| 8 | 51 | 38 | 74.5 | 61.1 | 1 | 2.0 |
| 9 | 245 | 212 | 86.5 | 81.7 | 13 | 5.3 |
| 10 | 148 | 134 | 90.5 | 84.7 | 15 | 10.1 |
| **Pooled** | **772** | **674** | **87.3** | **84.8** | **65** | **8.4** (Wilson 95%: 6.7-10.6) |

Reading caveats, all of which weaken the pooled numbers rather than strengthen them:
- Findings cluster. Per-repo median loose precision is 100% in every round, while pooled figures are dragged by one to three repos (round 8: opentelemetry-go held all 13 false positives; round 9: beats, tidb, teleport held 28 of 33). Counting same-file clusters once gives 86.5-96.5% loose depending on the round.
- Rounds differ in which strict tags counted (round 5 used `discarded-result` + `empty-stub`; round 10 `empty-stub` only). Two of the strict figures are driven by single-repo placeholders (round 10: 10 of 15 are talos `TestEmpty`; round 7: golang-migrate TODO stubs plus 4 intentional empty placeholders).
- Same-model labeler; no human triage.
- Wilson bounds treat findings as independent. They are not, so the true lower bound is lower.

### What the findings actually are

Across rounds 5-10, roughly 8% of findings are strict true positives. Most of the remaining true positives are deliberate smoke tests (no-panic, race, liveness, lifecycle, compile-check) written by someone who chose "run it and see that nothing crashes" as the oracle. They are real findings under the loose definition and a high-volume advisory source, but they are not defects, and the owner's own earlier reading (`G1` question) was whether to ship them as defect signals.

### Other languages

Python and Kotlin have corpus triage only (`docs/backtest-triage/SUMMARY-test-no-assertion-*.md`): Kotlin n=1 finding after a golden-image fix on one repo, and Python from the first corpus runs. There is no held-out blind evidence for either. **Recommendation: no promote verdict for either until they have their own held-out rounds.**

## Recommended verdicts (for the owner to accept, change or reject)

| Rule / language | Mode | Recommended verdict | Reason |
|---|---|---|---|
| `test-no-assertion-go`, strict signal (`empty-stub` + `discarded-result`) | strict | **Candidate to promote**, subject to the H4 floor | Precision on this subset is the best-behaved tag (`empty-stub` 14/14 in round 10, 12/15 in round 9 including 3 non-test fixtures since fixed). Volume is low: about 8% of findings. |
| `test-no-assertion-go`, smoke-only tier | precision-first | **Keep opt-in** as a descriptive tier | 87% loose, but the findings are mostly deliberate smoke tests, so the advisory would be dismissed at a high rate. |
| `test-no-assertion-python` | n/a | **Keep opt-in** | No held-out evidence. |
| `test-no-assertion-kotlin` | n/a | **Keep opt-in** | One repo, one finding after fix. |
| `test-tautology`, `test-mock-only` | n/a | **Not covered by this draft** | Not backtested in these rounds. |

Mechanics of the strict-only promotion: the checker currently emits both tiers. Promoting only the strict subset needs the oracle tag (`empty-stub`, discarded-result) to be a stable finding attribute, but the tagger is behind `KIBITZER_ORACLE_TAGS=1`, covers only 50-60% of true positives, and does not emit `discarded-result` at all. So "promote strict only" is not implementable today without more work. Treat this as the open design question.

## Parameters the owner must fill

```
C1: budget = <OWNER> ms; basis: <OWNER>        # must precede Story 1.2.3 measurement
H4 floor (from G0 Part A): <OWNER>
Hook per-file cap N: <unset: no collapsing>     # derive from Story 1.2.2 distribution
Strict vs precision-first per language: go=<OWNER>
Quiet-path variant: V1 (copyable) | V2 (pointer)   # depends on Story 1.2.1 Goodhart (a)
Verdict, test-no-assertion-go: promote | keep-opt-in | drop = <OWNER>
```

## Known rule limits the verdict should price in

Classes that recurred across held-out rounds, each fixed after the fact and each re-surfacing in a new shape: assertions carried by a wrapper or collaborator that holds `t`, helpers returned to `t.Run`, gated `main()` harnesses, vendored/fixture directories. A fresh batch finds one or two new shapes each round, so the false-positive rate on unseen repos has an open tail; the cost of each miss is a noisy finding, not a missed defect. Cross-file helpers are unresolvable by design (one parse per file); `assertion_helpers` is the escape hatch.

## Provenance

PR #130 (`sdd/test-effectiveness-critique-impl`), rounds in `docs/backtest-triage/oracle-tagger/`. Per-round numbers are those reported by the evaluating agents; the pooled and Wilson figures were computed from them here and the ground-truth files were not re-labeled.
