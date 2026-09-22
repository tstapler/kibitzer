# Stack research: replace-magic-literal

Verification method: not guessed by analogy. Node-kind names below come from two sources,
both authoritative: (1) the vendored `node-types.json` files this repo already ships at
`codegen/node-types/*.json` (the exact grammar-shipped schema, used by `build.rs` to
codegen the `<Lang>Kind` enums in `src/node_kind.rs`), and (2) a real compiled `cargo test`
run (temporarily added to `src/rules.rs`'s test module, run via
`cargo test --bin kibitzer rules::tests::temp_probe_magic_literal_candidate_kinds -- --nocapture`,
then reverted — confirmed clean via `git diff --stat src/rules.rs`) that called
`tree_sitter::Language::id_for_node_kind` against each candidate string for all 7 linked
grammars, the same technique the existing `node_kind_literals_are_valid_for_their_grammar`
test at [`src/rules.rs:1171`](src/rules.rs#L1171) already uses to catch a typo'd kind name.
Every kind below returned a non-zero id (or, for the three anonymous-token exceptions noted,
was independently confirmed present in `node-types.json` with `"named": false`).

## 1. Tree-sitter grammar versions (from `Cargo.lock`) and literal node kinds

Resolved versions (`Cargo.lock`, matching `Cargo.toml`'s pins):

| Language | Crate | Resolved version |
|---|---|---|
| Go | `tree-sitter-go` | 0.25.0 |
| TypeScript / Tsx | `tree-sitter-typescript` | 0.23.2 (one crate, two grammars: `LANGUAGE_TYPESCRIPT`, `LANGUAGE_TSX`) |
| JavaScript | `tree-sitter-javascript` | 0.23.1 |
| Python | `tree-sitter-python` | 0.23.6 |
| Java | `tree-sitter-java` | 0.23.5 |
| Kotlin | `tree-sitter-kotlin-ng` | 1.1.0 |
| Rust | `tree-sitter-rust` | 0.24.2 |
| (core) | `tree-sitter` | 0.26.12 |

This repo already vendors each grammar's `node-types.json` under `codegen/node-types/`
(`go.json`, `typescript.json`, `tsx.json`, `javascript.json`, `python.json`, `java.json`,
`kotlin.json`, `rust.json` — 8 files for the 8 `Language` variants; TS and Tsx have separate
files despite sharing a crate, matching `build.rs`'s `GRAMMARS` table). These are the ground
truth for node kind names — more reliable than grammar.js/GitHub because they're the exact
version pinned, already in-repo, and load-bearing for the existing `<Lang>Kind` codegen.

### Literal node kinds (verified `named: true`, all confirmed real via the `cargo test` probe)

| Language | Numeric literal kinds | String literal kind(s) | Notes |
|---|---|---|---|
| Go | `int_literal`, `float_literal`, `imaginary_literal`, `rune_literal` | `interpreted_string_literal`, `raw_string_literal` | Matches existing usage in [`src/go_error_context.rs:88`](src/go_error_context.rs#L88), which already lists both string kinds. |
| TypeScript / Tsx | `number` (single unified kind — covers hex/octal/binary/bigint) | `string`, `template_string` | One `number` kind for everything; no `numeric_literal`. |
| JavaScript | `number` | `string`, `template_string` | Same grammar family as TS (`tree-sitter-typescript` crate reuses the JS grammar's literal shapes; `tree-sitter-javascript` is its own crate but structurally identical here). |
| Python | `integer`, `float` | `string` (also `concatenated_string` for adjacent-string-literal concatenation) | No `string_literal`/`numeric_literal` names — plain `integer`/`float`/`string`. |
| Java | `decimal_integer_literal`, `hex_integer_literal`, `octal_integer_literal`, `binary_integer_literal`, `decimal_floating_point_literal`, `hex_floating_point_literal` | `string_literal` (also `character_literal`, `null_literal`) | Matches existing usage in [`src/java_error_context.rs:75`](src/java_error_context.rs#L75) (`"string_literal" => has_message = true`). |
| Kotlin | `number_literal` (integers), `float_literal` | `string_literal`, `multiline_string_literal` (also `character_literal`) | `number_literal` and `float_literal` are separate concrete leaf kinds (verified: both appear as standalone top-level `node-types.json` entries with no `fields`/`children`, not `subtypes` groupings — the earlier appearance of both names inside an `_expression` subtypes list is just that list enumerating them as expression alternatives, not evidence they're abstract). |
| Rust | `integer_literal`, `float_literal` | `string_literal`, `raw_string_literal` (also `char_literal`, `boolean_literal`) | |

**Structural gotcha (verified via `node-types.json`, matters for extracting a literal's
comparison key):** every numeric-literal kind above is a flat leaf token (no `fields`, no
`children`) — `node.utf8_text(source)` gives the exact value directly. Every string-literal
kind, in every one of the 7 grammars, is a *compound* node with optional/required children
(`string_fragment`/`string_content`/`escape_sequence`/`interpolation`/`string_start`/
`string_end`, etc.) and **no fields at all** — there is no single child to pull a "value"
from. The safe, grammar-uniform approach (and the one this repo already uses elsewhere for
comparing spans) is `node.utf8_text(source)` on the *whole* string-literal node, quotes
included, as the comparison key. A string containing interpolation (Python f-string,
Kotlin/JS template literal, Java text block interpolation) is still the same node kind with
an `interpolation`/`template_substitution` child — worth excluding or at least noting as a
plan-phase decision, since two interpolated strings with the same template but different
interpolated values would (correctly) not compare equal under a raw-text key, but a checker
naively walking string-literal kinds would still visit the node.

## 2. Const/let/val-style binding node kinds (for the named-constant exclusion)

| Language | Declaration kind | Structure | Verified |
|---|---|---|---|
| Go | `const_declaration` | wraps one or more `const_spec` children, each with `name` (identifier(s)) and `value` (`expression_list`) fields | `id_for_node_kind` valid; structure from `node-types.json` |
| TypeScript/JS/Tsx | `lexical_declaration` | has a `kind` field whose child is the **anonymous** token `const` or `let` (`"named": false` in `node-types.json` — same pattern as this file's existing Kotlin `if_kind`/positional-child handling); wraps `variable_declarator` children, each with `name` and `value` fields | `lexical_declaration`/`variable_declarator` valid; `kind` field's `const`/`let` tokens confirmed anonymous in `node-types.json`, so detection must compare `child_by_field_name("kind").kind() == "const"`, not use `id_for_node_kind(_, true)` (which returns 0 for anonymous tokens by design) |
| Python | *(no true const)* | plain `assignment` node; only a naming convention (`ALL_CAPS`) signals intent | `assignment` kind confirmed valid; this is a policy call for the plan phase, not a grammar fact |
| Java | `local_variable_declaration` / `field_declaration` with a `final` modifier | `declarator` field → repeated `variable_declarator` (`name`/`value` fields); optional `modifiers` child node whose own children include the modifier keywords | `local_variable_declaration`, `field_declaration`, `variable_declarator`, `modifiers` all valid named kinds. **`final` itself is an anonymous token** (`node-types.json`: `{"type": "final", "named": false}`) — confirmed present but returns `id=0` from `id_for_node_kind(_, true)` as expected for any unnamed kind; detection needs `id_for_node_kind("final", false)` or a raw-children scan checking `.kind() == "final"`, not a named-children-only walk (the `modifiers` node's documented named children are only `annotation`/`marker_annotation` — keyword modifiers like `final`/`public`/`static` are real children but omitted from `node-types.json`'s `children.types` list, which only enumerates named types). |
| Kotlin | `property_declaration` with a `val` (not `var`) keyword | **No fields at all** (confirmed empty `"fields": {}` in `node-types.json`) — purely positional children: an anonymous `val`/`var` token, then `variable_declaration` (holds the name), then an `expression` sibling for the initializer | `property_declaration`/`variable_declaration` valid named kinds. `val`/`var` are anonymous tokens (`node-types.json`: `"named": false` for both) — same detection caveat as Java's `final`. This matches the file's existing precedent that Kotlin declarations are frequently positional-only (see `kotlin_body`/`kotlin_params` helpers already in `src/rules.rs` for the same pattern). |
| Rust | `const_item` / `static_item` | flat: both have `name` and `value` fields directly (no intermediate declarator node, unlike JS/Java) | both valid. (`let_declaration` also exists — `pattern`/`value` fields — for the non-const-but-conventionally-immutable case the requirements' "let-style binding" phrasing anticipates; whether to treat a `let` without a `mut` specifier as a named constant is a plan-phase scope decision, not a grammar fact.) |

## 3. New Cargo.toml dependencies needed

None. All 7 grammar crates (`tree-sitter-go`, `tree-sitter-typescript`, `tree-sitter-javascript`,
`tree-sitter-python`, `tree-sitter-java`, `tree-sitter-kotlin-ng`, `tree-sitter-rust`) plus
core `tree-sitter` are already `[dependencies]` in `Cargo.toml` and already used by
`src/rules.rs`'s existing `long-function`/`deep-nesting`/`long-parameter-list`/`flag-argument`/
`unreachable-code` rules via the same `LangRuleConfig`/`lang_config()` table
([`src/rules.rs:72-144`](src/rules.rs#L72-L144), [`src/rules.rs:487-703`](src/rules.rs#L487-L703)).
The vendored `node-types.json` files under `codegen/node-types/` (already present, already
consumed by `build.rs`) are sufficient ground truth for every node kind this feature needs —
no new vendoring required. Implementation is additive fields on `LangRuleConfig` (e.g.
literal-kinds list, const-binding-kind detector) plus a new rule branch in the existing
`SyntaxRulesChecker`'s `Check` impl — the same extension point `default_checks()` already
wires all 8 `syntax-rules-*` checker names into
([`src/config.rs:721-738`](src/config.rs#L721-L738)), so no new `default_checks()` entries are
needed either — `replace-magic-literal` rides the existing `syntax-rules-<lang>` checkers.
