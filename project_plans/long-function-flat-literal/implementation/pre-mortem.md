# Pre-Mortem: long-function-flat-literal

Imagine it is 4 weeks after this was shipped. The feature caused a regression. What went
wrong?

## Scenario A: A real Extract Function candidate was silenced

A developer had a 60-line Go function:
```go
func createHandlers() []Handler {
    return []Handler{
        {name: "auth", fn: buildAuthHandler()},
        {name: "api",  fn: buildAPIHandler()},
        ...
    }
}
```

The entries contain `buildAuthHandler()` calls. `has_logic_descendants` should have
caught these, but it did not, and the `long-function` finding was silenced.

**Root cause**: `has_logic_descendants` was missing the `cfg.call_kinds` check, or
the check was using the wrong comparison (e.g. `==` on a slice element instead of
`contains`).

**Prevention**: Test 2.1.1b (`long_function_does_not_exempt_literal_with_call`) directly
covers this case. Ensure the test uses a call expression inside a composite literal
entry, not just at the top level.

## Scenario B: `node_kind_literals_are_valid_for_their_grammar` doesn't cover the new field

The `collection_literal_kinds` field was added but not included in the grammar-verification
test. A typo in one language's kind name (e.g. `"arrray"` for TypeScript) compiles fine
but silently matches nothing — the exemption never fires for that language. No test
catches it.

**Root cause**: Task 1.1.1j (add to the grammar-verification test) was skipped.

**Prevention**: Run `cargo test node_kind_literals` after Task 1.1.1j and verify the
test list includes the new field. The test is not optional.

## Scenario C: TypeScript `object` exemption silences a real factory function

A TypeScript function:
```typescript
function buildConfig(): AppConfig {
    return {
        timeout: DEFAULT_TIMEOUT,
        retries: MAX_RETRIES,
        ...50 more fields
    };
}
```
was silenced because its body is one `return` of an `object` with no calls and no
control flow. But the function should be split by concern (network config, retry config,
etc.) — the finding was actionable.

**Root cause**: The exemption for TypeScript `object` is too broad. Unlike Go's
`[]tc{{...}, {...}}` which is clearly a list of test cases, a large flat configuration
object is a legitimate Extract Function target.

**Mitigation / judgment call**: This is a precision trade-off, not a correctness bug.
The exemption for `"object"` in TypeScript can be narrowed in a follow-up if the
backtest corpus (Task 3.1.1b) reveals this false-negative pattern. For the initial
implementation, `"array"` only (not `"object"`) for TypeScript/JavaScript would be
the more conservative choice. The plan should consider defaulting TypeScript to
`collection_literal_kinds: &["array"]` only, with `"object"` as a potential follow-up.

## Scenario D: Python `list` with comprehension

A Python function:
```python
def cases() -> list[Case]:
    return [
        Case(name=f"case {i}", input=i, want=i + 1)
        for i in range(50)
    ]
```

`for i in range(50)` is a list comprehension — a `for_statement` equivalent inside a
`list`. `has_logic_descendants` would correctly catch `for_in_clause` (the Python
grammar's kind for the `for x in y` part of a comprehension) IF `for_in_clause` is in
`cfg.nesting_kinds` for Python.

**Root cause risk**: Python's `for_in_clause` (comprehension) may NOT be in
`cfg.nesting_kinds` — which currently lists `for_statement`, `while_statement`,
`match_statement`, `lambda`. A comprehension `for` is a different node kind.

**Prevention**: Add a test (`py_long_function_does_not_exempt_list_comprehension`)
covering this case. If `for_in_clause` is not in `cfg.nesting_kinds`, the predicate
must also check for comprehension-specific kinds (or Python's `collection_literal_kinds`
should be `&[]` until this is confirmed safe). Flag this as a verification item in
Task 1.1.1f.

## Scenario E: Go `go_statement_container` returns wrong node

The `go_statement_container` function returns the `statement_list` node or, if none is
found, the block itself. A malformed/generated Go source with no `statement_list` child
would cause the predicate to iterate `block`'s named children directly, including the
`{` and `}` punctuation as unnamed children (invisible to `named_children`) but also any
other direct named children. In practice this fallback path (`unwrap_or(block)`) would
find more than one named child, and the predicate would return false (no exemption). The
fallback is safe — it produces a false-negative (still flags) rather than a
false-positive (incorrectly exempts).

**Verdict**: No action needed; the fallback is conservative.

## Summary of preventive actions

| Risk | Action |
|------|--------|
| A: call check missed | Test 2.1.1b; verify `cfg.call_kinds` check is `contains`, not `==` |
| B: grammar verification gap | Task 1.1.1j is mandatory, not optional |
| C: TypeScript object over-broad | Consider `collection_literal_kinds: &["array"]` only for TS/JS; defer `"object"` |
| D: Python comprehension | Add Python comprehension test; verify `for_in_clause` handling |
| E: Go statement_list fallback | No action; fallback is conservative |
