# Implementation Plan: hide-delegate-check

**Feature**: Native `hide-delegate` check in `src/rules.rs` — flags expression chains of 3+
dot-accesses (method calls and/or field reads) across all 7 `LangRuleConfig` languages, with a
syntactic fluent-allowlist + builder-verb-prefix heuristic suppressing the fluent-builder false
positive class.
**Target user** *(per triad review's Product-lens gap)*: two distinct consumers, both already
served by every other `rules.rs` check — a human code reviewer reading `kibitzer run` output
during PR review, and an AI coding agent consuming findings inline via `run_checks`/MCP tools
mid-session (kibitzer's stated primary audience — see this repo's own top-level description). The
finding-message wording (Task 1.3.1d) is written for the AI-agent case specifically, since that's
the audience most likely to act on a finding mechanically without human judgment in the loop (see
pre-mortem.md Failure #3 and its fix, applied below).
**Date**: 2026-09-22
**Status**: Ready for implementation
**ADRs**: ADR-001-syntactic-fluent-allowlist-plus-builder-prefix-suppression.md

---

## Domain Glossary
*(Ubiquitous language — every domain term that appears as a type, method, or variable name. Exact
names here must be used consistently in code, tests, and comments.)*

| Term | Definition | Notes |
|------|-----------|-------|
| `hide-delegate` | The rule id registered in `CATALOG` and emitted as the `[hide-delegate]` prefix on every `Finding` this check produces. Named after Fowler's *Hide Delegate* refactoring, the fix Law-of-Demeter violations call for. | Rule id string, used verbatim in `RuleMeta.id` and every finding message. |
| dot-chain | One expression composed of consecutive chain hops off a common chain root — e.g. `a.B().C().D()`. | Not a type; a description of a shape in the AST. |
| chain hop | One dot-access step in a dot-chain: either a bare field/property read or a method call, contributing one unit toward the chain's hop count. | Counted via `chain_step`; each hop's accessor name feeds the suppression heuristic. |
| chain root / chain base | The point at which `chain_step`'s returned next-node is no longer itself a `dot_chain_kinds` node — an identifier, `this`/`self`, a literal, or a bare (non-member) call. | Walking stops here; the base itself is never counted as a hop. |
| outermost chain node | The topmost `dot_chain_kinds` node of one chain expression — its parent's kind is not itself in `dot_chain_kinds`. | `walk_chains` computes a hop count exactly once per outermost node, mirroring `check_block_for_unreachable`'s "flag once per block" principle. |
| `MAX_CHAIN_LINKS` | `const usize = 2` — the maximum hop count allowed before a chain is flagged (so a chain is flagged when hop count > 2, i.e. 3+ dot-accesses). | Sibling to `MAX_NESTING_DEPTH`/`LONG_FUNCTION_LINES` in shape and placement (module top of `src/rules.rs`). |
| `dot_chain_kinds` | New `LangRuleConfig` field: `&'static [&'static str]`, the node kinds that can appear as a chain hop for this language (a member-access kind and a call kind; Java lists both `method_invocation` and `field_access`). | Not the same field as the existing `chain_kinds` (if/elif chaining) — reusing that name would be a real collision, per architecture.md §2. |
| `chain_step` | New `LangRuleConfig` field: `fn(Node, &[u8]) -> Option<(Node<'_>, String)>`. Given a `dot_chain_kinds` node, returns the next node toward the chain root plus the accessor name text at this hop; returns `None` when the node isn't a real hop (a bare call whose callee isn't a member access, e.g. Rust's `std::mem::size_of::<T>()`). | Field-based for Go/TS/JS/Python/Java/Rust, positional for Kotlin — mirrors the existing `body_finder`/`kotlin_body` split. |
| `fluent_allowlist` | New `LangRuleConfig` field: `&'static [&'static str]`, per-language accessor names whose stdlib/ecosystem contract makes them safe to treat as non-Demeter-violating (Rust iterator adapters, Java Stream/Optional, JS/TS Array/Promise, Kotlin's `also`/`apply` plus collection methods). | Empty for Go and Python in v1 — a deliberate, documented scope decision, not an oversight. |
| builder-verb prefix | One of a fixed, language-agnostic set of name prefixes (`set`, `with`, `add`, `put`, `append`, `and`) that make a hop's accessor name look like a classic OOP builder setter. | Stored once as `BUILDER_VERB_PREFIXES`, shared across all 7 languages — unlike `fluent_allowlist`, not per-language data. |
| `walk_chains` | New top-level function, sibling to `walk_declarations`/`walk_blocks` (`src/rules.rs:745-766`), recursing the whole parse tree looking for outermost `dot_chain_kinds` nodes. | Called from `SyntaxRulesChecker::check()` alongside the two existing walks. |
| `check_chain` | New helper invoked by `walk_chains` once per outermost chain node: walks `chain_step` repeatedly to collect hop names, computes hop count, and — if over `MAX_CHAIN_LINKS` and not suppressed — pushes a `Finding`. | Mirrors `check_declaration`/`check_block_for_unreachable`'s per-site-check shape. |
| `is_chain_suppressed` | New helper: `fn(&[String], &LangRuleConfig) -> bool`. True iff every hop name is in `cfg.fluent_allowlist` OR matches a `BUILDER_VERB_PREFIXES` entry (checked independently per hop, so a chain may mix allowlist hops and prefix hops). | The AC4 suppression heuristic — a syntactic proxy, not type inference. See ADR-001. |
| `BUILDER_VERB_PREFIXES` | Shared `const &[&str]` = `["set", "with", "add", "put", "append", "and"]`. | Matched case-insensitively with a camelCase word-boundary check (`has_builder_verb_prefix`) so it covers both Go's `WithTimeout` and Java/JS's `withTimeout` without also matching unrelated names like `andrew`. |

---

## Pattern Decisions

| Component | Pattern Chosen | Source | Alternative Rejected | Reason |
|-----------|---------------|--------|---------------------|--------|
| Overall integration approach (Step 0.5 creative pass) | Native sibling-function extension of `SyntaxRulesChecker` (`walk_chains` alongside `walk_declarations`/`walk_blocks`) | research/architecture.md §1-2, §7; research/build-vs-buy.md | (B) External command-based checker wrapping PMD's `LawOfDemeter` (Java only) + hand-rolled native checks for the other 6 languages. (C) tree-sitter `Query`/`.scm`-based chain matching, one query file per language. | (B) Java-only — leaves 6/7 languages needing the same native work anyway, requires a local JVM+PMD install, and doesn't fit the plugin system's binary-distribution model (build-vs-buy.md §1). (C) Introduces a second, inconsistent extension mechanism (`tree_sitter::Query` + `.scm` compilation) into a file with zero existing `Query`/`.scm` usage today, for a rule whose complexity doesn't justify new plumbing (build-vs-buy.md §4). |
| Per-language extension point | Table-driven Strategy — three new function-pointer/slice fields on `LangRuleConfig` (`dot_chain_kinds`, `chain_step`, `fluent_allowlist`) | GoF Strategy, applied via the file's existing function-pointer-field convention (`body_finder`, `bool_param_finder`, `panic_detector`) | A `trait ChainWalker` with 7 per-language `impl` blocks | A 7-impl trait hierarchy is heavier than the function-pointer-field convention every other `LangRuleConfig` capability already uses; it would be the only trait-based extension point in a file where everything else is data-plus-free-functions, breaking consistency for no behavioral gain. |
| Suppression heuristic | Two independent per-hop syntactic tests (allowlist membership OR builder-verb-prefix match) | research/architecture.md §4 (its own two-part recommendation) | Full type inference via symbol/whole-program resolution | Explicitly out of scope per requirements.md's Non-goals (no cross-file/whole-program type resolution); no existing kibitzer infrastructure resolves types from tree-sitter's syntactic AST anywhere in the codebase (research/features.md §1.2). |
| Suppression data storage | Third `LangRuleConfig` field (`fluent_allowlist`), not a parallel `match Language` block | Table-driven Strategy, same as above | A standalone `fn fluent_allowlist_for(lang: Language) -> &'static [&'static str]` match statement living outside `LangRuleConfig` | Splits per-language data across two places (the table and a parallel match) instead of one; `bool_param_finder`/`terminal_kinds`/`panic_detector` all establish that per-language data belongs *in* the table, not beside it. |
| Hop counting | Iterative descent via `chain_step`, mirroring `max_nesting_depth`'s recursive-descent shape | Existing repo precedent (`max_nesting_depth`/`walk_if_chain`, `src/rules.rs:913-1000`) | A generic `tree_sitter::Visitor`/`Query`-based traversal abstraction | build-vs-buy.md §4 already rules this out — no `Query`/`.scm` machinery exists in the codebase, and adding one for a single rule is disproportionate to the rule's actual complexity (a plain recursive walk, same as every sibling rule). |
| Chain-root/receiver modeling | No `ReceiverOrigin` sum type; `this`/`self`/a stranger/a locally-constructed value are all treated identically (plain AST nodes, no classification) | Type-driven design considered and rejected | A `ReceiverOrigin` enum (`Stranger`/`LocalNew`/`SelfRef`) tracked per chain root | Populating it correctly needs def-use/data-flow tracking across statements, explicitly out of scope (single-expression, syntactic-only per Non-goals) — the enum's extra variants could never be populated with confidence, making it a sum type with permanently-unreachable-with-confidence states rather than a real modeling win. |
| Threshold representation | Plain `usize` const (`MAX_CHAIN_LINKS`), no wrapper type | Consistency with `MAX_NESTING_DEPTH`/`LONG_FUNCTION_LINES`/`LONG_PARAM_LIST_COUNT` | A `ChainHopCount(usize)` newtype | No other numeric threshold in `rules.rs` is wrapped either; a newtype for one rule alone breaks consistency with its closest sibling rule (`deep-nesting`) without adding safety the file doesn't already lack. |

---

## Tech Debt Disposition
*(Every hotspot/architecture violation touched by this feature, per `research/architecture.md`. "None identified" if none.)*

| Area | Existing Issue | Disposition | Justification |
|------|----------------|--------------|----------------|
| `src/rules.rs` (`LangRuleConfig` / `SyntaxRulesChecker`) | None — confirmed not a hotspot, per research/architecture.md §7 | Extend as-is | "No refactor of `rules.rs` or `LangRuleConfig` is warranted before landing this" (architecture.md §7): zero new abstractions needed at the `Checker`/registry/`config.rs` level; the new fields follow the file's existing `body_finder`/`bool_param_finder` dual-implementation (field-based vs. positional) convention byte-for-byte; this is the pattern's sixth rule, not its first stress test. |
| `src/config.rs` | None | No change required | `syntax_rules_checks()` (`src/config.rs:718-738`) already wires one `Check` per language pointing at `SyntaxRulesChecker`, which runs every `rules::CATALOG` rule in one pass — `hide-delegate` rides along automatically once it's in `CATALOG`, satisfying AC1 with zero `config.rs` edits (architecture.md §1). |

---

## Design Positions on Edge Cases
*(Explicit scope/design decisions prose, per the task brief — not just encoded in tasks.)*

1. **`this`/`self`-qualified chains are NOT special-cased.** `this.getA().getB().getC()` counts
   identical hops to `x.getA().getB().getC()`. PMD's own `LawOfDemeter` rule has a years-open,
   unresolved issue on exactly this (pmd/pmd#4414) — either direction (undercount `this` chains
   as PMD does, or count them uniformly) is defensible, but per pitfalls.md §4's evidence
   (`bool_param_finder`'s "don't act when ambiguous" philosophy, `go_call_resolution.rs`'s
   "unresolved stays flagged" precedent), this codebase's convention is to default toward
   flagging, not suppressing, when a distinction would require guessing. Counting `this`/`self`
   uniformly is the simpler implementation *and* the false-negative-avoiding choice, so both
   considerations point the same way.
2. **The AC4 heuristic (`is_chain_suppressed`) is a syntactic proxy for the locally-constructed-
   receiver exemption, not a detector of it.** Law of Demeter's own literature (Yegor Bugayenko's
   "one dot" rebuttal, cited in features.md §2.2) exempts a chain on an object the method itself
   `new`'d — a *semantic*, origin-tracking fact this repo's syntax-only AST cannot observe without
   the data-flow tracking ruled out above. The fluent-allowlist + builder-verb-prefix heuristic
   approximates the common case (`new Builder().setX().setY().build()` mostly has
   prefix-matching hops) but is neither necessary nor sufficient for the literature's actual
   exemption — documented as a known gap, not silently conflated with it.
3. **Static/namespaced calls are excluded only where the grammar gives a distinct node kind.**
   Rust's `std::mem::size_of::<T>()` uses `scoped_identifier` as the call's `function` field — a
   different node kind from `field_expression`, so `rust_chain_step` naturally returns `None` for
   it with zero special-casing. Java (`Thread.currentThread().getId()`), Kotlin, and Go have no
   such distinct node kind: a class/package-qualified access is syntactically identical to an
   instance chain (a plain identifier as the `object`/`operand`). This is a known false-positive
   class shared with PMD's own unresolved issue (pmd/pmd#2180) — not fixed here, tracked via the
   mandatory corpus backtest (Phase 6) instead of guessed at with an unreliable
   "capitalized-identifier-means-static" heuristic (which fails for Go, where exported
   identifiers are always capitalized regardless of static-ness).
4. **Indexed access is a hard chain-boundary, not a continuation.** `a[i].b[j].c` (PMD #2181):
   an `index_expression`/subscript node is never in `dot_chain_kinds`, so when `chain_step`'s
   returned next-node has that kind, the walk stops — only the hops after the last index
   boundary are counted (in this example, just `.c`, one hop). This undercounts relative to the
   "true" reach-through depth, a deliberate conservative choice: no attempt is made to "see
   through" an index to keep counting, since doing so correctly would require distinguishing an
   index that changes the effective root type from one that doesn't, which is exactly the kind
   of guess this repo's checks avoid.
5. **Chains split across statements via an intermediate variable are out of scope by
   construction.** `let b = a.getB(); b.getC().doThing();` is invisible to a per-expression walk
   — this matches the requirement's own text ("≥3 dot-accesses in *one expression*") and mirrors
   `deep-nesting`'s own per-declaration (not whole-program) analysis philosophy. Stated as a known
   ceiling, not a silently-accepted gap.
6. **The walk descends into lambda/closure bodies mid-chain, and treats each inner chain as its
   own separate outermost chain.** `xs.forEach(x -> x.getB().getC().getD())`: the outer
   `xs.forEach(...)` chain (1 hop) and the inner `x.getB().getC().getD()` chain (3 hops) are each
   evaluated independently by `walk_chains`'s ordinary whole-tree recursion — no special pruning
   or fusion logic is needed, because the inner chain's outermost node is reached and dedup-
   checked the same way any other outermost node is (architecture.md §2). Verified as its own
   test, Task 4.3.3a.
7. **This check has no confirmed transcript occurrence.** Per `CLAUDE.md`'s "Writing a new check"
   bar and `docs/check-ideas.md`'s "2+ independent occurrences in real session transcripts"
   standard: `hide-delegate` is not in `check-ideas.md` at all. It proceeds on
   `requirements.md`'s backlog item and GitHub issue #44's design rationale (Fowler's *Hide
   Delegate* + Lieberherr, Holland & Riel's original Law of Demeter paper) alone — the same
   footing `duplicate-code` shipped on before it, per pitfalls.md §1. This is stated explicitly
   so the corpus backtest (Phase 6) is understood as the *only* evidence gate this check gets
   before shipping, not a supplementary one.
8. **Ambiguity resolves toward flagging, not suppressing, throughout.** Every decision above that
   involved a real choice under ambiguity (`this`-qualification, static calls, index boundaries)
   defaults toward the false-negative-avoiding direction — consistent with `bool_param_finder`'s
   and `go_call_resolution.rs`'s existing precedent (pitfalls.md §4) and the structural argument
   that this repo's entire false-positive-detection pipeline (`backtest-triage.py`,
   `report_false_positive`, `docs/<check>-false-positives.md`) can catch over-flagging but has no
   equivalent mechanism to ever notice under-flagging.

---

## Migration Plan

Omitted — no schema or data changes are involved (a pure code-plus-docs addition to an existing
checker).

## Observability Plan
- **Logs**: No new logging infrastructure. Findings flow through kibitzer's existing `Finding{line,
  message}` → stdout/SARIF output pipeline, same as every other `rules.rs` rule; the `[hide-delegate]`
  message prefix keeps output greppable per the convention `docs/syntax-rules.md` already documents.
- **Metrics**: None emitted at runtime — no `rules.rs` rule emits metrics today. The de facto quality
  metric for this check is the corpus false-positive rate recorded during Phase 6's mandatory
  backtest (AC6), tracked in the PR description and, if confirmed false positives are found, in
  `docs/hide-delegate-false-positives.md`. **Acceptance threshold (per triad review's Product-lens
  gap — AC6 previously had no quantified number)**: no repo's-worth of findings should sit above a
  ~30% confirmed-false-positive rate at Phase 6.2 triage (a judgment-call ceiling, not a hard repo
  convention — no other `rules.rs` check has a precedent number to inherit, since none has
  published a target). Below that, ship with the gaps already documented (Story 3.1.1's `build()`
  terminal, ADR-001's known false-negative/false-positive directions); at or above it, exercise the
  Risk Control "narrow v1 scope" fallback below before merging, per pitfalls.md §5's precedent that
  a revision cycle is expected, not optional.
- **Alerts**: None — `Severity::Advisory`, non-blocking, matching every other `rules.rs` rule's
  default severity.

## Risk Control
- **Feature flag**: None finer-grained than the existing per-checker-name opt-out. A repo can
  disable `hide-delegate` only by disabling the whole `syntax-rules*` checker for its language via
  `.claude/inspect.json`'s `disabled` list (`docs/suppressing-checks.md`) — no `rules.rs` rule has
  its own independent flag today, and `hide-delegate` doesn't introduce one either, for consistency.
- **Rollback procedure**: Revert the single PR. Because `src/config.rs` needs zero changes
  (Tech Debt Disposition table above), rollback is a pure `src/rules.rs` + `docs/syntax-rules.md`
  revert with no config migration to unwind.
- **Staged rollout**: None formal — matches how `deep-nesting`/`flag-argument`/`unreachable-code`
  shipped directly into `default_checks()` in one PR. Phase 6's corpus backtest is the pre-merge
  staging gate; if it surfaces an unacceptable false-positive rate, the fallback is narrowing v1
  scope (e.g., dropping detection for one language, not just its allowlist) before merging, not a
  post-merge staged rollout mechanism this codebase doesn't have.

## Unresolved Questions
- The corpus false-positive rate (AC6) is unknown until Phase 6 actually runs. Per pitfalls.md §5's
  precedent (commit `82797d7` and five others), at least one false-positive-driven revision to the
  suppression heuristic should be budgeted as near-certain rather than treated as a plan risk to
  design away — Rust iterator chains (`servo/servo`, `BurntSushi/ripgrep`) are the most likely
  first source, per pitfalls.md §3/§5. This must be resolved (numbers recorded, any confirmed false
  positives fixed and logged to `docs/hide-delegate-false-positives.md`) before Phase 6's tasks are
  considered done — it does not block starting Phase 1-5's implementation work.
- No Python or Kotlin exemplar exists in the current backtest corpus (`docs/backtest-repos.md`
  explicitly notes this gap, cited in pitfalls.md §2). Phase 6's triage should state this gap
  explicitly rather than imply those two languages' detection was corpus-validated.
- **Pre-mortem.md P2 items, accepted as tracked follow-ups rather than blocking Phase 1-6 scope**:
  (a) Failure #3 — an AI agent could satisfy this check by relocating a chain behind a trivial new
  delegating wrapper instead of fixing the underlying coupling; Phase 6.1's transcript triage
  should treat this "relocate, not resolve" pattern as its own category, and the finding message
  (Task 1.3.1d) should note that a valid fix exposes new behavior on the base type, not merely
  forwards the chain. (b) Failure #4 — the per-language `fluent_allowlist` consts have no defined
  update owner or re-validation cadence; `docs/hide-delegate-false-positives.md` (opened only if
  Phase 6 finds confirmed false positives, Task 6.2.1d) should name a re-triage trigger (e.g. "when
  `docs/backtest-repos.md`'s corpus is refreshed, or at least annually") if and when that file is
  created. Neither is a P1 blocking Phase 1-5 implementation start; both are follow-up hygiene for
  whoever owns Phase 6 and beyond.

## Dependency Visualization

```
Phase 1: Core engine scaffolding
  (MAX_CHAIN_LINKS [provisional pending Phase 6], CATALOG entry, LangRuleConfig fields
   w/ no-op defaults, walk_chains/check_chain, wired into SyntaxRulesChecker::check())
        |
        v
Epic 2.1: Go only (pulled forward — see Phase 1.5 below)
        |
        v
Phase 1.5: Early validation checkpoint (pre-mortem.md Failure #1's cheap off-ramp)
  Smoke-test Go's hide-delegate against one real corpus repo (kubernetes/kubernetes).
  Go/no-go: revise Phase 1/Epic 2.1 now if fundamentally broken, BEFORE sinking cost
  into 5 more languages.
        |
        v  (go decision)
Phase 2 (remainder): Per-language wiring (5 epics, parallelizable after Phase 1.5)
  +-- 2.2 TypeScript ------+  (JS/TSX inherit automatically via
  +-- 2.3 Python ----------+   ..lang_config(Language::TypeScript))
  +-- 2.4 Java ------------+
  +-- 2.5 Kotlin ----------+
  +-- 2.6 Rust ------------+
        |
        v  (sequencing preference, not a hard technical dependency — per triad review's
           Engineering-lens gap: is_chain_suppressed only needs the LangRuleConfig field to
           exist, not real per-language allowlist data, so Phase 3's stub logic (Task 1.3.1c)
           could technically start in parallel with Phase 2; listed sequentially here because
           testing suppression meaningfully (Phase 4) does need real allowlists to exist first)
Phase 3: Suppression heuristic
  (BUILDER_VERB_PREFIXES, has_builder_verb_prefix, real is_chain_suppressed)
        |
        v  (needs Phase 2 + 3 complete: real node kinds + real suppression)
Phase 4: Tests
  4.1 harness -> 4.2 per-language flagged/suppressed pairs (7 langs, incl. the
              two_hops_not_flagged boundary test) -> 4.3 edge-case tests
              (this-qualified, index boundary, lambda body)
        |
        v
Phase 5: Docs (docs/syntax-rules.md)
        |
        v
Phase 6: Backtest and validation (AC6 — mandatory before "done"; blocks merge, not a
  fast-follow — see Risk Control)
  6.1 session-transcript backtest -> 6.2 public corpus backtest + triage
      (includes kibitzer's own repo as a backtest target, per pre-mortem.md Failure #2)
      (may loop back into Phase 2/3 if false positives are confirmed)
```

---

## Phase 1: Core Engine Scaffolding

### Epic 1.1: Threshold, catalog, and description wiring
**Goal**: Register `hide-delegate`'s identity (const threshold, catalog metadata, checker
description string) before any walk logic exists, so later phases have a stable target to wire
findings into.

#### Story 1.1.1: `MAX_CHAIN_LINKS` const and `hide-delegate` `CATALOG` entry
**As a** kibitzer maintainer, **I want** the chain-depth threshold and rule metadata declared the
same way every other `rules.rs` rule's are, **so that** `hide-delegate` is indistinguishable in
shape from `deep-nesting`/`long-function` for future readers.
**Acceptance Criteria**:
- AC2/AC7: The threshold is a named `const`, not a magic number, and a `RuleMeta` entry exists in
  `CATALOG` with category, id, description, and `Severity::Advisory`.
  - *Given* `src/rules.rs`'s existing `CATALOG` array (`RuleMeta` entries for `long-function`,
    `deep-nesting`, `long-parameter-list`, `flag-argument`, `unreachable-code`), *When* a Go file
    contains `a.GetB().GetC().DoThing()` inside a function body, *Then* `check_chain` (Phase 1.3)
    computes a hop count of 3, compares it against `MAX_CHAIN_LINKS: usize = 2`, and — since 3 > 2
    — pushes a `Finding` whose message starts with `[hide-delegate]`, the same id string as the
    new `CATALOG` entry's `id: "hide-delegate"`.
**Files**: `src/rules.rs`

##### Task 1.1.1a: Add `MAX_CHAIN_LINKS` const (~2 min)
- Add `const MAX_CHAIN_LINKS: usize = 2;` beside `MAX_NESTING_DEPTH`/`LONG_PARAM_LIST_COUNT`
  (`src/rules.rs:10-18`), with a doc comment stating it's the max *allowed* hop count (flagged
  when hop count exceeds it, i.e. 3+ dot-accesses per AC2). **Per pre-mortem.md Failure #5 (P1)**:
  the doc comment must also state this value is *provisional pending Phase 6's corpus
  backtest* — chosen by shape-analogy to `MAX_NESTING_DEPTH`, not data-driven calibration against
  real dot-chain density — so a later change to this constant is expected maintenance, not a
  breaking change to a locked contract.
- Files: `src/rules.rs`

##### Task 1.1.1b: Add `hide-delegate` `RuleMeta` entry to `CATALOG` (~2 min)
- Append to the `CATALOG` array (`src/rules.rs:34-65`): `id: "hide-delegate"`,
  `category: "design"` (matches `flag-argument`'s category — both are Fowler refactorings),
  `description: "Expression chains 3+ dot-accesses (method calls and/or field reads) — Law of
  Demeter / Fowler's Hide Delegate."`, `default_severity: Severity::Advisory`.
- Files: `src/rules.rs`

##### Task 1.1.1c: Add `hide-delegate` to `SyntaxRulesChecker::description()` (~2 min)
- `description()` (`src/rules.rs:720-722`) currently lists "long-function, deep-nesting,
  long-parameter-list, flag-argument, unreachable-code" by name — append `, hide-delegate` so the
  doc string stays accurate (an easy-to-miss detail; nothing else references this string, but it's
  a repo convention worth keeping honest since it's the one place all 6 rules are named together
  outside `CATALOG`).
- Files: `src/rules.rs`

---

### Epic 1.2: `LangRuleConfig` scaffolding fields
**Goal**: Add the three new struct fields with safe no-op defaults across all 7 language configs
in one compiling step, so later per-language tasks (Phase 2) can replace one language's defaults
at a time without ever leaving the crate in a non-compiling state.

#### Story 1.2.1: Add fields + no-op defaults to every `lang_config()` arm
**Acceptance Criteria**:
- The crate compiles with `dot_chain_kinds: &[]`, `chain_step: no_chain_step`,
  `fluent_allowlist: &[]` in every language, before any real per-language logic exists.
  - *Given* the `LangRuleConfig` struct with the three new fields added, *When* `cargo build` is
    run, *Then* the build succeeds with all 7 `lang_config()` match arms (Go, TypeScript, Tsx,
    JavaScript, Python, Java, Kotlin, Rust) populated — Tsx/JavaScript inherit the no-op values
    automatically via their existing `..lang_config(Language::TypeScript)` spread.
**Files**: `src/rules.rs`

##### Task 1.2.1a: Add the three field declarations to `LangRuleConfig` (~4 min)
- Add `dot_chain_kinds: &'static [&'static str]`, `chain_step: fn(Node, &[u8]) -> Option<(Node<'_>,
  String)>`, `fluent_allowlist: &'static [&'static str]` to the struct (`src/rules.rs:72-144`),
  each with the doc comment text from the Domain Glossary above (explicitly noting
  `dot_chain_kinds` is distinct from the existing `chain_kinds` field to avoid the name collision
  architecture.md §2 flags).
- Files: `src/rules.rs`

##### Task 1.2.1b: Add `no_chain_step` no-op helper (~2 min)
- `fn no_chain_step(_node: Node, _src: &[u8]) -> Option<(Node<'_>, String)> { None }` — mirrors
  `no_panic_detector`'s existing shape (`src/rules.rs:448-450`).
- Files: `src/rules.rs`

##### Task 1.2.1c: Wire no-op defaults into all `lang_config()` arms (~5 min)
- Add `dot_chain_kinds: &[]`, `chain_step: no_chain_step`, `fluent_allowlist: &[]` to Go,
  TypeScript, Python, Java, Kotlin, and Rust's struct literals (`src/rules.rs:489-701`). Tsx and
  JavaScript need no edit — they inherit via `..lang_config(Language::TypeScript)`.
- Files: `src/rules.rs`

##### Task 1.2.1d: Verify the crate compiles (~2 min)
- Run `cargo build` and confirm success with no warnings about unused fields.
- Files: none (verification only)

---

### Epic 1.3: `walk_chains` / `check_chain` engine
**Goal**: Implement the whole-tree chain walk, hop-counting, and finding-emission logic once,
generically, so Phase 2's per-language work is purely data (node kinds, allowlists), not new
control flow.

#### Story 1.3.1: Whole-tree chain walk with outermost-node dedup, hop counting, and finding emission
**Acceptance Criteria**:
- AC1: The check runs with no `.claude/inspect.json` required, consistent with every other
  `rules.rs` check.
  - *Given* a fresh checkout of a target repo with no `.claude/inspect.json` present, *When*
    `kibitzer run <repo>` is invoked (which calls `config::default_checks()`, which already
    includes `syntax_rules_checks()`'s 8 `Check` entries per `src/config.rs:718-738`), *Then*
    `SyntaxRulesChecker::check()` calls `walk_chains` for every matching file exactly as it already
    calls `walk_declarations`/`walk_blocks`, and a Go file containing
    `a.GetB().GetC().DoThing()` produces a `[hide-delegate]` finding with zero project-level
    config — no new `Check`, `checker` name, or registry entry was needed (architecture.md §1).
**Files**: `src/rules.rs`

##### Task 1.3.1a: Implement `walk_chains` (~5 min)
- `fn walk_chains(node: Node, cfg: &LangRuleConfig, src: &[u8], findings: &mut Vec<Finding>)` —
  recurses the whole tree (sibling shape to `walk_blocks`, `src/rules.rs:758-766`); at a node whose
  `kind()` is in `cfg.dot_chain_kinds`, calls `check_chain` only if the node's `parent()` is either
  `None` or has a `kind()` not in `cfg.dot_chain_kinds` (the "outermost node of its own chain"
  dedup rule, architecture.md §2) — then recurses into every child regardless.
- **Stack-safety note (per triad review's Engineering-lens gap)**: no `rules.rs` walk today
  (`walk_blocks`, `max_nesting_depth`) has an explicit recursion-depth guard — this plan follows
  that existing precedent rather than introducing a new one unilaterally, since `tree_sitter::Node`
  recursion depth is bounded by source nesting depth, which real parsers already limit upstream
  (tree-sitter itself rejects pathologically deep input before this walk ever sees it). Treated as
  an accepted, pre-existing repo-wide risk class, not a `hide-delegate`-specific gap to solve here;
  Phase 6's corpus backtest (which includes minified/generated-code-heavy repos indirectly via the
  7-repo corpus) is the practical signal if this ever proves wrong in practice.
- Files: `src/rules.rs`

##### Task 1.3.1b: Implement `check_chain` hop collection (~5 min)
- `fn check_chain(node: Node, cfg: &LangRuleConfig, src: &[u8], findings: &mut Vec<Finding>)` —
  starting at `node`, repeatedly call `(cfg.chain_step)(current, src)`; on `Some((next, name))`,
  push `name` and set `current = next` only if `next.kind()` is still in `cfg.dot_chain_kinds`
  (otherwise `next` is the chain base — stop without descending further); on `None`, stop
  immediately (this outermost node wasn't a real hop, e.g. a bare `foo()` call). Hop count =
  number of names collected.
- Files: `src/rules.rs`

##### Task 1.3.1c: Forward-declare `is_chain_suppressed` stub (~3 min)
- `fn is_chain_suppressed(_names: &[String], _cfg: &LangRuleConfig) -> bool { false }` — a
  placeholder so `check_chain` compiles and is testable end-to-end before Phase 3 replaces it with
  the real allowlist/prefix logic. Document with a `// TODO(Phase 3)`-style comment pointing at the
  real implementation's future location.
- Files: `src/rules.rs`

##### Task 1.3.1d: Emit the `Finding` (~3 min)
- In `check_chain`, when hop count > `MAX_CHAIN_LINKS` and `!is_chain_suppressed(&names, cfg)`,
  push `Finding { line: node.start_position().row + 1, message: format!("[hide-delegate]
  expression chains {hop_count} dot-accesses (over {MAX_CHAIN_LINKS}) — add a method on the base
  object that exposes the needed behavior directly, rather than a wrapper that just forwards the
  same chain one level down") }` — same `[rule-id] ... — <suggestion>` shape as
  `[deep-nesting]`/`[long-function]` (`src/rules.rs:826-839`). **Per triad review's UX blocker**:
  the wording must distinguish the intended fix (a new method that exposes real behavior on the
  base type) from the "relocate, not resolve" anti-pattern pre-mortem.md Failure #3 warns an
  AI-agent consumer could otherwise reach for (a trivial one-line wrapper that just forwards the
  same chain, satisfying the linter without reducing coupling) — the phrase "rather than a
  wrapper that just forwards the same chain one level down" makes that distinction explicit in
  the message itself, not only in this plan's prose.
- Files: `src/rules.rs`

##### Task 1.3.1e: Call `walk_chains` from `SyntaxRulesChecker::check()` (~2 min)
- Add `walk_chains(tree.root_node(), &cfg, src, &mut findings);` after the existing
  `walk_declarations`/`walk_blocks` calls (`src/rules.rs:739-740`).
- Files: `src/rules.rs`

---

## Phase 1.5: Early Validation Checkpoint (blocks Phase 2.2-2.6)

**Why this phase exists**: `pre-mortem.md` Failure #1 (P1) — Phase 6's corpus backtest was
originally the *only* point where the core walk/threshold/suppression design meets real code, but
by then all 7 languages, 14+ unit tests, and docs would already be built around it. If the design
is fundamentally wrong (threshold, walk shape, suppression heuristic), that's 6x the rework cost
found as late as possible instead of as early as possible. This phase inserts a cheap early
off-ramp: wire *one* language fully (Epic 2.1, Go — chosen because architecture.md's Go AST
example is the most-verified of the seven), then smoke-test it against real code before sinking
cost into the other 5 languages (2.2-2.6).

### Epic 1.5.1: Smoke-test Go against real code before wiring the rest
**Goal**: Catch a fundamentally wrong threshold/walk/suppression design after 1 language's worth
of work, not after 6.

#### Story 1.5.1.1: Run the newly-wired Go check against one real corpus repo
**Acceptance Criteria**:
- The Go `hide-delegate` implementation (Epic 2.1, done first, ahead of 2.2-2.6) produces a
  plausible, not obviously-broken finding rate against real Go code, before any other language is
  wired.
  - *Given* Epic 2.1 (Go `chain_step`/`dot_chain_kinds`/empty `fluent_allowlist`) is complete and
    the crate builds, *When* `kibitzer run <kubernetes/kubernetes checkout, or another already-
    cloned Go corpus repo>` is invoked (corpus clone per `docs/backtest-repos.md`/
    `scripts/clone-backtest-repos.sh` — reuse Phase 6's eventual clone if already present; clone
    just `kubernetes/kubernetes` now if not), *Then* `[hide-delegate]` findings are eyeballed for
    an obviously-unreasonable rate (e.g. firing on a large fraction of all method calls, which
    would indicate a threshold or walk-shape bug, not a suppression-heuristic tuning gap) —
    **this is a manual sanity spot-check (~10-15 min), not a full Phase 6 triage pass**.
- If the spot-check finds the design fundamentally broken (not just noisy), halt and revise Epic
  1.3 (`walk_chains`/`check_chain`) or `MAX_CHAIN_LINKS` before proceeding to Epic 2.2-2.6. If it
  looks reasonable (even if not perfectly tuned — tuning is still Phase 6's job), proceed.
**Files**: none (verification task, no source changes)

##### Task 1.5.1.1a: Clone or reuse one Go corpus repo (~3 min)
- `scripts/clone-backtest-repos.sh` (or reuse an existing clone) for `kubernetes/kubernetes` only —
  no need for the full 7-repo corpus this early.
- Files: none

##### Task 1.5.1.1b: Run `kibitzer run` and eyeball the finding rate (~10 min)
- `kibitzer run <path-to-kubernetes-clone>`, grep for `[hide-delegate]`, spot-check ~10-15 findings
  by hand for obvious over-firing (per pre-mortem.md Failure #5's boundary-calibration concern —
  note whether findings cluster suspiciously at exactly hop-count 3, which would be an early signal
  `MAX_CHAIN_LINKS = 2` is too aggressive, ahead of Phase 6's full triage).
- Files: none (verification)

##### Task 1.5.1.1c: Go/no-go decision (~2 min)
- If the spot-check is reasonable: proceed to Epic 2.2 (TypeScript) onward. If not: revise Epic 1.3
  or the threshold before continuing — this is the cheap off-ramp pre-mortem.md Failure #1 calls
  for, exercised after 1 language's cost, not 6.
- **Per triad review's Product-lens gap**: also fold in the cheapest available disconfirming-
  evidence check here rather than deferring it to Phase 6.1 — a quick manual grep of any locally
  available `~/.claude/projects/*/*.jsonl` transcripts for Law-of-Demeter-shaped complaints
  (chained-access rewrites, "reduce coupling"-style edits touching 3+ hop chains) as a ~5 minute
  sanity check that this check's premise has *some* real signal behind it, ahead of the full
  Phase 6.1 backtest. This does not replace Phase 6.1's formal pass — it's a cheap early look,
  consistent with Phase 1.5's purpose of surfacing a wrong bet before 5 more languages are built,
  not a substitute for the mandatory gate.
- Files: none (decision point)

---

## Phase 2: Per-Language Wiring

Each epic below replaces one language's Phase 1 no-op defaults with real `dot_chain_kinds`,
`chain_step`, and `fluent_allowlist` values. Node kinds are taken verbatim from
research/architecture.md §3's vendored-`node-types.json`-verified table — not re-derived here.
**Sequencing note (per Phase 1.5 above): Epic 2.1 (Go) is implemented and smoke-tested (Phase 1.5)
before Epics 2.2-2.6 (the remaining 5 languages) begin** — the epics are listed here in their
original numeric order for reference, but Go is pulled forward in execution order.

### Epic 2.1: Go
**Goal**: Wire Go's `selector_expression`/`call_expression` chain shape, with an empty
`fluent_allowlist` (Go has no comparably prevalent fluent-chaining stdlib idiom, per
architecture.md §4).

#### Story 2.1.1: Go `chain_step` + node kinds
**Acceptance Criteria**:
- Go chains are detected using the exact node/field names verified in architecture.md §3 (`operand`,
  `field` on `selector_expression`; `function`, `arguments` on `call_expression`).
**Files**: `src/rules.rs`

##### Task 2.1.1a: Implement `go_chain_step` (~5 min)
- For a `call_expression` node: if its `function` field is a `selector_expression`, return
  `Some((that selector's "operand" field, that selector's "field" field's text))`; otherwise
  return `None` (a bare call like `foo()`, not a chain hop). For a `selector_expression` node
  passed directly (a bare non-call field read): return `Some((operand, field text))` unconditionally.
- Files: `src/rules.rs`

##### Task 2.1.1b: Wire Go's `LangRuleConfig` fields (~2 min)
- Set `dot_chain_kinds: &["call_expression", "selector_expression"]`, `chain_step: go_chain_step`,
  `fluent_allowlist: &[]` in Go's `lang_config()` arm.
- Files: `src/rules.rs`

##### Task 2.1.1c: Verify compile (~2 min)
- `cargo build`.
- Files: none (verification only)

---

### Epic 2.2: TypeScript (JavaScript, Tsx inherit)
**Goal**: Wire TS's `member_expression`/`call_expression` shape and a Stream/Array/Promise-style
`fluent_allowlist`; Tsx and JavaScript pick this up automatically via their existing
`..lang_config(Language::TypeScript)` spread — no separate edit for them.

#### Story 2.2.1: TS/JS `chain_step` + fluent allowlist
**Acceptance Criteria**: TS/JS chains use `object`/`property` on `member_expression` and
`function`/`arguments` on `call_expression`.
**Files**: `src/rules.rs`

##### Task 2.2.1a: Implement `ts_js_chain_step` (~5 min)
- Same shape as `go_chain_step`, substituting `member_expression`'s `object`/`property` fields for
  `selector_expression`'s `operand`/`field`.
- Files: `src/rules.rs`

##### Task 2.2.1b: Add `TS_JS_FLUENT_ALLOWLIST` const (~3 min)
- `const TS_JS_FLUENT_ALLOWLIST: &[&str] = &["map", "filter", "reduce", "flatMap", "forEach",
  "some", "every", "find", "findIndex", "sort", "then", "catch", "finally"];` — Array/Promise
  methods per architecture.md §4.
- Files: `src/rules.rs`

##### Task 2.2.1c: Wire TypeScript's `LangRuleConfig` fields (~2 min)
- Set `dot_chain_kinds: &["call_expression", "member_expression"]`, `chain_step: ts_js_chain_step`,
  `fluent_allowlist: TS_JS_FLUENT_ALLOWLIST` in TypeScript's arm only — Tsx/JavaScript inherit via
  the existing spread.
- Files: `src/rules.rs`

---

### Epic 2.3: Python
**Goal**: Wire Python's `attribute`/`call` shape with a deliberately empty `fluent_allowlist`.

#### Story 2.3.1: Python `chain_step`, empty allowlist stated as a v1 scope decision
**Acceptance Criteria**:
- AC3 (partial): Python is in v1 detection scope, with suppression quality explicitly narrower
  than Rust/Java/JS.
  - *Given* Python source `def f(a):\n    a.get_b().get_c().do_thing()\n`, *When* `walk_chains`
    processes it, *Then* a `[hide-delegate]` finding fires (detection works uniformly); *and Given*
    a pandas-style chain `df.groupby("x").agg("sum").reset_index()`, *When* the same walk runs,
    *Then* it is **also** flagged, because Python's `fluent_allowlist` is empty for v1 — pandas is
    a third-party ecosystem convention, not stdlib, and architecture.md §4 recommends against
    guessing at third-party names the way `bool_param_finder`'s precedent already avoids guessing
    at unverified conventions.
**Files**: `src/rules.rs`

##### Task 2.3.1a: Implement `py_chain_step` (~5 min)
- `call`/`attribute` node kinds, `function`/`object` and `attribute` fields, same shape as
  `go_chain_step`.
- Files: `src/rules.rs`

##### Task 2.3.1b: Wire Python's `LangRuleConfig` fields (~2 min)
- Set `dot_chain_kinds: &["call", "attribute"]`, `chain_step: py_chain_step`, `fluent_allowlist:
  &[]` — with a doc comment stating this is deliberately empty for v1, not an oversight (per the
  GWT above and architecture.md §4).
- Files: `src/rules.rs`

---

### Epic 2.4: Java (dual-kind fusion special case)
**Goal**: Wire Java's fused `method_invocation` (call+receiver in one node) alongside plain
`field_access`, plus a Stream/Optional `fluent_allowlist`.

#### Story 2.4.1: Java `chain_step` handling both `method_invocation` and `field_access`
**Acceptance Criteria**: A mixed field-and-method chain (`a.b.c().d`) is walked correctly across
both node kinds.
**Files**: `src/rules.rs`

##### Task 2.4.1a: Implement `java_chain_step` (~5 min)
- For a `method_invocation` node: return `Some((its "object" field, its "name" field's text))`
  directly — no unwrap step, since Java fuses receiver+call into one node (stack.md, "Java is the
  outlier"). For a `field_access` node: return `Some((its "object" field, its "field" field's
  text))`. One function handling both kinds, dispatched on `node.kind()`.
- Files: `src/rules.rs`

##### Task 2.4.1b: Add `JAVA_FLUENT_ALLOWLIST` const (~3 min)
- `const JAVA_FLUENT_ALLOWLIST: &[&str] = &["map", "filter", "collect", "reduce", "sorted",
  "distinct", "limit", "flatMap", "orElse", "stream", "boxed"];`
- Files: `src/rules.rs`

##### Task 2.4.1c: Wire Java's `LangRuleConfig` fields (~2 min)
- Set `dot_chain_kinds: &["method_invocation", "field_access"]`, `chain_step: java_chain_step`,
  `fluent_allowlist: JAVA_FLUENT_ALLOWLIST`.
- Files: `src/rules.rs`

---

### Epic 2.5: Kotlin (positional-field special case)
**Goal**: Wire Kotlin's field-less `navigation_expression`/`call_expression` using the existing
`kotlin_body`/`kotlin_params` positional-child precedent, plus the `also`/`apply` language-contract
exemption folded into the allowlist.

#### Story 2.5.1: Kotlin `chain_step` (positional) + allowlist including `also`/`apply`
**Acceptance Criteria**: Kotlin's `"fields": {}` grammar shape (confirmed in stack.md/
architecture.md §3.2) is handled the same way `kotlin_body`/`kotlin_params` already handle it.
**Files**: `src/rules.rs`

##### Task 2.5.1a: Implement `kotlin_chain_step` (~5 min)
- For a `navigation_expression` node: its children are positional (`[expression, identifier]`) —
  return `Some((first named child, last named child's text))`, mirroring `kotlin_body`'s existing
  "find by kind, not field name" pattern (`src/rules.rs:397-409`). For a `call_expression` node:
  its callee is the first positional child; if that callee's `kind()` is `navigation_expression`,
  delegate to the same extraction on it; otherwise return `None` (a bare call, chain base).
- Files: `src/rules.rs`

##### Task 2.5.1b: Add `KOTLIN_FLUENT_ALLOWLIST` const (~3 min)
- `const KOTLIN_FLUENT_ALLOWLIST: &[&str] = &["also", "apply", "map", "filter", "filterNot",
  "flatMap", "fold", "sorted", "distinct", "forEach", "associate", "joinToString"];` — `also`/
  `apply` are included here (not a separate bespoke code path) because their suppression *outcome*
  — unconditional exemption by accessor name — is identical to any other allowlist hit; their
  justification differs (a documented stdlib scope-function contract vs. a fluent-naming
  convention, architecture.md §4) but the mechanism doesn't need to, so this is a deliberate
  implementation simplification, not a loss of fidelity to the research.
- Files: `src/rules.rs`

##### Task 2.5.1c: Wire Kotlin's `LangRuleConfig` fields (~2 min)
- Set `dot_chain_kinds: &["navigation_expression", "call_expression"]`, `chain_step:
  kotlin_chain_step`, `fluent_allowlist: KOTLIN_FLUENT_ALLOWLIST`.
- Files: `src/rules.rs`

---

### Epic 2.6: Rust
**Goal**: Wire Rust's `field_expression`/`call_expression` shape and an Iterator/Option/Result
`fluent_allowlist`; confirm the `scoped_identifier` static-call exclusion falls out for free.

#### Story 2.6.1: Rust `chain_step` + fluent allowlist; static-path exclusion is a side effect, not a special case
**Acceptance Criteria**:
- AC4 (the requirement's own canonical example): a chain where the literal "same return type"
  reading would fail must still be suppressed via the allowlist.
  - *Given* Rust source `fn f(data: &[i32]) -> Vec<i32> {\n    data.iter().filter(|x| **x >
    0).map(|x| x * 2).collect()\n}\n`, *When* `check_chain` walks the outermost `call_expression`
    (the `.collect()` call), *Then* it collects 4 hop names (`collect`, `map`, `filter`, `iter`) —
    each of which changes the concrete type (`Iter<T>` → `Filter<...>` → `Map<...>` → `Vec<i32>`,
    so the literal "same return type as receiver" reading in requirements.md's AC4 text does
    *not* apply here) — and `is_chain_suppressed` returns `true` because every name is in
    `RUST_FLUENT_ALLOWLIST`, so **no** `[hide-delegate]` finding is emitted despite the hop count
    (4) exceeding `MAX_CHAIN_LINKS` (2). This is the exact requirement text's own example,
    correctly analyzed as allowlist-covered, not same-type — see ADR-001.
- Static/namespaced calls: `std::mem::size_of::<T>()`'s `scoped_identifier` function-field is never
  in `dot_chain_kinds`, so no chain is ever detected starting from it.
**Files**: `src/rules.rs`

##### Task 2.6.1a: Implement `rust_chain_step` (~5 min)
- `call_expression`/`field_expression`, fields `function`/`value`, `field` — same shape as
  `go_chain_step`. When a `call_expression`'s `function` field is a `scoped_identifier` (not a
  `field_expression`), return `None` — not a special case added for the static-call edge case, it
  falls out naturally because `scoped_identifier` is simply never one of the two kinds this
  function checks for.
- Files: `src/rules.rs`

##### Task 2.6.1b: Add `RUST_FLUENT_ALLOWLIST` const (~3 min)
- `const RUST_FLUENT_ALLOWLIST: &[&str] = &["map", "filter", "filter_map", "flat_map", "and_then",
  "collect", "fold", "zip", "chain", "take", "skip", "enumerate", "rev", "sum", "count",
  "for_each", "iter", "into_iter", "ok", "ok_or", "ok_or_else", "unwrap_or", "unwrap_or_else",
  "unwrap_or_default", "map_err", "as_ref", "as_mut", "cloned", "copied"];` — **per triad review's
  Engineering-lens gap**: Epic 2.6's own goal text promises "Iterator/Option/Result" coverage, but
  the original list was Iterator-only; the added names (`ok`, `ok_or*`, `unwrap_or*`, `map_err`,
  `as_ref`/`as_mut`, `cloned`/`copied`) are the common `Option`/`Result` combinator vocabulary,
  closing that plan-internal inconsistency before Phase 2.6 lands rather than discovering it
  reactively in Phase 6 (this codebase's own `.child_by_field_name("x")?.kind()`-style chains are
  exactly this shape, per pre-mortem.md Failure #2).
- Files: `src/rules.rs`

##### Task 2.6.1c: Wire Rust's `LangRuleConfig` fields (~2 min)
- Set `dot_chain_kinds: &["call_expression", "field_expression"]`, `chain_step: rust_chain_step`,
  `fluent_allowlist: RUST_FLUENT_ALLOWLIST`.
- Files: `src/rules.rs`

---

## Phase 3: Suppression Heuristic

### Epic 3.1: Builder-verb-prefix matching
**Goal**: Replace Phase 1's `is_chain_suppressed` stub with the real two-part heuristic
(allowlist OR prefix), completing AC4.

#### Story 3.1.1: `BUILDER_VERB_PREFIXES` + case-insensitive, word-boundary-aware prefix matcher
**Acceptance Criteria**:
- AC4: The full suppression heuristic — a chain is suppressed only when *every* hop's name is
  either in the language's `fluent_allowlist` or matches a builder-verb prefix.
  - *Given* Java source `Builder builder = new Builder();\nObject o =
    builder.setName("a").setAge(1).setActive(true);\n`, *When* `check_chain` collects hop names
    `["setActive", "setAge", "setName"]` (order is outermost-to-base but irrelevant to the check),
    *Then* `has_builder_verb_prefix` returns `true` for all three (`set` prefix, case-insensitive,
    followed by an uppercase letter — `setName` matches, but a hypothetical `andrew` would not,
    since the character after `and` is lowercase `r`), so `is_chain_suppressed` returns `true` and
    no `[hide-delegate]` finding is emitted despite 3 hops exceeding `MAX_CHAIN_LINKS`.
  - *Given* Go source `c.WithTimeout(5).WithRetries(3).WithHost("x")`, *When* the same check runs,
    *Then* `has_builder_verb_prefix` matches `With` case-insensitively against the `with` prefix
    entry (Go's exported-method capitalization doesn't defeat the match), so this chain is also
    suppressed — demonstrating the prefix heuristic is shared/language-agnostic, unlike
    `fluent_allowlist`.
  - **Documented known gap** (stated here per the "no fix without root cause" principle, not
    silently patched): a chain ending in a non-prefixed terminal call, e.g.
    `new Builder().setX().setY().build()` (hop names `build`, `setY`, `setX`), is **not**
    suppressed under this literal "every hop must match" rule, because `build` matches neither the
    allowlist nor any prefix. This is an intentional, documented limitation, not an oversight: per
    Design Position 8 above, ambiguity resolves toward flagging, and no prefix/suffix list was
    invented beyond what architecture.md §4's research explicitly specified. If the Phase 6 corpus
    backtest shows this is a frequent false-positive source, it becomes a triaged,
    evidence-backed follow-up (`docs/hide-delegate-false-positives.md`), not a pre-emptive guess.
**Files**: `src/rules.rs`

##### Task 3.1.1a: Add `BUILDER_VERB_PREFIXES` const (~2 min)
- `const BUILDER_VERB_PREFIXES: &[&str] = &["set", "with", "add", "put", "append", "and"];` with a
  doc comment stating this is shared/language-agnostic, unlike `fluent_allowlist`.
- Files: `src/rules.rs`

##### Task 3.1.1b: Implement `has_builder_verb_prefix` (~5 min)
- `fn has_builder_verb_prefix(name: &str) -> bool` — for each entry in `BUILDER_VERB_PREFIXES`,
  check `name.len() > prefix.len()` and `name[..prefix.len()].eq_ignore_ascii_case(prefix)` and the
  byte immediately after the prefix is an ASCII uppercase letter or `_` (covers `WithTimeout`,
  `withTimeout`, and Rust's `and_then`-style snake_case) — ruling out `andrew`/`android` matching
  `and`.
- Files: `src/rules.rs`

##### Task 3.1.1c: Implement the real `is_chain_suppressed` (~3 min)
- Replace Task 1.3.1c's stub: `names.iter().all(|n| cfg.fluent_allowlist.contains(&n.as_str()) ||
  has_builder_verb_prefix(n))`.
- Files: `src/rules.rs`

---

## Phase 4: Tests

### Epic 4.1: Multi-language test harness

#### Story 4.1.1: `check_hide_delegate` harness
**Acceptance Criteria**: A shared harness exists so all 7 per-language test pairs (Phase 4.2) can
be written without duplicating parser setup.
**Files**: `src/rules.rs`

##### Task 4.1.1a: Add `check_hide_delegate` test helper (~4 min)
- `fn check_hide_delegate(lang: Language, ts_lang: tree_sitter::Language, src: &str) ->
  Vec<Finding>` inside `mod tests` — parses `src` with the given grammar and calls `walk_chains`,
  mirroring `check_unreachable`'s existing shape exactly (`src/rules.rs:1017-1029`).
- Files: `src/rules.rs`

---

### Epic 4.2: Per-language flagged + suppressed test pairs

#### Story 4.2.1: Go
**Acceptance Criteria**:
- AC5: `{lang}_flags_hide_delegate`-style tests exist per language, covering both a flagged chain
  and a suppressed builder-pattern chain.
  - *Given* the source `"package main\nfunc f(a A) {\n\ta.GetB().GetC().DoThing()\n}\n"`, *When*
    `check_hide_delegate(Language::Go, tree_sitter_go::LANGUAGE.into(), src)` runs, *Then* the
    returned findings contain one whose `message` contains `"[hide-delegate]"` (3 hops: `GetB`,
    `GetC`, `DoThing`, none allowlisted or prefixed).
  - *Given* the source `"package main\nfunc f(c *Config) {\n\tc.WithTimeout(5).WithRetries(3
    ).WithHost(\"x\")\n}\n"`, *When* the same harness runs, *Then* no finding contains
    `"[hide-delegate]"` (3 `With`-prefixed hops, suppressed via `has_builder_verb_prefix`).
**Files**: `src/rules.rs`

##### Task 4.2.1a: `flags_hide_delegate` (Go, no prefix — matches `flags_deep_nesting`'s existing bare naming) (~3 min)
- Files: `src/rules.rs`

##### Task 4.2.1b: `go_suppresses_builder_chain` (~3 min)
- Files: `src/rules.rs`

##### Task 4.2.1c: `two_hops_not_flagged` (~3 min)
- **Added per validation.md's gap analysis** (AC2 requires an exact `> MAX_CHAIN_LINKS` boundary
  check — every other planned test uses 3+ hops, none exercises exactly 2). Mirrors this
  codebase's existing exact-boundary test convention (`five_params_is_not_long`,
  `LONG_PARAM_LIST_COUNT`'s own boundary test).
- Source: `"package main\nfunc f(a A) {\n\ta.GetB().DoThing()\n}\n"` — 2 hops (`GetB`, `DoThing`),
  neither allowlisted nor prefix-matched.
- Assertion: `check_hide_delegate(Language::Go, tree_sitter_go::LANGUAGE.into(), src)` returns no
  finding containing `"[hide-delegate]"` — hop count 2 is not `> MAX_CHAIN_LINKS` (2).
- Per pre-mortem.md Failure #5: also note in this test's doc comment that `MAX_CHAIN_LINKS` is
  provisional pending Phase 6, so a future threshold change is expected maintenance to this test's
  fixture, not a break of a locked contract.
- Files: `src/rules.rs`

#### Story 4.2.2: TypeScript
##### Task 4.2.2a: `ts_flags_hide_delegate` (~3 min)
- Source: `"function f(doc: Document) {\n  doc.querySelector(\"a\").closest(\"div\").dataset;\n}\n"`
  — 3 hops (`querySelector`, `closest`, `dataset`), none allowlisted → flagged. (This is
  features.md edge case 12's `document.querySelector(...).closest(...).dataset` example —
  intentionally chosen to document that this technically-correct-but-arguably-noisy DOM-chain case
  is **not** exempted in v1; tracked as a corpus-backtest risk, not pre-emptively special-cased.)
- Files: `src/rules.rs`

##### Task 4.2.2b: `ts_suppresses_fluent_chain` (~3 min)
- Source: `"function f(xs: number[]) {\n  xs.filter(x => x > 0).map(x => x * 2).sort();\n}\n"` — 3
  hops, all in `TS_JS_FLUENT_ALLOWLIST` → suppressed.
- Files: `src/rules.rs`

#### Story 4.2.3: JavaScript
##### Task 4.2.3a: `js_flags_hide_delegate` (~3 min)
- Source: `"function f(a) {\n  a.getB().getC().doThing();\n}\n"` — flagged.
- Files: `src/rules.rs`

##### Task 4.2.3b: `js_suppresses_fluent_chain` (~3 min)
- Source: `"function f(xs) {\n  xs.filter(x => x > 0).map(x => x * 2).sort();\n}\n"` — suppressed
  (JS inherits TypeScript's `fluent_allowlist`).
- Files: `src/rules.rs`

#### Story 4.2.4: Python
##### Task 4.2.4a: `py_flags_hide_delegate` (~3 min)
- Source: `"def f(a):\n    a.get_b().get_c().do_thing()\n"` — flagged.
- Files: `src/rules.rs`

##### Task 4.2.4b: `py_prefix_heuristic_still_suppresses` (~3 min)
- Source: `"def f(b):\n    b.with_x(1).with_y(2).with_z(3)\n"` — 3 `with_`-prefixed hops suppressed
  via `has_builder_verb_prefix` even though Python's `fluent_allowlist` is empty, demonstrating the
  two suppression paths are independent (AC4's two-part heuristic applies to every language, even
  ones with an empty allowlist).
- Files: `src/rules.rs`

#### Story 4.2.5: Java
##### Task 4.2.5a: `java_flags_hide_delegate` (~3 min)
- Source: `"class C { void f(Msg m) {\n  m.getFoo().getBar().getBaz();\n} }\n"` — the textbook
  protobuf-accessor-chain case (features.md edge case 10) — flagged.
- Files: `src/rules.rs`

##### Task 4.2.5b: `java_suppresses_stream_chain` (~3 min)
- Source: `"class C { void f(List<Integer> xs) {\n  xs.stream().filter(x -> x >
  0).map(x -> x * 2).collect(Collectors.toList());\n} }\n"` — suppressed via
  `JAVA_FLUENT_ALLOWLIST` (`stream`, `filter`, `map` all listed; `collect` too).
- Files: `src/rules.rs`

#### Story 4.2.6: Kotlin
##### Task 4.2.6a: `kotlin_flags_hide_delegate` (~3 min)
- Source: `"fun f(a: A) {\n  a.getB().getC().doThing()\n}\n"` — flagged.
- Files: `src/rules.rs`

##### Task 4.2.6b: `kotlin_suppresses_also_apply_chain` (~3 min)
- Source: `"fun f(x: X) {\n    x.also { it.a() }.apply { it.b() }.map { it }\n}\n"` — 3 hops
  (`also`, `apply`, `map`), all in `KOTLIN_FLUENT_ALLOWLIST` → suppressed, exercising the `also`/
  `apply` stdlib-contract exemption alongside an ordinary allowlist entry.
- Files: `src/rules.rs`

#### Story 4.2.7: Rust
##### Task 4.2.7a: `rust_flags_hide_delegate` (~3 min)
- Source: `"fn f(a: &A) {\n    a.get_b().get_c().do_thing();\n}\n"` — flagged (the
  `getNext().getNext().getNext()`-style traversal Lieberherr et al.'s own paper calls the textbook
  Law-of-Demeter violation — features.md/pitfalls.md §4 — deliberately chosen to confirm this
  shape is never accidentally suppressed by name-repetition).
- Files: `src/rules.rs`

##### Task 4.2.7b: `rust_suppresses_iterator_chain` (~3 min)
- Source: `"fn f(data: &[i32]) -> Vec<i32> {\n    data.iter().filter(|x| **x >
  0).map(|x| x * 2).collect()\n}\n"` — requirements.md's own canonical example (4 hops, all in
  `RUST_FLUENT_ALLOWLIST`) — suppressed. Directly verifies AC4.
- Files: `src/rules.rs`

---

### Epic 4.3: Edge-case documentation tests
**Goal**: Encode Design Positions 1, 4, and 6 as executable tests, not just prose — so a future
change that silently alters this behavior fails CI instead of going unnoticed.

#### Story 4.3.1: `this`-qualified chains counted uniformly
##### Task 4.3.1a: `java_this_qualified_chain_is_still_flagged` (~3 min)
- Source: `"class C { void f() {\n  this.getA().getB().getC();\n} }\n"` — flagged, same as a
  stranger-qualified chain (Design Position 1).
- Files: `src/rules.rs`

#### Story 4.3.2: Indexed access breaks the chain at the index boundary
##### Task 4.3.2a: `go_index_expression_breaks_chain` (~3 min)
- Source: `"package main\nfunc f(a [][]B) {\n\ta[0].C[1].D()\n}\n"` — NOT flagged; only `.D()` is
  counted past the last index boundary (1 hop), documenting the deliberate undercounting from
  Design Position 4.
- Files: `src/rules.rs`

#### Story 4.3.3: Lambda body walked as its own separate chain
##### Task 4.3.3a: `java_lambda_body_chain_flagged_independently` (~4 min)
- Source: `"class C { void f(List<A> xs) {\n  xs.forEach(x -> x.getB().getC().getD());\n} }\n"` —
  asserts exactly one `[hide-delegate]` finding exists (the inner `x.getB().getC().getD()`, 3
  hops), even though the outer `xs.forEach(...)` chain alone (1 hop) would never trigger one —
  confirming Design Position 6's "no special pruning needed" claim.
- Files: `src/rules.rs`

---

## Phase 5: Documentation

### Epic 5.1: `docs/syntax-rules.md`

#### Story 5.1.1: Add `hide-delegate` to the rule catalog doc
**Acceptance Criteria**:
- AC8: `hide-delegate` is documented alongside `long-function`/`deep-nesting`/etc., since
  `default_checks()`'s doc comment enumerates the full default catalog by name.
  - *Given* `docs/syntax-rules.md`'s existing rule table (lines 28-34, one row per rule id) and
    per-language node-kind prose (lines 36-117), *When* a maintainer greps `docs/syntax-rules.md`
    for `hide-delegate` after this story lands, *Then* they find a table row (category `design`,
    severity `advisory`, threshold `> 2 hops`) and a per-language paragraph covering all 7
    languages' `dot_chain_kinds`/`chain_step` shapes and `fluent_allowlist` contents, matching the
    document's existing structure for the other 5 rules.
**Files**: `docs/syntax-rules.md`

##### Task 5.1.1a: Add `hide-delegate` row to the rule table (~4 min)
- Insert a row after `unreachable-code` (or in rule-landing order — matches existing convention):
  `| \`hide-delegate\` | design | advisory | > 2 hops (3+ dot-accesses) | Expression chains 3+
  dot-accesses (method calls and/or field reads) in one expression — Fowler's *Hide Delegate* /
  Law of Demeter. A chain is suppressed if every hop's accessor name is either a known
  stdlib-fluent method for that language or matches a builder-verb prefix (\`set\`/\`with\`/
  \`add\`/\`put\`/\`append\`/\`and\`) — a syntactic proxy for "fluent builder," not true type
  inference. |`
- Files: `docs/syntax-rules.md`

##### Task 5.1.1b: Add per-language node-kind paragraph (~5 min)
- Following the existing per-language prose block's style, add a `hide-delegate` paragraph per
  language covering: Go (`selector_expression`/`call_expression`, empty allowlist), TS/JS/Tsx
  (`member_expression`/`call_expression`, Array/Promise allowlist), Python (`attribute`/`call`,
  empty allowlist — v1 scope decision stated explicitly), Java (`method_invocation` fuses
  receiver+call, plus `field_access`, Stream/Optional allowlist), Kotlin (`navigation_expression`/
  `call_expression`, positional fields, `also`/`apply` + collection-method allowlist), Rust
  (`field_expression`/`call_expression`, Iterator/Option/Result allowlist, `scoped_identifier`
  static-call exclusion).
- Files: `docs/syntax-rules.md`

##### Task 5.1.1c: Add a "Known limitations" note (~4 min)
- Short paragraph or bullet list under the new prose covering Design Positions 1-5 from this plan
  (no `this`/`self` special-casing, no locally-constructed-receiver detection, static/namespaced
  calls only excluded where the grammar gives a distinct node kind, indexed access is a hard chain
  boundary, chains split across statements via an intermediate variable are out of scope).
- Files: `docs/syntax-rules.md`

##### Task 5.1.1d: Cross-reference note (~0 min, no new work)
- `SyntaxRulesChecker::description()`'s string update is already covered by Task 1.1.1c; no
  additional edit needed here.
- Files: none

---

## Phase 6: Backtest and Validation (AC6 — Mandatory Before "Done")

### Epic 6.1: Session-transcript backtest

#### Story 6.1.1: `kibitzer check backtest` against every `syntax-rules*` checker name
**Acceptance Criteria**:
- AC6 (partial): The check is backtested against session transcripts before being marked done.
  - *Given* real Claude Code session transcripts under `~/.claude/projects/*/*.jsonl`, *When*
    `kibitzer check backtest syntax-rules` (and the six other `syntax-rules-{typescript,tsx,
    javascript,python,java,kotlin,rust}` names) is run per `docs/backtesting.md`, *Then* the output
    is filtered for the `[hide-delegate]` prefix (per `docs/backtest-repos.md`'s isolation
    convention — `hide-delegate` has no checker of its own to target directly; it rides along
    under the `syntax-rules*` names, per pitfalls.md §2) and any obviously-wrong fires are noted.
**Files**: none (verification task, no source changes)

##### Task 6.1.1a: Run the backtest across all 7 checker names (~5 min)
- `kibitzer check backtest syntax-rules`, then repeat for `syntax-rules-typescript`,
  `-tsx`, `-javascript`, `-python`, `-java`, `-kotlin`, `-rust`; grep each output for
  `[hide-delegate]`. **Enforcement note (per triad review's Engineering-lens gap)**: this step
  needs local `~/.claude/projects/*/*.jsonl` and has no CI equivalent, so AC6's "mandatory before
  done" framing is enforced by PR-review convention, the same way it is for every other `rules.rs`
  check's backtest requirement in this repo — not a gap specific to `hide-delegate`, but worth
  naming explicitly here since this plan leans on Phase 6 more heavily than most (per Design
  Position 7, it's the *only* evidence gate this check gets).
- Files: none

##### Task 6.1.1b: Record findings (~3 min)
- Note fire count and any obviously-wrong fires in the PR description (this feature has no
  dedicated `validation.md` — that artifact belongs to the SDD workflow's Phase 4, not this plan).
- Files: none

---

### Epic 6.2: Public repo corpus backtest + triage

#### Story 6.2.1: Run the checker against the 7-repo corpus and triage
**Acceptance Criteria**:
- AC6: False-positive rate on the corpus is recorded before the check is marked done.
  - *Given* the cloned corpus repos (`kubernetes/kubernetes`, `apache/cassandra`, `servo/servo`,
    `BurntSushi/ripgrep`, `denoland/deno`, `microsoft/vscode`, `stapler-squad`, per
    `docs/backtest-repos.md`), *When* `kibitzer run <repo>` is invoked for each (using
    `default_checks()`, so `hide-delegate` runs automatically per AC1 — no per-repo config needed),
    *Then* `[hide-delegate]` findings are collected and triaged via `scripts/backtest-triage.py`,
    and the resulting true/false-positive counts are recorded in the PR description, with Rust
    iterator chains in `servo/servo`/`BurntSushi/ripgrep` checked first as the most likely false-
    positive source (pitfalls.md §3/§5).
**Files**: `docs/hide-delegate-false-positives.md` (created only if confirmed false positives are found)

##### Task 6.2.1a: Ensure the corpus is cloned (~2 min)
- `scripts/clone-backtest-repos.sh`, per `docs/backtest-repos.md`. **Per pre-mortem.md Failure #2
  (P2)**: also run `kibitzer run .` from within the kibitzer checkout itself, in addition to the
  7 external corpus repos — `src/rules.rs` is exactly the Option/Result-chain-heavy Rust code this
  check's own `RUST_FLUENT_ALLOWLIST` needs to correctly not-flag, and it's the one codebase
  guaranteed to be scanned by this check on every future PR once merged (kibitzer dogfoods its own
  default checks).
- Files: none (verification)

##### Task 6.2.1b: Run `kibitzer run <repo>` per corpus repo and collect `[hide-delegate]` findings (~5 min per repo)
- Files: none (verification)

##### Task 6.2.1c: Triage with `scripts/backtest-triage.py` (~10 min)
- Per `docs/backtest-triage/README.md`. Explicitly note in the triage record that no Python or
  Kotlin exemplar exists in the corpus (`docs/backtest-repos.md`'s documented gap, pitfalls.md
  §2) — those two languages' detection is unit-tested (Phase 4) but not corpus-validated.
- Files: none (verification)

##### Task 6.2.1d: Fix confirmed false positives, if any (~10-20 min, scope depends on findings)
- If the triage confirms real false positives, open `docs/hide-delegate-false-positives.md`
  following the exact structure of `docs/go-error-context-false-positives.md` (mechanism section +
  documented scope gaps), and fix the underlying heuristic gap in `src/rules.rs`
  (`fluent_allowlist`/`has_builder_verb_prefix`/`dot_chain_kinds` as appropriate) before
  considering the check done. Budgeted as near-certain per pitfalls.md §5's precedent (commit
  `82797d7` and five others), not optional polish.
- Files: `docs/hide-delegate-false-positives.md`, `src/rules.rs` (if fixes are needed)

##### Task 6.2.1e: Record the corpus false-positive rate (~3 min)
- State `findings triaged false-positive / total findings` in the PR description, satisfying AC6's
  "false-positive rate ... is recorded" requirement.
- Files: none
