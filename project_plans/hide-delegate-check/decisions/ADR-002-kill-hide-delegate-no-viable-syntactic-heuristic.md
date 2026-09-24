# ADR-002: Kill `hide-delegate` — No Syntactic Heuristic Clears the False-Positive Bar

**Date**: 2026-09-23
**Status**: Accepted — supersedes ADR-001
**Related**: ADR-001, `../implementation/plan.md`'s "Observability Plan" (the ~30%
false-positive ceiling) and "Risk Control" (the narrow-scope fallback this ADR invokes and
then goes beyond).

## Context

ADR-001 accepted a two-part syntactic suppression proxy (per-language stdlib-fluent
allowlist + a shared builder-verb-prefix match) as the best available approximation of
"locally-constructed receiver," explicitly deferring validation to `plan.md`'s Phase 6
corpus backtest (AC6) and budgeting at least one revision cycle as near-certain.

A first implementation pass shipped (commits `4daa48d`, `9f5b310`), but Phase 6's backtest
was done dishonestly-shallow: only 43 findings across two repos were sampled (25 on
`burntsushi-ripgrep`, 18 on `kubernetes-kubernetes`), reported as "100% false positive,"
and the check was sent to review anyway with uncommitted, undisclosed partial fixes in the
working tree. The review (see the backlog item's recorded verdict) correctly failed this on
AC6 grounds and flagged the dishonesty.

This session picked the branch back up, found the uncommitted partial fix (field-only-chain
exemption, root-hop exemption, a `CONSTRUCTOR_NAME_PREFIXES`/`SUFFIXES` heuristic for
`Foo::new()`/`Foo::builder()`/`Type::config()`-rooted chains, and expanded
`RUST_FLUENT_ALLOWLIST` entries), and re-ran the backtest **exhaustively** instead of
sampling:

- **`BurntSushi/ripgrep`** (idiomatic, well-regarded Rust corpus): all 249 new findings
  (274 total across the shard) individually read with full function context and triaged.
  **0 true positives, 249 false positives — 100% FP rate.** Full record:
  `docs/backtest-triage/burntsushi-ripgrep/hide-delegate.jsonl`.
- **`kubernetes/kubernetes`** (huge, widely-referenced real-world Go corpus): 18,397 total
  findings on the full tree — full triage is infeasible at that scale, so ~400+ findings
  were read in depth via stratified sampling across every major subsystem (`cmd/`, 15+
  `pkg/` subsystems, `staging/` client-go/apimachinery/apiserver/kubectl, `test/e2e`,
  `test/integration`, `vendor/`) plus every distinct chain shape (3 through 19+ hops).
  **0 true positives found anywhere.** 512 findings recorded with verdicts, all
  `false_positive`. Full record:
  `docs/backtest-triage/kubernetes-kubernetes/hide-delegate.jsonl`.

`plan.md`'s Observability Plan set a ~30% confirmed-false-positive ceiling at Phase 6.2
triage, below which the check ships with documented gaps, and at or above which the Risk
Control's "narrow v1 scope" fallback applies before merging. The actual number — **0
confirmed true positives across two languages and ~650+ manually-verified findings** — is
not a tuning gap to narrow scope around. It is total failure.

### Root cause

The check cannot syntactically distinguish a genuine Law-of-Demeter violation from
idiomatic real-world chaining, because **the two have the identical AST shape**:

| Genuine violation (what the check should catch) | Idiomatic real code (what it actually caught) |
|---|---|
| `a.getB().getC().doThing()` | `c.CoreV1().Pods(ns).Create(x)` (client-go) |
| `a.getB().getC().doThing()` | `f.debug_struct("X").field(...).finish()` (Rust `Debug` impl) |
| `a.getB().getC().doThing()` | `logger.V(1).Info(...)` (klog/logr structured logging) |
| `a.getB().getC().doThing()` | `metric.WithLabelValues(...).Inc()` (Prometheus) |
| `a.getB().getC().doThing()` | `self.wtr.borrow().supports_color()` (interior-mutability accessor) |

Both columns are `identifier.Call1().Call2(args).Call3(args)` — a chain of 3+ dot-accesses
with no syntactic marker distinguishing "reaching through an object graph you were merely
handed" from "navigating your own owned/composed state" or "using a well-known
generated/third-party fluent API." Distinguishing them requires knowing whether each hop's
*receiver type* is foreign or owned — real type/semantic information tree-sitter's
syntactic AST does not carry, and which `requirements.md`'s own Non-goals and ADR-001's
Rationale #4 already put out of scope (no cross-file/whole-program type resolution).

Every fix attempted in this branch's history was a **name-based** proxy for that missing
type information (allowlist membership, builder-verb prefixes, constructor-name
detection). Each closed a handful of instances (the working tree's fix, before this
session, had already eliminated 18 of the original 25 ripgrep false positives this way) but
none of them — nor any plausible extension of the same idea — closes the gap, because the
false-positive surface isn't a fixed, enumerable vocabulary. It's "any method name in any
codebase's or dependency's public API that isn't one of a few hundred hand-picked stdlib
combinator names." Widening the allowlist to cover client-go, klog, Prometheus, gomega,
protobuf, and every project's own test-builder DSL is exactly the "unbounded grab-bag"
ADR-001's Rationale #3 already identified as the failure mode two *narrow* proxies were
chosen to avoid — and even an unboundedly wide allowlist would still misclassify the
textbook violation `a.GetB().GetC().GetD().DoThing()` as safe the moment any of its hop
names coincidentally matched a whitelisted verb.

### Options considered

1. **Keep narrowing scope** (raise `MAX_CHAIN_LINKS`, drop languages) per `plan.md`'s Risk
   Control fallback. Rejected: the false-positive mechanism isn't chain-length- or
   language-specific — it reproduced identically in Rust and Go, across every chain length
   from 3 to 19 hops, and a higher threshold only shrinks the (already-empty) true-positive
   set alongside the false-positive one.
2. **Keep expanding the allowlists** with client-go/klog/Prometheus/gomega/protobuf entries
   and more constructor-name prefixes. Rejected: chases a moving, effectively unbounded
   target (every dependency and every project's own domain vocabulary), addresses symptoms
   one library at a time rather than the root cause, and the corpus evidence shows even
   generous widening still misses novel shapes (`M8` in the kubernetes triage: `st.From*`/
   `build*`-rooted test builders that are structurally identical to the already-exempted
   `st.Make*`/`New*` idiom but use different verbs).
3. **Ship it opt-in-only, disabled by default.** Rejected as disproportionate new scope:
   per `plan.md`'s own Risk Control section, no rule in `src/rules.rs` has an independent
   enable/disable flag today — every rule rides along automatically via `CATALOG` once
   registered, and the only existing opt-out is disabling an entire `syntax-rules*`
   checker (which would also silence `deep-nesting`/`long-function`/etc.). Building
   per-rule opt-in infrastructure to carry one rule with a demonstrated ~100% noise floor
   is solving the wrong problem.
4. **Kill the check.** Chosen — see Decision.

## Decision

**Revert `hide-delegate` entirely** (commits `4daa48d`, `9f5b310`, via `git revert`) rather
than ship it in any form. The check does not clear `default_checks()`'s implicit quality
bar — every other rule in `src/rules.rs` fires on real, actionable structural problems at a
low noise rate; this one found zero true positives in two large, independently-triaged,
real-world corpora.

## Consequences

- **Positive**: `src/rules.rs` and `docs/syntax-rules.md` return to their pre-feature
  state; no user of `kibitzer run`/`kibitzer check native syntax-rules-*` is exposed to a
  rule that would have produced thousands of noise findings on any codebase of
  `kubernetes/kubernetes`'s scale (18,397 findings on that repo alone).
- **Positive**: The corpus backtest data (`docs/backtest-triage/{burntsushi-ripgrep,
  kubernetes-kubernetes}/hide-delegate.jsonl`) and the mechanism analysis above are kept as
  a durable record, so a future attempt at this rule idea starts from this evidence instead
  of re-discovering the same dead end.
- **Negative**: The Law-of-Demeter/Hide-Delegate refactoring gap this backlog item was
  meant to close remains unaddressed by kibitzer. Per the Root Cause section above, closing
  it for real needs receiver-type information (is this hop's object foreign or owned?) that
  no native `rules.rs` check currently has access to — a prerequisite capability, not a
  tuning problem, and out of scope for a follow-up to this item without that capability
  existing first.
- **Follow-on (not scheduled, no owner)**: If kibitzer ever gains type-aware analysis
  (e.g., via a language server / semantic-analysis integration rather than tree-sitter's
  syntactic AST alone), revisit this idea then — the corpus data above defines exactly the
  bar a type-aware version would need to clear (correctly exempt every mechanism in the
  triage notes; correctly flag the textbook `a.GetB().GetC().DoThing()` shape they're
  syntactically indistinguishable from today).

## Evidence

- `docs/backtest-triage/burntsushi-ripgrep/hide-delegate.jsonl` — 274 records, all
  `false_positive`.
- `docs/backtest-triage/kubernetes-kubernetes/hide-delegate.jsonl` — 512 records, all
  `false_positive`.
- `../implementation/plan.md`, `../implementation/validation.md` — the original design and
  test-coverage plan, kept for historical record of the approach that was tried.
- ADR-001 — the suppression-heuristic design this ADR supersedes.
