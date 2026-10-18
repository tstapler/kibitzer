# Requirements: bash-shell-support

## Problem

Kibitzer has no shell-script language support: no `tree-sitter-bash` grammar dependency, no
`Language::Bash` variant, no `syntax-rules-bash` or `comment-quality-bash` checks, and
`duplicate_code.rs`'s `file_globs()` doesn't include `.sh`/`.bash`. Shell scripts are common
glue code (install scripts, CI pipelines, bootstrap tooling) and receive zero advisory
coverage today.

## Goal

Extend kibitzer's existing per-language native check families — `syntax-rules-*`,
`comment-quality-*`, and `duplicate-code` — to shell scripts (`.sh`, `.bash`), following the
same shape as Kotlin/Rust's rollout. Bash-specific semantic checks (unquoted variable
expansion, unchecked `$?`, `set -e`/`set -o pipefail` auditing) are explicitly out of scope
for v1.

## Acceptance Criteria

1. `Cargo.toml` declares `tree-sitter-bash` at a compatible version (0.25.x on crates.io).
2. `codegen/node-types/bash.json` is vendored from the same `tree-sitter-bash` release, and
   `build.rs`'s `GRAMMARS` table includes a `("bash", "BashKind", …)` entry so the
   `BashKind` enum is generated at build time.
3. `src/checker.rs` has a `Language::Bash` variant added to `Language`, `Language::ALL`,
   `Language::ts_language`, `Language::extensions` (covering `"sh"` and `"bash"`), and the
   `from_extension_recognizes_every_language_variant` regression test still passes.
4. `src/tree_walk.rs`'s `comment_kinds` function covers `Language::Bash`, returning the
   correct comment node kind(s) from `tree-sitter-bash`'s actual grammar (verified against
   `node-types.json` before coding, not assumed).
5. `src/checkers/rules.rs`'s `lang_config` function handles `Language::Bash` with a
   `bash_lang_config()` implementation. All grammar-specific fields (function kinds,
   block kind, nesting kinds, etc.) are verified from `tree-sitter-bash`'s `node-types.json`
   rather than assumed by analogy to other languages. Known grammar gaps (e.g. missing
   break/continue equivalent) are documented with caveat comments similar to the Kotlin
   precedent at `src/rules.rs:134-137`.
6. `syntax-rules-bash` and `comment-quality-bash` are registered in
   `src/config.rs`'s `syntax_rules_checks()` and `comment_quality_checks()` respectively,
   globbed to `**/*.sh` and `**/*.bash`.
7. `duplicate-code` and `duplicate-code-cross-file` glob lists in `src/config.rs`'s
   `core_checks()` include `**/*.sh` and `**/*.bash`.
8. The `default_checks_still_registers_all_eight_syntax_rules_checkers` test in `config.rs`
   is updated (or a new parallel test added) to assert `syntax-rules-bash` is present.
9. At least one unit test in `src/checkers/rules.rs` (or a dedicated `bash` test file)
   exercises `syntax-rules-bash` against a minimal shell function with a long-parameter-list
   or deep-nesting scenario to verify the grammar wiring is correct end-to-end.
10. `cargo build` and `cargo test` pass with no new warnings or test failures after the
    change.

## Out of Scope

- Shellcheck-equivalent semantic analysis (unquoted expansions, `set -e`, unchecked `$?`).
  Those are proxy/linter-territory per issue #14, not native tree-sitter checks.
- Bash-specific native checks analogous to `go-ignored-error` / `go-error-context`. The
  shape is acknowledged here (unquoted variable expansion, unchecked `$?`,
  `set -e`/`set -o pipefail`) but full design belongs in a follow-on issue.
- `.zsh`, `.fish`, or other shell dialects — v1 scope is bash/sh only.

## Dependencies

None — self-contained, same shape as Kotlin or Rust support was.
