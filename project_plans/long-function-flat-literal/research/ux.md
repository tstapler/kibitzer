# UX Research: long-function-flat-literal

## Current experience

A user with a 49-line Go table-test helper like the reproducer:

```go
func cases() []tc {
    return []tc{
        {name: "case 1", input: 1, want: 2},
        // ... 45 rows in total
    }
}
```

receives:

```
table_test.go:10: [long-function] body spans 49 lines (over 40) — consider Fowler's
Extract Function (https://refactoring.com/catalog/extractFunction.html)
```

Acting on this finding produces no improvement: extracting the literal to a helper
function creates a second 49-line function, reproducing the finding. The user either:
(a) adds a `scope` glob silencing all `syntax-rules` for the path (kills other useful
findings for that file), (b) adds a `.kibitzer/accepted/` entry (maintenance overhead),
(c) ignores kibitzer's output for this file (trains the user to ignore the tool).

All three outcomes are worse than not firing the finding.

## Desired experience

No finding is emitted for the `cases()` function. The tool stays silent precisely
because there is no refactoring to suggest. Other `long-function` findings in the same
file (functions with real logic over 40 lines) continue to fire.

## Message text when the check still fires

No change. The existing message is accurate for every shape where the finding fires
after this change:
```
[long-function] body spans N lines (over 40) — consider Fowler's Extract Function (...)
```

## Documentation surface

One addition to `docs/syntax-rules.md`'s `long-function` description (currently one
sentence in the catalog table): a note that functions whose body is a single `return`
of a flat collection literal are exempt, the same way `replace-magic-literal`'s row
documents its table-test scope (line 160).

No change to `docs/suppressing-checks.md` — the existing `scope`/`.kibitzer/accepted/`
mechanisms remain valid for users who want to silence the check for other reasons.
