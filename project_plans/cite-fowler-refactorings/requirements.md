# Requirements: cite-fowler-refactorings

## Source

Backlog item `b8c516f9-c21e-4698-8820-579db853a22e` — "Cite the matching Fowler
refactoring by name in existing check messages." No interactive ideation was run;
this document is derived directly from the item's description.

## Complexity

1 (quick task) — pure message-copy change in two `format!` calls, no new logic,
no new dependencies, no architecture/UX surface.

## Problem

Two existing native checks in `src/rules.rs` already detect exactly the smell two
*Refactoring* (Fowler) catalog entries target, but their finding messages don't say
so:

- `long-function` (`LONG_FUNCTION_LINES = 40`, `src/rules.rs:823-830`) is the trigger
  condition for **Extract Function**.
- `deep-nesting` (`MAX_NESTING_DEPTH = 4`, `src/rules.rs:832-840`) is the trigger
  condition for **Replace Nested Conditional with Guard Clauses**.

A user unfamiliar with the catalog gets a bare "body spans N lines" instead of a
named technique they can look up for the mechanical steps to fix it.

## Proposal (from the item)

Update both findings' `message` text to name the refactoring explicitly and link
the catalog page, e.g.:

- "consider Extract Function (https://refactoring.com/catalog/extractFunction.html)"
- "consider Replace Nested Conditional with Guard Clauses
  (https://refactoring.com/catalog/replaceNestedConditionalWithGuardClauses.html)"

No new detection logic. Pure message-copy change in `src/rules.rs`. The two URLs
are worth verifying resolve at ship time.

## Acceptance Criteria

1. The `long-function` finding message names "Extract Function" and links
   `https://refactoring.com/catalog/extractFunction.html`.
2. The `deep-nesting` finding message names "Replace Nested Conditional with Guard
   Clauses" and links `https://refactoring.com/catalog/replaceNestedConditionalWithGuardClauses.html`.
3. Both URLs return a successful (non-404/non-error) response, verified before ship.
4. No detection logic changes — thresholds (`LONG_FUNCTION_LINES`, `MAX_NESTING_DEPTH`),
   trigger conditions, and finding `line`/severity are unchanged.
5. All existing tests that assert on these two messages (`.contains("[long-function]")`,
   `.contains("[deep-nesting]")`) still pass — substring matches on the rule-id
   prefix survive appending more text after them.
6. `RuleMeta.description` for `long-function` and `deep-nesting` in the `CATALOG`
   constant (`src/rules.rs:34-46`) optionally gets the same refactoring-name
   treatment for consistency (open question, see suggestions).

## Out of scope

- Adding refactoring citations to any other check (`long-parameter-list`,
  `flag-argument`, `unreachable-code`, etc.) — not requested by this item.
- New detection logic or threshold changes.
- A general "link to a catalog" feature/config mechanism.

## Dependencies

None — this is the smallest standalone win in the backlog for this theme.
