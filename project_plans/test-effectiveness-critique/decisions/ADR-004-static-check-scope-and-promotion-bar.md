# ADR-004: Static test-quality checks start narrow, stay registry-only until triaged, and only nudge on tests added or signature-changed in the diff

**Status**: Accepted (plan phase) | **Date**: 2026-10-11 | **Informs**: requirements Open Question 5

## Context
`CLAUDE.md` requires a native per-file checker that needs no repo setup to land in `default_checks()` and requires backtest evidence first. Smell detectors misclassified over 70% of smells on generated tests (Panichella, EMSE 2022, VERIFIED in research); no study validates a static no-assertion detector (`research/literature-redundancy-smells-llm.md` section 3). Agents optimise for silencing checks, so findings on pre-existing tests would invite deletion (`research/ux.md` section 2).

## Decision
1. Implement only high-precision patterns first: assertion-free test body after bounded-depth same-file helper resolution (a call into another file that receives the framework handle counts as asserting; a precision-first name-pattern mode is measured in plan Story 1.2.2), literal tautologies (`assert True`, `expect(1).toBe(1)`, self-comparison), and mock-verification-only (opt-in, medium confidence).
2. Exempt by default: snapshot/golden assertions, expected-exception annotations and `should_panic`, fuzz/property harnesses, Go `// Output:` examples, generated files, and any test passing its framework handle (`t`) to a function the checker cannot resolve (assumed asserting).
3. Anchor findings on the test function's first line, so existing diff scoping suppresses findings on tests the diff did not add or change.
4. Register in `test_quality_checks()` but do not chain into `default_checks()` until the owner accepts per-rule triaged precision at gate G1. Each rule is promoted, kept opt-in, or dropped independently.
5. Messages never instruct deletion; they present strengthen / justify / remove-with-named-covering-test as a decision.

## Alternatives rejected
- Ingest ESLint/golangci JSON instead: deferred; useful as a later input, but needs the user to run those linters and does not serve the hook path.
- Port tsDetect rules wholesale: rejected; GPL-3.0 and Java-centric (`research/build-vs-buy.md` section 1a), and weak validity.

## Consequences
The `test-no-assertion` hook finding cannot be satisfied by deleting a pre-existing test, because it never fires on one in hook mode; suite-wide listing exists only in the explicit report.

## Amendment (plan repair, 2026-10-11)
- Title and decision 3 read "only nudge on tests **added or signature-changed** in the diff". Anchoring on the function's first line passes diff scoping only when that line is in a changed range, so an edit that removes assertions from the body of an existing test is not flagged; that case is covered only by the conditional Story 4.1.2 (`test-assertion-removed`).
- Decision 4 mechanism: registration is `inventory::submit!` in each checker module (`src/checker.rs:248`); `test_quality_checks()` returns `Check` entries that nothing chains (precedent `prose_checks()`, `src/config.rs:784`). `src/checkers/mod.rs` gets only `pub mod` lines.
- The "assertion_helpers" option is resolved once from `.kibitzer/inspect.json` so the three checkers and the report share one vocabulary.
- The H4 precision floor that gates promotion is fixed at G0 before triage, not derived afterwards.
