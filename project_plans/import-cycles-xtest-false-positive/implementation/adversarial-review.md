# Adversarial Review: import-cycles-xtest-false-positive

**Date**: 2026-11-10  
**Verdict**: CLEAN

## Blockers

(none)

## Concerns

(none)

## Minors

- **`is_some_and` MSRV** — plan.md flags this as a concern, but the codebase already uses
  `is_some_and` extensively (arch_export.rs:21, cache.rs:246, etc.) and the declared Rust
  edition is 2024 (≥ 1.85). No MSRV action needed; can remove the note from plan.md.

- **`arch_model` fallback is dir-relative, not `go_package_identity`** — research.md states
  that when a file is absent from `file_packages`, the arch model falls back to
  `go_package_identity` directly. This is incorrect. `package_key_for_file` in
  `arch_model.rs:334–342` falls back to `dir.strip_prefix(repo_root)` (a plain directory
  path), not to `go_package_identity`. Consequence: excluded external test files will still
  be grouped into the arch model under a directory-relative key (e.g. `widget`) rather
  than the module-qualified one (`example.com/m/widget`). This is an existing behavior for
  any file absent from `file_packages` and does not affect cycle detection, which uses only
  the import graph. No plan change needed; worth noting in the PR.

- **Coverage of multi-module monorepos** — The regression test uses a single-module layout.
  External test packages in a module nested under a polyglot repo root (the existing
  `go_import_graph_finds_nested_module_below_a_non_go_repo_root` scenario) could also
  have external test files. The fix is per-file (reads the parsed package clause) so it
  applies regardless of module nesting; no additional test strictly required, but a note
  in the PR about applicability to nested modules would help reviewers.

- **`go_package_focus.rs` dead code** — Once the upstream fix lands, the
  `test_import_edges_excluded` counter in `go_package_focus.rs` will always be 0 for
  external test edges (since those edges no longer exist in the graph). The plan correctly
  defers cleanup to a follow-up; just ensure the follow-up is filed.
