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

## Target user, success measure and risky assumption

*(Added retroactively after the Product Triad Review; the original requirements named none of these.)*

- **Target user:** developers and AI coding agents using kibitzer hooks/CI in repos that contain shell scripts (install, bootstrap, CI glue). No transcript evidence of a specific shell-quality miss was gathered, so demand is assumed, not measured.
- **Success measure (beyond the wiring ACs below):** the CLAUDE.md "Writing a new check" done-bar: a corpus backtest with hand-triaged findings and a strict false-positive rate under the 40% target (ADR-001 bar, cited from the superseded plan at `7634824`; no ADR file exists). Recall and adoption are not measured.
- **Risky assumption:** that generic grammar-shaped rules (long-function, magic literals, comment heuristics), which worked for Go/Kotlin/Rust, are useful on shell default-on. The first triage disproved it for `comment-quality-bash` (90% FP) and `replace-magic-literal` (52.5% FP); see `docs/backtest-triage/bash-2026-10-18.md`.

## Acceptance Criteria

1. `Cargo.toml` declares `tree-sitter-bash` at a compatible version (0.25.x on crates.io). *(Reconciled: the backlog's acceptance criterion says 0.23.x and 0.23.3 shipped; both 0.23.3 and 0.25.1 avoid a `tree-sitter` conflict, see `research/stack.md`.)*
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

*(As shipped, also not covered: extension-less/shebang-only scripts, `.zsh`/`.ksh`/`.bats`/`.envrc`/dotfiles; `unreachable-code`, `long-parameter-list` and `flag-argument` cannot fire for bash by grammar; `deep-nesting` under-counts inside `elif`/`else`; `comment-quality-bash` runs only `verbose-comment`. See `plan.md` follow-ups.)*

- Shellcheck-equivalent semantic analysis (unquoted expansions, `set -e`, unchecked `$?`).
  Those are proxy/linter-territory per issue #14, not native tree-sitter checks.
- Bash-specific native checks analogous to `go-ignored-error` / `go-error-context`. The
  shape is acknowledged here (unquoted variable expansion, unchecked `$?`,
  `set -e`/`set -o pipefail`) but full design belongs in a follow-on issue.
- `.zsh`, `.fish`, or other shell dialects — v1 scope is bash/sh only.

## Dependencies

None — self-contained, same shape as Kotlin or Rust support was.
