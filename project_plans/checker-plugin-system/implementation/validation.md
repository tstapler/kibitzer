# Validation Plan: checker-plugin-system

**Date**: 2026-09-06

## Happy Path Scenario

Given a baseline kibitzer install with an empty plugin `Registry` (no `registry.json`,
`default_checks()` running unmodified), when the user runs
`kibitzer plugin install kibitzer-stub-plugin --source <manifest>` and then any check
invocation (`kibitzer run . --trigger batch`, the MCP `run_checks` tool, or the PostToolUse
hook), then the stub plugin's canned SARIF finding (`ruleId ==
"kibitzer-stub-plugin-finding"`) appears in kibitzer's normal check output — proving the
install → register → run path works end-to-end, with the check wired into the effective
config automatically and with no `.claude/inspect.json` hand-editing.

## Requirement → Test Mapping

| Requirement | Test File | Test Name | Type | Scenario |
|---|---|---|---|---|
| REQ-SM1: core binary's `Cargo.toml` gains zero new hard deps; `cargo build --workspace` succeeds with the stub as a sibling member (Success Metrics; Story 1.1.1) | N/A | `cargo build --workspace && cargo metadata --no-deps --format-version 1 \| jq '.packages[].name'` (Task 1.1.1b) | Build verification | Happy path — two packages listed, exit 0 |
| REQ-1.1.2: stub plugin binary emits valid SARIF 2.1.0 with exactly one result (Story 1.1.2) | `crates/kibitzer-stub-plugin/src/main.rs` | `stub_plugin_emits_sarif_with_one_warning_level_result` | Unit | Happy path — build the JSON value, parse it back, assert `runs[0].results[0].ruleId == "kibitzer-stub-plugin-finding"` and `level == "warning"` |
| REQ-1.2.1: `dist plan` treats `kibitzer-stub-plugin` as an independent App with no Homebrew formula (Story 1.2.1) | N/A | `dist plan` output inspection (Task 1.2.1b) | Build/CLI verification | Happy path — two Apps, four targets each, one Homebrew formula (kibitzer only) |
| REQ-2.1.1a: `PluginManifest` deserializes all fields from JSON (Story 2.1.1) | `src/plugin.rs` | `plugin_manifest_deserializes_all_fields_from_json` | Unit | Happy path |
| REQ-2.1.1b: `default_plugin_dir()` respects `XDG_DATA_HOME` | `src/plugin.rs` | `default_plugin_dir_respects_xdg_data_home_override` | Unit | Happy path |
| REQ-2.1.1c: `PluginName::parse` rejects a malicious/malformed name at construction (architecture-review.md BLOCKER) | `src/plugin.rs` | `plugin_name_parse_rejects_path_traversal_and_slash_and_empty_strings` | Unit | Error path |
| REQ-2.1.1d: `PluginName::parse` accepts a well-formed name | `src/plugin.rs` | `plugin_name_parse_accepts_valid_identifier` | Unit | Happy path |
| REQ-2.1.1e: an invalid CLI-supplied name is rejected by clap before `main()` dispatch, before any filesystem op | `tests/plugin_contract.rs` | `plugin_remove_rejects_path_traversal_name_via_clap_before_any_filesystem_op` | Integration | Error path |
| REQ-2.1.2a: a saved `Registry` loads back with identical content (Story 2.1.2) | `src/plugin.rs` | `registry_save_then_load_round_trips_installed_plugin` | Unit | Happy path |
| REQ-2.1.2b: loading a nonexistent registry file returns `Registry::default()`, never errors | `src/plugin.rs` | `registry_load_returns_default_when_file_missing` | Unit | Error path |
| REQ-2.1.2c: loading a corrupt registry file also fails open to `Registry::default()` | `src/plugin.rs` | `registry_load_returns_default_when_file_contents_are_corrupt` | Unit | Error path |
| REQ-2.1.3a: version comparator rejects an incompatible (too-low) current version (Story 2.1.3, ADR-003) | `src/plugin.rs` | `version_meets_minimum_returns_false_when_current_is_older_than_minimum` | Unit | Error path |
| REQ-2.1.3b: version comparator accepts a compatible current version | `src/plugin.rs` | `version_meets_minimum_returns_true_when_current_meets_minimum` | Unit | Happy path |
| REQ-2.1.3c: comparison is numeric, not lexicographic (`"0.10.0" >= "0.9.0"`) | `src/plugin.rs` | `version_meets_minimum_compares_numerically_not_lexicographically` | Unit | Happy path (regression guard) |
| REQ-2.1.3d: malformed version strings fail to parse | `src/plugin.rs` | `parse_plain_version_rejects_non_numeric_version_string` | Unit | Error path |
| REQ-3.1.1a: `kibitzer plugin list` with zero plugins prints the exact message and exits 0 (Story 3.1.1) | `tests/plugin_contract.rs` | `plugin_list_prints_no_plugins_installed_message_when_registry_empty` | Integration | Happy path |
| REQ-3.1.1b: a malicious plugin name is rejected by clap before any subcommand body runs | `tests/plugin_contract.rs` | `plugin_remove_rejects_invalid_name_before_dispatch_reaches_remove_plugin` | Integration | Error path |
| REQ-3.2.1a: a local-path `source` is read directly with no network call (Story 3.2.1) | `src/plugin.rs` | `fetch_manifest_reads_local_path_without_network_call` | Unit | Happy path |
| REQ-3.2.1b: an `https://` source with a non-allowlisted host is rejected before any request (ADR-002) | `src/plugin.rs` | `fetch_manifest_rejects_disallowed_https_host_before_request` | Unit | Error path |
| REQ-3.2.1c: target-triple resolution maps the current OS/arch to one of the four known triples | `src/plugin.rs` | `current_target_triple_resolves_known_os_arch_pair` | Unit | Happy path |
| REQ-3.2.1d: target-triple resolution bails on an unsupported platform | `src/plugin.rs` | `current_target_triple_errors_on_unsupported_platform` | Unit | Error path |
| REQ-3.2.1e: `fetch_target_bytes` strips a `file://` prefix before reading a local fixture path | `src/plugin.rs` | `fetch_target_bytes_strips_file_url_prefix_before_reading_local_path` | Unit | Happy path |
| REQ-3.2.2a: a checksum mismatch hard-fails and leaves no trace on disk or in the registry (Story 3.2.2, ADR-002) | `tests/plugin_contract.rs` | `install_plugin_rejects_checksum_mismatch_and_leaves_no_trace` | Integration | Error path |
| REQ-3.2.2b: a version-incompatible manifest is refused before any binary download (ADR-003) | `tests/plugin_contract.rs` | `install_plugin_rejects_incompatible_min_kibitzer_version_before_download` | Integration | Error path |
| REQ-3.2.2c: re-installing the same version is a no-op success (idempotent) | `tests/plugin_contract.rs` | `install_plugin_is_idempotent_noop_when_already_installed_at_same_version` | Integration | Happy path |
| REQ-3.2.2d: the version-compat + already-installed gates are pure decision logic, independent of I/O | `src/plugin.rs` | `install_gate_short_circuits_on_already_installed_same_version` | Unit | Happy path |
| REQ-3.3.1a: `status` reports `binary missing` distinctly from `ok` (Story 3.3.1) | `tests/plugin_contract.rs` | `plugin_status_reports_binary_missing_when_file_deleted_out_of_band` | Integration | Error path |
| REQ-3.3.1b: `status` reports `ok` when the binary is present and its hash matches | `tests/plugin_contract.rs` | `plugin_status_reports_ok_when_binary_present_and_hash_matches` | Integration | Happy path |
| REQ-3.3.1c: `list_plugins` returns registry entries | `src/plugin.rs` | `list_plugins_returns_registry_entries_in_order` | Unit | Happy path |
| REQ-3.3.1d: `plugin_status` on an unregistered name errors with `"no plugin named"` | `src/plugin.rs` | `plugin_status_errors_with_no_plugin_named_when_name_not_registered` | Unit | Error path |
| REQ-3.3.2a: `remove` refuses when a live local `.claude/inspect.json` check still references the plugin (Story 3.3.2) | `tests/plugin_contract.rs` | `remove_plugin_refuses_when_referenced_by_local_inspect_json_check` | Integration | Error path |
| REQ-3.3.2b: `remove --force` proceeds despite the reference, deleting the binary dir and registry entry | `tests/plugin_contract.rs` | `remove_plugin_with_force_deletes_binary_and_registry_entry_despite_reference` | Integration | Happy path |
| REQ-3.3.2c: `remove_plugin` deletes the registry entry and binary directory when unreferenced | `src/plugin.rs` | `remove_plugin_deletes_registry_entry_and_binary_directory_when_unreferenced` | Unit | Happy path |
| REQ-4.1.1a: an installed plugin's check appears alongside every default in `find_effective_config` (Story 4.1.1) | `src/config.rs` | `find_effective_config_includes_registered_plugin_check_alongside_defaults` | Unit | Happy path |
| REQ-4.1.1b: zero plugins installed leaves `default_checks()`'s names/order byte-for-byte unchanged | `src/config.rs` | `find_effective_config_matches_default_checks_exactly_when_no_plugins_registered` | Unit | Error path (regression guard for the "no plugins" default state) |
| REQ-4.1.1c: the plugin check surfaces through the full CLI/config stack, not just the in-process function | `tests/plugin_contract.rs` | `plugin_install_register_run_end_to_end_surfaces_stub_finding` | Integration | Happy path |
| REQ-4.2.1a: a hung command is killed and reported as a timeout, not left hanging (Story 4.2.1) | `src/check.rs` | `run_check_reports_timeout_and_kills_process_when_command_hangs` | Unit | Error path |
| REQ-4.2.1b: a normal, fast-exiting command is completely unaffected by the timeout wiring | `src/check.rs` | `run_check_behaves_unchanged_when_command_exits_quickly` | Unit | Happy path |
| REQ-4.3.1a: `missing_binary_for` distinguishes a plugin check with an absent binary from a non-plugin check (Story 4.3.1) | `src/plugin.rs` | `missing_binary_for_returns_path_when_plugin_binary_absent` | Unit | Happy path |
| REQ-4.3.1b: `missing_binary_for` returns `None` for a name that isn't a registered plugin at all | `src/plugin.rs` | `missing_binary_for_returns_none_for_non_plugin_check_name` | Unit | Error path (negative case) |
| REQ-4.3.1c: `run_check` short-circuits with `plugin_missing: true` and forced `Advisory` severity, never spawning the command | `src/check.rs` | `run_check_returns_plugin_missing_result_with_advisory_severity_when_binary_absent` | Unit | Error path |
| REQ-4.3.1d: MCP `list_checks` appends `plugin=not-installed` to a plugin-backed check line whose binary is missing | `src/mcp.rs` | `list_checks_appends_plugin_not_installed_tag_when_binary_missing` | Unit | Happy path (tag renders) |
| REQ-4.3.1e: MCP `run_checks` renders `[skipped]`, not `[Advisory]`/`[Blocking]`, for a `plugin_missing` result | `src/mcp.rs` | `run_checks_renders_skipped_prefix_for_plugin_missing_result` | Unit | Happy path (rendering) |
| REQ-4.3.1f: the hook's PostToolUse advisory-context line also renders `[skipped]` for the same case | `src/hook.rs` | `hook_advisory_context_renders_skipped_prefix_for_plugin_missing_result` | Unit | Happy path (rendering) |
| REQ-4.3.1g: MCP/hook surfacing works against a real spawned process, not just constructed structs | `tests/plugin_contract.rs` | `run_checks_over_mcp_reports_skipped_not_blocking_when_plugin_binary_missing` | Integration | Happy path |
| REQ-5.1.1: install → register → run works end-to-end via `CARGO_BIN_EXE` subprocesses, no real network call (Story 5.1.1) | `tests/plugin_contract.rs` | `plugin_install_register_run_end_to_end_surfaces_stub_finding` | Integration | Happy path (see Happy Path Scenario above) |
| REQ-5.2.1: `docs/plugins.md` exists and `CLAUDE.md` links it (Story 5.2.1) | N/A | manual review of `docs/plugins.md` + `CLAUDE.md`'s "Default check catalog" paragraph (Task 5.2.1a/b) | Manual/doc check | Happy path |
| REQ-5.3.1: PR body contains a real, unedited five-command terminal transcript (Story 5.3.1) | N/A | manual walkthrough capture (Task 5.3.1a) | Manual/process check | Happy path |
| REQ-SM2: uninstalling/never installing leaves default behavior unchanged (Success Metrics) | `src/config.rs`, `src/plugin.rs` | `find_effective_config_matches_default_checks_exactly_when_no_plugins_registered`, `remove_plugin_deletes_registry_entry_and_binary_directory_when_unreferenced` | Unit | Happy + error paths together close this metric |

## UX Acceptance Tests

Scripted CLI invocations against a built `kibitzer` binary (`CARGO_BIN_EXE_kibitzer`), each
asserting on stdout/stderr/exit-code/filesystem-state — no browser involved.

| UX Criterion (ux.md §"UX acceptance criteria") | Test File | Test Name | Tool | Steps |
|---|---|---|---|---|
| 1. Install completes in exactly 1 command, zero prompts | `tests/plugin_contract.rs` | `ux_install_completes_in_one_command_with_no_prompts` | CLI subprocess | Run `kibitzer plugin install kibitzer-stub-plugin --source <manifest>` once; assert exit 0, stdout contains `installed kibitzer-stub-plugin`, and no stdin was read (process doesn't block waiting for input — assert by not providing a stdin pipe). |
| 2. Confirm install in 1 additional command, without reading any file directly | `tests/plugin_contract.rs` | `ux_plugin_list_confirms_install_without_reading_files_directly` | CLI subprocess | After install, run `kibitzer plugin list`; assert stdout contains `kibitzer-stub-plugin v0.1.0`; the test itself never opens `registry.json` to derive the expected string — it's a literal fixture constant. |
| 3a. Undo install in 1 command when unreferenced | `tests/plugin_contract.rs` | `ux_remove_undoes_install_in_one_command_when_unreferenced` | CLI subprocess | After install, run `kibitzer plugin remove kibitzer-stub-plugin` (no `--force`); assert exit 0, `default_plugin_dir().join("kibitzer-stub-plugin")` no longer exists. |
| 3b. Undo requires 2 commands (or `--force`) when a local check references the plugin | `tests/plugin_contract.rs` | `ux_remove_requires_force_or_edit_when_referenced_by_local_check` | CLI subprocess | Write a `.claude/inspect.json` with a `kibitzer-stub-plugin` check entry; run `kibitzer plugin remove kibitzer-stub-plugin` (fails, non-zero); run again with `--force` (succeeds, exit 0). |
| 4. Re-running `install` at the same version is a no-op success | `tests/plugin_contract.rs` | `ux_reinstall_same_version_is_noop_success_without_checking_list_first` | CLI subprocess | Install once; install again with the same manifest immediately (no intervening `plugin list`); assert second run exits 0 and stdout contains `already installed`. |
| 5. Every error state names the fix command or explicitly states no state changed | `tests/plugin_contract.rs` | `ux_every_install_error_state_names_next_action_not_a_bare_stack_trace` | CLI subprocess | Drive E1 (missing source), E2 (disallowed host), E3 (incompatible version), E4 (unreachable URL), E5 (checksum mismatch) each as a separate invocation; assert each exits non-zero and stderr contains its documented substring (`"not an allowed host"`, `"requires kibitzer"`, `"checksum mismatch"`, etc.), never a raw Rust panic/backtrace. |
| 6. No `--skip-checksum`/`--insecure` escape hatch exists for E2/E5 | `tests/plugin_contract.rs` | `ux_no_insecure_flag_exists_to_bypass_checksum_or_host_allowlist` | CLI subprocess | Run `kibitzer plugin install --help`; assert stdout does not contain `--insecure` or `--skip-checksum`; separately assert a checksum-mismatch install still fails even if an unrecognized flag is passed (clap rejects the unknown flag). |
| 7. No partial binary left under `default_plugin_dir()` after a checksum-mismatch error | `tests/plugin_contract.rs` | `ux_checksum_mismatch_leaves_no_partial_binary_on_disk` | CLI subprocess + filesystem check | Run install with a manifest whose `sha256` doesn't match; assert non-zero exit, then assert `default_plugin_dir().join("kibitzer-stub-plugin")` does not exist at all (not even a temp file). |
| 8. Every error row states a next step (retry / edit config / upgrade / different source) | `tests/plugin_contract.rs` | (covered by criterion 5's test, same assertions) | CLI subprocess | Same as criterion 5 — each error message is asserted to name its specific remedy, not just "an error occurred". |
| 9. `remove`'s refusal names the exact file and check name | `tests/plugin_contract.rs` | `ux_remove_refusal_names_inspect_json_and_check_name_explicitly` | CLI subprocess | Same setup as 3b's refusal case; assert stderr contains both `.claude/inspect.json` and `kibitzer-stub-plugin` literally, not a generic "in use" message. |
| 10. An agent can distinguish "present" from "missing" via `list_checks` without running the check | `tests/plugin_contract.rs` | `ux_list_checks_tag_lets_agent_distinguish_missing_plugin_without_running_it` | MCP tool invocation (`mcp__kibitzer__list_checks` via a direct call, or the CLI-equivalent render path) | Install, then delete the binary out-of-band; call `list_checks`; assert the plugin's line contains `plugin=not-installed`; assert no subprocess for the plugin binary was ever spawned (no stderr/exit-code artifact from it). |
| 11. An agent can distinguish a missing-plugin skip from a real finding without parsing exit-127 stderr | `tests/plugin_contract.rs` | `ux_run_checks_skipped_prefix_avoids_exit_127_stderr_parsing` | CLI/MCP `run_checks` invocation | Same missing-binary setup; run `run_checks`; assert the rendered line is prefixed `[skipped]` and contains `is not installed`, and that no `sh`/exit-127 text appears anywhere in the output. |
| 12. A missing-plugin skip is never rendered with a `[Blocking]`/`[Advisory]` bracket | `tests/plugin_contract.rs` | `ux_missing_plugin_skip_never_rendered_with_severity_bracket` | CLI/MCP `run_checks` invocation | Register the plugin with `severity: blocking` in its manifest, then delete its binary; run `run_checks`; assert the output line starts with `[skipped]`, never `[Blocking]`. |
| 13. Accessibility — N/A (CLI-only, no color-only signal, no keyboard nav) | N/A | N/A | N/A | Documented as intentionally not applicable per `design/ux.md`'s own justification — every state above is already distinguished by a literal text substring, so no separate accessibility test is needed. |

## Test Stack

- **Unit**: Rust's built-in `#[test]` harness (`cargo test`), `assert!`/`assert_eq!` from
  std, no external assertion library — matches `src/config.rs`/`src/check.rs`'s existing
  convention (in-module `#[cfg(test)] mod tests`).
- **Integration**: `cargo test --test plugin_contract` (new `tests/plugin_contract.rs`),
  spawning real `CARGO_BIN_EXE_kibitzer`/`CARGO_BIN_EXE_kibitzer-stub-plugin` subprocesses
  against temp directories and an isolated `XDG_DATA_HOME`, following
  `tests/hook_contract.rs`'s existing `TempRepo` pattern — no mocks/test doubles for the
  filesystem or subprocess boundary; local `file://`/plain-path manifest fixtures replace
  the network boundary so no real HTTP call is ever made in CI.
- **E2E / UX**: same `tests/plugin_contract.rs` harness, scripted CLI invocations asserting
  stdout/stderr/exit-code/filesystem-state — no browser or GUI tooling applies to this
  CLI-only feature.

## Coverage Targets and How to Measure

| Stack | Coverage command | Target |
|---|---|---|
| Rust | `cargo tarpaulin --out Stdout` | ≥80% line, with `src/plugin.rs` (new module) held to ≥90% given its trust-boundary responsibilities (checksum, host allowlist, path construction) |

- All public functions in `src/plugin.rs` (`PluginName::parse`, `Registry::load`/`save`,
  `install_plugin`, `remove_plugin`, `list_plugins`, `plugin_status`,
  `registered_plugin_checks`, `missing_binary_for`, `version_meets_minimum`,
  `fetch_manifest`, `fetch_target_bytes`): happy path + error path covered per the table
  above.
- All external integrations: unit-tested with a local-file stand-in for the network branch
  (`fetch_manifest`/`fetch_target_bytes`'s local-path branch), plus at least one integration
  test (`plugin_install_register_run_end_to_end_surfaces_stub_finding`) exercising the real
  subprocess/filesystem boundary end-to-end. No real HTTPS call is made in the automated
  suite (per requirements.md's "no silent network access" constraint) — the allowlist/HTTPS
  logic itself is covered by `fetch_manifest_rejects_disallowed_https_host_before_request`,
  which asserts rejection happens before any request, not by a live network test.
- UX acceptance criteria: all 13 numbered criteria in `design/ux.md` have a corresponding
  test row above (criterion 13 is N/A, justified in-place, not silently dropped).
- Migration/schema: N/A — no schema changes (`registry.json` is a wholly new file; nothing
  pre-existing is migrated into or out of it, per plan.md's own Migration Plan section).
