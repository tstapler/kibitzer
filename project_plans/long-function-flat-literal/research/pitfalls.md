# Pitfalls: long-function-flat-literal

## 1. Over-broad exemption: exempting functions with logic inside entries

**Risk**: A composite literal whose entries contain call expressions — e.g.
`{name: fmt.Sprintf("case %d", i), input: computeVal()}` — is not a flat data table.
Extract Function applies to a `computeVal()` call; silencing `long-function` here is
wrong.

**Mitigation**: Step 4 of `is_flat_data_literal_body` walks the literal's descendants and
returns `false` (exemption does not apply) if any `call_kinds` or `nesting_kinds` node
is found. The predicate is conservative: one call expression anywhere in the literal
blocks the exemption.

**Residual risk**: a literal with entries like `{name: "x", input: 1 + 2}` (a simple
binary expression, not a call) would still be exempted even though `1 + 2` could
theoretically be named. Binary expressions are too common in literal fields (computed
offsets, bit flags, port numbers) to treat as logic; `cfg.call_kinds` and `cfg.nesting_kinds`
are the right gates.

## 2. Confusing "one statement" with "one line"

**Risk**: A function whose body is one `return` statement that spans one line plus a
giant inline comment still has one statement. The predicate would exempt it even though
the body is short in practice (it's already under 40 lines via comment elision — the
threshold counts lines, not statements). This is not a real risk because the exemption
only fires after `body_lines > LONG_FUNCTION_LINES` — i.e., the body must already be
over 40 lines to reach the predicate.

## 3. Go's `statement_list` nesting mismatch

**Risk**: Calling `body.named_children()` instead of
`(cfg.statement_container)(body).named_children()` would count the `statement_list` node
itself as one child, making every Go function body appear to have one statement.
The predicate would then proceed to check whether `statement_list` is a `return_kind`,
which it isn't, and return `false` (no exemption) — a silent wrong-but-safe result for
Go instead of a false exemption. But for any language using `identity_stmt` as
`statement_container`, the same code path returns the block directly and works correctly.

**Mitigation**: explicitly use `(cfg.statement_container)(body)` in Step 1.
Add a test that constructs a Go body with two statements and confirms no exemption fires,
to guard against accidental direct-child iteration.

## 4. TypeScript `object` confusion

**Risk**: TypeScript's `object` node kind is used for both object literals and for
object destructuring patterns in some grammars. `{a, b}` in a destructure context
would appear as an `object` node.

**Mitigation**: the exemption path reaches Step 3 only when the return statement has
exactly one named child, and object destructuring on a `return` statement is not valid
TypeScript syntax. The grammar parses `return {a, b}` as a return of a shorthand object
literal, not a destructure. No action needed.

## 5. Kotlin/Rust false confidence from `&[]`

**Risk**: Setting `collection_literal_kinds = &[]` for Kotlin and Rust silently means
the exemption never fires for those languages — a Rust `fn cases() -> Vec<Case>` that
returns `vec![...]` (a macro call, not a literal) will still be flagged by `long-function`.

**Mitigation**: this is the correct behavior — `vec![...]` contains a macro invocation,
which is call-like; the extract-function suggestion is actionable (extract the data into
a `const` array or a module-level static). The `&[]` entries are correct, not gaps. A
comment on the field definition should explain why.

## 6. Interaction with `inline-ignore` directives

A user may already have `// kibitzer:ignore long-function` on a flat-literal function.
After this change, the finding no longer fires, and the directive becomes redundant.
`inline-ignore` is checked by the `inline-ignore` checker, which would flag the
now-unused directive as a warning — a net positive, prompting the user to remove the
stale suppression.

## 7. Backtest corpus: generated data factories

Some generated codebases produce flat-literal-like functions (protobuf test fixtures,
codegen'd lookup tables) whose entries happen to not contain call expressions. The
exemption would correctly silence `long-function` for these. Confirm via the
real-world corpus run (AC9) that this suppression is genuinely noise, not a case
where splitting or extracting would improve maintainability. Expectation: yes, generated
data factories are noise for Extract Function.
