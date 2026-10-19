# Cross-artifact consistency review: test-effectiveness-critique

Inputs: requirements.md, implementation/plan.md, design/ux.md, ADR-001..005 (amended). Totals: 2 BLOCKER, 9 CONCERN, 5 NITPICK.

## BLOCKER

B1. SingleRun contradicts the one-command report (plan Glossary `ExclusionReason`, Story 2.1.3 vs ux.md 3.2/UX-R1/R4). Plan: with one input "no test can be judged and each is marked `SingleRun`" and `SingleRun` is a "not assessed" reason, so a single `ctx.json` yields zero findings. ux.md wireframe shows 212 assessed from one input and lists no SingleRun. Fix: make single-run a footer stability note ("Stability not assessed: single run") that caps nothing and excludes nothing; keep `Flaky` as the only stability exclusion, and drop `SingleRun` from `ExclusionReason` (or cap tier at Medium instead of excluding).

B2. Requirements In Scope bullet 2 ("inventory of existing tools and what each emits, machine-ingestable or not") has no story. Story 0.1.1 covers the signal catalog and bibliography only. Fix: add Task 0.1.1d to Story 0.1.1 (tool-inventory table in docs/test-effectiveness.md, sourced from research/stack.md) with an AC.

## CONCERN

C1. ADR-001 amendment still says `Unlocatable`; plan says `NoSource` (repair log admits it). Fix: edit ADR-001 line 23 to `NoSource`.
C2. JSON field name drift: ux.md S3 record uses `rule`; plan Story 2.3.1 AC and Glossary use `signal`. ux.md also adds `commit`, `schema_version`, `not_assessed` that the AC omits. Fix: pick one (`rule`) and update the AC.
C3. Rule-id strings: ux.md wireframe Signal column prints `zero-unique-coverage`, `no-assertion`, `mock-only` while claiming "same string as `[rule]`"; plan uses `test-zero-unique-coverage`. The plan never enumerates ids for ZeroUniqueKills/PseudoTestedRegion, which the RULES drift guard needs. Fix: add a Signal-to-rule-id table in the plan; correct the wireframe.
C4. ADR-002 decision 3 (surfaces share only rule-name/suppression conventions) vs plan: the report shares `src/test_code.rs` and AssertionVocabulary (Task 0.2.1c), and ux.md shows static signals (no-assertion, mock-only) as report rows. No Story 2.3.1 AC says the report computes static signals; Story 3.2.1 needs a NoAssertion flagged set. Fix: amend ADR-002 and add an AC.
C5. Attribution provenance: ADR-001 says tool, version, run time and attribution method must be supplied (flags if absent) and adapters reject otherwise; plan rejects only tool+version and defines no attribution source for coverage.py/lcov or flag. Fix: add `--attribution`/default rule in Story 2.2.1/2.2.3.
C6. ux.md caps not in plan: R14 "in-order is medium at most", wireframe "no mutation data: capped at medium". Plan caps are only Low (integration/snapshot, FirstKillerOnly, unknown commit). Also ADR-003 item 4 treats isolated as the suspect attribution while R14 warns about in-order. Fix: add to the G0 tier-rule or delete from ux.md.
C7. G0 dependency hidden: Stories 1.2.2 (cap percentile) and 1.3.1 (H4 floor) read G0-decision.md, but the dependency diagram routes G0 only to Phase 3, so Phase 1 evidence is blocked behind spikes 0.3.x. Fix: add the edge, or move the H4 floor and cap rule into a G0a record.
C8. Success Metric 3 (corpus: cassandra, servo, deno, vscode) is met only "in part" (static only) with no owner acceptance step. Fix: add acceptance of the reduced scope to Story 0.4.1.
C9. Leave-one-out attribution (`CoverageAttribution::LeaveOneOut`, ADR-003 consequence, R_max in 0.3.2) has no producing adapter/recipe/story. Fix: add a task in Epic 3.1/3.2 or drop the variant.

## NITPICK

N1. ux.md uses `[inline-ignore]` for malformed directives; docs/suppressing-checks.md and plan use `[ignore-syntax]` (`inline-ignore` is the check name). Fix ux.md D1/D-E2.
N2. ux.md 2.3 "emitted only for tests inside changed ranges" overstates; per ADR-004 amendment only first-line-in-range is flagged (body edits are 4.1.2). Align wording.
N3. Report ranking key ("lowest-risk-first, then cost") and `--input-commit`, `--experimental`, `--junit` flags appear in only one of plan/ux.md.
N4. UX surfaces without ACs (acknowledged in repair log): S6 unknown-option warning, S7 `--verbose`, S8 recipe errors, UX-A1..A3, D-E4/D-E5.
N5. Scope-drift candidates (all conditional or owner-requested): 2.3.3 ArchModel enrichment, 3.3.4 LLM pilot, 4.1.2 assertion-removed, 2.2.6 JUnit adapter, 1.1.4 `test-mock-only` (ADR-004 "transitive" vs Story 1.1.2 "bounded depth" helper resolution).

## Traceability and ADR checks that passed

- Requirements scope items 1, 3, 4, 5 and Success Metrics 1, 2, 4, Observability, Risk Control, NFRs map to stories (0.1.1, 0.3.x/3.x, 2.3.4, 4.x, 1.2.3, 2.1.5).
- ADR-003 amendment (pre-registration, JointSafe, N2/N4, rank p, tiers P1-P3, MutationBudget, Q3 statistic) matches plan Stories 0.3.1, 0.3.4, 3.1.x, 3.3.2.
- ADR-005 amendment (inverted index, `roaring` only on scale-test failure, quick-xml with 2.2.5) matches Stories 2.1.1/2.1.5/2.2.5.
- ADR-004 amendment (added or signature-changed; `inventory::submit!`; `test_quality_checks()` unchained) matches Stories 1.1.2/1.1.5.
- Glossary count 29 verified by recount. Finding vs TestFinding and TestMatrix usage consistent.
