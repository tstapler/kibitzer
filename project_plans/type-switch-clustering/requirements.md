# Requirements: type-switch-clustering

**Date**: 2026-11-04
**Type**: feature addition
**Complexity**: 2 — new ArchModel-level checker using already-landed dependencies
**Source**: backlog item `88ab9ebf-25f0-4a13-aec5-c3987cf93555` / GitHub issue #60

## Problem Statement

`go-type-switch-density` (#38, already landed in `src/checkers/go_type_switch_density.rs`)
flags a single heavy type switch per file — it explicitly says in its own comment: "it
does not know whether the same type set recurs across other methods of the same type (the
signal that would turn this into an actual refactoring candidate) — that clustering step
is a stated dependency, tracked separately." `type_edges` (#40, already landed in
`src/arch_model.rs`) now provides the full type hierarchy. This item adds the missing
cross-method clustering layer: group type-switch sites by the field/variable they switch
on, constrain to methods of the same owning type, and flag clusters of 2+ methods as
either **Replace Conditional with Polymorphism** (a suitable hierarchy already exists in
`type_edges`) or **Replace Type Code with Subclasses** (no hierarchy exists and one must
be created).

## Baseline

Both upstream dependencies are **already landed**:
- `go-type-switch-density` checker — `src/checkers/go_type_switch_density.rs`, registered
  in `default_checks()` via `config.rs:883`.
- `type_edges: Vec<TypeRelationEdge>` — `src/arch_model.rs:266`; populated by
  `resolve_type_edges`; exposed via `list_supertypes`/`list_subtypes` MCP tools in
  `src/mcp.rs:1366`.

Today there is no checker that reads `ArchModel` to cluster type-switch sites across
methods. The existing `architecture_checks.rs` has an `ArchModel`-based checker shape
(the `ArchModelChecker` trait at line ~597), which is the right integration point for
cross-file, cross-method analysis.

## Users / Consumers

- AI agents (Claude Code sessions via MCP or hook output) receiving a kibitzer finding
  that names the specific type, the shared switch subject, the methods involved, and the
  recommended refactoring (Polymorphism vs. Subclasses).
- CLI `kibitzer run` / `kibitzer check` batch runs on Go codebases.

## Success Metrics

- A new `go-type-switch-clustering` checker fires on a Go struct whose 2+ methods each
  contain a `switch x.(type)` keyed on the same receiver-field or parameter (the same
  subject expression), and the finding names: the owning type, the switch subject, the
  method names, and which Fowler refactoring applies.
- The checker uses `ArchModel.type_edges` to distinguish which recommendation to make:
  if `list_subtypes` of the switched-on type already yields 1+ subtypes (a hierarchy
  exists), recommend **Replace Conditional with Polymorphism**; otherwise recommend
  **Replace Type Code with Subclasses**.
- Zero false positives on the standard backtest-repos corpus
  (`kubernetes/kubernetes`, `BurntSushi/ripgrep`, `denoland/deno`, etc.) at the chosen
  clustering threshold (2+ methods, same switch subject).
- Registered in `default_checks()` as advisory (consistent with `go-type-switch-density`).
- Backtest run (`kibitzer check backtest go-type-switch-clustering`) executed and results
  recorded in a `docs/go-type-switch-clustering-false-positives.md` page.

## Appetite

Small (3–5 days). Both data sources (`ArchModel` with `type_edges`, plus the per-file
type-switch AST sites already parseable from the existing `go_type_switch_density`
machinery) are available. The main implementation work is: (1) extract type-switch sites
per method with their switch-subject expression and owning-type name, (2) group by
(owning-type, subject), (3) filter to clusters of size ≥ 2, (4) look up `type_edges` for
the switched-on type, (5) emit the right finding message.

## Constraints

- Go-only for v1 (consistent with `go-type-switch-density` rationale). Java/TS
  `instanceof` chains are a future scope-widening item.
- Must not introduce a second full-repo tree-sitter pass — reuse `ArchModel`'s existing
  symbol walk (methods are already `SymbolNode`s with `parent` set to the owning type;
  the type-switch sites can be extracted during the same pass or via a targeted per-file
  re-walk at model build time).
- Must follow `ArchModel`-based checker conventions (the `ArchModelChecker` or equivalent
  trait at `src/architecture_checks.rs:~597`) rather than inventing a new integration
  shape.
- Must not duplicate logic already in `go_type_switch_density.rs` — reuse or extract
  the `type_case_count` / `collect_dense_type_switches` traversal helpers as a shared
  module-level function or move them to a shared location.

## Non-functional Requirements

- **Performance**: the clustering pass runs at `batch` trigger granularity (whole-repo
  `ArchModel` already built), not on every `PostToolUse` edit — consistent with
  `architecture_checks.rs`'s existing convention for model-dependent checkers.
- **Correctness over completeness**: if the switch-subject expression differs even
  slightly between two methods (e.g. `x.kind` vs. `kind`) they must NOT be clustered —
  conservative string-equality on the normalized subject expression text.
- **Backtest required** before landing (per `CLAUDE.md`'s "Backtest every functional
  change" rule).

## Scope

### In Scope
- `go-type-switch-clustering` ArchModel-level checker in a new file
  `src/checkers/go_type_switch_clustering.rs`.
- Per-method type-switch site extraction (switch subject, case count, owning type name)
  — either reusing `go_type_switch_density.rs`'s helpers or extracting them to a small
  shared module.
- Clustering by (owning-type, switch-subject) across methods of that type.
- `type_edges` lookup to choose Polymorphism vs. Subclasses recommendation.
- Registration in `default_checks()` as advisory, Go-only.
- Unit tests with multi-file fixtures covering: cluster fires, cluster doesn't fire for
  single-method case, subject-mismatch prevents cluster, hierarchy-present vs.
  hierarchy-absent recommendation difference.
- Backtest run against `docs/backtest-repos.md` corpus; false-positive doc created.

### Out of Scope (this item)
- Java/TS `instanceof` or downcast chains — same smell, different AST shape; future item.
- Go `if _, ok := x.(T); ok { ... } else if ...` type-assertion chains — explicitly
  deferred in `go_type_switch_density.rs`'s own comment.
- Automatically applying the refactoring — detection only, same as all other checkers.
- Extending the CLI `refactor` subcommand for this refactoring.

## Rabbit Holes

- Switch subjects across methods may differ in their receiver variable name (`r.kind` vs.
  `s.kind` where both are the same field on the same type). Normalizing to `<field>` only
  (strip the receiver prefix) would increase clustering recall but risks false clusters.
  v1 should use conservative full-expression string comparison and document the
  limitation.
- A `type_edges` lookup for the switched-on interface type requires knowing which type
  the type switch is branching over, not just which field. The switch's subject value
  (`x` in `switch x.(type)`) must be traced back to a declared interface type — if that
  type name can't be resolved to a known `SymbolNode`, the hierarchy-present check should
  default to "no hierarchy, suggest Subclasses" conservatively.
- Multi-file method spread: in Go, all methods of a type are in the same package but may
  be in different files. `ArchModel`'s symbol table already crosses file boundaries within
  a package, so grouping by `SymbolNode.parent` (owning type name) is the right join key.
