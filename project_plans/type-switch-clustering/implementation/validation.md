# Validation: type-switch-clustering

**Date**: 2026-11-04

## Plan completeness check

| Requirement | Plan coverage | Status |
|---|---|---|
| Checker fires on 2+ methods sharing a switch subject | Story 2 (extraction) + Story 3 (clustering) | ✅ covered |
| Uses `type_edges` to pick Polymorphism vs. Subclasses | Story 3.2 step 5 (hierarchy lookup) | ✅ covered |
| Go-only for v1 | Explicit in scope + checker `language()` guard | ✅ covered |
| No second parse pass on every edit | `batch`-only granularity, Story 3.4 | ✅ covered |
| `default_checks()` advisory registration | Story 3.4 | ✅ covered |
| Backtest before landing | Story 5 | ✅ covered |
| False-positive doc | Story 5.3 | ✅ covered |
| Unit tests | Story 2.2 + Story 4 | ✅ covered |

## Dependency verification

| Dependency | Evidence | Verified |
|---|---|---|
| `go-type-switch-density` helpers available for reuse | `src/checkers/go_type_switch_density.rs` exists; `type_case_count`, `node_text` are in-file fns | ✅ |
| `ArchModel.type_edges` available | `src/arch_model.rs:266` `pub type_edges: Vec<TypeRelationEdge>` | ✅ |
| `ArchModelChecker` trait at architecture_checks.rs ~597 | Grep confirmed in `src/architecture_checks.rs` | ✅ |
| `is_generated` importable | Used in `go_type_switch_density.rs:7` from `checkers/file_size` | ✅ |
| `inventory::submit!` pattern for auto-registration | Used in `go_type_switch_density.rs:67` | ✅ |

## Adversarial review resolution

Three plan amendments from the adversarial review are incorporated:

1. **Story 5.2 — TP floor gate**: if corpus backtest yields <2 true positives at exact
   subject-string matching, strip-receiver normalization must be attempted before the check
   ships (not deferred to a follow-up).
2. **Story 3.2 step 5 — exact suffix equality**: hierarchy lookup uses exact suffix match
   on case type names (pointer prefix stripped) vs. `edge.from`/`edge.to`, not substring
   containment.
3. **Story 3.2 message — truncation**: method list truncated at 3 entries, "…and N more"
   appended for larger clusters.

## Scope boundary check

Nothing in the plan extends to Java/TS `instanceof` chains, automatic refactoring
application, or LSP/diagram integration. All out-of-scope items are explicitly listed in
requirements.md. No new MCP tools are introduced (the checker emits findings via the
existing `Finding` struct, same as every other checker). No `ArchModel` schema changes —
Option B (re-walk) chosen precisely to keep the blast radius minimal.

## Risk residual

The main unresolved risk is whether exact subject-string matching will produce enough true
positives to be useful in practice. This is empirically answerable only by running the
backtest (Story 5). The plan gates landing on this result. If the TP rate is too low after
strip-receiver normalization, the checker should be held back or made opt-in until the
normalization heuristic can be validated. This is not a reason to cancel the item — it is
the standard "confirmed real-world signal" gate kibitzer applies to all new checkers.
