# Requirements: typed-node-kind-migration

**Date**: 2026-09-22
**Type**: migration
**Complexity**: 4 — high-stakes / cross-cutting

## Problem Statement
Native checkers compare `node.kind()` against raw `&str` literals (`node.kind() == "if_statement"`, or a `match` on `.kind()`). A typo in one of these literals (`"if_statment"`) compiles fine and just silently never matches — the checker goes quiet on real code with no error, no warning, and no test failure unless the specific fixture happens to exercise that exact code path. `src/node_kind.rs` (codegen'd from each grammar's `node-types.json` via `build.rs`) already provides typed `<Lang>Kind` enums (`GoKind`, `TypeScriptKind`, `TsxKind`, `JavaScriptKind`, `PythonKind`, `JavaKind`, `KotlinKind`, `RustKind`) that turn that typo into a compile error instead — this migration is converting existing call sites to use them.

## Baseline
Every one of the 23 files below relies purely on unit/backtest test coverage to catch a `.kind()` string typo — there is no compiler backstop. `rules.rs` has one narrower, complementary runtime check (`node_kind_literals_are_valid_for_their_grammar`, validated at *test* time against `tree_sitter::Language::id_for_node_kind`) that covers only its own `LangRuleConfig` literals, not the other 22 files' inline comparisons.

## Users / Consumers
Internal only — kibitzer's own maintainers (Tyler, this session) editing/adding native checkers. No external API or behavior surface changes.

## Success Metrics
- Every listed file's `node.kind() ==` / `.kind() ==` / `match … .kind()` string comparisons against a *named concrete* tree-sitter kind are replaced with the corresponding `<Lang>Kind` enum comparison.
- Zero behavior change: `cargo test --workspace` passes identically, and re-running each affected checker's backtest corpus (`docs/backtest-repos.md`) produces the same finding counts as pre-migration.
- A misspelled kind name at any migrated call site fails `cargo build`, not silently at runtime.

## Success Metrics — Non-Goals as Success
Files/comparisons that must stay as raw strings (anonymous/punctuation tokens, synthetic `ERROR`/`MISSING` nodes not in `<Lang>Kind`'s named set) are explicitly left alone — forcing them into the enum would require widening `Other`'s semantics and isn't part of this migration's scope.

## Appetite
Large (3–6 weeks equivalent of engineering effort, executed compressed via SDD's parallel implementation workers)
*(23 files, several large/high-risk. Cut scope by file if risk assessment during planning finds a file's comparisons aren't safely expressible as the generated enum — flag and leave as a follow-up rather than force-fitting.)*

## Constraints
- No behavior change permitted — this is a pure internal-safety refactor, not a feature or bugfix.
- Must pass this repo's full CI gate (`cargo fmt --check`, `cargo clippy -D warnings`, `cargo build --workspace`, `cargo test --workspace`) exactly as documented in kibitzer's own CLAUDE.md.
- Every touched native checker must be re-backtested per kibitzer's own "Writing a new check" discipline (CLAUDE.md) before being considered migrated, since a subtle enum-mapping mistake would otherwise only surface as an invisible false-negative — the exact failure mode this migration exists to prevent.

## Non-functional Requirements
- **Performance SLO**: not applicable — `<Lang>Kind::of(node)` is a cheap tag-check, not expected to change per-file check latency measurably.
- **Scalability**: not applicable.
- **Security classification**: internal / not applicable.
- **Data residency**: not applicable.

## Scope
### In Scope
The 23 files currently matching `grep -rln 'node.kind() ==\|\.kind() ==\|match .*\.kind()' src/` (re-run 2026-09-22, post `src/checkers/` reorganization — supersedes the 21-file list in `docs/typed-node-kinds.md`, which predates two newly-landed Go checkers):

```
src/checkers/complexity.rs             src/checkers/java_ignored_error.rs
src/checkers/complexity_tests.rs       src/checkers/java_lost_exception_cause.rs
src/checkers/go_blank_imports.rs       src/checkers/java_swallowed_interrupt.rs
src/checkers/go_bulk_fetch_linear_scan.rs  src/checkers/primitive_obsession.rs
src/checkers/go_error_context.rs       src/checkers/rules.rs
src/checkers/go_ignored_error.rs       src/declarations.rs
src/checkers/go_table_driven_test.rs   src/dedup.rs
src/checkers/go_type_switch_density.rs src/go_call_resolution.rs
src/checkers/java_error_context.rs     src/god_class.rs
                                        src/import_graph.rs
                                        src/isp_fat_interface.rs
                                        src/plugin.rs
                                        src/symbol_extract.rs
                                        src/tree_walk.rs
```
(`src/node_kind.rs` itself also matches the grep — it's the enum definitions/generated code, not a migration target, and is explicitly excluded.)

### Out of Scope
- Building any new codegen or enum-generation mechanism — `build.rs`/`src/node_kind.rs` infra is complete and unchanged by this work.
- Widening `<Lang>Kind::Other` or adding new grammar coverage.
- Retiring `rules.rs`'s `node_kind_literals_are_valid_for_their_grammar` runtime test — leave it in place even where redundant, unless its own file's migration naturally removes the literals it validates (in which case removing the now-dead test is in scope for that file's story only).
- Any new checker functionality, threshold changes, or backtest-driven rule tuning — this migration must not change what any checker flags.

## Rabbit Holes
- `rules.rs`, `symbol_extract.rs`, `import_graph.rs`, and `declarations.rs` are the largest files in the crate and were flagged in `docs/typed-node-kinds.md` as **not** pure mechanical find-replace: some match arms compare against a node's *field* type (not just `.kind()`), a supertype grouping several concrete kinds under one comparison, or a kind name that differs subtly between two grammars sharing one checker function (e.g. a Go-vs-Java kind string that looks the same but maps to different enum variants). Plan phase must explicitly walk each of these four files' comparisons before committing to "mechanical" treatment.
- Two files (`go_bulk_fetch_linear_scan.rs`, `go_table_driven_test.rs`) are newer than `docs/typed-node-kinds.md` and were never assessed for migration risk — treat them as unknown-risk until a planning pass looks at their actual `.kind()` usage, not as "the same as the other Go checkers."
- Grammars share underlying kind names inconsistently (e.g. a "supertype" in one grammar's `node-types.json` may be concrete in another) — a checker function shared across languages (if any exist) is a higher-risk conversion than a single-language file.

## Alternatives Considered
- Leaving the raw-string comparisons as-is and relying solely on `rules.rs`'s existing test-time validation mechanism, generalized to cover all files — rejected because that only catches the typo at `cargo test` time (still requires running tests, not `cargo build`), and would require building new generalized validation infra rather than reusing what already compiles cleanly today (`<Lang>Kind` already exists and is unused elsewhere).
- Doing this ad hoc / file-by-file without SDD planning — rejected per `docs/typed-node-kinds.md`'s own explicit recommendation, given the four high-risk files' non-mechanical comparisons and the zero-behavior-change bar.

## Feasibility Risks
- Enum mapping mistakes are the primary risk class this migration itself is designed to prevent for *future* edits — but a wrong mapping introduced *during* the migration is invisible in exactly the same way (compiles fine, quietly changes which kind matches). Mitigation: mandatory backtest-corpus re-run per touched checker, not just `cargo test`.
- `<Lang>Kind::Other` swallows every anonymous/punctuation/synthetic-node case — any existing comparison against one of those (if present) cannot migrate cleanly to a specific variant and must be identified and left as-is rather than mapped to `Other` (which would defeat the compile-time-safety purpose for that comparison and silently accept any of them).

## Observability Requirements
Standard CI/test signal is sufficient — no new logging, metrics, or alerting. The compile-time typo protection *is* the observability improvement.

## Risk Control
No feature flag or staged rollout applicable (internal refactor, not a runtime-toggleable behavior). Rollback is a plain `git revert` per file/story if a backtest regression surfaces post-merge; land each file as its own story/commit (not one monolithic commit) specifically so a single bad conversion can be reverted without losing the other 21.

## Open Questions
- Should the four flagged high-risk files (`rules.rs`, `symbol_extract.rs`, `import_graph.rs`, `declarations.rs`) be split into their own epic with dedicated planning/review time, separate from the more mechanical Go/Java checker files? (Recommend yes — defer final call to Phase 3 planning.)
- For the two newly-discovered files (`go_bulk_fetch_linear_scan.rs`, `go_table_driven_test.rs`), confirm during research whether their `.kind()` usage is simple (mechanical) or hides the same field-type/supertype subtlety as the four flagged files.
