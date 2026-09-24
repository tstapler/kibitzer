# Research: stack details for `hide-delegate` check

## Dependencies — none new

`Cargo.toml` already pins tree-sitter and a grammar crate for all 7 languages
`LangRuleConfig` covers. Confirmed via `grep tree-sitter Cargo.toml`:

```
tree-sitter = "0.26"
tree-sitter-go = "0.25"
tree-sitter-typescript = "0.23"
tree-sitter-javascript = "0.23"
tree-sitter-python = "0.23"
tree-sitter-java = "0.23.5"
tree-sitter-kotlin-ng = "1.1.0"
tree-sitter-rust = "0.24"
```

This check is a pure AST walk over trees these grammars already produce (the
same trees `deep-nesting`/`long-function`/`flag-argument` walk today) — no new
crate, no `[dependencies]` change. `RuleMeta`/`LangRuleConfig`/`Checker` are
all already `pub(crate)`/`pub` in `src/rules.rs`; the new check is additive
data (new `const`, new `CATALOG` entry, new fields or a parallel small table
on `LangRuleConfig`) plus a new walk function, not new plumbing.

## Node kinds for chained dot-access, per language

Verified by reading each vendored grammar's `node-types.json` directly
(`~/.cargo/registry/src/index.crates.io-*/tree-sitter-<lang>-<ver>/src/node-types.json`
— the exact crate versions pinned above), not guessed by analogy, matching
the repo's existing convention (`src/rules.rs:67-71`'s comment on
`LangRuleConfig`, and the per-grammar doc comments throughout `lang_config()`,
e.g. `src/rules.rs:220-224`, `:663-671`, `:673-676`).

| Language | Member/field-access node | Its fields | Call node | Its fields |
|---|---|---|---|---|
| Go | `selector_expression` | `operand` (`_expression`), `field` (`field_identifier`) | `call_expression` | `function` (`_expression`), `arguments` |
| JS | `member_expression` | `object` (`expression`), `property` | `call_expression` | `function` (`expression`), `arguments` |
| TS | `member_expression` (same shape as JS; TS grammar reuses/extends the JS one) | same as JS | `call_expression` | same as JS |
| Python | `attribute` | `object` (`primary_expression`), `attribute` (`identifier`) | `call` | `function` (`primary_expression`), `arguments` |
| Java | `field_access` | `object` (`primary_expression`), `field` (`identifier`) | `method_invocation` | `object` (`primary_expression`), `name`, `arguments` — **note**: unlike the other 6 languages, Java's call node *itself* carries the receiver (`object`) and method name directly; there's no intermediate member-access node wrapping a call's callee. |
| Kotlin (`tree-sitter-kotlin-ng` 1.1.0) | `navigation_expression` | **no named fields** — positional children `[expression, identifier]` (confirmed empty `"fields": {}` in node-types.json, consistent with the existing `kotlin_body`/`kotlin_params` positional-child helpers already in `src/rules.rs` for the same reason) | `call_expression` | **no named fields either** — fully positional |
| Rust | `field_expression` | `value` (`_expression`), `field` (`field_identifier`) | `call_expression` | `function` (`_literal`), `arguments` — **note**: Rust has no distinct "method call" node kind; `a.b().c()` is `call_expression(function: field_expression(value: call_expression(function: field_expression(...))))`, the same nesting shape Go/JS/Python use, just with Rust's own node names. Confirmed by grepping node-types.json for `method`/`call` — only `call_expression` exists. |

**Chain-walk shape (6 of 7 languages):** a fluent/dotted chain is a
`call_expression` (or Python's `call`) whose `function`/`function` field is a
member-access node (`selector_expression`/`member_expression`/`attribute`/
`field_expression`), whose own `object`/`operand`/`value` field is in turn
another `call_expression` or member-access node, recursing down. Each
member-access node or each call whose callee is a member-access node is one
"dot-hop" to count toward the ≥3 threshold.

**Java is the outlier**: `method_invocation` already bundles receiver +
method name in one node (no separate `field_access`-wrapping-`call_expression`
nesting) — so the Java walk counts consecutive `method_invocation.object`
chains (each one being either another `method_invocation` or a plain
`field_access`) rather than unwrapping `call_expression → field_access` like
the other languages. This needs its own small helper, analogous to how
`kotlin_body`/`kotlin_params`/`go_bool_params` etc. already special-case one
language's grammar shape inside an otherwise-shared per-language table
(`src/rules.rs:99-105`, `:196-206`).

**Kotlin's positional fields** mean the walk can't use
`child_by_field_name("object"/"value")` for `navigation_expression`/
`call_expression` — it must take the first named child (the receiver
expression) the same way `kotlin_body`/`kotlin_params` already do
(`src/rules.rs:99-105` doc comment references this exact pattern for Kotlin's
`function_declaration`/`anonymous_function`).

## Fluent-builder suppression heuristic — feasibility note

Per requirements.md's open design question (acceptance criterion 4): tree-sitter
gives a purely syntactic tree, no type checker, so "same return type as
receiver" cannot be resolved from the AST alone in any of these 7 languages.
None of the existing `LangRuleConfig` fields or grammars expose inferred types
— `bool_param_finder` is the closest analog and it only works because Java/
TS/Kotlin/Rust write the type as a syntactic annotation right next to the
parameter (a `type` field), not because tree-sitter infers anything.

Practical fallback heuristics available purely syntactically (for the plan
phase to choose among, not decided here):
1. **Same-method-name-family heuristic**: builder chains conventionally reuse
   short, lowercase, verb-like names (`filter`, `map`, `with*`, `set*`) — weak,
   language-specific, easy to false-positive/negative.
2. **Same-identifier-repeated-as-receiver heuristic**: unresolvable without
   types — chains like `a.b().c().d()` vs `a.map().filter().collect()` are
   syntactically identical shapes; nothing in the parse tree distinguishes a
   Law-of-Demeter violation from a fluent chain except what each hop returns.
3. **Defer to per-file naming/annotation conventions**: e.g. Java streams
   `.stream()...` are a recognizable syntactic prefix; Rust iterator adapters
   chain off `.iter()`/`.into_iter()` methods; these are language-specific
   "known fluent-API entry point" allowlists, not a general heuristic.

This confirms the requirements doc's own framing: full type-based "same
return type" resolution is infeasible from tree-sitter's syntactic AST alone,
and the plan phase must pick and document one syntactic proxy heuristic (or
explicitly scope v1 to only flag non-builder-shaped chains, e.g. by requiring
the property/field names among hops to be distinct rather than a known-fluent
verb pattern) rather than the literal "same type" check described in the
backlog item.

## Test infrastructure pattern to follow

Not `testdata/` fixture files — `testdata/` in this repo holds larger,
purpose-specific corpora (`testdata/comment-quality-corpus/`,
`testdata/dogfood-architecture/`), not per-check unit-test fixtures.
`deep-nesting` (and every other `src/rules.rs` check) instead uses **inline
`#[test]` functions with source strings embedded as Rust string literals**,
one test per language per behavior, e.g. `flags_deep_nesting` (Go,
`src/rules.rs:1258`), `ts_flags_deep_nesting` (:1583), `py_flags_deep_nesting`
(:1797), `java_flags_deep_nesting` (:1947), `kotlin_flags_deep_nesting`
(:2089), `rust_flags_deep_nesting` (:2227). Each builds a source string,
calls `check_source(src)` (or the TS/Java/etc. equivalent harness already
defined earlier in the same `#[cfg(test)] mod tests` block), and asserts a
finding's `message` contains the `[rule-id]` prefix. The new check should add
`{lang}_flags_hide_delegate` (flagged case) and a suppressed-builder-chain
counterpart per language, matching this exact naming and assertion style —
per acceptance criterion 5.

## Summary for the plan phase

- No new dependency; reuse pinned tree-sitter grammar crates as listed above.
- Node-kind table per language (to add to or parallel `LangRuleConfig`):
  Go `selector_expression`/`call_expression`; JS/TS `member_expression`/
  `call_expression`; Python `attribute`/`call`; Java `method_invocation`/
  `field_access` (single-node receiver+call, no unwrap needed); Kotlin
  `navigation_expression`/`call_expression` (positional fields only); Rust
  `field_expression`/`call_expression` (no distinct method-call node).
- Java and Kotlin each need a bespoke per-language helper (Java: no
  wrapping-call unwrap step; Kotlin: positional child access), following the
  existing precedent of `kotlin_body`/`kotlin_params`/`go_bool_params`-style
  per-language helper functions already in `src/rules.rs`.
- The builder-suppression heuristic as literally specified ("same return
  type") is not implementable from tree-sitter's syntactic AST in any of
  the 7 languages — this is a real open design question for `plan.md` to
  resolve with a syntactic proxy heuristic, not an implementation detail.
- Tests: inline `#[test]` fns per language in `src/rules.rs`, source-string
  fixtures, `check_source(...)`/lang-specific harness, assert on
  `finding.message.contains("[hide-delegate]")` — no `testdata/` files.
