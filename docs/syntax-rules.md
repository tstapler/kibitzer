# Native syntax rule catalog

`kibitzer check native <checker-name> <file>` runs a catalog of purely
syntactic, tree-sitter-based code-smell rules in a single parse pass
(`src/rules.rs`). Each finding is prefixed `[rule-id]` so output stays
greppable per-rule even though all rules run together.

The catalog is language-parameterized: one `SyntaxRulesChecker` instance is
registered per supported language, each under its own checker name (`lookup()`
matches on exact name, so each needs a distinct one — see `src/checker.rs`'s
`registry()`):

| Language   | Checker name                  | File globs                              |
|------------|--------------------------------|------------------------------------------|
| Go         | `syntax-rules`                 | `**/*.go`                                 |
| TypeScript | `syntax-rules-typescript`      | `**/*.ts`                                 |
| TSX        | `syntax-rules-tsx`             | `**/*.tsx`                                |
| JavaScript | `syntax-rules-javascript`      | `**/*.js`, `**/*.jsx`, `**/*.mjs`, `**/*.cjs` |
| Python     | `syntax-rules-python`          | `**/*.py`                                 |
| Java       | `syntax-rules-java`            | `**/*.java`                               |
| Kotlin     | `syntax-rules-kotlin`          | `**/*.kt`, `**/*.kts`                     |
| Rust       | `syntax-rules-rust`            | `**/*.rs`                                 |

The rules and their thresholds are the same across languages
(`rules::CATALOG` is language-agnostic); only the underlying tree-sitter node
kinds each language's `lang_config()` entry checks against differ:

| Rule ID               | Category   | Default severity | Threshold             | Description |
|------------------------|------------|-------------------|------------------------|--------------|
| `long-function`        | complexity | advisory          | > 40 lines             | Function/method body spans more lines than this. |
| `deep-nesting`         | complexity | advisory          | > 4 levels             | Function/method body nests control-flow constructs deeper than this. An `else if` chain is treated as one flat branch, not added nesting, in every language. |
| `long-parameter-list`  | style      | advisory          | > 5 identifiers        | Function/method parameter list names more identifiers than this. |
| `flag-argument`        | design     | advisory          | n/a                    | A boolean-typed parameter is branched on directly (an `if`/ternary condition, or an operand of one) inside the function body — Fowler's *Remove Flag Argument*. A parameter only ever forwarded to another call is not flagged. Requires a statically-known boolean type, so it's a no-op on plain JavaScript and on untyped Python parameters. |
| `unreachable-code`     | dead-code  | advisory          | n/a                    | A statement follows an unconditional `return`/`break`/`continue`/panic-call in the same `{ ... }` block — Fowler's *Remove Dead Code*. Only the first dead statement in a block is flagged (everything after it is dead by construction). Doesn't descend into `switch`/`match`/`when` case bodies. Go's `panic(...)` and Rust's `panic!`/`unreachable!`/`todo!`/`unimplemented!` count as diverging calls; other languages have no such built-in and are return/break/continue-only. Kotlin is return-only — `tree-sitter-kotlin-ng` 1.1.0 has no dedicated node kind for a bare `break`/`continue` (it parses as a plain identifier). |

Per-language node kinds (`src/rules.rs`'s `lang_config()`), verified against
each grammar's real `to_sexp()` output:

- **Go**: function-like — `function_declaration`, `method_declaration`.
  Nesting — `if_statement`, `for_statement`, `expression_switch_statement`,
  `type_switch_statement`, `select_statement`, `func_literal`. Chained
  `else if` is a bare `if_statement` directly in the `alternative` field.
  Parameters are counted per name under `parameter_declaration`/
  `variadic_parameter_declaration` (grouped names like `func f(a, b string)`
  count as two).
- **TypeScript/TSX**: function-like — `function_declaration`,
  `function_expression`, `generator_function_declaration`,
  `method_definition`, `arrow_function`. Nesting — `if_statement`,
  `for_statement`, `for_in_statement`, `while_statement`, `do_statement`,
  `switch_statement`, `arrow_function`, `function_expression`. Chained
  `else if` is wrapped in an intermediate `else_clause` node before the
  nested `if_statement` — unwrapped so it still counts as flat. Every
  parameter (including defaults, rest, destructuring, optional) is wrapped in
  its own `required_parameter`/`optional_parameter` node, so each wrapper
  counts as one parameter.
- **JavaScript**: same function-like/nesting/`else_clause` handling as
  TypeScript. `formal_parameters`' children are bare pattern nodes with no
  wrapper (`identifier`, `assignment_pattern`, `rest_pattern`,
  `object_pattern`, `array_pattern`); each *named* child counts as one
  parameter (unnamed punctuation children like `,` are filtered out).
- **Python**: function-like — `function_definition` (used for both top-level
  functions and class methods; a `@decorator` wraps it in a
  `decorated_definition` node, and `async def` produces a plain
  `function_definition` with no wrapper — neither needs special-casing since
  the walk recurses into every descendant regardless of kind). Nesting —
  `for_statement`, `while_statement`, `match_statement`, `lambda`. Chained
  `elif` is a distinct `elif_clause` node (not an `if_statement` wrapped in
  an `else_clause` like JS/TS) but shares the same `condition`/
  `consequence`/`alternative` fields, so it's recognized as a chain
  continuation rather than added nesting. `parameters`' children are bare
  pattern nodes (`identifier`, `default_parameter`, `typed_parameter`,
  `typed_default_parameter`, `list_splat_pattern` for `*args`,
  `dictionary_splat_pattern` for `**kwargs`) — each counts as one parameter,
  except the bare `positional_separator` (`/`) and `keyword_separator` (`*`)
  marker nodes, which name no parameter and are excluded from the count.
- **Java**: function-like — `method_declaration`, `lambda_expression`
  (`constructor_declaration` deliberately excluded — not yet verified against
  the grammar). Nesting — `for_statement`, `enhanced_for_statement`,
  `while_statement`, `do_statement`, `switch_expression`,
  `lambda_expression`. `if_statement` has proper `condition`/`consequence`/
  `alternative` fields (Go-like, no wrapper); `method_declaration` has proper
  `parameters` (`formal_parameters`, containing `formal_parameter` nodes) and
  `body` (`block`) fields.
- **Kotlin** (`tree-sitter-kotlin-ng`): function-like —
  `function_declaration` (unified across top-level functions and methods,
  like Python) and `anonymous_function` (the `fun(x: Int) { ... }` expression
  form). Neither exposes `body`/`parameters` as named fields — unlike every
  other supported language, `function_value_parameters` and `function_body`
  are purely positional children, found by node kind rather than
  `child_by_field_name` (see `body_finder`/`params_finder` on
  `LangRuleConfig`, and `kotlin_body`/`kotlin_params` in `src/rules.rs`).
  Nesting — `for_statement`, `while_statement`, `do_while_statement`,
  `when_expression`, `lambda_literal` (the `{ x -> ... }` literal form,
  treated as nesting-only, not function-like — its parameter list uses a
  different node kind than regular functions and is deliberately not
  counted), `anonymous_function`. `if_expression` only exposes `condition` as
  a named field; the then-branch and else/elif continuation are positional
  named children, resolved via `if_branches`'s field-then-positional
  fallback. `function_value_parameters` wraps `parameter` nodes; a `vararg`
  modifier produces a **sibling** `parameter_modifiers` node rather than
  nesting inside the `parameter`, so the counter filters to
  `kind() == "parameter"` to avoid over-counting.
- **Rust** (`tree-sitter-rust`): function-like — `function_item` only (covers
  free functions, inherent/trait `impl` methods, and default trait-method
  bodies alike — one node kind for all three). A trait method *declaration*
  with no body is the distinct `function_signature_item` kind, excluded from
  `function_kinds` (its `body_finder` lookup would return `None` anyway, same
  as Go interface methods). `if_expression` has proper `condition`/
  `consequence`/`alternative` fields (Go-like); a chained `else if` wraps in
  an intermediate `else_clause` (JS/TS-like). Nesting —
  `for_expression`/`while_expression`/`loop_expression`/`match_expression`/
  `closure_expression` (a closure is nesting-only, like Go's `func_literal` —
  its parameter list uses a different node kind, `closure_parameters`, not
  `parameters`). `parameters`' children are `parameter` nodes plus, for a
  method, a leading `self_parameter` (`&self`/`&mut self`/`self`) — excluded
  from the count the same way Go's implicit receiver never appears in its
  parameter list at all.

`flag-argument`'s boolean-type detection per language, verified against real
`to_sexp()` output (`bool_param_finder` in `src/rules.rs`): Go's plain `bool`
`type_identifier`; TS's `predefined_type` `boolean` inside a parameter's
`type_annotation` (plain JS parameters carry no `type` field at all, so the
check is a no-op there — it never guesses a boolean from a name or usage);
Python's `type` field on a `typed_parameter`/`typed_default_parameter` whose
text is exactly `bool` (an unannotated parameter is skipped, same reasoning
as JS); Java's primitive `boolean_type` or boxed `Boolean` `type_identifier`;
Kotlin's `user_type` wrapping an `identifier` reading `Boolean` (Kotlin has
no primitive-type keywords); Rust's by-value `primitive_type` reading `bool`
(a `&bool` reference parameter is deliberately not unwrapped). Destructured/
tuple/pattern parameters are skipped everywhere — only a plain named
parameter is a candidate. The condition side of the check reuses each
language's `if_kind` (so `else if`/`elif` conditions count too) plus, where
the grammar exposes a ternary with a `condition` field, that node kind too
(`ternary_expression` for TS/JS/Java; Python's `conditional_expression` and
Kotlin/Go/Rust, which have no ternary, are `None`).

Thresholds are fixed constants in `src/rules.rs` for now; per-rule
configurability is a natural follow-up, not required for the initial catalog.

## Wiring into `.claude/inspect.json`

```json
{
  "checks": [
    {
      "name": "go-syntax-rules",
      "checker": "syntax-rules",
      "severity": "advisory",
      "scope": ["**/*.go"],
      "triggers": ["Edit", "Write"]
    },
    {
      "name": "ts-syntax-rules",
      "checker": "syntax-rules-typescript",
      "severity": "advisory",
      "scope": ["**/*.ts"],
      "triggers": ["Edit", "Write"]
    }
  ]
}
```
