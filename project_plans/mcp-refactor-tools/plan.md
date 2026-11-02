# Plan: MCP Refactor Tools

**Date**: 2026-11-01

## Overview

Deliver MCP refactor tooling in three ordered phases so each phase is independently shippable and testable:

1. **Phase 1 — ArchFinding `operation` field + simulation layer** (read-only, no opt-in required)
2. **Phase 2 — Read-only planning tools** (`suggest_refactors`, `simulate_split`, `plan_refactor`)
3. **Phase 3 — Write tools + opt-in gate** (`apply_refactor`, `verify_refactor`, `revert_refactor`)

## Phase 1: ArchFinding enhancement + simulation primitives

### 1.1 Add `AvailableOperation` type and `operation` field to `ArchFinding`

**File**: `src/architecture_checks.rs`

```rust
/// An operation that can resolve or improve this finding.
/// Advisory only — does not affect severity or suppression.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AvailableOperation {
    /// Short identifier: "extract_class", "move_symbol", "split_package", "break_cycle"
    pub name: String,
    /// The MCP call an agent should make to plan this operation.
    /// Example: `plan_refactor({"operation":"extract_class","package":"pkg/foo","type_name":"Bar"})`
    pub mcp_call: String,
    pub feasible: bool,
    /// Why not feasible, if `!feasible`. Empty string when feasible.
    #[serde(default)]
    pub blocker: String,
}

// In ArchFinding:
#[serde(default)]
pub operation: Option<AvailableOperation>,
```

Backward-compatible: `#[serde(default)]` follows the established precedent for all additive fields in `CheckResult`, `ArchFinding`, etc.

### 1.2 Wire `operation` hints into existing checkers

**File**: `src/architecture_checks.rs` (ImportCycleChecker, LayeringChecker)

When `ImportCycleChecker` produces a finding and a clean `move_symbol` or `split_package` operation is available, populate `operation`. This requires calling into the simulation layer (1.3). Since the checker already receives `Option<&ArchModel>`, this is feasible inline.

Rule: only populate `operation` when feasibility check passes quickly (sub-millisecond); if the simulation is expensive, leave `operation` as `None` and let `suggest_refactors` do the full analysis.

### 1.3 Simulation primitives (new file: `src/refactor_sim.rs`)

Functions:
- `simulate_split(model: &ArchModel, package: &str, partition: &[(String, Vec<String>)]) -> SimulationResult`
- `SimulationResult { violations_resolved, violations_introduced, importer_changes, feasible, blocker }`

Implementation: clone the relevant `ImportGraph` slice, move symbols per partition, re-run `ImportCycleChecker` and `LayeringChecker` on the modified graph. Pure in-memory, no disk writes.

## Phase 2: Read-only MCP planning tools

### 2.1 `suggest_refactors` tool

**File**: `src/mcp.rs` (new handler on `KibitzerServer`)

Input:
```json
{"path": "/repo/root", "scope": ["pkg/foo", "pkg/bar"], "max_results": 10}
```

Output:
```json
{
  "suggestions": [
    {
      "operation": "extract_class",
      "package": "pkg/foo",
      "type_name": "Bar",
      "evidence": {"entity_placement_score": 0.73, "churn_rank": 2},
      "feasible": true,
      "blocker": "",
      "mcp_call": "plan_refactor({\"operation\":\"extract_class\",...})"
    }
  ],
  "abstained": [
    {"operation": "split_package", "package": "pkg/baz", "reason": "no churn signal and score below threshold"}
  ],
  "truncated": false
}
```

Ranking: `entity_placement_score` (from `extract_class.rs`) × churn rank (from `hotspots.rs`) × active-violation flag. Cap at `max_results` (default 10, max 50).

Abstention logic: if the only signal is a marginal threshold crossing, emit in `abstained` not `suggestions`.

### 2.2 `simulate_split` tool

**File**: `src/mcp.rs`

Input:
```json
{"path": "/repo/root", "package": "pkg/foo", "partition": [{"name": "foo_a", "symbols": ["TypeA", "TypeB"]}, {"name": "foo_b", "symbols": ["TypeC"]}]}
```

Output:
```json
{
  "feasible": true,
  "violations_resolved": 2,
  "violations_introduced": 0,
  "importers_affected": 14,
  "importer_list": ["pkg/x", "pkg/y"],
  "importer_list_truncated": false
}
```

### 2.3 `plan_refactor` tool

**File**: `src/mcp.rs`

Input:
```json
{"path": "/repo/root", "operation": "extract_class", "args": {"package": "pkg/foo", "type_name": "Bar", "groups": [["FieldA","FieldB"],["FieldC"]]}}
```

Output: `RefactorPlan` serialized as JSON:
```json
{
  "plan_id": "01J...",
  "staleness_token": "<HEAD commit SHA>",
  "operation": "extract_class",
  "edits": [{"file": "pkg/foo/bar.go", "description": "extract BarHelper"}],
  "importers_touched": ["pkg/x"],
  "exported_surface_changes": ["BarHelper added", "Bar.fieldA removed"],
  "verification_steps": ["go build ./...", "kibitzer run pkg/foo"]
}
```

Plan state stored in `KibitzerServer` as `Arc<Mutex<HashMap<PlanId, RefactorPlan>>>`. Plans expire after the session ends (in-memory per-session). Staleness token = HEAD commit SHA at plan creation time.

## Phase 3: Write tools + opt-in gate

### 3.1 Opt-in config field

**File**: `src/config.rs`

Add to `KibitzerConfig`:
```rust
#[serde(default)]
pub allow_write_tools: bool,
```

`apply_refactor`, `verify_refactor`, `revert_refactor` check this field and return `{"error": "write tools not enabled; set allow_write_tools: true in .kibitzer/inspect.json"}` when false.

### 3.2 Path-scoped locking

**File**: `src/daemon_lock.rs` or new `src/refactor_lock.rs`

Acquire advisory `flock(2)` locks on per-path lock files under `$XDG_CACHE_HOME/kibitzer/refactor-locks/` before any write. Lock files are named by a hash of the repo root + relative path. Refused when another holder has the lock (non-blocking `flock`).

### 3.3 Dirty-tree guard

Before `apply_refactor`: run `git status --porcelain` in the repo root. Refuse if any output — the user must commit or stash first.

### 3.4 `apply_refactor` tool

Input: `{"plan_id": "01J..."}`.

Steps:
1. Look up plan by `plan_id` in the session's plan store — return error if not found.
2. Compare `staleness_token` to current HEAD — return stale error if mismatch.
3. Acquire path-scoped locks.
4. Check dirty tree.
5. Apply edits: write each file per the plan's edit list; stage with `git add <paths>` (staged but not committed, so the user reviews before committing).
6. Run `verification_steps` listed in the plan.
7. Return `{"applied": true, "diff_summary": "...", "verification": {"passed": true, "output": "..."}}`.
8. Release locks.

Writes only inside the repository root (validated by `Path::starts_with(repo_root)`).

### 3.5 `verify_refactor` tool

Input: `{"plan_id": "01J..."}`. Re-runs the plan's verification steps without re-applying edits. Returns verification output.

### 3.6 `revert_refactor` tool

Input: `{"plan_id": "01J..."}`. Runs `git checkout -- <paths>` for the plan's edited files only, if they are staged (i.e., have not been committed by the user). Returns error if the files have been committed or further modified since apply.

## Documentation

New file: `docs/refactor-tools.md` — covers:
- Tool schemas with example requests/responses
- Opt-in setup
- Safety model (dry-run, lock, dirty-tree guard)
- Per-session plan lifecycle

Update `docs/getting-started.md` with a "Refactoring suggestions" section.

## Test plan

| Test | Phase | Category |
|---|---|---|
| `ArchFinding` with `operation` field round-trips through `serde_json` | 1 | unit |
| Old `ArchFinding` JSON (no `operation`) deserializes without error | 1 | unit |
| `simulate_split` resolves a known import cycle in a fixture | 1 | unit |
| `simulate_split` introduces a new violation when partition is invalid | 1 | unit |
| `suggest_refactors` fixture test: known candidates list matches expected | 2 | integration |
| `suggest_refactors` abstains on thin-evidence package | 2 | integration |
| `plan_refactor` returns a well-formed `RefactorPlan` with a valid plan_id | 2 | integration |
| `apply_refactor` without opt-in returns error | 3 | integration |
| `apply_refactor` with dirty tree returns error | 3 | integration |
| Two sessions plan overlapping paths; second `apply_refactor` is refused | 3 | integration |
| Staleness token mismatch causes `apply_refactor` to return stale error | 3 | integration |
| `revert_refactor` restores files to pre-apply state | 3 | integration |

## Adversarial review

**Risk**: Simulation correctness — `simulate_split` could claim "no violations introduced" when the modified graph actually has new cycles. Mitigation: unit-test against fixture graphs where the expected violation count is known; run the real checkers on the cloned graph, not a simplified proxy.

**Risk**: Plan staleness race — HEAD changes between `plan_refactor` and `apply_refactor`, staleness check passes, but the plan's file line numbers are now stale. Mitigation: staleness token = HEAD commit SHA; any commit invalidates the plan, not just commits touching the plan's files. This is conservative but safe.

**Risk**: Concurrent session lock race — two sessions both pass the dirty-tree check before either acquires locks. Mitigation: dirty-tree check must happen while the lock is held (check inside lock, not before acquire).

**Risk**: Write tools enabled by default via a global `~/.kibitzer/inspect.json` — user might not realize they're enabling writes in all repos. Mitigation: opt-in is per-repo (`.kibitzer/inspect.json` in the repo root), not per-user global config. Document this clearly.

**Risk**: `AvailableOperation.mcp_call` string becomes stale if the tool schema changes. Mitigation: generate `mcp_call` strings from structured data, not hardcoded strings; add a test that exercises the generated string against the live tool registry.
