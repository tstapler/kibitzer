# go-package-focus: Build vs. Buy Research

*Researched: 2026-10-23. Covers five question areas from the feature requirements.*

---

## 1. Go Analysis Tools — Package Cohesion and Package Splits

### go vet / staticcheck / golangci-lint

**What they cover:** `go vet` ships ~30 analyzers targeting correctness bugs (nil dereferences,
printf format mismatches, loop variable capture). Staticcheck extends that with SA/S1/ST
diagnostic families — all bug-oriented or style-oriented, none cohesion-oriented.
`golangci-lint` aggregates ~100 linters; no linter in the default or opt-in registry
performs LCOM computation or package-split recommendation for Go.

**Verdict: Not recommended.** None of these tools surfaces package-cohesion or split-candidate
signals. They cannot be reused for this feature; their absence confirms there is no
off-the-shelf solution to buy here.

---

### deadcode (golang.org/x/tools/cmd/deadcode)

**What it covers:** Finds functions unreachable from a program's entry point using Rapid Type
Analysis (RTA). Published December 2023; the Go team runs it after refactoring work to find
gaps in test coverage and dead public APIs.

**Fit for go-package-focus:** It answers "which functions are dead," not "which declarations
form a coherent responsibility cluster." It could be a secondary signal (an exported function
never called outside the package is a split candidate with no cross-boundary consumers), but
it is not a cohesion analyzer.

- **Pros:** Official, well-maintained, accurate type information.
- **Cons:** Entry-point analysis only (requires a `main` or test binary); cannot analyze
  library packages in isolation; does not emit package-level cohesion evidence; requires
  shelling out to a Go binary from Rust.

**Verdict: Viable as a future signal source** (unreferenced exported symbols strengthen a
split case). Not sufficient on its own; defer to after the initial advisory report lands.

---

### lcom4go (Yahoo Japan)

**What it covers:** An LCOM4 analyzer for Go structs. Treats receiver methods as class methods
and struct fields as instance variables; merges method components when one method calls another
through the receiver. Requires running as a Go tool.

- **Pros:** The only known Go-specific LCOM implementation; models receiver semantics correctly.
- **Cons:** Struct-level metric, not package-level; does not identify split candidates or
  produce evidence reports; not on pkg.go.dev (internal Yahoo tooling from 2022 blog post);
  would require vendoring or reimplementing.

**Verdict: Not recommended.** Kibitzer already has `LcomChecker` in `src/architecture_checks.rs`
(receiver-method analysis for Go structs). Building on what's already there is cheaper than
adopting an external dependency.

---

### go-coupling (github.com/richardwooding/go-coupling, v0.3.0)

**What it covers:** A Go library computing afferent coupling (Ca), efferent coupling (Ce),
instability, and cycle detection. Reads Cargo.toml for Rust crate graphs and can extend to
Go packages. Latest version v0.3.0 (July 2026).

- **Pros:** Implements Martin's metrics; open source; cycle detection included.
- **Cons:** A Go library — cannot be imported from Rust directly; metrics only, no per-declaration
  evidence or split-candidate reasoning; kibitzer already replicates Ca/Ce via `fan_in_out`
  (with a known deduplication bug to fix). The fan_in_out dedup fix (AC-1) closes the gap
  without a new dependency.

**Verdict: Not recommended** for direct use. Useful as a reference implementation when verifying
the deduplication fix.

---

### archfit (github.com/alexei-led/archfit, v0.5.1)

**What it covers:** A Go library computing modularity metrics (coupling, cohesion, component
fitness). Has a `modularity` package in its internal metrics path.

- **Pros:** Broader scope than go-coupling; open source.
- **Cons:** Internal (not a public API commitment); Go only; young project with limited corpus
  validation; same invocation friction as go-coupling from Rust.

**Verdict: Not recommended.** Limited track record and same cross-language invocation cost.

---

## 2. Build Timing Tools — Compilation Cost Collection

### -debug-actiongraph (go build -debug-actiongraph=out.json)

**What it covers:** An undocumented internal Go flag that writes a JSON action-dependency
graph before the build and again after, with per-action timing. The `actiongraph` CLI
(github.com/icio/actiongraph, github.com/dolanor-galaxy/actiongraph) reads this file and
provides `top` (slowest packages) and `tree` (dependency view) subcommands.

- **Pros:** Rich timing data per action; machine-readable JSON; actiongraph CLI exists;
  accurately reflects incremental vs. cold build differences.
- **Cons:** Deliberately undocumented and unsupported by the Go team (confirmed by actiongraph
  README); flag behavior or JSON schema can change between Go releases without notice. AC-7
  explicitly requires a version-tested adapter precisely because of this fragility.

**Verdict: Recommended for prototyping** — this is exactly what AC-7 calls for. Implement
behind a version gate: detect Go version at runtime, only emit the flag for versions where
the JSON schema is confirmed, and degrade gracefully (log a warning and skip build-cost
data) on unknown versions. Never invoke during per-edit lint hooks.

---

### -toolexec (go build -toolexec="<wrapper>")

**What it covers:** A fully supported, documented flag that wraps each compiler/linker
invocation with a command. The wrapper receives the tool name and arguments; timing can be
measured by recording wall time around each invocation.

- **Pros:** Officially documented; stable across Go releases; can time individual package
  compilations with precision; wrapper is a separate binary that kibitzer can ship.
- **Cons:** More complex to implement (requires a small Go binary to act as the wrapper);
  slightly higher overhead than -debug-actiongraph (extra process per compile action); results
  must be collected from the wrapper's output and aggregated.

**Verdict: Recommended as the stable alternative.** If -debug-actiongraph breaks after a Go
upgrade, fall back to a -toolexec timing wrapper. The two can coexist: use -debug-actiongraph
when available (richer data, no extra binary) and -toolexec as the fallback.

---

### go build -v

**What it covers:** Prints each package name as it is compiled. No timing data — only a
sequential list.

- **Pros:** Universally available, zero setup.
- **Cons:** No timing, no per-package duration; not useful for build-cost regression detection.

**Verdict: Not recommended** for AC-7. Useful only for debugging which packages are rebuilt
in a given invocation.

---

### compilebench (golang.org/x/benchmarks/cmd/compilebench)

**What it covers:** A benchmark harness for the Go compiler itself — measures how long the
compiler takes to parse and type-check specific packages when called directly. Used by the
Go team to track compiler performance regressions.

- **Pros:** Stable, well-understood benchmark protocol.
- **Cons:** Benchmarks the compiler, not a project's build graph; not designed to measure
  build time of a user's codebase; produces compiler micro-benchmark numbers, not actiongraph
  timing that maps to real build experience.

**Verdict: Not recommended.** Wrong scope for this feature.

---

### benchstat (golang.org/x/perf/cmd/benchstat)

**What it covers:** Statistical comparison of Go benchmark results; computes confidence
intervals between two sets of benchmark runs.

- **Pros:** Excellent for detecting statistically significant changes in a metric over time.
- **Cons:** Post-processing tool only; requires benchmark output from another tool; does not
  collect build timing on its own.

**Verdict: Viable as a companion** to -debug-actiongraph or -toolexec output. Use benchstat
to compute whether a build-cost change is statistically significant across multiple runs.
Not a replacement for the timing collector itself.

---

## 3. LCOM / Coupling Libraries — Rust or Go Crates

### Search results summary

No Rust crate for LCOM, Ca/Ce instability, or package-level cohesion metrics was found on
crates.io. The `pkgrank` crate (Rust, crates.io) builds a PageRank/betweenness graph over
Cargo package dependencies, but it operates on Rust crates via `cargo metadata` only and
does not compute LCOM or Martin's coupling metrics. It has no Go support.

The ROSE LCOM Tools paper (FSE 2025) covers C++ and Ada only. Research notes that LCOM
implementations across tools diverge significantly — each tool uses its own formula, producing
incomparable scores for the same codebase.

**Verdict: Build.** No suitable existing library can be adopted from Rust. LCOM for Go
structs is already implemented in `src/architecture_checks.rs` (`LcomChecker`). The new
package-focus analyzer needs package-level cohesion signals (shared state ownership,
responsibility clustering), not a new LCOM variant. These signals are best derived from the
existing `ArchModel` call-site and field-access data, not from an external library.

---

## 4. Go analysis Package vs. Tree-Sitter for Package Boundary Analysis

### golang.org/x/tools/go/analysis

**What it covers:** A framework for writing Go static analyzers that run inside the Go
toolchain. Provides fully type-checked AST, SSA form, call graphs (via `golang.org/x/tools/go/callgraph`
and `pointer`/`cha`/`rta` algorithms), cross-package resolution, and scope/type information.
Analyzers can declare facts that are passed between packages (e.g., "this interface is used
by package X").

- **Pros:** Accurate resolution of qualified types, embedded interfaces, promoted methods,
  generics, init functions, build tags, and dynamic calls — all documented gaps in isp_fat_interface.rs.
  Can cross package boundaries for consumer tracking. The definitive tool for type-aware Go analysis.
- **Cons:** Requires invoking `go/packages` loader, which shells out to `go list` — needs a
  working Go toolchain and module cache at analysis time. From Rust, must be a separate Go
  binary that kibitzer spawns; output must be serialized (JSON) and parsed by Rust. Adds a
  Go toolchain build-time dependency to kibitzer. Significantly higher engineering cost than
  extending tree-sitter analysis. Whole-program load for large repos (kubernetes/kubernetes)
  can be slow and memory-intensive.

### Tree-sitter (current kibitzer approach)

**What it covers:** Fast incremental parsing of individual files; extracts declaration names,
import paths, receiver types, method names, struct fields, and call-site patterns. All
existing kibitzer checkers use this approach.

- **Pros:** Self-contained; no external toolchain dependency; works on any Go file without
  a complete module cache; already integrated; fast enough for per-edit hooks.
- **Cons:** No type information — cannot resolve qualified types across packages, cannot
  follow embedded interface hierarchies, cannot determine which `pkg.Type` a `var x T`
  refers to across package boundaries. These gaps are documented in isp_fat_interface.rs and
  would also affect a package-focus analyzer.

### Accuracy comparison for package-focus

The package-focus check (AC-3, AC-5) requires: tracking which declarations are referenced
across package boundaries, resolving embedded interface promoted methods, analyzing generics
and build-tag variants, and following dynamic calls. Tree-sitter can approximate most of
these with import-path and receiver-name matching, but will produce false positives for
embedded interfaces and false negatives for dynamic dispatch patterns.

**Verdict for MVP:** Use tree-sitter. For an advisory-only report (AC-8 prohibits CI gating),
approximate signals with documented uncertainty qualifiers are acceptable. The accuracy gap
is not a correctness bug — it's a known limitation to disclose in findings.

**Verdict for future iteration:** If the false-positive rate from the corpus runs (AC-6)
proves unacceptable, build a small Go helper binary using `go/analysis` that emits a JSON
symbol-reference report, and integrate it as an optional enrichment path. This avoids a
mandatory Go toolchain dependency in the common case.

---

## 5. Existing Kibitzer Infrastructure Assessment

### What's already there

| Component | Location | Coverage | Gap for go-package-focus |
|---|---|---|---|
| Import graph (per-file edges) | `src/import_graph.rs:16–42` | Go, TS, JS, Java, Kotlin, Python | `fan_in_out` counts per-file occurrences, not distinct `(from, to)` pairs (AC-1 bug) |
| fan_in_out / coupling | `src/architecture_checks.rs:180–187` | All languages with import graph | Dedup bug: inflates Ce/Ca; fix is a `BTreeSet` dedup before counting |
| InstabilityChecker | `src/architecture_checks.rs` | Package instability (Ce, Ca, I) | Downstream of fan_in_out bug; correct after AC-1 fix |
| LcomChecker | `src/architecture_checks.rs` | LCOM4 for Go structs (receiver-method connectivity) | Struct-level only; does not produce package-level split candidates |
| ArchModelChecker trait | `src/architecture_checks.rs:551–565` | Provides `needs_private_symbols()` hook | Package-focus checker must override this to `true` |
| ArchModel (symbols, call sites, field accesses, type relations) | `src/arch_model.rs` | Multi-language, per-package | Has embedded-interface and promoted-method gaps (documented in isp_fat_interface.rs) |
| IspFatInterfaceChecker | `src/isp_fat_interface.rs` | Go interface consumer analysis | Documented gaps: no promoted methods, no consumers via fields/containers/dot imports |
| model_registry() | `src/architecture_checks.rs:567–578` | Runs all ArchModelCheckers | GoPackageFocusChecker not yet registered |
| extract_class.rs | `src/extract_class.rs` | Extract Class heuristics | Must not duplicate: package-focus findings must explicitly distinguish from Extract Class signals |

### Assessment

The existing infrastructure is **sufficient for an MVP advisory report without new data structures**. The work is:

1. **Fix** the `fan_in_out` dedup bug (AC-1) — a two-line change: deduplicate edges by `(from, to)` before counting. This corrects all downstream metrics.
2. **Add** `GoPackageFocusChecker` as a new `ArchModelChecker` in the existing registry. It reuses `ArchModel` (already has call sites, field accesses, import edges, symbols) without new data collection.
3. **Override** `needs_private_symbols()` to `true` — the trait hook for this already exists.
4. **Build cost collector** (AC-7) is independent of the ArchModel path; implement as a separate command/subcommand that invokes `go build -debug-actiongraph`, not as part of the per-edit hook pipeline.

No substantial new infrastructure is needed for the MVP. The ISP gaps (embedded interfaces,
promoted methods) will create accuracy ceilings on cross-boundary consumer tracking; document
these in findings rather than fixing them as a prerequisite.

---

## Summary Table

| Capability | Option | Verdict |
|---|---|---|
| Package cohesion check (go vet / staticcheck / golangci-lint) | Off-the-shelf linter | Not recommended — none exists |
| Unreferenced exports as split signal | deadcode (golang.org/x/tools) | Viable, deferred to post-MVP |
| LCOM for Go structs | lcom4go | Not recommended — kibitzer already has LcomChecker |
| Ca/Ce/instability library | go-coupling (Go) / pkgrank (Rust) | Not recommended — fix fan_in_out bug instead |
| Build timing | -debug-actiongraph + actiongraph CLI | Recommended (MVP) — version-gate required |
| Build timing fallback | -toolexec wrapper binary | Recommended (stable fallback) |
| Build timing baseline | go build -v / compilebench / benchstat | Not recommended as collectors; benchstat viable as post-processor |
| Type-accurate analysis | go/analysis framework | Viable for v2 if tree-sitter false-positive rate is unacceptable; not needed for advisory MVP |
| Tree-sitter analysis | Current kibitzer approach | Recommended for MVP — sufficient for advisory signals with documented uncertainty |
| LCOM/coupling Rust crate | pkgrank, others | Not recommended — no suitable crate exists; build from ArchModel data |
| Existing infrastructure | import_graph, arch_model, architecture_checks | Sufficient after AC-1 fan_in_out dedup fix; no new data structures needed |

---

## Decision

**Build** the `go-package-focus` checker natively in Rust on top of existing kibitzer
infrastructure. No external dependency provides the Go package cohesion evidence report
this feature requires. The two places where buying is viable (deadcode for unreferenced
exports, go/analysis for type-accurate analysis) are deferred: deadcode as a future signal
source after the MVP lands, go/analysis as an optional enrichment path if corpus runs reveal
an unacceptable false-positive rate.

For the build-cost collector, buy the `-debug-actiongraph` JSON format (use the existing
`actiongraph` tooling as a reference) with a documented version gate, and keep `-toolexec`
as the fallback for unsupported Go versions.
