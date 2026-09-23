# UX research: replace-magic-literal

Scope, per the brief: the CLI/message/documentation experience for a developer or
coding agent encountering this finding — not a GUI. Sources are cited by
`path:line` below; all are in this worktree.

## 1–2. Message format and job-to-be-done

### Existing tone/format convention

Every `SyntaxRulesChecker` finding self-prefixes `[rule-id]`, states the concrete
number the threshold was compared against, then gives an imperative fix
(`src/rules.rs:790-793`, `:826-829`, `:836-839`, `:848-851`, `:864-867`):

```
[unreachable-code] statement is unreachable — an unconditional `return` on line 12 ends this block first
[long-function] body spans 57 lines (over 40) — consider splitting it up
[deep-nesting] body nests 6 levels deep (over 4) — consider extracting a function or inverting a condition
[long-parameter-list] parameter list names 7 identifiers (over 5) — consider a config struct
[flag-argument] boolean parameter `dryRun` is branched on directly in the body — Fowler's Remove Flag Argument: split into two named functions or replace with a small enum
```

The multi-occurrence precedent is `duplicate-code` (a different checker, so its
message doesn't self-prefix — see accepting-findings.md's rule below), which
reports **one finding per repeated group**, anchored at the *last* occurrence's
line, and lists every occurrence's line number inline
([`src/duplicate_code.rs:113-121`](../../../src/duplicate_code.rs)):

```
6-line block repeated 3 times (lines 10, 42, 88) — consider extracting a shared function
```

### Recommended message

Follow both precedents together — `[replace-magic-literal]` self-prefix (this
check lands in `rules.rs`'s `SyntaxRulesChecker`, not a standalone checker) plus
`duplicate-code`'s "list every line" shape:

```
[replace-magic-literal] numeric literal `86400` appears 3 times (also lines 41, 129) — consider extracting a named constant
[replace-magic-literal] string literal "application/json" appears 4 times (also lines 8, 55, 102) — consider extracting a named constant
```

Emit **one finding per distinct repeated literal value per file** (not one per
occurrence — an N-times-repeated literal producing N findings would be noise,
the same reasoning `duplicate-code` already applies via its `covered_until`
dedup, `src/duplicate_code.rs:107-123`). Anchor at the first occurrence (unlike
`duplicate-code`'s last-occurrence anchor) — the first occurrence is the one a
developer or agent would naturally open first to start the extraction, and it's
the most stable line if later occurrences get edited. Quote the literal itself
(backticks for numeric, double-quotes preserved for string) and its exact repeat
count, and list every *other* line number in the same finding.

### Job-to-be-done

The answer is "give the agent everything it needs to do the extraction in one
pass," not "flag that a number recurs." A coding agent acting on this finding
needs to: (1) know the literal is safe to hoist (already implied by the finding
firing at all — see the named-constant exclusion in the requirements), (2) pick
a constant name, and (3) rewrite every occurrence. Step 3 is exactly where
telling only "this is repeated" forces a re-grep of the file for the literal's
raw text — wasted tool calls and a chance to miss an occurrence (e.g. a literal
appearing inside a string interpolation the naive grep misses, or one the agent
mis-copies). Naming every other line inline, the way `duplicate-code` already
does, removes that whole regrep step and gives a deterministic occurrence list
to edit against. This matters more here than for `long-function`/`deep-nesting`
(single-site fixes where "look at this one function" is already complete
information) and is exactly why `duplicate-code` set the precedent worth
copying rather than `long-function`'s.

## 3. Suppression/acceptance guidance for docs/syntax-rules.md

Read in full: `docs/suppressing-checks.md` and `docs/accepting-findings.md`.
Summary of the two levers and which one a maintainer hitting a table-driven-test
false positive should reach for:

- **`docs/suppressing-checks.md`** — config-based, in `.claude/inspect.json`.
  Two variants: (a) turn the whole default check off everywhere (`disabled`),
  or (b) re-declare the check with a `scope` exclusion glob to skip specific
  files/directories while keeping it everywhere else
  (`docs/suppressing-checks.md:48-64`). This is the right lever for a **whole
  fixture file** of intentionally repeated table-driven test data — e.g.
  `!**/*_test.go` or a narrower `!internal/testdata/**` pattern — since the
  false positive isn't one line, it's the file's entire nature.
- **`docs/accepting-findings.md`** — a checked-in `.kibitzer/accepted/*.json`
  file, one per accepted finding, for a **single specific line** that's a
  genuine, correctly-flagged tradeoff a maintainer has deliberately decided to
  keep (`docs/accepting-findings.md:20-40`). Per that doc's own worked
  guidance (`docs/accepting-findings.md:42-49`), because `replace-magic-literal`
  lives in a self-prefixing checker (`syntax-rules-*`), an acceptance entry uses
  `"rule": "replace-magic-literal"` — the bracketed id, not the checker name —
  matching how `flag-argument`/`long-function`/`deep-nesting` entries are keyed
  today.
- There is explicitly **no third option**: `docs/suppressing-checks.md:11-15`
  and `docs/accepting-findings.md:80-82` both state kibitzer has no inline
  per-line suppression comment (`// kibitzer:disable`) by design, which matches
  the requirements doc's correction that this project must not invent one.

Recommended `docs/syntax-rules.md` addition (alongside the note the other five
rules already get in that doc's body, `docs/syntax-rules.md:28-34`): a short
sentence after the catalog table pointing a maintainer who hits a table-driven
test's repeated literal false positive at *both* levers with the file-vs-line
split spelled out — e.g. "A whole fixture/table-driven-test file with
intentionally repeated literals should use a `scope` exclusion glob
(`docs/suppressing-checks.md`); one specific, deliberately-kept repeat should
use a `.kibitzer/accepted/` entry keyed `\"rule\": \"replace-magic-literal\"`
(`docs/accepting-findings.md`)." This is documentation-only — it doesn't need
(and per the requirements doc, must not add) a new mechanism.

## 4. Severity

**Advisory is right; don't deviate.** Evidence against a `Blocking` default:

- Every entry in `CATALOG` (`src/rules.rs:34-65`) — `long-function`,
  `deep-nesting`, `long-parameter-list`, `flag-argument`, `unreachable-code` —
  is `Severity::Advisory`.
- Zooming out to `config::default_checks()`'s full default catalog
  (`src/config.rs:601-646`), the *only* `Severity::Blocking` default in the
  entire repo is `markdown-link-integrity` (`src/config.rs:603-606`) — an
  unambiguous correctness defect (a link that 404s is simply broken, no
  judgment call). Every other smell/style default — including the two closest
  analogues here, `duplicate-code` and `primitive-obsession`
  (`src/config.rs:613-617`) — is Advisory.
- The requirements doc's own "Appetite" section names the honest open risk as
  false-positive rate at scale (test/fixture literal repetition, HTTP status
  codes, array indices) — a smell that self-admittedly has an unresolved
  false-positive profile is the opposite of a candidate for `Blocking`, which
  would fail a build/gate on every hit.
- "Cheapest to detect, best established" (the requirements doc's framing,
  quoting the backlog item) describes tooling maturity and effort-to-ship, not
  certainty of correctness for any single hit — it's not evidence for a
  stricter severity default. Nothing in the requirements or existing docs
  argues for treating this differently from its four `syntax-rules.md`
  siblings.

## Summary

- Message: `[replace-magic-literal] {numeric|string} literal {value} appears {N} times (also lines {…}) — consider extracting a named constant` — one finding per distinct literal per file, anchored at the first occurrence, listing every other occurrence's line number inline (following `duplicate-code`'s multi-line-list precedent, not `long-function`'s single-site style), because the job-to-be-done is handing a coding agent a complete, regrep-free occurrence list to act on.
- `docs/syntax-rules.md` should add one sentence pointing a maintainer at a `scope` exclusion glob (`docs/suppressing-checks.md`) for a whole table-driven-test/fixture file and at a `.kibitzer/accepted/` entry keyed `"rule": "replace-magic-literal"` (`docs/accepting-findings.md`) for one specific deliberately-kept repeat — no new suppression mechanism, matching both docs' explicit "no inline comment" stance.
- Severity should be `Advisory`, matching every other entry in `CATALOG` and every smell-style default in `config::default_checks()` (the sole `Blocking` default, `markdown-link-integrity`, is an unambiguous-correctness case this isn't); the "cheapest to detect" framing is about effort, not certainty, and the requirements doc's own open false-positive risk argues against anything stricter.
