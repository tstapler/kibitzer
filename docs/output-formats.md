# Structured output formats

By default a `command` check is pure pass/fail: kibitzer looks at the exit
code and, if it also follows the `{file}:{line}: message` convention, scopes
that text to changed lines (see the `command` field's doc comment in
`src/config.rs`). That loses information a linter already has — a failing
run with 1 warning reads identically to one with 50 errors.

Setting `output_format` on a check tells kibitzer to parse the command's
stdout as a known structured shape instead, so severity and finding counts
survive into the reported output.

## `sarif`

[SARIF](https://sarifweb.azurewebsites.net/) 2.1.0 is the only supported
format today — most linters that emit structured output already have a
`--format sarif` / `--output-format sarif` flag (ESLint, ruff, etc.), so
there's no per-linter field-mapping config to maintain.

```json
{
  "checks": [
    {
      "name": "eslint-sarif",
      "command": "eslint --format @microsoft/eslint-formatter-sarif {file}",
      "severity": "advisory",
      "scope": ["**/*.ts"],
      "triggers": ["Edit", "Write"],
      "output_format": "sarif"
    }
  ]
}
```

`output_format` requires `command` — native checkers already report
structured `Finding`s and have no use for it.

When the command's stdout parses as SARIF, kibitzer replaces the raw output
with a plain-text rendering: a leading count-by-level line, then one line per
result:

```
2 error(s), 1 warning(s)
src/app.ts:12: [error] 'foo' is never used (no-unused-vars)
src/app.ts:40: [error] missing return type (explicit-function-return-type)
src/app.ts:41: [warning] prefer const (prefer-const)
```

If stdout isn't valid SARIF (misconfigured formatter flag, tool crashed
before writing output, etc.), kibitzer falls back to raw stdout+stderr
rather than silently hiding the mismatch.

### Known limitation: no diff-aware scoping yet

Diff-aware output filtering (`scope_output_to_changed_lines`) only
understands the `{file}:{line}: message` text convention, not SARIF's
structured locations. A SARIF check's full output is always shown — it's
not filtered down to the lines an edit actually touched. Wiring SARIF
locations into the same scoping logic is a natural follow-up if it turns
out to matter in practice.
