# Validation: Mechanical Refactoring Engine

## Adversarial review pass

### Challenge 1: "The apply path is the highest-risk new capability kibitzer has ever shipped. What breaks if rollback fails?"

**Concern**: `apply.rs` uses `rename(temp, original)`. On success, the original is overwritten. If the process is killed *during* the rename loop (after some renames but before others), the tree is partially rewritten. Rollback requires knowing which files were already renamed, but that state lives only in memory.

**Verdict**: Real risk. The mitigation in plan.md ("delete temp files on failure") only handles the case where the *writes to temp paths* fail, not the case where *renames* partially complete. For a kill-during-rename scenario, the tree is corrupt and no automatic recovery is possible.

**Resolution**: Add a write-ahead log (WAL) file at `.kibitzer/plans/<sha>.wal` listing each (temp_path, original_path) pair. On `apply_refactor` startup, check for an existing WAL for this plan SHA — if found, complete or undo the rename loop before proceeding. On MCP server startup, `gc_plans` also scans for orphaned WAL files and rolls back any incomplete apply. This adds ~20 lines; do not skip it.

### Challenge 2: "The feasibility check simulates the post-move import graph in memory. Is that simulation accurate for packages with build tags?"

**Concern**: A file with `//go:build linux` belongs to a different effective package set than `//go:build windows`. The import graph in `ArchModel` is built without evaluating build constraints (kibitzer runs on the analysis host, not the target). A "feasible" move on the analysis host may create a cycle only on one build platform.

**Verdict**: Real risk, but bounded. The architecture model is already documented as build-tag-agnostic (it analyzes all files regardless of tag). The feasibility report should include a warning when any moved file or affected importer contains a build tag. This is already in the requirements as a "warning" (not blocking) — confirm it's emitted.

**Resolution**: In `feasibility.rs`, after computing `touched_files`, scan each for `//go:build` or `// +build` on the first few lines via a cheap string scan (not tree-sitter — too slow for this). Emit `Warning::BuildTaggedFiles { files: Vec<PathBuf> }` in the report. Document in `plan.md` that this warning means "verify on all target platforms."

### Challenge 3: "Persisting RefactorPlans under `.kibitzer/plans/` means those files accumulate. What prevents this directory from growing unboundedly in a CI environment?"

**Concern**: In CI (GitHub Actions, etc.), kibitzer runs in ephemeral environments, so plan accumulation is not a practical concern there. But in long-running developer environments, or in a shared repo where multiple developers use the MCP server concurrently, plans can accumulate.

**Verdict**: Low risk but addressable. The plan already proposes 24h GC on server startup. This is sufficient for the primary use case. The only gap is if the server is never restarted: plans could accumulate over weeks. A secondary GC trigger (e.g., on every 10th `apply_refactor` call) closes this. Document the GC policy.

**Resolution**: The 24h startup GC is sufficient. Add a note in `plan.md` that the `.kibitzer/plans/` directory is safe to `git ignore` (add it to `.gitignore` in the engine's setup step, not manually by users).

### Challenge 4: "The plan says 'import-path rewriting via tree-sitter.' But kibitzer already reads files via `std::fs::read_to_string` and modifies them with byte-offset substitution. Is tree-sitter actually required for import rewriting, or is a regex sufficient?"

**Concern**: Import declarations in Go have a well-defined syntax (`import "path"`, `import alias "path"`, grouped import blocks). A regex like `"old/path"` → `"new/path"` inside `import (...)` blocks would work for most cases.

**Verdict**: Regex is risky. It would match string literals inside comments that happen to contain the import path, and it would match import paths that are substrings of other paths (e.g., rewriting `"foo/bar"` would incorrectly match `"foo/bar/baz"`). Tree-sitter's `import_spec.path` node is already the right primitive; kibitzer already has the grammar loaded.

**Resolution**: Keep tree-sitter for import-path rewriting. The byte-offset substitution approach (locate node, replace bytes) is the right pattern — it's already used in other kibitzer rewrites. Add a unit test case with a path that is a prefix of another path to confirm no substring collision.

### Challenge 5: "The backtest runs `plan_refactor` (dry-run) against kubernetes/kubernetes. That repo is ~1.5M lines of Go. How long does this take, and does it block CI?"

**Concern**: `kubernetes/kubernetes` has 7,000+ packages. Running feasibility checks (which rebuild the import graph for each candidate) across all 7k packages is O(packages × import-graph-size) — likely 10–30 minutes.

**Verdict**: Real constraint. The backtest script must be scoped. The plan already says "scope to 20 largest packages by symbol count." Confirm that the backtest script implements this scoping and is marked as `#[ignore]` in Rust tests (requires `--include-ignored` to run) or runs only in a separate CI job (not in `cargo test`).

**Resolution**: The backtest is a shell script (`scripts/refactor-backtest.sh`), not a Rust test — it runs outside `cargo test`. The CI pipeline (`.github/workflows/`) should not call it by default; it runs only on a labeled PR ("needs-backtest") or manually. Document this in `plan.md`. For the synthetic integration test in `tests/refactor_integration_test.rs`, use a small testdata module (< 10 packages) to keep `cargo test` under 5 seconds.

## Validation summary

No blocking issues found. Five concerns resolved with concrete mitigations:

1. **Write-ahead log for rollback durability** — add WAL file before rename loop; GC on startup.
2. **Build-tag warning in feasibility report** — already in requirements; confirm implementation.
3. **Plan GC policy** — 24h on startup is sufficient; add `.kibitzer/plans/` to `.gitignore`.
4. **Tree-sitter over regex for import rewriting** — correct per plan; add prefix-collision unit test.
5. **Backtest scoping** — script only, not in `cargo test`; scoped to 20 packages; CI opt-in.

## Ready to implement?

Yes, with the WAL addition to `apply.rs` as the most important pre-implementation design decision. All other resolutions are implementation-time choices. The staged delivery order (Stage 1 first, read-only, no file writes) is the right risk ladder: Stage 1 can ship and be validated before Stage 2's write path is written.
