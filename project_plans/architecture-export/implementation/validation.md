# Validation Plan: architecture-export

**Date**: 2026-08-24

## Happy Path Scenario

Given a repo with a valid `.claude/inspect.json` and a small mixed Go/TypeScript fixture
(two packages, a handful of exported types/functions, one cross-package import edge), when
a human runs `kibitzer architecture export --path . --out arch.json`, then `arch.json` is
written containing a pretty-printed `ArchModel` (packages keyed by path, each with its
`SymbolNode`s and a `PruningSummary`), the process exits `0`, and the same underlying
`ArchModel` — built once via `build_model` — is independently reachable through the other
three views: `kibitzer architecture diagram` renders it as a text-tree + Mermaid diagram,
the MCP `list_architecture_symbols`/`get_architecture_node` tools return scoped JSON slices
of it via `ModelCache`, and an editor's LSP `textDocument/documentSymbol` /
`workspace/symbol` requests surface it through `kibitzer lsp`. One model, four views, no
divergent reimplementation.

## Requirement → Test Mapping

| Requirement | Test File | Test Name | Type | Scenario |
|---|---|---|---|---|
| **Structured model, package + symbol level (Scope item 1)** | `src/arch_model.rs` | `build_model_groups_files_into_packages_and_attaches_import_edges` | Unit | Happy path — 2 Go files under one package dir + 1 under another, plus an `ImportGraph` edge; result has exactly the expected `packages` keys and the edge is passed through unchanged (Story 1.3.1 AC1) |
| | `src/arch_model.rs` | `build_model_skips_files_with_no_language_mapping` | Unit | Edge path — a `.go` file and a `README.md` in the same dir; only the `.go` file appears in `PackageNode.files` (Story 1.3.1 AC2) |
| | `src/arch_model.rs` | `build_model_excludes_private_symbols_by_default_and_counts_them` | Unit | Happy path — 3 exported + 2 unexported functions; `symbols.len() == 3`, `pruning.private_symbols_skipped == 2` (Story 1.3.1 AC3) |
| | `src/arch_model.rs` | `build_model_skips_generated_files_and_counts_them` | Unit | Error/edge path — file starting `// Code generated ... DO NOT EDIT.` contributes zero symbols, `generated_files_skipped == 1` (Story 1.3.1 AC5) |
| | `src/arch_model.rs` | `build_model_skips_files_with_parse_errors_without_failing` | Unit | Error path — one well-formed + one syntactically broken Go file; broken file excluded, path recorded in `pruning.files_with_parse_errors`, `build_model` still returns `Ok(_)` (Story 1.3.1 AC6) |
| | `src/symbol_extract.rs` | `extract_symbols_for_file_builds_owner_qualified_method_ids` | Unit | Happy path — Go `type T struct{}; func (t T) M() {}` → `SymbolNode{kind: Method, parent: Some("T")}`, `id == "<pkg>::T.M"` (Story 1.2.2) |
| | `src/symbol_extract.rs` | `extract_symbols_for_file_disambiguates_same_named_methods_on_different_types` | Unit | Error/edge path — two types `A`/`B` each with a `Close` method in one package; ids `"<pkg>::A.Close"` and `"<pkg>::B.Close"` don't collide (Story 1.2.2 uniqueness AC — the concrete bug this scheme exists to prevent) |
| | `src/symbol_extract.rs` | `extract_symbols_for_file_strips_generic_type_parameters_from_name` | Unit | Edge path — Go `func F[T any](x T) T {...}` → `SymbolNode.name == "F"` (Pattern Decisions: generic identity) |
| | `src/arch_model.rs` | `build_model_end_to_end_on_mixed_go_ts_fixture_produces_expected_shape` | Integration | File I/O — real fixture directory on disk (Go + TS files), full `import_graph::build` + `build_model` pipeline, asserts package/symbol counts match the fixture |
| **CLI export command (Scope item 2)** | `src/arch_export.rs` | `run_export_writes_pretty_printed_arch_model_json_with_trailing_newline` | Integration | File I/O — Happy path; matches `install.rs:35`'s exact pretty-print/newline convention (Story 2.1.1 AC1) |
| | `src/arch_export.rs` | `run_export_dry_run_prints_json_and_writes_no_file` | Integration | File I/O — Happy path (Story 2.1.1 AC2) |
| | `src/arch_export.rs` | `run_export_scope_filters_exported_packages` | Integration | File I/O — Happy path; `--scope "web/**"` leaves only `web/ui` in output (Story 2.1.1 AC3) |
| | `src/arch_export.rs` | `run_export_reports_no_supported_languages_found_and_exits_zero` | Integration | Error path — dir with only `README.md`; exact message, no file written, exit 0 (Story 2.1.1 AC5) |
| | `src/arch_export.rs` | `run_export_reports_no_packages_matched_scope_and_exits_zero` | Integration | Error path — non-empty repo, `--scope "nonexistent/**"`; exact "no packages matched scope" message, exit 0 (Story 2.1.1 AC6, closes `ux.md`'s flagged gap) |
| **MCP query tools (Scope item 3)** | `src/mcp.rs` | `list_architecture_symbols_returns_total_matched_and_json_symbols` | Integration | MCP protocol — Happy path; response parses as JSON with `total_matched`/`returned`/`symbols` (Story 3.1.1 AC2) |
| | `src/mcp.rs` | `list_architecture_symbols_returns_empty_array_for_zero_matches_not_error` | Integration | MCP protocol — Edge path; `package: "does/not/exist"` → `total_matched: 0, symbols: []`, no MCP error (Story 3.1.1 AC4) |
| | `src/mcp.rs` | `list_architecture_symbols_paginates_full_set_via_next_cursor` | Integration | MCP protocol — 5 symbols, `limit: 2`, 3 successive calls exhaust the set exactly once with stable order (Story 3.1.1 AC3) |
| | `src/mcp.rs` | `get_architecture_node_resolves_package_before_symbol_id` | Integration | MCP protocol — Happy path; package-path `node` → `kind: "package"` (Story 3.1.2 AC1) |
| | `src/mcp.rs` | `get_architecture_node_returns_not_found_echoing_queried_node` | Integration | Error path — `node: "does/not/exist"` → `{"kind":"not_found","node":"does/not/exist"}` (Story 3.1.2 AC2) |
| | `src/mcp.rs` | `get_architecture_node_resolves_owner_qualified_method_not_colliding_sibling_type` | Integration | Edge path — types `A`/`B` both with `Close`; `node: "<pkg>::A.Close"` resolves to `A`'s method, not `B`'s (Story 3.1.2 AC3) |
| | `src/mcp.rs` | `get_info_instructions_name_both_new_tools_and_json` | Unit | Happy path — `instructions` contains both tool names + substring `"JSON"` (Story 3.1.3) |
| | `src/arch_model.rs` | `model_cache_get_or_build_invokes_build_closure_once_for_repeat_call` | Unit | Happy path — call-counting closure; second call with unchanged stamps doesn't rebuild (Story 1.4.1 AC2) |
| | `src/arch_model.rs` | `model_cache_rebuilds_when_key_include_private_flips` | Unit | Edge path — `key` mismatch on `include_private` replaces the single cache slot, old model dropped (Story 1.4.1 AC4) |
| **LSP integration (Scope item 4)** | `src/lsp.rs` | `initialize_advertises_document_and_workspace_symbol_capabilities` | Integration | LSP protocol — Happy path; both capability fields `Some(OneOf::Left(true))` (Story 4.1.1) |
| | `src/lsp.rs` | `document_symbol_nests_method_under_parent_type` | Integration | LSP protocol — Happy path; one file, `Nested` tree with `T` → child `M` (Story 4.2.1 AC1) |
| | `src/lsp.rs` | `document_symbol_returns_none_for_unmapped_language` | Integration | Error path — `.md` file URI → `Ok(None)`, no panic (Story 4.2.1 AC3) |
| | `src/lsp.rs` | `document_symbol_still_returns_partial_symbols_when_parse_tree_has_errors` | Integration | Edge path — file with a syntax error still returns whatever extracts cleanly, diverging deliberately from `build_model`'s skip-whole-file policy (Story 4.2.1 AC5) |
| | `src/lsp.rs` | `initialized_spawns_background_build_and_returns_before_it_completes` | Integration | LSP protocol — Happy path; handler returns before `IndexState` leaves `Building` (Story 4.3.0 AC1) |
| | `src/lsp.rs` | `did_save_rebuild_serves_stale_snapshot_until_swap_completes` | Integration | Edge path — `symbol` call during an in-flight `did_save` rebuild still returns the pre-rebuild `Ready` snapshot (Story 4.3.0 AC2) |
| | `src/lsp.rs` | `symbol_returns_synthetic_still_indexing_entry_while_building` | Integration | Error/edge path — `IndexState::Building` → one synthetic entry, `query` ignored (Story 4.3.1 AC1) |
| | `src/lsp.rs` | `symbol_filters_query_substring_against_pruned_names` | Integration | Happy path — `Reader`/`Writer`/`Closer` indexed, query `"Re"` → only `Reader` (Story 4.3.1 AC2) |
| | `src/lsp.rs` | `symbol_returns_none_when_index_state_failed` | Integration | Error path — `IndexState::Failed(_)` → `Ok(None)`, not an LSP error (Story 4.3.1 AC3) |
| **C4-like diagram generation (Scope item 5)** | `src/arch_diagram.rs` | `render_text_tree_lists_every_package_and_symbol_at_code_level` | Unit | Happy path — pure render function, no I/O (Story 2.2.1 AC2) |
| | `src/arch_diagram.rs` | `render_component_diagram_omits_symbol_names_at_component_level` | Unit | Happy path — `--level component` output has no symbol names (Story 2.2.1 AC3) |
| | `src/arch_diagram.rs` | `render_component_diagram_falls_back_to_text_tree_note_over_node_cap` | Unit | Error/edge path — 200-package synthetic fixture over `MAX_NODES = 150`; Mermaid section replaced by cap note, text-tree still full (Story 2.2.1 AC4) |
| | `src/arch_diagram.rs` | `diagram_cli_help_contains_not_standards_conformant_c4_substring` | Integration | Process I/O — real `--help` invocation via `Command`, matching `mcp.rs`'s test-fixture convention (Story 2.2.1 AC1) |
| | `src/arch_diagram.rs` | `diagram_cli_writes_text_tree_and_mermaid_to_out_file` | Integration | File I/O — `--out <file>` happy path (Story 2.2.1 AC5) |

**Requirements coverage**: all 5 In-Scope bullet items have at least one happy-path unit
test, one error/edge-path unit test, and one integration test that crosses a real boundary
(file I/O for the model/CLI/diagram surfaces, MCP protocol for the query tools, LSP protocol
for the editor surface) — **5/5**.

## UX Acceptance Tests

| UX Criterion | Test File | Test Name | Tool | Steps |
|---|---|---|---|---|
| 1. No dead ends on "not found" | `src/mcp.rs`, `src/arch_export.rs`, `src/arch_diagram.rs` | `get_architecture_node_returns_not_found_echoing_queried_node` / `run_export_reports_no_supported_languages_found_and_exits_zero` / `render_component_diagram_falls_back_to_text_tree_note_over_node_cap` | Scripted (`cargo test`) | Call each of the three empty/not-found paths; assert the response/message names what was searched (`node` value, `<path>`, `--scope`) |
| 2. Empty is not silent, but not an error | `src/mcp.rs` | `list_architecture_symbols_returns_empty_array_for_zero_matches_not_error` | Scripted (`cargo test`) | Call `list_architecture_symbols` with a non-matching `package`; parse response JSON; assert `total_matched: 0, symbols: []`, no MCP error/exception raised |
| 3. `export` completes well under 5s on a realistic multi-language repo | `src/arch_export.rs` | `run_export_completes_under_5s_on_benchmark_fixture` | Scripted (`cargo test`, wall-clock assertion, Task 1.3.1f) | Run `run_export` (or `build_model`) against a realistic external multi-language fixture (kibitzer's own repo is Rust and not a meaningful benchmark for this feature's Go/TS/Tsx/JS/Python/Java/Kotlin coverage, per plan.md's Performance Target section); assert elapsed `< 5s`; also run against a synthetic ~2,000-file fixture asserting only "completes, no panic" |
| 4. Diagrams never lock a reader out of the underlying information | `src/arch_diagram.rs` | `render_component_diagram_falls_back_to_text_tree_note_over_node_cap` | Scripted (`cargo test`) | Force the 150-node cap with a synthetic fixture; assert the text-tree section still contains one line per node in the filtered model, independent of Mermaid-vs-cap-note state |
| 5. No command overclaims standards conformance | `src/arch_diagram.rs` | `diagram_cli_help_contains_not_standards_conformant_c4_substring` | Scripted (`cargo test`, spawns real binary via `std::process::Command`) | Run `kibitzer architecture diagram --help`; assert stdout contains the literal substring `"not a standards-conformant C4"` |
| 6. Naming stays inside the established MCP family (`path`, not `repo_path`/`root`/`target`) | `src/mcp.rs` | `list_architecture_symbols_request_field_is_named_path` | Scripted (`cargo test` on the request struct / schema) | Assert `ListArchitectureSymbolsRequest`/`GetArchitectureNodeRequest` have a `path: String` field (compiles + a schema/field-name assertion); grep-based CI check as a backstop |
| 7. `get_info()` disambiguates the query tools from the whole-repo tool | `src/mcp.rs` | `get_info_instructions_name_both_new_tools_and_json` | Scripted (`cargo test`) | Call `get_info()`; assert `instructions` contains `list_architecture_symbols`, `get_architecture_node`, and `"JSON"` |
| 8. Every optional field states its default inline | `src/mcp.rs` | `request_struct_fields_have_doc_comments_stating_defaults` | Scripted grep/manual review | Grep `///` doc comments above each `Option<_>`/defaulted field in `ListArchitectureSymbolsRequest`/`GetArchitectureNodeRequest`, assert each mentions its default — no automated doc-comment-content assertion exists in Rust, so this is a code-review checklist item backed by a grep sanity check, not a `cargo test` |
| 9. Pagination is resumable without state loss | `src/mcp.rs` | `list_architecture_symbols_paginates_full_set_via_next_cursor` | Scripted (`cargo test`) | 5-symbol fixture, `limit: 2`; page through 3 calls; assert the union of all pages equals the full set exactly once, in stable order |
| 10. A GUI symbol picker never shows an error for an unindexable state | `src/lsp.rs` | `document_symbol_returns_none_for_unmapped_language` / `symbol_returns_none_when_index_state_failed` | Scripted (`cargo test`) | Call `document_symbol` on a `.md` URI and `symbol` with `IndexState::Failed`; assert both return `Ok(None)`, never `Err(_)` |
| 11. Cold-cache latency is disclosed, not hidden | README / LSP integration docs | `readme_or_lsp_docs_mention_first_workspace_symbol_call_latency` | Manual review (no runtime assertion possible for documentation content) | Confirm the README or `kibitzer lsp --help`/module docs state that the first `workspace/symbol` call after opening a large repo pays a synchronous background-index cost; flag as a doc-review checklist item at Phase 6 verification, not a `cargo test` |
| 12. Exit codes carry no false signal | `src/arch_export.rs` | `run_export_exit_code_is_zero_across_empty_and_nonempty_outcomes` | Scripted (`cargo test` on the binary's exit code via `std::process::Command`) | Run `export` against a fixture with symbols, an empty-languages fixture, and a zero-scope-match fixture; assert exit code `0` in all three cases |

**UX acceptance coverage**: 12/12 criteria have a corresponding test or explicit manual-review
step. Criteria 8 and 11 are documentation/doc-comment-content checks that Rust's type system
and `cargo test` cannot assert directly — both are flagged as scripted-grep-backstop or
manual-review items rather than silently dropped, per this feature having no GUI to drive
with a browser-based tool.

## Test Stack
- **Unit**: Rust `#[cfg(test)] mod tests` inline in the module under test (e.g.
  `src/arch_model.rs`, `src/symbol_extract.rs`, `src/arch_diagram.rs`), run via `cargo test`
  — matches `src/rules.rs`/`src/install.rs`'s existing convention exactly (snake_case,
  descriptive names, no separate test files).
- **Integration**: This repo has no separate `tests/` directory — "integration" here means
  the same inline `#[cfg(test)] mod tests` pattern, but exercising a real boundary: real
  files written to a tempdir (mirroring `src/mcp.rs`'s existing `tmp_dir`/`write_fixture`
  helpers), a real `KibitzerServer`/`Backend` instance for MCP/LSP protocol round-trips, or
  a real compiled-binary invocation via `std::process::Command` (mirroring `src/mcp.rs`'s
  existing use of `Command` for CLI-level assertions, e.g. the `--help` substring test).
- **E2E / UX**: No GUI exists for this feature (per `design/ux.md`'s framing — every surface
  is CLI stdout/exit code, MCP JSON, or LSP protocol payloads consumed by an editor's
  built-in UI kibitzer doesn't render). UX acceptance criteria are covered by the same
  scripted `cargo test`/`Command`-based assertions as the integration tier, plus two
  documentation/manual-review items (criteria 8, 11) that assert doc-comment/README content
  rather than runtime behavior. No Playwright or browser automation applies here.
- **Migration**: N/A — no schema/database in this feature (confirmed: plan.md has no
  Migration Plan section; this feature is purely additive files + in-memory cache, per its
  own Risk Control section).

## Coverage Targets and How to Measure
| Stack | Coverage command | Target |
|---|---|---|
| Rust | `cargo test` (pass/fail gate only) | All tests green. **Gap**: no coverage-percentage tooling exists in this repo today — `cargo-tarpaulin` (or `cargo llvm-cov`) is absent from `Cargo.toml`/CI, confirmed by grep; no repo precedent sets a numeric coverage target. This plan does not invent one. If line/branch coverage percentage becomes a stated goal later, `cargo-tarpaulin` is the natural fit (matches this crate's plain `cargo test` workflow with no extra toolchain), but adding it is out of scope for this feature and should be raised as a separate decision, not bundled into architecture-export's implementation. |

- All public functions: happy path + error paths covered — see Requirement → Test Mapping
  above; every `pub fn` introduced by the plan (`build_model`, `extract_symbols_for_file`,
  `ArchModel::{package,find_symbol,filtered}`, `ModelCache::get_or_build`, `run_export`, the
  two diagram render functions, the two new MCP tool handlers, `document_symbol`, `symbol`)
  has at least one happy-path and one error/edge-path test above.
- All external integrations (MCP protocol, LSP protocol, file I/O): unit-mocked (pure
  `build_model`/`extract_symbols_for_file`/render functions take pre-read `(PathBuf,
  String)` pairs or an in-memory `ArchModel`, no disk access inside the function itself) +
  at least one integration test each — file I/O via `arch_export.rs`/`arch_diagram.rs`
  tempdir tests, MCP protocol via `src/mcp.rs`'s real-`KibitzerServer` tests, LSP protocol
  via `src/lsp.rs`'s real-`Backend` tests.
- UX acceptance criteria: all 12 criteria in `design/ux.md` have a corresponding test or
  manual-review step (10 scripted, 2 documentation/manual) — see UX Acceptance Tests table.
