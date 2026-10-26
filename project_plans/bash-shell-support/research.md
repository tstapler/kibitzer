# Research: bash-shell-support

## How language support currently works (verified against source)

### Language variant registration — `src/checker.rs`

`Language` is a plain Rust enum with 8 variants (Go through Rust). Adding a new language
requires touching **four** places in that single file:

1. The `enum Language` body — add the variant.
2. `Language::ALL` — add it to the exhaustiveness-checking constant.
3. `Language::ts_language()` — add the `tree_sitter_bash::LANGUAGE.into()` arm.
4. `Language::extensions()` — add `"sh"` and `"bash"` to the exhaustive match.

The doc comment on `Language::ALL` explicitly calls out this four-way update requirement and
the test `from_extension_recognizes_every_language_variant` that enforces it.

### Grammar-backed enum generation — `build.rs` + `codegen/node-types/*.json`

`build.rs` reads a `GRAMMARS` constant — one `(module_stem, EnumName, json_path)` triple per
grammar — and generates a `<Lang>Kind` enum in `$OUT_DIR`. Adding Bash requires:
- Vendoring `bash.json` to `codegen/node-types/bash.json`.
- Adding `("bash", "BashKind", "codegen/node-types/bash.json")` to `GRAMMARS`.
- Adding `pub mod bash { include!(…/bash_kind.rs); }` + `pub use bash::BashKind;` to
  `src/node_kind.rs`.

The generated `BashKind` enum is used by checkers for type-safe `node.kind()` comparisons,
replacing error-prone hand-typed strings.

### Syntax-rules checker — `src/checkers/rules.rs`

`lang_config(lang: Language) -> LangRuleConfig` is an exhaustive match — adding
`Language::Bash` without a `bash_lang_config()` arm will be a compile error, which is the
right forcing function. `LangRuleConfig` has ~20 fields; the ones that must be verified
against the actual bash grammar are:

- `function_kinds`: bash grammar's equivalent is `function_definition`.
- `if_kind`: `if_statement` in bash grammar.
- `nesting_kinds`: `for_statement`, `while_statement`, `c_style_for_statement`,
  `case_statement`, `subshell`.
- `else_wrapper_kinds`: `else_clause` in bash.
- `block_kind`: `compound_statement` (bash's `{ … }` block uses this node kind).
- `terminal_kinds`: `return_statement` only — bash has no typed `break`/`continue` node
  kind in tree-sitter-bash (needs explicit verification against `node-types.json`).
- `literal_kinds`/`numeric_literal_kinds`: bash string/number nodes; tree-sitter-bash
  uses `word`, `string`, `number` — verify against `node-types.json`.
- `param_counter`: bash functions have no formal parameter list in the syntax (arguments
  are positional `$1`/`$2` at call time); `function_definition` has no parameter-list child
  in tree-sitter-bash. The `long-parameter-list` rule will produce no findings for bash, but
  the config entry must still compile — use a `|_| 0` stub.
- `body_finder`, `params_finder`: bash's `function_definition` body is its last named child
  (a `compound_statement`); `params_finder` should return `None` always (no formal params).

### Comment quality checker — `src/checkers/comment_quality.rs`

`CommentQualityChecker::name_for()` has a match on `Language` — adding `Language::Bash`
there and in `tree_walk::comment_kinds` is the only required change. Bash comment node kind
in tree-sitter-bash is `comment` (verified: same as Go/Python/TS).

### Duplicate code checker — `src/checkers/duplicate_code.rs`

`DuplicateCodeChecker::file_globs()` is a plain `&[&str]` — append `"**/*.sh"` and
`"**/*.bash"`. No logic changes needed (hashing is language-agnostic).

### Default checks wiring — `src/config.rs`

Three helper functions need edits:
- `syntax_rules_checks()`: add `native_check("syntax-rules-bash", …, &["**/*.sh", "**/*.bash"])`.
- `comment_quality_checks()`: add `native_check("comment-quality-bash", …, &["**/*.sh", "**/*.bash"])`.
- `core_checks()`: add `"**/*.sh"` and `"**/*.bash"` to `duplicate-code` and
  `duplicate-code-cross-file` glob slices.

The `default_checks_still_registers_all_eight_syntax_rules_checkers` test hard-codes the
current 8 names — it must be updated to assert 9 (or renamed to reflect the count).

## tree-sitter-bash grammar facts (from crates.io + node-types.json research)

- Latest crates.io version: `0.23.3` (as of 2026-10). Published by tree-sitter-org.
  VERIFIED: the crate exposes `LANGUAGE` (type `tree_sitter::LanguageFn`), matching the
  pattern used by `tree-sitter-go::LANGUAGE.into()` etc. in `checker.rs`.
- The `tree-sitter` version dependency in `tree-sitter-bash 0.23.3` requires
  `tree-sitter ^0.23` — this repo uses `tree-sitter = "0.26"`. Semver compatibility:
  `0.26` satisfies `^0.23` because tree-sitter follows a pre-1.0 convention where the minor
  version is the breaking surface; check the actual `Cargo.lock` resolution before assuming
  compatibility.
  **Risk:** if `tree-sitter-bash 0.23.x` declares a lower `^0.x` bound that conflicts with
  the repo's `0.26` pin, a different bash grammar version or a lock upgrade may be needed.
  Resolve by trying `cargo add tree-sitter-bash` and inspecting the lock file.
- Bash-specific grammar quirks relevant to `LangRuleConfig`:
  - **No formal parameter list**: `function_definition` → `word` (name) + `compound_statement`
    (body). `long-parameter-list` will always return 0 — not an error, just a no-op rule.
  - **Heredocs**: parsed as `heredoc_body` nodes with their own string content — these do not
    pose a syntax-rules or comment-quality risk but may cause `replace-magic-literal` to
    fire on heredoc strings if literal_kinds includes `word`. Scope note: mitigate by
    narrowing `literal_kinds` to `"string"` and `"number"` only (not `"word"`), matching the
    proposal's intent to ship a baseline rather than comprehensive literal coverage.
  - **`break`/`continue`**: present in the bash grammar as `break_statement`/`continue_statement`
    — confirm in `node-types.json`; if present, include in `terminal_kinds`. If absent (like
    Kotlin), document and omit.
  - **Quoting complexity**: single-quoted strings, double-quoted strings, `$'...'` ANSI-C
    strings, and unquoted `word` tokens are all different node kinds. This is exactly why the
    proposal recommends verifying `node-types.json` before writing `LangRuleConfig`.

## Analogous precedents in this codebase

- **Kotlin** (most recent addition): Had missing `break`/`continue` node kinds, documented
  in `terminal_kinds`'s field comment. Same discipline applies to Bash.
- **Rust** (second-most-recent): Required `unwrap_statement: rust_unwrap_statement` to
  handle Rust's expression-statement wrapping — Bash will need a similar investigation for
  whether return/break appear directly or wrapped.

## Risk assessment

| Risk | Likelihood | Mitigation |
|------|-----------|------------|
| `tree-sitter-bash` version conflict with `tree-sitter 0.26` | Medium | Check `Cargo.lock` after `cargo add`; upgrade minor if needed |
| Bash grammar irregularity causing `LangRuleConfig` fields to be wrong | High | Mandatory `to_sexp()` / `node-types.json` verification step before writing config |
| `long-parameter-list` never firing (no formal params) | Certain (by design) | Document in code comment; not a defect |
| `replace-magic-literal` over-firing on `word` tokens | Medium | Scope `literal_kinds` to `string`/`number` only for v1 |
| Heredoc false positives in `comment-quality-bash` | Low–Medium | `comment_kinds` only returns `comment`; heredoc body is a different kind |
