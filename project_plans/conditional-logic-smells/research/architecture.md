# Architecture Research: conditional-logic-smells

Builds on two prior passes over `src/checkers/rules.rs`:
- `project_plans/hide-delegate-check/research/architecture.md`: covers `SyntaxRulesChecker`, the
  `LangRuleConfig` per-language table, the "third walk sibling to `walk_declarations`/`walk_blocks`"
  pattern, and the established precedent for adding rules without touching `config.rs`.
- `project_plans/replace-magic-literal/research/architecture.md`: documents the two-pass
  collect-then-emit walk shape (`walk_literals` → `emit_literal_findings`), the
  `collect_identifiers` generalization, and the third walk (`walk_literals`) called from `check()`.

This file does **not** re-derive those foundations; it extends them to three new rules with materially
different traversal and state requirements.

---

## Context: file has grown significantly since prior passes

Both prior architecture docs were written against `src/rules.rs` at ~2329 lines. The checker was
reorganized: rules.rs now lives at `src/checkers/rules.rs` and is **5390 lines** (verified:
`wc -l src/checkers/rules.rs`). Tests dominate the growth — the test module starts at line 2373,
so the implementation is ~2370 lines, nearly unchanged from the prior 2329-line snapshot. The
`check()` body now runs **four** walks (not two as `hide-delegate-check`'s doc describes):

```
walk_declarations(root, cfg, src, &mut findings);        // long-function, deep-nesting,
                                                          // long-parameter-list, flag-argument
walk_blocks(root, cfg, src, &mut findings);              // unreachable-code
walk_dense_expressions(root, cfg, src, &mut findings);   // extract-variable (added since prior pass)
walk_literals(root, cfg, src, &mut collector);           // replace-magic-literal (collect pass)
emit_literal_findings(&collector, cfg, &filter, &mut findings);  // replace-magic-literal (emit)
```

Generated files skip `walk_dense_expressions` and `walk_literals`/`emit_literal_findings`
(`src/checkers/rules.rs:1567-1592`). `CATALOG` now has 7 entries (line 150).

---

## 1. Integration point for each rule

### `decompose-conditional`

**Multiplex into `walk_dense_expressions`, which already extracts condition sub-nodes.**

`walk_dense_expressions` (`src/checkers/rules.rs:2210-2241`) already visits every if/elif/ternary
condition node and every `return` statement. For each branching node (`is_branching_node`,
line 2245) it extracts `node.child_by_field_name("condition")` and passes it to
`expression_nesting`. `decompose-conditional` needs the same condition sub-node but a different
scoring function (decision-point count, not operator-call nesting). The two checks are independent
in their computations and can run side-by-side in `walk_dense_expressions` with one `if let Some`
branch each. No new top-level walk is needed.

Concretely, in `walk_dense_expressions` (line 2224):
```rust
if let Some(condition) = dense.map(|(_, n)| n) {
    let nesting = expression_nesting(condition, ...);  // existing extract-variable
    let decisions = condition_decision_count(condition, cfg);  // new decompose-conditional
    if decisions >= DECOMPOSE_CONDITIONAL_THRESHOLD { ... }
}
```

This is the cheapest integration shape: zero new top-level walks, shares the already-extracted
condition node.

### `consolidate-conditional`

**New sibling walk `walk_consecutive_ifs`, called from `check()`.**

`check_block_for_unreachable` already iterates `container.named_children()` linearly
(`src/checkers/rules.rs:1914-1947`), but its control flow exits at the first terminal statement.
Folding `consolidate-conditional` into it would prevent early exit or require a secondary
accumulation pass inside the same function — a hidden coupling of two orthogonal traversal goals.
This is exactly the shape `replace-magic-literal`'s architecture note warned against when
considering folding `walk_literals` into `walk_blocks`.

The right shape is a dedicated `walk_consecutive_ifs(node, cfg, src, findings)` with whole-tree
recursion (a consecutive-if run can appear in any block, not just function bodies), called from
`check()` after `walk_blocks`. Each `block_kind` node gets an inner scan: iterate its
`statement_container`'s named children; maintain a `Vec<Node>` run of consecutive `if_kind`
nodes; flush and check the run when a non-if statement is encountered or the block ends.

### `remove-control-flag`

**New two-pass pair `collect_bool_flags` + `emit_flag_findings`, called from `check_declaration`.**

This rule is function-scoped (a boolean local that is only used as a loop-exit flag, inside one
function body). It fits the `check_declaration` pattern: function body is in scope, and the check
fires once per function declaration. However, it needs two passes:

- **Pass 1** (`collect_bool_flags`): walk the function body, collect all `bool` local variable
  declarations with their names and declaration node IDs.
- **Pass 2** (`emit_flag_findings`): walk the body again, for each collected name classify every
  use-site: assigned inside a loop only, tested only as a loop condition, never read for a
  non-exit purpose. Names that satisfy the flag-only pattern produce findings.

The two-pass pair is added as sibling helpers to `check_declaration` (not inside it) and called
from `check_declaration` after the existing long-function/deep-nesting/parameter checks. This
mirrors `replace-magic-literal`'s `walk_literals`/`emit_literal_findings` split applied at
function scope rather than file scope.

---

## 2. `decompose-conditional` condition scoring

**`complexity.rs`'s walker is not reusable here; `expression_nesting` is the right starting point.**

`complexity.rs::count_decision_points` is **Go-only** — `GoKind::IfStatement`,
`GoKind::ForStatement`, etc. are hardcoded (`src/checkers/complexity.rs:216-224`). Its comment at
line 204 explicitly says "Go-specific today … #49/#55 should design their own multi-language shape
rather than inherit this one."

`expression_nesting` (`src/checkers/rules.rs:2272-2328`) already operates on a condition sub-node,
uses `cfg.operator_kinds` and `cfg.call_kinds` (both already on `LangRuleConfig`), and is
cross-grammar. For `decompose-conditional`, the right adaptation is a simpler function:
`condition_decision_count(node, cfg)` that counts `&&`/`||` operator tokens and ternary-expression
nodes under the condition, without the chain-flattening logic `expression_nesting` applies. Chain
flattening (treating `a && b && c` as one level) is correct for `extract-variable` (a long flat
`&&` chain is readable); for `decompose-conditional` each `&&`/`||` is one more thing a reader
must hold in their head, so they should NOT be flattened — each one counts.

**Threshold**: `DECOMPOSE_CONDITIONAL_THRESHOLD = 3` is a reasonable conservative start (a
condition with 4 or more `&&`/`||`/ternary nodes). This aligns with this repo's established
precedent of "bump the threshold if corpus backtest shows noise" (see the `MAGIC_LITERAL_MIN_OCCURRENCES`
ADR comment at line 29). No corpus evidence yet exists in this repo for condition-complexity
specifically — the mandatory backtest against the real-world corpus
(`scripts/clone-backtest-repos.sh`, `docs/backtest-repos.md`) must be run before finalizing the
threshold, using `scripts/backtest-triage.py` to track true/false-positive rates. This threshold
is a starting hypothesis, not a pre-committed value.

No new `LangRuleConfig` fields are needed: `cfg.operator_kinds` already names
`binary_expression` for every grammar (which covers `&&`/`||`), and `cfg.ternary_kind` already
names the ternary kind. `condition_decision_count` can walk with just these two fields.

---

## 3. `consolidate-conditional` block scan

**Separate `walk_consecutive_ifs`; data structure: `Vec<Node>` current-run accumulator.**

As argued in §1, sharing `check_block_for_unreachable`'s walk is not the right choice — the
control-flow shapes are incompatible (early-exit-on-terminal vs. accumulate-and-check). A
dedicated `walk_consecutive_ifs` function recurses the whole tree and, at each `block_kind` node,
calls an inner `check_block_for_duplicate_returns` helper that iterates `statement_container`'s
named children.

The inner helper maintains a `Vec<Node>` for the current consecutive run of `if_kind` nodes.
When a non-if statement is encountered (or iteration ends), if `run.len() >= 2`, the helper checks
whether each if-node's body is a single-statement block matching the return-or-assignment pattern:

1. Extract each if-node's consequence via `if_branches` (line 2146; already cross-grammar, using
   `child_by_field_name("consequence")` with Kotlin positional fallback).
2. For each consequence block, extract its sole named child via `statement_container`.
3. Compare the raw source text of that child across all nodes in the run: if they're all
   identical (e.g. all `return nil` or all `x = 0`), emit a `consolidate-conditional` finding.

Using raw source text comparison (not an AST structural hash) is intentionally simple and
conservative: an identical text match across two single-statement if bodies is an unambiguous
consolidation opportunity. Structural-but-not-textual equivalences (e.g. `return nil` vs
`return (nil)`) are left for a follow-up; the false-negative rate on syntactically identical arms
is zero.

No new `LangRuleConfig` fields are needed: `block_kind`, `statement_container`, `if_kind` (via
`is_branching_node`), and `if_branches` cover everything. `return_kind` and `terminal_kinds` are
available if the check is later extended to assignments.

---

## 4. `remove-control-flag` variable tracking

**Two-pass approach inside `check_declaration`; reuse `collect_identifiers` for pass 2.**

A single-pass model — "is this identifier a boolean flag?" — cannot work because whether a
variable is "only used as a flag" can only be determined after all its use-sites have been seen.
This is the same two-pass argument that motivates `walk_literals`/`emit_literal_findings`
(`project_plans/replace-magic-literal/research/architecture.md §3`).

**Pass 1** (`collect_bool_flags`): walk the function body for local variable declaration nodes
whose type annotation or initializer signals a boolean (`bool`/`boolean`/`Boolean` depending on
grammar). Collect names and declaration-site node IDs. Per-language shape varies:
- Go: `short_variable_declaration` (`x := false`) or `var_declaration` with `bool` type
- TS/JS: `lexical_declaration` with `boolean` type annotation or `false`/`true` initializer
- Python: assigned `False`/`True` without further type context (high false-positive risk; may
  be scoped out for v1)

A new `LangRuleConfig` field is needed: `bool_local_finder: for<'a> fn(Node<'a>, &'a [u8]) -> Option<String>`
— given any statement-level node in a function body, returns `Some(var_name)` if it is a boolean
local variable declaration, `None` otherwise. This follows the established
`bool_param_finder: fn(Node, &[u8]) -> Vec<String>` pattern (line 274) applied to locals rather
than parameters.

**Pass 2** reuses `collect_identifiers` (`src/checkers/rules.rs:2107-2117`), which already walks
all identifier references. The collected `HashMap<String, usize>` is scanned: a boolean local
whose name appears only in loop-condition positions (inside a `nesting_kinds` node's condition
sub-tree) and assignment positions (not in call arguments or return values) is flagged. This
classification does not need a new identifier walk — `collect_condition_identifiers` (line 2085)
already shows the existing pattern for position-aware identifier scanning.

---

## 5. `LangRuleConfig` changes required

For each rule, the minimum new fields:

**`decompose-conditional`**: **No new fields.** `cfg.operator_kinds` (already `binary_expression`
in all 8 grammars) and `cfg.ternary_kind` (already present, `None` where the grammar has no
ternary) are sufficient for `condition_decision_count`. One new constant:
```rust
/// A branch condition containing this many decision-point operators/ternaries is flagged
/// by `decompose-conditional`. Subject to corpus calibration before landing.
const DECOMPOSE_CONDITIONAL_THRESHOLD: usize = 3;
```

**`consolidate-conditional`**: **No new fields.** `block_kind`, `statement_container`,
`if_kind` (not a struct field — used via `is_branching_node`'s `cfg.if_kind` check), and
`if_branches` (a free function at line 2146) cover all needed operations. One new constant:
```rust
/// Consecutive `if` stmts in one block each with an identical single-statement body —
/// flagged when at least this many exist.
const CONSOLIDATE_CONDITIONAL_MIN_RUN: usize = 2;
```

**`remove-control-flag`**: **One new field.**
```rust
/// Given a statement-level node in a function body, returns `Some(name)` if the node is a
/// boolean local variable declaration — Go `var`/`:=` with `bool` type, TS `let`/`const`
/// with `boolean` annotation or `true`/`false` initializer, etc. `None` for all other
/// statements. Mirrors `bool_param_finder`'s per-grammar shape applied to locals rather
/// than parameters.
bool_local_finder: for<'a> fn(Node<'a>, &'a [u8]) -> Option<String>,
```

Type: `for<'a> fn(Node<'a>, &'a [u8]) -> Option<String>` — same lifetime-annotated fn-pointer
form as `binding_finder` (line 321), not a bare `fn(Node, &[u8]) -> Option<String>`, because the
function may need to return a `String` derived from slicing `src`, which requires the `'a` link.
(If the return type stays a heap-allocated `String` and never borrows from `src`, a bare
`fn(Node, &[u8]) -> Option<String>` suffices — verify at implementation time.)

The new field also needs a row in `node_kind_literals_are_valid_for_their_grammar`
(`src/checkers/rules.rs:2547`), which currently validates all `LangRuleConfig` kind-list fields —
but `bool_local_finder` is a function pointer, not a kind list, so it does **not** get a row
there. No change needed to that test.

---

## 6. Disposition: Extend as-is

**Extend as-is.** The implementation core of `src/checkers/rules.rs` is ~2370 lines (lines 1–2372),
essentially unchanged from the prior 2329-line analysis — the file is 5390 lines total only
because the test module (~3000 lines starting at line 2373) grew with new check coverage.
Adding three rules costs two new top-level walk functions (`walk_consecutive_ifs`,
`check_declaration` two-pass helpers), one new `LangRuleConfig` field, and one multiplexed
`if` branch in `walk_dense_expressions` — none of these strains the established
`LangRuleConfig`-table / walk-dispatch / per-rule-helper pattern. The rule that most adds
implementation complexity (`remove-control-flag`) is scoped to `check_declaration` where
`flag-argument` already proved the pattern works. Extracting per-rule modules into a
`src/checkers/rules/` subdirectory is a legitimate future step once rule count clearly outgrows
single-file orientation, but it is not forced by this addition.

---

## Summary of concrete recommendations for plan.md

1. **`decompose-conditional`**: multiplex into `walk_dense_expressions` alongside `extract-variable`
   — same condition sub-node, new `condition_decision_count(node, cfg)` helper using
   `cfg.operator_kinds`/`cfg.ternary_kind`. Threshold `DECOMPOSE_CONDITIONAL_THRESHOLD = 3`
   pending corpus backtest. No new `LangRuleConfig` fields.

2. **`consolidate-conditional`**: new `walk_consecutive_ifs` sibling of `walk_blocks`, called from
   `check()`. Inner `check_block_for_duplicate_returns` accumulates consecutive `if_kind` runs
   via `Vec<Node>`, uses `if_branches`/`statement_container` to compare single-statement body
   texts. `CONSOLIDATE_CONDITIONAL_MIN_RUN = 2`. No new `LangRuleConfig` fields.

3. **`remove-control-flag`**: two-pass pair `collect_bool_flags`/`emit_flag_findings` added to
   `check_declaration`. One new `LangRuleConfig` field:
   `bool_local_finder: for<'a> fn(Node<'a>, &'a [u8]) -> Option<String>`.
   Pass 2 reuses existing `collect_identifiers` for identifier counting.

4. **`complexity.rs`'s `count_decision_points` is not reusable** — Go-only by design, as its
   own comment states. `decompose-conditional` builds its own cross-grammar
   `condition_decision_count` using `LangRuleConfig` fields already present.

5. **Disposition: Extend as-is** — implementation core is ~2370 lines; test growth is the
   dominant contributor to the 5390-line total. No structural refactor warranted before landing.
