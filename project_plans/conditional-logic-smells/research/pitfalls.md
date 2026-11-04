# Pitfalls and Risks: `decompose-conditional`, `consolidate-conditional`, `remove-control-flag`

Sourced from reading `src/checkers/rules.rs` (5,390 lines), `src/checkers/complexity.rs`,
`docs/backtesting.md`, `docs/backtest-repos.md`, `docs/extract-variable-false-positives.md`,
and `docs/syntax-rules.md`. All line numbers and constants verified against the source.

---

## 1. False-positive risks specific to each rule

### `decompose-conditional`

The planned approach applies `count_decision_points` (from `complexity.rs`) to the condition
sub-node of an `if`/ternary. On a condition, `count_decision_points` counts only `&&`/`||`
binary expressions (the `if`, `for`, and `case` branches of its match statement do not appear
inside a condition sub-expression). This makes it a **flat logical-operator count**, not a
nesting depth metric — different from `extract-variable`'s `expression_nesting`.

Idioms that produce legitimately high counts and should NOT be flagged:

- **Error multi-match chains**: `if errors.Is(err, io.EOF) || errors.Is(err, io.ErrUnexpectedEOF)` —
  each `||` counts as one decision point. These are idiomatic in Go and should not be consolidated
  into a single expression (they're already as simple as possible). kubernetes uses chains like
  `if isNotFound(err) || isTimeout(err) || isConflict(err)` throughout its retry logic; at a
  threshold of 4–5 such chains trigger routinely.
- **Guard chains in validation code**: `if a != nil && b != nil && c != nil && d != ""` — four
  `&&`s = 3 decision points. kubernetes and cassandra are full of nil-guard preconditions that
  are deliberately flat, not candidates for restructuring.
- **Table-driven test assertions**: `if tc.expectError && err == nil && tc.name != ""` — test
  setup conditions that cross the threshold are noise.
- **build-tag / generated-file guards**: conditions in auto-generated code that shouldn't be
  touched (the checker has no `is_generated` guard by default; `rules.rs` calls
  `crate::checkers::file_size::is_generated` in a few checkers but not in `SyntaxRulesChecker`
  itself — adding one for `decompose-conditional` is worth noting explicitly).
- **Type assertions with fallback**: `if v, ok := x.(T); ok && v.Field != nil` — idiomatic Go
  two-value type assertion that shouldn't be split.

**Threshold calibration evidence**: `docs/extract-variable-false-positives.md` records that
when `extract-variable` used a node-count metric, 2,200 findings were produced across the
corpus; switching to a nesting-depth metric with threshold 5 reduced this to ~220. The
analogous risk for `decompose-conditional` using a flat count threshold of 3 or 4 is high noise:
`a && b && c && d` (3 `&&`s) = 3 decision points and fires at threshold 3, yet this is a
completely ordinary guard condition. Starting at threshold 5 or 6 matches the calibrated
`extract-variable` precedent; below 4 is virtually certain to produce unacceptable noise.

---

### `consolidate-conditional`

The rule detects 2+ consecutive `if` statements in the same block each returning/assigning the
same value, with the suppression that no call expressions appear in the conditions. Even with
that suppression, high false-positive patterns remain:

- **Error guard chains**: `if err != nil { return nil, err }` repeated for sequential operations
  returns the same value (`nil, err`) with no call in the condition. The conditions have no call
  expressions, so the suppression does NOT fire. These blocks appear thousands of times in
  kubernetes, stapler-squad, and any idiomatic Go code. Consecutive `if err != nil { return nil,
  err }` pairs WILL fire the rule, yet consolidating them into a single combined condition
  (`if err1 != nil || err2 != nil`) is semantically wrong (the checks must be sequential; each
  guards the next operation). This is the **single largest false-positive risk** for this rule.

- **Nil-safe proto/struct field setters**: builder patterns like
  ```go
  if req.Name != "" { obj.Name = req.Name }
  if req.Value != 0 { obj.Value = req.Value }
  ```
  These assign different fields to different values, so they won't trigger the "same value" gate —
  but proto handling code that assigns the same sentinel (e.g., both branches assign an empty
  string default) can appear structurally identical while being semantically distinct.

- **Validation with identical early-return but different context**: a function validating
  multiple independent invariants where each failure returns the same error type (e.g., 
  `return ErrInvalid`) but under logically distinct conditions. Consolidating would merge unrelated
  validation into a single hard-to-read compound condition.

- **Switch-case emulation**: consecutive ifs that are effectively a hand-written type switch or
  value switch — go's style guidelines in some codebases prefer explicit if-chains over switch
  for readability.

The "no call expressions in conditions" suppression catches side-effectful conditions but does
**not** catch the error guard pattern above, which is the dominant false-positive shape in Go.
A secondary suppression — skip when the condition references the same variable as the return
expression (e.g., both condition `err != nil` and return `nil, err` share `err`) — would
substantially reduce noise for this case.

---

### `remove-control-flag`

The rule detects a boolean local variable written only as `= true`/`= false` inside a loop,
read only in the loop condition, and never read after the loop. Even with strict scope analysis,
genuinely needed variables can match this shape:

- **Result accumulation booleans**: `allOK := true; for ...; { if bad { allOK = false } };
  return allOK` — `allOK` is read AFTER the loop, not just in the loop condition, so the
  strict "never read after the loop" gate should exclude it. But if the checker's read-tracking
  is incomplete (e.g., it tracks identifier references in condition expressions but misses
  references in return statements or as function arguments), it will false-positive here.

- **Variables captured by deferred functions**: `var done bool; defer func() { if done { ... }
  }()` — the deferred closure captures `done` by reference. The identifier appears to be used
  only inside a loop body (written as `done = true`), but the deferred function reads it. A
  purely syntactic scope analysis (which a tree-sitter checker is) cannot see this.

- **Variables captured by goroutines launched in the loop**: `go func() { if found { process() }
  }()` inside a loop — a data race candidate where `break`-replacement would be unsafe. The
  checker cannot detect this without data-flow analysis.

- **Multi-level break target**: in Go, `break` only exits the innermost loop/switch. A boolean
  flag that breaks out of an outer loop (`outerLoop: for { for { if cond { done = true; break } }
  if done { break outerLoop } }`) cannot naively be replaced with `break outerLoop` without
  rewriting the control-flow structure. A per-file AST checker that sees only one level of loop
  cannot reliably verify that a `break` replacement is equivalent.

- **Found-flag with post-loop processing**: `var found bool; var target Item; for _, item :=
  range items { if item.Match() { found = true; target = item } }; if found { process(target) }`
  — the flag is used both as a loop condition (conceptually) and after the loop. Collapsing to
  `break` changes semantics because `target` must also be captured.

Go-first scope is appropriate here — these patterns are most acute in Go where labeled breaks
are the only multi-level break mechanism, closures capture by reference, and defer is a
first-class feature.

---

## 2. `rules.rs` size and the `LONG_FUNCTION_LINES` self-enforcement risk

`src/checkers/rules.rs` is **5,390 lines** (verified with `wc -l`). The test section begins at
line 2,373; the production code runs from line 1 to roughly 2,370.

`LONG_FUNCTION_LINES = 40` is defined at line 16 and enforced by the checker on itself. The
comment at line 83 explicitly documents that literal-kind constants were hoisted out of
`lang_config()` bodies "so adding them didn't push these constructors' bodies over
`LONG_FUNCTION_LINES` — the same threshold this rule catalog enforces on every other codebase."
This is live self-enforcement, not aspirational.

The key functions to watch:

- `check_declaration` (line 2021): currently ~54 lines of production logic. It already exceeds
  `LONG_FUNCTION_LINES` — the checker would fire on its own file if pointed at itself. Adding
  new per-declaration checks (`decompose-conditional`, `consolidate-conditional`) here would
  make it longer still.

- `walk_declarations` and `walk_blocks` are dispatch entry points; each `lang_config()`
  constructor is already pushed to multiple table-initialization helper constants (the hoisted
  `GO_LITERAL_KINDS`, etc.) to stay under 40 lines.

Adding three rules to `SyntaxRulesChecker` via `rules.rs` creates real pressure to either:
(a) Keep each new check in its own function that `check_declaration`/`walk_blocks` delegates to,
OR (b) Split the new rules into separate files the way `go_error_context.rs`, `go_blank_imports.rs`,
etc. are structured — which is probably the right call if any new check requires non-trivial
logic (especially `remove-control-flag`, which needs variable-write tracking).

The test section (lines 2,373–5,390 = 3,017 lines) is already the majority of the file. Each
new rule needs at least 4–6 test cases; at ~8 lines each, that's another 100–150 lines per rule.
The file will reach ~5,850 lines after three rules — a code-smell signal the checker itself
would flag if it had a file-line rule (it doesn't, but `file-complexity` runs on this file too).

---

## 3. Backtesting discipline and minimum bar

Per `docs/backtesting.md` and `docs/backtest-repos.md`, two complementary tools are required
before shipping any new checker:

1. **Transcript backtest**: `kibitzer check backtest <name> --only-new` against
   `~/.claude/projects`. Shows whether the rule fires on real past edits.

2. **Public corpus**: `find ~/code/github.com/kubernetes/kubernetes -name '*.go' | xargs -n1
   kibitzer check native syntax-rules`, filtering output for the new rule ID. Required repos
   that are most relevant here: `kubernetes/kubernetes` (Go), `BurntSushi/ripgrep` (Rust),
   `denoland/deno` (Rust + TypeScript), `apache/cassandra` (Java).

The minimum bar: "run against the corpus at least once and explain every finding." Changed
findings that can't be explained are blockers. Record before/after counts in the PR.

**Specific patterns in the corpus likely to generate false positives**:

- `kubernetes/kubernetes`:
  - `decompose-conditional`: admission-controller guard chains (`if a != nil && a.Spec != nil &&
    a.Spec.Rules != nil`) are present in hundreds of files; at threshold ≤ 4 these all fire.
  - `consolidate-conditional`: sequential `if err != nil { return nil, err }` after every
    `client.Get`/`client.Update` call — dominates the entire codebase; this rule will produce
    thousands of findings without the error-guard suppression described above.
  - `remove-control-flag`: label/annotation lookup loops using `found` variables that then
    return or accumulate the found value — post-loop reads would prevent firing if the checker
    tracks them, but are easy to miss.

- `apache/cassandra`:
  - `decompose-conditional`: Java boolean expressions in parser/validation code (`if (!isValid
    && x > 0 && y != null && ...)`). Java's style is often more verbose in conditions.
  - `consolidate-conditional`: checked exceptions mean many consecutive `if (e1 != null) return
    null` patterns; same concern as Go error guards.

- `BurntSushi/ripgrep`:
  - `decompose-conditional`: Rust `matches!()` macro results composed with `&&`/`||` produce
    low nesting but high decision-point count; Rust cfg/feature conditions are another concern.
  - Low noise expected (ripgrep is clean code), making it a useful calibration baseline.

---

## 4. Threshold calibration risk for `decompose-conditional`

The only calibrated threshold evidence in this repo for the same kind of check is from
`extract-variable-false-positives.md`:

> depth 4 fired ~2,000 times on ordinary two-level conditions; depth 5 fired ~220 times on
> genuinely nested ones (corpus: kubernetes, vscode, cassandra, servo, deno, ripgrep,
> stapler-squad).

`decompose-conditional` using a **flat decision-point count** (not nesting depth) on conditions
is a different metric:

| Threshold | Example condition that fires | Assessment |
|-----------|------------------------------|------------|
| 2 | `if a && b && c` | Fires on every 3-clause guard — catastrophic noise |
| 3 | `if a && b && c && d` | Still fires on common 4-clause guards — high noise |
| 4 | `if a && b && c && d && e` | Plausible start; still common in k8s admission code |
| 5 | `if a && b && c && d && e && f` | Analogous to `extract-variable`'s depth-5 calibration point; likely acceptable |
| 6+ | Very long boolean chains only | Low noise, may miss real signal |

**Recommendation from corpus evidence**: start at threshold 5 (matching `extract-variable`'s
calibrated precedent for the same 40%-FP bar), run the corpus, then adjust. Never ship below 4.

**Too-low risk**: noise drowns true positives; users learn to ignore findings from these rules,
eroding trust in all kibitzer output.

**Too-high risk**: the rule becomes decorative — a condition with 7+ logical operators is
already a code smell caught by `extract-variable` (if deeply nested) or obvious on inspection.

---

## 5. Inter-rule interference: `decompose-conditional` vs `deep-nesting` and `extract-variable`

### vs `deep-nesting`

`deep-nesting` (line 158, threshold `MAX_NESTING_DEPTH = 4`) fires at the **declaration level**
when a function body's structural nesting exceeds 4 levels of `if/for/switch/select/func_literal`.
`decompose-conditional` fires at the **statement level** on the condition expression. They measure
different things — structural depth vs. condition breadth — and can co-fire on the same `if`
statement (one that is deeply nested AND has a complex condition).

There is **no existing deduplication mechanism** in kibitzer for two rules firing on the same
line. Each `SyntaxRulesChecker` rule fires independently through `check_declaration` or
`walk_blocks`. Co-firing is architecturally expected and tolerated (e.g., a 50-line deeply-nested
function already fires both `long-function` and `deep-nesting`). Co-fire from `decompose-conditional`
and `deep-nesting` is therefore acceptable by precedent, not a bug — but the messages must be
distinct enough that users understand they're different diagnostics.

### vs `extract-variable`

This is the more subtle risk. `extract-variable` already fires on `if`/ternary conditions when
`expression_nesting` ≥ 5 (counts operator AND call nesting depth). `decompose-conditional`
using `count_decision_points` on a condition counts only `&&`/`||` (no calls, no nesting —
it's a flat count). The two metrics are genuinely different:

- `a() && b(c(d()))`: `expression_nesting` = 3 (call → call → call); decision-point count = 1
  (`&&`). `extract-variable` will NOT fire at threshold 5; `decompose-conditional` will NOT fire
  at threshold 5. No co-fire.
- `a && b && c && d && e && f`: `expression_nesting` = 1 (flat `&&` chain, one level); decision-
  point count = 5. `extract-variable` does NOT fire (depth 1 < 5). `decompose-conditional` DOES
  fire at threshold 5. Complementary, no co-fire.
- `a() && b() || c() && d() || e() && f()`: mixed `&&`/`||` = `expression_nesting` ≈ 2–3
  (mixed logical ops each count as separate levels); decision-point count = 5. BOTH rules
  could fire at their respective thresholds.

The practical deduplication mechanism for the overlap case is **threshold separation**: if
`decompose-conditional` is set to fire at count ≥ 5 and `extract-variable` fires at nesting
depth ≥ 5, the overlap zone (both fire on the same condition) is conditions that are both wide
(many `&&`/`||`) AND deep (mixed operators with call arguments). These genuinely complex
conditions arguably merit both messages since they're diagnosing different aspects (breadth
vs. depth). If the design intent is strict no-double-fire, the simplest mechanism is:

> In `walk_dense_expressions`, if `decompose-conditional` has already fired on this `if`
> condition node, skip `extract-variable` for the same node (or vice versa).

But this requires shared state between rules inside a single walk, which does not exist today.
The alternative — accept co-fire and document it as "two different quality signals" — matches
existing precedent and requires no new infrastructure.
