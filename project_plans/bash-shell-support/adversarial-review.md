# Adversarial Review: bash-shell-support

**Date**: 2026-10-18
Verdict: CONCERNS

Retroactive plan of record reviewed against `requirements.md` and the code in `e8a422a` / `1dc4dc3`. Code was not edited.

## Spot-checks run (all against this worktree)

| Plan claim | Check | Result |
|---|---|---|
| T1: lock resolves `tree-sitter-bash 0.23.3`, `tree-sitter 0.26.12`; `Cargo.toml` `tree-sitter-bash = "0.23"` | `grep Cargo.lock`, `Cargo.toml:41` | Holds |
| T3: `Language::Bash` at `checker.rs:79,94,107,137` | read lines 76-140 | Holds (variant, `ALL`, `ts_language`, `["sh","bash"]`) |
| T5/C2-C4: `bash_lang_config` at `rules.rs:1250`, `terminal_kinds: &[]` :1273, `else_wrapper_kinds: &[]` :1263, `body_finder: field_body` :1266, `return_kind` placeholder :1278, `lang_config` arm :906 | read `rules.rs:1196-1290` | Holds exactly |
| C5: named null-object fns `rules.rs:1198-1214` | read | Holds |
| T4: `tree_walk.rs:13`, `comment_quality.rs:194` | read | Holds |
| T9: `hint.rs:19`, `tests.rs:305`, `inline_ignore.rs:273` (13 -> 15), `anchor_tests.rs` fixtures, `"py" \| "sh" \| "bash"` | read | Holds |
| T8: `symbol_extract.rs:291-300` | read | Holds |
| T2: `#[expect]` in `node_kind.rs` (:50, :54) | grep | Holds |
| T6: `bash-file-size` at `config.rs:928`, `syntax-rules-bash` :957, `comment-quality-bash` :987, duplicate-code globs :831/:848, `duplicate_code.rs:54-55` | grep | Holds |
| T10/T11/T12: `cargo test --bin kibitzer bash` | ran | 5 passed |
| T10/AC10: `cargo test --bin kibitzer` | ran | `1704 passed; 0 failed` (matches plan) |
| T6/C8: `default_checks` tests | ran | `..._nine_syntax_rules_checkers ... ok`, 4 passed |
| C8: `1dc4dc3` removed stray `comment-quality-bash` from syntax-rules list | `git show 1dc4dc3 -- src/config.rs` | Holds |
| Commit stats: 17 files, 3009 insertions, 2801 JSON; `1dc4dc3` has no `hint.rs` (C11) | `git show --stat` | Holds |
| C10: no bash entry in `docs/backtest-triage/` | `ls` | Holds |
| Follow-up 3: `arch_export.rs:20`, `arch_model.rs:316` delegate to `Language::for_path`; `lsp.rs:190` | read | Holds |
| Follow-up 4: `import_preamble.rs` has no bash arm; `docs/affected-validation.md:79`, `duplicate-code-fuzzy.md:94` omit bash | read | Holds |
| Corrections C1, C6 vs `research/stack.md` (dev-dependency only, 62 named kinds, 112 diff lines vs 0.25.1) | read | Table matches the research file |

## Acceptance-criteria traceability (requirements.md AC1-AC10)

| AC | Plan task | Evidence | Assessment |
|---|---|---|---|
| 1 `tree-sitter-bash` "0.25.x" | T1 | Shipped 0.23.3 | Literal AC not met; disclosed (C6, Unresolved Questions). OK as disclosed, still open. |
| 2 vendored JSON + `GRAMMARS` entry | T2 | `bash.json` in stat; `build.rs` +1 | Met |
| 3 `Language::Bash` + regression test | T3 | verified above | Met |
| 4 `comment_kinds` | T4 | arm verified | Met, but the plan's Given/When/Then ("`# note` is a comment node") cites no test, only "compiles" (see CONCERN 3) |
| 5 `bash_lang_config` + caveat comments | T5 | doc comment `rules.rs:1236-1249`, `assert_valid` Bash arm | Met |
| 6 registered in `config.rs` | T6 | verified above | Met; no test pins `comment-quality-bash` or `bash-file-size` registration (plan admits) |
| 7 duplicate-code globs | T6 | `config.rs:831,848` | Met |
| 8 registration test | T6 | `..._nine_...` passes | Met. Note it asserts only `syntax-rules-bash` |
| 9 end-to-end test | T10 | `bash_flags_deep_nesting` passes | Met (deep-nesting branch of the "or") |
| 10 build + test clean | T10 | 1704 passed re-run here | Met for `--bin kibitzer`; "no new warnings" rests on the plan's own `cargo build`/`clippy` claim, which I did not re-run |

## Blockers

None.

## Concerns

- [ ] **Backtest numbers 85 vs 106 are internally contradictory, not just unverified.** The plan's Backtest table puts "276 -> 85" before narrowing and "106" after narrowing (labelled "from the caller's brief"). `research/pitfalls.md:42` quotes the same figures as "276 -> 106 (later 85)", i.e. the opposite order. The commit message of `e8a422a` says only "276 -> 85"; `1dc4dc3` states no number. The plan's explanation ("106 > 85 because mixed strings are flagged again") is a rationale built on an order that no checked source establishes. Both rows are correctly tagged UNVERIFIED, but the causal gloss is not. Recommendation: state that the order is disputed between sources, and delete the "more findings because..." explanation or mark it INFERRED.
- [ ] **The FP-rate gate is unmet, and the plan understates that in its follow-ups.** `CLAUDE.md` says a checker not run against the corpus "isn't done"; the original Task 8 asked for a <40% FP rate and triage. The plan admits this in the Backtest notes ("main remaining gap") but the Open follow-ups list has only the smaller "record triage" bullet (item 8) and the Unresolved Questions frame it as optional ("Whether to commit..."). The status line says "Implemented". Recommendation: add an explicit follow-up/blocker-for-ship item "compute bash FP rate via `scripts/backtest-triage.py` on shell scripts" and make "Status" say "implemented; backtest done-bar not met".
- [ ] **Weak evidence for AC4 and AC3's Given/When/Then.** T4 is "Verified by: compiles; 1704 passed". No cited test asserts `comment_kinds(Language::Bash) == ["comment"]` or that a `# note` line yields a comment node (I found no bash-specific `comment_kinds` test). `comment-quality-bash` behaviour is evidenced only by the pitfalls agent's manual runs. Recommendation: cite a concrete test, or downgrade the AC to INFERRED and list the missing test.
- [ ] **The "ADR-001 <40% FP rate" source is not citable.** No `docs/adr/` file contains it (`docs/adr` is empty here; `grep -rl ADR-001` hits only other projects' plans). The only source is the superseded original plan (`git show 7634824:project_plans/bash-shell-support/plan.md`, Task 8). The plan cites it as a standard without a link. Recommendation: cite that commit path or the ADR file where it lives.
- [ ] **A weak citation underpins the "Extend as-is" tech-debt disposition.** The plan cites `project_plans/go-stdlib-idioms-fp/research/architecture.md:73,167` and `architecture-export/research/architecture.md:20` as proof that `LangRuleConfig` is the established seam. Line 73 of the former is a "Fix 2" heading (not a seam statement); line 20 of the latter describes `LangSymbolConfig` precedent. Only :167 of go-stdlib-idioms-fp and the code itself (`rules.rs:1398` SEAM comment) support the claim. Recommendation: cite `rules.rs` directly.
- [ ] **No test pins the shipped behaviours the plan lists as deliverables:** `comment-quality-bash` / `bash-file-size` registration, `comment_leader` -> `#` for `.sh` (the one miss the compiler could not catch), and the zero-symbol consequence for `arch_export`/`arch_model`. The plan discloses each; it still leaves the only compiler-invisible bug fixed without a regression test. Recommendation: promote `comment_leader` test from follow-up item 8 to a tracked task.

## Minors

- Story AC numbering (AC1-AC10) reuses the requirements' numbers but is split across stories with no explicit mapping table; AC6/AC7/AC8 combine requirements, and plan AC10 appears only inside T10. A one-line traceability table (like the one above) would help.
- Complexity "2 (defaulted)" has no source rule cited.
- T10 attributes the one-off `backtest_isolates_the_duplicate_cross_file_index...` failure to an env-var race; it is correctly marked INFERRED and not confirmed on `master`. Fine, but I did not see it fail (my single full run passed).
- "Observability: none new ... no timing measured" is INFERRED and flagged as such; acceptable.
- Line ranges drift slightly: test cited at `rules.rs:3369-3380` actually starts at :3372; `bash_lang_config` doc comment begins a few lines before the cited :1236. Cosmetic.
- Plan references `research/architecture.md` as correcting `hint.rs` attribution (C11); the research file still says `1dc4dc3` (:17, :40). Plan notes it as a follow-up; fine.
- Corrections table C1-C11 matched the research files wherever I could check (C1, C2, C3, C4, C6, C11); C7 and C9/C10 matched the code/commits.
