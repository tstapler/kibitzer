# Validation: MCP Refactor Tools

**Date**: 2026-11-01

## Plan validation against requirements

| Requirement | Plan coverage | Status |
|---|---|---|
| Tool schemas published with example requests/responses | docs/refactor-tools.md created in Phase 1-3 | ✅ covered |
| `suggest_refactors` fixture test matches expected list including abstentions | Phase 2 integration test | ✅ covered |
| Write tools inert without opt-in | Phase 3.1 `allow_write_tools` guard + integration test | ✅ covered |
| Concurrent-session test: second apply refused | Phase 3.2 path-scoped locking + integration test | ✅ covered |
| `ArchFinding.operation` field stable, backward-compatible | Phase 1.1 `#[serde(default)]` + round-trip tests | ✅ covered |

## Gaps and open questions

1. **`simulate_split` scope for non-Go languages**: The plan notes Go-only limitations for `list_refactor_candidates` (HAC clustering over `field_accesses`). `simulate_split` operates on the `ImportGraph`, which is multi-language. The plan should clarify that `simulate_split` is available for any language with an import graph, but `suggest_refactors`' Extract Class suggestions are Go-only until `field_accesses` is extended to other languages.

2. **Plan ID format**: The plan says "01J..." (suggesting ULID). Should confirm: use `uuid` crate (already in `Cargo.toml`?) or add ULID crate. Check `Cargo.toml` before implementation.

3. **On-disk plan durability**: The plan chooses in-memory per-session plan store. This means a reconnect (e.g., after network drop in socket mode) loses all plans. The item description says "plan state must be keyed per session or stored on disk." If the shared-socket server is the primary deployment, reconnect-durability matters. Recommend adding on-disk persistence as a follow-up issue rather than blocking Phase 3.

4. **`go build` verification**: `verify_refactor` running `go build ./...` requires Go to be installed. For repos where Go is not on PATH, the verification step should gracefully fall back to kibitzer's own static checks only, with a message noting the build check was skipped.

5. **Boundary: what counts as a "clean partition"**: `simulate_split` feasibility returns `feasible: false` when the partition introduces new violations. But it does not check whether the partition is syntactically valid (e.g., a type split that would leave an incomplete method set). This is noted as out-of-scope for the simulation layer; only import-graph validity is checked.

6. **LSP #11 compatibility**: The `operation` field on `ArchFinding` feeds issue #11 (LSP integration). The `lsp.rs` module must be updated to pass `operation` through to the LSP `CodeAction` response. This is a follow-up dependency, not a blocker for this item's acceptance criteria.

## Risk assessment

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Simulation false negatives (cycle missed in cloned graph) | Medium | High | Fixture tests for known cycles; use real checkers, not simplified proxy |
| Plan staleness race condition | Low | Medium | Lock acquired before dirty-tree check; HEAD SHA as token |
| Write tools enabled too broadly by global config | Low | High | Per-repo opt-in only; documented prominently |
| `mcp_call` strings in `operation` field become stale | Medium | Low | Generated from structured data; tested against live registry |
| Phase 3 complexity delays Phase 1/2 | Medium | Low | Phased delivery: Phase 1+2 are independently shippable |

## Implementation order recommendation

1. **Phase 1** (ArchFinding `operation` field + `refactor_sim.rs`) — highest leverage: unblocks LSP (#11), checker message improvements (#41), and the planning tools simultaneously. No opt-in required, low risk.
2. **Phase 2** (`suggest_refactors`, `simulate_split`, `plan_refactor`) — delivers the agent-facing value proposition for read-only use.
3. **Phase 3** (write tools) — highest complexity and risk; benefits from Phase 1+2 being stable first.

## Checklist before implementation

- [ ] Verify `uuid` or ULID crate availability in `Cargo.toml` (or add it) — needed for plan IDs.
- [ ] Confirm `.kibitzer/inspect.json` parsing in `src/config.rs` supports additive fields without breaking existing repos.
- [ ] Check `daemon_lock.rs` API for `flock`-based locking: determine whether a new `refactor_lock.rs` module is needed or whether existing machinery can be extended.
- [ ] Confirm `hotspots.rs` exposes a public API suitable for `suggest_refactors` ranking (or add accessor).
- [ ] Review existing MCP handler pattern in `mcp.rs` to confirm `async fn list_refactor_candidates` is the right template for new handlers.
