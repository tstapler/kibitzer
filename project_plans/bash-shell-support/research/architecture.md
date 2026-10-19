# Architecture research: bash-shell-support (retroactive)

Scope: every site a new `Language` variant must touch, compared with what commits e8a422a and 1dc4dc3 changed. Method: `grep -rn "Language::Rust" src` vs `grep -rn "Language::Bash" src`, `git diff 7634824..HEAD`, and a sweep of `**/*.rs` glob lists. Labels: VERIFIED = command run against this tree; INFERRED = reasoning only.

## 1. Language-variant touch points

| Site | Kind | Compiler-enforced? | Landed? | Planned? |
|---|---|---|---|---|
| `src/checker.rs` enum, `Language::ALL`, `ts_language`, `extensions` | exhaustive match + manual `ALL` | match yes, `ALL` no (guarded by `checker.rs` tests) | yes | yes (plan Task 3) |
| `src/tree_walk.rs` `comment_kinds` | exhaustive match | yes | yes | yes (Task 4) |
| `src/checkers/rules.rs` `lang_config` + `bash_lang_config` | exhaustive match | yes | yes | yes (Task 5) |
| `src/checkers/comment_quality.rs:193` checker-name match | exhaustive match | yes | yes | partly (named in config wiring) |
| `src/config.rs` `file_size_checks`, `syntax_rules_checks`, `comment_quality_checks`, `core_checks` (duplicate-code x2) | glob lists | no (guarded by the `config.rs:1884` ALL-vs-registered test) | yes | yes |
| `src/checkers/duplicate_code.rs` `file_globs` | glob list | no | yes | yes |
| `src/checkers/file_size.rs` `lang_config` | exhaustive match | yes | yes | MISSED by plan |
| `src/symbol_extract.rs:288` `lang_symbol_config` | exhaustive match | yes | yes (empty kinds) | MISSED by plan |
| `src/inline_ignores/hint.rs:19` `comment_leader` | `Some(Python)` arm, `Some(_)` fallback is `//` | NO: wildcard arm silently gives bash `//` | yes (1dc4dc3, verify finding) | MISSED by plan |
| `src/inline_ignores/anchor_tests.rs` (rows at :51, :70, :91, :266) | test fixtures | no | yes | MISSED by plan |
| `src/inline_ignores/tests.rs:305` `fixture(lang)` | exhaustive match | yes | yes | MISSED by plan |
| `src/checkers/inline_ignore.rs:273` glob-count assertion (13 -> 15) | hard-coded count | test only | yes | MISSED by plan |
| `src/node_kind.rs` generated `BashKind` + `build.rs` + `codegen/node-types/bash.json` | codegen | no | yes (`#[expect(dead_code)]`) | partly |
| `docs/syntax-rules.md`, `docs/comment-quality.md` | doc tables | no | yes | yes |

VERIFIED: `grep Language::Rust` hits that have no `Language::Bash` twin are all tests, `import_graph.rs:92` (Rust-only module graph, correctly Bash-free), and `symbol_extract.rs:480,527` (Rust impl handling, correctly Bash-free).

## 2. Sites still missed or intentionally unhandled

- `src/arch_export.rs:21` `has_supported_extension` and `src/arch_model.rs:316` `language_for_path` both delegate to `Language::for_path`, so `.sh` now counts as "supported" but yields zero symbols. A repo of only shell scripts passes the "no supported languages" guard and exports an empty model. VERIFIED by reading both. No test pins this. The `symbol_extract.rs` comment documents the zero-symbol choice but not the export-guard consequence. Not fixed.
- `src/checkers/import_preamble.rs:15-30` maps extensions by string, not by `Language`. No bash arm; the checker is a no-op for `.sh`. Probably correct (shell has no import preamble), INFERRED; unrecorded.
- `src/import_graph.rs`, `src/declarations.rs`, `src/unreferenced_symbols.rs`, `src/lsp.rs:190`: all go through `Language::for_path`/`language_for_path`, so `.sh` files flow in with no symbols or imports. INFERRED benign, not tested.
- `docs/duplicate-code-fuzzy.md:94` example scope glob lists `*.rs` but not `*.sh`; illustrative only. `docs/affected-validation.md:79` lists the languages and omits Bash. VERIFIED by grep; both cosmetic.
- `.kts`/`.sh` split: `Kotlin` has two extensions, `Bash` has `sh` and `bash`. Extensions `zsh`, `ksh`, shebang-only scripts, and `.bashrc`-style names are not detected (INFERRED from `extensions()`; per requirements scope, not re-checked).

## 3. Which sites the original plan missed

The plan (`plan.md` Tasks 3-5) enumerated checker.rs, tree_walk.rs, rules.rs, and config wiring only. It said exhaustive matches would "enforce build failure otherwise", which is true for matches but not for the following, all found during implementation or sdd:6-verify:

1. `file_size.rs` (exhaustive match; build failure surfaced it).
2. `symbol_extract.rs` (exhaustive match; build failure surfaced it, forcing a design decision the plan never made).
3. `inline_ignores/hint.rs`: wildcard `Some(_)` arm compiled fine and silently emitted `//` hints for bash. This is the one miss the compiler could not catch, fixed in 1dc4dc3.
4. `inline_ignores/anchor_tests.rs` and `inline_ignores/tests.rs` fixtures, and the hard-coded `globs.len() == 13` in `inline_ignore.rs`.
5. `arch_model`/`arch_export` "supported but empty" semantics (still open, section 2).

Root cause (INFERRED): the plan enumerated sites by reading the `Language` doc comment in `checker.rs:65`, not by grepping `Language::Rust`. A checklist from that grep would have found sites 1-4 up front. Recommended reusable step for any future language: run `grep -rn "Language::Rust" src` and diff against the plan before implementing.

## 4. Existing architecture/hotspot analysis on rules.rs

- `project_plans/architecture-export/research/architecture.md:20` names `lang_config`/`LangRuleConfig` the "direct precedent" per-language table, and `LangSymbolConfig` was modelled on it. `project_plans/go-stdlib-idioms-fp/research/architecture.md:73,167` already added the `slog_key_predicate` exemption hook for Go and states per-language behaviour lives in `LangRuleConfig` fields. `project_plans/long-function-flat-literal/research/architecture.md` covers `check_declaration`. VERIFIED by grep.
- No `code-hotspot-analysis` output exists under `project_plans/` (VERIFIED: `ls project_plans | grep -i hotspot` is empty). Size is a proxy: `src/checkers/rules.rs` is 4490 lines (`wc -l`), with the language config at ~900 and `bash_lang_config` at ~1218-1290.

## 5. Tech Debt Disposition for adding a language to rules.rs

**Extend as-is.** `LangRuleConfig` is already the seam: one new `bash_lang_config()` arm plus `fn` hooks (`no_const_binding`, `no_params`, `zero_params`, `no_bool_params`) absorbed Bash's missing concepts without touching checker logic. Residual debt: `slog_key_predicate` was reused as a generic literal-exemption hook (`bash_is_expansion_string`, `rules.rs:1283`), so the field name now misdescribes its job; rename to `literal_exemption` the next time a fourth language needs it (cheap mechanical rename, 3 call sites plus the doc at `rules.rs:205,329`). Not worth doing in this change.
