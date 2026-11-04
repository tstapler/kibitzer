# Validation Plan: conditional-logic-smells
**Date**: 2026-11-03

## Happy Path Scenario

Given a Go source file containing a loop-exit control flag, two consecutive single-statement if-returns with the same value, and an if-condition chaining five `&&` operators, when `SyntaxRulesChecker::check()` runs against that file, then it emits exactly one `[remove-control-flag]`, one `[consolidate-conditional]`, and one `[decompose-conditional]` finding, each citing the correct line, with no false positives on clean code.

## Requirement → Test Mapping

| Requirement | Test File | Test Name | Type | Scenario |
|-------------|-----------|-----------|------|----------|
| REQ-1 | `src/checkers/rules.rs` | `remove_control_flag_should_fire_when_bool_used_only_as_loop_exit_flag` | Unit (happy path) | `done := false` declared; set only inside `for`; read only in `for` condition; not referenced after loop |
| REQ-1 | `src/checkers/rules.rs` | `remove_control_flag_should_not_fire_when_flag_read_after_loop` | Unit (error/not-fire) | Same `done` variable read after the `for` loop in a subsequent `if done {…}` statement |
| REQ-1 | `src/checkers/rules.rs` | `remove_control_flag_should_fire_when_nested_loop_uses_flag` | Unit (happy path variant) | `found := false` inside outer `for`; inner `for` sets it; outer condition reads it; no post-loop use |
| REQ-1 | `src/checkers/rules.rs` | `remove_control_flag_should_not_fire_when_non_bool_assigned` | Unit (error/not-fire) | `count := 0` incremented inside loop — integer, not bool; must not fire |
| REQ-2 | `src/checkers/rules.rs` | `consolidate_conditional_should_fire_when_two_consecutive_if_return_same_value` | Unit (happy path) | Two consecutive `if cond1 { return true }` / `if cond2 { return true }` with no intervening statement |
| REQ-2 | `src/checkers/rules.rs` | `consolidate_conditional_should_not_fire_when_condition_contains_call_expression` | Unit (error/not-fire) | `if validate(x) { return true }` / `if validate(y) { return true }` — condition contains a call; suppression rule 1 |
| REQ-2 | `src/checkers/rules.rs` | `consolidate_conditional_should_not_fire_when_condition_references_return_variable` | Unit (error/not-fire) | `if err != nil { return nil, err }` / `if err2 != nil { return nil, err2 }` — condition var appears in return; guard-clause suppression rule 2 |
| REQ-2 | `src/checkers/rules.rs` | `consolidate_conditional_should_not_fire_when_intervening_statement_breaks_run` | Unit (error/not-fire) | `if cond1 { return true }` / `x := compute()` / `if cond2 { return true }` — the intervening assignment breaks the consecutive run |
| REQ-3 | `src/checkers/rules.rs` | `decompose_conditional_should_fire_when_condition_has_five_or_more_logical_operators` | Unit (happy path) | `if a && b && c && d && e && f {` — five `&&` operators in condition, threshold is 5 |
| REQ-3 | `src/checkers/rules.rs` | `decompose_conditional_should_not_fire_when_condition_has_fewer_than_five_logical_operators` | Unit (error/not-fire) | `if a && b && c && d {` — four `&&` operators, one below threshold |
| REQ-3 | `src/checkers/rules.rs` | `decompose_conditional_should_not_fire_on_err_nil_guard` | Unit (error/not-fire) | `if err != nil { return nil, err }` — no logical operators, must not fire |
| REQ-3 | `src/checkers/rules.rs` | `decompose_conditional_should_not_fire_on_simple_comparison` | Unit (error/not-fire) | `if x > 0 {` — no logical operators, must not fire |
| REQ-3 | `src/checkers/rules.rs` | `decompose_conditional_should_not_fire_on_return_expression_with_many_logical_operators` | Unit (error/not-fire, position guard) | `return a && b && c && d && e && f` — five `&&` but in a `return` expression, not a condition |

## UX Acceptance Tests

N/A — pure infrastructure checker.

## Test Stack

- **Unit**: Rust `cargo test`, test module in `src/checkers/rules.rs` (the `#[cfg(test)] mod tests` block already present in that file)
- **Integration**: End-to-end `SyntaxRulesChecker::check()` calls on synthesized Go source snippets via the existing `check_full(lang, ts_lang, src)` helper already present in the test module; findings filtered by rule ID string in `Finding::message` using the same pattern as `extract_variable_findings`/`magic_literal_findings`
- **E2E / UX**: N/A

## Coverage Targets and How to Measure

| Stack | Coverage command | Target |
|---|---|---|
| Rust | `cargo tarpaulin --out Stdout` | ≥80% line |

## Test Helper Pattern

Each new rule follows the same filter-helper pattern already established in the module. Three new private helpers — alongside `extract_variable_findings` and `magic_literal_findings` — isolate findings by rule ID:

```rust
fn control_flag_findings(lang: Language, ts_lang: tree_sitter::Language, src: &str) -> Vec<Finding> {
    check_full(lang, ts_lang, src)
        .into_iter()
        .filter(|f| f.message.contains("[remove-control-flag]"))
        .collect()
}

fn consolidate_findings(lang: Language, ts_lang: tree_sitter::Language, src: &str) -> Vec<Finding> {
    check_full(lang, ts_lang, src)
        .into_iter()
        .filter(|f| f.message.contains("[consolidate-conditional]"))
        .collect()
}

fn decompose_findings(lang: Language, ts_lang: tree_sitter::Language, src: &str) -> Vec<Finding> {
    check_full(lang, ts_lang, src)
        .into_iter()
        .filter(|f| f.message.contains("[decompose-conditional]"))
        .collect()
}
```

All test cases use `Language::Go` / `tree_sitter_go::LANGUAGE.into()` for the Go-first scope; the source strings are wrapped with `package m\nfunc f() {\n…\n}\n` to satisfy the Go parser.

## CATALOG Registration Check

One registration test verifies that all three new rule IDs appear in `CATALOG` and in `SyntaxRulesChecker::description()`:

| Test Name | Type | Scenario |
|-----------|------|----------|
| `description_includes_all_three_new_conditional_rules` | Unit | `SyntaxRulesChecker::new(Language::Go).description()` contains `"remove-control-flag"`, `"consolidate-conditional"`, and `"decompose-conditional"` |

## default_checks() Wiring Check

One existing test (`default_checks_still_registers_all_nine_syntax_rules_checkers`) locks the count of `syntax-rules-*` entries in `default_checks()`. It will need its count updated after wiring; no new test is required beyond confirming the existing test stays green after the update.

## Backtest Gate (Pre-ship)

Each rule requires a corpus backtest with FP < 40% before merging. The backtest procedure (`docs/backtesting.md`, `docs/backtest-repos.md`) runs:

1. `kibitzer check backtest <name>` against `~/.claude/projects/*/*.jsonl` transcript corpus
2. `kibitzer check native <name> <file>` piped from `find` over the fixed public-repo corpus (`kubernetes/kubernetes`, `apache/cassandra`, `servo/servo`, `BurntSushi/ripgrep`, `denoland/deno`, `microsoft/vscode`, `stapler-squad`)

Results are recorded in a new `docs/backtest-triage/<name>.jsonl` file (following the existing `replace-magic-literal.jsonl` model) and the before/after FP rate noted in the PR body. A result at or above 40% FP is a hard blocker for shipping that rule.
