# Implementation Plan: go-package-focus
**Feature**: Evidence-based Go package decomposition guidance and build-cost monitoring
**Date**: 2026-10-23
**Status**: Ready for implementation
**ADRs**: ADR-001 (advisory-only), ADR-002 (debug-actiongraph), ADR-003 (new module)

---
## Domain Glossary

| Term | Definition | Notes |
|------|-----------|-------|
| `ImportEdge` | A record representing one import statement: `{from, to, file, line}` — one per `import` line in source | Multiple `ImportEdge`s can share the same `(from, to)` when a package imports the same target from multiple files |
| `fan_in_out` | Function computing efferent coupling Ce (distinct packages a node imports) and afferent coupling Ca (distinct packages that import a node) from a slice of `ImportEdge`s | Bug: currently counts raw edges, not distinct `(from, to)` pairs |
| Efferent coupling (Ce) | Count of distinct packages that a given package imports | Higher Ce → more dependencies → less stable |
| Afferent coupling (Ca) | Count of distinct packages that import a given package | Higher Ca → more dependents → harder to change |
| Instability (I) | Robert Martin's metric: `I = Ce / (Ca + Ce)`; 0 = maximally stable, 1 = maximally unstable | Meaningless when Ca + Ce = 0; `fan_in_out` bug inflates both Ce and Ca |
| `ArchModel` | Shared, repo-scoped data structure carrying packages, import_edges, call_edges, field_accesses, type_edges, file_import_aliases, and a PruningSummary | Owned by `src/arch_model.rs`; single source of truth for all structural checkers |
| `ArchModelChecker` | Trait for whole-repo checkers that consume an `ArchModel` rather than a per-file parse result | Defined in `src/architecture_checks.rs:551-565`; registered via `model_registry()` |
| `PruningSummary` | Output record of what `build_model` excluded: private symbols skipped, pruned symbol ids, files with parse errors | Needed by coverage diagnostics in go-package-focus |
| `needs_private_symbols()` | `ArchModelChecker` trait hook; override to `true` to receive unexported symbols in the model | go-package-focus must override to `true` — intra-package clustering needs private symbols |
| `go-package-focus` | The new advisory checker that identifies candidate Go package splits with named responsibility hypothesis, coverage diagnostics, and contrary evidence | Implemented in `src/go_package_focus.rs` |
| Responsibility hypothesis | A named, falsifiable claim about what concern a candidate split would separate (e.g. "encode vs. decode path") | Must be stated explicitly in each finding; must include contrary evidence |
| Coverage diagnostic | An explicit statement of what the checker could not analyze: private/pruned symbols, generated files, test-only symbols, unresolved externals, platform variants | Required in every finding per AC-4 |
| Build-cost collector | A CLI tool (`kibitzer go build-cost <dir>`) that measures Go compilation cost using `-debug-actiongraph` | Out-of-lint-hook; version-gated on Go ≥ 1.19 |
| ActionGraph | The internal Go build representation emitted by `-debug-actiongraph`; JSON; schema is undocumented and may change | Version-gated; degrade gracefully when unavailable |
| Decision procedure | Documented steps distinguishing CI-gateable dependency-policy enforcement from advisory structural heuristics; lives in `docs/go-package-focus.md` | AC-8 deliverable |

---
## Pattern Decisions

| Component | Pattern Chosen | Source | Alternative Rejected | Reason |
|-----------|---------------|--------|---------------------|--------|
| `fan_in_out` dedup | Collect distinct `(from, to)` pairs into `HashSet` before counting | `DipConcreteCouplingChecker`'s existing `BTreeSet` dedup pattern (`src/architecture_checks.rs:681-683`) | Change `ImportEdge` to be package-level rather than per-file | Invasive: per-file, per-line detail in `ImportEdge` is used by cycle reporting and future per-file attribution; switching the granularity breaks other consumers |
| `go-package-focus` placement | New module `src/go_package_focus.rs` implementing `ArchModelChecker` | Existing precedent: `src/extract_class.rs`, `src/god_class.rs`, `src/isp_fat_interface.rs` | Inline addition to `src/architecture_checks.rs` | `architecture_checks.rs` is 730+ lines; clustering logic, coverage diagnostics, edge-case handling, and tests add 300–500 lines — a new module is the natural cohesion boundary |
| Symbol clustering approach | Use `ArchModel::call_edges` and `ArchModel::field_accesses` to measure within-package affinity; group by shared callers/callee clusters | Graph connectivity (existing `ArchModel` data, no new dependency) | `go/analysis` framework for deeper type-level analysis | Adds heavy build dependency; tree-sitter approach reuses existing infrastructure; defer `go/analysis` if tree-sitter FP rate is unacceptable post-backtest |
| Build-cost implementation | `-debug-actiongraph` with version gate (Go ≥ 1.19); temp file, not `/dev/null` | Research ADR-002 | `-toolexec` wrapper binary | `toolexec` requires a wrapper binary, adds subprocess overhead, and measures timing the wrapper must parse from its own arguments — substantially more implementation complexity for a prototype |
| Advisory severity enforcement | All findings use `Severity::Advisory`; never registered as blocking; no autofix | Non-Goals in requirements; ADR-001 | Blocking severity with `kibitzer:ignore` escape hatch | False-positive rate on real-world code (monorepo entry points, generated types, shared state) is high enough that blocking would be routine noise |
| Finding message structure | Lead with named responsibility hypothesis; coverage disclaimer inline; alternatives section | AC-3/AC-4 UX requirements | Generic size-only message | Size-only results are explicitly marked informational (AC-3); actionable findings must have a hypothesis that can be falsified |

---
## Tech Debt Disposition

| Area | Existing Issue | Disposition | Justification |
|------|----------------|--------------|----------------|
| `fan_in_out` (`src/architecture_checks.rs:180-187`) | Counts raw `ImportEdge` instances, not distinct `(from, to)` package pairs; inflates Ce, Ca, Instability for all packages imported from multiple files | **Refactor-first**: fix as the first commit of this work, sequenced before any new checker builds on the counts | All downstream consumers (`InstabilityChecker`, `DipConcreteCouplingChecker`, `CouplingChecker`) inherit inflated counts; no new checker should be built on wrong numbers |
| `DipConcreteCouplingChecker` dedup (`src/architecture_checks.rs:681-683`) | Does its own manual pair dedup instead of using the (now-correct) `fan_in_out` | Leave in place; note in a comment that both approaches are consistent post-fix | Behavior is not changed by the `fan_in_out` fix; consolidation is a separate cleanup, not in scope here |

---
## Observability Plan

- **Logs**: `go-package-focus` findings include a `coverage_diagnostic` field in the message text; every finding logs which symbols were pruned/unresolved.
- **Metrics**: None (advisory checker; no runtime metrics surface is appropriate).
- **Alerts**: The build-cost subcommand exits nonzero and prints a diagnostic when Go is below 1.19 or `-debug-actiongraph` is rejected; the caller decides whether to treat that as an error.
- **Success metric**: after wiring into `default_checks()`, ≤ 20% of go-package-focus findings on the backtest corpus are triaged as false positives in `scripts/backtest-triage.py`. This metric is re-measured against each new major Go toolchain version release.

---
## Risk Control

- **Feature flag**: None needed — `go-package-focus` is advisory-only and wired into `default_checks()` only after corpus backtest passes. The checker can be disabled per-repo via `.kibitzer/inspect.json`'s `disable` list (existing mechanism).
- **Rollback procedure**: Remove `GoPackageFocusChecker` from `model_registry()` and the `default_checks()` entry; no data migration needed.
- **Staged rollout**: (1) Land `fan_in_out` fix with corpus backtest diff. (2) Land `go-package-focus` checker behind `model_registry()` only (not yet in `default_checks()`). (3) Run corpus backtest; triage FPs. (4) Wire into `default_checks()` once FP rate is acceptable.

---
## Unresolved Questions

- [ ] What FP rate is acceptable for `go-package-focus` before wiring into `default_checks()`? — blocks Story 2.1.4 — owner: Tyler. Proposed default: wire into default_checks() when FP rate ≤ 20% on well-maintained library/service repos (kubernetes cmd/* and other entry-point-heavy packages are excluded from the FP denominator since entry-point detection downgrades them to `[informational]`).
- [x] ~~Should `kibitzer go build-cost` be gated behind a `go` subcommand group in main.rs, or a top-level `build-cost` subcommand?~~ RESOLVED: Use `kibitzer go build-cost <dir>` under a `go` subcommand group in main.rs (consistent with the language-scoped command pattern; defer top-level if the `go` group becomes broadly useful).

---
## Dependency Visualization

```
Phase 1 (fan_in_out fix)
  └── Story 1.1.1 (fix + fixture + backtest)
        └── GATE: corpus backtest diff clean
              │
              ▼
Phase 2 (go-package-focus checker)
  └── Story 2.1.1 (skeleton + registration)
        └── Story 2.1.2 (hypothesis + diagnostics)
              └── Story 2.1.3 (edge cases + fixtures)
                    └── Story 2.1.5 (mode comparison + threshold adjudication)
                          └── GATE: AC-6 adjudication complete + thresholds documented
                                └── Story 2.1.4 (corpus backtest + default_checks())
                                      └── GATE: FP rate acceptable

Phase 3 (build-cost collector)  ← independent of Phase 2
  └── Story 3.1.1 (version gate + CLI subcommand)

Phase 4 (documentation)  ← depends on Phase 2 shape being stable
  └── Story 4.1.1 (split/keep decision procedure)
```

---

## Phase 1: Correctness Foundation

### Epic 1.1: `fan_in_out` Deduplication Fix

**Goal**: Fix the `fan_in_out` correctness bug so all downstream coupling and instability
metrics count distinct `(from, to)` package pairs rather than raw import statement counts.
This is the gating prerequisite for Phase 2.

#### Story 1.1.1: Fix `fan_in_out` to count distinct package pairs

**As a** kibitzer checker author, **I want** `fan_in_out` to count each `(from, to)` pair
once regardless of how many files in `from` import `to`, **so that** Ce, Ca, and
Instability reflect true package-level coupling, not per-file inflation.

**Acceptance Criteria**:

- Calling `fan_in_out` with two `ImportEdge`s sharing the same `(from, to)` but different
  `file` values returns Ce/Ca of 1, not 2.
  - *Given* edges `[{from: "a", to: "b", file: "a/f1.go"}, {from: "a", to: "b", file: "a/f2.go"}]`,
    *When* `fan_in_out` is called,
    *Then* the result for `"a"` has fan-out `1`, not `2`.

- A regression fixture in `testdata/dogfood-architecture/` exercises the multi-file-same-import
  case and the existing architecture check tests pass.

- A corpus backtest diff (`kibitzer check backtest instability`, `kibitzer check backtest
  coupling`) against the real-world repo corpus (kubernetes, ripgrep, servo) shows no
  unexpected new findings — only suppressed inflation artifacts are removed.

**Files**:
- `src/architecture_checks.rs` (lines 180-187: `fan_in_out`)
- `testdata/dogfood-architecture/` (new multi-file fixture package)

##### Task 1.1.1a: Deduplicate `(from, to)` in `fan_in_out`
- In `fan_in_out` (`src/architecture_checks.rs:180`), collect edges into a `HashSet<(&str, &str)>` of `(edge.from.as_str(), edge.to.as_str())` pairs first, then count from the deduplicated set.
- The existing `DipConcreteCouplingChecker` at line 681 uses `BTreeSet<(&str, &str)>` for the same purpose — use `HashSet` here since insertion order does not matter for counting.
- Files: `src/architecture_checks.rs`
- Size: XS

##### Task 1.1.1b: Add multi-file-same-import regression fixture
- In `testdata/dogfood-architecture/`, create a new subdirectory (e.g. `multi_import/`) with two `.go` files that both import the same package.
- The module path follows the existing `dogfood.example/app` pattern in `testdata/dogfood-architecture/go.mod`.
- Add or extend an existing unit test in `src/architecture_checks.rs` that asserts `fan_in_out` returns Ce=1 for the multi-file case.
- Files: `testdata/dogfood-architecture/` (new subpackage), `src/architecture_checks.rs` (test)
- Size: S

##### Task 1.1.1c: Run corpus backtest diff and confirm no regressions
- Before applying the fix, run `kibitzer run <corpus-repo> --output json > before.json` on both kubernetes and ripgrep (both repos required; their high fan-out diversity covers the full range of multi-file import patterns).
- Apply the `fan_in_out` fix.
- Run `kibitzer run <corpus-repo> --output json > after.json` on the same repos.
- Diff the finding sets (`before.json` vs. `after.json`); every disappeared finding must be classified individually as "inflation artifact" and noted by finding ID in the commit message; any new finding must be investigated before landing.
- No findings may silently flip — any finding that changed its message without appearing in the diff (same finding ID, different message text) requires investigation before landing.
- If new findings appear, diagnose before landing — do not proceed to Phase 2 with unexplained new findings.
- Files: none (verification step); results in commit message
- Size: S

---

## Phase 2: `go-package-focus` Advisory Checker

### Epic 2.1: Package Focus Checker Implementation

**Goal**: Implement `GoPackageFocusChecker` in a new module that identifies candidate Go
package splits with named responsibility hypothesis, typed cross-boundary evidence, and
explicit coverage diagnostics.

#### Story 2.1.1: New `go-package-focus` checker skeleton

**As a** kibitzer checker author, **I want** a compilable `GoPackageFocusChecker` struct
implementing `ArchModelChecker` registered in `model_registry()`, **so that** the checker
can be invoked and iterated on in isolation before its output logic is complete.

**Acceptance Criteria**:

- `kibitzer architecture check --checker go-package-focus` compiles and exits without panic
  against `testdata/dogfood-architecture/`.
  - *Given* `testdata/dogfood-architecture/` as the target,
    *When* `kibitzer architecture check` runs with `--checker go-package-focus`,
    *Then* the command exits 0 with either zero findings (skeleton) or valid `ArchFinding` output — no panic, no compilation error.

- `GoPackageFocusChecker::needs_private_symbols()` returns `true`.
  - *Given* an instance of `GoPackageFocusChecker`,
    *When* `needs_private_symbols()` is called,
    *Then* it returns `true`.

**Files**:
- `src/go_package_focus.rs` (new)
- `src/main.rs` (`mod go_package_focus;`)
- `src/architecture_checks.rs` (`model_registry()`)

##### Task 2.1.1a: Create `src/go_package_focus.rs` skeleton
- Define `pub struct GoPackageFocusChecker;`.
- Implement `ArchModelChecker`: `name()` returns `"go-package-focus"`; `check()` returns `vec![]`; `needs_private_symbols()` returns `true`.
- Add a module-level doc comment explaining the scope (intra-package symbol clustering, advisory only, Go-only).
- Define the JSON output schema for go-package-focus findings: `{ package: string, hypothesis: string, components: [{symbols: string[], state_fields: string[], external_calls: string[]}], contrary_evidence: string[], coverage: {private_skipped: number, generated_excluded: boolean, test_excluded: boolean, unresolved_edges: number} }`. This schema must be documented in `docs/go-package-focus.md` under a "Programmatic output" section.
- Files: `src/go_package_focus.rs`
- Size: XS

##### Task 2.1.1b: Wire into `model_registry()` and `main.rs`
- Add `mod go_package_focus;` to `src/main.rs` alongside the existing module declarations.
- Add `Box::new(crate::go_package_focus::GoPackageFocusChecker)` to `model_registry()` in `src/architecture_checks.rs`.
- Files: `src/main.rs`, `src/architecture_checks.rs`
- Size: XS

#### Story 2.1.2: Responsibility hypothesis, candidate declarations, coverage diagnostics

**As an** agent using kibitzer, **I want** `go-package-focus` findings to lead with a named
responsibility hypothesis and include coverage diagnostics, **so that** I can evaluate
whether the split suggestion is grounded and what the checker could not see.

**Acceptance Criteria**:

- Every finding message begins with a named responsibility hypothesis in the form
  `[go-package-focus] Candidate split in package <pkg>: <hypothesis> (advisory)`.

- Every finding includes a coverage diagnostic section listing: private/pruned symbol
  counts, whether generated files were excluded, whether test-only symbols were excluded,
  unresolved external edges count.
  - *Given* a package with 3 unexported symbols skipped (pruning),
    *When* `go-package-focus` fires on that package,
    *Then* the finding message includes a coverage diagnostic line such as
    `Coverage: 3 private symbols not modeled; generated files excluded; N unresolved edges`.

- Findings include the candidate declarations (symbol names) and typed cross-boundary edges
  (import direction + edge type).

- A size-only result (package large by LOC/file count but no clustering signal) is marked
  `[informational]` not `[advisory]`.

**Files**:
- `src/go_package_focus.rs`

##### Task 2.1.2a: Implement intra-package symbol clustering
- Walk `ArchModel::call_edges` and `ArchModel::field_accesses` restricted to the target package; build an adjacency map of symbols that share callers or field access patterns.
- **Edge definition**: use ONLY resolved `call_edges` (filter `edge.resolved == true`) plus all `field_accesses`. Unresolved call_edges (where `to` is a raw text string rather than a resolved symbol ID) are EXCLUDED from clustering — they could merge unrelated symbols via collisions on external callee names.
- **Type super-node rule**: before clustering, collapse all method IDs on a type into the type's node. Iterate symbols, group by `symbol.parent`, and merge each method's Union-Find node into its parent type's node. This guards against the same overlap concern as `extract_class.rs` and ensures clustering operates at the type level, not the raw method level.
- Use Union-Find (`src/union_find.rs` — already in the codebase) to cluster connected components.
- **Quality threshold**: emit a finding only when the clustering produces >= 2 components AND each component contains >= 2 *type-level* nodes (not raw method counts). A single type with no field-access connections to any other type does not qualify as a split candidate without at least a second type in the same component.
- **Minimum package size**: `MIN_SYMBOLS_FOR_CLUSTERING = 6` applies at the type-node level after the super-node collapse — not to the raw symbol count. A package with 10 methods on one type has 1 type-node and does not meet the threshold.
- **Maximum package size cap**: `MAX_TYPE_NODES_FOR_CLUSTERING = 60` — if the package has more than 60 type-level nodes after super-node collapse, skip clustering entirely and emit a coverage diagnostic: "package too large to cluster (N type nodes > MAX_TYPE_NODES_FOR_CLUSTERING)." This follows the precedent in `extract_class.rs` and avoids quadratic Union-Find cost on very large packages that would otherwise dominate corpus backtest runtime.
- Files: `src/go_package_focus.rs`
- Size: M

##### Task 2.1.2b: Build finding message with hypothesis, evidence, and coverage diagnostic
- For each package where clustering finds two or more components: emit an `ArchFinding` with:
  - Named responsibility hypothesis derived from component symbol names (use common-prefix heuristic; note "hypothesis — verify manually" if ambiguous).
  - Candidate declarations (symbol IDs from each component).
  - Typed cross-boundary edges (call or field-access edges that cross component boundaries).
  - Import direction (which components import which from `ArchModel::import_edges`).
  - Contrary evidence section (any cross-component shared state, shared lock, cycle risk).
  - Coverage diagnostic from `model.pruning` (private symbols skipped, files with parse errors, generated files excluded).
  - State/lifecycle ownership note: flag any shared mutable state (global vars, sync primitives) between candidate components.
- Size-only packages (large by file/LOC but no clustering signal): emit a separate `ArchFinding` with `severity_override: Some(Severity::Advisory)` and `[informational]` prefix.
- Ordering: emit findings sorted by package path alphabetically. Within a package, emit the split finding before informational findings. This ensures reproducible output across runs.
- Files: `src/go_package_focus.rs`
- Size: M

#### Story 2.1.3: Edge case handling and scenario fixtures

**As a** kibitzer maintainer, **I want** `go-package-focus` to handle real-world Go
structural patterns correctly (generated code, init registration, test cycles, generics),
**so that** the checker passes corpus backtest without high false-positive rates on these
patterns.

**Acceptance Criteria**:

- A package whose only `.go` files have a standard generated-file banner is not surfaced as
  a split candidate.
  - *Given* a Go file containing `// Code generated by protoc-gen-go. DO NOT EDIT.`,
    *When* `go-package-focus` runs,
    *Then* no finding is emitted for that package.

- A `_test.go`-only import cycle is not surfaced.
  - *Given* two packages that import each other only through `_test.go` files,
    *When* `go-package-focus` runs,
    *Then* no cycle-risk finding is emitted for those packages.

- An `init()` function that registers into a global registry is excluded from clustering
  (not treated as a connection between the `init` caller and the registration target).
  - *Given* a package with an `init()` function that calls a registry,
    *When* clustering runs,
    *Then* `init` is not counted as a call edge connecting the package to the registry package.

- A cohesive large parser (single parse context shared across all symbols) produces no split
  finding.
  - *Given* `testdata/dogfood-architecture/cohesive_parser/` with a shared parse-context
    struct accessed by all public functions,
    *When* `go-package-focus` runs,
    *Then* no split finding is emitted (all symbols share field access → single cluster).

- A decoder/storage separation case (two symbol groups with independent callers, no shared
  state) produces a split finding.
  - *Given* `testdata/dogfood-architecture/mixed_concerns/` with decoder symbols and storage
    symbols having no shared field accesses,
    *When* `go-package-focus` runs,
    *Then* a split finding is emitted with two candidate groups.

**Files**:
- `src/go_package_focus.rs` (edge case guards)
- `testdata/dogfood-architecture/cohesive_parser/` (new negative fixture)
- `testdata/dogfood-architecture/mixed_concerns/` (new positive fixture)
- `testdata/dogfood-architecture/shared_lock/` (new negative fixture: shared mutex)

##### Task 2.1.3a: Add edge-case guards to checker
- Generated files: `build_model` already skips them (banner check in `src/arch_model.rs`); add an assertion in tests that no generated-file package is surfaced.
- Test-only cycles: filter out import edges originating from `_test.go` files (use `edge.file` suffix check) before cycle-risk evaluation.
- `init()` registration: exclude any call edge where the `from` symbol name is `init` from the clustering adjacency graph.
- Generics instantiation: note in the finding when the package contains generic type parameters (detect via `SymbolKind::Type` + presence of `[` in the type's raw source span); add a coverage note "generics may inflate edge counts."
- Platform/build-tag variants: note in the coverage diagnostic when `PruningSummary::files_with_parse_errors > 0` (proxy for tag variants that didn't parse under the current config).
- **Entry-point detection**: when a package has high fan-out AND its cross-package clustering is driven entirely by outgoing call edges with no shared field accesses (i.e. `field_accesses` for the package is zero AND `component count >= 2` is driven purely by call-graph connectivity), emit `[informational]` severity rather than `[advisory]` and annotate the finding with "likely wiring/entry-point package." This downgrades — but does not suppress — the finding, so it does not count against the FP rate at the corpus backtest gate. Monorepo entry-point packages (e.g. `kubernetes/cmd/*`) legitimately have high fan-out as wiring packages; without this heuristic the corpus backtest gate cannot pass.
- Files: `src/go_package_focus.rs`
- Size: M

##### Task 2.1.3b: Add positive, negative, and edge-case fixtures
- `testdata/dogfood-architecture/mixed_concerns/`: two `.go` files with independent function sets and no shared struct fields — should produce a split finding.
- `testdata/dogfood-architecture/cohesive_parser/`: one `.go` file with multiple functions all accessing a shared `ParseContext` struct — should produce no split finding.
- `testdata/dogfood-architecture/shared_lock/`: package with a `sync.Mutex` or channel shared across function groups — should produce no split finding (shared state → contrary evidence overrides).
- Each fixture includes a comment explaining the intended checker outcome for reviewers.
- Files: `testdata/dogfood-architecture/mixed_concerns/`, `testdata/dogfood-architecture/cohesive_parser/`, `testdata/dogfood-architecture/shared_lock/`
- Size: S

#### Story 2.1.5: Mode comparison, threshold adjudication, and documentation (AC-6 gate)

**As a** kibitzer checker author, **I want** to compare size-only vs. graph-only vs.
combined evidence modes against a fixed set of adjudicated corpus cases, **so that**
threshold choices are empirically grounded and the AC-6 acceptance/rejection/abstention
record is complete before the checker enters `default_checks()`.

**Acceptance Criteria**:

- The checker is run in three modes (size-only, graph-only, combined) against the fixtures
  from Story 2.1.3 (`mixed_concerns/`, `cohesive_parser/`, `shared_lock/`) plus at least
  two packages from the real-world corpus.
  - *Given* a set of packages with known ground-truth verdicts,
    *When* each mode is run independently,
    *Then* the results table records accept/reject/abstain for each (package, mode) pair.

- Each case is adjudicated: accept (finding is correct), reject (finding is a false positive),
  or abstain (insufficient information to decide), with a one-line rationale per row.

- Threshold values (`MIN_SYMBOLS_FOR_CLUSTERING`, minimum component count, minimum type-node
  count per component) are confirmed or revised based on adjudication outcomes and recorded.

- Results are documented in `docs/go-package-focus.md` under a "Threshold decisions" section
  (or a companion `docs/go-package-focus-thresholds.md`) with the full adjudication table and
  rationale for chosen thresholds.

- This story is complete before Story 2.1.4 begins — Story 2.1.4's `default_checks()` wiring
  is gated on this story's documented adjudication.

**Files**:
- `docs/go-package-focus.md` (threshold adjudication section) or `docs/go-package-focus-thresholds.md` (new)
- `src/go_package_focus.rs` (threshold constant updates if adjudication changes them)

##### Task 2.1.5a: Run three-mode comparison against fixture and corpus cases
- Run `go-package-focus` with each evidence mode (size-only: skip graph edges; graph-only: ignore file/LOC size; combined: default) against `mixed_concerns/`, `cohesive_parser/`, `shared_lock/`, and 2–3 packages from kubernetes or ripgrep.
- Record each (package, mode) result in a table: finding emitted or not, which component groups, why.
- Files: none (verification step); results captured in Task 2.1.5b artifact
- Size: M

##### Task 2.1.5b: Adjudicate cases and document thresholds
- For each row in the results table, assign a verdict (accept/reject/abstain) with a one-line rationale.
- Identify any threshold constant that should be revised based on the results; apply changes to `src/go_package_focus.rs`.
- Write the adjudication table and threshold rationale into `docs/go-package-focus.md` under a "Threshold decisions" section (or a standalone `docs/go-package-focus-thresholds.md` if the section would dominate that doc).
- Files: `docs/go-package-focus.md` or `docs/go-package-focus-thresholds.md`, `src/go_package_focus.rs` (threshold constant updates)
- Size: S

---

#### Story 2.1.4: Corpus backtest and wire into `default_checks()` (gated on Story 2.1.5)

**As a** kibitzer user, **I want** `go-package-focus` included in `default_checks()` only
after a real-world corpus backtest confirms acceptable FP rates, **so that** the checker
does not pollute day-to-day CI or agent output with noise.

**Gate**: Story 2.1.5's adjudication must be documented and thresholds confirmed before this
story begins.

**Acceptance Criteria**:

- `go-package-focus` is run against at least three corpus repos from `docs/backtest-repos.md`
  (kubernetes/kubernetes, BurntSushi/ripgrep, denoland/deno) and the FP rate is judged
  acceptable by the repo owner (see Unresolved Questions).

- The `go-package-focus` check entry appears in `default_checks()` in `src/config.rs` once
  the FP gate passes.
  - *Given* a repo with no `.kibitzer/inspect.json`,
    *When* `kibitzer run .` executes,
    *Then* the `go-package-focus` check runs as part of the default suite.

- The `default_checks()` count test in `src/config.rs` (line 1535) is updated to reflect
  the new entry.

**Files**:
- `src/config.rs` (`default_checks()`, `go_exception_checks()` helper, or a new `go_arch_checks()` helper)

##### Task 2.1.4a: Corpus backtest and FP triage
- Run `kibitzer run <repo>` with the new checker enabled against kubernetes, ripgrep, and deno.
- Use `scripts/backtest-triage.py` to track true/false-positive classification across runs.
- Document triage results in a commit message and proceed only if owner approves the FP rate.
- Files: none (verification step)
- Size: M

##### Task 2.1.4b: Wire into `default_checks()` and update count test
- Add `whole_repo_check("go-package-focus", "go-package-focus")` (or an equivalent `architecture_check(...)` helper call matching the existing `package-size` pattern) to `default_checks()` in `src/config.rs`.
- Update the exact-count assertion in the `default_checks_ids_are_unique` test at `src/config.rs:1434`.
- Files: `src/config.rs`
- Size: XS

---

## Phase 3: Build-Cost Collector

### Epic 3.1: `kibitzer go build-cost` CLI Subcommand

**Goal**: Implement a version-tested `kibitzer go build-cost <dir>` subcommand that
measures Go compilation cost via `-debug-actiongraph` without entering per-edit lint
hooks and without clearing the user's build cache.

#### Story 3.1.1: Version-gated build-cost collector

**As a** kibitzer user, **I want** `kibitzer go build-cost <dir>` to report Go
compilation action graph metrics, **so that** I can compare cold/warm/edited-source
build costs without modifying lint hooks.

**Acceptance Criteria**:

- Running `kibitzer go build-cost <dir>` on a Go ≥ 1.19 toolchain parses and outputs a
  summary of the `-debug-actiongraph` JSON (package count, total actions, cache-hit ratio).
  - *Given* a machine with Go 1.21 and a Go module at `<dir>`,
    *When* `kibitzer go build-cost <dir>` runs,
    *Then* it prints a summary including action count and cache-hit/miss counts, then exits 0.

- Running on a Go < 1.19 toolchain prints a human-readable diagnostic and exits 0 (not an
  error).
  - *Given* a machine with Go 1.18,
    *When* `kibitzer go build-cost <dir>` runs,
    *Then* it prints `go-package-focus: -debug-actiongraph requires Go >= 1.19 (found go1.18)` and exits 0.

- The subcommand never issues `go clean -cache` and never writes to the user's `GOCACHE`.
  - *Given* any invocation of `kibitzer go build-cost`,
    *When* the command runs,
    *Then* no `go clean` subprocess is spawned and `GOCACHE` is not modified.

- The action graph is written to a temp file (not `/dev/null`) so the subcommand works on
  Windows.

**Files**:
- `src/go_build_cost.rs` (new)
- `src/main.rs` (`Go` subcommand group, `BuildCost` subcommand)

##### Task 3.1.1a: Create `src/go_build_cost.rs` with version gate
- Implement `fn go_version() -> Result<(u32, u32)>` that runs `go version` and parses the `go1.X.Y` component.
- Implement `fn run_build_cost(dir: &Path) -> Result<BuildCostReport>` that:
  1. Calls `go_version()`; returns `Err` with human-readable diagnostic if below 1.19.
  2. Creates a temp file via `std::env::temp_dir()`.
  3. Runs `go build -v -debug-actiongraph <tempfile> ./...` in `dir`.
  4. Reads and parses the temp file JSON into a `BuildCostReport` struct (action count, cache hits, cache misses, total packages).
  5. Deletes the temp file.
  6. Returns the report.
- `BuildCostReport` derives `Debug` and `Display`; `Display` prints the summary line.
- Files: `src/go_build_cost.rs`
- Size: M

##### Task 3.1.1b: Add `kibitzer go build-cost` subcommand to `main.rs`
- Add a `Go { action: GoAction }` variant to the top-level `Command` enum in `src/main.rs`, with a `GoAction::BuildCost { dir: PathBuf }` variant.
- Wire it to `go_build_cost::run_build_cost(&dir)`.
- Handle the `Go ≥ 1.19` check result: print the diagnostic and return `ExitCode::SUCCESS` (not a failure exit) when the version gate fails.
- Add `mod go_build_cost;` to `src/main.rs`.
- Files: `src/main.rs`
- Size: S

---

## Phase 4: Documentation

### Epic 4.1: Split/Keep Decision Procedure

**Goal**: Document the reusable procedure distinguishing CI-gateable dependency-policy
enforcement from advisory package-focus heuristics, so that users understand when to act
on findings and when to dismiss them.

#### Story 4.1.1: `docs/go-package-focus.md` decision procedure

**As a** team lead using kibitzer, **I want** a documented procedure for deciding whether
to split a Go package, **so that** I can apply kibitzer's findings consistently and
distinguish enforcement from advice.

**Acceptance Criteria**:

- `docs/go-package-focus.md` exists and covers: (a) what `go-package-focus` measures and
  does not measure; (b) the split decision checklist (positive evidence required, contrary
  evidence that blocks a split); (c) the distinction between dependency-policy enforcement
  (CI-gateable via `dependency-rules`) and structural advice (advisory only); (d) known
  limitations (generics, init registration, generated code, embedded interfaces, dynamic
  calls, incomplete history).
  - *Given* a developer reading `docs/go-package-focus.md`,
    *When* they encounter a `go-package-focus` finding in advisory output,
    *Then* the doc gives them a concrete checklist to evaluate the finding and a clear
    statement of which signals are gateable and which are not.

- The doc cross-references ADR-001 (advisory-only) and AC-7's `-debug-actiongraph`
  limitation.

**Files**:
- `docs/go-package-focus.md` (new)

##### Task 4.1.1a: Write `docs/go-package-focus.md`
- Sections: Overview, What the checker measures, Split decision checklist, Keep-together signals (shared state/lock, single parse context, init registration), Dependency-policy enforcement vs. advisory advice, Known coverage gaps, Examples (decoder/storage positive; cohesive parser negative; shared lock negative).
- Keep each section concise — the target reader is an agent or engineer acting on a finding, not a researcher.
- Cross-reference `docs/backtesting.md` for how to backtest before changing thresholds.
- Files: `docs/go-package-focus.md`
- Size: S

---

## Effort Estimate

Effort classes are by token-weighted agent cost, not elapsed time.

| Story | Tasks | Size | Phase gate? |
|-------|-------|------|-------------|
| 1.1.1 `fan_in_out` fix + fixture + backtest | 1.1.1a, 1.1.1b, 1.1.1c | S+XS+S | GATE: backtest diff clean |
| 2.1.1 Checker skeleton + registration | 2.1.1a, 2.1.1b | XS+XS | — |
| 2.1.2 Hypothesis + diagnostics | 2.1.2a, 2.1.2b | M+M | — |
| 2.1.3 Edge cases + fixtures | 2.1.3a, 2.1.3b | M+S | — |
| 2.1.5 Mode comparison + threshold adjudication | 2.1.5a, 2.1.5b | M+S | GATE: AC-6 adjudication complete |
| 2.1.4 Corpus backtest + default_checks() | 2.1.4a, 2.1.4b | M+XS | GATE: FP rate approved (after 2.1.5) |
| 3.1.1 Build-cost collector | 3.1.1a, 3.1.1b | M+S | — |
| 4.1.1 Decision procedure doc | 4.1.1a | S | — |

Total budget: 2 M (clustering), 2 M (diagnostics/edge cases), 3 M (backtest + mode comparison), 3 S (fixtures + threshold adjudication), 2 S (plumbing), 4 XS (registration/wiring), 1 S (doc).

Highest-risk items: 2.1.2a (clustering correctness against real packages), 2.1.5a (mode comparison may reveal threshold needs rework), 2.1.4a (FP rate on corpus — result unknown until run).

---

## Appendix: Acceptance Criteria Cross-Reference

| AC | Covered by |
|----|-----------|
| AC-1 (fan_in_out dedup + fixture) | Story 1.1.1 |
| AC-2 (inventory SOLID signals + coverage gaps) | Story 2.1.2b (coverage diagnostic), Story 2.1.2a (clustering vs extract_class guard) |
| AC-3 (advisory go-package-focus check, named hypothesis, typed edges, state/lifecycle, contrary evidence) | Stories 2.1.1, 2.1.2 |
| AC-4 (coverage diagnostics: private/pruned/unresolved/source-type/build-config/consumers) | Story 2.1.2b |
| AC-5 (positive/negative/structural scenarios, edge cases) | Story 2.1.3 |
| AC-6 (size-only vs. graph-only vs. combined comparison, acceptance/rejection/abstention) | Story 2.1.5 (mode comparison + threshold adjudication), Story 2.1.2b (informational flag for size-only results) |
| AC-7 (version-tested build-cost collector, cold/warm/edited workflows) | Story 3.1.1 |
| AC-8 (split/keep decision procedure, CI-gateable vs. advisory distinction) | Story 4.1.1 |
