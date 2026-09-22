# Implementation Plan: cite-fowler-refactorings

**Feature**: Make `long-function` and `deep-nesting` finding messages (and, optionally, their
`CATALOG` descriptions) name and link the matching Fowler refactoring, matching the
`flag-argument`/`unreachable-code` citation precedent already in `src/rules.rs`.
**Date**: 2026-09-22
**Status**: Ready for implementation
**ADRs**: None

---

## Creative Pass (Step 0.5 — alternatives considered)

Three approaches were considered for *how* the citation text gets into the two messages:

- **A. Inline literal edit** (chosen) — extend the two existing `format!` string literals in
  place, exactly as `flag-argument`'s message already does at `src/rules.rs:865`. Strength:
  zero new abstraction, matches the only two precedents in the file exactly, smallest possible
  diff. Weakness: if a third/fourth check needs a citation later, the name+URL pair is
  duplicated again rather than looked up — acceptable per `research/build-vs-buy.md`, which
  found that even the *existing* two citations (`flag-argument`, `unreachable-code`) don't
  share a table.
- **B. Shared `rule_id -> (name, url)` const table**, consulted by both `format!` sites and by
  `CATALOG`. Strength: single source of truth if a URL ever changes. Weakness: no existing
  precedent for it in this file (the two prior citations don't use one either); at n=2 checks
  (n=4 counting the two prior, uncited-by-table checks) it's premature abstraction with real
  indirection cost and no current duplication problem to solve.
- **C. Shared `fn fowler_citation(id: &str) -> &'static str` helper** returning just the
  `"Fowler's <Name> (<url>)"` suffix, called from both `format!` sites. Strength: DRYs the
  suffix text itself. Weakness: the two messages have different lead-in clauses and captured
  identifiers, so the helper only saves the trailing ~10 words while adding a new function,
  match arm, and indirection layer — not worth it for two call sites.

**Chosen: A**, per `research/build-vs-buy.md` §1's explicit recommendation. Rejected
alternatives are recorded in the Pattern Decisions table below.

---

## Step 1 — System type

This is a pure text-content change to two existing native static-analysis checks in a Rust
CLI/linter (`kibitzer`). No new component, service, or data flow is introduced.

---

## Domain Glossary

No new domain terms are introduced by this change. It edits the content of two existing
`Finding.message` string literals and (optionally) two existing `RuleMeta.description` string
literals; no new type, method, or variable is added to the codebase.

---

## Pattern Decisions

| Component | Pattern Chosen | Source | Alternative Rejected | Reason |
|-----------|---------------|--------|---------------------|--------|
| Citation text placement | Extend the two existing `format!` literals in `check_declaration` in place (`src/rules.rs:826-828`, `:836-838`) | Repo precedent: `flag-argument`'s runtime message, `src/rules.rs:865` | Shared `rule_id -> (name, url)` const/lookup table consulted by both `format!` sites and `CATALOG` | `research/build-vs-buy.md` §1: at n=2 (n=4 including the 2 prior, table-free citations), a lookup table is premature abstraction the codebase's own precedent doesn't use |
| Citation text placement | (same as above) | (same as above) | Shared `fn fowler_citation(id) -> &'static str` helper returning the citation suffix | The two messages have different lead-in text and captured identifiers (`{body_lines}`/`{depth}`); a helper saves only the trailing clause while adding a function + call-site indirection for two uses |
| `CATALOG.description` update (AC#7, optional) | Extend the two existing `&'static str` literals in place (`src/rules.rs:38`, `:44`), name-only, no URL | Repo precedent: `flag-argument`/`unreachable-code` descriptions, `src/rules.rs:56,62` | Add the URL to `CATALOG.description` too, for symmetry with the runtime message | `CATALOG` is `#[allow(dead_code)]` self-documentation, not machine-read; existing precedent (`:56,62`) names the refactoring only, no URL — matching that exactly avoids inventing a new sub-convention this task doesn't need |
| URL liveness verification | One-time manual `curl -sI` immediately before ship (AC#3) | `research/build-vs-buy.md` §2 | Add a linkcheck crate, or extend `markdown-link-integrity` to follow `http(s)://` targets | `src/markdown_link_integrity.rs:357` deliberately short-circuits `http(s)://` targets as always-valid — a documented boundary against flaky, network-dependent checks; reversing it for two URLs that don't change is disproportionate |

---

## Tech Debt Disposition

None identified. No `research/architecture.md` exists for this project (not needed at this
scope), and no architectural hotspot is touched — this change is confined to two string
literals (plus optionally two more) inside one existing function in `src/rules.rs`.

---

## Observability Plan
- **Logs**: N/A — no logging behavior changes; `Finding.message` is not a log line.
- **Metrics**: N/A
- **Alerts**: N/A

## Risk Control
- **Feature flag**: not gated — this is a message-text change to advisory-severity findings,
  not a behavior change requiring staged exposure.
- **Rollback procedure**: standard revert via PR close + revert commit.
- **Staged rollout**: full rollout on merge (next `brew upgrade kibitzer` per the release
  process in this repo's `CLAUDE.md`).

## Unresolved Questions

None. All open questions from `requirements.md` and the research phase were resolved during
research (exact call sites, test-safety, URL liveness, and the build-vs-buy tradeoff are all
settled — see `research/stack.md`, `research/pitfalls.md`, `research/build-vs-buy.md`).

## Dependency Visualization

```
Task 1.1.1a (edit long-function format!, src/rules.rs:826-828)
Task 1.1.1b (edit deep-nesting format!, src/rules.rs:836-838)   } independent of each other
        |
        v
Task 1.1.2a (add 2 new unit tests, src/rules.rs mod tests)      -- depends on 1.1.1a + 1.1.1b
        |
Task 1.1.3a (optional: edit CATALOG descriptions, :38/:44)      -- independent, can run anytime
        |                                                          before 1.2.1a
        v
Task 1.2.1a (verify: curl -sI both URLs; cargo test rules::;
             cargo fmt --check; cargo clippy)                   -- depends on all above
```

---

## Phase 1: Cite Fowler refactorings in `long-function`/`deep-nesting`

### Epic 1.1: Update finding messages, catalog descriptions, and tests
**Goal**: `long-function` and `deep-nesting` findings name and link their matching Fowler
refactoring, with new tests locking in the behavior and all existing tests passing unmodified.

#### Story 1.1.1: Cite the refactoring in both runtime finding messages
**As a** kibitzer user reading a `long-function` or `deep-nesting` finding, **I want** the
message to name the matching Fowler refactoring and link its catalog page, **so that** I know
which specific technique to apply without already knowing the catalog.

**Acceptance Criteria**:
- The `long-function` finding message names "Extract Function" and links
  `https://refactoring.com/catalog/extractFunction.html`.
  - *Given* the 45-line Go function body used in the existing `flags_long_function` test
    (`src/rules.rs:1230-1242`) is checked by `check_declaration`, *When* `check_source` runs
    over it, *Then* the resulting `Finding.message` contains both the substring
    `"Extract Function"` and the substring
    `"https://refactoring.com/catalog/extractFunction.html"`.
- The `deep-nesting` finding message names "Replace Nested Conditional with Guard Clauses" and
  links `https://refactoring.com/catalog/replaceNestedConditionalWithGuardClauses.html`.
  - *Given* the 5-level-nested Go function body used in the existing `flags_deep_nesting` test
    (`src/rules.rs:1257-1278`) is checked by `check_declaration`, *When* `check_source` runs
    over it, *Then* the resulting `Finding.message` contains both the substring
    `"Guard Clauses"` and the substring
    `"https://refactoring.com/catalog/replaceNestedConditionalWithGuardClauses.html"`.
- `LONG_FUNCTION_LINES`, `MAX_NESTING_DEPTH`, the two trigger conditions, and each `Finding`'s
  `line`/severity construction stay byte-identical.
  - *Given* the pre-edit and post-edit versions of `src/rules.rs`, *When* `git diff
    src/rules.rs` is inspected after Task 1.1.1a/1.1.1b, *Then* the diff touches only the
    string-literal contents inside the two `format!(...)` calls at lines 826-828 and 836-838 —
    `LONG_FUNCTION_LINES` (`:11`), `MAX_NESTING_DEPTH` (`:15`), the `if body_lines > ...` /
    `if depth > ...` conditions, and the `Finding { line, .. }` struct construction are
    unchanged.
- All existing tests pass unmodified.
  - *Given* the 29 pre-existing call sites in `src/rules.rs` asserting
    `.message.contains("[long-function]")` or `.contains("[deep-nesting]")`, *When* `cargo
    test rules::` is run after Task 1.1.1a/1.1.1b, *Then* all of them pass with zero edits to
    their assertions (substring match on the `[rule-id]` prefix is unaffected by appended
    trailing text).

**Files**: `src/rules.rs`

##### Task 1.1.1a: Edit the `long-function` message (~3 min)
- In `check_declaration` (`src/rules.rs:826-828`), change the `format!` literal from:
  `"[long-function] body spans {body_lines} lines (over {LONG_FUNCTION_LINES}) — consider
  splitting it up"`
  to:
  `"[long-function] body spans {body_lines} lines (over {LONG_FUNCTION_LINES}) — consider
  Fowler's Extract Function (https://refactoring.com/catalog/extractFunction.html)"`
- Keep the existing 3-line `format!(...)` layout (opening paren, literal on its own line,
  closing paren) so no other line numbers in the file shift.
- Files: `src/rules.rs`

##### Task 1.1.1b: Edit the `deep-nesting` message (~3 min)
- In `check_declaration` (`src/rules.rs:836-838`), change the `format!` literal from:
  `"[deep-nesting] body nests {depth} levels deep (over {MAX_NESTING_DEPTH}) — consider
  extracting a function or inverting a condition"`
  to:
  `"[deep-nesting] body nests {depth} levels deep (over {MAX_NESTING_DEPTH}) — consider
  Fowler's Replace Nested Conditional with Guard Clauses
  (https://refactoring.com/catalog/replaceNestedConditionalWithGuardClauses.html)"`
- Keep the existing 3-line `format!(...)` layout so no other line numbers shift.
- Files: `src/rules.rs`

#### Story 1.1.2: Lock in the citation with two new unit tests
**As a** kibitzer maintainer, **I want** dedicated tests asserting the citation text is
present, **so that** a future edit to these messages can't silently drop the citation without
a test failing.

**Acceptance Criteria**:
- Two new unit tests exist in `src/rules.rs`'s `mod tests`:
  `long_function_message_cites_extract_function`, `deep_nesting_message_cites_guard_clauses`.
  - *Given* Task 1.1.2a has been applied to `src/rules.rs`, *When* `cargo test rules::
    long_function_message_cites_extract_function rules::deep_nesting_message_cites_guard_clauses`
    is run, *Then* both tests exist, compile, and pass.

**Files**: `src/rules.rs`

##### Task 1.1.2a: Add the two new unit tests (~5 min)
- Depends on: Task 1.1.1a, Task 1.1.1b (tests assert on the post-edit message text).
- Immediately after the existing `flags_long_function` test (ends `src/rules.rs:1242`, right
  before `allows_shallow_nesting`), insert:
  ```rust
  #[test]
  fn long_function_message_cites_extract_function() {
      let mut src = String::from("package main\nfunc f() {\n");
      for _ in 0..45 {
          src.push_str("\tprintln(\"line\")\n");
      }
      src.push_str("}\n");
      let findings = check_source(&src).unwrap();
      assert!(findings.iter().any(|f| f.message.contains("Extract Function")
          && f.message.contains("https://refactoring.com/catalog/extractFunction.html")));
  }
  ```
- Immediately after the existing `flags_deep_nesting` test (ends `src/rules.rs:1278`, right
  before `allows_short_parameter_list`), insert:
  ```rust
  #[test]
  fn deep_nesting_message_cites_guard_clauses() {
      let src = "package main\n\
           func f(x int) {\n\
           \tif x > 0 {\n\
           \t\tfor i := 0; i < x; i++ {\n\
           \t\t\tswitch i {\n\
           \t\t\tcase 0:\n\
           \t\t\t\tif i == 0 {\n\
           \t\t\t\t\tprintln(\"deep\")\n\
           \t\t\t\t}\n\
           \t\t\t}\n\
           \t\t}\n\
           \t}\n\
           }\n";
      let findings = check_source(src).unwrap();
      assert!(findings.iter().any(|f| f.message.contains("Guard Clauses")
          && f.message.contains("https://refactoring.com/catalog/replaceNestedConditionalWithGuardClauses.html")));
  }
  ```
- Files: `src/rules.rs`

#### Story 1.1.3 (AC#7 — committed, resolving adversarial-review Concern #1 / pre-mortem P2 by taking this in scope rather than leaving it ambiguous): Match the citation in both `CATALOG` descriptions
**As a** kibitzer maintainer browsing `CATALOG` (e.g. for a future `kibitzer rules list`
command), **I want** `long-function`/`deep-nesting`'s descriptions to name their Fowler
refactoring too, **so that** `CATALOG` stays consistent with `flag-argument`/`unreachable-code`,
which already do this.

**Acceptance Criteria**:
- `CATALOG`'s `RuleMeta.description` for `long-function` and `deep-nesting` names the matching
  refactoring, in the same "name only, no URL" style already used at `src/rules.rs:56,62`.
  - *Given* `CATALOG[0]` (id `"long-function"`, `src/rules.rs:38`) and `CATALOG[1]` (id
    `"deep-nesting"`, `src/rules.rs:44`) after Task 1.1.3a, *When* their `description` fields
    are read, *Then* they end with `"— Fowler's Extract Function."` and `"— Fowler's Replace
    Nested Conditional with Guard Clauses."` respectively — matching the style of
    `CATALOG[3]`'s (`flag-argument`) `"— Fowler's Remove Flag Argument."`.

**Files**: `src/rules.rs`

##### Task 1.1.3a: Edit the two `CATALOG.description` literals (~2 min)
- At `src/rules.rs:38`, change `"Function/method body spans more than 40 lines."` to
  `"Function/method body spans more than 40 lines — Fowler's Extract Function."`.
- At `src/rules.rs:44`, change `"Function/method body nests if/for/switch/select/func_literal
  more than 4 levels deep."` to `"Function/method body nests if/for/switch/select/func_literal
  more than 4 levels deep — Fowler's Replace Nested Conditional with Guard Clauses."`.
- No other field on either `RuleMeta` changes.
- Files: `src/rules.rs`

### Epic 1.2: Verification
**Goal**: Confirm the change is safe to ship — live URLs, full test suite, and standard CI
gates (`fmt`, `clippy`) all pass.

#### Story 1.2.1: Verify URLs and run the full local check suite before shipping
**As a** kibitzer maintainer, **I want** to re-verify both catalog URLs and the full test/lint
suite immediately before opening the PR, **so that** an external site going down or a
formatting/lint regression isn't discovered only in CI.

**Acceptance Criteria**:
- Both catalog URLs return 2xx.
  - *Given* the two URLs `https://refactoring.com/catalog/extractFunction.html` and
    `https://refactoring.com/catalog/replaceNestedConditionalWithGuardClauses.html`, *When*
    `curl -sI <url>` is run against each immediately before opening the PR, *Then* both
    responses' status line is `HTTP/1.1 200` (or another 2xx).
- The full test suite, `cargo fmt --check`, and `cargo clippy` all pass with the change applied.
  - *Given* all edits from Tasks 1.1.1a, 1.1.1b, 1.1.2a, and 1.1.3a are applied, *When*
    `cargo test`, `cargo fmt --all --check`, and `cargo clippy --workspace --all-targets --
    -D warnings` are run, *Then* all three exit 0.

**Files**: none (verification-only; no file changes)

##### Task 1.2.1a: Run verification commands (~4 min)
- Depends on: Task 1.1.1a, Task 1.1.1b, Task 1.1.2a, Task 1.1.3a.
- Run `curl -sI https://refactoring.com/catalog/extractFunction.html` and `curl -sI
  https://refactoring.com/catalog/replaceNestedConditionalWithGuardClauses.html`; confirm both
  status lines are 2xx.
- Run `cargo test` (or at minimum `cargo test rules::`); confirm 0 failures.
- Run `cargo fmt --all --check`; confirm no diff.
- Run `cargo clippy --workspace --all-targets -- -D warnings`; confirm 0 warnings.
- Files: none
