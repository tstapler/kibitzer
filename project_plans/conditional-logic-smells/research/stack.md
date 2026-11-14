# Technology Stack Research: Conditional Logic Smell Checks

Sources: `codegen/node-types/go.json`, `src/checkers/complexity.rs`, `src/checkers/rules.rs`,
`src/scope.rs`, `src/declarations.rs`, `src/symbol_extract.rs`.

---

## 1. tree-sitter node kinds (Go, verified from `codegen/node-types/go.json`)

### `if` statement

Node kind: `if_statement`

Fields:
- `condition` (required): type `_expression` — this is the sub-node to score for
  `decompose-conditional`
- `consequence` (required): type `block`
- `alternative` (optional): type `block` or `if_statement` (chained `else if`)
- `initializer` (optional): type `_simple_statement`

### Loop constructs

**`for_statement`** — the umbrella node kind (line 1013). Fields: `body: block`.
Children (optional, mutually exclusive): `_expression` (condition-only loop),
`for_clause`, `range_clause`.

**`for_clause`** (line 977) — C-style `for init; cond; post`. Fields: `condition`
(`_expression`, optional), `initializer` (`_simple_statement`, optional), `update`
(`_simple_statement`, optional).

**`range_clause`** (line 1820) — `for k, v := range x`. Fields: `left`
(`expression_list`, optional), `right` (`_expression`, required).

### Boolean variable declarations

**`short_var_declaration`** (line 1973) — `:=`. Fields: `left: expression_list`,
`right: expression_list`. (Note: the node-types file spells it `short_var_declaration`,
not `short_var_decl` — verify callers use the correct spelling.)

**`var_declaration`** (line 2508) — `var x bool`. No fields; children are one
`var_spec` or `var_spec_list`. `var_spec` (line 2527) has fields: `name`
(`identifier`, multiple), `type` (`_type`, optional), `value` (`expression_list`,
optional).

### Assignment statement

Node kind: `assignment_statement` (line 105 / 330). No fields documented beyond
its child expressions (LHS/RHS are positional children).

### Return statement

Node kind: `return_statement` (line 1887). No fields; children: optional
`expression_list`. The return values are accessed as named children of the
`expression_list`, not as a direct `value` field.

---

## 2. `complexity.rs` public interface

Exported (`pub(crate)`) surface:

```rust
// src/checkers/complexity.rs

pub(crate) enum SubtestHandling { IncludeAll, ExcludeRunSubtests }

pub(crate) fn total_complexity(root: Node, source: &[u8]) -> usize
pub(crate) fn cyclomatic_complexity(decl: Node, source: &[u8], subtests: SubtestHandling) -> usize
```

`count_decision_points` is **private** (no `pub`/`pub(crate)`) — it cannot be called
directly from `rules.rs`.

**Using `cyclomatic_complexity` on a condition sub-node**: The function signature
accepts any `Node`; it does not assert the node is a function declaration. It calls
`1 + count_decision_points(decl, ...)` which recurses through children counting
`if_statement`, `for_statement`, case nodes, and short-circuit `&&`/`||`. Calling
it on the `condition` field of an `if_statement` is therefore valid — it will return
1 + (number of `&&`/`||` operators and nested if/for constructs in that condition
expression), which is the cyclomatic contribution of that condition alone. The base
`+1` will over-count by one relative to a "decision-points-only" count, but that is
consistent with how the existing `file-complexity` check measures function bodies.

For `decompose-conditional`, the right call is:
```rust
let cond_node = if_node.child_by_field_name("condition").unwrap();
let complexity = cyclomatic_complexity(cond_node, source, SubtestHandling::IncludeAll);
// fire if complexity > threshold (e.g. 3 or 4)
```

---

## 3. `LangRuleConfig` extensibility

`LangRuleConfig` currently has **27 fields** (lines 236–348 of `src/checkers/rules.rs`):

`name`, `file_globs`, `function_kinds`, `if_kind`, `nesting_kinds`,
`else_wrapper_kinds`, `chain_kinds`, `param_counter`, `body_finder`, `params_finder`,
`bool_param_finder`, `ternary_kind`, `block_kind`, `statement_container`,
`unwrap_statement`, `terminal_kinds`, `panic_detector`, `literal_kinds`,
`numeric_literal_kinds`, `binding_finder`, `return_kind`, `call_kinds`,
`operator_kinds`, `density_boundary_kinds`, `uppercase_call_is_constructor`,
`collection_literal_kinds`, `slog_key_predicate`.

All fields are `'static` slices or `fn` pointers — the struct is zero-heap at
runtime (all data lives in the binary). Adding 2–3 fields has no meaningful size
impact.

**Reusable fields for the new checks:**

| Check | Reusable fields |
|---|---|
| `decompose-conditional` | `if_kind` (find the if node), `function_kinds` (walk from function root), `ternary_kind` (flag ternaries too) |
| `consolidate-conditional` | `if_kind`, `terminal_kinds` / `return_kind` (detect matching returns), `block_kind` (get sibling statements) |
| `remove-control-flag` | `function_kinds` (scope walk), `if_kind` (find uses inside if-conditions) |

A new threshold field (e.g. `decompose_cond_threshold: usize`) may be wanted for
`decompose-conditional` to let non-Go languages tune it independently; the Go
config could start at the same value as `CYCLOMATIC_COMPLEXITY_THRESHOLD` (10) or
a lower per-condition value like 3.

---

## 4. Variable tracking infrastructure for `remove-control-flag`

No read/write use-tracking for local variables within a function body exists in the
codebase. The three relevant files do different things:

- **`src/scope.rs`** (`ScopeMap`, `Spread`): tracks function/class scope *boundaries*
  (byte ranges) and can answer "which function scope does byte offset B fall in?" and
  "how many distinct function scopes did a set of byte offsets span?" Useful for
  `remove-control-flag` to verify that all uses of the candidate variable stay within
  one function scope, but it does not track individual variable references.

- **`src/declarations.rs`** (`DeclarationGraph`): cross-file, top-level declarations
  (functions, types, interfaces). Has no concept of local variables.

- **`src/symbol_extract.rs`**: export-level symbol extraction for the architecture
  model. Not relevant to local variable tracking.

**Implication for `remove-control-flag`**: the checker must implement its own
single-pass AST walk over the function body. The algorithm:

1. Walk the function body looking for `short_var_declaration` or `var_declaration`
   nodes where the declared type is `bool` or the initializer is `true`/`false`.
2. Collect the declared name(s).
3. For each candidate, do a second pass counting all uses:
   - assignments (`assignment_statement` where LHS matches the name)
   - reads (identifier nodes matching the name that are inside `if_statement`
     conditions or loop conditions — `for_clause.condition` / bare `for` condition)
   - reads anywhere else (would disqualify the variable as a pure loop-exit flag)
4. Fire if uses are exclusively: one initializing declaration, one or more assignments
   inside the loop body, and one or more reads only in the enclosing `for_statement`'s
   condition / loop-break `if` condition.

The `ScopeMap` can optionally be used to restrict the candidate search to the
innermost enclosing function scope, avoiding false positives from closures that
capture an outer boolean.
