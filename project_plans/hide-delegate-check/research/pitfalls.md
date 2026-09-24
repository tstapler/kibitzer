# Research: pitfalls and risks for `hide-delegate` (chain-depth / Law of Demeter check)

## 1. Evidence bar this check must clear

`docs/check-ideas.md` documents that this repo holds new check ideas to a
"confirmed transcript occurrence" bar (2+ independent occurrences of the same
agent mistake in real Claude Code session transcripts) before landing them.
**As of the current `docs/check-ideas.md`, that bar has never been met for any
idea** — two separate mining passes (free-text correction-phrase regex, and
structural Edit-immediately-followed-by-Edit churn) both came back null across
thousands of transcripts. `hide-delegate` is not in `check-ideas.md` at all —
it originates from `requirements.md`'s "backlog item," not from mined
evidence. Precedent (`duplicate-code`, logged in `check-ideas.md`) shows the
project has shipped checks "ahead of a confirmed transcript occurrence"
before, so this isn't a hard blocker, but the plan should say explicitly that
`hide-delegate` is going in without a transcript-evidence citation, consistent
with how `duplicate-code` was framed, rather than silently implying evidence
exists.

Practically, this means the **corpus backtest (`docs/backtest-repos.md`) is
the only real evidence gate available** for this check — there's no
transcript "does it fire on the thing it's meant to catch" signal to lean on,
only the "is it noisy" signal from running it at scale.

## 2. Mechanics of `kibitzer check backtest` and the corpus — what actually ties in

- `kibitzer check backtest <name>` (`docs/backtesting.md`) takes a checker
  **name from `kibitzer check list`** (i.e. the registered `Checker`/name in
  `src/checker.rs::registry()`, not the `[rule-id]` string in `rules::CATALOG`).
  For `deep-nesting` there is no `deep-nesting`-named checker — it's one rule
  inside the per-language `syntax-rules*` checkers (`syntax-rules`,
  `syntax-rules-typescript`, ... per `docs/syntax-rules.md`). So
  `hide-delegate` will **not** get its own backtest target; it rides along
  under `syntax-rules{,-typescript,-tsx,-javascript,-python,-java,-kotlin,-rust}`
  the same as every other `rules::CATALOG` entry, filtered after the fact by
  grepping the `[hide-delegate]` prefix in output (this is exactly how
  `docs/backtest-repos.md` says to isolate one rule's findings: `grep
  '\[flag-argument\]'`).
- The corpus workflow (`docs/backtest-repos.md`) is a whole-tree run, not a
  transcript replay: `find <repo> -name '*.go' | xargs -0 -n1 kibitzer check
  native syntax-rules` or `kibitzer run <repo>` using `default_checks()`. This
  means AC1 ("active with no `.claude/inspect.json` required... consistent
  with every other `rules.rs` check") is a hard prerequisite for the corpus
  backtest to even see the new rule — if it isn't wired into
  `config::default_checks()`, `kibitzer run <repo>` silently won't run it.
- Triage across repeated runs uses `scripts/backtest-triage.py`
  (`docs/backtest-triage/README.md`). Its `FINDING_RE` was, until commit
  `82797d7`, hardcoded to require a `[rule-id]` bracket — most *non-rules.rs*
  checkers don't emit one, which silently produced 0 findings for them. This
  is now fixed, but it's a reminder that the triage tooling's regex
  assumptions are a real failure mode to double-check against, not just the
  checker logic itself.
- The corpus (`docs/backtest-repos.md`) that actually exercises chain-heavy,
  multi-language code relevant to `hide-delegate`: `kubernetes/kubernetes`
  (Go — no fluent chains by convention, good true-negative-heavy baseline),
  `apache/cassandra` (Java — heavy use of builder patterns and stream chains),
  `servo/servo` + `BurntSushi/ripgrep` (Rust — iterator-chain-heavy,
  see §3), `denoland/deno` (Rust + TypeScript), `microsoft/vscode`
  (TypeScript — promise/thenable chains, fluent option builders). No Python
  or Kotlin exemplar exists yet in the corpus (`docs/backtest-repos.md`
  explicitly notes this gap) — if the plan claims Python/Kotlin coverage in
  AC3, there's no corpus repo to validate that claim against without adding
  one.

## 3. False-positive risk classes for chain-depth / Law-of-Demeter checks

Industry-known FP classes for this rule family (mirrored in most static
analyzers that implement it, e.g. PMD's `LawOfDemeter`, checkstyle style
mods), and their concrete manifestation in this repo's actual corpus:

- **ORM/query-builder DSLs** — `.where().orderBy().limit()` (any Java/Kotlin
  ActiveRecord-style API; in the corpus, Cassandra's query builder code).
  Each hop returns the *same* builder type, matching AC4's intended
  suppression signal — but only if the heuristic can actually detect that
  (see §5).
- **Test assertion DSLs** — `expect(x).to.have.property(...)`,
  `assertThat(x).isEqualTo(...).isInstanceOf(...)`. Common in
  `microsoft/vscode`'s and Cassandra's test trees. These often return a
  *different* wrapper type at each hop by design (`Assertion` →
  `ChainableAssertion` → ...) even though semantically it's the same
  "fluent" pattern the type-sameness heuristic is meant to protect — a
  same-type heuristic can misfire here in the false-positive direction if the
  intermediate types are related-but-not-identical.
- **Promise/Future chains** — `.then().then().catch()` (deno/vscode TS). Each
  `.then()` typically returns a *different* `Promise<T>` (generic parameter
  changes each hop) even though it's a single logical fluent operation — a
  literal same-return-type-text heuristic will likely see `Promise<A>` vs
  `Promise<B>` as different and fail to suppress, a false positive.
- **Rust iterator chains** — `.iter().filter().map().collect()`
  (`servo/servo`, `BurntSushi/ripgrep`, `denoland/deno`'s Rust side; also
  this very codebase, `src/rules.rs` itself uses iterator chains
  extensively). Each adaptor changes the concrete type
  (`Iter<T>` → `Filter<Iter<T>, F>` → `Map<Filter<...>, G>`) even though this
  is the single most idiomatic, encouraged Rust pattern and arguably the
  *worst* language/rule combination to ship without a strong exclusion —
  Rust has no runtime reflection and tree-sitter sees no type annotations on
  a `.filter(|x| ...)` call at all, so type-sameness is entirely unavailable
  here without extra special-casing (e.g. an explicit "well-known iterator
  adaptor method name" allowlist, a heuristic *not* in scope notes).
- **Generated code** — protobuf-generated builders (Java/Kotlin `Builder`
  classes), GraphQL codegen, Lombok `@Builder`. These are exactly the
  "returns same type" builder shape and should suppress cleanly if the
  heuristic works — but generated files are also exactly the kind of file
  the repo has already had to special-case for *other* checks
  (`go-error-context`, `duplicate-code`, `file-complexity` were all patched
  in `82797d7` to exclude generated files after corpus backtesting found
  them noisy there). `rules.rs` currently has no generated-file exclusion at
  all (not mentioned in `docs/syntax-rules.md`) — worth checking whether
  `hide-delegate` needs the same treatment the other checkers already
  learned to need.
- **Deeply nested but intentional JSX/DOM chains** — less relevant to a
  dot-chain-on-one-expression rule than to `deep-nesting`, but
  `document.querySelector(...).closest(...).dataset...` in JS/TS is a real,
  common, and legitimate 3+ hop chain with no shared return type at all
  (`Element` → `Element` → `DOMStringMap`) — a genuine Demeter violation by
  the letter of the rule, but one nobody would ask an LLM agent to "fix" by
  wrapping in a delegate method. This is a case where the rule is
  *technically correct* but likely to read as noise to an agent/reviewer —
  worth flagging as a design question (is `document`/`window`-rooted access
  exempt, the way some Demeter-check implementations exempt chains starting
  from a known "fluent root"?).
- **Go idiom is comparatively low-risk**, as the task description already
  notes: Go's dominant error-wrapping/chaining idiom (`if err != nil { return
  fmt.Errorf(...) }`) isn't a dot-chain at all, and Go has no first-class
  builder-chaining convention as pervasive as Java/JS — `kubernetes/kubernetes`
  is likely to be the cleanest true-negative baseline in the corpus, and a
  spike in Go findings there would be a strong noise signal.

## 4. The "same return type" heuristic (AC4) — underspecified without real types

`requirements.md` AC4 flags this itself as "the key open design question."
Concretely, with only a syntactic tree-sitter AST (no type-checker, per the
repo's Non-goals), there is **no way to read a chain link's actual return
type** unless the language's grammar happens to expose an explicit type
annotation at the call site — which it essentially never does for a chained
method call (`a.b().c().d()` carries zero type annotations at any of the
three call nodes; type information exists only in `a`'s declaration and each
method's *declaration*, both of which may be in another file, another
package, or the standard library/a dependency — cross-file/whole-program
resolution is explicitly out of scope per Non-goals).

This leaves only weak, syntactic proxy signals, each with real failure
modes:

- **Same method name repeated across hops** (e.g. `.filter().filter()`,
  `.getNext().getNext().getNext()`) — the closest thing to "same type" tree-sitter
  can see without resolution. This is actively dangerous as a suppression
  signal: `a.getNext().getNext().getNext()` (linked-list/tree traversal via
  a repeated accessor) is the **textbook example of a Law of Demeter
  violation** in Lieberherr et al.'s own framing, cited in
  `requirements.md`'s Background section — using "same method name" as a
  proxy for "safe builder chain" would suppress exactly the pattern the
  check exists to catch. This is the single most important false-negative
  risk to design against explicitly.
- **Method-name convention heuristic** (`with*`/`set*`/short verbs like
  `where`/`limit`/`filter`/`map` vs. `get*`/noun-like accessor names) — a
  real, workable signal used by some existing Demeter linters, but it's a
  *naming* heuristic, not the *type* heuristic AC4 actually specifies; if the
  plan substitutes this for "same return type" it should say so explicitly
  rather than silently reinterpreting the acceptance criterion.
- **Declared return-type text match**, for languages whose grammar exposes an
  explicit return type on the *declaration* (Go, TS, Java, Kotlin, Rust) —
  only works if the declaration is resolvable in the same file (mirrors this
  repo's own `src/go_call_resolution.rs` precedent, see §5) — narrow, but at
  least grounded in a real signal rather than a name guess, and consistent
  with the repo's established "resolve narrowly, default to unresolved
  rather than guess" pattern.

**Which failure direction is worse, given Advisory severity + default-on +
no opt-in?** Two lines of evidence in this codebase point the same way:

1. `bool_param_finder`'s doc comment (`src/rules.rs:106-110`) states the
   existing design philosophy for exactly this kind of
   type-ambiguity-under-syntactic-AST problem: "Returns only parameters
   whose type is unambiguously a plain boolean... to keep the check
   low-false-positive" — i.e., when ambiguous, this codebase's convention is
   to **not act** (don't flag) rather than guess. Applied to `hide-delegate`,
   the analogous rule is: don't *suppress* on an ambiguous/unresolved
   type-sameness signal — flag it, and let the human/agent judge it.
2. `src/go_call_resolution.rs`'s doc comment states the same principle from
   the opposite side: "Every failure mode returns `None` rather than
   guessing, so a caller that only suppresses on `Some(false)` never gets a
   false suppression from a botched resolution." `go_ignored_error.rs` only
   suppresses a finding when resolution *positively confirms* the
   non-matching case; an unresolved call stays flagged. This is the direct
   precedent for AC4: only suppress the fluent-builder case on a **positive,
   narrow, resolvable** same-type confirmation; default to flagging when the
   heuristic can't determine an answer.

   Beyond precedent, there's a structural reason false negatives are worse
   specifically for this check: the repo's entire quality-assurance loop for
   a shipped checker — `docs/backtesting.md`'s corpus workflow,
   `scripts/backtest-triage.py`, the `report_false_positive` MCP tool
   (`docs/reporting-false-positives.md`), and the checker-specific
   `docs/<name>-false-positives.md` logs — is built to **surface and fix
   over-flagging**. There is no equivalent mechanism to detect
   under-flagging (a suppressed real Demeter violation produces no output at
   all, so nothing in this pipeline can ever notice it happened). Given
   that asymmetry, a heuristic that leans toward over-suppression is
   effectively unauditable by every quality process this repo has, while a
   heuristic that leans toward over-flagging is exactly the failure mode the
   repo already has working, low-friction infrastructure to catch and burn
   down (as it did for 7 checkers in one sweep, commit `82797d7`).

## 5. Precedent: a shipped native check found too noisy and revised after backtesting

Direct, on-point precedent exists — this is not a hypothetical risk:

- **`82797d7` — "fix: burn down 7 checkers' false positives from a corpus
  backtest sweep"** (`git log 82797d7`). After running
  `kibitzer-corpus-fp-triage` against `kubernetes/kubernetes` and
  `stapler-squad`, seven already-shipped checkers needed source changes:
  `go-blank-imports`, `go-primitive-obsession`, `go-error-context`,
  `comment-quality-go` (five separate mechanisms), `duplicate-code` /
  `file-complexity` (generated-file exclusion), and `go-ignored-error` (the
  `go_call_resolution.rs` module described in §4 was added *in this commit*,
  specifically to reduce false positives that a purely syntactic check had
  been producing). Two sub-cases were left explicitly open, documented as
  "needing real type/interprocedural analysis" rather than silently
  tolerated.
- **`1eab4f7` — "fix(comment-quality): fix six false positives found by a
  real 3-repo backtest"** and **`c45882e` — "docs(comment-quality): log two
  false positives from stelekit backtest (#70)"** — same pattern, different
  checker, showing this isn't a one-time event but the expected lifecycle of
  every native check in this repo.
- The `docs/<checker>-false-positives.md` convention
  (`docs/go-error-context-false-positives.md`,
  `docs/go-primitive-obsession-false-positives.md`, six others) exists
  specifically because this happens repeatedly enough to need a standing
  process (`docs/reporting-false-positives.md`), not an exception process.

**Implication for `hide-delegate`:** the plan should budget for this as a
near-certainty, not a risk to design away entirely — ship with the narrowest
defensible AC4 heuristic (only suppress on a positively-resolved same-type
signal, per §4), run the mandatory corpus + transcript backtests before
calling it done (per `CLAUDE.md`'s bar), and expect to open
`docs/hide-delegate-false-positives.md` and iterate at least once on real
corpus findings — Rust iterator chains (§3) are the most likely first fire,
given how pervasive they are in this codebase's own style and in
`servo`/`ripgrep`/`deno`.
