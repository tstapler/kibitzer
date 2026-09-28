# ADR-003: Requirements for a Third `hide-delegate` Attempt (Type-Aware, Not Syntactic)

**Date**: 2026-09-28
**Status**: Proposed — not yet started, no owner
**Related**: ADR-001 (allowlist proxy, killed), ADR-002 (root-cause analysis, killed the check
entirely), a third independent implementation (PR #106) that reproduced the same failure mode
under different exclusion heuristics and was reverted (PR #111) after cross-checking it against
ADR-002's own triage data.

## Context

Two independent attempts at `hide-delegate` have now failed for the same underlying reason.
ADR-002 diagnosed it precisely: a syntactic AST can't tell "reaching through a foreign
collaborator's internals" (a real Law of Demeter violation) apart from "unwrapping your own
owned/composed state or a well-known fluent API" (idiomatic) — both are
`identifier.Call().Call().Call()` with no distinguishing marker in the syntax alone.

PR #106 tried a different set of exclusions (self/this-rooted base, any-arg-present, PascalCase
base) instead of ADR-001's name allowlists, on the theory that structural signals might avoid the
"unbounded vocabulary" trap ADR-002 identified. Re-running it against ADR-002's own confirmed-false-
positive lines showed it didn't: 73% exact-line overlap on `ripgrep`, and on `kubernetes/kubernetes`
it produced *more* raw noise (3325 findings on the 365-file subset ADR-002 sampled, vs. their 512)
while still reproducing 76 of those exact lines. The non-overlapping findings on both sides are the
same failure class ADR-002 already named: zero-arg iterator chains, interior-mutability accessors,
client-go-shaped chains, config-drilling field chains.

**The pattern across all three attempts**: every fix has been a proxy for information the syntactic
AST doesn't carry — what a call's *return type* actually is, and whether that type belongs to the
chain's own package/composition or crosses into an unrelated collaborator's. ADR-002's Consequences
section named this directly: "closing it for real needs receiver-type information... a prerequisite
capability, not a tuning problem." This ADR specifies what that capability would need to look like,
and the validation gate a fourth attempt must clear *before* writing the detection logic — not after,
which is the mistake ADR-001's first pass made (per ADR-002's Context: "Phase 6's backtest was done
dishonestly-shallow... reported as 100% false positive, and the check was sent to review anyway").

## What's missing today

`ArchModel` (`src/arch_model.rs`) already tracks `SymbolNode` (types/methods/functions, no return
type), `CallEdge` (resolved caller→callee, no type info), and `TypeRelationEdge` (extends/implements,
unrelated to this problem). Nothing in the model records a function or method's *return type* — the
one piece of information every failed attempt has tried to approximate through name-based proxies.

## Proposed capability: resolved return-type edges

Extend `SymbolNode` (or add a parallel edge type, matching `CallEdge`'s/`TypeRelationEdge`'s
existing "raw text when unresolved, `SymbolNode::id` when resolved" convention) with each
`Function`/`Method` symbol's declared return type — resolvable only where the grammar gives it as
static, in-file syntax:

- **Go, Java, Kotlin, TypeScript, Rust**: return type is direct syntax (`func f() *Foo`, `Foo f()`,
  `fun f(): Foo`, `f(): Foo`, `fn f() -> Foo`) — extractable the same way `TypeRelationEdge`
  extraction already reads per-language declaration syntax.
  - Rust needs implicit-`Self`-return detection too (fluent builders returning `Self`/`&mut Self`) —
    that's the single most common "not a violation" shape.
- **Python, JavaScript**: no static return type in the general case (type hints are optional and
  frequently absent) — this capability, and by extension a real fix for those two languages, has no
  syntactic ceiling. Any fourth attempt should scope to the five statically-typed languages only and
  say so up front, rather than discover it mid-implementation.

This is a genuinely new, moderately large capability (per-language return-type parsing + resolution
across the existing `resolve_call_edges` machinery) — comparable in scope to the `type_edges` work
(issue #40 / PR #99), not a small addition to `rules.rs`.

## Proposed detection rule (once return types resolve)

A chain hop is a candidate Demeter violation only if its resolved return type is:

1. **Not** the enclosing type itself (rules out `Self`-returning fluent builders).
2. **Not** one of a small, closed set of generic/monadic wrapper constructors — `Option`/`Result`/
   `Iterator` (Rust), `Optional`/`Stream` (Java), nullable/`Sequence` (Kotlin), `Promise`/`Array`
   (TS) — rules out `.iter().enumerate()`, `.map_err(...).map(...)`, and similar combinator chains.
   This is a **type-constructor** allowlist, not a **method-name** allowlist — closed and small
   (single digits per language) rather than ADR-002's "any method name in any codebase's or
   dependency's public API," which is the actual property that made ADR-001's allowlist unbounded.
3. **Declared in a different package/module** than the chain's starting symbol's own package.

A chain flags only if **2 or more consecutive hops** meet all three conditions — crossing into an
unrelated package's types more than once, not merely calling one foreign getter.

### Known risk this doesn't resolve

Condition 3 alone will likely still misfire on `client-go`-shaped chains
(`informerFactory.Certificates().V1().CertificateSigningRequests()`): each hop's return type *is* a
distinct type in a distinct sub-package of the same client library, which is exactly the shape
condition 3 is meant to catch, and exactly the shape ADR-002 confirmed is idiomatic. Closing this
gap for real would need a fourth signal — something like "the crossed types are all reachable from
one first-party root type via a documented builder/fluent path" — which is either another
unbounded-vocabulary trap or needs a human-curated per-repo exemption list, at which point this is
no longer a zero-config `default_checks()` rule (see `docs/suppressing-checks.md`'s local-overlay
mechanism as the honest fallback if this turns out to be the ceiling).

## Mandatory validation gate before writing detection code

This is the single most important lesson from three failed attempts: **backtest the type-resolution
signal against ADR-002's own triage data before writing the chain-flagging logic at all.**

1. Build only the return-type resolution (no detection rule yet).
2. For every one of ADR-002's 786 confirmed-false-positive records (both jsonl files, not a
   sample), resolve each hop's return type and check condition 3 (different package) manually
   against the record's own file/line.
3. Compute, before writing a single line of `rules.rs` detection code: what fraction of those 786
   confirmed false positives would condition 3 alone have excluded? If it's not dramatically better
   than PR #106's exclusions (73%/2.3% overlap — see PR #111's revert commit for the exact numbers),
   stop here. Building the full detection rule on top of a return-type signal that doesn't clear
   this bar is the same mistake as ADR-001 and PR #106, just with more infrastructure first.
4. Only after that check passes: implement the detection rule, then run the *exhaustive* (not
   sampled) corpus backtest per `docs/backtest-repos.md` and `docs/backtesting.md`, holding to the
   same 30% false-positive ceiling `plan.md`'s Observability Plan already set.

## Decision

Not started. This ADR exists so a fourth attempt begins from this evidence and this gate, instead
of re-discovering ADR-001 through ADR-003's dead ends a third and fourth time. No owner, no
timeline — pick this up only if/when the return-type capability described above is worth building
for its own sake (e.g. another checker needs it), since building it purely to validate one rule idea
that may still fail step 3 above is a large investment for an uncertain payoff.

## Evidence

- ADR-001, ADR-002 (this project's own prior decisions).
- PR #106 (the reverted third attempt) / PR #111 (the revert, with the exact overlap numbers in
  its commit message).
- `docs/backtest-triage/{burntsushi-ripgrep,kubernetes-kubernetes}/hide-delegate.jsonl` — the 786
  manually-triaged records step 3 above must be checked against.
