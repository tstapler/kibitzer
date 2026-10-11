# Architecture Review: long-function-flat-literal

## Lens 1 — Single Responsibility

`check_declaration` currently handles `long-function`, `deep-nesting`,
`long-parameter-list`, and `flag-argument` — four rules in one function. Adding a guard
expression (`&& !is_flat_data_literal_body(body, cfg)`) to the `long-function` branch
does not change this. The function remains a "check one declaration for all per-declaration
rules" dispatcher; each rule's logic is still a self-contained branch. The new predicate
`is_flat_data_literal_body` is factored out as its own named function rather than
inlined, maintaining the existing convention (`max_nesting_depth`, `collect_condition_identifiers`
are all similarly factored helpers). Passes.

## Lens 2 — Open/Closed

The `LangRuleConfig` struct is open for extension via new fields. Adding
`collection_literal_kinds` extends it without modifying the struct's existing fields.
The 8 per-language constructors each add one new field entry — a mechanical, safe
extension. Passes.

## Lens 3 — DRY

`density_boundary_kinds` already includes collection literal kinds for Go (`composite_literal`)
and TypeScript (`array`, `object`). Architecture research (Option A) considered reusing
this to avoid a new field. We chose Option B (explicit new field) for the same reason
`binding_finder` was not derived from `literal_kinds` in `replace-magic-literal`: derived
fields fail silently when the source changes for unrelated reasons. This is a deliberate,
documented divergence from maximum DRY — the correct trade-off here. No concern.

## Lens 4 — Hotspot risk

This change adds approximately:
- 25 lines for `is_flat_data_literal_body` + `has_logic_descendants`
- 1 line guard in `check_declaration`
- 8 lines (one per constructor) for `collection_literal_kinds`
- ~30 lines of tests

`check_declaration` grows by 1 line. `src/checkers/rules.rs` grows by ~65 lines. The file
is already large (~2400 lines post `replace-magic-literal`), but no hotspot alert is
triggered by a 65-line addition that follows an established pattern. The two new
functions do not push any single function above the 40-line threshold:
`is_flat_data_literal_body` is ~12 lines, `has_logic_descendants` is ~12 lines. Passes.

## Lens 5 — Test coverage fitness

The 5 planned tests (Epic 2) cover:
- True negative (flat literal exempted, AC1)
- False negative gate (call expression in literal, AC2)
- False negative gate (multi-statement body, AC4)
- Cross-language coverage (TypeScript, AC5)
- Regression guard (existing `flags_long_function` still passes, AC6/existing)

This is proportionate to the change's surface area — matches the 4–6 test standard for
a small precision fix. Passes.

## Verdict

Architecture is sound. No structural concerns. The change is proportionate and follows
every established pattern in the file.
