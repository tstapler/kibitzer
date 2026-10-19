# Validation Plan: bash-shell-support

**Date**: 2026-10-18
**Mode**: Retroactive (Complexity 2). The code landed in `e8a422a`, `1dc4dc3` and `ec9dca8` before this phase ran, so this file maps requirements to tests that already exist and records the result of running them. It does not design new tests. Where a requirement has no test, that is listed under Gaps rather than papered over.

Evidence labels: **VERIFIED** = command run in this worktree on 2026-10-18 (output quoted); **INFERRED** = reasoned, not executed; **UNVERIFIED** = reported elsewhere, not reproduced.

## Happy Path Scenario

Given a repo with no `.kibitzer/inspect.json` (the default catalog) and a `deploy.sh` containing a function nested five loops deep, when kibitzer runs its default checks on that file, then `syntax-rules-bash` reports one `[deep-nesting]` finding, a 3-line `greet()` reports nothing, and `# kibitzer:ignore <rule> -- <why>` placed above the finding suppresses it.

Anchoring tests: `bash_flags_deep_nesting`, `bash_allows_a_simple_function`, `default_checks_still_registers_all_nine_syntax_rules_checkers`, `anchor_conformance_should_DropFinding_When_DirectiveOnReportedLineOrRowAbove` (fixture `syntax-rules-bash`/`x.sh`).

## Requirement → Test Mapping

Test file paths are relative to the repo root. Names are the real test function names (all in `cargo test --bin kibitzer`, i.e. `src/**`).

| Requirement | Test File | Test Name | Type | Scenario | Result |
|-------------|-----------|-----------|------|----------|--------|
| AC1: `tree-sitter-bash` declared | `Cargo.toml:41`, `Cargo.lock:1414` | (no test; `cargo build` fails if the dependency is missing or does not resolve) | Build | Happy path | VERIFIED `cargo build` -> `Finished`; `Cargo.lock` has `tree-sitter-bash 0.23.3`. See Gap G7 on the "0.25.x" wording |
| AC2: `bash.json` vendored, `BashKind` generated | `build.rs:42`, `src/node_kind.rs:55` | (no direct test; `BashKind` is compiled in, `#[expect(unused_imports)]` fails the build if it ever becomes used and the attribute stays) | Build | Happy path | VERIFIED `cargo build` clean. Byte-identity of `bash.json` with the crate was checked once by `cmp` (`research/stack.md`), not by a test (G7) |
| AC3: `Language::Bash` in enum, `ALL`, `ts_language`, `extensions` (`sh`, `bash`) | `src/checker.rs:429` | `from_extension_recognizes_every_language_variant` | Unit | Happy path (round-trips `sh`, `bash` for every `Language::ALL`) | ok |
| AC3 (error path: extension reaches the right behavior) | `src/inline_ignores/tests.rs:1667` | `comment_leader_should_BeHash_When_ShellScript` (`a.sh`, `a.bash` resolve to `#`) | Unit | Path -> language -> behavior | ok |
| AC4: `comment_kinds` covers Bash with the real grammar kind | `src/tree_walk.rs:62` | `bash_comment_kind_matches_a_real_parsed_comment` (parses `# note\n` with `tree_sitter_bash::LANGUAGE`, asserts `comment_kinds(Bash)` contains the parsed node's kind) | Unit | Happy path, grammar-verified rather than assumed | ok |
| AC5: `bash_lang_config()` kinds verified against the grammar | `src/checkers/rules.rs:2214` | `node_kind_literals_are_valid_for_their_grammar` (includes `assert_valid(... lang_config(Language::Bash) ...)` at `:2265`) | Unit | Every node-kind string in the config exists in the grammar | ok |
| AC5: dispatch | `src/checkers/rules.rs:2275` | `lang_config_dispatches_to_the_matching_constructor` (Bash arm at `:2296`) | Unit | `lang_config(Bash)` returns `bash_lang_config()` | ok |
| AC5: caveat comments for absent kinds | `src/checkers/rules.rs:1236-1249` (doc comment) | (not testable; documentation) | Review | Names `return_statement`, `break_statement`, `continue_statement`, `until_statement` as absent | VERIFIED by read (plan.md C2) |
| AC6: `syntax-rules-bash` registered | `src/config.rs:1444` | `default_checks_still_registers_all_nine_syntax_rules_checkers` | Unit | Name present in `default_checks()` | ok |
| AC6: `comment-quality-bash` registered, scoped `**/*.sh`, `**/*.bash` | `src/config.rs:1472` | `default_checks_register_bash_comment_quality_and_file_size` (also covers `bash-file-size`) | Unit | Name and both globs present | ok |
| AC6: inline-ignore scope covers the new extensions | `src/checkers/inline_ignore.rs:262` | `inline_ignore_checker_should_BeRegisteredRawTextAdvisoryScope` (iterates `Language::ALL` x `extensions()`, asserts `globs.len() == 15`) | Unit | Glob count 13 -> 15 | ok |
| AC6: scope read from `default_checks()` | `src/config.rs` | `default_checks_should_ScopeInlineIgnoreToGrammarExtensionsAndMd_When_Read` | Unit | Scope derived from grammar extensions | ok |
| AC7: `duplicate-code` and `duplicate-code-cross-file` globs include `.sh`, `.bash` | `src/config.rs:830-831,847-848`; `src/checkers/duplicate_code.rs:54-55` | **None.** Manual only: `kibitzer check native duplicate-code a.sh` on a 3x repeated 6-line shell block -> `a.sh:29: 6-line block repeated 3 times (lines 3, 16, 29)` | Manual | Happy path | VERIFIED manually (not in the suite). See Gap G1 |
| AC8: registration regression test asserts `syntax-rules-bash` | `src/config.rs:1444` | `default_checks_still_registers_all_nine_syntax_rules_checkers` | Unit | Regression guard | ok |
| AC9: end-to-end syntax-rules test on a shell function | `src/checkers/rules.rs:3318` | `bash_flags_deep_nesting` | Unit | Happy path (finding contains `[deep-nesting]`) | ok |
| AC9 | `src/checkers/rules.rs` | `bash_allows_a_simple_function` | Unit | Negative: 3-line function yields nothing | ok |
| AC9 (error path / known limitation) | `src/checkers/rules.rs:3372` | `bash_deep_nesting_inside_else_branch_is_not_yet_counted` | Unit | Pins the `if_branches` false negative (G3) | ok |
| AC9 (`long-parameter-list` variant) | n/a | n/a: cannot fire for bash (no formal parameters); `zero_params` stub. AC9 allows "or deep-nesting" | n/a | n/a | Satisfied by deep-nesting |
| AC10: build and test clean | n/a | `cargo build`, `cargo clippy --all-targets`, `cargo test --bin kibitzer` | Command | Whole suite | VERIFIED: build `Finished`, clippy 0 `warning` lines, bin target `1707 passed; 0 failed`. `tests/daemon_spawn.rs` is flaky when run in parallel (G5) |

### Behavior beyond the numbered ACs (from Goal / plan.md tasks)

| Behavior | Test File | Test Name | Type | Scenario | Result |
|----------|-----------|-----------|------|----------|--------|
| `replace-magic-literal` flags plain repeats and mixed expansions (T11) | `src/checkers/rules.rs` | `bash_flags_repeated_plain_string_and_mixed_expansion_string` (3x `"linux"` -> 1; 3x `"Usage: $0"` -> 1) | Unit | Happy path | ok |
| `replace-magic-literal` exempts pure expansions (T11) | `src/checkers/rules.rs` | `bash_exempts_repeated_pure_expansion_string` (3x `"$@"`, 3x `"${1}"` -> 0) | Unit | Error path (false-positive guard) | ok |
| Inline-ignore works for each of the three bash checks (T9, T7) | `src/inline_ignores/anchor_tests.rs:19,51,70,91` | `anchor_conformance_should_DropFinding_When_DirectiveOnReportedLineOrRowAbove`, `anchor_conformance_should_HaveFixtureOrExemption_When_DefaultCheckerAdded` (fixtures `bash-file-size`, `syntax-rules-bash`, `comment-quality-bash`, all `x.sh`) | Integration | Directive on the finding row or the row above drops it; every default checker has a fixture | ok |
| `bash-file-size` directive placement | `src/inline_ignores/anchor_tests.rs:466,479` | `file_size_ignore_should_Suppress_When_DirectiveInFirstTenLines`, `file_size_ignore_should_NotSuppress_When_DirectiveBelowRowTen` | Integration | Shared file-size placement rule (Go-fixture based; bash coverage is via the anchor fixture above) | ok |
| Shell `#` directives parse; markers inside `echo` strings are ignored (T9) | `src/inline_ignores/tests.rs:457,469` | `scan_leading_comments_should_ParseDirective_When_ShellHashComment`, `scan_leading_comments_should_ReturnNothing_When_MarkerInsideEchoString` | Unit | Happy and error path | ok |
| `fixture(Language::Bash)` exhaustive match (T9) | `src/inline_ignores/tests.rs:305` | (compile-time exhaustiveness) | Build | A new `Language` without a fixture does not compile | VERIFIED `cargo build --tests` clean |

## Run Output (VERIFIED, 2026-10-18)

```
$ cargo test --bin kibitzer bash
running 7 tests
test config::tests::default_checks_register_bash_comment_quality_and_file_size ... ok
test checkers::rules::tests::bash_allows_a_simple_function ... ok
test tree_walk::tests::bash_comment_kind_matches_a_real_parsed_comment ... ok
test checkers::rules::tests::bash_deep_nesting_inside_else_branch_is_not_yet_counted ... ok
test checkers::rules::tests::bash_flags_deep_nesting ... ok
test checkers::rules::tests::bash_exempts_repeated_pure_expansion_string ... ok
test checkers::rules::tests::bash_flags_repeated_plain_string_and_mixed_expansion_string ... ok
test result: ok. 7 passed; 0 failed

$ cargo test --bin kibitzer default_checks
test config::tests::default_checks_register_bash_comment_quality_and_file_size ... ok
test config::tests::default_checks_still_registers_all_nine_syntax_rules_checkers ... ok
test config::tests::default_checks_should_ScopeInlineIgnoreToGrammarExtensionsAndMd_When_Read ... ok
test result: ok. 5 passed; 0 failed

Individually run, each `ok`, 1 passed unless noted:
  tree_walk::tests::bash_comment_kind_matches_a_real_parsed_comment
  inline_ignores::tests::comment_leader_should_BeHash_When_ShellScript
  inline_ignores::anchor_tests::anchor_conformance_should_DropFinding_When_DirectiveOnReportedLineOrRowAbove   (2 passed with the next)
  inline_ignores::anchor_tests::anchor_conformance_should_HaveFixtureOrExemption_When_DefaultCheckerAdded
  checkers::inline_ignore::tests::inline_ignore_checker_should_BeRegisteredRawTextAdvisoryScope
  checker::tests::from_extension_recognizes_every_language_variant
  checkers::rules::tests::node_kind_literals_are_valid_for_their_grammar
  checkers::rules::tests::lang_config_dispatches_to_the_matching_constructor
  inline_ignores::tests::scan_leading_comments_should_ParseDirective_When_ShellHashComment   (2 passed with the next)
  inline_ignores::tests::scan_leading_comments_should_ReturnNothing_When_MarkerInsideEchoString

$ cargo build            -> Finished `dev` profile
$ cargo clippy --all-targets 2>&1 | grep -c '^warning'   -> 0
$ cargo test --no-fail-fast
  unittests src/main.rs        1707 passed; 0 failed
  tests/affected_cli.rs           9 passed
  tests/daemon_spawn.rs           7 passed; 7 failed   (parallel; see G5)
  tests/false_positives_cli.rs    2 passed
  tests/hook_contract.rs         33 passed
  tests/inline_ignore_cli.rs     33 passed
  tests/output_sanitize_cli.rs    7 passed
  tests/plugin_contract.rs       16 passed
$ cargo test --test daemon_spawn -- --test-threads=1   -> 14 passed; 0 failed
```

Counts since `plan.md` was written: the bin target is 1707 (plan.md quoted 1704 and "5 bash tests"); the difference is the three tests `ec9dca8` added (`bash_comment_kind_matches_a_real_parsed_comment`, `comment_leader_should_BeHash_When_ShellScript`, `default_checks_register_bash_comment_quality_and_file_size`). `plan.md` still says 5 in places and its follow-up 8 lists the registration and `comment_leader` tests as missing; they now exist.

## Test Stack
- **Unit**: Rust built-in `#[test]` with `assert!`/`assert_eq!`; tree-sitter parsing of inline source strings (no fixtures on disk).
- **Integration**: anchor conformance fixtures (`src/inline_ignores/anchor_tests.rs`) drive real checkers through the inline-ignore seam; `tests/*.rs` are CLI-level.
- **E2E / UX**: none. This change has no UX surface (no `design/ux.md`); the UX section of the template is omitted.
- **Migration**: N/A (no schema or stored state).

## Coverage Targets and How to Measure

| Stack | Coverage command | Target |
|---|---|---|
| Rust | `cargo tarpaulin --out Stdout` | >=80% line (not run; see G10) |

Qualitative target met: the new public surface (`Language::Bash`, `comment_kinds`, `bash_lang_config`, three default registrations, the `#` leader) each has at least one assertion. Error paths are thin by nature (the config is data plus no-op fns).

## Adversarial notes carried forward from the original validation.md

Each point below was checked against the landed code.

| Original point | Status now |
|---|---|
| Dependency version risk: `tree-sitter-bash 0.23` might pull a second `tree-sitter`. | Resolved, no risk. `Cargo.lock` holds `tree-sitter 0.26.12` and `tree-sitter-bash 0.23.3` (plan.md C1). Keep: AC1's "0.25.x" wording still disagrees with the shipped 0.23.3 (G7). |
| Node kinds in the config were unverified and a wrong string silently never matches. | Resolved and now enforced by `node_kind_literals_are_valid_for_their_grammar` plus `bash_comment_kind_matches_a_real_parsed_comment`. Four sketched values were in fact wrong (plan.md C2-C4), which confirms the gate was needed. |
| `param_counter: \|_\| 0` makes `long-parameter-list` never fire. | Holds, by design. Documented in `bash_lang_config` and `docs/syntax-rules.md`; `flag-argument` is likewise inert. Not pinned by a test (G9). |
| `DuplicateCodeChecker::file_globs()` was missing from the plan. | Closed in code (`duplicate_code.rs:54-55`; the cross-file checker delegates at `duplicate_cross_file_checker.rs:34`). Still untested (G1). |
| `from_extension_recognizes_every_language_variant` should be named explicitly. | Done; run above. |
| Numeric `replace-magic-literal` is inert because bash digits parse as `word`. | Confirmed by run (plan.md C-notes, `research/pitfalls.md` section 1: `[ "$n" -gt 42 ]`, `8080`, `$((a+7))` give zero findings). Accepted; no test. |
| `extract-variable` via `call_kinds: ["command"]` needs a backtest. | Backtest ran: 2 `extract-variable` findings in 300 pooled (`research/pitfalls.md` section 2). Fires rarely; fine for v1. |

## Gaps

Genuine, not invented. None blocks the existing behavior; ordered by how much they weaken the done-definition.

- **G1 - AC7 has no automated test.** Neither the `core_checks()` globs (`config.rs:830-848`) nor `DuplicateCodeChecker::file_globs()` (`duplicate_code.rs:54-55`) is asserted for `.sh`/`.bash`, and no duplicate-code test uses a shell source. The only evidence is the manual run above (and `research/pitfalls.md` for the cross-file variant, which was not re-run here). The anchor fixtures for `duplicate-code` and `duplicate-code-cross-file` are Go-based.
- **G2 - Extension-less scripts and shebangs are not detected.** Language selection is by extension only (`checker.rs:137`); no test documents that `deploy` with `#!/usr/bin/env bash` is skipped by `kibitzer run .` (the behavior was observed manually per plan.md follow-up 1). `.zsh`, `.ksh`, `.bats`, `.envrc`, `.bashrc` are also unrecognized, and zsh-as-bash ERROR-node behavior is untested.
- **G3 - `if_branches` nesting is pinned only as a known limitation.** `bash_deep_nesting_inside_else_branch_is_not_yet_counted` asserts the false negative; there is no test for the desired behavior, and `elif` and later then-statement nesting are not pinned individually. A fix must change that test on purpose.
- **G4 - No false-positive triage.** The ADR-001 "<40% FP rate" bar was never computed; there is no bash entry under `docs/backtest-triage/` and `scripts/backtest-triage.py` was not used. Backtest numbers exist only in the `e8a422a` message and `plan.md` (different samples, not reproducible). `kibitzer check backtest` transcript replay was not run for bash. This is the main gap against the repo's own "a checker that hasn't been run against this corpus isn't done" rule. Known residual noise classes: `[ "x$VAR" = "x" ]`, `"y"`/`"n"`, `"\n"`; `long-function` drives 106 of 300 pooled findings.
- **G5 - `tests/daemon_spawn.rs` is flaky in parallel.** Three parallel runs this session failed 9, 7 and 6 of 14 tests (different tests each time); serial (`--test-threads=1`) passed 14 of 14, and a single named test passed alone. An earlier session saw `backtest::tests::backtest_isolates_the_duplicate_cross_file_index_and_still_finds_duplicates` fail once in a full run (plan.md T10). Both are INFERRED process-environment/port races, unrelated to bash (the diff touches no daemon code); not run against `master`, so "pre-existing" is UNVERIFIED. AC10 ("`cargo test` passes") is therefore met only with `--test-threads=1` for that target.
- **G6 - Registration tests check names, not all scopes.** `syntax-rules-bash`'s globs are not asserted (only its name, in the nine-name test), and `bash-file-size` appears in the new test but its anchor fixture is the only behavioral coverage. No test iterates `Language::ALL` to assert every language has its default checks registered (plan.md "Added by the plan reviews").
- **G7 - Dependency and vendored-JSON facts are not test-pinned.** Nothing asserts `bash.json` matches the crate version, and AC1 says 0.25.x while 0.23.3 shipped (decision still open in plan.md Unresolved Questions).
- **G8 - Collateral sites untested.** `symbol_extract.rs:294` returns zero symbols for `.sh`; `arch_export`/`arch_model` treating `.sh` as supported (empty model) and `import_graph`, `declarations`, `lsp.rs` receiving `.sh` have no tests (plan.md follow-up 3).
- **G9 - Inert rules are not pinned as inert, and comment-quality pitfalls are manual.** No test asserts `long-parameter-list`, `flag-argument` and `unreachable-code` never fire on bash, or that numeric `replace-magic-literal` stays quiet. `# shellcheck disable=...`, `#!/usr/bin/env bash` and heredoc `#` lines producing no `comment-quality-bash` finding was verified by run in `research/pitfalls.md` section 4, not by a test. `long-function` and `extract-variable` have no bash-specific unit test (they were exercised only in the backtest).
- **G10 - No coverage measurement.** `cargo tarpaulin` was not run ; the >=80% target is unmeasured (UNVERIFIED).
- **G11 - `comment-quality-bash` detection depth.** It misses most shell-shaped commented-out code (only `# x=$(foo bar)` fired of five samples, plan.md follow-up 5). The anchor fixture proves suppression works, not detection quality.

## Verdict

All 10 acceptance criteria are mapped to evidence: 6 to named automated tests (AC3, AC4, AC5, AC6, AC8, AC9), 3 to build or command evidence (AC1, AC2, AC10), and 1 to a manual run only (AC7, G1). Every named test passes. The one failing area, `tests/daemon_spawn.rs` in parallel, is outside this change (G5). Recommended before ship: add a small test for G1 (a `.sh` duplicate-code case plus the two glob assertions), then decide on G4.
