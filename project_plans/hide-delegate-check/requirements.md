# Requirements: Native check — Hide Delegate (chained method/field-access depth)

item_id: 1a2cd8d7-282e-4204-b420-fbb473dd3b71

## Background

Client code that reaches through an object graph — `a.getB().getC().doThing()` —
couples the caller to the whole chain's shape, not just `a`'s interface. This is a
Law of Demeter violation (Lieberherr, Holland & Riel, "Object-Oriented Programming:
An Objective Sense of Style," OOPSLA 1988) and Fowler's **Hide Delegate** is the fix:
wrap the chain behind a method on `a`.

## Proposal (from backlog item)

Extend `src/rules.rs` with a chained-access-depth rule, structurally identical in
shape to the existing `deep-nesting` check but walking method-call/field-access
chains instead of block nesting: flag a chain of ≥3 dot-accesses in one expression
(tunable threshold, same config pattern as `long-function`'s 40-line default).

## Scope notes (from backlog item)

- Fluent builder APIs (`.filter().map().collect()`) are the expected false-positive
  class — worth excluding chains where every intermediate call returns the *same*
  type (a strong builder-pattern signal) rather than a chain that changes type at
  each hop, which is the actual Law-of-Demeter-relevant case.
- No new capability needed — this is a per-file AST rule, same tier as the existing
  `rules.rs` checks.

## Depends on

None.

## Repo conventions this check must follow (`src/rules.rs`, `src/config.rs`)

- New rule lands in `default_checks()` (`config::default_checks()` /
  `core_checks()`/`syntax_rules_checks()` family in `src/config.rs`) as part of
  landing it — not as a follow-up. See `CLAUDE.md`'s "Default check catalog"
  section.
- Structural shape to mirror: `deep-nesting` in `src/rules.rs` — a `const` threshold
  documented at module top (see `MAX_NESTING_DEPTH`), an entry in `CATALOG`
  (`RuleMeta` — id, category, description, default_severity), a per-language
  `LangRuleConfig` walk, and a `Finding` with a `[rule-id]` prefixed message
  matching the existing `[deep-nesting]`/`[long-function]` convention.
- Must integrate with the existing `LangRuleConfig` per-language node-kind table
  (Go/TS/JS/Python/Java/Kotlin/Rust each have distinct AST shapes for "the same"
  construct — verified against real `to_sexp()` output, not guessed by analogy,
  per the comment at `src/rules.rs:67-71`).
- New checker must be backtested per `CLAUDE.md`'s "Writing a new check" section
  before being considered done:
  - `kibitzer check backtest <name>` against real historical Claude Code session
    transcripts (`docs/backtesting.md`).
  - The real-world repo corpus (`docs/backtest-repos.md` /
    `scripts/clone-backtest-repos.sh`), triaged with
    `scripts/backtest-triage.py` (`docs/backtest-triage/README.md`).
- `docs/check-ideas.md` holds new check ideas to a "confirmed transcript
  occurrence" bar — worth confirming this pattern actually shows up in real
  session transcripts, not just plausible in theory.

## Acceptance criteria

1. A new native check (working id: `hide-delegate`, exact id TBD in plan.md) is
   added to `src/rules.rs` and wired into `config::default_checks()`, active with
   no `.claude/inspect.json` required — consistent with every other `rules.rs`
   check.
2. The check flags an expression chaining ≥3 dot-accesses (method calls and/or
   field accesses) in a single expression, with the threshold expressed as a
   named `const` (mirroring `MAX_NESTING_DEPTH`/`LONG_FUNCTION_LINES`), not a
   magic number.
3. The check runs across the same language set already covered by the
   `LangRuleConfig` table used by `deep-nesting`/`long-function` (Go, TS, JS,
   Python, Java, Kotlin, Rust) — or, if full per-language chain-type inference is
   infeasible for some of them, the plan explicitly states which languages are
   in scope for v1 and why.
4. Fluent builder chains are suppressed using the type-based heuristic in the
   scope notes: a chain where every intermediate call returns the same type as
   its receiver is not flagged. The plan documents how "same type" is determined
   per language given tree-sitter's syntactic (not type-checked) AST — this is
   the key open design question and must be resolved or explicitly deferred with
   a documented fallback heuristic before implementation.
5. Unit tests exist per language following the existing `flags_deep_nesting`-style
   test naming/structure (`{lang}_flags_hide_delegate` or similar), covering both
   a flagged chain and a suppressed builder-pattern chain.
6. The check is backtested against session transcripts (`kibitzer check backtest`)
   and the public repo corpus before being marked done, per `CLAUDE.md`'s
   mandatory backtesting requirement — false-positive rate on the corpus is
   recorded in the plan/validation artifacts.
7. `RuleMeta` entry added to `CATALOG` in `src/rules.rs` with category, id,
   description, and default severity consistent with existing entries
   (`Severity::Advisory`, matching `deep-nesting`'s severity).
8. Documentation follows the existing pattern: this check should be referenced
   in `docs/syntax-rules.md` alongside `long-function`/`deep-nesting`/etc., since
   `config::default_checks()`'s doc comment enumerates the full default catalog
   by name.

## Non-goals

- No new checker capability/tier (no `command`, no architecture model) — this is
  a per-file AST rule at the same tier as existing `rules.rs` checks.
- No cross-file or whole-program type resolution — the type-sameness heuristic
  must work from the single-file syntactic AST tree-sitter already provides,
  consistent with how every other `rules.rs` check operates.
