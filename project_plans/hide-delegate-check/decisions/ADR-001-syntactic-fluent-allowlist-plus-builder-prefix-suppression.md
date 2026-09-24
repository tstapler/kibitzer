# ADR-001: Fluent-Builder Suppression Is a Two-Part Syntactic Proxy, Not "Same Return Type"

**Date**: 2026-09-22
**Status**: Accepted

## Context

`requirements.md`'s acceptance criterion 4 states the suppression rule literally: "a chain where
every intermediate call returns the same type as its receiver is not flagged." This is the single
most consequential open design question the requirements doc itself flags ("the key open design
question").

Two independent research passes (`research/stack.md`, `research/architecture.md` §4) confirm this
literal reading is not implementable: tree-sitter's syntactic AST carries no resolved types for a
chained call's return value in any of the 7 target grammars — type information lives only in
`a`'s declaration and each intermediate method's own declaration, both potentially in another
file, package, or a dependency, and cross-file/whole-program resolution is explicitly out of
scope per `requirements.md`'s Non-goals.

Worse, the requirement's own canonical false-positive example — `.filter().map().collect()` — does
not even satisfy its own literal criterion. A Rust `Iterator` chain changes concrete type at every
hop (`Iter<T>` → `Filter<I,P>` → `Map<Filter<...>,F>` → `Vec<U>`); a checker implementing "same
return type as receiver" literally would **not** suppress the requirement's own headline example.
The "same type" framing describes classic OOP builder/setter chains (`StringBuilder.append()`
returning `StringBuilder`), not iterator/stream fluent APIs, which are a materially different
false-positive class with no receiver-identity contract at all.

Three options were considered for what to build instead, given the literal criterion is both
unimplementable and self-contradictory:

1. **Declared-return-type text matching** for the subset of languages whose grammar exposes an
   explicit return-type annotation on the callee's *declaration*, resolved narrowly within the
   same file (mirroring `src/go_call_resolution.rs`'s existing narrow-resolution precedent).
2. **A single same-method-name-repeated heuristic** (`a.getNext().getNext().getNext()` looks
   "builder-like" because the name repeats).
3. **Two independent, narrow, per-hop syntactic proxies**: a per-language stdlib-fluent-method-name
   allowlist (covering the iterator/stream case the literal reading misses) plus a
   language-agnostic builder-verb-prefix check (covering the classic setter case the literal
   reading was actually describing), with Kotlin's `also`/`apply` folded into its allowlist as an
   unconditional stdlib-contract exemption.

## Decision

**Option 3: two independent, narrow, per-hop syntactic proxies — allowlist membership OR
builder-verb prefix, checked per hop, suppressing a chain only when every hop satisfies at least
one.**

## Rationale

1. **It's the only option that correctly handles the requirement's own example.** Option 1
   (declared-return-type matching) still fails on `.filter().map().collect()`, since each Rust
   iterator adaptor's declared return type is a distinct generic struct
   (`Filter<Self, P>`/`Map<Self, F>`), not a repeated name — same-return-type-text matching
   would see different text at every hop and never suppress. Option 3's allowlist directly
   targets accessor *names*, not return types, sidestepping the problem entirely for exactly the
   case that motivated the requirement in the first place.

2. **Option 2 (same-method-name-repeated) is actively dangerous, not just weak.** Per
   `research/pitfalls.md` §4: `a.getNext().getNext().getNext()` — a linked-list/tree traversal via
   a repeated accessor — is Lieberherr, Holland & Riel's own textbook example of a Law of Demeter
   violation, cited in `requirements.md`'s own Background section. Using "same name repeated" as a
   suppression signal would suppress exactly the pattern this check exists to catch. This is the
   single most important false-negative risk identified in research and rules the option out
   outright, not just as a weaker choice.

3. **Two narrow, independently-falsifiable proxies are safer than one broad, unfalsifiable one.**
   A single "does this look like a builder" heuristic invites scope creep (accumulating more and
   more name patterns until it becomes an unaudited grab-bag). Splitting into two named,
   bounded lists — a fixed per-language allowlist and a fixed 6-entry language-agnostic prefix set
   — means each can be independently reviewed, independently falsified by the corpus backtest
   (Phase 6 of `../implementation/plan.md`), and independently extended later without touching the
   other.

4. **Every ambiguous case defaults toward flagging, consistent with existing repo precedent.**
   `bool_param_finder`'s doc comment (`src/rules.rs:106-110`) states this codebase's convention
   explicitly for exactly this class of problem: when a type-ambiguity question can't be resolved
   from the syntactic AST, don't act (don't flag) rather than guess. `go_call_resolution.rs`
   applies the same principle from the suppression side: "every failure mode returns `None` rather
   than guessing, so a caller that only suppresses on `Some(false)` never gets a false suppression
   from a botched resolution." Applied here: a hop suppresses only on a positive, narrow match
   (allowlist hit or clean prefix match) — an unresolved/ambiguous hop stays flagged. This is also
   structurally forced by this repo's own tooling: `backtest-triage.py`/`report_false_positive`/
   `docs/<check>-false-positives.md` can detect and burn down over-flagging (as it did for 7
   checkers in commit `82797d7`), but nothing in the pipeline can ever notice under-flagging — a
   suppressed real violation produces no output at all. Given that asymmetry, erring toward
   over-suppression is effectively unauditable, while erring toward over-flagging is exactly the
   failure mode this repo already has low-friction infrastructure to catch.

## Consequences

- **Positive**: Directly and correctly suppresses `requirements.md`'s own headline example
  (`.filter().map().collect()`) — see `../implementation/plan.md` Story 2.6.1's Given-When-Then.
- **Positive**: Each of the two proxies is independently testable and independently extensible.
  Widening `RUST_FLUENT_ALLOWLIST` to cover a newly-noisy adaptor found in a future backtest touches
  one `const`, not shared logic.
- **Negative — known, documented false-negative direction**: A hand-rolled builder using neither a
  recognized stdlib name nor a `set`/`with`/`add`/`put`/`append`/`and` prefix (e.g. a builder whose
  setters are named `name(...)`/`age(...)` with no verb prefix at all, a common Rust builder idiom)
  is still flagged. Accepted per Rationale #4 — the false-negative direction is the safer failure
  mode for this specific check, and this gap is exactly the kind of thing the mandatory corpus
  backtest (plan.md Phase 6) is positioned to surface with real evidence rather than pre-emptive
  guessing.
- **Negative — known, documented false-positive direction**: A genuine object-graph traversal that
  happens to use a `set`/`with`-prefixed accessor name (e.g. `a.withB().withC()` reaching through
  unrelated types) is wrongly exempted. Stated explicitly in `docs/syntax-rules.md`'s
  `hide-delegate` row (plan.md Task 5.1.1a) and in `is_chain_suppressed`'s doc comment, not left
  implicit.
- **Negative**: A chain ending in a non-prefixed terminal call (`new Builder().setX().setY()
  .build()` — `build` matches neither list) is not suppressed, even though it's the single most
  common real-world builder shape. Documented explicitly in plan.md Story 3.1.1 as a known gap
  rather than silently patched with an unresearched extra rule (e.g. exempting the chain's final
  hop) that the research didn't specify and the corpus hasn't yet justified.
- **Follow-on**: If Phase 6's backtest shows the `build()`-terminal gap (or any other specific
  gap) fires often enough in the real corpus to be worth fixing, that becomes a
  `docs/hide-delegate-false-positives.md`-tracked, evidence-backed follow-up PR — not a
  pre-emptive change to this ADR's scope.

## Alternatives Rejected

See Context above (Options 1 and 2) and the Pattern Decisions table in `../implementation/plan.md`.
