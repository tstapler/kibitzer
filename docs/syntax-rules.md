# Native syntax rule catalog

`kibitzer check native syntax-rules <file>` runs a catalog of purely syntactic,
tree-sitter-based code-smell rules against a Go file in a single parse pass
(`src/rules.rs`). Each finding is prefixed `[rule-id]` so output stays
greppable per-rule even though all rules run together.

Go only for now — the catalog structure (`rules::RuleMeta`, `rules::CATALOG`)
is designed to grow with more rules and, eventually, more languages, but
adding a new tree-sitter grammar is out of scope here.

| Rule ID               | Category   | Default severity | Threshold             | Description |
|------------------------|------------|-------------------|------------------------|--------------|
| `long-function`        | complexity | advisory          | > 40 lines             | Function/method body spans more lines than this. |
| `deep-nesting`         | complexity | advisory          | > 4 levels             | Function/method body nests `if`/`for`/`switch`/`select`/`func_literal` deeper than this. |
| `long-parameter-list`  | style      | advisory          | > 5 identifiers        | Function/method parameter list names more identifiers than this. |

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
    }
  ]
}
```
