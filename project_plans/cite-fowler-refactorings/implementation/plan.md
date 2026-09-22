# Implementation Plan: cite-fowler-refactorings

**Feature**: Name the matching Fowler *Refactoring* catalog entry (with a link) in the `long-function` and `deep-nesting` finding messages in `src/rules.rs`.
**Date**: 2026-09-22
**Status**: Ready for implementation
**ADRs**: None

---

## Domain Glossary
N/A — complexity 1, no new domain types introduced.

---

## Creative Pass — Approaches Considered

1. **Inline string literal edit inside the existing two `format!` calls** (chosen). Strength: zero new indirection, matches the existing `flag-argument` precedent (`src/rules.rs:865`) that already inlines "Fowler's Remove Flag Argument" directly in its `format!`. Weakness: the URL text is duplicated verbatim if a third rule ever wants the same catalog entry (non-issue today — no such rule exists).
2. **Rule-id → URL lookup table consulted by both `CATALOG` and the live message**. Strength: single source of truth for URLs. Weakness: `CATALOG` (`src/rules.rs:34-46`) is `#[allow(dead_code)]` and never read by `check_declaration` at runtime — a lookup table would be a second, disconnected source of truth relative to the actual `message` strings, the opposite of the stated goal (research/build-vs-buy.md).
3. **External constants module (e.g. `refactoring_links.rs` with `pub const EXTRACT_FUNCTION_URL`)**. Strength: names the URL as a reusable symbol. Weakness: each URL is used exactly once, in one `format!` call — a new module and import for a value used once is unwarranted ceremony for a two-line string edit.

Approach 1 wins and is recorded in Pattern Decisions below.

---

## Pattern Decisions

| Component | Pattern Chosen | Source | Alternative Rejected | Reason |
|-----------|---------------|--------|---------------------|--------|
| `long-function` / `deep-nesting` finding messages | Inline string literal inside existing `format!` call | `flag-argument` precedent, `src/rules.rs:865` | Rule-id → URL lookup table | `CATALOG` (`src/rules.rs:34-46`) is dead code (`#[allow(dead_code)]`), never consulted by `check_declaration`; a table would be a second source of truth disconnected from the live `message` strings (research/build-vs-buy.md §1) |
| `long-function` / `deep-nesting` finding messages | Inline string literal inside existing `format!` call | `flag-argument` precedent, `src/rules.rs:865` | External constants module for the two URLs | Each URL is used exactly once; a new module/import adds indirection with no reuse benefit for a two-`format!`-call change |

---

## Tech Debt Disposition

None identified — no architecture research was run for this Complexity-1 item; the touched code (`check_declaration` in `src/rules.rs`) is not flagged as a hotspot in any other available research.

---

## Migration Plan
N/A — complexity 1.

## Observability Plan
N/A — complexity 1.

## Risk Control
N/A — complexity 1.

## Unresolved Questions
None.

## Dependency Visualization

```
Task 1.1.1a (edit long-function message)  ─┐
Task 1.1.2a (edit deep-nesting message)   ─┼─→ Task 1.1.3a (cargo test rules::) ─→ Task 1.1.4a (re-verify URLs, ship)
                                            │
Task 1.2.1a (optional: CATALOG descriptions) ─┘  (independent, no ordering dependency on 1.1.x)
Task 1.3.1a (optional: docs/syntax-rules.md)  ─┘  (independent, no ordering dependency on 1.1.x)
```

---

## Phase 1: Cite Fowler Refactorings in Finding Messages

### Epic 1.1: Update the two live finding messages
**Goal**: `long-function` and `deep-nesting` findings name their matching Fowler refactoring and link the catalog page, with no change to detection logic.

#### Story 1.1.1: `long-function` message names Extract Function
**As a** kibitzer user unfamiliar with the Fowler catalog, **I want** the `long-function` finding to name "Extract Function" and link its catalog page, **so that** I can look up the mechanical steps to fix it.
**Acceptance Criteria**:
- The `long-function` finding message names "Extract Function" and links `https://refactoring.com/catalog/extractFunction.html`.
  - *Given* a Go function whose body spans 41 lines (over `LONG_FUNCTION_LINES = 40`), *When* `check_declaration` runs on its declaration node, *Then* the pushed `Finding.message` equals `"[long-function] body spans 41 lines (over 40) — consider Extract Function (https://refactoring.com/catalog/extractFunction.html)"`.
- No detection logic changes — threshold, trigger condition, and finding `line`/severity are unchanged (part of AC4, verified via Story 1.1.3).
**Files**: `src/rules.rs`

##### Task 1.1.1a: Edit the `long-function` `format!` string (~3 min)
- In `check_declaration` (`src/rules.rs:826-828`), change the `format!` body from:
  `"[long-function] body spans {body_lines} lines (over {LONG_FUNCTION_LINES}) — consider splitting it up"`
  to:
  `"[long-function] body spans {body_lines} lines (over {LONG_FUNCTION_LINES}) — consider Extract Function (https://refactoring.com/catalog/extractFunction.html)"`
- Do not touch `line`, the surrounding `if body_lines > LONG_FUNCTION_LINES` condition, or the `LONG_FUNCTION_LINES` constant (`src/rules.rs:11`).
- Files: `src/rules.rs`

#### Story 1.1.2: `deep-nesting` message names Replace Nested Conditional with Guard Clauses
**As a** kibitzer user unfamiliar with the Fowler catalog, **I want** the `deep-nesting` finding to name "Replace Nested Conditional with Guard Clauses" and link its catalog page, **so that** I can look up the mechanical steps to fix it.
**Acceptance Criteria**:
- The `deep-nesting` finding message names "Replace Nested Conditional with Guard Clauses" and links `https://refactoring.com/catalog/replaceNestedConditionalWithGuardClauses.html`.
  - *Given* a function body that nests if/for/switch 5 levels deep (over `MAX_NESTING_DEPTH = 4`), *When* `check_declaration` runs on its declaration node, *Then* the pushed `Finding.message` equals `"[deep-nesting] body nests 5 levels deep (over 4) — consider Replace Nested Conditional with Guard Clauses (https://refactoring.com/catalog/replaceNestedConditionalWithGuardClauses.html)"`.
- No detection logic changes — threshold, trigger condition, and finding `line`/severity are unchanged (part of AC4, verified via Story 1.1.3).
**Files**: `src/rules.rs`

##### Task 1.1.2a: Edit the `deep-nesting` `format!` string (~3 min)
- In `check_declaration` (`src/rules.rs:836-838`), change the `format!` body from:
  `"[deep-nesting] body nests {depth} levels deep (over {MAX_NESTING_DEPTH}) — consider extracting a function or inverting a condition"`
  to:
  `"[deep-nesting] body nests {depth} levels deep (over {MAX_NESTING_DEPTH}) — consider Replace Nested Conditional with Guard Clauses (https://refactoring.com/catalog/replaceNestedConditionalWithGuardClauses.html)"`
- Do not touch `line`, the surrounding `if depth > MAX_NESTING_DEPTH` condition, or the `MAX_NESTING_DEPTH` constant (`src/rules.rs:15`).
- Files: `src/rules.rs`

#### Story 1.1.3: Existing tests still pass unmodified
**As a** maintainer, **I want** the 29 existing `.contains("[long-function]")` / `.contains("[deep-nesting]")` call sites in `src/rules.rs`'s `#[cfg(test)]` modules to keep passing, **so that** the message change is verified as additive, not breaking.
**Acceptance Criteria**:
- All existing tests asserting on these two messages still pass with zero test-file edits.
  - *Given* the two `format!` edits from Tasks 1.1.1a and 1.1.2a are applied, *When* `cargo test rules::` is run, *Then* every test that calls `.message.contains("[long-function]")` or `.message.contains("[deep-nesting]")` passes, because both substrings still appear verbatim at the start of each message.
**Files**: `src/rules.rs`

##### Task 1.1.3a: Run `cargo test rules::` to confirm AC #5 (~2 min)
- Run `cargo test rules::` from the repo root.
- Confirm all tests pass with no failures and no test-file changes were needed.
- Files: none (verification only)

#### Story 1.1.4: URLs verified live immediately before shipping
**As a** maintainer, **I want** both catalog URLs re-verified as resolving right before ship, **so that** the shipped message doesn't link a dead page (per AC #3 and the pitfalls research's "re-verify at ship time" note — verified once already at research time, 2026-09-22).
**Acceptance Criteria**:
- Both URLs return a successful (non-404/non-error) response, verified immediately before ship.
  - *Given* the two edits from Tasks 1.1.1a/1.1.2a are complete, *When* `curl -sI https://refactoring.com/catalog/extractFunction.html` and `curl -sI https://refactoring.com/catalog/replaceNestedConditionalWithGuardClauses.html` are run, *Then* both return an HTTP status in the 200-299 range (matching the 200 OK confirmed at research time).
**Files**: none (external verification)

##### Task 1.1.4a: Re-verify both catalog URLs resolve (~2 min)
- Run `curl -sI https://refactoring.com/catalog/extractFunction.html | head -1` and `curl -sI https://refactoring.com/catalog/replaceNestedConditionalWithGuardClauses.html | head -1`.
- Confirm both show an HTTP `2xx` status line immediately before opening/merging the PR.
- Files: none (verification only)

---

### Epic 1.2 (optional/stretch): `CATALOG` description consistency — AC #6
**Goal**: Apply the same refactoring-name treatment to the inert `CATALOG` descriptions for consistency with the live messages. Explicitly optional per requirements AC #6 ("open question, see suggestions") and research/build-vs-buy.md §1 (no runtime consumer). Do only after Epic 1.1 is complete and confirmed passing.

#### Story 1.2.1: `CATALOG` entries for `long-function` and `deep-nesting` name their refactoring
**As a** future reader of `src/rules.rs`'s `CATALOG` constant (e.g. a future `kibitzer rules list` command), **I want** the descriptions to name the same refactorings as the live messages, **so that** the two sources of documentation stay consistent.
**Acceptance Criteria**:
- `CATALOG`'s `long-function` and `deep-nesting` `description` fields optionally name their refactoring, matching the live message treatment.
  - *Given* `RuleMeta { id: "long-function", ... }` at `src/rules.rs:36-40`, *When* its `description` field is read, *Then* it reads `"Function/method body spans more than 40 lines — consider Extract Function."` (or equivalent phrasing naming the refactoring).
**Files**: `src/rules.rs`

##### Task 1.2.1a: Edit `CATALOG` descriptions (~3 min, optional)
- In the `CATALOG` constant, update `description` for `long-function` (`src/rules.rs:38`, currently `"Function/method body spans more than 40 lines."`) to name "Extract Function".
- Update `description` for `deep-nesting` (`src/rules.rs:44`, currently `"Function/method body nests if/for/switch/select/func_literal more than 4 levels deep."`) to name "Replace Nested Conditional with Guard Clauses".
- These are plain `&'static str` fields, not `format!` calls — no interpolation to preserve.
- Files: `src/rules.rs`

---

### Epic 1.3 (optional/stretch): `docs/syntax-rules.md` consistency
**Goal**: Align the hand-maintained prose table with the new message treatment, matching the pattern already used by that table's `flag-argument`/`unreachable-code` rows. Explicitly optional — not required by any acceptance criterion (research/stack.md, research/build-vs-buy.md §2).

#### Story 1.3.1: `docs/syntax-rules.md` table rows name the refactoring
**As a** reader of `docs/syntax-rules.md`, **I want** the `long-function` and `deep-nesting` rows to name their refactoring like the `flag-argument`/`unreachable-code` rows already do, **so that** the doc table is internally consistent.
**Acceptance Criteria**:
- The `long-function` and `deep-nesting` rows in `docs/syntax-rules.md`'s rule table optionally name their refactoring.
  - *Given* `docs/syntax-rules.md:30`, currently `"Function/method body spans more lines than this."`, *When* the row is updated, *Then* its description clause names "Extract Function", matching the style of the `flag-argument` row's existing Fowler citation.
**Files**: `docs/syntax-rules.md`

##### Task 1.3.1a: Update `docs/syntax-rules.md` rows 30-31 (~3 min, optional)
- Edit `docs/syntax-rules.md:30` (`long-function` row) to name "Extract Function", matching the phrasing style already used in that table's `flag-argument`/`unreachable-code` rows.
- Edit `docs/syntax-rules.md:31` (`deep-nesting` row) to name "Replace Nested Conditional with Guard Clauses".
- Files: `docs/syntax-rules.md`
```
