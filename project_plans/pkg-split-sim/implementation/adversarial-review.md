# Adversarial Review: pkg-split-sim

**Date**: 2026-11-01

## Challenge 1: The `include_private: true` cost may be prohibitive

**Claim in plan**: the simulation always runs with `PruneConfig { include_private: true }`.

**Challenge**: on a large package (e.g., the Go stdlib's `net/http` or `encoding/json`), the number of unexported symbols vastly exceeds the exported surface. Building a model with `include_private: true` may take several seconds, making the tool unusably slow for interactive use.

**Response**: the pitfalls doc flags this and mandates a benchmark before committing to always-on. The `--skip-private-analysis` flag in the risk notes is the escape hatch. The performance SLO (< 1s on a package with ≤ 200 files, ≤ 2000 symbols) should be benchmarked during implementation, not assumed. If it fails, the tool still provides value for cycle detection and blocker detection (which don't require private symbols) even when `--skip-private-analysis` is set.

## Challenge 2: The "unexported shared state" detection is an underapproximation

**Claim in plan**: detect `FieldAccessEdge`s crossing the partition boundary where the field is unexported.

**Challenge**: this misses the common case where two methods in different partition parts both hold a reference to a shared object (e.g., a `*sync.WaitGroup` passed as a parameter or stored in an interface field). The simulator will not flag this as shared state, leading to a false "no shared state blocker" conclusion.

**Response**: the pitfalls doc explicitly calls this out as an underapproximation. The coverage diagnostics section must include: "Shared-state detection is syntactic and may miss sharing through unexported types, interface fields, or parameter passing." This is not fixable without flow analysis (out of scope). The checker should be labeled "advisory — may miss indirect sharing" in both CLI output and MCP tool description.

## Challenge 3: Fixture snapshot tests are brittle over time

**Claim in plan**: fixture tests assert `SplitSimResult` against a committed `expected.json`.

**Challenge**: any change to `SplitSimResult`'s structure (new fields, renamed variants) will break all seven fixture tests at once, creating a maintenance burden disproportionate to the change.

**Response**: use `#[serde(default)]` on all new fields in `SplitSimResult` (and sub-structs), matching the precedent in `ArchModel` (`type_edges`, `pruning.pruned_symbol_ids` etc.). This allows `expected.json` files to omit new fields without breaking deserialization. When a structural change *does* require updating snapshots, `cargo insta` (or a simple `--update-expected` flag) can regenerate all seven at once. The maintenance cost is bounded.

## Challenge 4: `affected.rs` helper extraction touches a correctness-critical module

**Claim in plan**: extract the reverse-BFS logic from `affected.rs` into a standalone helper function.

**Challenge**: `affected.rs` is the correctness-critical module for CI test narrowing. Refactoring it risks introducing a subtle bug in the extracted function (e.g., missing a bail-out condition), which would silently cause `kibitzer architecture affected` to under-test.

**Response**: the extracted `compute_reverse_reachable` function is a pure graph-walk with no bail-out conditions — the bail-out logic lives in `compute_affected` around it, not inside the BFS. The extraction is mechanical: move the inner BFS loop into its own function, keep the same argument types and return type, confirm the existing `affected.rs` tests still pass. Specifically: a new unit test `test_reverse_reachable_matches_affected` should verify that `compute_affected(graph, changed_files, config)` and `compute_reverse_reachable(graph, changed_files.map_to_packages())` produce identical package sets on a synthetic graph.

## Challenge 5: CLI `--partition` parsing is easy to get wrong for edge cases

**Claim in plan**: parse `--partition P1=file1.go,file2.go` (multi-flag).

**Challenge**: file names containing commas or equals signs (rare but legal in some OSes), or paths with spaces, will break naive split-on-comma parsing.

**Response**: Go file names do not contain commas or equals signs in practice (the Go toolchain itself does not support them). Document this assumption in the CLI help text. For the MCP tool (which uses JSON), file paths are array elements and have no parsing ambiguity. A defensive check in the CLI parser: if a file path contains `,` or `=`, emit a helpful error ("file path contains a reserved delimiter character; use the MCP tool for complex paths").

## Challenge 6: The "legal split" ground-truth verification depends on `affected.rs` being implemented

**Claim in plan**: Story 9 verifies reverse-dep counts against `kibitzer architecture affected` on two fixtures.

**Challenge**: `affected.rs` landed in a prior project, but the item description notes this feature "builds on #36, #37" — if those issues are not fully resolved, the ground-truth verification step has no reference implementation to compare against.

**Response**: the requirements doc already notes this as a feasibility risk ("softly sequenced after `affected.rs` lands"). If `affected.rs` is not yet available, Story 9 falls back to a `go list -test -deps` comparison documented manually in `verification.md`. This is explicitly the fallback stated in the requirements doc: "the ground-truth verification step falls back to a manual `go list -test -deps` comparison documented in the fixture PR." The acceptance criterion is met either way as long as the comparison is performed and documented.

## Challenge 7: Test-only cycles are reported but the policy is unclear

**Claim in plan**: label cycle findings as production/test/mixed; report both.

**Challenge**: a test-only cycle (e.g., `package foo_test` → `package bar_test` → `package foo_test` via the split) is technically a cycle but does not block compilation (Go builds test binaries separately from production binaries). Reporting it as a blocker with the same severity as a production cycle is misleading.

**Response**: the `CycleFinding` struct should have a `severity: CycleSeverity { Production, TestOnly }` field. Production cycles are hard blockers; test-only cycles are warnings (advisory). The CLI text output labels them differently: "BLOCKER: production import cycle" vs. "WARNING: test-only import cycle". The fixture for test-only cycles verifies that the simulation does not classify it as a hard blocker.
