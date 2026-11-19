# ADR-001: Cross-Grammar Condition Complexity Helper for `decompose-conditional`

**Date**: 2026-11-03  
**Status**: Accepted  
**Deciders**: planning subagent (triage)

---

## Context

`decompose-conditional` needs to count decision points in an if-condition sub-node. The
existing `cyclomatic_complexity` function in `src/checkers/complexity.rs` does exactly this —
but its inner `count_decision_points` function is Go-only: it hardcodes `GoKind` enum variants
(`GoKind::IfStatement`, `GoKind::BinaryExpression`, etc.) and is therefore not callable for
TypeScript, Python, Java, Kotlin, Rust, or Bash without a grammar-specific rewrite.

`LangRuleConfig` already carries `operator_kinds` (the `&&`/`||` equivalents for each grammar)
and `ternary_kind` (the ternary-expression kind, or `None` where the grammar has no ternary).
These are the two sets of decision-point contributors that matter for a condition sub-node
(loop-construct and case-label decision points don't appear inside an `if` condition).

## Decision

Add a new `condition_decision_count(node: Node, cfg: &LangRuleConfig, src: &[u8]) -> usize`
function in `src/checkers/rules.rs` that:

1. Counts nodes whose `kind()` is in `cfg.operator_kinds` (each such node contributes 1).
2. Counts nodes whose `kind()` matches `cfg.ternary_kind` if `Some` (each contributes 1).
3. Recurses into all children.
4. Returns the total.

This function does NOT call `cyclomatic_complexity` or `count_decision_points`. It is not
placed in `complexity.rs` because it is grammar-agnostic via `LangRuleConfig`, while
`complexity.rs` is explicitly Go-specific (see its own doc comment: "Go-specific today —
#49/#55 should design their own multi-language shape rather than inherit this one").

## Alternatives Rejected

**Call `cyclomatic_complexity` on the condition sub-node directly**: Would work for Go only.
The function accepts any `Node` (not restricted to function declarations), so the call itself
would compile — but `count_decision_points`'s `GoKind` match would silently return 0 for
every non-Go operator node, giving incorrect zero counts on TypeScript/Python/Java.
Rejected: silent language-scope regression, not a compile-time error.

**Move `count_decision_points` to `rules.rs` and thread `cfg`**: Possible but touches
`complexity.rs`'s Go-specific logic more than necessary, risks subtlety around
`is_short_circuit`'s `&&`/`||` raw-string comparison (which already does what
`cfg.operator_kinds` does, just for Go). Rejected: unnecessary coupling.

## Consequences

- `condition_decision_count` is a ~15-line function; no new `LangRuleConfig` fields are needed.
- `decompose-conditional` fires on all `SyntaxRulesChecker`-supported languages once their
  `operator_kinds` are set — Go, TypeScript, TSX, JavaScript, Python, Java, Kotlin, Rust, Bash.
- `complexity.rs` is not touched.
- Future callers that need cross-grammar condition complexity reuse `condition_decision_count`.
