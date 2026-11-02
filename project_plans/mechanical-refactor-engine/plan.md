# Plan: Mechanical Refactoring Engine

## Architecture overview

Three new modules, each mapping to a staged issue:

```
src/
  refactor/
    mod.rs          — public API: RefactorOperation, RefactorPlan, FeasibilityReport
    feasibility.rs  — read-only cycle/visibility/caller-impact checks (Stage 1, #154)
    catalog/
      mod.rs        — AvailableOperation registry
      go_move.rs    — MoveFiles and ExtractPackage operations for Go (Stage 2, #155)
    apply.rs        — atomic apply + rollback + plan persistence (Stage 2)
    verify.rs       — post-apply toolchain verification (Stage 2)
```

New MCP tools in `src/mcp.rs` (Stage 3, #156):
- `list_refactor_operations`
- `simulate_refactor`
- `plan_refactor`
- `apply_refactor`

Integration hook in `src/go_package_focus.rs`: attach `available_operations` to each `ArchFinding` when the engine can produce a plan for it.

## Stage 1: Feasibility and simulation (read-only)

### `src/refactor/mod.rs` — core types

```rust
pub struct RefactorOperation {
    pub kind: OperationKind,
    pub source_package: String,   // module-qualified import path
    pub target_package: String,
    pub files: Vec<PathBuf>,      // specific files to move, empty = all
}

pub enum OperationKind { MoveFiles, ExtractPackage, RenamePackage }

pub struct FeasibilityReport {
    pub feasible: bool,
    pub blocking: Vec<BlockingReason>,
    pub warnings: Vec<Warning>,
    pub touched_files: Vec<PathBuf>,
    pub importers_affected: Vec<String>, // package paths that import the moved pkg
}

pub enum BlockingReason {
    ImportCycle { cycle_path: Vec<String> },
    InternalVisibilityViolation { importer: String, target: String },
    CgoPackage { file: PathBuf },
    ExternalModuleImporters { count: usize },
}
```

### `src/refactor/feasibility.rs` — cycle detection

Uses `ArchModel`'s `ImportGraph`. Algorithm:
1. Build the post-move import graph in memory (no file writes) by removing the source package node and adding a target package node with the same edges.
2. Run Kahn's algorithm (topological sort) on the modified graph. If it fails (cycle), emit `BlockingReason::ImportCycle` with the cycle path extracted from the residual.
3. Check `internal/` visibility: the target package's path determines its visibility scope. Any importer currently importing the source package that would not be allowed to import the target package under the `internal/` rule is a blocking violation.
4. Count external-module importers (importers whose `from` path does not share the module prefix from `go.mod`). These cannot be rewritten by this engine; report as advisory warning unless `--allow-external` is passed.

Implementation note: Kahn's algorithm is already adjacent to `union_find.rs`'s connected-component work in `go_package_focus.rs`. Extract a `cycle_check(graph, extra_edges, removed_nodes)` utility into a new `src/refactor/graph_util.rs` module.

### MCP tool: `list_refactor_operations`

Input: `{ path: String, package: Option<String> }`
Output: `Vec<AvailableOperation>` — each entry names the operation kind, source package, and a one-line human description. Returns `[]` for packages with no supported operation.

### MCP tool: `simulate_refactor`

Input: `RefactorOperation` (JSON)
Output: `FeasibilityReport` (JSON)
No state; no side effects.

## Stage 2: Go operation catalog and apply

### `src/refactor/catalog/go_move.rs` — MoveFiles operation

`plan_move_files(model, op) → Result<RefactorPlan>`:
1. Validate: all files in `op.files` exist and belong to `op.source_package`.
2. Run `feasibility::check(model, op)` — reject if `!feasible`.
3. Compute edit list:
   a. For each moved file: `EditKind::ReplacePackageDecl { file, old_name, new_name }`.
   b. For each moved file with `_test.go` suffix using `<pkg>_test` pattern: `EditKind::ReplacePackageDecl { file, old_name: "<pkg>_test", new_name: "<newpkg>_test" }`.
   c. For each file in the module that imports `op.source_package`: scan `import_spec` nodes via tree-sitter, emit `EditKind::ReplaceImportPath { file, old_path, new_path }`.
4. Compute verification steps: `["go build ./...", "go vet ./..."]` scoped to affected packages.
5. Return `RefactorPlan { id: sha256(plan_json), edits, verification_cmds, created_at, touched_paths }`.

### `src/refactor/apply.rs` — atomic apply

`apply_plan(plan) → Result<ApplyResult>`:
1. Staleness check: for each `touched_path` in plan, compare current mtime against plan's `created_at`. Any path modified after planning → `Err(PlanStale)`.
2. Write all edits to temp paths (`<original>.kibitzer-refactor-tmp`) in the same directory.
3. If all temp writes succeed: `rename(temp, original)` for each file in order.
4. If any step fails: delete all temp files; return `Err(ApplyFailed { partial_renames: ... })`.
5. On success: delete `.kibitzer/plans/<sha>.json` (plan consumed).

### `src/refactor/verify.rs` — toolchain verification

`verify_plan(plan, repo_root) → VerifyResult`:
Runs each command in `plan.verification_cmds` in `repo_root` via `std::process::Command`. Captures stdout+stderr. Returns `VerifyResult { passed: bool, command_outputs: Vec<CommandOutput> }`.

On failure: calls `apply::rollback(plan)` (which re-applies edits in reverse using the stored original byte ranges) and returns the command output as the error body.

### Plan persistence

`save_plan(plan) → Result<PathBuf>`: serialize plan as JSON to `.kibitzer/plans/<sha>.json`.
`load_plan(sha) → Result<RefactorPlan>`: load and deserialize.
`gc_plans(repo_root)`: delete plans older than 24h (called on MCP server init).

## Stage 3: MCP integration and architecture-checker hook

### New MCP tools in `src/mcp.rs`

```rust
#[tool(description = "List mechanical refactoring operations available for a Go package")]
async fn list_refactor_operations(&self, req: ListRefactorOpsRequest) -> Result<...>

#[tool(description = "Simulate a refactoring operation — reports feasibility without writing any files")]
async fn simulate_refactor(&self, req: SimulateRefactorRequest) -> Result<...>

#[tool(description = "Plan a refactoring operation — returns an ordered edit list and verification steps")]
async fn plan_refactor(&self, req: PlanRefactorRequest) -> Result<...>

#[tool(description = "Apply a previously planned refactoring, run toolchain verification, and roll back on failure")]
async fn apply_refactor(&self, req: ApplyRefactorRequest) -> Result<...>
```

### go-package-focus integration

In `src/go_package_focus.rs`, after generating each `ArchFinding`, call `catalog::list_operations_for_package(&model, &finding.package)`. If the result is non-empty, attach it to the finding's JSON output as `"available_operations": [...]`. This is purely additive; no existing output changes if the engine returns an empty list.

## Testing strategy

### Unit tests (per module)
- `feasibility.rs`: 4 cases — acyclic move (pass), cycle-creating move (block), internal/ violation (block), cgo package (block)
- `go_move.rs`: 2 successful plan cases (single-file move; multi-file move with test files), 1 stale-plan rejection
- `apply.rs`: successful atomic apply, mid-apply failure leaves tree untouched (mock failing rename), rollback correctness
- `verify.rs`: passing build (mock), failing build triggers rollback

### Integration tests
`tests/refactor_integration_test.rs`: end-to-end MoveFiles on a small synthetic Go module under `testdata/refactor-corpus/`. The test module must have: a package with an `internal/` import constraint, one `_test.go` file, one generated file, and one build-tagged file. Covers the full plan → apply → verify → assert-`go build` path.

### Backtest (required before Stage 2 ships, per CLAUDE.md)
`scripts/refactor-backtest.sh`: dry-run `plan_refactor` (no apply) against `kubernetes/kubernetes` and `BurntSushi/ripgrep`. Assert: zero panics; all plans with `feasible: true` can be serialized/deserialized; plans marked infeasible correctly identify cycles or violations. Do not apply plans against corpus repos.

## Ordering and dependencies

```
Stage 1 (feasibility, MCP list+simulate) — no apply, no corpus risk, ships first
  ↓
Stage 2 (go_move catalog, apply, verify) — requires Stage 1's FeasibilityReport type
  ↓
Stage 3 (full MCP suite, go-package-focus hook) — requires Stage 2's RefactorPlan type
```

Stage 1 can merge independently. Stage 2 is the highest-risk stage (file writes + rollback). Stage 3 is mostly plumbing once 1 and 2 exist.

## Risks and mitigations

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|-----------|
| gopls move_package API unstable | High | Low | Direct tree-sitter rewrite; gopls is optional fast-path |
| Import-path rewrite misses a usage site | Medium | High | Backtest against large corpus; fail-safe: verify step catches it |
| Rollback leaves stale temp files | Low | Medium | GC scan on MCP server startup; temp files named with plan SHA |
| Plan staleness window too tight | Low | Low | mtime-based; configurable tolerance via `.kibitzer/inspect.json` |
| cgo packages cause panics | Medium | Medium | Detect `import "C"` in feasibility check; return `BlockingReason::CgoPackage` |
| Corpus backtest is slow (kubernetes is large) | High | Low | Scope backtest to 20 largest packages by symbol count; document scope |
