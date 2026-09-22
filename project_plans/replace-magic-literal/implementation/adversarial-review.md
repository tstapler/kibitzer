# Adversarial Review: replace-magic-literal

**Date**: 2026-09-22
**Verdict**: CONCERNS

Cross-checked plan.md's line-number/function-name claims directly against `src/rules.rs`
(2329 lines) — `CATALOG` (34-65), `LangRuleConfig` (72-144), `check()` (732-742),
`walk_declarations` (745-753), `walk_blocks` (758-766), `collect_condition_identifiers`
(881-896), `collect_identifiers` (898-908, single call site confirmed), `branched_on.contains`
at exactly line 861, and the `assert_valid` helper (1172-1190) all match exactly as cited.
`grep -n "is_generated" src/rules.rs` confirms zero existing hits, matching ADR-001's premise.
The `..lang_config(Language::TypeScript)` inheritance pattern for Tsx/JavaScript (lines
547-556) is real, pre-existing precedent, not a new architectural bet. The plan's factual
grounding is unusually solid — findings below are about logic gaps and untested edge cases,
not fabricated citations.

## Blockers

None. Nothing found here rises to "must fix before writing code" — the two most serious
items (allow-list coverage gaps) are one-line fixes best made during Task 1.2.1a/1.1.1b
rather than a reason to halt planning.

## Concerns

- [ ] **`MAGIC_LITERAL_ALLOWLIST` is a single global raw-text list, but the per-language
  `literal_kinds` tables (Story 1.1.1) deliberately include language-specific string forms
  whose empty-value raw text never matches it** — Kotlin's `multiline_string_literal` (empty
  triple-quoted string is `""""""`, 6 quote chars), Rust's `raw_string_literal` (empty is
  `r""` or `r#""#`), and Go's backtick raw string (`` `` ``) all have literal_kinds entries
  per Story 1.1.1's own per-language table (plan.md lines 132, 145-169), but
  `MAGIC_LITERAL_ALLOWLIST = &["0", "1", "-1", "\"\"", "''"]` (Task 1.2.1a) only covers the
  two most common quote styles. AC3 explicitly extends the allow-list guarantee to "empty
  ... and their per-language equivalents" (requirements.md:86) — as written, an empty Kotlin
  triple-quoted string or Rust raw string repeated twice would be incorrectly flagged, a
  direct AC3 violation for those languages' own literal forms. Compounding this: Task
  2.1.1b's allow-list unit test is Go-only, so this gap would not surface until backtest (or
  never, if no corpus file happens to contain an empty raw/multiline string literal).
  **Recommendation**: either normalize each literal's raw text (strip per-language
  delimiters/prefixes before allow-list comparison) or make the emptiness check structural
  (e.g., "is this literal's *content* the empty string once delimiters are stripped")
  rather than a fixed string list. Resolve as part of Task 1.1.1b's `-1` probe work, since
  it's the same "raw text varies by grammar" class of problem.

- [ ] **The AC3 "empty collection literal" (`[]`, `{}`) guarantee is satisfied only by
  omission, not by design, and isn't tested or documented as such.** None of the 8
  languages' `literal_kinds` values (Story 1.1.1's table) include a collection/composite
  literal node kind, so `[]`/`{}` are never visited by `walk_literals` — the AC is met, but
  accidentally, as a side effect of `literal_kinds` scope rather than an explicit allow-list
  entry or code comment saying so. If a future contributor extends `literal_kinds` to cover
  another literal shape (plausible — e.g., adding boolean literals or enum-constant literals
  later) without re-reading this reasoning, the empty-collection exemption could silently
  break with no test to catch it. **Recommendation**: add a one-line doc comment next to
  `literal_kinds`'s definition (Task 1.1.1a) stating this is intentional, and add one
  regression test (e.g., Go `s := []int{}` / `m := map[string]int{}` repeated twice → zero
  findings) locking in the guarantee explicitly rather than by accident.

- [ ] **`emit_literal_findings`'s numeric/string label heuristic ("derive ... from whether
  `text` starts with a quote character," Task 1.2.1e) mislabels several literal forms the
  plan's own `literal_kinds` table includes**: Go backtick raw strings, Rust
  `raw_string_literal` (`r"..."`/`r#"..."#`), and Python's prefixed string forms
  (`r"..."`, `b"..."`, `f"..."`, all valid under Python's `string` node kind per
  Unresolved Question 2) all start with a non-quote character and would be mislabeled
  "numeric literal" in the Finding message instead of "string literal." Cosmetic only —
  doesn't affect firing/exclusion — but it's a concrete defect in a task the plan writes as
  if `starts_with('"')`-style logic is sufficient. **Recommendation**: derive the label from
  `literal_kinds` membership against the language's known string-kind subset (already known
  per-language data) rather than sniffing the raw text's first character.

- [ ] **Pitfalls.md §3's "cross-scope/cross-function const" ambiguity is not explicitly
  resolved or tested, despite being flagged by name as something Phase 3 should decide
  explicitly** ("worth resolving explicitly in Phase 3's plan rather than left implicit,"
  pitfalls.md:117-120, restated in the Phase-3-decisions summary at pitfalls.md:250-252).
  `resolve_excluded_constants` (Task 1.2.1d) excludes a bound literal whenever its name's
  raw-text occurrence count is ≥2 anywhere in the file, with no distinction between "used
  in a different function" (Fowler's actual smell: literal correctly factored and reused
  across sites) and "used once more inside its own declaring function" (a plain, possibly
  trivial local reference). The plan's Pattern Decision table addresses the adjacent
  *shadowing* question explicitly (accepted false negative, matches
  `collect_condition_identifiers` precedent) but never states whether "referenced elsewhere"
  was deliberately read as "anywhere in the file, including the same function" versus
  requiring a cross-function/cross-site reference — pitfalls.md treats these as separate,
  both-flagged concerns. **Recommendation**: add one sentence to ADR-001's Decision section
  (or the Pattern Decision table's "Const-reference scope resolution" row) stating this was
  a deliberate choice, and add a test case for it (a const referenced exactly once, only
  within its own declaring function) so the behavior is locked in rather than incidental.

- [ ] **AC6's "unit tests exist per language ... covering: a true positive, the allow-list
  near-universal values, and the named-constant exclusion" is only fully satisfied for Go**
  (Story 2.1.1 explicitly limits the allow-list test and the generated-file-guard test to
  Go, per its "representative — not duplicated 8x" rationale). This is defensible for the
  allow-list specifically, since `is_allowlisted_literal` is a single shared, non-per-language
  function (Task 1.2.1e) — testing it once genuinely proves it for all 8 languages. But that
  argument does *not* extend to the previous two findings above (the Kotlin/Rust/Go raw-string
  and label-mismatch issues are per-language `literal_kinds` data, not shared logic), so the
  current test plan would not catch either of them even after those are fixed unless a
  language-specific regression test is added alongside the fix. Not a standalone blocker, but
  it explains why the two findings above went unnoticed by the plan's own AC6 test coverage.

## Minors

- Rust macro-invocation literals (arguments inside a `macro_invocation`'s `token_tree`) are
  not addressed anywhere in the plan. Likely a non-issue in practice — tree-sitter-rust
  typically parses macro-invocation bodies as an opaque `token_tree` rather than typed
  literal nodes, so they'd structurally never reach `walk_literals` (same "excluded by
  omission" pattern as the empty-collection-literal finding above) — but this reasoning is
  asserted here, not verified against `codegen/node-types/rust.json`, and the plan doesn't
  mention it at all. Worth a one-line confirmation during Task 1.1.1h's Rust work, not worth
  blocking on.
- The Python `^[A-Z][A-Z0-9_]*$` screaming-snake-case heuristic (Task 1.1.1e) will also match
  a single-character all-caps local loop variable or a class name conflated with a constant
  (e.g., `X = 5` — one uppercase letter, technically matches) — ADR-001 already documents this
  detector as "fundamentally weaker... an accepted, documented asymmetry," so this is a known,
  already-disclosed limitation, not new — noting only that a single-char edge case wasn't
  explicitly called out alongside the general acknowledgment.
- `docs/suppressing-checks.md`'s literal text (verified: it names checker-level items like
  `markdown-link-integrity`, `duplicate-code`, etc., and points to `docs/syntax-rules.md` for
  the syntax-rules family rather than listing individual rule ids) does *not* actually gain
  "the new check name" as AC9's text literally requests — the plan's Epic 4.2 correctly
  identifies that no other `CATALOG` rule id (e.g. `long-function`) is listed there either,
  and documents the reconciliation rather than silently skipping it. This is good practice,
  confirmed accurate by direct read of the file — noted here only because it's a literal,
  if reasonable, non-conformance to an AC's exact wording, and should be called out plainly
  in the PR description as the plan already intends.
- Two-pass collect-then-emit design and the `LiteralCollector` struct have no dedicated unit
  tests of their own (only integration-style tests via `check()`) — consistent with every
  other rule in this file (no isolated tests of `walk_declarations`/`walk_blocks` either), so
  this is precedent-consistent, not a gap specific to this feature.
