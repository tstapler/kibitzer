# Architecture Review: replace-magic-literal
**Date**: 2026-09-22
**Verdict**: CONCERNS

## Constitution Check

No `docs/adr/ADR-000-architecture-constitution.md` exists in this repository — `docs/adr/`
is not a directory here at all (`docs/` holds flat files only: `accepting-findings.md`,
`backtesting.md`, `syntax-rules.md`, etc.; this feature's own ADR-001 lives under
`project_plans/replace-magic-literal/decisions/`, a different convention). No constitutional
hard constraints apply; all findings below come from the four lenses.

## Blockers

None — resolved in repair iteration 2.

The prior blocker (Lens 4, tech-debt disposition: `lang_config()`'s pre-existing 217-line
`long-function` violation being deepened by Epic 1.1 under an "Extend as-is" disposition) is
fixed in the current `plan.md`, verified against the live file:

- **Disposition table row added**: `plan.md`'s Tech Debt Disposition table now has a dedicated
  row for `src/rules.rs::lang_config()` with disposition **Refactor-first**, and the
  justification does not gloss over the wrong premise — it states outright that
  `research/architecture.md` §5's "no hotspot/churn evidence... flags `rules.rs`" claim "is
  false — confirmed wrong by this same command," and gives approximate post-refactor sizes
  (`lang_config()` ~20 lines, each extracted constructor ~25-45 lines) rather than an
  unquantified "doesn't strain."
- **Sequenced as a real task, before Epic 1.1**: new Epic 1.0 / Story 1.0.1 / Tasks 1.0.1a
  (extract the 8 match arms into named constructor functions, e.g. `go_lang_config()`) and
  1.0.1b (verify via `cargo test --lib` and `kibitzer run src --trigger batch` that the
  `long-function` finding no longer fires) precede Epic 1.1 in document order, and the
  Dependency Visualization diagram shows Epic 1.0's chain flowing into Epic 1.1 with an
  explicit arrow, not just a passing mention.
- **Epic 1.1 updated to target the new functions, not stale line ranges**: Tasks
  1.1.1c–1.1.1h (Go/TS/Python/Java/Kotlin/Rust) each now read "In `go_lang_config()`
  (extracted from `lang_config()`'s Go arm by Epic 1.0's Task 1.0.1a)" (and the per-language
  equivalents) rather than referencing offsets inside the old 217-line function. Task 1.1.1a
  (the two new `LangRuleConfig` struct fields) correctly still cites `src/rules.rs:143-144` —
  that's the struct definition, not `lang_config()`'s body, so it's unaffected by the
  extraction and not a stale reference.
- **No new numbering collision**: Epic 1.0 is additive (1.0 was previously unused; Epics
  1.1–4.2 are unchanged), and Story/Task numbering under it (1.0.1, 1.0.1a/b) follows the
  plan's existing `X.Y.Z` / lettered-subtask convention with no duplicate or skipped IDs
  anywhere in the document.

One residual soft gap, not blocker-severity: the second Tech Debt Disposition row
(`src/rules.rs`'s overall 2329-line file-size violation, kept "Extend as-is") still justifies
itself with unquantified language ("not a strain on it") rather than an explicit projected
total line count — the same style the original blocker objected to, just on the row it wasn't
about. Left as a Nitpick-level note below rather than reopening the blocker, since it doesn't
compound an existing violation the way the `lang_config()` row did.

## Concerns

- [ ] **Task 1.2.1e (`emit_literal_findings`)** — the numeric/string label is derived by
  "whether `text` starts with a quote character." This misclassifies Go's
  `raw_string_literal` (backtick-delimited, e.g. `` `hello` ``, listed in this same task's
  own `literal_kinds` value in Task 1.1.1c) and Rust's `raw_string_literal` forms (`r"..."`,
  `r#"..."#`, listed in Task 1.1.1h) as "numeric" in the finding message, since neither
  starts with `"`. Impact is limited to the message's cosmetic label (grouping/firing is
  still by raw text, unaffected), but it's an avoidable "parse, don't validate" gap: the
  node's actual kind is known at collection time (`walk_literals` already holds `node.kind()`
  when it pushes into `occurrences`) and is then discarded, only to be unreliably re-guessed
  later from raw text. **Remediation**: have `walk_literals` store the originating
  `node.kind()` (or a two-variant `LiteralKind` derived from it) alongside each occurrence,
  and have `emit_literal_findings` read that stored kind instead of pattern-matching the
  raw text.

- [ ] **Story 1.2.1 (`binding_finder` return type, `LiteralCollector.bound`)** — both are
  typed as anonymous 2-tuples (`Option<(String, Node)>`, `Vec<(String, Node)>`) rather than a
  small named struct (e.g. `struct ConstBinding<'a> { name: String, initializer: Node<'a> }`).
  Eight independent hand-written `binding_finder` implementations (Tasks 1.1.1c–1.1.1h) each
  construct this tuple positionally; a swapped element order in any one of them (initializer
  where name is expected, or vice versa) compiles silently and produces a subtly wrong
  exclusion rather than a compile error. Per-language unit tests (Epic 2.1) should catch this
  in practice, so this isn't blocking, but a named struct removes the whole mistake class at
  zero runtime cost and costs one small type definition. **Remediation**: add the struct in
  Task 1.2.1a alongside `LiteralCollector`, and use it as `binding_finder`'s return type and
  `bound`'s element type.

## Nitpicks

- `LiteralCollector.occurrences: HashMap<String, Vec<Node>>` keys are raw literal text with
  no domain wrapper (e.g. a `LiteralText(String)` newtype) — acceptable primitive obsession
  for a Transaction-Script-style checker at this scale, and consistent with this file's
  existing `HashSet<String>`/generalized-to-`HashMap<String, usize>` idiom for identifiers.
  No change requested.
- The multi-value-destructuring gap (Unresolved Question 3) and Python's casing-convention
  exclusion heuristic being weaker than the other 7 languages' keyword-based checks are both
  already disclosed explicitly in ADR-001's Consequences section and plan.md's Unresolved
  Questions — this is the right way to handle a known, scoped limitation and needs no
  further action.
- Lens 3 checks (PoEAA/GoF pattern fit, build-vs-buy consistency) all pass: the plan matches
  `research/build-vs-buy.md`'s "build natively in `src/rules.rs`" recommendation exactly, and
  the function-pointer Strategy dispatch for `literal_kinds`/`binding_finder` follows this
  file's own established idiom (`body_finder`/`params_finder`) rather than inventing a new
  per-rule trait hierarchy — correctly reasoned in the Pattern Decisions table.
- Tech Debt Disposition table row 2 (`src/rules.rs`'s overall 2329-line file-size violation,
  "Extend as-is") still justifies itself qualitatively ("proportionate to the existing
  pattern, not a strain on it") rather than with an explicit projected post-feature line
  count, echoing the phrasing style the original blocker objected to — but on a row that
  isn't compounding an existing violation the way `lang_config()`'s was, so it doesn't rise
  to blocker or concern severity. Optional polish: state the expected total (e.g. "~2329 +
  ~40 new lines ≈ 2370").
