# Validation Plan: affected-tests

**Date**: 2026-09-22

## Happy Path Scenario

Given a stapler-squad-shaped Go repo at some `HEAD` with a resolvable `--base` ref, when a
developer edits `internal/widget/widget.go` and runs `kibitzer architecture affected --base
<sha>`, then stdout is a newline-separated list containing `widget`'s own package key plus
every package that transitively imports it, and the process exits `0`.

## Requirement → Test Mapping

Requirement IDs are drawn from `requirements.md`'s Scope → In Scope bullets (R1-R4) and
Success Metrics (R5-R8), plus the specific correctness-critical behaviors this task's brief
calls out by name (R9-R13), which are sub-cases of R1/R3/R4 already named in
`requirements.md`'s Rabbit Holes / Feasibility Risks and worked out concretely in
`implementation/plan.md`'s Domain Glossary and Phase 1-4 tasks.

| Requirement | Test File | Test Name | Type | Scenario |
|-------------|-----------|-----------|------|----------|
| R1 — Diff working tree/PR branch against a `--base` ref → `ChangedFile` list (In Scope bullet 1.1) | `src/affected.rs` | `resolve_base_sha_returns_sha_matching_git_rev_parse_when_ref_resolves` | Unit (happy) | `resolve_base_sha(repo_root, "main")` against a tempdir git repo returns `Ok(sha)` equal to `git rev-parse main`'s own output, and the SHA is pinned for every later git call in the same `compute_affected` run. |
| R1 | `src/affected.rs` | `resolve_base_sha_returns_err_naming_the_ref_when_ref_is_unresolvable` | Unit (error) | `resolve_base_sha(repo_root, "totally-not-a-ref")` returns `Err` whose message contains `"totally-not-a-ref"` — a hard error, not a `BailOutReason`. |
| R1 | `src/affected.rs` | `diff_changed_files_detects_pure_rename_as_renamed_not_delete_add` | Integration (shells to real `git -M` in a tempdir repo) | `a/foo.go` renamed to `b/foo.go` with no content change yields one `ChangedFile{path:"b/foo.go", status:Renamed{old:"a/foo.go"}}`, not a `Deleted`+`Added` pair. |
| R1 | `src/affected.rs` | `diff_changed_files_unions_committed_uncommitted_and_untracked_changes_without_duplicates` | Integration (real git repo) | A committed change, an uncommitted tracked edit, and an untracked new file all appear exactly once each in the unioned `Vec<ChangedFile>`. |
| R2 — Walk `ArchModel.import_edges` in reverse from changed packages to the affected set (In Scope bullet 1.2) | `src/affected.rs` | `reverse_bfs_returns_full_transitive_closure_for_a_linear_import_chain` | Unit (happy) | Reverse adjacency for `a→b→c→d`, seeded with `{"d"}`, returns `{"a","b","c","d"}` — indirect (3-hop) dependents included, not just direct importers. |
| R2 | `src/affected.rs` | `reverse_bfs_returns_only_the_seed_when_seed_package_has_no_importers` | Unit (error/edge) | A seed with no entry in `ReverseAdjacency` (leaf package, no importers) still returns exactly `{seed}` — a changed package with nothing importing it is still its own affected set (its own tests must still run). |
| R2 | `src/affected.rs` | `compute_affected_returns_packages_including_transitive_importer_of_the_changed_file` | Integration (builds a real `ArchModel` from files on disk via `collect_repo_files`/`build_model`) | A fixture repo where `internal/widget/widget.go` is the only change: `compute_affected` returns `Ok(AffectedResult::Packages(v))` where `v` contains `widget`'s own key and every (possibly transitive) importer of it. |
| R3 — Emit a package list or the `__ALL__` bail-out sentinel (In Scope bullet 1.3; Success Metric 2: "never a narrowed set that silently under-tests") | `src/main.rs` (inline `#[cfg(test)] mod tests`, matching this file's existing test module at line 810) | `run_affected_prints_newline_separated_package_list_and_exits_success` | Unit (happy) | `AffectedResult::Packages(vec!["example.com/app/a".into(), "example.com/app/b".into()])` formats to stdout exactly `"example.com/app/a\nexample.com/app/b\n"`, `ExitCode::SUCCESS`. |
| R3 | `src/main.rs` | `run_affected_prints_nothing_and_exits_nonzero_when_compute_affected_errs` | Unit (error) | A propagated `Err` (bad repo root, git spawn failure) yields empty stdout, the error message on stderr, and a non-zero exit — distinct from every `Ok(_)` outcome including `BailOut`. |
| R3 | `tests/affected_cli.rs` (new — matches this repo's existing `tests/hook_contract.rs`/`tests/false_positives_cli.rs` convention of spawning the real binary for CLI-contract tests) | `affected_cli_prints_all_sentinel_to_stdout_and_reason_to_stderr_on_bail_out` | Integration (spawns the built `kibitzer` binary against a fixture repo) | A diff touching `go.mod` produces stdout exactly `"__ALL__\n"`, a human-readable `BailOutReason` note on stderr, and exit `0` — not a distinct exit code. |
| R3 | `tests/affected_cli.rs` | `affected_cli_prints_empty_stdout_with_stderr_note_when_nothing_affected` | Integration (spawns the binary) | `base == HEAD` with a clean tree: stdout is exactly `""` (not `"\n"`, not a placeholder), a stderr note is printed, exit `0` — an empty answer is never confused with a bail-out. |
| R4 — Go support (In Scope bullet 2) | `src/affected.rs` | `resolve_changed_packages_maps_a_modified_go_file_to_its_package_key` | Unit (happy) | `ChangedFile{path:"a/a.go", status:Modified}` against a working-tree `ArchModel` with `file_packages{"a/a.go":"example.com/app/a"}` resolves into the seed set. |
| R4 | `src/affected.rs` | `resolve_changed_packages_silently_skips_files_with_no_package_entry` | Unit (error/edge) | `ChangedFile{path:"README.md", ..}` (unsupported extension) and `ChangedFile{path:"vendor/github.com/foo/bar/bar.go", ..}` (excluded by `SKIP_DIRS`) contribute nothing to the seed set — not an error, not a bail-out. |
| R4 | `src/affected.rs` | `compute_affected_end_to_end_against_a_real_go_module_fixture_produces_the_expected_affected_set` | Integration (tempdir repo with real `.go` files + `go.mod`, real tree-sitter parse via `build_model`) | A small multi-package Go fixture (`a` imports `b`) with `b/b.go` changed: `compute_affected` returns `Packages` containing both `b` and `a`. |
| R5 — Each `BailOutReason` variant fires correctly (In Scope bullet 3; Success Metric 4) | `src/affected.rs` / `src/config.rs` | `compute_affected_returns_glob_matched_bail_out_when_go_mod_changes` | Unit (happy) | A diff touching `go.mod` plus an unrelated file returns `Ok(BailOut(GlobMatched{glob:"**/go.mod", path:"go.mod"}))` *before* any graph-walk work — glob check runs first. |
| R5 | `src/affected.rs` | `compute_affected_returns_package_fully_removed_bail_out_when_last_file_in_a_package_is_deleted` | Unit (happy) | Deleting `a/only.go` where `a/` has no other recognized-language files left returns `Ok(BailOut(PackageFullyRemoved{path:"a/only.go"}))`; the equivalent cross-directory rename (`a/foo.go` → `b/foo.go`, emptying `a/`) bails out identically. |
| R5 | `src/affected.rs` | `compute_affected_returns_shallow_clone_bail_out_when_base_has_no_common_history` | Unit (happy) | A shallow clone (or two unrelated root commits) where `base_sha` predates the shallow boundary / shares no history returns `Ok(BailOut(ShallowCloneOrNoCommonHistory))`, not `Err` and not a silently-wrong file list. |
| R5 | `src/affected.rs` | `compute_affected_does_not_bail_out_when_a_deleted_file_leaves_a_surviving_sibling_in_its_package` | Unit (error/negative) | Deleting `a/one_of_two.go` while `a/other.go` still exists resolves `resolve_removed_path` to `Some("example.com/app/a")` — added to the seed set, no bail-out — proving the bail-out is scoped to *full* package removal, not any delete. |
| R5 | `src/affected.rs` | `compute_affected_routes_each_bail_out_category_and_the_hard_error_case_correctly` | Integration (full orchestration: real git repo + real `ArchModel` build) | One consolidated test exercising all three `BailOutReason` categories plus the unresolvable-`--base` hard-error case (R9) through `compute_affected` itself, not just the lower-level helpers — catches a regression where the orchestration wiring (not the helper logic) drops a category. |
| R6 — `AffectedConfig` additive-union: `extra_bail_out_globs` extends, never replaces, `default_bail_out_globs()` (Rabbit Holes: "a hazard if hardcoded narrowly"; resolved Open Question) | `src/config.rs` | `effective_bail_out_globs_equals_defaults_exactly_when_no_override_is_configured` | Unit (happy) | Deserializing `{}` (no `affected` block) yields `config.affected.effective_bail_out_globs()` equal to `default_bail_out_globs()` exactly. |
| R6 | `src/config.rs` | `effective_bail_out_globs_unions_extra_globs_with_defaults_instead_of_replacing_them` | Unit (error/negative — regression guard against the rejected replace-on-set design) | `{"affected":{"extra_bail_out_globs":["proto/**"]}}` yields `effective_bail_out_globs().len() == default_bail_out_globs().len() + 1` and the result still contains `"**/go.mod"` — a maintainer adding a repo-specific glob never silently loses `go.mod`/`go.sum` protection. |
| R6 | — | — | Integration: N/A | Pure in-memory config deserialization; no external call or data store involved. |
| R7 — Reverse-BFS cycle-safety: a graph with an import cycle must not infinite-loop | `src/affected.rs` | `reverse_bfs_terminates_on_a_two_package_import_cycle_and_visits_both_packages` | Unit (happy) | Reusing `import_graph.rs`'s own two-package cycle fixture (`a` imports `b` and vice versa), `reverse_bfs` seeded with `{"a"}` terminates (bounded steps, explicit visited set) and returns exactly `{"a","b"}` — no infinite loop. |
| R7 | `src/affected.rs` | `reverse_bfs_visited_set_prevents_revisiting_a_node_reachable_by_two_paths` | Unit (error/edge) | A diamond-shaped reverse graph (two distinct import paths converging back on the same ancestor) still terminates and produces a deduplicated result set, not a duplicate-visit infinite requeue. |
| R8 — Nested `**/go.mod`-style glob matching at any depth, not just repo root | `src/config.rs` | `default_bail_out_globs_matches_go_mod_go_sum_and_proto_files_at_any_directory_depth` | Unit (happy) | `ChangedFile{path:"go.sum"}`, `ChangedFile{path:"tools/scanner/go.mod"}` (nested, multi-module), and `ChangedFile{path:"api/v1/service.proto"}` all match `default_bail_out_globs()` via `glob::matches_scope`. |
| R8 | `src/config.rs` | `default_bail_out_globs_does_not_match_an_unrelated_go_source_file` | Unit (error/negative) | `ChangedFile{path:"internal/widget/widget.go"}` does not match — the default list isn't so broad it defeats the feature's purpose. |
| R9 — Hard-error vs. `__ALL__`-bail-out split: an unresolvable base ref is a hard error; an unbounded-blast-radius diff is `__ALL__` at exit 0 (explicitly reconciled across `requirements.md`, `research/ux.md`, and `plan.md` Story 1.1.1) | `src/affected.rs` | `compute_affected_returns_err_not_bail_out_for_an_unresolvable_base_ref` | Unit (happy — asserting the correct branch of the split) | `compute_affected(repo_root, "does-not-exist-ref", &config)` returns `Err(_)`, never `Ok(BailOut(_))` — asserted explicitly, not just inferred from an `Ok` case's absence. |
| R9 | `src/affected.rs` | `compute_affected_returns_ok_bail_out_not_err_for_an_unbounded_blast_radius_diff` | Unit (error — the split's contrasting branch) | The same orchestration, given a `go.mod`-touching diff against a *resolvable* base, returns `Ok(BailOut(_))` — a successful computation, not a failure — confirming the two branches never collapse into each other. |
| R9 | `tests/affected_cli.rs` | `affected_cli_exits_zero_for_bail_out_and_nonzero_for_an_unresolvable_base_ref` | Integration (spawns the binary against a real repo) | `--base does-not-exist-ref` exits non-zero with empty stdout and stderr naming the ref; `--base <sha-that-touches-go.mod>` exits `0` with `"__ALL__\n"` on stdout — both asserted in the same test to make the contrast explicit. |
| R10 — `Config.affected`/`AffectedConfig` wiring into `Config` and `kibitzer schema` (Story 1.2.1) | `src/config.rs` | `config_deserializes_affected_block_reachable_via_find_config` | Unit (happy) | `.claude/inspect.json` with `{"affected":{"extra_bail_out_globs":["proto/**"]}}`, loaded via `config::find_config`, exposes `config.affected.extra_bail_out_globs == vec!["proto/**"]`. |
| R10 | `src/config.rs` | `schema_command_includes_affected_config_shape_without_a_hand_written_schema_path` | Unit (error/negative — regression guard) | `Command::Schema`'s output JSON contains an `"affected"` property derived from `AffectedConfig`'s `JsonSchema` derive; a hand-written/forgotten schema entry would show as an absent or stale property here. |
| R11 — `kibitzer architecture affected` CLI surface exists and dispatches correctly (Story 3.2.1) | `tests/affected_cli.rs` | `affected_subcommand_help_lists_path_and_base_flags` | Unit (happy) | `kibitzer architecture affected --help` output contains `--base <BASE>` and `--path <PATH>`. |
| R11 | `tests/affected_cli.rs` | `affected_subcommand_without_required_base_flag_exits_nonzero_with_usage_error` | Unit (error) | Omitting the required `--base` flag exits non-zero with a clap usage error on stderr, before `compute_affected` is ever called. |
| R11 | `tests/affected_cli.rs` | `affected_cli_end_to_end_matches_compute_affected_result_for_the_same_inputs` | Integration (spawns the binary) | For a fixture repo/base pair, the CLI's stdout/exit-code matches what direct unit-testing of `compute_affected`'s `AffectedResult` for the same inputs predicts — confirms no separate/divergent code path between `run_affected` and `compute_affected`. |
| R12 (Success Metric 1 & 3) — Comparison run against `scripts/test-affected.py` on real stapler-squad diffs, output "possible" and side-by-side documented | `docs/affected-validation.md` (new, this repo — not a `src/*.rs` test) | N/A — manual comparison procedure (Task 4.1.3), not an automated `#[test]` | Manual/documented validation, not unit or integration in the `cargo test` sense | ≥10 real historical stapler-squad commits run through both `kibitzer architecture affected --base <parent-sha>` and `python3 scripts/test-affected.py <parent-sha>`; one table row per commit recording both outputs, a match/divergence verdict with a stated root cause for every divergence, and at least one wall-clock timing observation per tool against the "not slower than `test-affected.py`" NFR. |
| R13 (Success Metric 4, restated as an explicit coverage gate) — automated tests collectively cover the graph-walk logic and every bail-out trigger category | `src/affected.rs`, `src/config.rs` | `cargo test affected:: config::` (a CI gate, not a single `#[test]` fn) | Coverage gate | Every acceptance-criterion example in `plan.md` Phases 1-3 has a corresponding passing test (Task 4.1.1a); this row is the checklist item that the rows above are, collectively, that coverage — not a new test in itself. |

## UX Acceptance Tests

N/A — no user-facing GUI surface; see plan.md's CLI/stdout contract (Story 3.2.2, "run_affected's
stdout/exit-code contract"), which the unit tests (`run_affected_prints_*`) and CLI integration
tests (`tests/affected_cli.rs`) above already cover: exact stdout bytes for the package-list,
`__ALL__`-sentinel, empty-result, and error cases, plus the corresponding exit codes.

## Test Stack

- **Unit**: Rust's built-in `#[test]` + `assert!`/`assert_eq!`, inline `#[cfg(test)] mod tests`
  in the module under test (`src/affected.rs`, `src/config.rs`, `src/main.rs`) — matching this
  repo's existing convention (`src/import_graph.rs`, `src/change_coupling.rs`,
  `src/complexity_tests.rs`, `src/arch_export.rs`). Tempdir git-repo fixtures follow
  `change_coupling.rs`'s `TempGitRepo`/`commit_touching` helper shape rather than mocking `git`.
- **Integration**: `std::process::Command`-spawning the built `kibitzer` binary against fixture
  repos in a new `tests/affected_cli.rs`, matching the existing `tests/hook_contract.rs` and
  `tests/false_positives_cli.rs` convention for CLI-contract-level tests (these live outside
  `src/`, unlike the module-level unit tests). No mocks/test doubles for `git` itself — per
  Pattern Decisions in `plan.md`, tests spin up a real git repo in a tempdir rather than
  abstracting git access behind a trait.
- **E2E / UX**: N/A.

## Coverage Targets and How to Measure

| Stack | Coverage command | Target |
|---|---|---|
| Rust | `cargo tarpaulin --out Stdout` | ≥80% line, scoped to `src/affected.rs`, the new `AffectedConfig`/`default_bail_out_globs` code in `src/config.rs`, and the new `package_for_file` method in `src/arch_model.rs` |

- All public functions in the Domain Glossary (`resolve_base_sha`, `diff_changed_files`,
  `build_reverse_adjacency`, `reverse_bfs`, `resolve_changed_packages`, `resolve_removed_path`,
  `compute_affected`, `run_affected`): happy path + error/edge path covered per the table above.
- The one external integration point (shelling out to `git`) has both unit-level tempdir-repo
  tests and at least one full CLI integration test (`tests/affected_cli.rs`) per requirement,
  matching this template's "external call" rule applied to `git` invocations.
- `cargo test affected:: config::` (Task 4.1.1a) is the collective pass/fail gate; `docs/affected-validation.md`
  (Task 4.1.3, R12) is the separate real-world validation step this repo's CLAUDE.md requires in
  place of `kibitzer check backtest` for a feature that isn't a per-file checker.
