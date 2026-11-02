# Requirements: MCP Refactor Tools

**Item ID**: 501d736e-c622-4817-b283-931d947875dd  
**Project**: mcp-refactor-tools  
**Date**: 2026-11-01

## Problem Statement

Architecture checker findings (import cycles, layering violations, oversized/high-fan-in packages, refactor candidates) are dead ends for an AI agent today: they report what is wrong but provide no actionable next step. The architecture model is already reachable over MCP via existing read-only tools (`architecture_assessment`, `get_architecture_node`, `list_architecture_symbols`, `list_callers`, `list_callees`, `list_refactor_candidates`), but there is no tool that proposes, simulates, or performs a refactor based on those findings.

This feature adds:
1. **New MCP tools** (read-only and write-gated) for suggesting, simulating, planning, applying, verifying, and reverting refactors.
2. **`operation` field on `ArchFinding`** so checker findings name the available operation and the MCP call that would plan it.

## Scope

This is the **interface layer** for the mechanical-refactor umbrella (issue #153). It does not add new detection checkers — it surfaces existing findings (import cycles, LCOM-based Extract Class, misplaced symbols from refactor candidates) as actionable operations via MCP tools.

## Proposed Tools

### Read-only (always available)

| Tool | Description |
|---|---|
| `suggest_refactors(scope?)` | Ranks operations that are feasible (simulation passes) and worth considering (uses existing fan-out, churn, advisory signals). Each entry carries evidence, feasibility result, blockers, and operation name. Abstains with a reason when evidence is thin. |
| `simulate_split(package, partition)` | Returns a feasibility and impact report for a hypothetical package split. |
| `plan_refactor(operation, args)` | Returns a `RefactorPlan` (edits, importers touched, exported-surface changes, verification steps, plan_id, staleness token). Never writes. |

### Write-gated (opt-in via `.kibitzer/inspect.json`)

| Tool | Description |
|---|---|
| `apply_refactor(plan_id)` | Applies a plan atomically, runs verification, returns result and diff summary. |
| `verify_refactor(plan_id)` | Re-runs verification on an applied plan. |
| `revert_refactor(plan_id)` | Restores the pre-apply tree for an unmodified apply. |

## Architecture-Checker Integration

When a finding corresponds to an available operation, the `ArchFinding.message` names:
- The operation name
- The MCP call that would plan it
- Whether it is currently feasible

This is advisory only. No autofix from `PostToolUse` hooks. Findings remain suppressible via existing `kibitzer:ignore` directives (issue #75).

## Safety Requirements

1. Write tools are **disabled by default**; a repo opts in via `.kibitzer/inspect.json`.
2. Dry-run is the default; `apply_refactor` accepts only a `plan_id` from `plan_refactor` and rejects stale or unknown plan IDs.
3. Refuses on a dirty working tree or when another session holds a lock on an overlapping path.
4. Writes only inside the repository root; returns a diff, never reaches outside.
5. Responses are bounded in size (capped item lists, counts and truncation reported).
6. Plan state is per-session or stored on disk (shared socket server keeps per-session `KibitzerServer` — consistent with existing pattern in `mcp_socket.rs`).

## Acceptance Criteria

1. Tool schemas for all six tools are published alongside existing MCP tools, with example requests and responses in `docs/`.
2. `suggest_refactors` output on a fixture repository matches a reviewed expected list, including abstentions.
3. Write tools are inert without opt-in: `apply_refactor` is rejected with a clear error when the repo has no opt-in config.
4. A concurrent-session test: two sessions plan overlapping paths; the second `apply_refactor` is refused.
5. `ArchFinding` gains a stable `operation` field (optional, serialized, backward-compatible via `#[serde(default)]`) consumable by hooks/LSP without changing existing finding fields.

## Constraints

- No new detection checkers in this item — only the interface and simulation/planning/application layer.
- Do not change existing `CheckResult`, `ArchFinding`, or MCP tool signatures in breaking ways (additive `#[serde(default)]` fields are fine).
- The `operation` hint on findings is advisory; it must never change the severity or suppress behavior of any existing check.
- Concurrent-session locking must work for both stdio and shared-socket MCP modes.
