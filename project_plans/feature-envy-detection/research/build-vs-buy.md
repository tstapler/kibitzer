# Build vs Buy: Feature Envy Detection

## Context

The Feature Envy checker must run as a Rust `ArchModelChecker` inside kibitzer's
existing pipeline — consuming an already-built `ArchModel` (packages, field_accesses,
call_edges), re-parsing Go method bodies via tree-sitter, and emitting `ArchFinding`
values. kibitzer has no JVM, no subprocess-to-analyzer pattern, and no dependency on
the Go toolchain at check time. These constraints govern what "buy" actually means.

---

## Option 1: Existing OSS Rust crate or Go linter (staticcheck / revive)

**What exists.** No Rust crate on crates.io implements feature envy or move-method
detection. The closest category of crates — `cargo-clippy` / rust-clippy's internal
lints — covers style and correctness, not OO code smells (which don't map cleanly onto
Rust's ownership model anyway). There is no published `code-smells` or
`feature-envy-detector` crate.

In the Go ecosystem, staticcheck and revive do not implement feature envy detection.
Both tools operate at the per-file/per-package syntactic level and do not build or
consume a cross-package field-access or call graph. revive includes a small set of
style rules (exported-function comment, unused-parameter, etc.); staticcheck's
extended checks (`SA*`, `S*`, `QF*`) are correctness- and simplification-oriented. The
closest staticcheck check (`SA4009`) flags unused function arguments — structurally
different from "this method touches a foreign type's data more than its own." Neither
tool ships a "move this method to another type" advisor.

**Shelling out.** Even if a matching Go tool existed, shelling out to it from a Rust
checker that runs inside a Claude Code hook or MCP server call would require the target
repo to have that linter installed, would add uncontrolled latency, and would produce
raw text output rather than the structured `ArchFinding` values kibitzer's pipeline
expects. kibitzer already avoids this pattern for all native checks.

**Verdict: Not recommended.** No matching crate or linter exists. Even if one did, the
subprocess integration cost — dependency on a tool not part of kibitzer's distribution,
per-repo install requirement, raw-text output — outweighs any benefit.

---

## Option 2: JDeodorant / RefactoringMiner (Java tools)

**What they implement.** JDeodorant (Tsantalis et al., 2009) is the canonical
implementation of the distance-based Move Method algorithm from "Identification of Move
Method Refactoring Opportunities" (IEEE TSE). RefactoringMiner (also Tsantalis et al.)
detects applied refactorings after the fact from git history. Both are Eclipse
plug-ins / standalone Java tools. JDeodorant specifically implements the Entity
Placement metric: for each method, it computes a distance score from the method to each
class in the system, recommending the method move to the class with the smallest
distance. The distance function counts distinct entities (fields + method calls) a
method references in a given class, normalized by total entities accessed.

**Integration feasibility.** Calling JDeodorant from Rust requires spawning a JVM
subprocess. This is impractical:

- kibitzer targets `brew install kibitzer` / `cargo install kibitzer` distribution. A
  JVM dependency would be a hard runtime requirement that most users don't have
  pre-installed.
- JDeodorant is an Eclipse plug-in; the standalone variant requires OSGi / Eclipse
  runtime scaffolding, not just a plain `java -jar` invocation.
- It operates on Java source code, not Go. No Go front-end exists for it.
- Output is Eclipse-specific UI state, not machine-readable JSON or anything kibitzer
  could parse into `ArchFinding` values.

RefactoringMiner has the same JVM dependency and is Git-history-driven rather than
static-analysis-driven, which is the wrong interface shape for an `ArchModelChecker`.

**Verdict: Not recommended.** JVM dependency, Eclipse runtime requirement, Java-only
front-end, and incompatible output format make integration impractical given kibitzer's
Rust/tree-sitter/no-subprocess-to-analyzer architecture.

---

## Option 3: Implement the Tsantalis & Chatzigeorgiou 2009 distance metric from the paper

**Algorithm summary.** Tsantalis 2009 defines a distance function `d(m, C)` between
method `m` and class `C` as one minus the overlap between the entity set of `m` (fields
and methods `m` accesses/calls) and the entity set of `C` (fields and methods `C`
defines or exposes). A method is flagged as a Move Method candidate when
`d(m, home_class) > d(m, candidate)` for some other class `candidate`. Concretely:

```
entity_usage(m, C) = |entities_in_m that belong to C| / |entities_in_m|
d(m, C) = 1 - entity_usage(m, C)
```

A method is a Move Method candidate when its usage of some foreign class `F` is higher
than its usage of its own class `H`.

**Feasibility with kibitzer's syntactic constraint.** kibitzer cannot do real type
inference — `collect_foreign_field_accesses` (already in `god_class.rs`) is explicit
about this: it tracks only syntactically-visible type names at declaration sites. This
means the entity sets are approximate, not exact. The question is whether the
approximation is good enough.

Evidence in favor: the GodClassChecker's ATFD computation already uses this same
approximation across the entire kubernetes/kubernetes repo and produces only 3 findings
— the triple-metric AND is tight enough that the syntactic approximation's noise is
tolerable. Feature Envy is a weaker single-metric signal, so the per-method ratio
threshold and minimum-count guard (`FEATURE_ENVY_MIN_FOREIGN`) fill the same noise-
suppression role.

**Simplified ratio vs. full entity-placement distance.** The full Tsantalis 2009
algorithm requires enumerating entity sets for every class in the system per method,
which is expensive and requires the kind of complete type resolution kibitzer
intentionally avoids. The requirements.md specifies a simpler ratio:

```
foreign_ratio = foreign_accesses / (foreign_accesses + local_accesses)
flag when foreign_ratio >= THRESHOLD and foreign_accesses >= MIN_FOREIGN
```

This is a one-sided approximation of the entity-placement distance: it checks whether
`m` accesses a foreign class more than its own class, without computing `d(m, C)` for
every `C`. It answers the same practical question (is this method in the wrong place?)
while operating entirely within the syntactic-only constraint. The literature rationale
(Tsantalis 2009) provides the conceptual grounding; the simplified ratio is the
calibration appropriate for a syntactic tool.

**Implementation cost.** The prerequisite infrastructure already exists in kibitzer:

- `collect_foreign_field_accesses` / `walk_selectors_on_typed_locals` /
  `collect_typed_locals` in `src/god_class.rs` — the same AST walker needed here,
  requiring only a `pub(crate)` visibility change to share.
- `ArchModel.field_accesses: Vec<FieldAccessEdge>` for own-receiver accesses.
- `methods_by_type` in `src/architecture_checks.rs`.
- `FileCache` pattern from `god_class.rs` for re-parsing method bodies.
- `ArchModelChecker` trait and `ArchFinding` format from `src/architecture_checks.rs`.

The incremental implementation is a new `src/feature_envy.rs` that wires these
together — the architecture is proven by GodClassChecker and the data structures are
already populated.

**Verdict: Recommended.** Building from scratch on the existing AST walker
infrastructure is both lower-cost and higher-fidelity than any external option, given
that no external option fits kibitzer's constraints. The simplified ratio is a justified
adaptation of the Tsantalis 2009 conceptual foundation for a syntactic-only tool.

---

## Option 4: Fork or adapt an existing Rust Feature Envy implementation

No Rust implementation of feature envy detection exists on crates.io or in publicly
available open-source Rust static analysis tools (confirmed by searching crates.io for
"feature envy", "move method", "code smell"). The Rust static analysis ecosystem
(rust-clippy, cargo-semver-checks, cargo-deny, cargo-audit, cargo-geiger) is entirely
correctness- and security-oriented, not OO design-smell-oriented. This option is a null
result.

**Verdict: Not recommended.** No applicable implementation exists to fork or adapt.

---

## Decision

**Build from scratch using kibitzer's existing AST walker infrastructure.**

The choice is effectively between Option 3 (build) and nothing viable to buy. The
prerequisites — `collect_foreign_field_accesses`, `ArchModel.field_accesses`,
`methods_by_type`, `FileCache` — are already in the codebase, reducing the net-new work
to wiring and threshold calibration. The simplified ratio heuristic is the right
adaptation of Tsantalis 2009 for a syntactic-only Go checker: it answers the same
practical question without requiring full type inference or a cross-class distance
matrix computation.

The full Tsantalis 2009 entity-placement distance metric is *not* recommended for v1:
it requires computing entity sets for every class in the repo per method, is expensive,
and benefits more from real type inference than kibitzer's tree-sitter walker can
provide. It is worth revisiting if kibitzer ever adds type-resolution capabilities.
