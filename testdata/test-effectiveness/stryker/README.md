# Stryker (mutation-testing-elements) fixtures

Hand-written to mutation-testing-elements schema v2 (`schemaVersion: "2"`); real-run fixture pending (blocked on owner A11 time-box / Story 0.2.2a).

No Stryker was installed or run to produce these files. Shapes follow the published report schema
(`mutation-testing-report-schema.json`, package v3.9.0): `files.<path>.mutants[]` with `id`, `status`,
`killedBy`, `coveredBy`; top-level `testFiles.<file>.tests[].id/name`; `framework.name/version`.
The producer version in `framework.version` (9.0.1) is a placeholder, not a recorded run.

| File | Purpose |
|---|---|
| `report-v2.json` | Valid report. 4 killed mutants, 2 with one killer: single-killer share 0.50. |
| `report-unlisted-version.json` | `schemaVersion: "99"` (allow-list miss). |
| `report-no-framework.json` | No `framework` block (tool version must come from the sidecar). |
| `report-budget.run.json` | Run sidecar with `budget_exhausted: true`, 4 of 6 mutants run. |

## Fixture-recorded plausibility range

Schema version `2`: the share of killed mutants with exactly one killer is expected in `[0.30, 1.00]`
(mirrored by `FIXTURED_VERSIONS` in `src/test_effectiveness/adapters/mutation_elements.rs`). The range is
a hand-chosen placeholder until a real run records the observed share.
