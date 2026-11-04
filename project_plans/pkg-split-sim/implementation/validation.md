# Validation: pkg-split-sim

**Date**: 2026-11-01

## Phase 4 Validation Summary

This doc records the outcome of validating the plan against requirements, adversarial review, and real-world feasibility constraints.

### AC-1: Deterministic output — VALID

`SplitSimResult` uses `BTreeMap`/`BTreeSet` throughout for deterministic field ordering. `serde_json::to_string_pretty` on a `BTreeMap` produces stable key order. Fixture snapshot tests (`expected.json`) pin the exact output for seven scenarios. If any non-determinism slips in (e.g., `HashMap` in a nested type), the fixture test will catch it at CI time.

### AC-2: Fixture suite — VALID (seven scenarios specified)

All seven required fixture scenarios are defined in plan.md Story 8:
1. Legal split ✓
2. Cycle-creating ✓
3. Private shared state ✓
4. `init()` registration ✓
5. Build-tag variant ✓
6. Test-only cycle ✓
7. Generated code ✓

Each fixture has a specified structure (minimal Go tree, `go.mod`, committed `expected.json`). The adversarial review confirmed snapshot brittleness is handled via `#[serde(default)]`.

### AC-3: Reverse-dep ground-truth verification — VALID WITH FALLBACK

Story 9 performs the real split on `legal-split/` and `cycle-creating/` fixtures, then compares counts against `kibitzer architecture affected` (primary) or `go list -test -deps` (fallback if `affected` is not yet implemented). The requirements doc acknowledges this sequencing dependency. VERDICT: the AC is achievable; the fallback path is well-defined.

### AC-4: CLI and MCP availability — VALID

Stories 10 and 11 wire both surfaces. The MCP tool follows the existing `get_architecture_node` parameter pattern — no new MCP infrastructure needed. The CLI wiring adds one `ArchitectureAction` variant — same pattern as `SplitSim` predecessor `Affected`.

### AC-5: Abstain with reason — VALID

`CoverageDiagnostics` is a first-class field of `SplitSimResult` and is always populated (never omitted). The pitfalls doc enumerates the specific signals to report. The adversarial review's "underapproximation" challenge is handled by an explicit advisory label in output, not by suppressing the diagnostic.

### Cross-check: go-package-focus overlap

The `go-package-focus` check (AC-3) produces an advisory evidence report that includes `cycle risks` and `required new exports` — two fields that overlap with this simulator's output. The decision: these two are different commands with different input shapes (go-package-focus takes a package only; split-sim takes a package + explicit partition). The overlap in *output fields* is acceptable because the simulation's output is deterministic and partition-specific, while go-package-focus's output is advisory and partition-agnostic. Both should cite each other in their CLI help text.

### Risk 1: `include_private: true` cost

**Mitigation confirmed**: benchmark gate in plan.md + `--skip-private-analysis` escape hatch. No change to plan needed.

### Risk 2: `affected.rs` helper extraction

**Mitigation confirmed**: extraction is a pure mechanical refactor of the BFS inner loop; existing tests cover the outer function; a new unit test verifies equivalence. The risk is low.

### Risk 3: Test-only cycle severity

**Mitigation confirmed**: adversarial review Challenge 7 → `CycleSeverity { Production, TestOnly }` enum field on `CycleFinding`. The fixture test exercises this classification.

## Validation verdict

**PROCEED**. All acceptance criteria have a concrete implementation path. The two feasibility risks (private-symbol performance cost, affected-rs sequencing) have documented mitigations and fallback paths. The adversarial review identified no blocking design flaw — the "underapproximation of shared state" is an inherent static-analysis limitation, not a plan defect, and is handled via coverage diagnostics and advisory labeling.

## Pre-mortem

**Most likely failure mode**: the `include_private: true` model-build cost is too high on large packages, making the tool unusable for the primary consumer (agents running in CI). Mitigation: benchmark early in implementation (before Stories 4–6 are built); if cost exceeds the SLO, implement `--skip-private-analysis` before wiring the CLI.

**Second most likely**: a fixture test diverges from the expected output after a non-breaking `SplitSimResult` schema change (new optional field). Mitigation: `#[serde(default)]` on all new fields + a `--update-expected` flag in the test runner.

**Unlikely but high-impact**: the simulated reverse-dep counts diverge from the ground-truth `affected` counts on the real split, revealing a bug in the projected-graph construction. Mitigation: Story 9 is the detection step; if it fires, the projected-graph construction (Story 3) is the most likely culprit — specifically, the handling of edges that originate from files outside the target package but target a symbol that moves.
