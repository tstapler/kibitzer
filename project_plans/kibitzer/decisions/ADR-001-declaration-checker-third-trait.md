# ADR-001: Content/Naming Rules Get a Third Checker Trait, Not a Widened `ArchitectureChecker`

**Date**: 2026-08-22
**Status**: Accepted

## Context

kibitzer's native architecture linting expands from 3 checkers (`ImportCycleChecker`,
`LayeringChecker`, `CouplingChecker`, all implementing `ArchitectureChecker` — `check(&ImportGraph,
&ArchitectureConfig) -> Vec<ArchFinding>`, `src/architecture_checks.rs:19`) to include two new rule
categories: package-content rules ("component X may only contain declaration kind Y") and
naming-convention rules ("declarations of kind K in component X must match pattern Y").

Both new categories need per-declaration AST information — `ImportGraph` carries only import edges
at directory/package granularity, never file contents. There is no way to answer "does this
package contain a function when it should only contain structs" from `ImportGraph` alone.

Three options were considered for where this capability lives:

1. **Widen `ArchitectureChecker::check()`'s signature** to also accept a new declaration graph
   (e.g. `check(&ImportGraph, &DeclarationGraph, &ArchitectureConfig)`), letting content/naming
   checkers implement the same trait.
2. **Fold declaration extraction into `import_graph.rs`**, extending `ImportGraph`'s per-language
   `build_*` functions to also collect declarations while they're already walking each file's tree.
3. **A third, parallel trait + registry** — `DeclarationChecker`, consuming a new `DeclarationGraph`,
   with its own `registry()`/`lookup()` in a new `src/declaration_checks.rs`, dispatched through the
   existing `Check.architecture_checker: Option<String>` flat-string field (one merge point resolving
   against both registries — see Domain Glossary's `AnyArchitectureChecker`).

## Decision

**Option 3: a third trait and registry.**

## Rationale

1. **Established precedent, applied a second time, not a novel pattern.** kibitzer already runs
   *two* separate checker traits/registries for two separate consumption shapes: `Checker`
   (`src/checker.rs:64`, per-file) and `ArchitectureChecker` (`src/architecture_checks.rs:19`,
   whole-repo `ImportGraph`). Adding `DeclarationChecker` for a third, genuinely different
   consumption shape (whole-repo `DeclarationGraph`) is the same architectural move made again, not
   a new one being introduced. A reviewer who already understands why kibitzer has two registries
   understands the third immediately.

2. **"Existing checkers continue to work unmodified" holds more literally.** Requirements.md's
   Scope section states `ImportCycleChecker`/`LayeringChecker`/`CouplingChecker` "continue to work
   unmodified against the extended graph and new config." Option 1 (widen the signature) would
   force every existing implementor to accept a parameter it ignores — a call-site and trait-impl
   change to all three, even if mechanical. Option 3 touches zero lines in
   `ImportCycleChecker`/`LayeringChecker`/`CouplingChecker` or their trait definition.

3. **The two graphs have incompatible resolution algorithms and granularities, so merging their
   construction (Option 2) creates a false coupling.** `import_graph.rs`'s per-language `build_*`
   functions each implement import-*path resolution* (Go: manifest-anchored string equality; JS:
   filesystem-relative candidate resolution; Python: dot-counting + heuristic package-root walk) —
   an entirely different problem from declaration *enumeration* (walk every file, classify each
   top-level node by kind). Interleaving them into one pass means every future change to either
   concern risks breaking the other, and — concretely — Go interface satisfaction for naming rules
   (`NamingChecker`'s `Implements(X)` scope, deferred per the plan's Pattern Decisions but a design
   constraint even so) needs a whole-*package* declaration index that has nothing to do with import
   resolution at all.

4. **Avoiding redundant parsing has an existing, reusable mechanism that's a better fit than
   entangling the two passes.** `checker.rs::GrammarCache` already exists precisely so multiple
   checkers sharing a `Language` share one parse. Notably, `import_graph.rs`'s `build_go`/`build_js`
   *don't* use `GrammarCache` today (each constructs its own `Parser` per file) — so the cleaner
   change is giving the new declaration pass its own `GrammarCache`-backed (or equivalently
   self-contained) walk, independent of import-graph construction, rather than coupling the two.

## Consequences

- **Positive**: `src/architecture_checks.rs` is untouched by this feature except for one new
  registry entry (`ComponentDependencyChecker`, Phase 1) — the file's existing 3 checkers and their
  tests need zero changes.
- **Positive**: `DeclarationGraph`/`Declaration`/`DeclKind` (new module `src/declarations.rs`) can
  evolve its own per-language extraction schedule (Go/JS first, Java/Kotlin/Python later — see the
  plan's Phase 2 vs. Phase 4/5 split) independently of `ImportGraph`'s own extraction schedule,
  even though in practice this plan ships them roughly in lockstep per language.
- **Negative**: One more indirection layer (`AnyArchitectureChecker` enum) is needed at every point
  that must reason about "either kind of architecture check." This turned out to be *three* points,
  not two: `config.rs::validate()`'s unknown-checker-name error, `check.rs::run_architecture_check`'s
  graph-building dispatch, and — caught by adversarial review during Phase 3 planning, not anticipated
  when this ADR was first written — `check.rs::check_native_against_git_head_repo`'s git-HEAD-snapshot
  dispatch (the plan's Story 2.2.3 fixes this: that function originally looked up only the
  `ArchitectureChecker` registry, so a blocking `content-rules`/`naming-rules` violation could never
  downgrade via the "predates your edits" baseline check the other four native checkers already get).
  Still a small, contained cost (one enum, one `lookup_any_architecture_checker` function reused a
  third time) versus the wider blast radius of the rejected alternatives — but the third site is a
  concrete example of exactly the kind of easy-to-miss call site this indirection-layer tradeoff
  warned about, worth noting for future 4th-category follow-ons per the Follow-on item below.
- **Negative**: `Check.architecture_checker` stays a single flat string namespace spanning two
  different trait registries with no schema-level indication of which — a config author can't tell
  from the field name alone whether `"content-rules"` resolves via `ImportGraph` or
  `DeclarationGraph`. Accepted per architecture.md §6: `checker::registry()` already resolves 7
  distinct checker names through one flat namespace today, so this matches an existing kibitzer
  convention (category encoded in the name, not a separate schema field) rather than introducing a
  new one.
- **Follow-on**: a future 4th checker category (should one arise) has a clear template to follow —
  decide what graph shape it needs, add a trait+registry if genuinely new, or fold into an existing
  one if genuinely compatible. This ADR's reasoning (per-item 1–4 above) is the checklist for that
  future decision, not just this one.

## Alternatives Rejected

See table above and the Pattern Decisions table in `../implementation/plan.md`.
