# Adversarial Review: hide-delegate-check

**Date**: 2026-09-22
**Verdict**: CONCERNS

Overall this is an unusually well-documented plan — every deviation from the literal AC4 text
is backed by an ADR, and the "ambiguity resolves toward flagging" philosophy is applied
consistently. Nothing found here rises to a true architectural blocker; the concerns below are
either cheap-to-fix data gaps or risks the plan itself already half-anticipates but doesn't quite
close.

## Blockers

None.

## Concerns

- [ ] **`RUST_FLUENT_ALLOWLIST` doesn't deliver what Epic 2.6's own goal promises.** Epic 2.6's
  goal text says the allowlist covers "Iterator/Option/Result adapters," but Task 2.6.1b's actual
  const (`map, filter, filter_map, flat_map, and_then, collect, fold, zip, chain, take, skip,
  enumerate, rev, sum, count, for_each, iter, into_iter`) contains zero Option/Result-specific
  names — no `unwrap`, `unwrap_or`, `unwrap_or_else`, `ok_or`, `map_err`, `is_some`, `is_none`,
  `is_ok`, `is_err`, `ok`, `err`, `expect`. A very common Rust idiom like
  `x.map_err(f).ok().unwrap()` (3 hops, none allowlisted) would misfire despite the epic claiming
  Option/Result is covered. This is a plan-internal inconsistency, not a deliberately deferred
  scope decision (unlike Python/Go's documented empty allowlists) — cheap to fix now by expanding
  the const in Task 2.6.1b, expensive to discover only after Phase 6's backtest flags it as noise
  on `servo`/`ripgrep`/this-repo's-own-code (all three are Rust-iterator-heavy per pitfalls.md
  §3). Recommend expanding the const before implementation or correcting the epic goal text.

- [ ] **The `build()`-terminal gap undercuts AC4's practical purpose for the most common builder
  shape.** Story 3.1.1 and ADR-001 both explicitly document that `new Builder().setX().setY()
  .build()` is *not* suppressed, because `build` matches neither the allowlist nor a builder-verb
  prefix, and ADR-001 itself calls this "the single most common real-world builder shape." AC4's
  stated purpose is suppressing "the expected false-positive class" of fluent builders; leaving
  the single most common instance of that class unsuppressed, by design, on a default-on
  no-opt-in Advisory check, is a real gap — not necessarily disqualifying (it's honestly
  documented and has a stated backtest-driven follow-up path), but likely to be the very first
  thing Phase 6's Cassandra-corpus run flags, given pitfalls.md §3 already calls out
  `apache/cassandra`'s "heavy use of builder patterns." Recommend either treating this as the
  known #1 expected Phase-6 finding in the PR description up front, or proactively handling the
  terminal-hop case (e.g., exempting a chain's final hop when every non-terminal hop already
  matches prefix/allowlist) rather than waiting for the reactive cycle.

- [ ] **The DOM-chain false positive (Task 4.2.2a) may not get real backtest coverage.** The plan
  unit-tests and explicitly accepts that `document.querySelector(...).closest(...).dataset` fires,
  "tracked as a corpus-backtest risk, not pre-emptively special-cased" — but the TS/JS corpus repo
  (`microsoft/vscode`) is an Electron app that mostly goes through its own UI abstraction layers,
  not raw DOM query chains, so Phase 6 may not actually exercise this FP class at real scale
  before the check ships to every JS/TS kibitzer user. Recommend either adding a
  DOM-manipulation-heavy sample to the backtest, or adopting pitfalls.md §3's own suggested
  mitigation (exempting chains rooted at `document`/`window`).

- [ ] **`walk_chains` is a plain recursive whole-tree walk with no depth guard, and chain
  expressions are exactly the AST shape most likely to blow a stack on generated/minified code.**
  Task 1.3.1a mirrors `walk_blocks`'s existing recursive shape (confirmed: no recursion-depth
  guard or `RUST_MIN_STACK` handling exists anywhere in `src/rules.rs` today). That's a
  pre-existing property of the file, but a single pathologically long chained expression
  (thousands of chained calls in one line, plausible in minified/bundled JS or generated query
  builders) nests one AST level per hop, unlike block nesting which rarely gets anywhere close to
  that depth in practice. `research/pitfalls.md`'s own "Generated code" section explicitly raises
  "worth checking whether hide-delegate needs the same generated-file exclusion treatment" the way
  `go-error-context`/`duplicate-code`/`file-complexity` needed after `82797d7` — the plan never
  answers that question. Recommend testing against a real minified/bundled file during Phase 6,
  or explicitly stating this crash risk is accepted/inherited rather than leaving it unaddressed.

- [ ] **Phase 6's "pre-merge gate" is enforced by convention only, and one of its two legs can't
  run in CI.** `plan.md`'s Risk Control section states "Phase 6's corpus backtest is the pre-merge
  staging gate," which does resolve the sequencing question the task brief raised — but
  Task 6.1.1a's transcript backtest depends on the implementer's own local
  `~/.claude/projects/*/*.jsonl`, which is developer-machine data with no CI equivalent. Nothing
  technically blocks merging without running it beyond the same social convention every other
  `rules.rs` check already relies on (not unique to this plan), but worth naming explicitly since
  `requirements.md` AC6 frames this as a hard "before being marked done" gate.

- [ ] **The other three non-empty allowlists (TS/JS, Java, Kotlin) look like partial first drafts
  rather than exhaustively reviewed lists**, budgeted-for via backtest per ADR-001 but worth
  naming as a pattern: Java's list omits `anyMatch`/`allMatch`/`findFirst`/`isPresent`/
  `orElseThrow`; JS/TS omits `flat`/`slice`/`concat`/`includes`; Kotlin includes `also`/`apply`
  but omits the parallel scope-function family `let`/`run`/`takeIf`/`takeUnless`, which share the
  identical stdlib-contract justification the plan already accepts for `also`/`apply`. Lower
  severity than the Rust gap above since it's explicitly the kind of thing Phase 6 is designed to
  surface, but the Kotlin omission specifically contradicts the plan's own stated rationale for
  including `also`/`apply` in the first place.

## Minors

- Task 1.2.1d's acceptance text ("confirm success with no warnings about unused fields") isn't
  literally achievable at that checkpoint: `fluent_allowlist` is set in every `lang_config()` arm
  by Task 1.2.1c but not read anywhere until Phase 3's real `is_chain_suppressed` (Task 3.1.1c)
  replaces the Phase 1 stub, which ignores `cfg` entirely. `cargo build` alone won't fail on this
  (only CI's `cargo clippy --workspace --all-targets -- -D warnings`, `.github/workflows/ci.yml:30`,
  denies warnings), and it's harmless if the whole plan lands as one PR/commit as the "Staged
  rollout" section implies — but worth flagging so the implementer isn't confused by a spurious
  warning mid-implementation, or reorders Task 1.2.1d's checkpoint after Phase 3.
- `research/architecture.md` §2 and its own summary (item 3) describe "two new `LangRuleConfig`
  fields," but the plan adds three (`fluent_allowlist` is the third, first introduced in §4's
  prose, not §2's field list). Not scope creep — `fluent_allowlist` is required to implement §4's
  own suppression recommendation, and the Pattern Decisions table justifies storing it as a table
  field rather than a parallel `match` well — but the plan never explicitly notes it's correcting
  architecture.md's own field count, which could confuse a future reader diffing research vs. plan.
- No explicit call-out of whether a lambda body's independently-flagged inner chain (Design
  Position 6 / Task 4.3.3a) could read as "double counting" the same source region in console
  output when a human skims findings — behavior is correct per design, purely a readability nit.
