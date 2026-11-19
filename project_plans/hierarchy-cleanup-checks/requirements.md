# Requirements: hierarchy-cleanup-checks

**Date**: 2026-11-04
**Type**: feature addition
**Complexity**: 3-4 — three new ArchModelChecker impls, one new SymbolNode field, backtesting against public corpus
**Source**: backlog item `42bcb4ee-d74c-4352-8259-150d0cadbd37` / GitHub issue #61

## Problem Statement

Three inheritance-chapter refactorings from Fowler's catalog are not yet detectable by
kibitzer, all gated on `ArchModel.type_edges` from #40 (now shipped):

- **Remove Subclass** (Fowler §12.4, née "Replace Subclass with Fields") — a leaf subtype
  exists solely to override methods with constant returns and adds no unique fields; it
  could be collapsed into a field value on the base type.
- **Extract Superclass** (Fowler §12.8) — two sibling types with no existing hierarchy
  edge share significant duplicated structure or behavior; they would benefit from a common
  abstract supertype.
- **Collapse Hierarchy** (Fowler §12.9) — a subtype and its direct supertype are barely
  distinguishable; the hierarchy adds indirection without value.

## Dependencies

- **#40** (type-hierarchy-graph): `ArchModel.type_edges: Vec<TypeRelationEdge>` must be
  present — already shipped and confirmed in `src/arch_model.rs:266`.
- **#38** (extract-class): `extract_class.rs`'s Jaccard + HAC clustering machinery is
  reused for the Extract Superclass cross-type similarity case.

## Baseline

Today `ArchModel` has `type_edges` (from #40) with `from`/`to` symbol ids, `kind`
(`Option<TypeRelationKind: Extends|Implements>`), `resolved: bool`, `file`, `line`.
No checker currently reads `type_edges`. `SymbolNode` has `is_pure_delegation` (single
call or return-call body) but not `is_constant_return` (single literal return body) —
needed for Remove Subclass. The Extract Class clustering logic lives in
`src/extract_class.rs` and is currently only used within a single type's method set.

## Users / Consumers

- AI agents using kibitzer via MCP who want to identify inheritance-chapter refactoring
  opportunities in a codebase.
- Engineers running `kibitzer check` or the hook in batch mode.
- Future kibitzer checkers that build on hierarchy graph traversal.

## Success Metrics / Acceptance Criteria

1. **Remove Subclass checker** (`remove-subclass`) is implemented as an `ArchModelChecker`
   that fires on leaf subtypes (resolved `Extends` edges, no subtypes of their own) where
   every overriding method has a single-literal-return body and the subtype declares no
   additional fields beyond the supertype's.
2. **`SymbolNode.is_constant_return: bool`** is added to `src/arch_model.rs` and populated
   by `symbol_extract.rs` for Go and TypeScript (the two languages with existing
   `is_pure_delegation` extraction), with `#[serde(default)]` for cache compatibility.
3. **Extract Superclass checker** (`extract-superclass`) is implemented as an
   `ArchModelChecker` that finds unrelated type pairs (no existing `type_edges` path
   between them) with Jaccard similarity of field-access sets above a configurable
   threshold (default 0.5), restricted to types with enough methods for clustering to be
   meaningful (>= `MIN_METHODS_FOR_CLUSTERING` from `extract_class.rs`), and reports each
   pair with the shared method/field set listed.
4. **Collapse Hierarchy checker** (`collapse-hierarchy`) is implemented as an
   `ArchModelChecker` that fires on `(subtype, supertype)` pairs where the subtype's unique
   method count is below 20% of its total method count, using resolved `Extends` edges.
5. All three checkers are registered in `architecture_checks::model_registry()` and in
   `config::default_checks()` with `architecture_checker` kind, `advisory` severity.
6. Each checker has unit tests covering: the positive (fires) case, the negative (doesn't
   fire) case, and the boundary/threshold condition.
7. All three checkers are backtested against the public corpus (`scripts/clone-backtest-repos.sh`)
   with true/false-positive triage documented in `docs/`.
8. `docs/hierarchy-cleanup-checks.md` is written covering check rationale, thresholds,
   known false-positive patterns, and suppression examples.
9. No regression in `cargo test` after the changes.

## Out of Scope

- Go structural interface satisfaction (method-set matching without embedding) — already
  excluded from `type_edges` v1 per `arch_model.rs:237-239`.
- Rename of extracted superclass (naming is a human/LLM step).
- Extract Superclass for types in different packages (cross-package clustering deferred).
- Push-up / pull-down of specific methods (covered by #59 separately).
- Replace Type Code with Subclasses / Replace Conditional with Polymorphism (covered by #60).
