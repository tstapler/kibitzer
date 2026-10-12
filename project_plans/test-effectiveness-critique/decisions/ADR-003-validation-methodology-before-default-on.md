# ADR-003: A signal is not recommended for deletion until it beats a size-matched random baseline on a held-out fault proxy

**Status**: Accepted (plan phase) | **Date**: 2026-10-11 | **Resolves**: requirements Open Question 6; informs Q2

## Context
No per-test removability signal is validated in the literature (`research/literature-effectiveness-mutation.md` section 7 gap 2). Shi et al. (ISSTA 2018, VERIFIED in research) show reduced suites miss up to 52.2% of real failed builds and traditional metrics do not predict it. "No coverage change after deletion" is nearly tautological for coverage-redundant tests (`research/pitfalls.md` headline). Isolated per-test profiles can mislabel a test as redundant (verified Go counterexample, `research/pitfalls.md` P3).

## Decision
The corpus experiment (Epic 3) evaluates every dynamic signal and every static rule against:
1. **Primary effect**: coverage and mutation-score delta when the flagged set is deleted.
2. **Size-matched random-deletion baseline**: same number of tests removed uniformly at random, repeated over multiple seeds; a signal must beat it, not merely show a small delta.
3. **Held-out fault proxy**: chosen by spike 0.3.1 among historical-failure replay (Failed-Build Detection Loss style), disjoint-mutant-split, and fail-to-pass replay. Mutant-based proxies are labeled as same-family, not real faults.
4. **Attribution**: leave-one-out or in-order attribution for any shortlisted test; isolated profiles are labeled `isolated` and never used alone to justify a deletion claim.
5. **Precision**: manual triage of a stratified sample via `scripts/backtest-triage.py`.

Findings state which attribution method produced them.

## Consequences
- The "delete ~25%, no coverage change" anecdote is treated as a hypothesis (H1) with an explicit, possibly negative, outcome.
- Confidence tiers (high/medium/low) are defined from measured precision after the experiment; no numeric probabilities ship before that.
- Cost: leave-one-out is run on shortlists only; the full-suite run count is bounded by the sampling strategy from spike 0.3.2.

## Amendment (plan repair, 2026-10-11)
Supersedes the weaker wording above where they differ.
1. **Pre-registration**: hypotheses H1-H4, equivalence margins, p-value rule, K seeds, mutant sample size, and the H4 precision floor are fixed in `decisions/G0-decision.md` before any run (Story 0.4.1); changes after the first run go in a dated Deviations section of the protocol.
2. **Unit of deletion**: the `JointSafe` set (greedy, deterministic-order subset with empty joint loss), not the per-test flagged list. Deleting the individually flagged list is a labeled secondary result that measures the joint-loss effect.
3. **Null**: beating a uniform size-matched random set is necessary but not sufficient. A signal must also beat a (package, `TestKind`)-stratified null (N2) and a coverage-preserving random deletion null (N4), with a rank p-value (1 + r) / (K + 1) <= 0.05. K >= 999 where nulls are computed from a matrix, >= 19 where each null needs a mutation rerun. "5th percentile of 30 seeds" is withdrawn.
4. **Held-out proxy tiers**: P1 real-history replay (reported as headline only if a power calculation of about 96 events for a +/-0.10 interval is met), P2 mutant split by operator family and module (mandatory floor, labeled same-family), P3 independent mutation tool where one exists. If P1 is underpowered the write-up says "Q6 answered same-family only".
5. **Mutation cost**: mutation-delta arms are bounded by a `MutationBudget` from spike 0.3.4; only mutants killed by the full suite are rerun; arms over budget report coverage delta only. Per-test kill matrices exist only for PIT and Stryker, so per-test agreement (Q3) is answered only on JVM and JS/TS; Go, Python, and Rust are answered at suite level.
6. **Q3 statistic**: precision, recall, and directional containment of zero-unique-coverage in zero-unique-kill with Wilson intervals. The sets are not nested in general, and Jaccard is not the decision statistic.
7. **Scope**: results are an exploratory study on named, convenience-selected repos; no population claim.
8. **Attribution caps and the circularity guard (Phase 4 repair)**: neither `Isolated` nor `InOrder` attribution alone justifies a deletion claim. `InOrder` (including an assumed default) caps coverage-derived tiers at `Medium` because shared setup can overstate redundancy; `Isolated` is printed and uncapped by attribution; `LeaveOneOut` (shortlists only, Story 3.1.4) is the arbiter where available. Separately, `JointSafe` preserves coverage and kills on its selection data by construction, so a zero delta there is a harness invariant (I0), not validation; only a held-out measure `JointSafe` does not preserve (same-family P2 or real-history P1, never the selection data) may be reported as validated. A coverage-delta-only arm is "no validity evidence".
9. **Null result is a deliverable**: a null or negative outcome ships the descriptive-only report and a `NegativeResultReport` per the outcome table fixed at G0 (plan Story 0.4.1); removing the subcommand requires an explicit owner `remove`.
