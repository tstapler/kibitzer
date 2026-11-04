# Requirements: conditional-logic-smells

Three new native per-file checks for `src/checkers/rules.rs`'s `SyntaxRulesChecker`
catalog. Each is a structural AST pattern rather than a pure metric and each maps to
a named Fowler refactoring.

## Background

The existing `SyntaxRulesChecker` in `src/checkers/rules.rs` already houses
`long-function`, `deep-nesting`, `long-parameter-list`, `flag-argument`,
`unreachable-code`, `replace-magic-literal`, and `extract-variable` — all sharing
the same `LangRuleConfig` per-language node-kind table and the same
`check_declaration` / `check_block_for_unreachable` / `walk_*` traversal
infrastructure. These three new rules fit the same mold.

## Rules

### 1. `decompose-conditional`

**Fowler refactoring:** Decompose Conditional
(https://refactoring.com/catalog/decomposeConditional.html)

A branch whose `condition` node's complexity score exceeds a threshold is flagged
with a suggestion to extract the condition into a named predicate function.

- Reuses `complexity.rs`'s per-node decision-point counter (already accessible
  within the crate: `complexity::count_decision_points` or the equivalent walker).
- Threshold: TBD from corpus backtesting, expected ~3–5 operators/calls within a
  single condition expression.
- Language scope: same multi-language set as `flag-argument` (Go, TS/JS, Python,
  Java, Kotlin, Rust) — each already has `if_kind` / `ternary_kind` wired into
  `LangRuleConfig`.
- Does NOT fire on a whole function's complexity — `file-complexity` already covers
  that. This fires per condition, not per function.

### 2. `consolidate-conditional`

**Fowler refactoring:** Consolidate Conditional Expression
(https://refactoring.com/catalog/consolidateConditionalExpression.html)

Two or more consecutive `if` statements in the same block each with a single-
statement body assigning/returning the **same** value should be combined into one
condition with `||`.

Detection criteria (all must hold):
- 2+ sibling `if` statements adjacent in the same block (no intervening non-if
  statement between the first and last of the run).
- Each has exactly one statement in its `consequence` body (no else branch).
- Each statement is either:
  - a `return` of the same literal/identifier value, or
  - an assignment to the same local variable of the same value.
- None of the conditions has side effects (conditions are pure expression subtrees
  with no assignment operator or call expression inside them — conservative
  approximation; `call_expression` nodes in a condition cause the finding to be
  suppressed to avoid false-positives).

Language scope: Go-first (most reliable from existing node-kind coverage), with TS/JS
and Python as Phase 2.

### 3. `remove-control-flag`

**Fowler refactoring:** Remove Control Flag (Replace Control Flag with Break)
(https://refactoring.com/catalog/removeControlFlag.html)

A boolean local variable used *only* to break out of a loop early — set once
to `true`/`false` inside the loop, checked only as the loop condition, never
read after the loop — is flagged with a suggestion to use a direct `break` or
`return`.

Detection criteria (all must hold):
- A local `var`/`let`/`:=` boolean variable declaration initialized to `false`
  (or `true`).
- Its only write sites are `= true` or `= false` assignments inside a single
  containing loop.
- It appears *only* in the loop's controlling condition expression (no other reads).
- It is not referenced after the loop's closing brace.
- Scope: Go-first; this check requires tracking variable read/write sites across a
  function body, which is feasible within the per-function traversal already done
  in `check_declaration`.

## Acceptance Criteria

1. Each rule is registered in `CATALOG` in `src/checkers/rules.rs` with
   `id`, `category`, `description`, and `default_severity = Severity::Advisory`.
2. Each rule is wired into `default_checks()` via `syntax_rules_checks()` in
   `src/config.rs`, matching the pattern of the existing seven rules.
3. `decompose-conditional`: fires on a Go function whose `if` condition nests
   ≥ threshold operators/calls; does not fire on a simple `if err != nil` or
   `if x > 0` guard.
4. `consolidate-conditional`: fires on 2+ consecutive `if-return-same-value` or
   `if-assign-same-value` runs; does not fire when an intervening statement breaks
   the run or when the conditions contain call expressions.
5. `remove-control-flag`: fires on a boolean local variable used only as a loop-
   exit flag; does not fire when the variable is also read in a non-loop context or
   assigned a non-boolean value.
6. Each rule is backtested with `kibitzer check backtest <name>` against the
   transcript corpus, and against the standard backtest-repo corpus
   (see `docs/backtest-repos.md`), before it ships. False-positive rate and
   before/after numbers are recorded in a `docs/<rule-name>-false-positives.md`
   file, matching the existing `docs/flag-argument-false-positives.md` pattern.
7. Unit tests for each rule are added to `src/checkers/rules.rs`'s `#[cfg(test)]`
   block, with at least one "should fire" and one "should not fire" case per rule.
8. No existing test passes are broken by the new code.
9. `decompose-conditional` does not fire on the same `if` node that `deep-nesting`
   already flags — the two checks target distinct problems (condition complexity vs.
   branch nesting depth), so deduplication is acceptable via different anchors but
   double-firing on a single node is noise.

## Depends On

- `decompose-conditional` depends on issue #32's complexity walker
  (`src/checkers/complexity.rs`) being accessible as an in-crate function — it
  already is (`complexity::count_decision_points` / the internal walkers used by
  `FileComplexityChecker`).
- `consolidate-conditional` and `remove-control-flag` depend only on the existing
  `rules.rs` infrastructure and have no external prerequisites.

## Out of Scope

- Multi-file / cross-function analysis.
- Auto-fix / refactoring engine integration (a separate sub-system).
- Languages beyond the stated Go-first / multi-language expansion plan.
- Detecting `decompose-conditional` in ternary expressions (follow-up).
