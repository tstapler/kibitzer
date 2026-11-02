# Research: Mechanical Refactoring Engine

## What already exists in kibitzer

### Architecture model (confirmed)
`src/arch_model.rs` holds `ArchModel`, `ImportGraph`, `SymbolNode`, `CallEdge`, `TypeRelationEdge`. The model is already shared across MCP tools, CLI export, and the LSP server. It tracks:
- Per-package `ImportEdge` sets (from/to/file/line)
- Symbol declarations including `exported` flag and `parent` for methods
- Call sites (`RawCallSite`) and type-relation sites

### Import graph cycle detection (confirmed)
`src/import_graph.rs` builds directed `ImportEdge` sets at package granularity. No existing cycle-detection utility was found, but the data structure supports Tarjan/Kahn over the existing edge set. The `union_find.rs` module provides a Union-Find for connected-component queries already used by `go_package_focus.rs`.

### Go package focus (confirmed)
`src/go_package_focus.rs` builds intra-package reference graphs and identifies disconnected components. This is the primary consumer that would be extended to attach `available_operations` hints in Stage 3.

### MCP server (confirmed)
`src/mcp.rs` uses `rmcp` with `tool_router!`. Adding new MCP tools is a matter of adding `#[tool(...)]` methods on `KibitzerServer`. The existing `list_refactor_candidates` tool (`src/mcp.rs` at the `list_refactor_candidates` tool) already returns JSON-serialized refactor candidate data.

### Existing refactor-candidate detection (confirmed)
`src/extract_class.rs`, `src/middle_man.rs`, `src/single_call_site_delegation.rs` — detection-only checks. None of them produce machine-executable edit lists.

## gopls investigation

**Key question**: does `gopls` expose a programmatic package-level move API?

gopls implements the `workspace/executeCommand` LSP extension, including `gopls.move_package` (added in gopls v0.14, 2023). Reference: https://github.com/golang/tools/blob/master/gopls/doc/commands.md. However:
- `gopls.move_package` is classified as experimental; its contract has changed between minor versions.
- It requires a running gopls daemon and an LSP connection, adding a process-management dependency.
- It does not expose a dry-run / plan mode — the move is applied directly.
- For the feasibility-simulation phase (Stage 1, read-only), gopls provides no benefit.
- For the apply phase (Stage 2), gopls *could* be used as an optional accelerator when available, with a direct AST-edit fallback when not.

**Recommendation**: Implement import-path rewriting directly via tree-sitter (consistent with kibitzer's existing parser infrastructure) for correctness and zero extra process dependency. Optionally delegate to `gopls.move_package` if gopls is detected on PATH and the operation is within gopls's supported surface — treat gopls as a fast-path, not a requirement.

## Go import-path rewriting: direct AST approach

Tree-sitter's Go grammar (`tree-sitter-go`) exposes `import_declaration` → `import_spec` → `interpreted_string_literal`. Rewriting an import path is a targeted string replacement inside a known AST node — no full parse re-emit needed. kibitzer already uses tree-sitter for all its Go analysis (`src/checker.rs`, `src/node_kind.rs`).

Steps for a MoveFiles operation:
1. Parse destination package path from the module's `go.mod` (`module` directive).
2. In each moved file: replace the `package <name>` declaration.
3. In all files within the module (not just the moved package): scan `import_spec` nodes, replace the old import path string with the new one. Use tree-sitter to locate the node; use byte-offset replacement to avoid re-parsing. This is the same approach used by `goimports` and `gofmt` for targeted rewrites.
4. In `_test.go` files using the `<pkg>_test` pattern: rewrite `package <name>_test` to `package <newname>_test`.

Edge cases requiring special handling (per CLAUDE.md's corpus requirements):
- **Build tags** (`//go:build ...`): copy tags verbatim; warn in plan if they reference the old package name (rare but possible).
- **Generated files** (`// Code generated ...` first line): flag in plan as "rewrite may be overwritten by generator"; do not block the apply.
- **`internal/` packages**: enforce visibility rule — a package outside the `internal/` subtree cannot import a package inside it. Catch this in feasibility check.
- **`cgo` packages** (`import "C"`): the C pseudo-import complicates package renaming. Flag as unsupported in Stage 1; document as a known limitation.
- **`init()` side effects**: a package that registers itself via `init()` may be broken if importers drop the import after the move. Detect `init()` in moved files; warn in plan.
- **Dot imports** (`import . "pkg"`): these pull package-level names into the importing file's namespace. Moving the package changes which names are available. Flag as advisory warning.

## Atomicity strategy

**Option A: In-memory shadow, then atomic rename(2)**
Build all new file contents in memory, write each to a temp path in the same directory (same filesystem, so rename(2) is atomic per POSIX), then rename each temp path over the original. On any failure: delete temp files only; originals are untouched.

**Option B: Git stash + branch**
Create a temporary git commit of current state, apply all edits, run `go build`. On failure, `git checkout` back. Requires a clean working tree and a git repo, but gives a natural diff for agent review.

**Option C: Temp worktree**
kibitzer already uses git worktrees (the CLAUDE.md describes their use). A refactor plan could be executed in a fresh worktree, validated there, then merged to the main tree on success.

**Recommendation**: Option A for Stage 2 (simple, no git dependency, works in any writable directory). Option C as a future enhancement for long-running or multi-step refactors where agent review of a diff is desirable.

## Plan persistence

Holding a `RefactorPlan` in-process only means the plan is lost if the MCP server restarts between `plan_refactor` and `apply_refactor` calls. Agents in multi-turn workflows (common in stapler-squad) need the plan to survive a server reconnect.

**Recommendation**: Serialize `RefactorPlan` to a content-addressed JSON file under `.kibitzer/plans/<sha>.json` (sha = hash of plan contents). `apply_refactor` takes the plan SHA, loads it from disk, re-validates staleness, then applies. Plans older than 24 hours are garbage-collected on next startup.

## Literature

- Fowler, M., *Refactoring: Improving the Design of Existing Code*, 2nd ed., 2018 — "Move Function", "Extract Class", "Move Field" chapters define the behavior-preserving contract this engine must uphold.
- Opdyke, W.F., *Refactoring Object-Oriented Frameworks* (PhD thesis, 1992) — original formalization of behavior-preserving preconditions; the feasibility check model is directly derived from Opdyke's precondition predicates.
- gopls commands documentation: https://github.com/golang/tools/blob/master/gopls/doc/commands.md — `gopls.move_package` API surface and known limitations.
- Go module system specification: https://go.dev/ref/mod — import path rules, `internal/` visibility, module root detection from `go.mod`.
- kibitzer issue #138 (go-package-focus), #52 (Feature Envy), #154 (feasibility simulation), #155 (operation catalog), #156 (MCP integration) — referenced in the item description; these define the sub-issue breakdown.
