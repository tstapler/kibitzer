# Implementation Plan: bash-shell-support

**Feature**: Shell-script (`.sh`, `.bash`) support for `syntax-rules-bash`, `comment-quality-bash`, `duplicate-code`, `duplicate-code-cross-file` (plus `bash-file-size`), via `tree-sitter-bash`.
**Date**: 2026-10-18 (retroactive plan of record; the original forward-looking plan is superseded by this file)
**Status**: Implemented; FP triage done and rules tuned afterwards (see "Update after the Product Triad Review" at the end); fresh-sample check recorded in `docs/backtest-triage/bash-2026-10-18.md`. Commits `e8a422a` (feat) and `1dc4dc3` (sdd:6-verify fixes) on `backlog/kibitzer-bash-shell-support`. PR/ship step not yet done.
**Complexity**: 2 (no `Complexity` field in `requirements.md`; defaulted)
**ADRs**: None. No non-standard technology choice was made (same grammar-crate + `LangRuleConfig` shape as Kotlin/Rust).

Evidence labels: **VERIFIED** = file opened or command run in this worktree on 2026-10-18 (cited); **INFERRED** = reasoned, not executed; **UNVERIFIED** = reported by someone else, not reproduced.

Sources: `requirements.md`, `research.md` (original; partly wrong, see Corrections), `research/{stack,architecture,pitfalls,build-vs-buy}.md` (retroactive), `validation.md`.

---

## Corrections to the original plan and research

The original `plan.md`/`research.md` were written before implementation. These items were wrong or incomplete; each is checked against the tree or `research/*.md`.

| # | Original claim | Reality | Evidence |
|---|---|---|---|
| C1 | `tree-sitter-bash 0.23` may require `tree-sitter ^0.23`, so it might conflict with the repo's `tree-sitter 0.26` (rated Medium risk, "critical path"). | No conflict. The grammar crate depends only on `tree-sitter-language ^0.1`; `tree-sitter` is a dev-dependency only. Risk was nil. | VERIFIED `research/stack.md` (registry `Cargo.toml` of 0.23.3 and 0.25.1); `Cargo.lock` resolves `tree-sitter 0.26.12`, `tree-sitter-bash 0.23.3` (grep of `Cargo.lock`). |
| C2 | `terminal_kinds: ["return_statement"]`, `return_kind: "return_statement"`; `break_statement`/`continue_statement` "probably exist". | None of `return_statement`, `break_statement`, `continue_statement`, `until_statement` exist. `return`/`break`/`continue` parse as `command`; `until` parses as `while_statement`. Landed: `terminal_kinds: &[]` (so `unreachable-code` never fires); `return_kind` is a never-matching placeholder. | VERIFIED `src/checkers/rules.rs:1236-1241,1273,1278`; `research/stack.md`. |
| C3 | `else_wrapper_kinds: &["else_clause"]`. | Landed `else_wrapper_kinds: &[]`. Bash's `if_statement` is flat: then-statements, `elif_clause` and `else_clause` are siblings of the condition, not `consequence`/`alternative`. | VERIFIED `src/checkers/rules.rs:1263,1244-1247`; `if_branches` at `rules.rs:1897-1911`. |
| C4 | `bash_body` helper takes the "last named child" of `function_definition`. | Hazardous: `function_definition` has an optional `redirect` child that can come last, and the body may be a `compound_statement`, `if_statement`, `subshell` or `test_command`. Landed code uses the grammar's `body` field via the existing `field_body`; no `bash_body` helper exists. | VERIFIED `src/checkers/rules.rs:1266`; `research/stack.md` (field list from `bash.json`). |
| C5 | `param_counter: \|_\| 0`, `params_finder: \|_\| None`, `bool_param_finder: \|_, _\| vec![]` inline closures. | Landed as named fns `zero_params`, `no_params`, `no_bool_params`, `no_const_binding`. | VERIFIED `src/checkers/rules.rs:1198-1214`. |
| C6 | Version: "latest crates.io is 0.23.3". Requirements AC1 says "0.25.x". | 0.25.1 also exists (in the local registry cache) and has the same 62 named node kinds; 0.23.3 was chosen and shipped. The issue/AC text (0.25.x) and the dependency (0.23) disagree; both work with `tree-sitter 0.26`. A bump to 0.25 is INFERRED to compile (not tried) and would require re-vendoring `bash.json`. | VERIFIED `research/stack.md` (`cmp` of vendored JSON vs 0.23.3; 112 diff lines vs 0.25.1; named-kind set identical). |
| C7 | The plan listed 5 files to touch (`checker.rs`, `tree_walk.rs`, `rules.rs`, `config.rs`, `Cargo.toml`/`build.rs`/`node_kind.rs`) and `validation.md` added `duplicate_code.rs`. | At least 8 further sites needed edits: `file_size.rs`, `symbol_extract.rs`, `inline_ignores/hint.rs`, `inline_ignores/anchor_tests.rs`, `inline_ignores/tests.rs`, `checkers/inline_ignore.rs` (glob count 13 -> 15), `checkers/comment_quality.rs` (name arm), docs. Root cause (INFERRED): sites were enumerated from the `Language` doc comment instead of `grep -rn "Language::Rust" src`. | VERIFIED `git show --stat e8a422a 1dc4dc3`; `research/architecture.md` section 1 and 3. |
| C8 | Task 7: add a test asserting `syntax-rules-bash` is in the eight-name test. | Landed: test renamed `default_checks_still_registers_all_nine_syntax_rules_checkers`. `e8a422a` also (wrongly) listed `comment-quality-bash` in that syntax-rules-only list; `1dc4dc3` removed it. | VERIFIED `git show 1dc4dc3 -- src/config.rs`; `cargo test --bin kibitzer default_checks` -> `...nine_syntax_rules_checkers ... ok`. |
| C9 | AC9 / Task 7: smoke test with long-parameter-list or deep-nesting. | `long-parameter-list` cannot fire for bash (no formal params); the test uses deep-nesting. | VERIFIED `rules.rs:3317-3339` (`bash_flags_deep_nesting`). |
| C10 | Task 8: record backtest results in `docs/backtest-triage/` or a `rules.rs` comment. | Not done as specified. Results exist only in the `e8a422a` commit message and this plan. No bash entry under `docs/backtest-triage/`. | VERIFIED `ls docs/backtest-triage` (no bash directory); commit message text. |
| C11 | `research/architecture.md` section 1 says the `hint.rs` fix landed in `1dc4dc3`. | It landed in `e8a422a` (the `1dc4dc3` stat has no `hint.rs`). Minor correction to the research doc, not to the code. | VERIFIED `git show --stat 1dc4dc3` file list. |

---

## Domain Glossary

| Term | Definition | Notes |
|------|-----------|-------|
| `Language::Bash` | `Language` variant for shell scripts; extensions `sh`, `bash` | `src/checker.rs:79,94,107,137` |
| `BashKind` | Build-time generated enum of tree-sitter-bash named node kinds | From `codegen/node-types/bash.json`; unused by checkers (they use string kinds), kept via `#[expect(dead_code)]` / `#[expect(unused_imports)]` (`src/node_kind.rs`) |
| `bash_lang_config()` | The `LangRuleConfig` for `syntax-rules-bash` | `src/checkers/rules.rs:1250` |
| `LangRuleConfig` | Per-language table of grammar node kinds and hook fns driving `SyntaxRulesChecker` | Pre-existing; extended, not changed |
| `slog_key_predicate` | `LangRuleConfig` literal-exemption hook (name is a misnomer; originally Go slog keys) | Reused for bash via `bash_is_expansion_string` |
| pure-expansion string | A bash `string` whose only content is `$var`/`${..}`/`$(..)`/`$((..))` (`"$@"`, `"${1}"`) | Exempt from `replace-magic-literal`; `"Usage: $0"` is mixed and still flagged |
| `syntax-rules-bash` / `comment-quality-bash` / `bash-file-size` | Native per-file checks registered in `default_checks()`, Advisory, globs `**/*.sh`, `**/*.bash` | `src/config.rs:928,957,987` |
| `ANCHOR_FIXTURES` | Per-checker fixtures proving an inline-ignore directive suppresses a finding | `src/inline_ignores/anchor_tests.rs` |
| `CommentLeader` | Comment prefix used when kibitzer emits an inline-ignore hint | `#` for Bash (`src/inline_ignores/hint.rs:19`) |

9 terms.

---

## Pattern Decisions

CREATIVE pass (retroactive; the first two were considered at research time, per `research/build-vs-buy.md`):

| Approach | Strength | Weakness | Outcome |
|---|---|---|---|
| A. Extend native tree-sitter checks (new `Language` variant + `LangRuleConfig` arm) | Reuses all checkers, inline-ignore, daemon cache; zero install; default catalog standard in `CLAUDE.md` | Several rules are structural no-ops for bash; real-world noise unproven until backtest | **Chosen** |
| B. shellcheck via a `command` check / plugin | Covers exactly what v1 excludes (unquoted expansion, `$?`, `set -e`) | External install; GPL binary cannot be bundled (INFERRED); plugins run in every repo | Deferred to follow-up (see Open follow-ups) |
| C. Hand-rolled shell parser | No vendored grammar | Highest-risk component; bypasses `BashKind`/inline-ignore/symbol plumbing | Rejected |

| Component | Pattern Chosen | Source | Alternative Rejected | Reason |
|-----------|---------------|--------|---------------------|--------|
| `bash_lang_config()` | Table-driven Strategy via `LangRuleConfig` | existing repo pattern (Kotlin/Rust) | A bespoke `BashSyntaxChecker` | One table row absorbs bash's missing concepts through no-op hook fns; no checker-logic change (VERIFIED diff: `rules.rs` +132 lines, all in config/helpers/tests) |
| Missing params / const bindings | Null Object fns (`no_params`, `zero_params`, `no_bool_params`, `no_const_binding`) | GoF (Null Object) | `Option` fields on `LangRuleConfig` | Keeps struct shape unchanged for 8 existing languages |
| Expansion-string exemption | Reuse existing exemption hook (`slog_key_predicate`) | Fowler (don't add a seam until it recurs) | New `literal_exemption` field | Avoids touching all 8 configs; rename is a deferred mechanical cleanup |
| Symbol extraction for bash | Null kinds (empty `type_kinds`/`interface_kinds`/`function_kinds`) | Null Object | Real shell function extraction | Architecture model for shell is out of scope; zero symbols is explicit in a comment (`symbol_extract.rs:291-293`) |
| Node kinds in config | String kinds, not `BashKind` | `// SEAM(typed-node-kind-migration)` per ADR-001 | `BashKind` typed matching | `LangRuleConfig` fields are `&str` and cross-grammar-shared |

---

## Tech Debt Disposition

| Area | Existing Issue | Disposition | Justification |
|------|----------------|--------------|----------------|
| `src/checkers/rules.rs` / `LangRuleConfig` | 4490-line file; ~20-field per-language table | **Extend as-is** | `LangRuleConfig` is the established seam (VERIFIED `project_plans/architecture-export/research/architecture.md:20`, `go-stdlib-idioms-fp/research/architecture.md:73,167`): one new config fn plus no-op hooks, no checker logic touched, no new instance of an existing violation. No hotspot-analysis output exists under `project_plans/` (VERIFIED, `ls`); file size is the proxy. |
| `slog_key_predicate` field | Name misdescribes its job now that bash reuses it as a generic literal-exemption hook (`rules.rs:1283`) | **Extend as-is**, rename deferred | Rename to `literal_exemption` when a fourth language needs it; cheap mechanical change (3 call sites + docs at `rules.rs:205,329` per `research/architecture.md`). The reuse is documented in the `bash_is_expansion_string` doc comment (`rules.rs:1218`). |
| `Language` fan-out (13+ sites; some wildcard arms) | `hint.rs`'s `Some(_) => "//"` fallback compiled silently for bash | **Extend as-is** for this change; checklist recorded | Fix the instance (done, `hint.rs:19`); the reusable guard is a process step: run `grep -rn "Language::Rust" src` and diff against the plan before implementing any new language. |

---

## Unresolved Questions

- [ ] Reconcile `requirements.md` AC1 ("0.25.x") with the shipped dependency (0.23.x), either by editing the AC or bumping the crate. Not blocking; both work with `tree-sitter 0.26`. Owner: Tyler.
- [ ] Whether to commit backtest triage output under `docs/backtest-triage/` (original Task 8 asked for it). Owner: Tyler.

## Risk Control

- **Feature flag**: not gated. Checks are Advisory and disable per repo via `.kibitzer/inspect.json` (`docs/suppressing-checks.md`).
- **Rollback**: standard revert of `e8a422a` and `1dc4dc3`.
- **Staged rollout**: full rollout on merge; release via `cargo-dist` tag per `CLAUDE.md`.
- **Observability**: none new (no new service boundary or >100ms operation; INFERRED, no timing measured).

## Dependency Visualization

```
T1 dep ──> T2 codegen/BashKind ──> T3 Language::Bash ──┬─> T4 comment_kinds + cq name arm
                                                       ├─> T5 bash_lang_config ──> T11 expansion exemption ──> T12 if-chain test
                                                       ├─> T7 file_size.rs
                                                       ├─> T8 symbol_extract.rs
                                                       └─> T9 inline-ignore plumbing (hint/anchor/tests/glob count)
                  T3..T8 ──> T6 default_checks wiring ──> T10 tests ──> T13 docs ──> T14 backtest (informs T11)
```

Every exhaustive `match` on `Language` fails to compile until T3 is followed by T4, T5, T7, T8 (VERIFIED by `research/architecture.md`: those surfaced as build failures). T9 is the exception: not compiler-enforced.

---

## Phase 1: Native shell support (all tasks Done)

### Epic 1.1: Grammar and language registration

#### Story 1.1.1: Grammar dependency and `Language::Bash`
**As a** kibitzer user, **I want** `.sh`/`.bash` files recognized as a language, **so that** per-language checks can run on them.

**Acceptance criteria**
- AC1 `Cargo.toml` declares `tree-sitter-bash`.
  - *Given* `Cargo.toml` with `tree-sitter = "0.26"`, *When* `tree-sitter-bash = "0.23"` is added and built, *Then* `Cargo.lock` holds one `tree-sitter 0.26.12` and `tree-sitter-bash 0.23.3`.
- AC2 `bash.json` vendored and `BashKind` generated.
  - *Given* `codegen/node-types/bash.json` copied from the 0.23.3 crate, *When* `cargo build` runs `build.rs`, *Then* `BashKind` exists and the JSON is byte-identical to `tree-sitter-bash-0.23.3/src/node-types.json`.
- AC3 `Language::Bash` registered.
  - *Given* `Language::ALL` containing `Bash`, *When* `Language::for_path("deploy.sh")` and `("lib.bash")` run, *Then* both return `Some(Language::Bash)` and `from_extension_recognizes_every_language_variant` passes.

**Files**: `Cargo.toml`, `Cargo.lock`, `build.rs`, `codegen/node-types/bash.json`, `src/node_kind.rs`, `src/checker.rs`

##### Task T1 - Add `tree-sitter-bash` dependency. Cost: XS. **Done**
- Landed in `e8a422a`: `Cargo.toml` (`tree-sitter-bash = "0.23"`), `Cargo.lock` (+11 lines).
- Resolved: `tree-sitter-bash 0.23.3`, `tree-sitter 0.26.12`. No version-conflict diagnosis was needed (C1).
- Verified by: `grep -n -A2 'name = "tree-sitter-bash"' Cargo.lock` -> `version = "0.23.3"`; `cargo build` finishes with no warnings (run 2026-10-18).

##### Task T2 - Vendor `bash.json`, wire `build.rs` and `node_kind.rs`. Cost: S. **Done**
- `e8a422a`: `codegen/node-types/bash.json` (2801 lines), `build.rs` `GRAMMARS` entry `("bash", "BashKind", ...)`, `src/node_kind.rs` `pub mod bash` + `pub use bash::BashKind`.
- `1dc4dc3`: `#[allow(dead_code)]`/`#[allow(unused_imports)]` changed to `#[expect(...)]` (`src/node_kind.rs`), so a future use of `BashKind` forces removing the suppression.
- Node-kind findings (the original Task 2b) are recorded in the `bash_lang_config` doc comment (`rules.rs:1236-1249`) and match C2-C4.
- Verified by: `cmp codegen/node-types/bash.json <registry>/tree-sitter-bash-0.23.3/src/node-types.json` identical (`research/stack.md`); `cargo build` clean.

##### Task T3 - Add `Language::Bash`. Cost: XS. **Done**
- `e8a422a`, `src/checker.rs`: enum variant (:79), `Language::ALL` (:94), `ts_language` (:107), `extensions` -> `["sh","bash"]` (:137).
- Verified by: `cargo test checker::tests::from_extension_recognizes_every_language_variant` -> `ok` (run 2026-10-18).

### Epic 1.2: Checker wiring

#### Story 1.2.1: `syntax-rules-bash`
**As a** maintainer of shell scripts, **I want** complexity and magic-literal advisories, **so that** long, deeply nested or repetitive scripts surface.

**Acceptance criteria**
- AC4 Comment kind is `comment`.
  - *Given* a file `# note\nf() { :; }\n`, *When* `comment_kinds(Language::Bash)` is consulted, *Then* it returns `["comment"]` and the `# note` line is a comment node.
- AC5 `bash_lang_config` is verified against `bash.json` with caveat comments.
  - *Given* `bash.json` lacking `return_statement`, *When* `bash_lang_config()` is read, *Then* `terminal_kinds` is empty and the doc comment at `rules.rs:1236` names each absent kind and its consequence.
- AC9 End-to-end rule test.
  - *Given* a function nesting `for`/`while` five levels deep, *When* `SyntaxRulesChecker` walks it, *Then* one finding contains `[deep-nesting]`; a 3-line `greet()` produces none.

##### Task T4 - Comment kinds and comment-quality name arm. Cost: XS. **Done**
- `e8a422a`: `src/tree_walk.rs:13` (`Language::Bash` joins the `&["comment"]` arm); `src/checkers/comment_quality.rs:194` name arm `"comment-quality-bash"`.
- Verified by: compiles (exhaustive matches); `cargo test --bin kibitzer` -> 1704 passed. Pitfalls run: `# shellcheck disable=...`, `#!/usr/bin/env bash` and heredoc `#` lines produce no `comment-quality-bash` finding (VERIFIED `research/pitfalls.md` section 4).

##### Task T5 - `bash_lang_config()` and `lang_config` arm. Cost: M. **Done**
- `e8a422a`, `src/checkers/rules.rs`: `Language::Bash => bash_lang_config()` (:906); helpers (:1196-1214); config (:1250-1285).
- Landed values that differ from the original sketch: `else_wrapper_kinds: &[]`, `terminal_kinds: &[]`, `body_finder: field_body`, named null-object fns (C2-C5). `function_kinds: ["function_definition"]`, `if_kind: "if_statement"`, `nesting_kinds: [for_statement, c_style_for_statement, while_statement, case_statement, subshell]`, `block_kind: "compound_statement"`, `literal_kinds: ["string","number"]` (not `word`), `call_kinds: ["command"]`, `operator_kinds: ["binary_expression"]`, `density_boundary_kinds: ["subshell"]`.
- Rules that cannot fire for bash (documented in code and `docs/syntax-rules.md`): `long-parameter-list`, `flag-argument` (no params); `unreachable-code` (no terminal kinds); numeric `replace-magic-literal` is nearly inert because digits parse as `word` (VERIFIED by run: `[ "$n" -gt 42 ]`, `8080`, `$((a+7))` produce zero findings, `research/pitfalls.md` section 1).
- Verified by: tests `lang_config_dispatches_to_the_matching_constructor` and the `assert_valid(...Language::Bash...)` check at `rules.rs:2265-2268` (these assert every config node kind exists in the grammar); both inside the passing 1704.

##### Task T11 - Expansion-string exemption for `replace-magic-literal`. Cost: S. **Done** (two stages)
- `e8a422a`: exempt any `string` with an expansion child, after the backtest (see Backtest). `bash_is_expansion_string` plugged into `slog_key_predicate: Some(...)`.
- `1dc4dc3`: narrowed to pure-expansion strings (all named children are expansions and their bytes equal the string minus the two quotes), because the broad rule hid `"Usage: $0"`-style strings (`rules.rs:1216-1234`).
- Verified by: `bash_flags_repeated_plain_string_and_mixed_expansion_string` (3x `"linux"` -> 1 finding; 3x `"Usage: $0"` -> 1 finding) and `bash_exempts_repeated_pure_expansion_string` (3x `"$@"` + 3x `"${1}"` -> 0). `cargo test bash` -> 5 passed (run 2026-10-18).

##### Task T12 - Lock in the if-chain nesting limitation. Cost: XS. **Done**
- `1dc4dc3`: test `bash_deep_nesting_inside_else_branch_is_not_yet_counted` (`rules.rs:3369-3380`) and the corrected doc comment: `if_branches` inspects only the first two non-condition children, so nesting inside later then-statements/`elif`/`else` is under-counted (false negatives only). The original comment named `walk_if_chain`, which is the wrong function.
- Verified by: `cargo test bash` -> `bash_deep_nesting_inside_else_branch_is_not_yet_counted ... ok`.

#### Story 1.2.2: Default-catalog wiring and sibling per-language tables
**As a** kibitzer user, **I want** the bash checks on by default with no config, **so that** shell scripts get coverage like other languages.

**Acceptance criteria**
- AC6 Checks registered with bash globs.
  - *Given* `default_checks()`, *When* names are listed, *Then* `syntax-rules-bash`, `comment-quality-bash` and `bash-file-size` appear, each scoped to `["**/*.sh","**/*.bash"]`.
- AC7 Duplicate-code globs.
  - *Given* the `duplicate-code` and `duplicate-code-cross-file` checks, *When* their scope is read, *Then* both include `**/*.sh` and `**/*.bash`, and `DuplicateCodeChecker::file_globs()` matches.
- AC8 Registration regression test.
  - *Given* the test `default_checks_still_registers_all_nine_syntax_rules_checkers`, *When* run, *Then* it asserts `syntax-rules-bash` is registered and passes.

##### Task T6 - Default checks wiring. Cost: S. **Done**
- `e8a422a`, `src/config.rs`: `core_checks()` (both duplicate-code globs, :831,:848), `syntax_rules_checks()` (:957), `comment_quality_checks()` (:987); `src/checkers/duplicate_code.rs` `file_globs()` (+2 globs, closes the gap `validation.md` found).
- `1dc4dc3`: `file_size_checks()` gained `bash-file-size` (:928). It was omitted in `e8a422a`, which left bash files with a `file_size.rs` arm but no default check, so the arm was unreachable by default.
- Verified by: `cargo test --bin kibitzer default_checks` -> `...nine_syntax_rules_checkers ... ok` and `...ScopeInlineIgnoreToGrammarExtensionsAndMd_When_Read ... ok`. Gap: no test asserts `comment-quality-bash` or `bash-file-size` is registered (grep of `src/config.rs` finds them only at :987 and :928); inline-ignore anchor fixtures exercise both indirectly (T9).

##### Task T7 - `file_size.rs` arm. Cost: XS. **Done** (missed by original plan)
- `e8a422a`, `src/checkers/file_size.rs`: `Language::Bash => LangConfig { name: "bash-file-size", file_globs: ["**/*.sh","**/*.bash"] }`. Surfaced as a build failure (exhaustive match).
- Verified by: compiles; anchor fixture `("bash-file-size", "x.sh", ...)` in `anchor_tests.rs` (`1dc4dc3`).

##### Task T8 - `symbol_extract.rs` arm. Cost: XS. **Done** (missed by original plan)
- `e8a422a`, `src/symbol_extract.rs:291-300` (arm at :294): `LangSymbolConfig` with empty `type_kinds`/`interface_kinds`/`function_kinds`, so a `.sh` file yields zero symbols rather than half-supported behavior. This forced a design decision the original plan never made.
- Verified by: compiles. Consequence not tested (see Open follow-ups: `arch_export`/`arch_model`).

##### Task T9 - Inline-ignore plumbing. Cost: S. **Done** (missed by original plan)
- `e8a422a`: `src/inline_ignores/hint.rs:19` (`Some(Language::Python | Language::Bash)` -> `#`; the `Some(_)` wildcard would otherwise emit `//` hints for bash with no compile error, the only miss the compiler could not catch); `src/inline_ignores/anchor_tests.rs` (`syntax-rules-bash` and `comment-quality-bash` fixtures, `comment_syntax` `"py" | "sh" | "bash"` -> `# `); `src/inline_ignores/tests.rs:305` (`fixture(Language::Bash)`, exhaustive match); `src/checkers/inline_ignore.rs:273` (`globs.len()` 13 -> 15).
- `1dc4dc3`: added the `bash-file-size` anchor fixture.
- Verified by: `cargo test inline_ignore` -> 197 passed in the bin target (run 2026-10-18). Gap: no unit test pins `comment_leader` for Bash directly (the only reference is `hint.rs:59`; grep `comment_leader` over `src`).

### Epic 1.3: Verification and documentation

##### Task T10 - Tests. Cost: S. **Done**
- `e8a422a`: `bash_flags_deep_nesting`, `bash_allows_a_simple_function`, `check_bash_source` helper, `assert_valid` and dispatch-test Bash arms, `config.rs` rename to `..._nine_...`.
- `1dc4dc3`: `bash_flags_repeated_plain_string_and_mixed_expansion_string`, `bash_exempts_repeated_pure_expansion_string`, `bash_deep_nesting_inside_else_branch_is_not_yet_counted`; removed the stray `comment-quality-bash` entry from the syntax-rules name list.
- AC10 evidence (2026-10-18): `cargo build` finishes clean; `cargo clippy --all-targets` -> 0 `warning` lines; `cargo test --bin kibitzer` -> `1704 passed; 0 failed`. A first full `cargo test` run failed once in `backtest::tests::backtest_isolates_the_duplicate_cross_file_index_and_still_finds_duplicates` ("isolation env var must be restored"); it passed alone and in the next full run. INFERRED: a process-environment race between parallel tests, unrelated to bash (not confirmed on `master`).

##### Task T13 - Docs. Cost: XS. **Done**
- `e8a422a`/`1dc4dc3`: `docs/syntax-rules.md` (table row + "Bash caveats" section, :212 onward), `docs/comment-quality.md` (table row :25).
- Not updated (cosmetic, VERIFIED by grep in `research/architecture.md` section 2): `docs/affected-validation.md:79` language list, `docs/duplicate-code-fuzzy.md:94` example glob.

##### Task T14 - Backtest. Cost: S. **Done with caveats**
See Backtest record.

---

## Backtest record

Required by `CLAUDE.md` ("Writing a new check"). The numbers below are sample-specific; do not compare them across rows.

| Run | Sample | Result | Status |
|---|---|---|---|
| Implementer, run 1 (no expansion exemption) | First 150 paths of a `find ~/dotfiles ~/code -name '*.sh'` list (head -400, then head -150; list not committed) | 276 `syntax-rules-bash` finding lines in total (all rules, not only `replace-magic-literal`) | VERIFIED by the implementer this session (output of `kibitzer check native syntax-rules-bash` per file, `wc -l`); not reproducible from the repo since the list is not committed |
| Implementer, run 2 (any-expansion exemption) | same 150 paths | 85 lines: 56 `replace-magic-literal` + 29 `long-function` | VERIFIED by the implementer (same method) |
| Implementer, run 3 (pure-expansion exemption, `1dc4dc3`) | same 150 paths | 106 lines (up from 85 because mixed `"text $var"` strings are flagged again; the 21-line rise is not broken down by rule) | VERIFIED total by the implementer; the cause is INFERRED. Commit `e8a422a` message says "276 -> 85" without saying these were all-rule totals |
| Pitfalls agent re-run | `find ~/dotfiles ~/code -name '*.sh' -size +4k` over kubernetes, cassandra and this repo's own scripts | 300 findings pooled: 106 `long-function`, 192 `replace-magic-literal`, 2 `extract-variable` | VERIFIED by that agent (`research/pitfalls.md` section 2); different sample and filter |

Notes:
- The pitfalls re-run did not reproduce the implementer's 276/85/106 and cannot, because the samples differ. The "106" in the pitfalls table is the `long-function` count on its own sample, not the narrowed magic-literal figure; they coincide by accident.
- The standard backtest corpus (`docs/backtest-repos.md`) has no shell-centric repo (INFERRED from `research/build-vs-buy.md`); kubernetes and cassandra supplied the shell scripts. `kibitzer check backtest` transcript replay for bash was not run (no record of it).
- Remaining noise classes found by the pitfalls run (VERIFIED, `research/pitfalls.md` section 2): the POSIX empty-string idiom `[ "x$VAR" = "x" ]` (`"x"` flagged 3 to 12 times per file in cassandra `.build/run-python-dtests.sh:82`, `run-tests.sh:28`); `"\n"` separators; `"y"`/`"n"` prompt answers; repeated `uname`/`case` platform strings (mostly true positives by rule definition); an 80-line `echo "Usage: $0"` function yielding one finding whose message lists all 80 line numbers. `long-function` is the volume driver (106 of 300); the 40-line threshold is shared with other languages and was not tuned for shell (INFERRED too tight).
- The ADR-001 "<40% FP rate" target (original Task 8) was never computed; no true/false triage exists for bash under `docs/backtest-triage/` or via `scripts/backtest-triage.py`. This is the main remaining gap in the done-definition from `CLAUDE.md`.
- Verified clean: `duplicate-code` and `duplicate-code-cross-file` gave no finding on a shared 6-line shell header or a 5-line block repeated 3x; heredoc bodies produce no `replace-magic-literal` or comment findings (`research/pitfalls.md` sections 1, 3, 4).

---

## Effort estimate (retroactive)

Cost classes only; no human-time figures. Raw token counts for the implementation and verify runs were not captured, so measured cost is UNVERIFIED; classes are sized from the diff (17 files, 3009 insertions of which 2801 are vendored JSON; 208 non-JSON lines).

| Story | Tasks | Classes | Notes |
|---|---|---|---|
| 1.1.1 grammar + `Language::Bash` | T1-T3 | XS + S + XS | Straightforward; no version diagnosis needed |
| 1.2.1 `syntax-rules-bash` | T4, T5, T11, T12 | XS + M + S + XS | T5 dominant (node-kind verification) |
| 1.2.2 wiring + sibling tables | T6-T9 | S + XS + XS + S | Four of the sites were found by build failures/review, not by the plan |
| 1.3 tests, docs, backtest | T10, T13, T14 | S + XS + S | Backtest needs a local script corpus |

Verification multiplier x1.5-2 applied: `1dc4dc3` (the sdd:6-verify pass) produced five fixes (pure-expansion narrowing, `bash-file-size`, if-chain test, `#[expect]`, docs), consistent with the multiplier. Critical path: T3 -> T4/T5/T7/T8 (compiler-forced) -> T6 -> T10, about 4 agent waves; T9 and T13 parallel. No wall-clock blockers; the only open owner decision is the Unresolved Questions above.

---

## Tech debt and process lessons

1. Enumerate `Language` touch points with `grep -rn "Language::Rust" src` before planning a new language, and diff the result against the plan. Exhaustive matches cannot catch wildcard arms (`hint.rs`) or hard-coded counts (`inline_ignore.rs` glob count).
2. Treat a grammar-research claim as unverified until checked against the vendored `node-types.json`; four of the original plan's config values were wrong (C2-C4).

---

## Open follow-ups (deliberately not done)

Not required by `requirements.md` v1 or judged out of scope; none blocks shipping. Source for each is `research/pitfalls.md` or `research/architecture.md` unless noted.

1. **Shebang / extension-less script detection.** Language selection is by extension only (`Language::extensions()`, `checker.rs:137`). VERIFIED: `kibitzer check native comment-quality-bash ./deploy` analyzes an extensionless `#!/usr/bin/env bash` file, but `kibitzer run .` over a dir with `deploy` and `x.sh` reports only `x.sh`. Also uncovered: `.zsh`, `.ksh`, `.bats`, `.envrc`, `.bashrc`/`.bash_profile` (INFERRED; zsh files parsed as bash may yield ERROR nodes, untested). Highest-ranked recommendation in the pitfalls research.
2. **`replace-magic-literal` noise: `"x$VAR"` and single-character strings.** The `[ "x$VAR" = "x" ]` idiom, `"y"`/`"n"` answers and `"\n"` separators remain flagged (VERIFIED FP class). Candidate fix: exempt strings of length <= 1 or escape-only strings, plus handle `"x$VAR"` comparisons. Also cap the "also lines" list in the finding message (INFERRED fix).
3. **`arch_export` / `arch_model` count `.sh` as supported with zero symbols.** `arch_export.rs:21` `has_supported_extension` and `arch_model.rs:316` `language_for_path` delegate to `Language::for_path`, so a shell-only repo passes the "no supported languages" guard and exports an empty model (VERIFIED by reading both). No test pins it; the `symbol_extract.rs` comment documents zero symbols but not this consequence. Related: `import_graph`, `declarations`, `unreferenced_symbols`, `lsp.rs:190` also receive `.sh` with no symbols (INFERRED benign, untested).
4. **`import_preamble` / doc mentions.** `src/checkers/import_preamble.rs:15-30` maps by extension string with no bash arm (no-op for `.sh`; INFERRED correct, unrecorded). `docs/affected-validation.md:79` and `docs/duplicate-code-fuzzy.md:94` omit bash (cosmetic).
5. **Shell-shaped commented-out-code patterns.** `comment-quality-bash` misses most commented-out shell (VERIFIED: of `# cd "$dir"`, `# rm -rf "$build" && make clean`, `# for f in *.sh; do ...`, `# x=$(foo bar)`, `# if [ -f "$x" ]; then`, only `# x=$(foo bar)` fired). Needs patterns (`cd `, `rm `, bare `fi`/`done`/`then`, `VAR=...`) and a backtest.
6. **A bash-specific native check** (unquoted expansion, unchecked `$?`, `set -e`/`set -o pipefail` auditing), analogous to `go-ignored-error`. Out of scope per `requirements.md`; needs its own design and the `check-ideas.md` "confirmed transcript occurrence" bar.
7. **shellcheck via a `command` check.** Recommended follow-up in `research/build-vs-buy.md`: first a documented `command`-check recipe (`docs/output-formats.md:13-49`), plugin only if demand; opt-in, not default (machine-local plugins run in every repo; GPL binary cannot be bundled, INFERRED). shellcheck has no built-in SARIF formatter (INFERRED, not run).
8. **Smaller items.** Raise or special-case the `long-function` threshold for `case`-driven installers (INFERRED too tight; not backtested); rename `slog_key_predicate`; add direct tests that `comment-quality-bash`/`bash-file-size` are registered and that `comment_leader` returns `#` for `.sh`; fix `if_branches` so nesting in later branches counts (the T12 test must then change on purpose); record bash backtest triage under `docs/backtest-triage/`; reconcile `requirements.md` AC1 (0.25.x) with the 0.23 dependency; fix the `hint.rs` commit attribution in `research/architecture.md` (C11).

## Added by the plan reviews (`architecture-review.md`, `adversarial-review.md`)

Follow-ups not done in this change:
- `comment_leader` (`src/inline_ignores/hint.rs`) ends in a `Some(_) => "//"` wildcard, so a new language silently gets the wrong leader (bash did, until review). Make the match exhaustive.
- Assert over `Language::ALL` that every language has its default checks registered, so a missed registration fails a test.
- Derive per-language check globs from `Language::extensions()` (hand-maintained in three places today).
- `BashKind` has no consumer yet.
- `if_branches` under-counting nesting in later `elif`/`else` branches is a real false negative for installer scripts; give it its own fix.
- `slog_key_predicate` is now used by two languages (Go, Bash), not one; rename it (field plus `collector.slog_key_ids`, 19 occurrences) when convenient, not only at a fourth user.
- The ADR-001 "<40% false-positive rate" bar has no source outside the superseded plan at `7634824`, and no triage was done.
- Move the `*_lang_config` functions out of `src/checkers/rules.rs` (about 2040 production lines) and run hotspot analysis before the next language.

## Update after the Product Triad Review (iteration 1)

The triad review (product, UX and engineering all `needs-work`) found the CLAUDE.md done-bar unmet: no false-positive triage. Done since:
- **FP triage** of 116 findings on 150 shell scripts: `docs/backtest-triage/bash-2026-10-18.md` plus per-finding records. Result before fixes: 42.1% strict FP overall, 52.5% for `replace-magic-literal`, 90% for `comment-quality-bash`; the <40% bar was NOT met.
- **Fixes** (commit after `094bbdf`): `bash_is_exempt_literal` in `src/checkers/rules.rs` (redirect/command-arg numbers, boolean/separator/escape strings, `echo`/`printf` arguments); `comment-quality-bash` runs only `verbose-comment` (`BASH_SKIPS_CODE_HEURISTICS`). Re-measured on the same sample: 72 findings, 52 TP, 0 FP, 20 debatable. This is tuned-on-sample, not an independent estimate (see the caveat in the triage doc); a fresh-sample run is still open.
- **Flaky tests**: `tests/daemon_spawn.rs` also fails intermittently on a clean `master` checkout (2 of 3 runs), so it is not caused by this change. `backtest_isolates_the_duplicate_cross_file_index_and_still_finds_duplicates` failed once on this branch and passed on `master` once and on this branch in later runs; its origin is unconfirmed.
- **Docs**: extension-only detection, inert rules and the comment-quality gating are documented in `docs/syntax-rules.md` and `docs/comment-quality.md`; requirements AC1 reconciled (0.23.3 shipped).
- Still open: fresh-sample FP run, shebang detection, uncapped "also lines" list (shared across languages), `import_preamble`/arch_export follow-ups listed above.

### Flaky-test origin (confirmed)

`backtest::tests::backtest_isolates_the_duplicate_cross_file_index_and_still_finds_duplicates` is a pre-existing race, not caused by this change: the `backtest::` tests share the process-global `KIBITZER_DUPLICATE_INDEX_DIR` env var (`src/backtest.rs:217,238`), so concurrent backtest tests trip the "must be restored" assertion. Evidence: on this branch 2 of 12 parallel runs of `cargo test --bin kibitzer backtest::` failed (0 of 1 serial); on a clean `master` checkout (`d76c1ab`) 1 of 25 failed. Both `tests/daemon_spawn.rs` and this test pass with `--test-threads=1`. Not fixed here.
