# Implementation Plan: typed-node-kind-migration

**Feature**: Convert `src`'s raw `.kind() == "literal"` / `match .kind()` tree-sitter comparisons to the generated `<Lang>Kind` enums, so a misspelled kind name fails `cargo build` instead of silently never matching.
**Date**: 2026-09-22
**Status**: Ready for implementation
**ADRs**: ADR-001-leave-cross-grammar-kind-comparisons-unmigrated

---

## Domain Glossary

| Term | Definition | Notes |
|------|-----------|-------|
| `<Lang>Kind` | One of the 8 generated enums (`GoKind`, `TypeScriptKind`, `TsxKind`, `JavaScriptKind`, `PythonKind`, `JavaKind`, `KotlinKind`, `RustKind`) in `src/node_kind.rs`, one variant per named concrete kind in that grammar's `node-types.json`. | Already built, out of scope to change. |
| `Kind::of(node)` | `<Lang>Kind::of(node: tree_sitter::Node) -> Self` — the typed kind of a live node. The standard replacement for `node.kind()` at a comparison site. | |
| `Kind::from_kind_str(s)` | `<Lang>Kind::from_kind_str(s: &str) -> Self` — same conversion from an owned/borrowed `&str`. Used where a `&str` (not a live `Node`) is what's in hand. | |
| `Other` variant | Catch-all enum variant covering every anonymous/punctuation token and synthetic `ERROR`/`MISSING` node — anything `Node::kind()` can return that isn't a named concrete kind. | Must never be the *target* of a migrated comparison (see "named vs. anonymous token"). |
| Named kind / anonymous token | tree-sitter's own `node-types.json` distinction: a named kind (`"named": true`) gets an enum variant; an anonymous token (`"named": false`, e.g. Go's `&&`, Kotlin's bare `"*"` or `"class"` keyword) does not and has no typed equivalent. | Verify with `python3 -c "import json; ..."` against `codegen/node-types/<lang>.json` before converting any comparison against a short/symbolic literal. |
| Mechanical comparison | A `.kind() == "literal"` / `match .kind() { "a" => ... }` comparison inside a function whose caller only ever passes nodes from **one** grammar. Converts 1:1 to `Kind::of(node) == Kind::Variant`. | The majority case (19 of 23 files). |
| Cross-grammar-shared site | A comparison inside a function that runs identically for **more than one** grammar at once, with the kind string(s) supplied at runtime via a per-language config struct (`LangRuleConfig`, `LangSymbolConfig`) or a literal set spanning two grammars' vocabularies in one `match` arm. | Cannot become a single `<Lang>Kind` comparison without new abstraction. 11 sites exist: 8 in `rules.rs` (the 6 config-driven engine functions + `collect_identifiers`'s L899 site, which is generically called across all 8 grammars + `rules.rs:784`'s `.contains("comment")` substring check) and 3 in `symbol_extract.rs`. |
| Seam | The deliberate boundary left in place around a cross-grammar-shared site: the raw-string comparison stays exactly as it is today, marked with a standardized banner comment (`// SEAM(typed-node-kind-migration): ...`, see Story 3.1.2/3.2.2) naming this migration and pointing at this plan/ADR-001, so a future reader knows it's an intentional scoping decision, not an oversight — and so Phase 4's completeness sweep (Story 4.1.1) can programmatically distinguish "intentionally seamed" from "missed." | Applies to `rules.rs` and `symbol_extract.rs` only. |
| `child_by_field_name` field-name string | `Node::child_by_field_name(&str)`'s argument (`"left"`, `"body"`, `"function"`, etc.) — a distinct, unrelated mechanism from `.kind()` comparisons. | Has no typed equivalent in `node_kind.rs`/`build.rs` and remains a raw string permanently; out of scope for this migration. See `research/stack.md`. |
| `LangRuleConfig` / `LangSymbolConfig` | The two per-language config structs (`src/checkers/rules.rs:72`, `src/symbol_extract.rs:42`) whose `&'static [&'static str]` / `&'static str` fields (`if_kind`, `function_kinds`, `nesting_kinds`, `type_kinds`, etc.) are consumed generically by shared engine functions. | These fields are **not** converted to `<Lang>Kind` in this migration — see Tech Debt Disposition. |
| Codemod story | A mechanical-file story where the worker may (not must) use the verified `ast-grep` rewrite rule from `research/build-vs-buy.md` plus a `cargo build`-driven revert loop, instead of hand-editing every comparison. | Optional acceleration, never a substitute for the backtest acceptance criterion. |
| False-positive scope file | A file named in requirements.md's scope grep whose `.kind()` hits are actually `std::io::Error::kind()` (→ `std::io::ErrorKind`), unrelated to `tree_sitter::Node::kind()`. Confirmed for `src/dedup.rs:41` and `src/plugin.rs:682` by direct read — both are `Err(e) if e.kind() == std::io::ErrorKind::...` inside `match`/`if let` on I/O results, not tree-sitter node kinds. | Zero migration surface; closed by inspection, not by editing. |
| Backtest corpus (per checker) | The two-part mandatory re-verification from kibitzer's own CLAUDE.md: (1) transcript backtest (`kibitzer check backtest <name>`) and (2) real-world repo corpus (`kibitzer check native <name> <file>` over `docs/backtest-repos.md`'s cloned repos). | Required per story that changes a checker's actual comparison logic — see per-story ACs. |
| Completion signal | The final story that narrows/removes `src/node_kind.rs`'s `#![allow(dead_code, unused_imports)]` and re-runs the full workspace test suite + backtest corpus for every touched checker one last time. | Phase 4, single story, must run last. |

---

## Pattern Decisions

| Component | Pattern Chosen | Source | Alternative Rejected | Reason |
|-----------|---------------|--------|---------------------|--------|
| Node-kind representation at mechanical call sites | Sum type (`<Lang>Kind` enum) replacing primitive `&str` — "parse, don't validate" applied at the comparison site | Type-driven design | Keep `&str` + expand `rules.rs`'s existing `node_kind_literals_are_valid_for_their_grammar` runtime validator to cover all 23 files | Only catches a typo at `cargo test` time, not `cargo build`; also requires building new generalized validation infra, which is explicitly out of scope (requirements.md Alternatives Considered) |
| Multi-arm boolean kind check (`kind == "a" \|\| kind == "b"`, or `match kind { "a"\|"b" => true, _ => false }`) | `matches!(Kind::of(node), Variant::A \| Variant::B)` | Rust idiom / pitfalls.md §1 | Hand-written `match Kind::of(node) { A \| B => true, _ => false }` | Trips `clippy::match_like_matches_macro` under this repo's `-D warnings` CI gate — a failure mode the migration itself introduces if not avoided |
| Overall execution strategy for the 19 mechanical files | Hybrid: codemod-assisted or hand-edit per file (worker's choice), grouped into small per-language stories, each independently landable | Requirements.md Risk Control ("land each file as its own story/commit") | (1) Fully manual, no codemod option; (2) One giant PR converting all 23 files at once | (1) ~140 mechanical comparisons is repetitive low-judgment work a verified codemod already speeds up (research/build-vs-buy.md); (2) violates the explicit per-file-revertability requirement and makes backtest cost unmanageable in one story |
| The 8 `rules.rs` + 3 `symbol_extract.rs` cross-grammar-shared sites (11 total) | (C) Leave unmigrated this migration — raw-string comparisons stay exactly as-is, marked with a standardized seam comment | ADR-001 (deliberately overriding architecture.md §3's per-site recommendation: architecture.md §3 recommended (B) duplicate-per-language for 8 of the 11 sites and (C) only for `callee_text_for`; ADR-001 chose (C) uniformly, for the reasons in its Decision section) | (A) Introduce a `LangKind` trait + make `LangRuleConfig`/`LangSymbolConfig` generic over it; (B) duplicate each shared engine function once per language | (A) is new generic-abstraction infra, explicitly out of scope ("no new codegen/enum-generation mechanism"); (B) triples-to-sextuples the size of `rules.rs`'s engine section and is a real future maintenance trade-off that deserves its own dedicated story/review, not one folded into a zero-behavior-change migration |
| `rules.rs`/`symbol_extract.rs` internal structure | Isolate via seam (leave `LangRuleConfig`/`LangSymbolConfig` fields as `&'static [&'static str]`; migrate everything else in the file directly) | PoEAA — Transaction Script (each mechanical helper is a simple, self-contained procedure; no Domain Model/Repository layer warranted for a tree-walk comparison) | Wrapping every helper in a new `NodeKindChecker` service/repository abstraction | The functions are already simple, single-purpose procedures; adding a layer would be pattern-for-pattern's-sake with no recurring problem it solves |
| `import_graph.rs` / `declarations.rs` internal structure | Extend as-is (direct comparison swap, no new type) | PoEAA — Transaction Script | Isolate via seam | Confirmed (architecture.md §1, §5) these files have **no** shared cross-grammar function — every comparison is already scoped to one grammar via a dedicated `collect_<lang>_*` function or a per-language fn pointer, so there is nothing to isolate |

---

## Tech Debt Disposition

| Area | Existing Issue | Disposition | Justification |
|------|----------------|--------------|----------------|
| `src/checkers/rules.rs` | 6 generic engine functions (`walk_declarations`, `walk_blocks`, `check_block_for_unreachable`, `collect_condition_identifiers`, `max_nesting_depth`, `walk_if_chain`) compare `node.kind()` against `LangRuleConfig` string fields shared across all 8 languages; `rules.rs:784`'s `.kind().contains("comment")` is a substring check, not equality | **Isolate via seam.** Migrate the ~26 single-language/field-type sites directly; leave the 6 engine functions and the `.contains("comment")` site as raw strings with a seam comment | Forcing these into `<Lang>Kind` requires new generic-trait infra or 6x code duplication — both expand scope beyond "convert existing comparisons," which the zero-behavior-change constraint rules out |
| `src/symbol_extract.rs` | `enclosing_kind_name` (parameterized by a per-caller kind-name slice), `walk_calls`'s `ctx.cfg.function_kinds.contains(&node.kind())`, and `callee_text_for` (one `match` arm mixing Go's `selector_expression` and JS/TS's `member_expression`) | **Isolate via seam.** Migrate the ~25 Go-only/per-language-dispatch sites directly; leave these 3 as raw strings with a seam comment | Same reasoning as `rules.rs`; `callee_text_for` specifically is the single hardest snippet in the whole migration (one match arm literally unions two grammars' vocabularies) and is 3 lines — not worth a generic abstraction |
| `src/checkers/complexity.rs` | `collect_total_complexity`/`collect_complex_functions` read `crate::checkers::rules::lang_config(Language::Go).function_kinds` — a `&'static [&'static str]` sourced directly from `rules.rs`'s seam field | **Extend as-is, but do not migrate these 2 sites.** Because `LangRuleConfig::function_kinds`'s type isn't changing (see row above), `complexity.rs`'s `function_kinds.contains(&node.kind())` calls (lines 111, 141) cannot become a `GoKind` comparison without `rules.rs` first converting the field — which this migration explicitly does not do | Newly-found dependency (not called out in Phase 2 research): `complexity.rs` inherits `rules.rs`'s seam by construction. `complexity.rs`'s other 6 comparisons (lines 206, 211×5, 227, 240, 246, 250) are plain Go-literal comparisons and migrate normally. Separately: this cross-checker reach-into (`complexity.rs` → `rules.rs::lang_config`, a `pub(crate)` accessor) is a pre-existing layering smell independent of kind-typing — out of scope to fix here, but worth a tracked follow-up. |
| `src/import_graph.rs` | `QualifiedImportLangConfig`'s `package_decl_kind`/`import_stmt_kind` fields are `#[allow(dead_code)]`, documentation-only, never compared against a `Node::kind()` | **Extend as-is.** Leave the two fields as plain `&'static str` (or drop them as an optional micro-cleanup) | They're dead code today; converting them to an enum for a value that's never read has no safety benefit and drifts into a scope-creep cleanup not requested by requirements.md |
| `src/declarations.rs` | `collect_kotlin_declarations`'s `"class"`/`"interface"` keyword-child scan (L430-436) disambiguates two concepts that share one `class_declaration` node kind | **Extend as-is; leave as raw string (confirmed non-goal).** Both `"class"` and `"interface"` are `"named": false` in `kotlin.json` — no enum variant exists for either | Verified directly against `codegen/node-types/kotlin.json`; forcing this into `Other` would defeat the entire purpose of the migration for this exact site (any anonymous token collapses to the same catch-all) |
| `src/dedup.rs`, `src/plugin.rs` | Named in requirements.md's scope grep | **No-op — confirm and close.** `dedup.rs:41` and `plugin.rs:682` are `std::io::Error::kind()` (→ `ErrorKind`), not `tree_sitter::Node::kind()` | Verified by direct read of both call sites; there is no tree-sitter comparison in either file to migrate |
| `src/tree_walk.rs` | `walk_preorder` itself has zero `.kind()` calls; all 3 hits (lines 61, 75, 78) are inside `#[cfg(test)] mod tests`, using the Go grammar | **Extend as-is (test-only migration, minimal backtest risk).** Migrate the 2 equality checks in the test module to `GoKind`; the `.to_string()` collection call (line 61) has no equality to migrate | Correction to pitfalls.md §4's flat "shared-infra backtest cost" framing for this specific file: since production code (`walk_preorder`) has no kind logic at all, no dependent checker's *behavior* changes — only this file's own unit test gets typed, so this file needs no cross-checker backtest re-run beyond `cargo test tree_walk::` |

---

## Migration Plan

No schema or data changes. This is a pure source-code type migration (raw `&str` comparisons → generated enum comparisons) with zero runtime behavior change and no persisted state.

## Observability Plan

- **Logs**: None added — no new logging surface.
- **Metrics**: None added — not applicable per requirements.md NFRs.
- **Alerts**: None added. The compile-time typo protection this migration adds *is* the observability improvement (a wrong kind name now fails `cargo build`, visible in CI immediately, instead of needing a backtest run to notice).

## Risk Control

- **Feature flag**: None applicable — internal refactor, no runtime-toggleable behavior.
- **Rollback procedure**: `git revert` the single commit for the affected file/story. Every story in this plan is scoped to produce exactly one commit covering 1-4 files, so a bad conversion in one story never blocks or entangles another story's revert.
- **Staged rollout**: Land Phase 1 (reference pattern) first and get it reviewed before parallelizing Phase 2's mechanical stories, so all workers converge on one idiom (`Kind::of(node) == Kind::Variant`, `matches!` for multi-arm). Phase 3 (the 4 flagged files) starts only after at least one Phase 2 story has landed cleanly through CI, confirming the codemod/build-loop workflow. Phase 4 (completion signal) runs strictly last, after every other story has merged.
- **Backtest-regression merge gate (applies to every story's backtest acceptance criterion in Phases 1–4)**: A non-empty backtest diff blocks merge for that story. Do not proceed to seam-comment/cleanup tasks or mark the story done until the diff is resolved or the finding is confirmed as a pre-existing flake unrelated to this migration. This is the operative "fail" branch for every "finding counts match exactly" AC in this plan — none of them are optional or advisory.
- **Baseline-durability gate (applies to every baseline-capture task in Phases 1–4)**: `/tmp` is not durable or shared across the ~17 independent worker sessions this plan dispatches. Every baseline-capture task must, in addition to saving raw output to `/tmp`, paste the captured output — or a concise excerpt/finding-count summary if the raw output is large — into that story's commit message or PR description, so it's git-tracked and retrievable by a later story. Task 4.1.1d (the final cross-story integration re-run) must treat a prior story's baseline as unavailable if it cannot be retrieved from that story's commit/PR description, and **fails closed** in that case: it does not substitute a fresh capture as a stand-in baseline. It blocks completion until the baseline is located (e.g. by checking out that story's commit) or that checker's backtest is explicitly re-run from a documented earlier reference point (e.g. the commit immediately before that story's change).
- **Backtest sanity-check gate (applies to every backtest acceptance criterion in Phases 1–4)**: A pre-migration baseline of "0 findings" is not by itself evidence of "no regression" — it may mean the checker never ran (missing corpus clone, CLI mismatch) rather than that the checker legitimately found nothing. Every backtest task must confirm its pre-migration baseline shows a nonzero finding count for at least one checker/corpus-file combination expected to fire, OR record an explicit, checked reason a zero count is legitimate (e.g. "checker doesn't fire on this corpus slice, confirmed by inspecting the corpus file for the pattern it targets"). A bare "0 findings, identical" is not sufficient evidence of no regression on its own.

## Unresolved Questions

- [ ] Should `import_graph.rs`'s two dead-code-only fields (`package_decl_kind`, `import_stmt_kind`) be dropped entirely as a small side-cleanup, or left as documentation strings? — blocks nothing (either choice satisfies this migration's scope) — owner: implementer of Story 3.3.1, default to "leave as-is" if not resolved before that story starts.
- [ ] Should the 11 seamed cross-grammar sites (8 in `rules.rs`, 3 in `symbol_extract.rs`) become a tracked follow-up backlog item for a future "duplicate-per-language" migration (Option B)? — blocks nothing in this migration — owner: Tyler, decide after Phase 4 lands; ADR-001 documents the option for later reference either way.

## Dependency Visualization

```
Phase 1 (Foundation)
  1.1.1 Confirm false-positive scope files (dedup.rs, plugin.rs)  ─┐
  1.1.2 Reference pattern: go_ignored_error.rs + verify codemod   ─┴─► gates all of Phase 2/3 (idiom + tooling must be confirmed first)

Phase 2 (Mechanical batch, fully parallel after 1.1.2 lands)
  2.1.1 go_blank_imports / go_type_switch_density / go_table_driven_test
  2.1.2 go_error_context.rs
  2.1.3 go_bulk_fetch_linear_scan.rs / go_call_resolution.rs
  2.2.1 java_error_context / java_ignored_error / java_lost_exception_cause / java_swallowed_interrupt
  2.3.1 dedup.rs + plugin.rs (no-op confirmation)              ─── independent, no code change
  2.3.2 complexity.rs + complexity_tests.rs                    ─── independent of rules.rs epic (function_kinds sites simply excluded)
  2.3.3 primitive_obsession.rs / isp_fat_interface.rs / tree_walk.rs
  2.3.4 god_class.rs
        (all Phase 2 stories run independently in parallel — no shared-file contention)

Phase 3 (High-risk, after >=1 Phase 2 story merged through CI)
  3.1.1 rules.rs mechanical majority  ─►  3.1.2 rules.rs seam + comment + test-redundancy check
  3.2.1 symbol_extract.rs mechanical majority ─► 3.2.2 symbol_extract.rs seam + comment
  3.3.1 import_graph.rs (Extend as-is)
  3.3.2 declarations.rs (Extend as-is)
        (3.1.x, 3.2.x, 3.3.x are 3 independent epics, no file overlap)

Phase 4 (Completion signal — runs last, after every Phase 2/3 story merged)
  4.1.1 Narrow node_kind.rs allow(dead_code, unused_imports); full workspace test + full backtest re-run
  4.1.2 Land the migration-completeness sweep as a permanent CI script gate (after 4.1.1, so it checks the final state)
```

---

## Phase 1: Foundation

### Epic 1.1: Confirm scope, establish reference pattern, verify tooling
**Goal**: Close the two false-positive scope files by inspection, and land one small reference-quality migration plus a verified codemod rule so every Phase 2/3 worker converges on the same idiom before parallelizing.

#### Story 1.1.1: Confirm and close the two false-positive scope files
**As a** migration implementer, **I want** `dedup.rs` and `plugin.rs` confirmed as having zero tree-sitter migration surface, **so that** no worker wastes time trying to migrate an `std::io::ErrorKind` comparison into a `<Lang>Kind`.
**Acceptance Criteria**:
- Both files' `.kind()` call sites are confirmed to be `std::io::Error::kind()`, not `tree_sitter::Node::kind()`, and no code change is made to either file.
  - *Given* `src/dedup.rs:41` (`Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => false`) and `src/plugin.rs:682` (`Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}`), *When* each is read in context, *Then* both are I/O-error matches inside a `match`/`if let` on a `Result`, with no `tree_sitter::Node` in scope at that line.
- `cargo build --workspace` and `cargo test --workspace` still pass unchanged (no diff was made).
**Files**: `src/dedup.rs`, `src/plugin.rs`

##### Task 1.1.1a: Read and confirm both call sites (~3 min)
- Open `src/dedup.rs:35-45` and `src/plugin.rs:675-690`; confirm the receiver type of `.kind()` in each via the surrounding `Result<_, std::io::Error>` type.
- No edit made. Record confirmation in the PR/commit description for this story (a one-line note, not a new doc file).
- Files: `src/dedup.rs`, `src/plugin.rs`

#### Story 1.1.2: Reference migration — `go_ignored_error.rs` — and verify the ast-grep codemod rule
**As a** migration implementer, **I want** one small, fully mechanical file migrated by hand as the reference pattern, and the `ast-grep` rewrite rule re-verified against it, **so that** Phase 2 workers have both a worked example and a confirmed tool.
**Acceptance Criteria**:
- `go_ignored_error.rs`'s 4 comparisons (`function.kind() != "selector_expression"`, `operand.kind() != "identifier"`, `.filter(|n| n.kind() == "identifier")`, per research/stack.md §4) are converted to `GoKind::of(...) == GoKind::Variant` form.
  - *Given* `src/checkers/go_ignored_error.rs`'s `if function.kind() != "selector_expression" { return false; }`, *When* migrated, *Then* it reads `if GoKind::of(function) != GoKind::SelectorExpression { return false; }`, and a deliberately introduced typo (`GoKind::SelectorExpresion`) fails `cargo build` with an "no variant named" compile error.
- The `research/build-vs-buy.md` §2 `ast-grep` rewrite rule (`kind-to-enum.yml`, pattern `$NODE.kind() == $KIND` → `GoKind::of($NODE) == GoKind::$ENUM`) is re-run against a scratch copy of the pre-migration file in `/tmp` and confirmed to reproduce the same hand-written result; the rule file itself is a scratch artifact under `/tmp`, not committed to the repo.
  - *Given* the scratch copy of `go_ignored_error.rs` before migration, *When* `sg scan --rule /tmp/kind-to-enum.yml <scratch-file>` runs, *Then* its output matches the hand-migrated `GoKind::of(...) == GoKind::Variant` lines exactly.
- `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo build --workspace`, `cargo test --workspace` all pass.
- Backtest: `kibitzer check backtest go-ignored-error` (transcript) and `kibitzer check native go-ignored-error <file>` over the Go slice of `docs/backtest-repos.md`'s corpus (e.g. `kubernetes/kubernetes`) produce identical finding counts to the pre-migration baseline. This is a **cheap, single-checker** backtest — `go_ignored_error.rs` has no other checker depending on it.
  - *Given* a pre-migration baseline finding count captured via `kibitzer check native go-ignored-error <sample-file>` before this story's edit, *When* the same command runs post-migration, *Then* the finding count and file:line list are byte-identical.
**Files**: `src/checkers/go_ignored_error.rs`

##### Task 1.1.2a: Capture pre-migration backtest baseline (~3 min)
- Run `kibitzer check native go-ignored-error` over 2-3 sample Go files (including one from `docs/backtest-repos.md`'s corpus) and save the output to `/tmp/go-ignored-error-baseline.txt`.
- Confirm the baseline shows a nonzero finding count (or record an explicit, checked reason zero is legitimate) per the backtest sanity-check gate.
- Paste the baseline output (or a finding-count summary) into this story's commit/PR description per the baseline-durability gate — `/tmp` alone will not survive to Phase 4.
- Files: none (read-only)

##### Task 1.1.2b: Migrate the 4 comparisons by hand (~4 min)
- Add `use crate::node_kind::GoKind;` (or the correct existing import path — check `src/node_kind.rs`'s re-export path first).
- Replace all 4 `.kind()` comparisons per the AC above.
- Files: `src/checkers/go_ignored_error.rs`

##### Task 1.1.2c: Verify the codemod rule reproduces the same result (~3 min)
- Copy the pre-migration file to `/tmp/go_ignored_error_scratch.rs`, write `/tmp/kind-to-enum.yml` per research/build-vs-buy.md §2, run `sg scan --rule /tmp/kind-to-enum.yml --update-all /tmp/go_ignored_error_scratch.rs`, diff against the hand-migrated file.
- Files: none (scratch only, not committed)

##### Task 1.1.2d: Run full CI gate + backtest, compare to baseline (~4 min)
- Run `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo build --workspace && cargo test --workspace`.
- Re-run the same backtest commands from Task 1.1.2a and diff against `/tmp/go-ignored-error-baseline.txt`.
- Files: none

---

## Phase 2: Mechanical batch (16 files; 19 mechanical files total including Phase 1's 3)

### Epic 2.1: Go checkers batch
**Goal**: Migrate the remaining 6 Go-only checker files not covered by the Phase 1 reference story.

#### Story 2.1.1: Small Go checkers — `go_blank_imports.rs`, `go_type_switch_density.rs`, `go_table_driven_test.rs`
**As a** migration implementer, **I want** these 3 small Go-only files converted to `GoKind`, **so that** their kind comparisons get compile-time typo protection.
**Acceptance Criteria**:
- Every `.kind() ==`/`!=` and `match .kind()` comparison against a named concrete Go kind converts to `GoKind::of(...)`/`matches!(GoKind::of(...), ...)`. Anonymous-token comparisons (verify each short/symbolic literal against `codegen/node-types/go.json`'s `"named"` field first) stay raw strings.
  - *Given* `go_blank_imports.rs:123`'s `if prev.kind() == "comment" && ...`, *When* migrated, *Then* it reads `if GoKind::of(prev) == GoKind::Comment && ...` (confirmed `"comment"` is `"named": true` in `go.json`).
  - *Given* `go_table_driven_test.rs`'s `LITERAL_KINDS` slice (a `&[&str]` membership check, Pattern D), *When* migrated, *Then* it becomes a `&[GoKind]` slice compared via `.contains(&GoKind::of(node))` — no behavior change, same set membership semantics.
- `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings` (watch specifically for `clippy::match_like_matches_macro` on any converted `match`), `cargo build --workspace`, `cargo test --workspace` all pass.
- Backtest: `kibitzer check backtest go-blank-imports`, `kibitzer check backtest go-type-switch-density`, `kibitzer check backtest go-table-driven-test` (transcript) plus `kibitzer check native <name> <file>` over the Go backtest-repos corpus for each of the 3 checkers — all produce identical finding counts to pre-migration. These are cheap, single-checker backtests (no shared-infra dependents beyond `tree_walk.rs`, whose production code is unaffected per its own disposition row).
  - *Given* a pre-migration baseline for `go-blank-imports` captured against `kubernetes/kubernetes`'s Go files, *When* re-run post-migration, *Then* the finding count is identical.
**Files**: `src/checkers/go_blank_imports.rs`, `src/checkers/go_type_switch_density.rs`, `src/checkers/go_table_driven_test.rs`

##### Task 2.1.1a: Capture pre-migration backtest baselines for all 3 checkers (~4 min)
- Confirm each baseline shows a nonzero finding count (or record an explicit, checked reason zero is legitimate) per the backtest sanity-check gate.
- Paste each baseline output (or finding-count summary) into this story's commit/PR description per the baseline-durability gate.
- Files: none (read-only)

##### Task 2.1.1b: Migrate `go_blank_imports.rs` (codemod or hand) (~5 min)
- Files: `src/checkers/go_blank_imports.rs`

##### Task 2.1.1c: Migrate `go_type_switch_density.rs` (codemod or hand) (~3 min)
- Files: `src/checkers/go_type_switch_density.rs`

##### Task 2.1.1d: Migrate `go_table_driven_test.rs`, including the `LITERAL_KINDS` slice conversion (~5 min)
- Files: `src/checkers/go_table_driven_test.rs`

##### Task 2.1.1e: Run full CI gate + all 3 backtests, compare to baselines (~5 min)
- Files: none

#### Story 2.1.2: `go_error_context.rs` (20 comparisons)
**As a** migration implementer, **I want** this file's 20 Go kind comparisons converted, **so that** it gets the same compile-time protection as the smaller Go files.
**Acceptance Criteria**:
- All named-kind comparisons convert to `GoKind`; any anonymous-token comparisons (verify against `go.json` first) are flagged in a code comment and left raw.
  - *Given* a representative comparison such as `node.kind() == "return_statement"`, *When* migrated, *Then* it reads `GoKind::of(node) == GoKind::ReturnStatement`.
- `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo build --workspace`, `cargo test --workspace` all pass.
- Backtest: `kibitzer check backtest go-error-context` + `kibitzer check native go-error-context <file>` over the Go corpus produce identical finding counts to pre-migration. Cheap, single-checker.
  - *Given* a pre-migration baseline against a sample of `docs/backtest-repos.md`'s Go corpus, *When* re-run post-migration, *Then* finding counts match exactly.
**Files**: `src/checkers/go_error_context.rs`

##### Task 2.1.2a: Capture pre-migration backtest baseline (~3 min)
- Confirm the baseline shows a nonzero finding count (or record an explicit, checked reason zero is legitimate) per the backtest sanity-check gate.
- Paste the baseline output (or finding-count summary) into this story's commit/PR description per the baseline-durability gate.
- Files: none

##### Task 2.1.2b: Enumerate all 20 comparisons, check each short/symbolic literal against `go.json`'s `named` field, migrate the named ones (~5 min, may need 2 tasks given count)
- Files: `src/checkers/go_error_context.rs`

##### Task 2.1.2c: Second pass — remaining comparisons + `matches!` conversion for any multi-arm boolean checks (~5 min)
- Files: `src/checkers/go_error_context.rs`

##### Task 2.1.2d: Run full CI gate + backtest, compare to baseline (~4 min)
- Files: none

#### Story 2.1.3: `go_bulk_fetch_linear_scan.rs` (14 comparisons, has a `match`) + `go_call_resolution.rs` (6 comparisons)
**As a** migration implementer, **I want** these 2 confirmed-mechanical Go files converted, **so that** the previously-unassessed newer checker gets the same protection as its siblings.
**Acceptance Criteria**:
- `go_bulk_fetch_linear_scan.rs`'s `match function.kind() { "identifier" => ..., "selector_expression" => ..., _ => None }` (L200) converts to a `match GoKind::of(function) { GoKind::Identifier => ..., GoKind::SelectorExpression => ..., _ => None }` — confirmed safe because, unlike `symbol_extract.rs::callee_text_for`, every arm here is Go-only (architecture.md §2).
  - *Given* `go_bulk_fetch_linear_scan.rs:200`'s match, *When* migrated, *Then* both arms resolve to `GoKind` variants and the function's return value is unchanged for both `identifier` and `selector_expression` inputs.
- All 13 other named-kind comparisons in `go_bulk_fetch_linear_scan.rs` and all 6 in `go_call_resolution.rs` convert to `GoKind`.
- `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo build --workspace`, `cargo test --workspace` all pass.
- Backtest: `kibitzer check backtest go-bulk-fetch-linear-scan` + real-world corpus, and the equivalent for `go-call-resolution` (or its containing checker's registered name — confirm via `kibitzer list_checks` first), both produce identical finding counts to pre-migration.
  - *Given* a pre-migration baseline for `go-bulk-fetch-linear-scan` against a Go corpus sample containing both an `identifier`-callee and a `selector_expression`-callee call site, *When* re-run post-migration, *Then* both call sites are still classified identically.
**Files**: `src/checkers/go_bulk_fetch_linear_scan.rs`, `src/go_call_resolution.rs`

##### Task 2.1.3a: Capture pre-migration backtest baselines for both checkers (~4 min)
- Confirm each baseline shows a nonzero finding count (or record an explicit, checked reason zero is legitimate) per the backtest sanity-check gate.
- Paste each baseline output (or finding-count summary) into this story's commit/PR description per the baseline-durability gate.
- Files: none

##### Task 2.1.3b: Migrate `go_bulk_fetch_linear_scan.rs`'s direct comparisons (13 of 14) (~5 min)
- Files: `src/checkers/go_bulk_fetch_linear_scan.rs`

##### Task 2.1.3c: Migrate `go_bulk_fetch_linear_scan.rs`'s `match function.kind()` block (~4 min)
- Files: `src/checkers/go_bulk_fetch_linear_scan.rs`

##### Task 2.1.3d: Migrate `go_call_resolution.rs` (~4 min)
- Files: `src/go_call_resolution.rs`

##### Task 2.1.3e: Run full CI gate + both backtests, compare to baselines (~5 min)
- Files: none

### Epic 2.2: Java checkers batch

#### Story 2.2.1: `java_error_context.rs`, `java_ignored_error.rs`, `java_lost_exception_cause.rs`, `java_swallowed_interrupt.rs`
**As a** migration implementer, **I want** all 4 Java-only checker files converted to `JavaKind` in one story, **so that** the Java side of the migration follows the same reference idiom already established for Go (stack.md §4's `java_ignored_error.rs` example).
**Acceptance Criteria**:
- Every named-kind comparison across the 4 files converts to `JavaKind::of(...)`/`matches!(...)`.
  - *Given* `java_ignored_error.rs`'s `fn is_comment(node: Node) -> bool { matches!(node.kind(), "line_comment" | "block_comment") }`, *When* migrated, *Then* it reads `matches!(JavaKind::of(node), JavaKind::LineComment | JavaKind::BlockComment)`.
  - *Given* `java_ignored_error.rs`'s `if node.kind() != "catch_clause" { return; }`, *When* migrated, *Then* it reads `if JavaKind::of(node) != JavaKind::CatchClause { return; }`.
- `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings` (specifically confirm no `match_like_matches_macro` warnings from any of the 4 files), `cargo build --workspace`, `cargo test --workspace` all pass.
- Backtest: `kibitzer check backtest <name>` + real-world corpus (Java slice, e.g. `apache/cassandra`) for each of the 4 checkers — all produce identical finding counts to pre-migration. Cheap, single-checker each; `java_lost_exception_cause.rs`'s `enclosing_catch_name` shares `tree_walk.rs`'s `walk_preorder`, but per that file's disposition row, `walk_preorder`'s production logic is unaffected, so no additional cross-checker re-run is needed.
  - *Given* a pre-migration baseline for `java-swallowed-interrupt` against `apache/cassandra`'s Java files, *When* re-run post-migration, *Then* the finding count is identical.
**Files**: `src/checkers/java_error_context.rs`, `src/checkers/java_ignored_error.rs`, `src/checkers/java_lost_exception_cause.rs`, `src/checkers/java_swallowed_interrupt.rs`

##### Task 2.2.1a: Capture pre-migration backtest baselines for all 4 checkers (~5 min)
- Confirm each baseline shows a nonzero finding count (or record an explicit, checked reason zero is legitimate) per the backtest sanity-check gate.
- Paste each baseline output (or finding-count summary) into this story's commit/PR description per the baseline-durability gate.
- Files: none

##### Task 2.2.1b: Migrate `java_ignored_error.rs` (3 comparisons) (~3 min)
- Files: `src/checkers/java_ignored_error.rs`

##### Task 2.2.1c: Migrate `java_swallowed_interrupt.rs` (6 comparisons) (~4 min)
- Files: `src/checkers/java_swallowed_interrupt.rs`

##### Task 2.2.1d: Migrate `java_lost_exception_cause.rs` (7 comparisons) (~4 min)
- Files: `src/checkers/java_lost_exception_cause.rs`

##### Task 2.2.1e: Migrate `java_error_context.rs` (8 comparisons) (~4 min)
- Files: `src/checkers/java_error_context.rs`

##### Task 2.2.1f: Run full CI gate + all 4 backtests, compare to baselines (~5 min)
- Files: none

### Epic 2.3: Cross-cutting mechanical files

#### Story 2.3.1: No-op confirmation — `dedup.rs`, `plugin.rs`
Duplicate of Story 1.1.1's scope — **do not re-do this story**; it is listed here only so the file-count/commit-count reconciles against the full 23-file scope list. Story 1.1.1 already closes both files. No additional commit needed for Phase 2.

#### Story 2.3.2: `complexity.rs` + `complexity_tests.rs` (with an explicit seam exclusion)
**As a** migration implementer, **I want** `complexity.rs`'s 6 non-`function_kinds` comparisons and `complexity_tests.rs`'s 3 comparisons converted to `GoKind`, **so that** most of this file's typo risk is closed, while leaving the 2 sites that read `rules.rs`'s seam field untouched.
**Acceptance Criteria**:
- `complexity.rs:206,211(x5),227,240,246,250` (the `count_decision_points`/`is_short_circuit`/`is_run_subtest_closure` comparisons) convert to `GoKind`, **except** the `"&&" | "||"` arm inside `is_short_circuit` (L227), which stays raw because `go.json` marks both as `"named": false`.
  - *Given* `count_decision_points`'s `match node.kind() { "if_statement" | "for_statement" | ... => 1, "binary_expression" => ..., _ => 0 }` (L211-215), *When* migrated, *Then* it reads `match GoKind::of(node) { GoKind::IfStatement | GoKind::ForStatement | GoKind::ExpressionCase | GoKind::TypeCase | GoKind::CommunicationCase => 1, GoKind::BinaryExpression => usize::from(is_short_circuit(node)), _ => 0 }`.
  - *Given* `is_short_circuit`'s `matches!(op.kind(), "&&" | "||")` (L227), *When* reviewed, *Then* it is **left unchanged** with a one-line comment noting `&&`/`||` are anonymous tokens (`"named": false` in `go.json`) with no `GoKind` variant.
- `complexity.rs:111,141`'s `function_kinds.contains(&node.kind())` sites are **left unchanged**, with a one-line comment noting they read `rules.rs::lang_config(Language::Go).function_kinds`, a seamed raw-string field per this migration's scoping decision (ADR-001) — not a gap, an intentional exclusion.
  - *Given* `collect_total_complexity`'s `if function_kinds.contains(&node.kind()) { ... }` (L111), *When* reviewed, *Then* it remains exactly as-is, with a comment referencing `rules.rs`'s seam.
- `complexity_tests.rs:68`'s `n.kind() == "function_declaration"` converts to `GoKind::of(n) == GoKind::FunctionDeclaration`.
- `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo build --workspace`, `cargo test --workspace` all pass.
- Backtest: `kibitzer check backtest complexity` + real-world Go corpus produces identical finding counts to pre-migration, including for functions whose complexity depends on the still-raw `function_kinds` check (confirming the deliberate non-change didn't accidentally alter behavior).
  - *Given* a pre-migration baseline against a Go file with nested `if`/`for`/`switch` statements, *When* re-run post-migration, *Then* the computed complexity score for every function is identical.
**Files**: `src/checkers/complexity.rs`, `src/checkers/complexity_tests.rs`

##### Task 2.3.2a: Capture pre-migration backtest baseline (~3 min)
- Confirm the baseline shows a nonzero finding count (or record an explicit, checked reason zero is legitimate) per the backtest sanity-check gate.
- Paste the baseline output (or finding-count summary) into this story's commit/PR description per the baseline-durability gate.
- Files: none

##### Task 2.3.2b: Migrate `count_decision_points`'s match block and `is_run_subtest_closure`'s comparisons, leave `&&`/`||` raw with a comment (~5 min)
- Files: `src/checkers/complexity.rs`

##### Task 2.3.2c: Add seam-exclusion comments to the 2 `function_kinds.contains` sites (no logic change) (~2 min)
- Files: `src/checkers/complexity.rs`

##### Task 2.3.2d: Migrate `complexity_tests.rs:68` (~2 min)
- Files: `src/checkers/complexity_tests.rs`

##### Task 2.3.2e: Run full CI gate + backtest, compare to baseline (~4 min)
- Files: none

#### Story 2.3.3: `primitive_obsession.rs`, `isp_fat_interface.rs`, `tree_walk.rs`
**As a** migration implementer, **I want** these 3 small Go-scoped files converted, **so that** their kind checks (including `tree_walk.rs`'s test-only assertions) get compile-time protection.
**Acceptance Criteria**:
- `primitive_obsession.rs`'s 3 comparisons (`parameter_list`, `parameter_declaration`, `type_identifier`) and `isp_fat_interface.rs`'s 5 comparisons (`selector_expression`, `identifier`, `interface_type`, `method_elem`, `type_spec`) convert to `GoKind`.
  - *Given* `primitive_obsession.rs:166`'s `if ty.kind() == "type_identifier" { ... }`, *When* migrated, *Then* it reads `if GoKind::of(ty) == GoKind::TypeIdentifier { ... }`.
- `tree_walk.rs`'s 2 equality checks inside `#[cfg(test)] mod tests` (lines 75, 78) convert to `GoKind`; line 61's `.kind().to_string()` (Pattern F, no equality) is left as-is since there is nothing to compare.
  - *Given* `tree_walk.rs`'s test `if n.kind() == "function_declaration" { return false; }`, *When* migrated, *Then* it reads `if GoKind::of(n) == GoKind::FunctionDeclaration { return false; }`, and `walk_preorder`'s own production code (lines 26-42) is untouched.
- `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo build --workspace`, `cargo test --workspace` all pass.
- Backtest: `kibitzer check backtest primitive-obsession` and `kibitzer check backtest isp-fat-interface` + real-world Go corpus produce identical finding counts to pre-migration. `tree_walk.rs` needs **no** cross-checker backtest re-run beyond `cargo test tree_walk::` per its disposition row (production logic unchanged).
  - *Given* a pre-migration baseline for `isp-fat-interface` against a Go file with a >5-method interface, *When* re-run post-migration, *Then* the finding count is identical.
**Files**: `src/checkers/primitive_obsession.rs`, `src/isp_fat_interface.rs`, `src/tree_walk.rs`

##### Task 2.3.3a: Capture pre-migration backtest baselines for `primitive-obsession` and `isp-fat-interface` (~4 min)
- Confirm each baseline shows a nonzero finding count (or record an explicit, checked reason zero is legitimate) per the backtest sanity-check gate.
- Paste each baseline output (or finding-count summary) into this story's commit/PR description per the baseline-durability gate.
- Files: none

##### Task 2.3.3b: Migrate `primitive_obsession.rs` (~3 min)
- Files: `src/checkers/primitive_obsession.rs`

##### Task 2.3.3c: Migrate `isp_fat_interface.rs` (~4 min)
- Files: `src/isp_fat_interface.rs`

##### Task 2.3.3d: Migrate `tree_walk.rs`'s test module (~2 min)
- Files: `src/tree_walk.rs`

##### Task 2.3.3e: Run full CI gate + both backtests, compare to baselines (~5 min)
- Files: none

#### Story 2.3.4: `god_class.rs` (11 comparisons)
**As a** migration implementer, **I want** this file's Go kind comparisons converted, **so that** its detection logic gets the same compile-time protection as the other Go checkers.
**Acceptance Criteria**:
- All 11 comparisons (`method_declaration`, `function_declaration`, `parameter_declaration`, `expression_list`, `identifier`, `composite_literal`, `selector_expression`, `call_expression`) convert to `GoKind`.
  - *Given* `god_class.rs:212`'s `matches!(node.kind(), "method_declaration" | "function_declaration")`, *When* migrated, *Then* it reads `matches!(GoKind::of(node), GoKind::MethodDeclaration | GoKind::FunctionDeclaration)`.
- `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo build --workspace`, `cargo test --workspace` all pass.
- Backtest: `kibitzer check backtest god-class` + real-world Go corpus produces identical finding counts to pre-migration.
  - *Given* a pre-migration baseline against a Go file with a struct exceeding the god-class method/field threshold, *When* re-run post-migration, *Then* the same struct is flagged with the same field/method counts.
**Files**: `src/god_class.rs`

##### Task 2.3.4a: Capture pre-migration backtest baseline (~3 min)
- Confirm the baseline shows a nonzero finding count (or record an explicit, checked reason zero is legitimate) per the backtest sanity-check gate.
- Paste the baseline output (or finding-count summary) into this story's commit/PR description per the baseline-durability gate.
- Files: none

##### Task 2.3.4b: Migrate lines 212, 304, 322 (match), 338-345 (~5 min)
- Files: `src/god_class.rs`

##### Task 2.3.4c: Migrate lines 371 (match), 451-473 (~5 min)
- Files: `src/god_class.rs`

##### Task 2.3.4d: Run full CI gate + backtest, compare to baseline (~4 min)
- Files: none

---

## Phase 3: High-risk files (4 files, 3 independent epics)

**Gating note**: Story 3.1.1 → 3.1.2 and Story 3.2.1 → 3.2.2 are each a gated pair, not just a dependency shown in the diagram below. Story 3.1.2 must not be dispatched to a worker (parallel or otherwise) until Story 3.1.1 has merged; the same applies to 3.2.2 relative to 3.2.1. Both stories in each pair touch the same file, and dispatching them concurrently would produce two conflicting diffs.

### Epic 3.1: `src/checkers/rules.rs`
**Goal**: Migrate the mechanical majority (~26 of 34 sites) directly; isolate the 6 cross-grammar engine functions and the one substring check on the existing seam, per ADR-001.

#### Story 3.1.1: Migrate `rules.rs`'s single-language and field-type comparisons
**As a** migration implementer, **I want** every one-grammar-only comparison in `rules.rs` converted, **so that** the file's mechanical majority gets compile-time protection without touching its cross-grammar engine.
**Acceptance Criteria**:
- The 8 per-language helper functions (`go_bool_params`, `ts_js_bool_params`, `py_bool_params`, `java_bool_params`, `kotlin_bool_params`, `rust_bool_params`, `go_panic_detector`, `rust_panic_detector`, `go_statement_container`, `kotlin_body`, `kotlin_params`, `rust_unwrap_statement`, `go_param_identifier_count`, `js_ts_param_count`, `py_param_count`, `kotlin_param_count`, `rust_param_count`) each convert to their own grammar's `<Lang>Kind` — no shared type needed since each function only ever receives that grammar's nodes.
  - *Given* `go_bool_params`'s field-type check `ty.kind() != "type_identifier" || ty.utf8_text(src) != Ok("bool")` (L235), *When* migrated, *Then* it reads `GoKind::of(ty) != GoKind::TypeIdentifier || ty.utf8_text(src) != Ok("bool")`.
  - *Given* `java_bool_params`'s dual-kind check `ty.kind() == "boolean_type" || (ty.kind() == "type_identifier" && ...)` (L320-321), *When* migrated, *Then* it reads `JavaKind::of(ty) == JavaKind::BooleanType || (JavaKind::of(ty) == JavaKind::TypeIdentifier && ...)`.
- `collect_identifiers`'s `node.kind() == "identifier"` (L899) — since this function is itself called generically across all 8 grammars, this specific comparison is **added to the seam list** (Story 3.1.2), not migrated here, because `GoKind::Identifier` and `RustKind::Identifier` are distinct Rust types even though both grammars spell the kind `"identifier"`.
- `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo build --workspace`, `cargo test --workspace` all pass.
- Backtest: run `kibitzer check backtest <name>` + real-world corpus for **every** checker that calls into `rules.rs` (its default-checks catalog entries per kibitzer's `CLAUDE.md`: `complexity`, and any `contains-check`/`unreachable-code`/`flag-argument`-family checks registered from this file — confirm the exact list via `kibitzer list_checks` before running). This is a **multi-checker, shared-infra backtest**, not a single-checker one, per pitfalls.md §4.
  - *Given* a pre-migration baseline for every `rules.rs`-backed checker captured across at least 2 languages' backtest-repos corpora (e.g. Go + Python, to specifically exercise the Python `if_expression`-vs-`if_statement` risk named in pitfalls.md §5), *When* re-run post-migration, *Then* every checker's finding count is identical, with particular attention to Python nesting-depth findings (the concrete false-confidence risk this migration must not reintroduce).
**Files**: `src/checkers/rules.rs`

##### Task 3.1.1a: Capture pre-migration backtest baselines for every `rules.rs`-backed checker, across Go + Python at minimum (~8 min)
- Confirm each baseline shows a nonzero finding count (or record an explicit, checked reason zero is legitimate) per the backtest sanity-check gate.
- Paste each baseline output (or finding-count summary) into this story's commit/PR description per the baseline-durability gate — this is a multi-checker baseline set that Task 4.1.1d will need to retrieve.
- Files: none

##### Task 3.1.1b: Migrate Go helper functions (`go_bool_params`, `go_panic_detector`, `go_statement_container`, `go_param_identifier_count`) (~5 min)
- Files: `src/checkers/rules.rs`

##### Task 3.1.1c: Migrate TS/JS + Python helper functions (`ts_js_bool_params`, `js_ts_param_count`, `py_bool_params`, `py_param_count`) (~5 min)
- Files: `src/checkers/rules.rs`

##### Task 3.1.1d: Migrate Java + Kotlin helper functions (`java_bool_params`, `kotlin_bool_params`, `kotlin_body`, `kotlin_params`, `kotlin_param_count`) (~5 min)
- Files: `src/checkers/rules.rs`

##### Task 3.1.1e: Migrate Rust helper functions (`rust_bool_params`, `rust_panic_detector`, `rust_unwrap_statement`, `rust_param_count`) (~4 min)
- Files: `src/checkers/rules.rs`

##### Task 3.1.1f: Run full CI gate + every `rules.rs`-backed checker's backtest, compare to baselines (~10 min)
- Files: none

#### Story 3.1.2: Isolate `rules.rs`'s 6 cross-grammar engine functions + the substring check, on the seam
**Gating**: this story must not start (and must not be dispatched to a worker) until Story 3.1.1 has merged — both stories touch `rules.rs`, and running them concurrently would produce two conflicting diffs.
**As a** migration implementer, **I want** the 6 generic engine functions and `rules.rs:784`'s `.contains("comment")` check explicitly marked as seamed, **so that** their intentional exclusion from this migration is documented rather than looking like an oversight.
**Acceptance Criteria**:
- Every seamed site in this story uses the standardized banner comment format `// SEAM(typed-node-kind-migration): <one-line reason> — see ADR-001.` — a fixed, greppable marker (not free-form prose) so Phase 4's completeness sweep (Story 4.1.1) can mechanically distinguish "intentionally seamed" from "missed." Story 4.1.2 later turns this into a standing CI check that a future single-line edit to one of these seamed lines can't drift out of sync with its banner (banner-count vs. pinned-seam-line-count) — no action needed in this story beyond placing the banners correctly.
- `walk_declarations` (L745), `walk_blocks` (L758), `check_block_for_unreachable` (L774), `collect_condition_identifiers` (L881), `max_nesting_depth` (L913), `walk_if_chain` (L960), and `collect_identifiers`'s L899 site (deferred from Story 3.1.1) are **left with their existing `cfg.if_kind`/`cfg.block_kind`/etc. raw-string comparisons unchanged**, each annotated with a one-line comment: `// SEAM(typed-node-kind-migration): cfg fields stay &str, cross-grammar-shared, not migrated this pass — see ADR-001.`
  - *Given* `rules.rs:759`'s `if node.kind() == cfg.block_kind { ... }`, *When* reviewed, *Then* the line itself is byte-identical to pre-migration, with the seam comment added immediately above the function signature (not per-line, to avoid 6x comment noise).
- `rules.rs:784`'s `if stmt.kind().contains("comment") { ... }` is left unchanged with the same seam-comment treatment, noting it is a substring check with no single-variant equivalent.
  - *Given* `rules.rs:784`, *When* reviewed, *Then* the line is unchanged and a comment above it explains the substring check cannot become a single `<Lang>Kind` equality.
- `LangRuleConfig`'s fields (`if_kind`, `block_kind`, `nesting_kinds`, `chain_kinds`, `else_wrapper_kinds`, `terminal_kinds`, `function_kinds`, `ternary_kind`) remain `&'static str`/`&'static [&'static str]` — unchanged.
- `node_kind_literals_are_valid_for_their_grammar` (the existing runtime validator) is confirmed **still present and still passing** — it remains non-redundant since these literals still exist as strings (requirements.md's Non-Goals: only remove it if the file's migration naturally removes the literals it validates, which does not happen here).
  - *Given* `rules.rs`'s test module, *When* `cargo test rules::` runs post-migration, *Then* `node_kind_literals_are_valid_for_their_grammar` still exists, is unmodified, and passes.
- `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo build --workspace`, `cargo test --workspace` all pass.
- Backtest: since no comparison logic changed in this story (comment-only diff), a full `cargo test rules::` pass plus a spot-check backtest run (not the full multi-checker sweep from Story 3.1.1) is sufficient — record this explicitly as the "no logic change" case per pitfalls.md's cost-scaling guidance.
  - *Given* Story 3.1.1's post-migration backtest baseline, *When* re-run after this comment-only story, *Then* finding counts are unchanged (expected, since no comparison logic moved).
**Files**: `src/checkers/rules.rs`

##### Task 3.1.2a: Add seam comments above the 6 engine functions (~4 min)
- Files: `src/checkers/rules.rs`

##### Task 3.1.2b: Add seam comment above `rules.rs:784`'s substring check (~2 min)
- Files: `src/checkers/rules.rs`

##### Task 3.1.2c: Confirm `node_kind_literals_are_valid_for_their_grammar` still passes unmodified; run full CI gate + spot-check backtest (~5 min)
- Files: none

### Epic 3.2: `src/symbol_extract.rs`

#### Story 3.2.1: Migrate `symbol_extract.rs`'s single-language and per-language-dispatch comparisons
**As a** migration implementer, **I want** the ~25 mechanical comparisons converted, **so that** the Go-only field-access graph and the per-language `classify_node` branches get compile-time protection.
**Acceptance Criteria**:
- The Go-only functions (`go_receiver_type_name`, `go_type_declaration_symbols`, `go_struct_fields`, `go_receiver_var_name`, `parameter_list_names`, `shadow_candidate_names`, `direct_identifier_names`, `range_clause_declared_names`, `selector_access_kind`, `selector_is_call_target`) convert to `GoKind`.
- `classify_node`'s already-per-language `if language == Language::Go { ... }`-style branches (L418-453) convert each branch's `.kind()` checks to that branch's already-known `<Lang>Kind` (e.g. the Go branch uses `GoKind`, a Java branch would use `JavaKind`).
  - *Given* a `classify_node` branch gated by `language == Language::Go`, *When* migrated, *Then* every `.kind()` comparison inside that branch becomes `GoKind::of(...)`, since the branch guard already guarantees the grammar.
- The 3 explicit non-goal sites (`find_child_by_kind(node, "modifiers"|"public"|"visibility_modifier"|"private"|"internal"|"interface")` in `java_is_exported`/`kotlin_is_exported`/`kotlin_is_interface`, L100-154) are confirmed against `codegen/node-types/{java,kotlin}.json` and left as raw strings with a comment — `"public"` is `"named": false` in `java.json`; `"public"`/`"private"`/`"internal"`/`"interface"` are all `"named": false` in `kotlin.json`.
  - *Given* `kotlin_is_interface`'s scan for a literal `"interface"` child token, *When* reviewed, *Then* the comparison is left unchanged with a comment citing `kotlin.json`'s `"named": false` entry for `"interface"`.
- `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo build --workspace`, `cargo test --workspace` all pass.
- Backtest: run every checker that calls into `symbol_extract.rs` (confirm via `kibitzer list_checks` and a `grep -rl symbol_extract:: src/checkers/`) across at least Go, Java, and Kotlin backtest-repos corpora — a **multi-checker, shared-infra backtest**.
  - *Given* a pre-migration baseline covering a Kotlin file with both a `class` and an `interface` declaration, *When* re-run post-migration, *Then* both are still classified identically (this specifically re-validates that the non-goal sites' *raw-string* logic, unchanged, still discriminates class-vs-interface correctly).
**Files**: `src/symbol_extract.rs`

##### Task 3.2.1a: Capture pre-migration backtest baselines for every `symbol_extract.rs`-consuming checker across Go/Java/Kotlin (~8 min)
- Confirm each baseline shows a nonzero finding count (or record an explicit, checked reason zero is legitimate) per the backtest sanity-check gate.
- Paste each baseline output (or finding-count summary) into this story's commit/PR description per the baseline-durability gate.
- Files: none

##### Task 3.2.1b: Migrate the Go-only field-access-graph functions (~5 min)
- Files: `src/symbol_extract.rs`

##### Task 3.2.1c: Migrate `classify_node`'s per-language branches (~5 min)
- Files: `src/symbol_extract.rs`

##### Task 3.2.1d: Verify the 3 non-goal sites against `java.json`/`kotlin.json` and add comments (no logic change) (~4 min)
- Files: `src/symbol_extract.rs`

##### Task 3.2.1e: Run full CI gate + every consuming checker's backtest, compare to baselines (~10 min)
- Files: none

#### Story 3.2.2: Isolate `symbol_extract.rs`'s 3 cross-grammar-shared sites on the seam
**Gating**: this story must not start (and must not be dispatched to a worker) until Story 3.2.1 has merged — both stories touch `symbol_extract.rs`, and running them concurrently would produce two conflicting diffs.
**As a** migration implementer, **I want** `enclosing_kind_name`, `walk_calls`'s `function_kinds.contains` check, and `callee_text_for` explicitly marked as seamed, **so that** the hardest snippet in the whole migration (`callee_text_for`'s two-grammar match arm) is documented as an intentional Option-C exclusion.
**Acceptance Criteria**:
- Every seamed site in this story uses the same standardized banner comment format as Story 3.1.2: `// SEAM(typed-node-kind-migration): <one-line reason> — see ADR-001.` As with Story 3.1.2, Story 4.1.2 later enforces banner-count-vs-seam-line-count as a standing CI check — no action needed here beyond placing the banners correctly.
- `enclosing_kind_name(node, source, target_kinds: &[&str])` (L333) keeps its `&[&str]` parameter and `target_kinds.contains(&n.kind())` body unchanged; each of its per-language call sites (`&["class_declaration"]` for TS/JS/Kotlin, `&["class_definition"]` for Python, `JAVA_TYPE_KINDS` for Java) is annotated with a seam comment at the function definition (not per call site).
  - *Given* `enclosing_kind_name`'s body, *When* reviewed post-migration, *Then* it is byte-identical to pre-migration except for one added doc comment referencing ADR-001.
- `walk_calls`'s `ctx.cfg.function_kinds.contains(&node.kind())` (L590-614) is left unchanged with the same seam-comment treatment.
- `callee_text_for` (L565-573)'s `match function.kind() { "identifier" | "selector_expression" | "member_expression" => ..., _ => None }` is left unchanged, with a comment explicitly naming this as the single hardest snippet in the migration (per architecture.md §1) — a match arm unioning Go's `selector_expression` and JS/TS's `member_expression` vocabularies, deliberately excluded per Option C.
  - *Given* `callee_text_for`, *When* reviewed, *Then* the match is unchanged and the comment explains why (union of two grammars' kind vocabularies in one arm, no single enum can express it without a shared trait, which is out of scope).
- `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo build --workspace`, `cargo test --workspace` all pass.
- Backtest: comment-only diff — `cargo test symbol_extract::` plus a spot-check backtest run for the call-graph-dependent checkers (confirm via `kibitzer list_checks`) is sufficient, consistent with Story 3.1.2's "no logic change" treatment.
  - *Given* Story 3.2.1's post-migration backtest baseline, *When* re-run after this comment-only story, *Then* finding counts are unchanged.
**Files**: `src/symbol_extract.rs`

##### Task 3.2.2a: Add seam comment to `enclosing_kind_name` (~2 min)
- Files: `src/symbol_extract.rs`

##### Task 3.2.2b: Add seam comment to `walk_calls`'s `function_kinds.contains` site (~2 min)
- Files: `src/symbol_extract.rs`

##### Task 3.2.2c: Add seam comment to `callee_text_for` (~2 min)
- Files: `src/symbol_extract.rs`

##### Task 3.2.2d: Run full CI gate + spot-check backtest for call-graph-dependent checkers (~5 min)
- Files: none

### Epic 3.3: `import_graph.rs` and `declarations.rs` (Extend as-is)

#### Story 3.3.1: Migrate `import_graph.rs`
**As a** migration implementer, **I want** every genuinely-per-language comparison in `import_graph.rs` converted, **so that** this file — despite its superficial resemblance to `rules.rs` — gets full compile-time protection since it has no actual shared-function pattern.
**Acceptance Criteria**:
- Every per-language function (`collect_go_imports`, `collect_go_import_specs`, `go_import_spec`, `collect_js_imports`, `string_fragment_text`, `collect_java_imports`, `kotlin_package_identity`, `collect_kotlin_imports`, `collect_python_imports`, the Rust `use_declaration` handling) converts its `.kind()` comparisons to that function's single grammar's `<Lang>Kind`.
  - *Given* `collect_python_imports`'s `match node.kind() { "import_statement" => ..., "import_from_statement" => ..., _ => {} }` (L995), *When* migrated, *Then* it reads `match PythonKind::of(node) { PythonKind::ImportStatement => ..., PythonKind::ImportFromStatement => ..., _ => {} }`.
- Java's wildcard-import detection (`c.kind() == "asterisk"`, L633-636) converts to `JavaKind::Asterisk` (confirmed `"asterisk"` is `"named": true` in `java.json`); Kotlin's equivalent (`c.kind() == "*"`, L706-710) stays raw (confirmed `"*"` is `"named": false` in `kotlin.json`) — this pairing gets an explicit comment noting the two call sites look structurally parallel but must be treated differently.
  - *Given* Java's `c.kind() == "asterisk"` and Kotlin's `c.kind() == "*"` side by side, *When* migrated, *Then* only the Java site changes (to `JavaKind::of(c) == JavaKind::Asterisk`); the Kotlin site is unchanged with a comment cross-referencing the Java site and explaining why they differ.
- Java's static-import detection (anonymous `"static"` keyword scan, L616-624) stays raw (confirmed anonymous in `java.json`).
- `QualifiedImportLangConfig`'s `package_decl_kind`/`import_stmt_kind` fields are left as plain `&'static str` (per the Unresolved Question — default to leave-as-is unless resolved otherwise before this story starts).
- `import_graph.rs`'s inline `#[cfg(test)]` assertion `assert_eq!(name_field.kind(), "aliased_import")` (L2229) converts to `assert_eq!(PythonKind::of(name_field), PythonKind::AliasedImport)` (or the equivalent `KotlinKind`/whichever grammar that test targets — confirm from context) since it's a direct-literal equality inside an in-scope file.
- `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo build --workspace`, `cargo test --workspace` all pass.
- Backtest: run every checker consuming `import_graph.rs` (confirm via `kibitzer list_checks` / `grep -rl import_graph:: src/checkers/`) across Go, Java, Kotlin, and Python backtest-repos corpora.
  - *Given* a pre-migration baseline against a Kotlin file with a `import a.b.*` wildcard import, *When* re-run post-migration, *Then* the wildcard is still detected identically (validating the Java/Kotlin asymmetric-treatment comment didn't accidentally change Kotlin's behavior).
**Files**: `src/import_graph.rs`

##### Task 3.3.1a: Capture pre-migration backtest baselines for every `import_graph.rs`-consuming checker across Go/Java/Kotlin/Python (~8 min)
- Confirm each baseline shows a nonzero finding count (or record an explicit, checked reason zero is legitimate) per the backtest sanity-check gate.
- Paste each baseline output (or finding-count summary) into this story's commit/PR description per the baseline-durability gate.
- Files: none

##### Task 3.3.1b: Migrate Go and JS import-collection functions (~5 min)
- Files: `src/import_graph.rs`

##### Task 3.3.1c: Migrate Java and Kotlin import-collection functions, including the asterisk/`"*"` asymmetric pair and the anonymous `"static"` non-goal (~6 min)
- Files: `src/import_graph.rs`

##### Task 3.3.1d: Migrate Python `match` block and Rust `use_declaration` handling (~5 min)
- Files: `src/import_graph.rs`

##### Task 3.3.1e: Migrate the inline test module's `assert_eq!` literal (L2229) (~2 min)
- Files: `src/import_graph.rs`

##### Task 3.3.1f: Run full CI gate + every consuming checker's backtest, compare to baselines (~10 min)
- Files: none

#### Story 3.3.2: Migrate `declarations.rs`
**As a** migration implementer, **I want** every `collect_<lang>_declarations` function's comparisons converted, **so that** this file — the simplest of the four flagged files — gets full compile-time protection, with the Kotlin class/interface non-goal explicitly preserved.
**Acceptance Criteria**:
- `collect_go_declarations` (L161), `collect_js_ts_declarations` (L251), `collect_java_declarations` (L336), `classify_python_definition`/`collect_python_declarations` (L512/553) each convert their `match node.kind() { ... }` blocks to that function's `<Lang>Kind`.
  - *Given* `collect_java_declarations`'s match over `"class_declaration"`/`"interface_declaration"`/etc., *When* migrated, *Then* it reads `match JavaKind::of(node) { JavaKind::ClassDeclaration => ..., JavaKind::InterfaceDeclaration => ..., ... }`.
- `collect_kotlin_declarations` (L429) converts its `match node.kind() { "class_declaration" => ..., ... }` outer match to `KotlinKind`, but its inner keyword-child scan (`c.kind() == "class" || c.kind() == "interface"`, L430-436) stays raw, confirmed via `kotlin.json` that both `"class"` and `"interface"` are `"named": false` — a comment already exists nearby (per architecture.md §1) and should be extended to reference this migration/ADR-001 explicitly.
  - *Given* `collect_kotlin_declarations`'s outer `match node.kind() { "class_declaration" => { ... } }`, *When* migrated, *Then* the outer match arm becomes `KotlinKind::ClassDeclaration`, while the inner `.find(|c| c.kind() == "class" || c.kind() == "interface")` line is byte-identical to pre-migration.
- The inline `#[cfg(test)] mod tests`' `.kind() ==` assertions are handled per-case: L741 (`object_node.kind(), "object_declaration"`, a genuinely named Kotlin kind) converts to `KotlinKind::of(object_node) == KotlinKind::ObjectDeclaration`; L732, L750, L756 (asserting on the same anonymous `"class"`/`"interface"` tokens) stay raw with the same non-goal reasoning.
- `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo build --workspace`, `cargo test --workspace` all pass.
- Backtest: run every checker consuming `declarations.rs` across Go, TS/JS, Java, Kotlin, and Python backtest-repos corpora.
  - *Given* a pre-migration baseline against a Kotlin file containing both a plain `class` and an `interface` declaration under `class_declaration`, *When* re-run post-migration, *Then* both are still classified identically as `DeclKind::Class`/`DeclKind::Interface` respectively.
**Files**: `src/declarations.rs`

##### Task 3.3.2a: Capture pre-migration backtest baselines for every `declarations.rs`-consuming checker (~8 min)
- Confirm each baseline shows a nonzero finding count (or record an explicit, checked reason zero is legitimate) per the backtest sanity-check gate.
- Paste each baseline output (or finding-count summary) into this story's commit/PR description per the baseline-durability gate.
- Files: none

##### Task 3.3.2b: Migrate `collect_go_declarations` and `collect_js_ts_declarations` (~5 min)
- Files: `src/declarations.rs`

##### Task 3.3.2c: Migrate `collect_java_declarations` and `collect_kotlin_declarations`'s outer match (leave inner class/interface scan raw, extend its existing comment) (~5 min)
- Files: `src/declarations.rs`

##### Task 3.3.2d: Migrate `classify_python_definition`/`collect_python_declarations` (~4 min)
- Files: `src/declarations.rs`

##### Task 3.3.2e: Migrate the inline test module's genuinely-named assertions (e.g. L741), leave the anonymous-token ones raw (~3 min)
- Files: `src/declarations.rs`

##### Task 3.3.2f: Run full CI gate + every consuming checker's backtest, compare to baselines (~10 min)
- Files: none

---

## Phase 4: Completion signal

### Epic 4.1: Close out the migration
**Goal**: Narrow `node_kind.rs`'s blanket lint allow now that real call sites exist, run one final full-suite + full-backtest sweep across every touched checker to confirm the whole migration is behavior-neutral end to end, and turn the one-time completeness sweep into a standing CI gate so a future PR can't silently reintroduce a raw `.kind()` comparison.

#### Story 4.1.1: Narrow `node_kind.rs`'s `#![allow(...)]` and run the final full regression
**As a** migration implementer, **I want** the crate-wide dead-code/unused-import allow narrowed to only what's still genuinely unused, and every touched checker's full test + backtest suite re-run once more, **so that** the migration's own success signal (a real unused-variant/enum is now caught, not silenced) is verified, and no story's individual backtest run missed a cross-file interaction.
**Acceptance Criteria**:
- `src/node_kind.rs:13`'s `#![allow(dead_code, unused_imports)]` is removed entirely if `cargo build --workspace` succeeds without it; if any variant/enum is still genuinely unused (e.g. a language whose grammar has a named kind no checker references, such as an obscure `RustKind` variant), the allow is narrowed to a scoped `#[allow(dead_code)]` on just that item, with a comment naming which variant(s) remain unused and why.
  - *Given* `node_kind.rs`'s current file-level `#![allow(dead_code, unused_imports)]`, *When* removed and `cargo build --workspace` is re-run, *Then* it either succeeds cleanly (allow fully removed) or fails with specific `dead_code`/`unused_imports` warnings pointing at exact remaining unused items, which then each get a scoped, item-level `#[allow(dead_code)]` instead of the blanket one.
  - **Warning-count fallback**: run the removal as a dry-run early in this story (Task 4.1.1a) before committing to a scope. Given only ~140 of many hundreds of possible variants across 8 generated enums were ever migrated, a large warning count (dozens-to-hundreds, spread across most/all 8 enums) is an acceptable, budgeted outcome, not a blocker — in that case, apply one coarser per-enum `#[allow(dead_code)]` (not a per-variant allow, and not the original blanket file-level allow) for each enum with a large unused count, with a comment noting the count and that a full per-variant cleanup is an unbudgeted follow-up, not this story's merge gate.
- `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo build --workspace`, `cargo test --workspace` all pass with the narrowed/removed allow in place.
- Every checker touched by any Phase 2/3 story (the full list: `go-blank-imports`, `go-type-switch-density`, `go-table-driven-test`, `go-error-context`, `go-bulk-fetch-linear-scan`, `go-call-resolution` [or its containing checker], `java-error-context`, `java-ignored-error`, `java-lost-exception-cause`, `java-swallowed-interrupt`, `complexity`, `primitive-obsession`, `isp-fat-interface`, `god-class`, every `rules.rs`-backed checker, every `symbol_extract.rs`-consuming checker, every `import_graph.rs`-consuming checker, every `declarations.rs`-consuming checker, `go-ignored-error`) is re-run one final time via both `kibitzer check backtest <name>` and the real-world repo corpus, and every finding count matches that checker's very first pre-migration baseline (not just its own story's baseline, and not a freshly captured stand-in) — this is the cross-story integration check no individual story's backtest could catch alone.
  - *Given* the full set of pre-migration baselines captured across Stories 1.1.2 through 3.3.2, each retrievable from that story's commit/PR description per the baseline-durability gate, *When* every checker is re-run against the same corpus files one final time after all 23 files have landed, *Then* every finding count matches its original pre-migration baseline exactly, confirming no story's change had an unexpected cross-file interaction with another.
  - **Fail-closed clause**: if any prior story's original baseline cannot be located (its commit/PR description doesn't contain it, and the commit can't be checked out to re-derive it), Task 4.1.1d does **not** substitute a freshly captured "baseline" and proceed — it blocks this story's completion until that checker's original baseline is located, or its backtest is explicitly re-run from a documented earlier reference point (e.g. `git show <that story's commit>^:<file>` to reconstruct the pre-change state) and that re-derivation is itself recorded in this story's commit/PR description.
- **Migration-completeness sweep** (mechanical, catches a missed comparison that no backtest fixture happens to exercise): run `rg -n '\.kind\(\)\s*(==|!=)|\.kind\(\)\.contains\(|match\s+\S+\.kind\(\)' src/` (or an equivalent `ast-grep` query) against all 23 in-scope files. Every hit must be either (a) already converted to a `<Lang>Kind` comparison (so it shouldn't match this grep at all — confirming the grep itself has zero unexplained hits is the point), or (b) immediately adjacent to one of the documented seam/non-goal markers: a `// SEAM(typed-node-kind-migration): ...` banner (Story 3.1.2/3.2.2's 11 cross-grammar sites) or an inline comment citing the anonymous-token/non-goal reasoning from the Tech Debt Disposition table (e.g. `complexity.rs`'s `&&`/`||` arm, `declarations.rs`'s Kotlin class/interface scan, `import_graph.rs`'s Kotlin `"*"`/Java `"static"` sites, `symbol_extract.rs`'s 3 non-goal sites). Zero unexplained hits is required before the migration is declared complete; any unexplained hit is a missed site and must be fixed (converted or seam-annotated) before this story is marked done.
  - *Given* the sweep command run against all 23 files after every other story has merged, *When* its output is triaged line by line, *Then* every remaining hit maps 1:1 to a documented seam or non-goal comment, and the sweep is re-run once more after any fix to confirm zero unexplained hits remain.
- **Idiom-consistency skim** (Phase 2 ran ~7 stories in parallel; this confirms they converged): run `rg 'Kind::of\(' -A1 src/` across all touched files and manually skim that every hit follows the plan's chosen idiom (`Kind::of(node) == Kind::Variant`, or `matches!(Kind::of(node), Variant::A | Variant::B)` for multi-arm checks) rather than a divergent style (e.g. inconsistent use of `Kind::from_kind_str` where a live `Node` was in hand, or a hand-written `match` that should have been a `matches!`). This is a quick pass, not a full review — clippy already catches some drift (`match_like_matches_macro`); this catches the stylistic drift clippy doesn't.
  - *Given* the `rg 'Kind::of\(' -A1 src/` output across all Phase 2/3 touched files, *When* skimmed, *Then* no site uses a materially different idiom for an equivalent comparison shape than the rest of the codebase.
**Files**: `src/node_kind.rs`

##### Task 4.1.1a: Attempt to remove `#![allow(dead_code, unused_imports)]` entirely, run `cargo build --workspace` (~3 min)
- Files: `src/node_kind.rs`

##### Task 4.1.1b: For any remaining warnings, apply scoped item-level `#[allow(dead_code)]` with a naming comment (~4 min)
- Files: `src/node_kind.rs`

##### Task 4.1.1c: Run full CI gate (`cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo build --workspace && cargo test --workspace`) (~5 min)
- Files: none

##### Task 4.1.1d: Re-run every touched checker's transcript backtest + real-world corpus check, diff against each checker's original pre-migration baseline (~15 min, the largest single task in the plan by design — this is the final integration gate)
- Retrieve each checker's original pre-migration baseline from its story's commit/PR description (per the baseline-durability gate) — do not rely on `/tmp`, which will not have survived across the ~17 independent worker sessions.
- **Fails closed**: if a baseline can't be retrieved this way, do not substitute a freshly captured result and call it a match. Locate the baseline via `git show <story commit>^:<file>` re-derivation (recording that re-derivation in this story's commit/PR description) or block this task until it's found.
- Files: none

##### Task 4.1.1e: Run the migration-completeness sweep (`rg -n '\.kind\(\)\s*(==|!=)|\.kind\(\)\.contains\(|match\s+\S+\.kind\(\)' src/`) across all 23 files, triage every hit against the seam/non-goal comment list, fix any unexplained hit, re-run to confirm zero remain (~10 min)
- Files: none (read-only unless a missed site is found, in which case fix it in the relevant already-touched file)

##### Task 4.1.1f: Run the idiom-consistency skim (`rg 'Kind::of\(' -A1 src/`) across all touched files, confirm convergence on the plan's chosen idiom (~5 min)
- Files: none

#### Story 4.1.2: Land the migration-completeness sweep as a permanent CI gate
**Gating**: starts after Story 4.1.1 merges, so the gate is written and validated against the migration's final state (post-allow-narrowing), not a mid-migration snapshot.
**As a** migration implementer, **I want** Story 4.1.1's one-time completeness sweep turned into standing enforcement, **so that** a future contributor or agent can't silently reintroduce the exact typo-risk class (a new raw `.kind() == "literal"` comparison next to a converted one) this migration exists to close.
**Implementation choice — CI script, not a new kibitzer checker**: this lands as a plain CI script/step (a `rg`/`ast-grep` invocation plus a triage step, wired into the existing CI workflow) scoped to the 23 in-scope files — **not** a new native kibitzer check. Requirements.md's Out-of-Scope section explicitly rules out "building any new codegen/enum-generation mechanism," and a new kibitzer checker is new *checker* functionality (a durable, cross-repo, generically-applicable inspection with its own backtest/corpus obligations per kibitzer's own `CLAUDE.md`) — a different, heavier kind of thing than a one-repo, one-migration completeness grep. A CI script gate has no such obligations and is proportionate to what's actually needed: a mechanical presence/absence check scoped to this migration's 23 files, not a new generally-reusable check. State this reasoning in the script's own header comment so a future reader doesn't have to re-derive it.
**Acceptance Criteria**:
- A CI script (e.g. `scripts/check-node-kind-completeness.sh` or equivalent) runs `rg -n '\.kind\(\)\s*(==|!=)|\.kind\(\)\.contains\(|match\s+\S+\.kind\(\)'` against the same 23 in-scope files from Story 4.1.1's sweep, and fails (non-zero exit) if any hit is not immediately adjacent to a documented seam/non-goal comment (a `// SEAM(typed-node-kind-migration): ...` banner or an inline anonymous-token/non-goal comment per the Tech Debt Disposition table).
  - *Given* the script run against the migration's final state immediately after Story 4.1.1 lands, *When* it executes, *Then* it exits 0 (matching Task 4.1.1e's confirmed zero-unexplained-hits result).
  - *Given* a hypothetical future PR that adds a new unexplained `node.kind() == "some_literal"` comparison to one of the 23 files, *When* the script runs, *Then* it exits non-zero and identifies the exact file:line.
- The same script also enforces the seam-comment/seam-line invariant from Story 3.1.2/3.2.2 (the 11 pinned cross-grammar sites): it counts `// SEAM(typed-node-kind-migration): ...` banner occurrences against the count of pinned seam lines in `rules.rs`/`symbol_extract.rs` and fails if they diverge, so a future single-line edit to a seamed comparison that doesn't also touch its banner is caught (per pre-mortem #5's item 3 — closing the gap that a once-per-function banner wouldn't be visible in a scoped single-line diff).
  - *Given* a future edit that changes one of the 11 pinned seam lines without touching its banner comment, *When* the script runs, *Then* it fails, flagging the count mismatch.
- The script is wired into the repo's existing CI workflow (the same workflow that runs `cargo fmt --check`/`clippy`/`build`/`test`) as an additional required step, not a separate optional one.
- `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo build --workspace`, `cargo test --workspace` all still pass (this story adds a CI script and workflow wiring, not Rust source changes).
**Files**: a new CI script (e.g. `scripts/check-node-kind-completeness.sh`), the CI workflow file that invokes it.

##### Task 4.1.2a: Write the completeness-sweep + seam-count-invariant script, scoped to the 23 in-scope files (~8 min)
- Files: new CI script

##### Task 4.1.2b: Wire the script into the existing CI workflow as a required step; verify it passes on the current (clean) tree and fails on a deliberately reintroduced unexplained `.kind() ==` hit (~5 min)
- Files: CI workflow file
