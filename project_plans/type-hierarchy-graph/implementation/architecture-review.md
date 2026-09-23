# Architecture Review: type-hierarchy-graph
**Date**: 2026-09-22
**Verdict**: CONCERNS

## Constitution Violations
- [ ] None — no `docs/adr/ADR-000-architecture-constitution.md` exists in this repository (checked: `test -f docs/adr/ADR-000-architecture-constitution.md` → not found).

## Blockers
None. The prior BLOCKER (Task 2.1.2a's single `type_hierarchy` function combining
`call_traversal`'s and `list_architecture_symbols`'s full responsibility sets into one
70-100+ line function) is **resolved**. Verified by reading the current
`project_plans/type-hierarchy-graph/implementation/plan.md`:
- Story 2.1.2 now has five tasks instead of two: **2.1.2a** extracts a shared
  `paginate<T>(items, offset, limit) -> (Vec<T>, usize, Option<String>)` helper out of
  `list_architecture_symbols` (reused by both, not duplicated); **2.1.2b** extracts a
  `non_type_node_hint` helper for the node-kind/hint early-return check; **2.1.2c** is a
  slimmed `type_hierarchy` built from both helpers, stated at "~15-20 lines... comparable to
  `call_traversal`'s existing size"; **2.1.2d**/**2.1.2e** are the thin wrapper tool methods
  and tests (`plan.md:554-580`).
- The Tech Debt Disposition table's `src/mcp.rs` row (`plan.md:72`) now explicitly narrates
  that "a first draft of this plan proposed one `type_hierarchy` function... the
  architecture review caught that this combination would run 70-100+ lines" and gives a
  realistic, internally consistent per-piece estimate (`paginate<T>` ~15-25 lines,
  `non_type_node_hint` ~15-25 lines, `type_hierarchy` body ~15-20 lines, two ~10-line
  wrappers) that sums close to the original 70-100+ line estimate rather than
  under-claiming a single ~40-line body. This is a structurally sound split, not a
  cosmetic wording change — each resulting piece is comparable in size to an existing
  precedent function (`paginate<T>` ≈ `list_architecture_symbols`'s extracted logic,
  `type_hierarchy` ≈ `call_traversal`'s size), and `list_architecture_symbols` itself is
  edited to call the shared helper, which the table correctly notes shrinks rather than
  grows it.

Also resolved: `ArchModel` not deriving `Default` (`src/arch_model.rs:184`, still true —
VERIFIED this session). Story 1.1.1's AC (`plan.md:141-142`) now states "`cargo build
--tests` succeeds repo-wide after this story lands (not just `cargo build`)" as an explicit
acceptance criterion, and a new **Task 1.1.1c** (`plan.md:185-189`) lists all 18
`ArchModel { ... }` struct-literal construction sites the field addition breaks. Re-ran
`grep -rn "ArchModel {" src/` independently this session (excluding `-> ArchModel {`
function-signature matches): 18 literal-construction sites across `src/arch_model.rs` (8:
`build_model`, `filtered`, and 6 test-module literals), `src/arch_diagram.rs` (2),
`src/architecture_checks.rs` (2), `src/isp_fat_interface.rs` (1), `src/god_class.rs` (3),
`src/extract_class.rs` (1), `src/lsp.rs` (1) — matches the plan's count and file list
exactly.

## Concerns
- [ ] **Task 2.1.1b (`TypeHierarchyResponse.node_kind`)** — still `Option<String>` in the
  actual struct definition (`plan.md:519`), unchanged from the prior review. This edit pass
  updated the *prose* around it to assume the fix — the Data Model table
  (`plan.md:43`: "typed `Option<SymbolKind>`, not a hand-written string — reuses the
  existing enum's `#[serde(rename_all = "lowercase")]`") and Story 2.1.2's AC
  (`plan.md:~544`: "`node_kind: Some(SymbolKind::Function)` ... via `SymbolKind`'s existing
  `#[serde(rename_all = "lowercase")]` — see Task 2.1.1b") both now describe `node_kind` as
  typed `SymbolKind` — but the concrete struct code sample a builder would actually copy
  from, in Task 2.1.1b itself, was never updated to match. This is a new internal
  inconsistency introduced by the partial edit: two parts of the plan now assert a typed
  field, one part (the one with the literal code) still spells out `Option<String>`.
  **Remediation**: change `node_kind: Option<String>` to `node_kind: Option<SymbolKind>` in
  the Task 2.1.1b code block to match what the rest of the plan now already claims.

- [ ] **Story 1.6.1 (`resolve_one_type_edge`) / Task 1.6.1d** — unchanged from the prior
  review; still no AC or test covers `kind_hint: Some` combined with a target that fails to
  resolve (e.g. a TS class `implements` an external/framework interface not in this model,
  or `extends React.Component`). Task 1.6.1d's five tests (`plan.md:431`) are unchanged from
  before: they cover `kind_hint: None` + unresolved
  (`..._keeps_unresolved_edge_with_raw_text_and_none_kind`) and `kind_hint: Some` + resolved
  (`..._prefers_kind_hint_over_inferred_kind_when_both_available`), but not the `Some` +
  unresolved combination — plausibly the majority real-world case for TS/Java/Kotlin.
  **Remediation**: add `resolve_type_edges_keeps_kind_hint_when_target_is_unresolved` (a TS
  `implements Foo` site where `Foo` isn't in the model should still produce
  `TypeRelationEdge { kind: Some(Implements), resolved: false, to: "Foo", .. }`).

- [ ] **Task 1.1.1a (`TypeRelationEdge`)** — unchanged from the prior review. The
  `(resolved: bool, kind: Option<TypeRelationKind>)` pair still makes
  `(resolved: true, kind: None)` type-representable even though `resolve_one_type_edge`'s
  spec'd logic (Task 1.6.1c step 3, `plan.md:426`) never produces it. Low severity, not
  worth a bigger type-level redesign given the plan already deliberately chose
  `CallEdge`-consistency elsewhere. **Remediation**: add one test asserting
  `(resolved: true, kind: None)` doesn't occur for any Story 1.6.1/1.6.2 fixture, or note
  the implication in `TypeRelationEdge`'s doc comment.

## Nitpicks
- Unchanged from the prior review: the Pattern Decisions table's "Overall extraction
  architecture" row (`plan.md:51`) still describes the approach as "table-driven per-language
  config (extends `LangSymbolConfig`/`symbol_extract.rs`)," which overstates how literally
  `walk_calls`/`walk_struct_fields` (the actual precedent) extend `LangSymbolConfig` — they're
  bespoke per-language functions dispatched by a thin recursive walker. The approach itself
  is sound; only the wording overstates the precedent.
- Unchanged from the prior review: Story 1.2.1's ACs (`plan.md:236-237`) still cover a
  pointer embed (`*Animal`) and a generic embed (`Base[int]`) separately but not the combined
  shape (`*Base[int]`) — a small fixture-coverage gap worth adding to Task 1.2.1d's test list
  if the grammar actually nests these (not confirmed either way this session).
