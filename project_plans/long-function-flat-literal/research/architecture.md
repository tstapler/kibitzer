# Architecture Research: long-function-flat-literal

## 1. Change site: one guard in `check_declaration`, not a new walk

`check_declaration` (`src/checkers/rules.rs:1534`) is the single entry point for
`long-function`. It currently looks like:

```rust
fn check_declaration(decl: Node, cfg: &LangRuleConfig, src: &[u8], findings: &mut Vec<Finding>) {
    let line = decl.start_position().row + 1;
    if let Some(body) = (cfg.body_finder)(decl) {
        let body_lines = body.end_position().row - body.start_position().row + 1;
        if body_lines > LONG_FUNCTION_LINES {
            findings.push(Finding { ... "[long-function] ..." });
        }
        // deep-nesting follows ...
    }
}
```

The exemption inserts one guard inside the `body_lines > LONG_FUNCTION_LINES` arm:

```rust
if body_lines > LONG_FUNCTION_LINES && !is_flat_data_literal_body(body, cfg) {
    findings.push(...)
}
```

No new walk, no new `CATALOG` entry (not a new rule — a precision fix to an existing one),
no change to the two-walk pipeline (`walk_declarations` / `walk_blocks`). The helper
function is a pure predicate that inspects only the `body` node already in scope.

## 2. The flat-literal predicate: four sequential checks

`is_flat_data_literal_body(body: Node, cfg: &LangRuleConfig) -> bool`:

**Step 1 — one statement**: use `cfg.statement_container` to get the statement list node
(Go: `block > statement_list`; every other language: the block itself — `identity_stmt`),
then count its `named_children`. Must equal exactly 1. A function with zero statements
(empty body) is already under 40 lines; a function with ≥2 statements has logic.

**Step 2 — return statement**: the single named child's `kind()` must equal
`cfg.return_kind` (`"return_statement"` for Go/TS/JS/Python/Java;
`"return_expression"` for Kotlin/Rust). A `let`/assignment-only statement isn't a
data-returner.

**Step 3 — collection literal return value**: the return statement's single named child
must be a node whose kind is in the language's collection-literal set. Two design
options here (see §3 below for trade-off).

**Step 4 — no logic in the literal**: walk descendants of the collection literal node,
not crossing into nested function/closure boundaries, and confirm no node is of a kind
in `cfg.nesting_kinds`, `cfg.call_kinds`, or equals `cfg.if_kind`. If any logic node
is found, the exemption does not apply — the function has meaningful behavior inside
its literal that could be extracted.

Steps 1–4 are cheap: the guard fires after the body-line threshold is already exceeded,
so it only runs on functions that are flagged today. In practice these are rare; the cost
is negligible.

## 3. Collection-literal kinds: new field vs. derived from `density_boundary_kinds`

Two approaches for Step 3's "is this a data-container literal?" test:

### Option A: Derive from `density_boundary_kinds` minus `nesting_kinds`

`density_boundary_kinds` already exists (`LangRuleConfig:299–303`) and covers:
- Go: `["func_literal", "composite_literal"]`
- TypeScript: `["arrow_function", "function_expression", "object", "array", "class_body"]`
- Python: `["lambda", "list", "dictionary", "set", "tuple"]` (need to verify)

`nesting_kinds` covers the function/closure-like entries (`func_literal`,
`arrow_function`, `function_expression`, `lambda`). The set difference gives the pure
data-container kinds.

**Advantage**: no new `LangRuleConfig` field, change is smaller.
**Disadvantage**: `class_body` (TS) would remain in the set even though it's not a
test-fixture kind, and `tuple`/`set`/`dict` are in `density_boundary_kinds` for Python
only if they were added there (need to verify exact contents). The derivation logic
(`density_boundary - nesting_kinds`) is implicit and not self-documenting.

### Option B: New `collection_literal_kinds: &'static [&'static str]` field

Explicit, self-documenting, independently settable per language.

Go: `&["composite_literal"]`
TypeScript/TSX/JavaScript: `&["array", "object"]`
Python: `&["list", "dictionary", "tuple"]`
Java: `&[]` (no idiomatic flat-literal pattern; `new Case[]{...}` is an array creation, not a literal)
Kotlin: `&["multiline_string_literal"]` is incorrect; Kotlin list literals use `listOf(...)` which is a call — `&[]` is correct
Rust: `&[]` (`vec![...]` is a macro invocation, not a literal node kind)

**Advantage**: explicit, independently verifiable per grammar, self-documenting.
**Disadvantage**: new field in `LangRuleConfig`; 8 constructor functions each need an
entry, including empty-slice entries for Java/Kotlin/Rust where the pattern doesn't apply.

**Recommendation: Option B**, for the same reason `binding_finder` was a function pointer
rather than a derived computation from existing fields in `replace-magic-literal`'s
research: implicit derivations from existing fields produce correct code today but fail
silently when either source field is updated for unrelated reasons. A new dedicated field
costs 8 lines in the constructor functions (including 3 `&[]` entries) and is read by a
single new predicate — the overhead is minimal.

## 4. Multi-language scope

| Language | Idiomatic flat-literal shape | `collection_literal_kinds` |
|----------|------------------------------|---------------------------|
| Go | `return []SomeStruct{{...}, {...}}` | `"composite_literal"` |
| TypeScript | `return [{...}, {...}]` | `"array"` |
| TSX | same as TS | `"array"` |
| JavaScript | `return [{...}, {...}]` | `"array"` |
| Python | `return [Case(...), ...]` | `"list"` |
| Java | `return new Case[]{...}` | `&[]` (array creation is a call subtree, not a literal node) |
| Kotlin | `return listOf(...)` | `&[]` (a call expression, not a literal) |
| Rust | `return vec![...]` | `&[]` (a macro invocation) |

Java, Kotlin, and Rust effectively have no idiomatic flat-literal fixture function
pattern that the tree-sitter grammars represent as a pure literal node. Setting
`collection_literal_kinds = &[]` for these three is the correct answer — the predicate
returns `false` immediately at Step 3, and those languages' behavior is unchanged.

## 5. Go statement_list nesting

Go's tree-sitter grammar wraps `block` children in a `statement_list` node
(`go_statement_container`, `src/checkers/rules.rs:601`). The "one statement" check in
Step 1 must call `(cfg.statement_container)(body)` and then count that node's
`named_children`, not count `body.named_children()` directly. Every other language uses
`identity_stmt` for `statement_container`, so the same code path works uniformly.

## 6. `deep-nesting` is unaffected

The `deep-nesting` rule runs immediately after `long-function` in `check_declaration`
(`src/checkers/rules.rs:1548`). A flat composite literal body with no nesting kinds
cannot reach the 4-level nesting threshold; it would never fire anyway. No guard is
needed there.

## 7. Disposition: Extend as-is

One new predicate function (~25 lines), one new `LangRuleConfig` field (or zero, via
Option A), minimal changes to `check_declaration`. This does not strain the file or
violate any established convention. `lang_config()` was already refactored into 8 small
constructors by the `replace-magic-literal` work; a new `collection_literal_kinds` field
is a single line per constructor.
