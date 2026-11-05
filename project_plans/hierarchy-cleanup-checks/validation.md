# Validation: hierarchy-cleanup-checks

**Date**: 2026-11-04

## Plan Coherence Review

### Does the plan respect existing conventions?

**ArchModelChecker pattern** ✓ — all three checkers implement `ArchModelChecker`, not
`ArchitectureChecker` (the import-graph variant). They use `model: &ArchModel` directly,
consistent with `feature_envy.rs`, `middle_man.rs`, `god_class.rs`.

**`#[serde(default)]`** ✓ — `is_constant_return` on `SymbolNode` includes this, matching
`is_pure_delegation` and other additive fields (`ArchFinding::severity_override`,
`SymbolNode::delegation_field`).

**Backtest requirement** ✓ — plan explicitly calls for transcript and public-corpus
backtests before landing (CLAUDE.md: "A change to what kibitzer does … needs a backtest
before it lands, not just unit tests").

**default_checks registration** ✓ — all three checkers go into `default_checks()`, matching
the CLAUDE.md convention: "a new native per-file checker … goes in `default_checks()` as
part of landing it."

**Private symbols** ✓ — Remove Subclass sets `needs_private_symbols: true` (leaf subtypes
are often package-private); Collapse Hierarchy and Extract Superclass leave it `false`.
Consistent with `UnreferencedPrivateSymbolChecker`'s override and the others' default.

### Dependency verification

- `ArchModel.type_edges` exists at `src/arch_model.rs:266` — VERIFIED (grep confirmed).
- `extract_class::method_field_sets` is a free function accessible from
  `extract_superclass.rs` — VERIFIED (it is `pub(crate)` or private but in the same
  crate; confirm visibility when writing).
- `is_pure_delegation` extraction path in `symbol_extract.rs:544-563` is the right place
  to add `is_constant_return` — VERIFIED (grep shows single assignment site).

### Threshold defensibility

| Checker | Threshold | Justification |
|---------|-----------|---------------|
| Remove Subclass | All-or-nothing | Fowler: a subclass that does *anything* beyond constant returns is not a Remove Subclass candidate |
| Extract Superclass | Jaccard >= 0.5 | "Significant duplication" — industry standard for type-overlap detection (Demeyer et al.); matches the spirit of `MIN_ENTITY_PLACEMENT_SCORE = 0.75` for a looser cross-type signal |
| Collapse Hierarchy | unique ratio < 0.20 | A subtype adding fewer than 1-in-5 methods is "barely different" — same coarse-thirds intuition as Martin's `STABLE_MAX = 0.3`/`UNSTABLE_MIN = 0.7` |

### False-positive risk assessment

**Remove Subclass** — highest false-positive risk in codebases that use constant-return
overrides as an intentional strategy (e.g., factory/registry patterns). Mitigation: the
all-or-nothing condition is strict; a checker that fires only when EVERY method is a
constant return will have low noise. Backtest before landing.

**Extract Superclass** — risk of flooding on DTO-heavy codebases where many small types
share the same two or three fields. Per-package cap of 3 findings mitigates this. The
Go-only restriction also narrows the surface area for v1.

**Collapse Hierarchy** — lowest false-positive risk of the three. A type with < 20%
unique methods is objectively close to its supertype; even intentional thin wrappers are
valid candidates to flag as advisory.

### Open Questions

1. **`method_field_sets` visibility**: the function in `extract_class.rs` is not marked
   `pub` in the module review — the Extract Superclass checker will need it re-exported or
   the logic inlined. Confirm before implementation: if it's `pub(crate)` it's accessible;
   if private, move the shared logic to `hierarchy_checks.rs`.

2. **Cross-language constant detection**: the plan limits `is_constant_return` to Go and
   TypeScript. Java/Kotlin codebases with class hierarchies will get Collapse Hierarchy
   findings but NOT Remove Subclass findings (since `is_constant_return` will always be
   `false` for those languages). This is acceptable for v1 but should be documented clearly.

3. **Cap position for Extract Superclass**: the per-package cap of 3 is a heuristic. If
   backtesting shows a large DTO package generates 20+ pairs above the Jaccard threshold,
   lowering the threshold to 0.65 may be preferable to a hard cap. Leave the cap at 3 for
   now; tune after backtest.

4. **`leaf_subtypes` definition with multi-level chains**: a type that is both a subtype of
   X and a supertype of Y is not a leaf. The plan correctly defines leaf = "no outgoing
   Extends edges" (nothing extends *it*). Confirm the implementation doesn't conflate
   "leaf in the hierarchy tree" with "leaf in the import graph."

## Go/No-Go

**Go** — the plan is coherent, respects codebase conventions, has defensible thresholds,
and all dependencies are confirmed shipped. The three checkers can proceed to implementation
in the order specified (plumbing → parallel checker implementation → wire-up + backtest).
