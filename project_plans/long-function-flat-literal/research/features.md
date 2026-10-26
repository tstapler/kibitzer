# Features Research: long-function-flat-literal

## What changes

This is a precision fix, not a new feature. The observable behavior delta is:

**Before**: `[long-function]` fires on any function body > 40 lines, including functions
whose body is a single `return` of a flat composite/collection literal.

**After**: `[long-function]` fires on function bodies > 40 lines unless the body is a
single `return` of a flat composite/collection literal (no control flow, no call
expressions in the literal's subtree).

No new rule ID. No new `CATALOG` entry. No changes to the check's message text when it
does fire. No changes to other rules.

## Scope of the precision fix

The exemption is intentionally narrow. The following shapes still fire `long-function`:

| Shape | Why still flagged |
|-------|------------------|
| Function body: two statements (e.g. a variable binding + return) | "One statement" predicate fails |
| Function body: one `return` of a function call | Return value not a collection literal kind |
| Function body: one `return` of a composite literal that contains a call expression | Logic-presence check fails |
| Function body: one `return` of a composite literal that contains an `if` or `for` | Logic-presence check fails |
| Function body > 40 lines with any multi-statement structure | Unaffected; predicate not reached |

The shape that is newly exempted:

| Shape | Example |
|-------|---------|
| Go: one `return` of a `composite_literal` with no logic | `func cases() []tc { return []tc{{...}, {...}, ...} }` |
| TypeScript: one `return` of an `array` with no logic | `function cases(): TC[] { return [{...}, {...}, ...] }` |
| Python: one `return` of a `list` with no logic | `def cases() -> list[TC]: return [TC(...), TC(...), ...]` |

## Prior art / comparison

- **ESLint's `max-lines-per-function`** (JavaScript): configures `ignoreIIFEs` and
  `ignoreComments` but has no concept of "single-literal body." No direct prior art for
  this exemption.
- **SonarQube's cognitive complexity**: does not count literal lines toward complexity.
  Conceptually closest to Option 2 (count only statement lines) from the filed item.
- **Go vet / staticcheck**: no line-length check; no comparable exemption to research.

The filed item's Option 1 (no prior art reference needed — it's a first-principles
structural property of the body, not borrowed from another tool's implementation) is the
right shape. The relevant research is "can tree-sitter detect this cleanly" (yes, see
`research/architecture.md`) rather than "what does another tool do."

## Non-goals

- This does not add a configurable threshold for the exemption (e.g. "only if the literal
  has fewer than N entries"). The condition is structural, not size-based.
- This does not add a new `CATALOG` rule or change any existing rule's ID or message.
- This does not change how `long-function` fires on non-literal bodies.
- This does not suppress `long-function` for `_test.go` files broadly (Option 3 was
  explicitly rejected).
