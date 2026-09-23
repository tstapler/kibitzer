# ADR-001: Leave the 13 cross-grammar-shared kind comparisons unmigrated

**Status**: Accepted
**Date**: 2026-09-22

**Correction (Story 4.1.1/4.1.2)**: this ADR originally identified 11 sites (8 in
`rules.rs`, 3 in `symbol_extract.rs`). Story 3.2.2's `js_ts_is_exported`
`export_statement` check and Story 4.1.1's completeness sweep found two more genuinely
cross-grammar-shared sites in `symbol_extract.rs` (`walk_calls`'s `call_expression`
check, alongside the originally-identified `function_kinds.contains` check) that fit
this ADR's same reasoning and were seamed the same way. The final, confirmed count is
**13** (8 in `rules.rs`, 5 in `symbol_extract.rs` — verified via `rg -c
'SEAM\(typed-node-kind-migration\)' src/checkers/rules.rs src/symbol_extract.rs`). The
body below is left as originally written for historical accuracy; read "11"/"3" as
superseded by "13"/"5" throughout.

## Context

The typed-node-kind migration converts `node.kind() == "literal"` comparisons to the
generated `<Lang>Kind` enums (`GoKind`, `JavaKind`, etc.) so a misspelled kind name fails
`cargo build` instead of silently never matching. Of ~104 comparisons across the four
flagged high-risk files (`src/checkers/rules.rs`, `src/symbol_extract.rs`,
`src/import_graph.rs`, `src/declarations.rs`), research (see
`project_plans/typed-node-kind-migration/research/architecture.md` §1, §3) plus the plan's
own Story 3.1.2 enumeration found 11 sites that cannot become a single `<Lang>Kind`
comparison:

- 8 in `rules.rs`: the 6 generic engine functions (`walk_declarations`, `walk_blocks`,
  `check_block_for_unreachable`, `collect_condition_identifiers`, `max_nesting_depth`,
  `walk_if_chain`) driven by `LangRuleConfig`, a runtime struct whose `&'static str`
  fields (`if_kind`, `block_kind`, `nesting_kinds`, etc.) hold a different string per
  language but are consumed by one shared function body across all 8 grammars; plus
  `collect_identifiers`'s `node.kind() == "identifier"` site (L899), which is itself
  called generically across all 8 grammars even though it isn't `LangRuleConfig`-driven;
  plus `rules.rs:784`'s `.kind().contains("comment")` substring check, which has no
  single-variant equivalent regardless of grammar count.
- 3 in `symbol_extract.rs`: `enclosing_kind_name` (parameterized by a per-caller kind-name
  slice), `walk_calls`'s `LangSymbolConfig`-driven `function_kinds.contains` check, and
  `callee_text_for`'s single `match` arm that literally unions Go's `selector_expression`
  and JS/TS's `member_expression` in one arm.

Each `<Lang>Kind` enum is a distinct Rust type with no shared trait or common variant set
(`src/node_kind.rs`) — there is no single type these 11 sites could compare against without
either inventing new generic infrastructure or duplicating the surrounding function once
per language.

## Decision

Leave all 11 sites as raw `&str` comparisons for this migration. Mark each with a
standardized banner comment (`// SEAM(typed-node-kind-migration): <reason> — see
ADR-001.`) identifying it as an intentional seam, so a future reader — and a mechanical
completeness sweep (plan.md Story 4.1.1) — sees a deliberate, greppable scoping decision
rather than an incomplete migration.

Three options were considered (architecture.md §3). Note that architecture.md §3's own
recommendation was **not** uniform across all 11 sites: it recommended (B) for 8 of them
(the 6 `LangRuleConfig`-driven `rules.rs` engine functions plus `symbol_extract.rs`'s
`enclosing_kind_name` and `walk_calls` sites) and (C) only for `callee_text_for`
specifically. This ADR deliberately overrides architecture.md §3's per-site split and
applies (C) uniformly to all 11, for the reasons below — it is this ADR's own decision,
informed by but not simply adopting architecture.md's research.

- **(A) Introduce a shared abstraction** — a `LangKind` trait each `<Lang>Kind` enum
  implements, with `LangRuleConfig`/`LangSymbolConfig` made generic over it. **Rejected**:
  this is new generic-abstraction infrastructure, which requirements.md's Non-Goals
  section explicitly rules out ("Building any new codegen or enum-generation mechanism...
  is complete and unchanged by this work"). It also increases the blast radius of a
  migration whose own success metric is zero behavior change.
- **(B) Duplicate per-language** — split each shared function into N language-specific
  copies, each internally typed. **Rejected for this migration** (recommended by
  architecture.md §3 for 8 of the 11 sites as a possible *future*, separately-scoped
  follow-up): this roughly 6x's `rules.rs`'s engine section and 2x's the
  `symbol_extract.rs` functions, a real ongoing maintenance trade-off (one bug fix needs
  applying N times) that deserves its own dedicated proposal and review, not one folded
  silently into a type-safety-only refactor.
- **(C) Leave unmigrated** — chosen for all 11 sites. Lowest risk for a
  Complexity-4/zero-behavior-change migration; documents the gap explicitly rather than
  force-fitting, and keeps the disposition uniform across `rules.rs` and
  `symbol_extract.rs` rather than splitting 8 sites one way and 3 (or 1) another.

## Consequences

- These 11 sites (plus 2 downstream call sites in `src/checkers/complexity.rs` that read
  `rules.rs`'s still-raw `LangRuleConfig::function_kinds` field) keep today's typo risk:
  a misspelled kind string here still compiles fine and silently never matches, exactly
  the failure mode this migration exists to close everywhere else.
- `rules.rs`'s existing `node_kind_literals_are_valid_for_their_grammar` test-time
  validator remains non-redundant and must stay in place (per requirements.md's
  Non-Goals) — it is still the only backstop for these specific literals.
- A future project could pursue Option B for the 8 `LangRuleConfig`/`LangSymbolConfig`-
  driven sites (recommended, and the option architecture.md §3 itself favored for these)
  or Option A if a second cross-grammar-generic pattern emerges elsewhere in the crate
  making a shared trait worth its cost. `callee_text_for` specifically (3 lines, lowest
  blast radius, and the one site architecture.md §3 also recommended (C) for) is the
  weakest candidate for either follow-up and can likely stay on the seam indefinitely.
