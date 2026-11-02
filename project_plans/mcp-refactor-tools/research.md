# Research: MCP Refactor Tools

**Date**: 2026-11-01

## Codebase survey

### Existing MCP tools (src/mcp.rs — 3484 lines)

Current read-only tools registered in `KibitzerServer`:
- `architecture_assessment` — whole-repo structural + complexity report (prose)
- `get_architecture_node` — one package or symbol by exact reference (JSON)
- `list_architecture_symbols` — paginated, filtered symbol slice (JSON)
- `list_callers` / `list_callees` — function-level call-graph traversal, Go/TS/JS (JSON)
- `list_subtypes` / `list_supertypes` — type-hierarchy traversal (JSON)
- `run_checks` / `list_checks` — file-level quality checks (JSON/prose)
- `report_false_positive` — feedback tool
- `list_refactor_candidates` — HAC clustering → extract-class candidates (JSON)

No `suggest_refactors`, `simulate_split`, `plan_refactor`, `apply_refactor`, `verify_refactor`, or `revert_refactor` exist today.

### ArchFinding (src/architecture_checks.rs:16)

```rust
pub struct ArchFinding {
    pub file: Option<PathBuf>,
    pub line: Option<usize>,
    pub message: String,
    #[serde(default)]
    pub severity_override: Option<Severity>,
    // No `operation` field today
}
```

Adding `operation: Option<AvailableOperation>` with `#[serde(default)]` follows the established backward-compatibility pattern (`severity_override`, `CheckResult.command`, `CheckResult.findings`, `CheckResult.plugin_missing`, `CheckResult.inline` all use this pattern).

### Plan state and session model (src/mcp_socket.rs)

Each MCP session has its own `KibitzerServer` instance (both stdio and socket modes). The socket server creates a new `KibitzerServer` per session via `LocalSessionManager`. Plan state stored on `KibitzerServer` is inherently per-session — no cross-session plan store needed for read-only use. For write tools, plan IDs must be globally addressable (another session cannot complete a plan it didn't create). Two strategies:
1. **In-memory per-session** (`Arc<Mutex<HashMap<PlanId, Plan>>>` inside `KibitzerServer`) — simple, plan survives only as long as the session. Safe.
2. **On-disk plan store** (keyed file under `$XDG_CACHE_HOME/kibitzer/plans/<id>.json`) — survives reconnects, allows the write-op prompt to come from a new session. Requires a GC sweep.

Recommendation: start with in-memory per-session (simpler, sufficient for initial AC). Migrate to disk if reconnect-durability is needed later.

### Locking (src/daemon_lock.rs)

`daemon_lock.rs` already provides file-based locking for the socket daemon (`ensure_socket_dir`, `open_lock_file`). The same `flock(2)` mechanism can guard write-refactor operations by locking a per-path marker file under the kibitzer cache dir. Each `apply_refactor` must:
1. Resolve the set of modified paths from the plan.
2. Attempt to acquire path-scoped locks for all of them.
3. Check for dirty working tree via `git status --porcelain`.
4. Apply atomically; release locks on completion or panic.

### Existing opt-in pattern (.kibitzer/inspect.json)

Write-tool opt-in can piggyback on the existing config structure. A new top-level boolean field `"allow_write_tools": true` is the minimal addition; a per-operation `"allowed_operations": ["apply_refactor"]` list is more future-proof. The `.kibitzer/inspect.json` file is already loaded as `KibitzerConfig` in `src/config.rs`.

### Simulation layer (what-if analysis)

`simulate_split(package, partition)` needs to:
1. Clone the relevant slice of the import graph.
2. Move symbols per the proposed partition.
3. Re-run the import-cycle and layering checkers on the modified graph.
4. Report: new violations introduced, violations resolved, importer count per moved symbol.

This is purely in-memory over existing `ArchModel`/`ImportGraph` structures — no disk writes.

### suggest_refactors ranking criteria

The issue calls for ranking by "feasibility + worth considering". Useful signals already in the model:
- `list_refactor_candidates` score (`entity_placement_score`) for Extract Class
- Fan-in (importer count) for cycle-breaking or package-split candidates
- `hotspots.rs` churn data (if a file is also a churn hotspot, fixing its structure is higher priority)
- `architecture_checks.rs` active violations (a package already flagged by `import-cycles` is a concrete blocker, not just advisory)

Abstention condition: if the only signal is a marginal threshold crossing with no corroborating evidence (no churn, no violation, score barely above threshold), `suggest_refactors` should emit `abstained: true` with the reason, not a recommendation.

### operation field on ArchFinding

The `operation` field should carry:
```rust
pub struct AvailableOperation {
    pub name: String,           // e.g. "extract_class", "move_symbol", "split_package"
    pub mcp_call: String,       // e.g. "plan_refactor({\"operation\":\"extract_class\",...})"
    pub feasible: bool,
    pub blocker: Option<String>, // e.g. "dirty working tree" or "no clean partition found"
}
```

This is advisory; the finding's severity and suppression behavior do not change.

## Prior art

- **JDeodorant** (Tsantalis et al.) — Ships extract-class, move-method, extract-method as IDE refactoring actions. Evidence that the mechanics (AST-diff, import-graph rewriting) are tractable at scale.
- **RefactoringMiner** — Retrospective refactoring detection from commit history. Different direction but same structural definitions; confirmed that the ArchFinding→operation pairing follows established patterns.
- **EM-Assist (arXiv:2405.20551)** — Up to 76% of raw LLM-suggested Extract Method refactorings were hallucinated before static-analysis filtering. This validates kibitzer's approach of deterministic detection feeding the proposal, with only naming left to the caller.
- **Language Server Protocol** — `textDocument/codeAction` is the LSP analog of this feature's `operation` hint on findings. The `lsp.rs` integration (issue #11) consumes structured findings; the `operation` field needs to be stable across kibitzer versions to avoid breaking LSP clients.

## Risks and open questions

1. **Atomic apply across multiple files**: Git-backed atomicity (write all files, `git add`, leave uncommitted) is safer than direct FS writes with manual rollback. The plan should specify this approach explicitly.
2. **Plan staleness**: If the model changes between `plan_refactor` and `apply_refactor` (another commit lands), the plan is stale. A hash of the relevant ArchModel snapshot or the HEAD commit SHA is a suitable staleness token.
3. **scope parameter on suggest_refactors**: Without a scope, it could propose operations for the entire repo — expensive for large codebases. Scoping to a subdirectory or package list is important.
4. **Go-only limitation of list_refactor_candidates**: HAC clustering over `ArchModel::field_accesses` is Go-only today. `suggest_refactors` must clearly document which operations are language-gated.
5. **Dry-run verification**: `verify_refactor` re-runs checks post-apply; this requires the build to compile. For Go, `go build ./...` is the standard gate. For other languages, kibitzer may not have a build command — verification may be limited to re-running its own static checks.
