# Requirements: split-loop-check

## Problem

A single loop that advances two or more accumulators with no data dependency between them (e.g. summing two unrelated totals in one pass "for efficiency") does two jobs in one place — Fowler's **Split Loop** (Refactoring, 2nd ed., p. 227). It's a narrow, well-defined pattern: light data-flow within a loop body, no cross-file model needed.

## Proposal

Add a new native per-file checker `split-loop-<lang>` (initially targeting Go, then extending to the other supported languages). Within each loop body, identify variables assigned via accumulator-shaped operations (`x += ...`, `x = append(x, ...)`, `x[k] = ...`) and group them by mutual data dependency (does accumulator A's update expression directly reference accumulator B?). Flag loops with 2+ independent groups.

## Scope

- Multi-language: Go first (rich loop node-kind coverage), then TypeScript/JavaScript, Python, Java, Kotlin, Rust, Bash.
- v1 is conservative: direct-reference only. If accumulator A's RHS mentions accumulator B by name, they are dependent. Everything else is considered independent. Ambiguous cases (computed field paths, method chaining) are not flagged.
- Treat anything ambiguous as "don't flag" rather than risk false positives — this smell's fix is cheap for a human to apply once pointed at the loop, so precision matters more than recall.
- No cross-file model needed; purely structural/intra-loop analysis.
- Must be wired into `config::default_checks()` as a per-file native checker.
- Must include transcript and real-world corpus backtests before ship.

## Acceptance criteria

1. A loop with 2+ accumulator groups that share no direct variable reference is flagged with a message like `[split-loop] loop advances independent accumulators X and Y — Fowler's Split Loop`.
2. A loop where every accumulator's update expression directly references at least one other accumulator is NOT flagged.
3. A loop with only one accumulator is NOT flagged.
4. A loop where one accumulator is derived from another (e.g. `sum += item.price; count += 1` where the final use divides sum by count) is flagged, since the dependency test only looks at direct variable references in RHS expressions — this is a documented, accepted false positive risk where the user suppresses with `kibitzer:ignore`.
5. The checker is wired into `default_checks()` for the same set of file globs as `syntax-rules-<lang>` for each supported language.
6. The checker ID follows the pattern `split-loop-<lang>` (e.g. `split-loop-go`, `split-loop-python`) with a shared internal implementation analogous to how `syntax-rules-<lang>` shares `rules.rs`.
7. Transcript backtest (`kibitzer check backtest split-loop-go`) must be run, findings reviewed, and results documented before ship. False-positive rate must be below 40% (the bar set in ADR-001).
8. Real-world corpus backtest (`kubernetes/kubernetes`, `tstapler/stapler-squad`) must be run and documented.
9. A `docs/split-loop-false-positives.md` page is created documenting known false-positive patterns.
10. `default_checks()` test count assertion in `config.rs` is updated.

## Out of scope

- Detecting loops with more than a simple range/for-each shape (while loops with complex condition mutations are deferred to v2).
- Cross-loop dependency analysis.
- Auto-refactoring (apply the Split Loop transformation automatically).
- Tracking accumulators through function calls.
