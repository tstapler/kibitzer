# Go Package Focus: Evidence-Based Decomposition Guidance and Build-Cost Monitoring

## Item ID
0fb32f9d-0a3a-45bf-a268-687f40c7b4ee

## Target Users

- **Primary**: Go library/service developer or AI agent running `kibitzer run .` or `kibitzer architecture check` in a CI or agent context.
- **Secondary**: team lead reviewing architecture reports.

## Problem Statement

Kibitzer should help identify when a Go package has lost focus and whether a package split would improve it—without treating package size, compilation time, or a generic "SOLID score" as proof that it must split.

The useful question is: **does a proposed boundary isolate a coherent capability, invariant, or dependency behind a usable API, at lower maintenance cost than keeping the package together?** We also want to monitor build cost and design drift over time, rather than discover these problems only during a large refactor.

This follows the architecture-analysis work in #38. The proposal is research-backed design guidance, not a calibrated detector or an implemented check.

## Goals

1. Fix the `fan_in_out` deduplication bug so package coupling metrics (Ce/Ca/instability) are based on distinct package pairs, not per-file import counts.
2. Inventory existing receiver/SOLID architecture signals and document their package-level coverage gaps.
3. Add an advisory package-focus evidence report that identifies candidate package splits with explicit uncertainty qualifiers.
4. Prototype a version-tested Go build-cost collector that monitors regressions without polluting per-edit lint hooks.
5. Document a reusable split/keep decision procedure that distinguishes objective dependency-policy enforcement from advisory heuristics.

## Non-Goals

- Universal LOC/file/time cutoffs for splitting
- A "SOLID percentage" score
- Automatic package or module splitting
- Inferred LSP/OCP compliance from static analysis
- Claiming that more packages/interfaces imply better design

## Acceptance Criteria

1. **AC-1**: Define distinct-package coupling semantics; add a repeated-import regression fixture and correct `fan_in_out` and derived instability metrics so they count distinct `(from, to)` package pairs, not per-file import occurrences.

2. **AC-2**: Inventory existing receiver/SOLID signals (LCOM, ISP fat-interface, DIP, God Class, instability) and their coverage gaps — avoid implementing duplicates or labeling an Extract Class finding as a package decomposition finding.

3. **AC-3**: Add an advisory package-focus evidence report (`go-package-focus` check name) with: named responsibility hypothesis, candidate declarations, typed internal/cross-boundary edges, exact reference sites, state/lifecycle ownership, proposed import direction, required new exports, affected callers, cycle risks, and contrary evidence. Size-only results must be marked informational, not actionable.

4. **AC-4**: The report includes explicit coverage diagnostics: which symbols are private/pruned/unresolved, which sources are production/test/generated, which build configurations are supported, and which external consumers are unknown.

5. **AC-5**: Validate on reviewed positive, negative, and insufficient-data cases including: decoder/storage separation, cohesive large parser, shared lock/state machine, generated code, embedded interfaces, generics, init registration, test-only cycles, platform/tag variants, dynamic calls, and incomplete history.

6. **AC-6**: Compare size-only, graph-only, and combined evidence leads against adjudicated cases; record acceptance/rejection/abstention reasons before claiming precision or selecting thresholds.

7. **AC-7**: Prototype a version-tested build-cost collector using `-debug-actiongraph` or `-toolexec`; compare cold/warm/edited-source workflows; document that profiling must stay outside per-edit lint hooks.

8. **AC-8**: Document the split/keep decision procedure distinguishing objective dependency-policy enforcement (gate-able in CI) from advisory architecture heuristics (not CI-gateable).

## Research Context

### Codebase State

- `fan_in_out` (`src/architecture_checks.rs:180–187`) iterates over raw `ImportEdge` slices, counting every edge, not distinct `(from, to)` pairs. This inflates Ce/Ca and derived instability for packages imported across multiple files.
- `import_graph.rs:226–240` stores source import occurrences (one edge per file-import, multiple edges possible for the same package pair). This is correct for diagnostics but must be deduplicated for package-level metrics.
- `ArchModelChecker` trait (`src/architecture_checks.rs:551–565`) has a `needs_private_symbols()` hook — a package-focus analyzer needs to override this to `true`.
- `isp_fat_interface.rs` has documented gaps: missing embedded/promoted methods, consumers via fields/containers/returns/dot imports. Silence is not compliance.
- LCOM checks operate on receiver types within a package, not on whole-package responsibility boundaries — they are a signal, not a split trigger.
- The `model_registry()` does not include a package-focus checker yet.

### Prior Art in Repo

- Architecture export (#29 / PR architecture-export plan) established the "one model, many views" pattern.
- `extract_class.rs` is in the source tree — package decomposition vs. Extract Class distinction needs to be explicit.
- `docs/check-ideas.md` establishes the "confirmed transcript occurrence" bar for new check ideas.

## Constraints

- `advisory` configuration mode only; no autofix, no decomposition CI gate.
- Must backtest against the real-world corpus (`scripts/clone-backtest-repos.sh` targets).
- Version-tested toolchain adapter for `-debug-actiongraph` (internal diagnostic; behavior may change across Go versions).
- Do not clear the user's shared Go build cache during benchmarks.
- **Core assumption**: a named responsibility hypothesis with typed cross-boundary evidence leads to developer action (split or keep decision). If findings are dismissed without action by > 80% of users, the check is generating noise. Monitor via accepted-findings.md adoption.
