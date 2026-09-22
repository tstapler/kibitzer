# Requirements: replace-magic-literal

**Date**: 2026-09-22
**Type**: new native checker (extends `src/rules.rs`'s existing per-language node-kind table)
**Complexity**: 2 — mechanical AST rule following the exact shape of `long-function`/
`deep-nesting`/`long-parameter-list`, but with a new whole-file (not per-declaration) walk
and a name-binding exclusion that those three don't need.

## Problem Statement

Fowler's **Replace Magic Literal** — a non-trivial numeric or string literal repeated
two or more times in a file with no bound named constant — is one of the best-established
smells in the refactoring catalog and the cheapest to detect: PMD, SonarQube, and ESLint
(`no-magic-numbers`) all ship a version of it. kibitzer's own catalog-coverage audit
(`docs/refactoring-catalog-analysis.md:252`) already lists it as issue **#42**, tagged 🔧
("extends the existing engine") and explicitly placed in the **no-dependency batch** —
refactorings that need no other in-flight capability and can "ship independently" as soon
as someone picks them up (`docs/refactoring-catalog-analysis.md:173-186`). kibitzer has no
literal-repetition check today; `rules.rs`'s `CATALOG` (`src/rules.rs:34-65`) covers
`long-function`, `deep-nesting`, `long-parameter-list`, `flag-argument`, and
`unreachable-code` — none of which inspect literal values at all.

## Baseline

- `SyntaxRulesChecker` (`src/rules.rs:705-743`) is the home for this kind of check: one
  `Checker` per language, driven by a per-language `LangRuleConfig` table
  (`src/rules.rs:72-144`) that maps each of the 8 supported grammars (Go, TypeScript,
  Tsx, JavaScript, Python, Java, Kotlin, Rust — `lang_config()`, `src/rules.rs:487-703`)
  onto its own node-kind names, verified against real `to_sexp()`/`node-types.json`
  output rather than guessed by analogy across grammars.
- The existing three complexity rules (`long-function`, `deep-nesting`,
  `long-parameter-list`) all walk from `function_kinds` declarations
  (`walk_declarations`, `src/rules.rs:745-753`) — i.e., they only ever look inside one
  function body at a time. A literal-repetition check is **file-scoped**: the same
  literal can repeat across two unrelated functions, so it needs a new whole-tree walk
  in the shape of `walk_blocks` (`src/rules.rs:758-766`), not a reuse of
  `walk_declarations`.
- No literal-related node kinds exist anywhere in `LangRuleConfig` today — this is a new
  table column, not a reuse of an existing field.
- `config::default_checks()` → `syntax_rules_checks()` (`src/config.rs:590-599`) is how
  a `SyntaxRulesChecker`'s findings actually reach a repo without hand-authored
  `.claude/inspect.json` — a new rule inside `SyntaxRulesChecker` is automatically live
  everywhere once it ships, per this repo's CLAUDE.md standing rule that a new
  no-repo-setup-required checker "goes in `default_checks()` as part of landing it, not
  as a follow-up someone has to remember."
- **Suppression convention correction**: the backlog item's scope note suggests "an
  exclusion glob or a `#[allow]`-style suppression comment, matching the existing
  suppression convention in `docs/suppressing-checks.md`." That doc is explicit that
  kibitzer has **no** inline/per-line suppression comment mechanism (`// kibitzer:disable`,
  `# noqa`, etc.) by design — the granularity below "whole checker" or "whole
  file/directory via `scope`" is instead a checked-in per-finding acceptance file under
  `.kibitzer/accepted/` (`docs/accepting-findings.md`). Table-driven test/fixture files
  with legitimately repeated literals should use a `scope` exclusion glob in
  `.claude/inspect.json` (already-existing mechanism, `docs/suppressing-checks.md`) or
  per-finding acceptance — not a new suppression-comment mechanism this project would
  otherwise be implicitly asked to invent.
- Every new native checker needs a two-part validation pass per this repo's own
  CLAUDE.md: `kibitzer check backtest <name>` against real transcript edits
  (`docs/backtesting.md`) and a run against the real-world corpus
  (`docs/backtest-repos.md`, `scripts/backtest-triage.py`) — "a checker that hasn't been
  run against this corpus at least once isn't done."

## Users / Consumers

kibitzer's own maintainer and any downstream user running the default check catalog
(CLI, daemon, MCP server, Claude Code `PostToolUse` hook) — this is a default-on check,
not opt-in, so its false-positive rate directly affects every existing kibitzer install.

## Proposed Rule (from the backlog item)

Walk literal nodes (numeric/string) per language. Skip an allow-list of near-universal
values (`0`, `1`, `-1`, `""`, empty collection literals). Flag any remaining literal
value occurring **≥2 times** in a file, unless it is the direct initializer of a
`const`/`let … = <literal>`-style binding that is **referenced elsewhere by name** (i.e.
already correctly factored into a symbolic constant).

## Success Metrics / Acceptance Criteria

1. A new `replace-magic-literal` rule is added to `rules.rs`'s `CATALOG`
   (`src/rules.rs:34-65`) and fires via the existing `SyntaxRulesChecker` for every
   language `syntax-rules` already covers (Go, TypeScript, Tsx, JavaScript, Python,
   Java, Kotlin, Rust) — not a subset, since the underlying node-kind table already
   spans all eight and the smell is language-agnostic.
2. It flags a non-trivial numeric or string literal appearing ≥2 times in one file.
3. It does **not** flag `0`, `1`, `-1`, `""`, or an empty collection literal
   (`[]`, `{}`, and their per-language equivalents), matching the issue's allow-list.
4. It does **not** flag a literal that is the direct initializer of a named
   `const`/`let`/`val`-style binding referenced elsewhere by name in the file (i.e. the
   smell is already fixed).
5. It is registered in `config::default_checks()` automatically (inherited for free via
   `syntax_rules_checks()` — no separate registration step, unlike a standalone
   checker).
6. Unit tests exist per language following the existing `rules.rs` test convention
   (e.g. `flags_long_function`, `ts_flags_deep_nesting`, `py_flags_long_parameter_list`)
   covering: a true positive, the allow-list near-universal values, and the
   named-constant exclusion.
7. `kibitzer check backtest replace-magic-literal` has been run against real transcript
   history (`docs/backtesting.md`) and the result — fires on real repeated-literal
   edits, doesn't misfire pathologically — is recorded.
8. The checker has been run against the real-world backtest corpus
   (`docs/backtest-repos.md`) and any true/false-positive triage is tracked via
   `scripts/backtest-triage.py` per `docs/backtest-triage/README.md`.
9. `docs/syntax-rules.md` is updated to document the new rule alongside the other five,
   and `docs/suppressing-checks.md`'s default-catalog list gains the new check name.
10. Table-driven test/fixture files with legitimate repeated literals (e.g. parametrized
    test data) have a documented mitigation path (`scope` exclusion glob or
    `.kibitzer/accepted/`) that doesn't require a new suppression mechanism.

## Appetite

Small — matches the size of the existing three complexity rules it sits alongside. The
issue itself is filed as a no-dependency, ship-independently item; the honest unknown is
false-positive rate at scale (test/fixture literal repetition, HTTP status codes, array
indices), which the mandatory backtest step (AC 7–8) is what actually derisks it, not
speculative pre-analysis.
