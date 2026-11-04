# Adversarial Review: conditional-logic-smells Implementation Plan

**Date**: 2026-11-03
**Reviewer**: Adversarial architecture review
**Verdict**: 3 blockers, 3 concerns, 3 minors — do not ship until blockers are resolved.

---

## Blockers

### B1: `condition_decision_count` counting semantics are wrong as sketched

**What the plan says**: Task 3.1.1a — "New function `condition_decision_count(node, cfg, src)`: counts decision points using `cfg.operator_kinds` and `cfg.ternary_kind`."

**What is actually in the codebase**: `cfg.operator_kinds` for Go (and every other grammar except Python) is `&["binary_expression"]` — a single node kind that covers ALL binary expressions: `&&`, `||`, `>`, `<`, `==`, `!=`, and arithmetic. Python splits into `"boolean_operator"` (and/or) plus `"comparison_operator"`.

**What the example implies**: Task 3.1.1a example says `x > 0 && y > 0 && z > 0` returns 2. That expression contains 5 `binary_expression` nodes (two `&&` and three `>`). If the function counts all nodes in `operator_kinds`, it returns 5, not 2. The example is only consistent with counting logical operators (`&&`/`||`) exclusively.

**Impact**: An implementation faithful to "using `cfg.operator_kinds`" would fire at threshold 5 on `if a > 0 && b > 0 && c > 0 && d > 0 { return nil }` — a plain four-clause guard with no readability problem. Every simple 3+ clause guard clause in Go would hit the threshold. The public corpus backtest would show overwhelming false-positive noise.

**What must change before this ships**: The function needs to filter `binary_expression` nodes to those whose operator token is `&&` or `||` (the same test `is_short_circuit` already applies in `complexity.rs`), OR a new `LangRuleConfig` field `logical_operator_kinds` maps only the logical connectives per language. The current plan description cannot produce the semantics its own examples demonstrate.

**Evidence**: `src/checkers/rules.rs` line 996 (`operator_kinds: &["binary_expression"]`); `src/checkers/complexity.rs` lines 222–223 (`is_short_circuit` checks the operator token to restrict to `&&`/`||`); plan Task 3.1.1a example showing count=2 for two `&&` operators.

---

### B2: `walk_consecutive_ifs` is placed before the generated-file guard

**What the plan says**: Task 2.1.2b — "In `SyntaxRulesChecker::check()`, add a call to `walk_consecutive_ifs` after `walk_blocks`."

**What is actually in the codebase**: The generated-file guard appears AFTER `walk_blocks` and BEFORE `walk_dense_expressions` (lines 1565–1577 of `src/checkers/rules.rs`):

```rust
walk_declarations(...)    // no generated guard
walk_blocks(...)          // no generated guard
let kind = FileKind::classify(file, ctx.source);
if kind.generated {
    return Ok(findings);  // early return here
}
walk_dense_expressions(...)   // guarded
```

Placing `walk_consecutive_ifs` "after `walk_blocks`" puts it before this guard. Generated Go files (`zz_generated.deepcopy.go`, `generated.pb.go`) routinely contain long chains of `if err != nil { return nil, err }` blocks. Suppression 2 would catch many of these, but not all patterns. The existing `walk_dense_expressions` guard was added specifically because generated-file findings from `kubernetes/kubernetes` dominated raw output — the same problem would recur for `consolidate-conditional` on generated files.

**What must change**: `walk_consecutive_ifs` must be placed after the generated-file guard (after line 1576), alongside `walk_dense_expressions`. This is a one-line placement fix, but the plan as written would produce a generated-file regression the moment it ships on a repo with protobuf output.

**Evidence**: `src/checkers/rules.rs` lines 1565–1577; plan comment "Scoped to just these 2 passes; the other 5 rules are untouched" in the existing code explains why the guard was placed there.

---

### B3: `consolidate-conditional` assignment case is absent from the implementation

**What the requirements say**: Requirements §Rule 2 — "each statement is either: a `return` of the same literal/identifier value, **or an assignment** to the same local variable of the same value."

**What the plan implements**: Task 2.1.1a — "That statement must be a `return_kind` node." The assignment case (two or more consecutive `if (cond) { flag = true }` blocks that could be combined) is not in the implementation at all — no mention in the helper design, no test cases, no GIVEN/WHEN/THEN for assignment paths.

**Impact**: The implemented check covers only half of what AC4 requires. Any real-world codebase with the assignment form of this smell gets no finding. The backtest against the public corpus would correctly pass (no FPs from the missing path), but ship with a gap the requirements explicitly named.

**What must change**: Either scope the implementation to return-only (update requirements to match) with a formal out-of-scope record, or implement the assignment detection case in `check_block_for_duplicate_returns` with its own should-fire test cases. The plan cannot silently drop a named acceptance criterion.

**Evidence**: `project_plans/conditional-logic-smells/requirements.md` lines 46–51; plan Task 2.1.1a step 3 ("That statement must be a `return_kind` node").

---

## Concerns

### C1: `decompose-conditional` should-fire test case contradicts its own threshold

**What the plan says**: Story 3.1.6a "Minimum two should-fire cases: 1. `if a && b && c && d && e { ... }` (exactly 4 `&&` = 4 decision points — fires at threshold 5 when count >= threshold; adjust if threshold changes)."

**The contradiction**: `a && b && c && d && e` has four `&&` operators. If `condition_decision_count` counts logical operators only (the corrected semantics from B1), this returns 4. The threshold is 5, and the condition for firing is `count >= DECOMPOSE_CONDITIONAL_THRESHOLD`. Since 4 < 5, this case does NOT fire — it is a should-not-fire case, not a should-fire case.

A correct should-fire case at threshold 5 needs five logical operators: `a && b && c && d && e && f` (five `&&`s). The parenthetical "adjust if threshold changes" does not resolve this — the threshold is not being changed, so the test case is wrong as stated.

**Impact**: An implementor writing this test verbatim will write a test that asserts a finding is emitted, then discover it isn't, and either lower the threshold to 4 (undocumented) or silently fix the test without noting the discrepancy. Either outcome changes behavior without audit trail.

---

### C2: `check_declaration` already violates `LONG_FUNCTION_LINES`; new code makes it worse without a plan

**What the codebase shows**: `check_declaration` is 54 lines (`src/checkers/rules.rs` lines 2021–2075). `LONG_FUNCTION_LINES = 40`. This function is 14 lines over threshold.

**What the plan adds to it** (Task 1.1.2b):
```rust
if let Some(body) = (cfg.body_finder)(decl) {
    let flags = collect_bool_flags(body, cfg, src);
    if !flags.is_empty() {
        emit_flag_findings(body, &flags, cfg, src, findings);
    }
}
```
That is 5 more lines, reaching 59 — 19 over threshold.

**The catch-22**: kibitzer checks this repository. Any file kibitzer processes will have `walk_declarations` run, which calls `check_declaration`, which will then emit a `[long-function]` finding against `src/checkers/rules.rs` itself — in every hook invocation. The tech debt disposition accepts this ("Accept: new logic goes in separate helpers, not inline; `check_declaration` itself does not grow"). But the plan adds 5 lines directly to `check_declaration`, so the function does grow. It already exceeds the threshold, so this would fire from the first committed edit.

**What must change**: Either add a `kibitzer:ignore long-function -- pre-existing, tracked in check_declaration tech-debt` comment before the function, or extract the existing `flag-argument` block into its own helper before adding new code, bringing `check_declaration` back under 40. The plan needs to prescribe one of these explicitly.

---

### C3: `consolidate-conditional` return-value text comparison is unspecified and fragile for multi-value returns

**What the plan says**: Task 2.1.1a step 3 — "Within a run, compare the return value text across all members."

**What tree-sitter returns**: For `return nil, err`, `utf8_text` on the return's value subtree gives the raw source text including spaces. Two otherwise-equivalent returns that differ only in whitespace (`return nil,err` vs `return nil, err`) will not compare equal.

**A more concrete problem**: For the guard-clause suppression (B3-adjacent): `return nil, err` and `return nil, err2` have different texts, so suppression 2 correctly fires per statement. But the consolidation check compares return texts *across the run*. If two statements return the same value but with different internal whitespace, the run won't be detected even though it should be flagged.

**What must change**: The plan needs to specify how return-value text is normalized before comparison — at minimum, collapsing runs of whitespace and stripping leading/trailing whitespace from each token. Alternatively, compare the return value's AST structure (node text normalized per-token) rather than the raw span text.

---

## Minors

### M1: No task creates the false-positive docs required by AC6

**What requirements say**: AC6 — "False-positive rate and before/after numbers are recorded in a `docs/<rule-name>-false-positives.md` file, matching the existing `docs/flag-argument-false-positives.md` pattern."

**What the plan provides**: Backtest tasks say "Record TP/FP counts" and "Document goroutine-capture FPs in `docs/false-positives.md` or a rule-specific doc." There is no task with a file deliverable. The backtest task is marked complete when FP rate < 40%, but AC6 requires a written doc regardless of whether the rate is below 40%.

**What must change**: Each backtest story (1.1.3b, 2.1.3b, 3.1.4b) needs an explicit subtask: "Create `docs/<rule-name>-false-positives.md` with TP/FP counts, threshold value, example FP patterns, and known limitations."

---

### M2: `decompose-conditional` fires on all language variants but only Go is backtested

**What the architecture implies**: `decompose-conditional` hooks into `walk_dense_expressions` inside `SyntaxRulesChecker::check()`. `SyntaxRulesChecker` is instantiated for every language in `Language::ALL` (Go, TypeScript, Tsx, JavaScript, Python, Java, Kotlin, Rust, Bash). The new `condition_decision_count` call runs for all of them.

**What is backtested**: Only the Go corpus. The public corpus includes `servo/servo` (Rust), `microsoft/vscode` (TypeScript), `denoland/deno` (TypeScript/Rust), `apache/cassandra` (Java). None of these are included in the Phase 3 backtest plan.

**Risk**: A threshold of 5 calibrated against Go patterns may produce unacceptable FP rates in TypeScript (which has ternaries, complex generics, and nullable chaining) or Java (which has long boolean conditions in Spring annotations). Shipping into `default_checks()` turns on the rule for all languages simultaneously.

**What must change**: Either scope `decompose-conditional` to Go-only for the first ship (consistent with how `file-complexity` launched), or add at least one non-Go corpus run (TypeScript + Java) to the backtest plan before wiring into `default_checks()`.

---

### M3: `remove-control-flag` loop-node kinds stated without grammar verification

**What the plan says**: Task 1.1.1c — "Write sites are `= true`/`= false` assignments inside a `for_statement` / `range_clause` body." The plan uses these node kind names without citing a `to_sexp()` or `node-types.json` verification.

**What the codebase standard requires**: Every other grammar-specific detail in `rules.rs` includes a "Verified against real `to_sexp()` output, not guessed by analogy" comment (e.g., lines 383–388, 530–533). The Go grammar's `for` loop family is non-trivial: `for_statement` wraps a `for_clause` (three-clause), `range_clause`, or bare `expression` (while-style), and the `range_clause` is a child not a sibling. Using the wrong node kind silently produces zero findings (no crash, no error — just a check that never fires).

**What must change**: Task 1.1.1a or 1.1.1c should include a step verifying Go loop node kinds via `to_sexp()` on representative snippets, with the results recorded as inline comments on the functions — matching the existing convention in `rules.rs`.

---

## Summary

| ID | Category | Description |
|----|----------|-------------|
| B1 | correctness | `condition_decision_count` counts all `binary_expression` nodes via `operator_kinds`, not just `&&`/`||` — produces false positives on every multi-clause guard |
| B2 | correctness | `walk_consecutive_ifs` placed before generated-file guard fires on protobuf/deepcopy output |
| B3 | missing-coverage | `consolidate-conditional` assignment case missing from implementation despite being in requirements AC4 |
| C1 | correctness | Should-fire test for `decompose-conditional` uses 4 `&&` operators at threshold 5; 4 < 5 so test would not fire |
| C2 | architecture | `check_declaration` already 14 lines over `LONG_FUNCTION_LINES`; plan adds 5 more without a `kibitzer:ignore` or pre-refactor |
| C3 | correctness | Return-value text comparison is unspecified — multi-value returns with whitespace variation will not match |
| M1 | missing-coverage | No task creates `docs/<rule-name>-false-positives.md` files required by AC6 |
| M2 | architecture | `decompose-conditional` fires on all languages but only Go corpus backtested; ship scope should match backtest scope |
| M3 | correctness | Loop node kind names (`for_statement`/`range_clause`) stated without `to_sexp()` verification, contrary to codebase convention |
