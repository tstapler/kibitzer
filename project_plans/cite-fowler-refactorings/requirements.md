# Cite the matching Fowler refactoring by name in existing check messages

Backlog item: b8c516f9-c21e-4698-8820-579db853a22e

## Problem

`long-function` and `deep-nesting` (`src/rules.rs`) detect exactly the smells
Fowler's **Extract Function** and **Replace Nested Conditional with Guard
Clauses** target, but their finding messages don't say so — a user who
doesn't know the catalog gets no way to look up the fix.

## Scope

Pure message-copy change in `src/rules.rs`. No new detection logic, no
threshold changes.

1. `long-function` finding message names "Extract Function" and links
   `https://refactoring.com/catalog/extractFunction.html`.
2. `deep-nesting` finding message names "Replace Nested Conditional with
   Guard Clauses" and links
   `https://refactoring.com/catalog/replaceNestedConditionalWithGuardClauses.html`.
3. Both URLs return 2xx — verified via `curl -sI` before ship.
4. `LONG_FUNCTION_LINES`, `MAX_NESTING_DEPTH`, trigger conditions, and
   finding `line`/severity stay byte-identical.
5. All existing tests pass unmodified (29 call sites match on the
   `[long-function]`/`[deep-nesting]` prefix substring, not full message
   text, so appending text is safe).
6. Two new unit tests in `src/rules.rs`'s `mod tests`:
   `long_function_message_cites_extract_function`,
   `deep_nesting_message_cites_guard_clauses`.
7. (Optional) `CATALOG`'s `RuleMeta.description` for both ids also names the
   refactoring, matching the existing style already used for
   `flag-argument`/`unreachable-code` (`src/rules.rs:56,62`, "Fowler's
   Remove Flag Argument" / "Fowler's Remove Dead Code").

## Out of scope

- Any change to detection thresholds or trigger logic.
- Any other check's message.

## Verified facts (grounding, not assumptions)

- Message construction: `src/rules.rs:823-840` (`check_declaration`).
- Constants: `LONG_FUNCTION_LINES = 40` (`src/rules.rs:11`),
  `MAX_NESTING_DEPTH = 4` (`src/rules.rs:15`).
- `CATALOG` entries: `src/rules.rs:34-65`.
- All 29 test call sites assert `.contains("[long-function]")` or
  `.contains("[deep-nesting]")` — substring match, confirmed via
  `grep -n '\[long-function\]\|\[deep-nesting\]' src/rules.rs`.
- Both catalog URLs return `HTTP/1.1 200 OK` as of 2026-09-22
  (`curl -sI`, re-verify immediately before ship per AC#3).
