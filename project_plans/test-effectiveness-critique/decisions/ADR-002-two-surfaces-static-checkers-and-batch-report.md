# ADR-002: Two independent surfaces: static per-file checkers plus a separate batch report with its own output type

**Status**: Accepted (plan phase) | **Date**: 2026-10-11

## Context
Static vacuity signals fit the hook path and the `Finding {line, message}` contract (`src/checker.rs:54-57`). Dynamic signals need whole-suite input and must report signal name, raw value, baseline, and tool/version (requirements, Observability). `Finding` cannot carry those. The checker-plugin-system precedent argues against widening shared types (`research/architecture.md` section 5 item 3).

## Decision
1. Static checks (`test-no-assertion`, `test-tautology`, `test-mock-only`) are native `Checker`s in `src/checkers/`, built on a shared `src/test_code.rs` seam, chained through a new `test_quality_checks()` only after owner gate G1.
2. The batch report is a new top-level subcommand `kibitzer tests report` in module `src/test_effectiveness/`, with its own `TestFinding` struct. It is never wired into `default_checks()` or hook mode.
3. The two surfaces share rule-name and suppression conventions (`[rule-id]`, `kibitzer:ignore`) and the read-only test-code seam `src/test_code.rs` (`TestCodeProfile`, `AssertionVocabulary`, `test_functions`, `assertion_calls`). The report uses the seam to compute static rows (`test-no-assertion`, `test-mock-only`) for tests the `TestLocator` resolves; it shares no output type, no `Finding`, and no hook-path code with the checkers (amended in the Phase 4 repair; plan Story 2.3.1).

## Alternatives rejected
- Widen `Finding` with optional structured fields: rejected; touches every checker and the SARIF/scoping paths for one family's needs.
- Place the report under `kibitzer architecture ...`: rejected; it is not derived from `ArchModel` (optional enrichment only).
- Put the report under `kibitzer check ...`: rejected; `check` runs a named checker against a file or dir, and this takes report inputs.
- One combined "test quality" checker that ingests reports at hook time: rejected; violates the hook-latency NFR.

## Consequences
`main.rs` gains one enum variant and one dispatch line; handler logic lives in the new module so the 1182-line file does not grow materially.
