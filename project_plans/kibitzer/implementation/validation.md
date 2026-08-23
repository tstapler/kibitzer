# Validation Plan: kibitzer — Native Architecture Linting

**Date**: 2026-08-22

## Happy Path Scenario

Given a Go project whose `.claude/inspect.json` declares an `architecture.components` list
(`domain`, `infra`) and a `dependency_rules` entry restricting `domain` to `may_depend_on: []`,
when `domain/domain.go` imports `infra` and the user runs
`kibitzer check architecture component-deps <dir>` (or calls the `architecture_assessment` MCP
tool), then kibitzer prints a `{file}:{line}: [component-deps] ...` finding naming the violating
edge and exits non-zero / renders the finding as `[blocking]` or `[advisory]` per the configuring
`Check`'s severity.

## Requirement → Test Mapping

| Requirement | Test File | Test Name | Type | Scenario |
|---|---|---|---|---|
| Gap 1 — arbitrary named-component dependency rules (req. #1) | `src/config.rs` | `component_parses_name_and_paths`, `existing_layers_only_config_still_parses_with_empty_new_fields` (Story 0.1.1) | Unit | New `Component`/`DependencyRule` schema parses; old `layers`-only config unaffected |
| Gap 1 | `src/config.rs` | `layers_desugar_to_four_glob_patterns_per_layer`, `layers_desugar_to_suffix_allow_lists_not_pairwise`, `component_of_returns_first_declaration_order_match`, `component_of_returns_none_for_unmatched_node`, `rejects_layer_and_component_name_collision` (Story 0.1.2) | Unit | `layers` desugars to components/rules; resolution order; name-collision error |
| Gap 1 | `src/config.rs` | `rejects_unknown_component_in_dependency_rule`, `rejects_unknown_component_in_content_rule`, `rejects_unknown_component_in_naming_rule`, `unknown_component_error_omits_suggestion_when_no_close_match` (Story 0.1.3) | Unit | Typo'd component reference fails fast with Levenshtein suggestion |
| Gap 1 | `src/architecture_checks.rs` | `component_deps_flags_disallowed_edge`, `component_deps_deny_wins_over_allow`, `component_deps_denies_by_default_with_no_rule`, `component_deps_ignores_same_component_edges`, `component_deps_ignores_unmapped_nodes` (Story 1.1.1) | Unit | `ComponentDependencyChecker` allow/deny/default-deny semantics |
| Gap 1 — `layers` backward compat (req. Success Metric #3) | `src/architecture_checks.rs` | 1 test per existing `LayeringChecker` fixture via `assert_same_findings_by_location` (Task 1.1.2b — **names not given in plan, see Gap Analysis below**); `layers_desugar_does_not_substring_match_segment_names` (Task 1.1.2c) | Unit (golden regression) | `component-deps` over `effective_*()` produces identical `(file, line)` findings to `LayeringChecker` |
| Gap 1 — zero-match advisory / severity_override (repair-loop fix, plan.md Pattern Decisions) | `src/architecture_checks.rs`, `src/check.rs`, `src/mcp.rs` | `component_deps_flags_zero_match_component_as_advisory_even_when_check_is_blocking`, `cache_json_without_findings_field_deserializes_with_empty_findings`, `architecture_assessment_renders_zero_match_advisory_as_advisory_under_a_blocking_check`, `architecture_assessment_still_renders_a_real_blocking_violation_as_blocking` (Story 1.1.3) | Unit + MCP integration | Zero-match component glob always renders `[advisory]`; a real violation under the same blocking `Check` still renders `[blocking]`; old `cache.json` deserializes without the new fields |
| Gap 1 — dual-registry dispatch plumbing | `src/config.rs` | `rejects_unknown_architecture_checker_name`, `accepts_architecture_checker_with_batch_trigger` (existing, re-verified, Task 1.2.1e) | Unit (regression) | `validate()` still rejects/accepts through `lookup_any_architecture_checker` |
| Gap 1 — CLI verb (requirements.md's stated `kibitzer check native/architecture` success metric) | `src/main.rs` | **Not named in plan — see Gap Analysis.** Proposed: `cli_architecture_check_flags_violation_and_exits_nonzero`, `cli_architecture_check_unknown_checker_exits_nonzero_with_message` (Story 1.2.2, Task 1.2.2c) | CLI integration | `kibitzer check architecture component-deps <dir>` prints finding + exits 1; unknown checker name exits non-zero with message |
| Gap 2 — package-content rules (req. #2) | `src/declarations.rs` | `go_declarations_finds_struct_and_function`, `go_declarations_finds_interface` (Story 2.1.1); `ts_declarations_finds_class_and_interface` (Story 2.1.2) | Unit (disk fixture) | `DeclarationGraph` extraction for Go/TS |
| Gap 2 | `src/declaration_checks.rs` | `lookup_returns_none_before_any_checker_registered` (Story 2.1.3) | Unit | Empty/populated `DeclarationChecker` registry lookup |
| Gap 2 | `src/declaration_checks.rs` | `content_checker_flags_disallowed_kind`, `content_checker_skips_unmapped_declarations`, `content_checker_allows_everything_with_no_rule_for_component`, `content_checker_flags_zero_match_component_as_advisory` (Story 2.2.1) | Unit | `ContentRule` violation/skip/no-rule/zero-match-advisory semantics |
| Gap 2 | `src/config.rs`, `src/mcp.rs` | `accepts_content_rules_architecture_checker` (Task 2.2.2a); MCP integration test extending `architecture_assessment_reports_cycle_and_layering_findings` (Task 2.2.2b — **name not given, see Gap Analysis**) | Unit + MCP integration | `content-rules` parses and flows through `architecture_assessment` output |
| Gap 2 — git-HEAD-baseline dispatch fix for Declaration-kind checkers (repair-loop fix, Story 2.2.3, adversarial-review.md BLOCKER) | `src/check.rs` | `content_rules_blocking_violation_downgrades_when_it_predates_head`, `content_rules_blocking_violation_stays_blocking_when_new_since_head`, `naming_rules_blocking_violation_downgrades_when_it_predates_head`, `import_kind_checker_head_baseline_downgrade_still_works_through_dual_registry_dispatch` (Task 2.2.3c/d) | Unit/integration (git fixture via `TempRepo`) | Pre-existing `content-rules`/`naming-rules` violations downgrade to advisory at git HEAD baseline, same as `import-cycles`/`layering`/`coupling`/`component-deps`; new violations stay blocking |
| Gap 3 — naming-convention rules (req. #3) | `src/declaration_checks.rs` | `naming_checker_flags_non_matching_name`, `naming_checker_allows_matching_name`, `naming_checker_scoped_to_declared_kind_only` (Story 3.1.1) | Unit | `NamingRule` regex match/no-match/kind-scoping |
| Gap 3 | `src/config.rs` | `rejects_invalid_naming_rule_regex` (Story 3.1.1) | Unit | Invalid regex is a load-time error, not a runtime panic |
| Gap 3 | `src/declaration_checks.rs` | `naming_checker_flags_zero_match_rule_as_advisory`, `naming_checker_flags_zero_match_component_as_advisory` (Story 3.1.2) | Unit | Zero-match naming rule / zero-match component both surface as advisory |
| Gap 4 — Python/Java/Kotlin import-graph extraction (req. #4) | `src/import_graph.rs` | `java_import_to_sexp_fixture` (Story 4.1.1); **Kotlin fixture test name not given — see Gap Analysis, proposed** `kotlin_import_to_sexp_fixture` (Story 4.1.2) | Unit (grammar verification) | Real `to_sexp()` node shapes pinned before extraction code is written |
| Gap 4 | `src/import_graph.rs` | Existing `go_import_graph_finds_a_two_package_cycle`, `go_import_of_stdlib_package_is_ignored` re-run unmodified (Task 4.2.1d, verification only) | Unit (regression) | `build_go` → `build_qualified_name_language` refactor is behavior-preserving |
| Gap 4 | `src/import_graph.rs` | `java_import_graph_finds_a_two_package_cycle_with_normalized_identity`, `kotlin_import_graph_handles_plain_wildcard_and_aliased_imports`, `normalized_java_identity_matches_slash_globs` (Story 4.2.2) | Unit (disk fixture) | Java/Kotlin import extraction; dot-to-slash package-identity normalization matches `/`-segment globs |
| Gap 4 | `src/declarations.rs` | `java_declarations_distinguishes_class_and_interface`, `kotlin_declarations_distinguishes_class_interface_object` (Story 4.3.1) | Unit (disk fixture) | Java/Kotlin declaration extraction for content/naming rules |
| Gap 4 | `src/import_graph.rs` | **Python fixture test name not given — see Gap Analysis, proposed** `python_import_to_sexp_fixture` (Story 5.1.1) | Unit (grammar verification) | Real `to_sexp()` output for all 6 Python import node kinds |
| Gap 4 | `src/import_graph.rs` | `python_relative_import_resolves_to_sibling_package`, `python_absolute_import_resolves_via_init_py_heuristic`, `python_future_import_produces_no_edge` (Story 5.2.1) | Unit (disk fixture) | Python relative/absolute import resolution; `__future__` produces no edge |
| Gap 4 | `src/declarations.rs` | `python_declarations_finds_class_and_module_level_function_only` (Story 5.3.1) | Unit (disk fixture) | Python class/function declaration extraction, methods excluded |
| Dogfooding (requirements.md Phase 7 scope) | `testdata/dogfood-architecture/`, `.claude/inspect.json` | Task 7.1.1d / 7.1.2c ("run it, don't read it" verification — no unit test, real CLI invocation against a checked-in Go fixture) | End-to-end / manual verification | All 3 new rule categories fire against a real (if small) Go fixture; `kibitzer run . --trigger batch` at the kibitzer repo root exits 0 with advisory findings |

## UX Acceptance Tests

CLI/config-file/MCP-tool-output surface — no browser UI. "UX acceptance test" here means an
integration test asserting on exact/contained output text, matching the existing convention in
`src/mcp.rs`'s `#[tokio::test]` async integration tests (`output.contains(...)` assertions against
the `architecture_assessment` MCP tool's rendered string) and `src/check.rs`'s `TempRepo`-based git
fixture tests.

| # | UX Criterion | Test File | Test Name | Tool | Steps |
|---|---|---|---|---|---|
| 1 | `layers`-only config parses unchanged, byte-identical `LayeringChecker` findings | `src/config.rs`, `src/architecture_checks.rs` | `existing_layers_only_config_still_parses_with_empty_new_fields` + Story 1.1.2's golden-regression tests (names not given, see Gap Analysis) | `cargo test` | Parse a `layers`-only config; assert new fields are empty `Vec`s; run both `LayeringChecker` and `ComponentDependencyChecker` over the desugared config and assert set-equal `(file, line)` findings |
| 2 | Config-load error for undefined component reference names the bad value + edit-distance-≤2 suggestion | `src/config.rs` | `rejects_unknown_component_in_dependency_rule`, `rejects_unknown_component_in_content_rule`, `rejects_unknown_component_in_naming_rule`, `unknown_component_error_omits_suggestion_when_no_close_match` | `cargo test` | Assert `Err` message equals/contains the exact `"...undefined component 'x' — declared components are: ... (did you mean 'y'?)"` shape; assert the suggestion clause is omitted when no name is within distance 2 |
| 3 | Every finding line grep-able via a mandatory `[category]` prefix, uniform across all 6 categories | `src/architecture_checks.rs` (message-format audit, Task 6.1.1a/b/c) | **No end-to-end "grep all 6 categories at once" test named — see Gap Analysis.** Proposed: `architecture_assessment_every_category_finding_has_bracket_prefix` (`src/mcp.rs`) | MCP integration | Configure a fixture producing at least one finding per category (`import-cycle`, `layering`, `coupling`, `component-deps`, `content`, `naming`); assert every finding line matches `^\[[a-z-]+\]` |
| 4 | Per-category count breakdown line under the aggregate count | `src/mcp.rs` | Extends `architecture_assessment_reports_cycle_and_layering_findings` (Task 6.1.2c — **new assertions, no distinct test name given**; design/ux.md already flags this fixture as omitting `component-deps`) | MCP integration | Assert output contains a `"  import-cycle: N, layering: N, ..."` line immediately after the aggregate count line; include a `component-deps` finding in the fixture, closing design/ux.md's noted gap |
| 5 | No advisory finding masquerades as blocking; no blocking finding is silently downgraded to advisory | `src/architecture_checks.rs`, `src/mcp.rs` | `component_deps_flags_zero_match_component_as_advisory_even_when_check_is_blocking`, `architecture_assessment_renders_zero_match_advisory_as_advisory_under_a_blocking_check`, `architecture_assessment_still_renders_a_real_blocking_violation_as_blocking` | Unit + MCP integration | Configure `component-deps` as `"severity": "blocking"`; assert the zero-match-component advisory renders `[advisory]`; assert a real violation from the same `Check` still renders `[blocking]` |
| 6 | Zero-match-component-glob advisory fires regardless of which rule category references it | `src/architecture_checks.rs`, `src/declaration_checks.rs` | `component_deps_flags_zero_match_component_as_advisory_even_when_check_is_blocking`, `content_checker_flags_zero_match_component_as_advisory`, `naming_checker_flags_zero_match_component_as_advisory` | Unit | Each of the 3 checkers independently emits the shared `[component]` advisory via `zero_match_advisory<T>` |
| 7 | Deny-by-default and deny-wins-over-allow are documented, tested precedence rules | `src/architecture_checks.rs` | `component_deps_deny_wins_over_allow`, `component_deps_denies_by_default_with_no_rule` | Unit | Deny list wins even when the same name also appears in `may_depend_on`; a declared component with no `DependencyRule` denies all cross-component edges |
| 8 | `kibitzer check architecture <name> <dir>` prints one `{file}:{line}: {message}` line per finding, exits non-zero, distinct message for unknown checker | `src/main.rs` | **Not named in plan — see Gap Analysis.** Proposed: `cli_architecture_check_flags_violation_and_exits_nonzero`, `cli_architecture_check_unknown_checker_exits_nonzero_with_message` | CLI integration (`#[cfg(test)]` calling the extracted logic function directly, matching Task 1.2.2c's stated fallback since `main.rs` has no existing subprocess-test precedent — confirmed, `grep '#\[test\]' src/main.rs` returns nothing today) | Run against the Phase 7 dogfood fixture; assert stdout contains `domain/domain.go:` and `[component-deps]`, exit code 1; run with `does-not-exist`, assert stderr contains `"no architecture checker named 'does-not-exist' registered"`, exit code non-zero |
| 9 | Content/naming findings reuse `ArchFinding`'s existing fields — no new finding shape | `src/declaration_checks.rs` | `content_checker_flags_disallowed_kind`, `naming_checker_flags_non_matching_name` | Unit | Assert `file`/`line`/`message` fields are populated identically to how `ImportCycleChecker`/`LayeringChecker` findings are asserted (same `ArchFinding` struct, no new variant) |
| 10 | Mermaid diagram groups nodes into `subgraph {component}` blocks; unmatched nodes render outside any subgraph, exactly as before | `src/mermaid.rs` | `mermaid_diagram_groups_nodes_by_component` — **only covers the matched-node half; the "unmatched renders as before" half has no fixture with an unmatched node in the stated Given/When/Then, see Gap Analysis.** Proposed addition to the same test: assert an unmatched node's slugified id appears outside every `subgraph...end` block | Unit | Assert `"subgraph domain"`/`"subgraph infra"` appear, each containing their matched nodes' slugified ids; assert a third, unmatched node's id appears in the output but not nested inside any `subgraph` block |

## Test Stack
- **Unit**: Rust's built-in `#[test]` harness (`cargo test`), inline `#[cfg(test)] mod tests` per source file — no external test framework, no `dev-dependencies` declared in `Cargo.toml` (verified: `[dev-dependencies]` section is absent). Disk-fixture tests write real files under `std::env::temp_dir()` via a local `tmp_dir()`/`write()` helper (established in `src/import_graph.rs`'s test module) rather than mocking the filesystem or tree-sitter.
- **Integration**: `#[tokio::test]` async tests in `src/mcp.rs` calling `KibitzerServer::architecture_assessment(...)` directly (in-process, no subprocess) and asserting on the returned string; `TempRepo`-based real-git-repo fixtures in `src/check.rs` for git-HEAD-baseline-downgrade tests (existing precedent: `baseline_fails_when_violation_predates_the_edit`, `repo_wide_baseline_fails_when_violation_predates_the_edit`).
- **CLI/Output**: No existing precedent in `src/main.rs` (verified: no `#[test]`/`#[cfg(test)]` present there today). Story 1.2.2's own Task 1.2.2c anticipates this and recommends extracting the CLI verb's logic into a plain function callable from a `#[cfg(test)]` block rather than spawning a subprocess (matching how `mcp.rs` tests call `architecture_assessment` in-process) — no new `assert_cmd`/`assert_fs` dev-dependency is implied or needed.

## Coverage Targets and How to Measure

| Stack | Coverage command | Target |
|---|---|---|
| Rust | `cargo tarpaulin --out Stdout` — **not currently wired into CI**: `.github/workflows/ci.yml` runs `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace` only; no coverage tool is configured anywhere in the repo (verified: no `tarpaulin`/`llvm-cov` reference in `Cargo.toml`, `.github/workflows/*.yml`, or any `Makefile`). `cargo tarpaulin` is proposed here as the default (no alternative tool found to defer to) — adding it to CI is out of scope for this feature and not requested by requirements.md. | ≥80% line coverage on the modified/new modules (`src/config.rs`, `src/architecture_checks.rs`, `src/declarations.rs` (new), `src/declaration_checks.rs` (new), `src/import_graph.rs`, `src/check.rs`, `src/mcp.rs`, `src/mermaid.rs`) |

## Gap Analysis (Step 1/2/3 findings)

**Requirements coverage: 6/6.** All 4 requirements.md Scope gaps (dependency rules, content rules,
naming rules, Python/Java/Kotlin import extraction) and both repair-loop fixes (Story 2.2.3's
git-HEAD-baseline dispatch fix, Story 1.1.3's `severity_override` fix) have substantial named-test
coverage across Phases 0–5 above.

**UX acceptance tests: 10/10 mapped**, but 4 of the 10 map to a test plan.md describes only
generically (task text says "add a test"/"extend the existing test" without naming a function), not
to a concretely named one:

1. **CLI subcommand test names never given** (Story 1.2.2, Task 1.2.2c; UX criterion 8, and the
   requirements.md CLI success-metric row above). The two acceptance criteria are concrete and
   testable, but no `fn name` is written anywhere in Phase 1. This is the most consequential gap —
   it's also the *only* new user-facing entry point this feature adds (`kibitzer check architecture
   <name> <dir>`), and `src/main.rs` has zero existing test precedent to imply a name from context.
   Proposed names above.
2. **`layers`-desugar golden-regression test names never given** (Story 1.1.2, Task 1.1.2b — "one
   test per fixture," 4 fixtures, no names listed, unlike almost every other story in this plan).
   Lower risk since the *behavior* (Task 1.1.2a's `assert_same_findings_by_location` helper) is
   well-specified even though the 4 individual test names aren't.
3. **Kotlin and Python `to_sexp()` fixture test names never given** (Tasks 4.1.2a, 5.1.1a — both say
   "same pattern as Task 4.1.1a" without restating the name, unlike 4.1.1a's own explicit `fn
   java_import_to_sexp_fixture()`). Low risk — the naming pattern is obvious by analogy.
4. **MCP content-rules integration test name never given** (Story 2.2.2, Task 2.2.2b — "extend the
   existing test" without naming the extended/new test). Low risk, same class as #2.

**Test-naming convention (Step 3): no violations found.** Every explicitly named test in plan.md —
across all 8 phases — already follows the codebase's actual `<subject>_<condition>_<expected_outcome>`
snake_case convention (verified against `src/architecture_checks.rs`'s `layering_flags_a_reverse_dependency`
and `src/import_graph.rs`'s `go_import_graph_finds_a_two_package_cycle`). No test uses
`methodName_should_X_when_Y`, camelCase, or a `should`/`when` boilerplate prefix.

**Secondary UX-completeness gap found in this pass**: Story 6.2.2's Mermaid-subgraph acceptance
criterion has two clauses ("nodes matching a component get grouped" / "nodes matching no component
render outside any subgraph, exactly as today") but its one Given/When/Then example only exercises
the first clause — both example nodes match a component. The single named test
(`mermaid_diagram_groups_nodes_by_component`) is therefore at risk of covering only half its own
acceptance criterion unless implementation adds a third, unmatched node to the fixture (proposed
above). This mirrors the same class of gap design/ux.md already flagged for Story 6.1.2's
category-breakdown fixture (`component-deps` omitted from the illustrative example).
