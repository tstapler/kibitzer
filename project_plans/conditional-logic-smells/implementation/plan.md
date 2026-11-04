# Implementation Plan: conditional-logic-smells

**Feature**: Three new conditional-logic smell checks (`decompose-conditional`, `consolidate-conditional`, `remove-control-flag`)
**Date**: 2026-11-03
**Status**: Ready for implementation
**ADRs**: ADR-001-condition-complexity-helper.md

---

## Problem Statement

**Problem**: AI coding agents and developers working in kibitzer-checked codebases frequently introduce conditional logic smells (overly complex conditions, consolidatable guards, control-flag loops) that the existing checker catalog does not catch. These patterns are named Fowler refactorings with clear structural signatures, making them well-suited to static AST analysis. The existing `extract-variable`, `deep-nesting`, and `flag-argument` rules already demonstrate that kibitzer's users find this category of feedback actionable.

---

## Domain Glossary

| Term | Definition | Notes |
|------|-----------|-------|
| `RuleMeta` | Static catalog entry holding `id`, `category`, `description`, `default_severity` | Declared in `src/checkers/rules.rs`; read by future `kibitzer rules list` |
| `CATALOG` | `&[RuleMeta]` constant listing every rule the `SyntaxRulesChecker` can emit | New rules must be appended here |
| `LangRuleConfig` | Per-language node-kind table the AST walk consults; zero heap, 27 `'static` fields | `rules.rs` line 236; new rules reuse existing fields; `bool_local_finder` field added for `remove-control-flag` |
| `lang_config(Language)` | Constructor returning the `LangRuleConfig` for a given language | `src/checkers/rules.rs`; called in `check()` |
| `walk_declarations` | Recursive tree walk that calls `check_declaration` on every `function_kinds` node | Entry point for per-function smells |
| `check_declaration` | 54-line function emitting `long-function`, `deep-nesting`, `long-parameter-list`, `flag-argument` | Currently at the `LONG_FUNCTION_LINES` self-enforcement limit; new logic goes in helpers |
| `walk_blocks` | Recursive tree walk that calls `check_block_for_unreachable` on every `block_kind` | Used for `unreachable-code` |
| `walk_dense_expressions` | Recursive walk firing on `if`/return expressions over the nesting threshold | Used for `extract-variable`; `decompose-conditional` hooks into this same condition-node path |
| `if_branches` | Returns `(consequence, alternative)` from an if-node using field names, falling back to positional | Shared by all conditional-walking code |
| `statement_container` | `fn(Node) -> Node` field: given a `block_kind`, returns the node whose named children are statements | Go's `block > statement_list` indirection |
| `collect_identifiers` | Recursive helper counting every `"identifier"` reference by name | Reused in pass 2 of `remove-control-flag` |
| `cyclomatic_complexity` | `pub(crate)` function in `complexity.rs` returning 1 + decision points for any sub-node | Accepts any `Node`; `decompose-conditional` calls it on the condition sub-node |
| `SubtestHandling` | Enum controlling whether `.Run(closure)` subtests are excluded from complexity counts | `decompose-conditional` uses `SubtestHandling::IncludeAll` on a condition sub-node |
| condition decision count | The number of logical short-circuit decision points in an if-condition sub-node, computed with a new cross-grammar helper `condition_decision_count` | Counts only `&&`/`||` operators (and language equivalents such as `and`/`or`) plus `cfg.ternary_kind`; does NOT count `>`, `!=`, or other comparison-only operators; implemented by walking `binary_expression` nodes and checking each node's operator token text against `&&`/`||` (not via `cfg.operator_kinds` which covers all binary operators) |
| `walk_consecutive_ifs` | New helper: recurse-and-collect consecutive `if_kind` sibling runs | Entry point for `consolidate-conditional`; called from `check()` after the generated-file guard (alongside `walk_dense_expressions`), not after `walk_blocks` — to avoid firing on generated files |
| `check_block_for_duplicate_returns` | New helper: scan one block's statement list for runs of 2+ consecutive `if`-return-same-value (return-only; assignment-to-same-variable is out of scope for this phase) | Inner logic for `consolidate-conditional` |
| `bool_local_finder` | New `LangRuleConfig` field: `fn(Node, &[u8]) -> Option<String>` returning the declared name if the node is a bool local variable declaration | Used only by `remove-control-flag` |
| `collect_bool_flags` | New helper: first pass, finds all bool local declarations in a function body | Returns `HashMap<name, declaration_node>` |
| `emit_flag_findings` | New helper: second pass over the loop-containing AST; emits `remove-control-flag` findings | Checks write-only-inside-loop, read-only-in-loop-condition, never-read-after-loop |
| guard-clause pattern | `if err != nil { return nil, err }` — consecutive `if` statements each returning the same error; a canonical false-positive target for `consolidate-conditional` | Suppressed by: skip when condition references the same variable as the return expression |
| FP bar | ADR-001: < 40% false-positive rate on the public corpus sample before a rule ships as `default_checks()` | Gates each rule independently |

---

## Pattern Decisions

| Component | Pattern Chosen | Source | Alternative Rejected | Reason |
|-----------|---------------|--------|---------------------|--------|
| `decompose-conditional` condition complexity | New cross-grammar `condition_decision_count` helper using token-text filtering on `binary_expression` nodes (`&&`/`||` only) plus `cfg.ternary_kind` | research/architecture.md | Call `cyclomatic_complexity` directly on condition node; use `cfg.operator_kinds` to match all binary operators | `cyclomatic_complexity` hardcodes Go `GoKind` variants; `cfg.operator_kinds` for Go covers all binary expressions including `>`, `!=`, `==` — not just logical short-circuit operators. Token-text filtering (check if operator child text is `&&` or `||`) matches `complexity.rs::count_decision_points`'s existing approach and counts only short-circuit operators. |
| `decompose-conditional` threshold | Start at 5 (5 decision points in one condition) | research/pitfalls.md §1 | Threshold 4 or 3 | Corpus backtest showed idiomatic multi-clause guards at ≤4; threshold 5 reduces kubernetes admission-controller noise |
| `consolidate-conditional` guard-clause suppression | Skip when condition references same variable as return expression (secondary suppression) | research/pitfalls.md §2 | Rely only on "no call expressions" suppression | `if err != nil { return nil, err }` has no call in condition but would still fire without secondary suppression |
| `consolidate-conditional` min run | `CONSOLIDATE_CONDITIONAL_MIN_RUN = 2` | requirements.md | Min run 3 | Two consecutive duplicate-return ifs is already the smell Fowler names; raising to 3 would miss common two-branch cases |
| `remove-control-flag` new `LangRuleConfig` field | Add `bool_local_finder: for<'a> fn(Node<'a>, &'a [u8]) -> Option<String>` | research/architecture.md | Thread a separate `GoKind`-based scanner outside `LangRuleConfig` | Go-first scope but field placeholder signals future multi-language extension; consistent with existing `bool_param_finder` pattern |
| `remove-control-flag` deferred-closure/goroutine suppression | Accept as a known FP class; document in false-positives doc | research/pitfalls.md §4 | Syntactic goroutine-capture detection | Syntactic capture detection is unreliable without data-flow analysis; document the limitation rather than over-suppress |
| `decompose-conditional` co-fire with `extract-variable` | Accept co-fire; treat as distinct diagnostics | requirements.md AC9; research/pitfalls.md §5 | Suppress `decompose-conditional` when `extract-variable` also fires | Precedent: `long-function` and `deep-nesting` co-fire by design; each diagnostic names a distinct refactoring |
| `consolidate-conditional` detection scope | Return-value only (Phase 2) | requirements.md Phase 2 scope | Return AND assignment-to-same-variable detection | Assignment detection requires tracking LHS variable identity across consecutive if-stmts; return detection already covers the dominant pattern; separate backtesting required for each sub-pattern per ADR-001 |
| Implementation order | `remove-control-flag` first, `consolidate-conditional` second, `decompose-conditional` third | research/build-vs-buy.md | Build all three in parallel | FP risk ascending order; condition-complexity helper needed by `decompose-conditional` can be validated against corpus before wiring |
| Integration site for new checks | Called from `check()` after existing passes, in separate helper functions | CLAUDE.md ("check_declaration is already 54 lines") | Inline into `check_declaration` | `check_declaration` is already at the `LONG_FUNCTION_LINES` self-enforcement limit |

---

## Tech Debt Disposition

| Area | Existing Issue | Disposition | Justification |
|------|---------------|-------------|---------------|
| `check_declaration` function length | 54 lines — at the `LONG_FUNCTION_LINES = 40` self-enforcement threshold, already flagged by kibitzer | Accept: new rule logic goes in separate top-level helpers (`collect_bool_flags`, `emit_flag_findings`), not inline. Adding the two dispatch calls in Task 1.1.2b brings the function to ~59 lines. To silence the self-enforcement `long-function` finding while the function is in an over-threshold state, add `// kibitzer:ignore long-function -- check_declaration is a dispatcher; individual rule logic is in helpers` above the function signature. Remove when the function is eventually refactored into smaller pieces. | Extending `check_declaration` with inline logic would violate the threshold this codebase enforces; the dispatcher pattern keeps individual rule logic isolated and testable |
| `count_decision_points` Go-only | Private function in `complexity.rs` hardcodes `GoKind` variants; not reusable for cross-grammar condition complexity | Add a new `condition_decision_count(node, cfg)` function in `rules.rs`; walk `binary_expression` nodes and count those whose operator token text is `&&` or `||` (the same token-text check `count_decision_points` already uses); also count `cfg.ternary_kind` nodes; `count_decision_points` is not modified | Token-text filtering (not `cfg.operator_kinds`) correctly selects only logical short-circuit operators, matching the existing pattern in `complexity.rs`; see ADR-001 |
| `scope.rs` scope-boundary-only tracking | No variable read/write data-flow; `remove-control-flag` must implement its own two-pass AST walk | Accept for this feature; two-pass walk is Go-first and isolated | Full data-flow analysis is out of scope; two-pass syntactic walk is sufficient for the targeted pattern |

---

## Migration Plan

N/A — no schema or data changes.

---

## Observability Plan

N/A — no new service boundaries; findings surface via existing kibitzer check output.

---

## Risk Control

- **Feature flag**: not gated — added to `default_checks()` on merge after each rule passes the FP bar.
- **Rollback procedure**: revert PR; remove from `default_checks()`.
- **Staged rollout**: full rollout on merge; ADR-001 FP bar (< 40%) gates each rule independently before wiring into `default_checks()`. Backtest tasks come before the wiring tasks in every phase.
- **Backtest enforcement**: The PR that adds a rule to `default_checks()` / `syntax_rules_checks()` MUST include the `docs/backtest-triage/<rule-name>.jsonl` file committed alongside it. Without the backtest triage file, the PR fails code review. The backtest task (1.1.3, 2.1.3, 3.1.4) is a hard prerequisite for the wiring task — the wiring task should not be started until the backtest file exists and shows FP < 40%.
- **Self-enforcement**: kibitzer checks this repo; new checker code will be checked by existing rules. Helper functions (`collect_bool_flags`, `emit_flag_findings`, `condition_decision_count`, etc.) must stay under `LONG_FUNCTION_LINES`. `check_declaration` itself will temporarily exceed the threshold after Task 1.1.2b; suppress with `// kibitzer:ignore long-function -- check_declaration is a dispatcher; individual rule logic is in helpers` above its signature (see Tech Debt Disposition).
- **Success metric**: Each shipped rule has a measured corpus FP rate < 40% (ADR-001 bar). Secondary: at least one rule appears in `kibitzer check backtest all` output on real transcript data before its next commit to master — confirming the rule actually fires on real code, not just on hand-constructed test cases.

---

## Unresolved Questions

1. **`decompose-conditional` threshold**: 5 is the pre-committed starting point. The `kibitzer check backtest decompose-conditional` and public-corpus tasks (Phase 1, Task 1.1.2b/c) will produce the definitive number. If the FP rate at threshold 5 is > 40%, raise to 6 and re-run before wiring into `default_checks()`.
2. **`consolidate-conditional` language scope**: Go-first per requirements. TS/Python wiring is a natural follow-up, not in scope here.
3. **`remove-control-flag` goroutine-capture FP**: Documented as a known limitation. Volume in the public corpus determines whether a future static-analysis upgrade is warranted.

---

## Dependency Visualization

```
Phase 1: remove-control-flag
  |
  +--> bool_local_finder field on LangRuleConfig (Go impl)
  +--> collect_bool_flags / emit_flag_findings helpers
  +--> CATALOG entry + default_checks() wiring (gated on backtest)

Phase 2: consolidate-conditional
  |
  +--> walk_consecutive_ifs + check_block_for_duplicate_returns helpers
  +--> guard-clause suppression (secondary: condition-var == return-var)
  +--> CATALOG entry + default_checks() wiring (gated on backtest)

Phase 3: decompose-conditional
  |
  +--> condition_decision_count cross-grammar helper (new, in rules.rs)
  +--> walk_dense_expressions hook (fires on condition sub-node)
  +--> DECOMPOSE_CONDITIONAL_THRESHOLD constant (starts at 5)
  +--> CATALOG entry + default_checks() wiring (gated on backtest)
```

**Wall-clock blocker**: Backtest tasks (1.1.3b, 2.1.3b, 3.1.4b) require `scripts/clone-backtest-repos.sh` to have been run. This is a wall-clock blocker (~30-60 min network transfer for the first clone; cached on subsequent runs). Pre-clone the repos before starting implementation to avoid blocking.

---

## Phase 1: `remove-control-flag`

### Epic 1.1: Implement and ship `remove-control-flag`

#### Story 1.1.1: `LangRuleConfig` field + Go bool-local finder (S)

##### Task 1.1.1a: Add `bool_local_finder` field to `LangRuleConfig` (XS)

**Files**: `src/checkers/rules.rs`

Add field `bool_local_finder: for<'a> fn(Node<'a>, &'a [u8]) -> Option<String>` to `LangRuleConfig`. Provide a no-op default for all existing language configs (a fn that always returns `None`), and implement the Go version that recognizes:
- `short_var_declaration` where the right-hand side is `true` or `false` (a `bool` init)
- `var_declaration > var_spec` where the type is `bool` or the value is `true`/`false`

The Go implementation must return `Some(name)` for a single-name, bool-typed or bool-valued declaration, `None` otherwise (multi-name declarations and non-bool types pass through as `None`).

**Given**: Go source `found := false`  
**When**: `bool_local_finder` is called on the `short_var_declaration` node  
**Then**: returns `Some("found")`

**Given**: Go source `var count int = 0`  
**When**: `bool_local_finder` is called on the `var_declaration` node  
**Then**: returns `None`

##### Task 1.1.1b: Implement `collect_bool_flags` helper (S)

**Files**: `src/checkers/rules.rs`

New function `collect_bool_flags(body: Node, cfg: &LangRuleConfig, src: &[u8]) -> HashMap<String, Node>`:
- Recurses into `body` (a function body node).
- Calls `(cfg.bool_local_finder)(node, src)` on every node.
- Returns map from variable name to its declaration node.

**Given**: Function body containing `done := false` and `count := 0`  
**When**: `collect_bool_flags` runs on that body  
**Then**: returns `{"done": <decl_node>}` (count not included)

##### Task 1.1.1c: Implement `emit_flag_findings` helper (M)

**Files**: `src/checkers/rules.rs`

New function `emit_flag_findings(body: Node, flags: &HashMap<String, Node>, cfg: &LangRuleConfig, src: &[u8], findings: &mut Vec<Finding>)`:

For each `name` in `flags`, the function performs a single AST walk over `body` to classify every occurrence of `name`:
- Write inside a `for_statement` / `range_clause` body: `= true` or `= false` only
- Read inside a `for_statement`/`range_clause` condition/range expression
- Any other read (outside loop, in a non-condition position)

Emit a `[remove-control-flag]` finding anchored at the declaration line if and only if:
- All writes are inside a loop
- All reads are in a loop condition
- No reads exist outside loops

**Given**: Go function with `done := false; for !done { ... done = true ... }` where `done` is never read after the loop  
**When**: `emit_flag_findings` runs  
**Then**: emits one finding for `done`

**Given**: Go function with `done := false; for !done { ... }; if done { ... }` (read after loop)  
**When**: `emit_flag_findings` runs  
**Then**: emits no finding (variable read in non-loop context)

#### Story 1.1.2: Wire `remove-control-flag` into checker pipeline (S)

##### Task 1.1.2a: Add `CATALOG` entry for `remove-control-flag` (XS)

**Files**: `src/checkers/rules.rs`

Append to `CATALOG`:
```
RuleMeta {
    id: "remove-control-flag",
    category: "design",
    description: "Boolean local variable whose only writes are `= true`/`= false` inside a loop and only reads are in the loop condition — Fowler's Remove Control Flag.",
    default_severity: Severity::Advisory,
}
```

The finding message emitted at runtime must follow this format:

```
[remove-control-flag] boolean variable `{name}` is used only as a loop-exit flag — replace with a direct `break` or `return` (Fowler's Remove Control Flag: https://refactoring.com/catalog/removeControlFlag.html)
```

where `{name}` is the variable name returned by `bool_local_finder`.

##### Task 1.1.2b: Call `collect_bool_flags`/`emit_flag_findings` from `check_declaration` (XS)

**Files**: `src/checkers/rules.rs`

Inside `check_declaration`, after the existing `flag-argument` block, add:
```rust
if let Some(body) = (cfg.body_finder)(decl) {
    let flags = collect_bool_flags(body, cfg, src);
    if !flags.is_empty() {
        emit_flag_findings(body, &flags, cfg, src, findings);
    }
}
```

**Self-enforcement note**: Adding these two calls brings `check_declaration` to approximately 59 lines, over the `LONG_FUNCTION_LINES = 40` threshold. kibitzer checks this repo; the `long-function` check will fire on the function itself. To silence this self-enforcement finding while the function is in an over-threshold state, add `// kibitzer:ignore long-function -- check_declaration is a dispatcher; individual rule logic is in helpers` above the `check_declaration` function signature. Remove the ignore directive when the function is eventually refactored into smaller pieces.

**Given**: Go file with a loop-exit-flag function  
**When**: `check_declaration` runs on that function  
**Then**: `findings` contains a `[remove-control-flag]` entry

#### Story 1.1.3: Backtest `remove-control-flag` (M)

##### Task 1.1.3a: Run `kibitzer check backtest remove-control-flag` (S)

Run `kibitzer check backtest remove-control-flag` against `~/.claude/projects/*/*.jsonl`. Record TP/FP counts, example FP patterns, and final rate in `docs/backtest-triage/` and in this plan's Unresolved Questions if rate > 40%.

**Acceptance**: FP rate < 40% before proceeding to Task 1.1.3b.

##### Task 1.1.3b: Run against public corpus (M)

Clone/pull backtest repos per `docs/backtest-repos.md`. Run `kibitzer check native remove-control-flag <file>` piped from `find` over each repo. Triage with `scripts/backtest-triage.py`. Document goroutine-capture FPs as the known limitation class. Record final numbers in `docs/backtest-triage/remove-control-flag.jsonl`.

**Acceptance**: FP rate < 40%; goroutine-capture FPs documented in `docs/false-positives.md` or a rule-specific doc.

#### Story 1.1.4: Wire `remove-control-flag` into `default_checks()` (XS)

##### Task 1.1.4a: Add `remove-control-flag` to `syntax_rules_checks()` (XS)

**Files**: `src/config.rs`

> **Backtest enforcement**: The PR that adds a rule to `default_checks()` / `syntax_rules_checks()` MUST include the `docs/backtest-triage/<rule-name>.jsonl` file committed alongside it. Without the backtest triage file, the PR fails code review. The backtest task (1.1.3) is a hard prerequisite for this task — do not start this task until the backtest file exists and shows FP < 40%.

The Go `syntax-rules-go` check already passes through to `SyntaxRulesChecker`, which routes to `check_declaration`. No new `Check` struct is needed — `remove-control-flag` rides in the existing `syntax-rules-go` check. Confirm the rule fires end-to-end via `kibitzer run <testfile>` or the relevant unit test.

**Acceptance**: `default_checks_ids_are_unique` test passes; `default_checks_still_registers_all_nine_syntax_rules_checkers` count test updated if its assertion changes.

#### Story 1.1.5: Unit tests for `remove-control-flag` (S)

##### Task 1.1.5a: Write should-fire tests (S)

**Files**: `src/checkers/rules.rs` (test module)

Minimum two should-fire cases:
1. Simple `done := false; for !done { done = true }` — classic loop-exit flag
2. Nested loop: flag written in inner loop, read in outer loop condition

**Given/When/Then**: each test calls `walk_declarations` on a parsed Go snippet and asserts `findings` contains exactly one `[remove-control-flag]` finding at the declared line.

##### Task 1.1.5b: Write should-not-fire tests (S)

**Files**: `src/checkers/rules.rs` (test module)

Minimum two should-not-fire cases:
1. Flag read after the loop (`if done { return }`)
2. Flag assigned a non-bool expression (should not be recognized by `bool_local_finder`)

---

## Phase 2: `consolidate-conditional`

### Out of Scope (Phase 2)

- **Assignment-to-same-variable detection** (requirements §Rule 2 / AC4 `if-assign-same-value` runs): detecting `if a { x = 0 } if b { x = 0 }` is deferred to a follow-up. This implementation covers **return-same-value only**. AC4 as written in requirements.md ("fires on 2+ consecutive if-return-same-value or if-assign-same-value runs") is partially implemented; the `if-assign-same-value` sub-pattern requires separate backtesting to validate its FP rate and is not included here.

### Epic 2.1: Implement and ship `consolidate-conditional`

#### Story 2.1.1: Implement `walk_consecutive_ifs` and `check_block_for_duplicate_returns` (M)

##### Task 2.1.1a: Implement `check_block_for_duplicate_returns` helper (M)

**Files**: `src/checkers/rules.rs`

New function `check_block_for_duplicate_returns(block: Node, cfg: &LangRuleConfig, src: &[u8], findings: &mut Vec<Finding>)`:

**Scope**: this function detects **return-same-value only**. Assignment-to-same-variable detection (e.g., `if a { x = 0 } if b { x = 0 }`) is out of scope for this phase — see Phase 2 Out of Scope section.

1. Obtain the statement list via `(cfg.statement_container)(block)`.
2. Collect consecutive `if_kind` siblings into runs:
   - A run member must have no `alternative` (no else branch).
   - A run member's consequence (via `if_branches`) must contain exactly one statement.
   - That statement must be a `return_kind` node.
3. Within a run, compare the return value text across all members.
4. For a run of `CONSOLIDATE_CONDITIONAL_MIN_RUN = 2` or more with identical return values, apply suppressions:
   - **Suppression 1**: Skip if any condition in the run contains a `call_kinds` node.
   - **Suppression 2**: Skip if any condition references the same identifier as the return expression (guard-clause pattern: `if err != nil { return err }`).

     > **Clarification**: "condition references the same identifier as the return expression" means: collect all `identifier` nodes in the condition sub-tree (via `collect_identifiers`); collect all `identifier` nodes in the return's value sub-tree (the `expression_list` child of `return_statement`); if the intersection is non-empty, suppress. This handles `if err != nil { return nil, err }` (identifier `err` appears in both). It does NOT suppress `if err1 != nil { return nil, err1 }` / `if err2 != nil { return nil, err2 }` consecutively — those have DIFFERENT return values by text and would not form the same run anyway.

5. Emit a `[consolidate-conditional]` finding anchored at the first `if` in the run.

**Given**: Go function with:
```go
if x > 10 { return true }
if x < -10 { return true }
```
**When**: `check_block_for_duplicate_returns` runs on the containing block  
**Then**: emits one `[consolidate-conditional]` finding at the first `if`

**Given**: Go function with:
```go
if err != nil { return nil, err }
if err2 != nil { return nil, err2 }
```
**When**: `check_block_for_duplicate_returns` runs (condition references same var as return)  
**Then**: emits no finding (guard-clause suppression 2)

##### Task 2.1.1b: Implement `walk_consecutive_ifs` (S)

**Files**: `src/checkers/rules.rs`

New function `walk_consecutive_ifs(node: Node, cfg: &LangRuleConfig, src: &[u8], findings: &mut Vec<Finding>)`:
- Recurses the whole tree.
- Calls `check_block_for_duplicate_returns` on every `block_kind` node.

**Given**: A Go file with the duplicate-return pattern nested inside a function  
**When**: `walk_consecutive_ifs` runs on the file root  
**Then**: the inner block's finding is emitted

#### Story 2.1.2: Wire `consolidate-conditional` into checker pipeline (XS)

##### Task 2.1.2a: Add `CATALOG` entry for `consolidate-conditional` (XS)

**Files**: `src/checkers/rules.rs`

Append to `CATALOG`:
```
RuleMeta {
    id: "consolidate-conditional",
    category: "design",
    description: "2+ consecutive if statements each returning the same value with no else — Fowler's Consolidate Conditional Expression.",
    default_severity: Severity::Advisory,
}
```

The finding message emitted at runtime must follow this format:

```
[consolidate-conditional] {n} consecutive if statements each return `{value}` — combine into a single condition (Fowler's Consolidate Conditional Expression: https://refactoring.com/catalog/consolidateConditionalExpression.html)
```

where `{n}` is the run length and `{value}` is the shared return value text.

##### Task 2.1.2b: Call `walk_consecutive_ifs` from `check()` (XS)

**Files**: `src/checkers/rules.rs`

In `SyntaxRulesChecker::check()`, add a call to `walk_consecutive_ifs` **after the generated-file guard** (alongside `walk_dense_expressions`), NOT after `walk_blocks`. Placing it before the guard would fire on protobuf/deepcopy generated output. The correct order:

```rust
// In check():
walk_declarations(...);
walk_blocks(...);
// Generated-file guard HERE:
let kind = FileKind::classify(file, ctx.source);
if kind.generated {
    return Ok(findings);
}
walk_dense_expressions(tree.root_node(), &cfg, src, &mut findings);
walk_consecutive_ifs(tree.root_node(), &cfg, src, &mut findings);  // <-- after the guard
walk_literals(...);
```

Scope to Go-only by convention (helper uses `cfg.if_kind`/`cfg.block_kind`/`cfg.call_kinds`), not by an explicit language guard — the cross-grammar config fields make it correct for all languages even though Go is the primary target.

#### Story 2.1.3: Backtest `consolidate-conditional` (M)

##### Task 2.1.3a: Run `kibitzer check backtest consolidate-conditional` (S)

Same procedure as Task 1.1.3a. Record TP/FP counts. Expected dominant FP class: guard-clause patterns not caught by suppression 2. If a new suppression pattern emerges, implement it before Task 2.1.3b.

**Acceptance**: FP rate < 40%.

##### Task 2.1.3b: Run against public corpus (M)

Same procedure as Task 1.1.3b. Document any residual guard-clause FPs in `docs/false-positives.md` or a rule-specific doc.

**Acceptance**: FP rate < 40%.

#### Story 2.1.4: Wire `consolidate-conditional` into `default_checks()` (XS)

##### Task 2.1.4a: Confirm `consolidate-conditional` fires in `syntax-rules-go` (XS)

> **Backtest enforcement**: The PR that adds a rule to `default_checks()` / `syntax_rules_checks()` MUST include the `docs/backtest-triage/<rule-name>.jsonl` file committed alongside it. Without the backtest triage file, the PR fails code review. The backtest task (2.1.3) is a hard prerequisite for this task — do not start this task until the backtest file exists and shows FP < 40%.

Same pattern as Task 1.1.4a: `consolidate-conditional` rides in the existing `syntax-rules-go` check via `walk_consecutive_ifs` being called in `check()`. Confirm end-to-end via unit test or `kibitzer run`.

#### Story 2.1.5: Unit tests for `consolidate-conditional` (S)

##### Task 2.1.5a: Write should-fire tests (S)

Minimum two should-fire cases:
1. Two consecutive `if-return-true` with no call in condition, no overlapping vars
2. Three consecutive `if-return-nil` (run of 3)

##### Task 2.1.5b: Write should-not-fire tests (S)

Minimum two should-not-fire cases:
1. Guard-clause pattern: `if err != nil { return nil, err }` back-to-back (suppression 2)
2. Intervening non-if statement between the two if-return blocks (not a consecutive run)

---

## Phase 3: `decompose-conditional`

### Epic 3.1: Implement and ship `decompose-conditional`

#### Story 3.1.1: Implement `condition_decision_count` cross-grammar helper (S)

##### Task 3.1.1a: Add `condition_decision_count` to `rules.rs` (S)

**Files**: `src/checkers/rules.rs`

> **Step -1 (check for existing constant):** Before writing `condition_decision_count`, check `src/checkers/rules.rs` around line 2252 for an existing `LOGICAL_OPERATORS` constant (or similar). If it exists, reuse it rather than duplicating the constant. If it doesn't exist, define the constant once and reference it from both `walk_dense_expressions` and `condition_decision_count`.

> **Step 0 (verify before writing any code):** Parse `package main\nfunc f() bool { return a && b }` with the Go tree-sitter grammar. Call `node.to_sexp()` on the `binary_expression` node for `a && b`. Verify that the `&&` operator appears as a named child node whose `utf8_text()` returns `"&&"`. If instead `&&` is encoded in the node type string (e.g. `binary_expression[&&]`), adjust the implementation to match the actual grammar shape. This `to_sexp()` verification follows the established codebase convention documented in `src/checkers/rules.rs` line 67-71.

New function `condition_decision_count(node: Node, cfg: &LangRuleConfig, src: &[u8]) -> usize`:
- Counts **logical short-circuit decision points** in `node`'s subtree. Does NOT use `cfg.operator_kinds` — that field covers all binary operators including `>`, `!=`, `==`, which must not be counted.
- Walk `binary_expression` nodes; for each one, check if its operator token text is `&&` or `||` (Go/Rust/Java/TS/JS/Kotlin) or `and`/`or` (Python); count only those.
- Also count each `cfg.ternary_kind` node as 1 decision point.
- Does NOT call `cyclomatic_complexity` (which is Go-only via `GoKind`).
- Recurse into all children, summing counts.
- Returns the total (0 for a simple expression like `x != nil`).
- NOTE: `x > 0 && y > 0 && z > 0` contains 2 `&&` operators and 2 `>` operators — `condition_decision_count` returns 2 (counts only `&&`, not `>`).

**Given**: Go condition node for `x > 0 && y > 0 && z > 0` (two `&&` operators, two `>` comparisons)  
**When**: `condition_decision_count` runs  
**Then**: returns 2 (the two `&&` operators; the two `>` comparisons are NOT counted)

**Given**: Go condition node for `err != nil` (single comparison, no short-circuit operator)  
**When**: `condition_decision_count` runs  
**Then**: returns 0

##### Task 3.1.1b: Add `DECOMPOSE_CONDITIONAL_THRESHOLD` constant (XS)

**Files**: `src/checkers/rules.rs`

```rust
/// An `if`/ternary condition whose `condition_decision_count` reaches this is flagged
/// by `decompose-conditional`. Starting value from corpus backtest (research/pitfalls.md §1);
/// confirmed by Task 3.1.3a backtest.
const DECOMPOSE_CONDITIONAL_THRESHOLD: usize = 5;
```

#### Story 3.1.2: Wire `decompose-conditional` into `walk_dense_expressions` (S)

##### Task 3.1.2-pre: Define `is_branching_condition_position` helper (XS)

**Files**: `src/checkers/rules.rs`

New private function `is_branching_condition_position(node: Node, cfg: &LangRuleConfig) -> bool` that returns true only when `node` is an `if_statement`/`if_expression` or ternary expression (i.e., the containing node kind is in `cfg.if_kind` or `cfg.ternary_kind`), and not a `return_statement`. Used by `walk_dense_expressions` to guard `decompose-conditional` from firing on return expressions. Must be defined before Task 3.1.2a, which references it.

**Given**: A node whose parent is an `if_statement`  
**When**: `is_branching_condition_position` is called  
**Then**: returns `true`

**Given**: A node whose parent is a `return_statement`  
**When**: `is_branching_condition_position` is called  
**Then**: returns `false`

##### Task 3.1.2a: Emit `decompose-conditional` finding in `walk_dense_expressions` (S)

**Files**: `src/checkers/rules.rs`

**Position constraint**: `decompose-conditional` MUST fire ONLY when the dense expression comes from an `if`/ternary **condition** position (the `is_branching_node` check produces a `condition` sub-node). It MUST NOT fire when the dense expression comes from a `return` statement. `walk_dense_expressions` processes both `if`-condition sub-nodes AND `return` expression nodes; without the position guard, a `return a && b && c && d && e && f` statement would be scored and would fire as a false positive on idiomatic code.

In `walk_dense_expressions`, when `is_branching_node(node, cfg)` is true and a condition sub-node is extracted, add a second check alongside the existing `extract-variable` check. Guard the `decompose-conditional` check with `is_branching_condition_position` so it runs only for the if/ternary condition node, not for return statement expressions:

```rust
// Only fire decompose-conditional on condition sub-nodes, not return expressions
if is_branching_condition_position(&dense, cfg) {
    let cond_decisions = condition_decision_count(condition_node, cfg, src);
    if cond_decisions >= DECOMPOSE_CONDITIONAL_THRESHOLD {
        findings.push(Finding {
            line: node.start_position().row + 1,
            message: format!(
                "[decompose-conditional] condition has {cond_decisions} decision points \
                 (at or over {DECOMPOSE_CONDITIONAL_THRESHOLD}) — consider Fowler's Decompose Conditional \
                 (https://refactoring.com/catalog/decomposeConditional.html)"
            ),
        });
    }
}
// extract-variable fires on BOTH conditions AND return expressions (existing behavior unchanged)
```

Where `is_branching_condition_position` returns true only for the if/ternary condition node (i.e., when the containing node kind is in `cfg.if_kind` or `cfg.ternary_kind`), not for return statement expressions.

Both `extract-variable` and `decompose-conditional` may fire on the same if/ternary condition node; this is intentional (see Pattern Decisions). `extract-variable` retains its existing behavior of firing on both conditions and return expressions.

**Given**: Go `if` with condition `a && b && c && d && e && f` (5 `&&` operators = 5 decision points = threshold)  
**When**: `walk_dense_expressions` runs  
**Then**: emits `[decompose-conditional]` finding; also emits `[extract-variable]` only if expression nesting also exceeds `EXTRACT_VARIABLE_NESTING_THRESHOLD` (independent check)

**Given**: Go `if err != nil { return err }` (0 decision points)  
**When**: `walk_dense_expressions` runs  
**Then**: no `[decompose-conditional]` finding

**Given**: Go `if x > 0` (0 decision points)  
**When**: `walk_dense_expressions` runs  
**Then**: no `[decompose-conditional]` finding

**Given**: Go `return a && b && c && d && e && f` (5 `&&` operators = 5 decision points, but in a return statement, not a branch condition)  
**When**: `walk_dense_expressions` runs  
**Then**: no `[decompose-conditional]` finding (position is a return expression, not a branching condition; `is_branching_condition_position` returns false)

#### Story 3.1.3: Add `CATALOG` entry for `decompose-conditional` (XS)

##### Task 3.1.3a: Append to `CATALOG` (XS)

**Files**: `src/checkers/rules.rs`

```
RuleMeta {
    id: "decompose-conditional",
    category: "complexity",
    description: "An if/ternary condition contains 5+ decision points (&&/||/ternary) — Fowler's Decompose Conditional.",
    default_severity: Severity::Advisory,
}
```

The finding message emitted at runtime must follow this format (see also the `format!` snippet in Task 3.1.2a):

```
[decompose-conditional] condition has {cond_decisions} decision points (at or over {DECOMPOSE_CONDITIONAL_THRESHOLD}) — consider Fowler's Decompose Conditional (https://refactoring.com/catalog/decomposeConditional.html)
```

#### Story 3.1.4: Backtest `decompose-conditional` (M)

##### Task 3.1.4a: Run `kibitzer check backtest decompose-conditional` (S)

Same procedure as Task 1.1.3a. Expected dominant FP class: multi-clause nil-guard chains in generated code (already filtered by `kind.generated` in `check()`). If FP rate is > 40% at threshold 5, raise constant to 6 and re-run.

**Acceptance**: FP rate < 40%; threshold value locked.

##### Task 3.1.4b: Run against public corpus (M)

Same procedure as Task 1.1.3b. Focus on kubernetes/kubernetes (admission controller, nil-check patterns). Document any generated-file leakage (should not exist — generated-file guard is already applied).

**Acceptance**: FP rate < 40%.

#### Story 3.1.5: Wire `decompose-conditional` into `default_checks()` (XS)

##### Task 3.1.5a: Confirm `decompose-conditional` fires in `syntax-rules-go` (XS)

> **Backtest enforcement**: The PR that adds a rule to `default_checks()` / `syntax_rules_checks()` MUST include the `docs/backtest-triage/<rule-name>.jsonl` file committed alongside it. Without the backtest triage file, the PR fails code review. The backtest task (3.1.4) is a hard prerequisite for this task — do not start this task until the backtest file exists and shows FP < 40%.

`decompose-conditional` fires via `walk_dense_expressions` inside the existing `syntax-rules-*` checks. No new `Check` struct needed. Confirm end-to-end via unit test or `kibitzer run`.

#### Story 3.1.6: Unit tests for `decompose-conditional` (S)

##### Task 3.1.6a: Write should-fire tests (S)

Minimum two should-fire cases:
1. `if a && b && c && d && e && f { ... }` (exactly 5 `&&` operators = 5 decision points — fires at threshold 5 since count ≥ threshold; `a && b && c && d && e` has only 4 `&&`s = 4 decision points and would NOT fire)
2. Ternary/language-specific equivalent at or over threshold (TypeScript/Java)

##### Task 3.1.6b: Write should-not-fire tests (S)

Minimum two should-not-fire cases:
1. `if err != nil { return }` (0 decision points)
2. `if x > 0 { ... }` (0 decision points)

---

## Cross-cutting: Existing test suite

After each phase's implementation tasks, run `cargo test` to confirm no regressions. Any count-assertion test in `config.rs` that checks `default_checks().len()` must be updated if `syntax_rules_checks()` or other helpers change. Verify with:

```
cargo test default_checks
```

This covers `default_checks_ids_are_unique`, `default_checks_still_registers_all_nine_syntax_rules_checkers`, and related count assertions in `config.rs`.

---

## Summary Counts

| Level | Count |
|-------|-------|
| Phases | 3 |
| Epics | 3 |
| Stories | 15 |
| Tasks | 28 |
| Glossary terms | 18 |
