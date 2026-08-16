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

The three rules and their thresholds are the same across languages
(`rules::CATALOG` is language-agnostic); only the underlying tree-sitter node
kinds each language's `lang_config()` entry checks against differ:

| Rule ID               | Category   | Default severity | Threshold             | Description |
|------------------------|------------|-------------------|------------------------|--------------|
| `long-function`        | complexity | advisory          | > 40 lines             | Function/method body spans more lines than this. |
| `deep-nesting`         | complexity | advisory          | > 4 levels             | Function/method body nests control-flow constructs deeper than this. An `else if` chain is treated as one flat branch, not added nesting, in every language. |
| `long-parameter-list`  | style      | advisory          | > 5 identifiers        | Function/method parameter list names more identifiers than this. |

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
