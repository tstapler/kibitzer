# Architecture Review: kibitzer
**Date**: 2026-08-22
**Verdict**: CONCERNS (0 blockers, 5 concerns, 3 nitpicks)

No `docs/adr/ADR-000-architecture-constitution.md` exists in this repo — Constitution Check section
skipped per instructions (not fabricating constraints).

## Blockers

None. The plan is unusually self-auditing (ADR-001, Pattern Decisions table, and Unresolved
Questions already catch several issues a first-pass review would normally surface — e.g. the
Java/Kotlin dot-vs-slash identity bug, the CLI-verb gap, the dogfooding-target mismatch). The
issues below are real but none block starting implementation; several are cheap to fold into
Phase 0/2 before code is written.

## Concerns

- [ ] **`NamingRule.kind: String` / `ContentRule.allowed_kinds: Vec<String>` (Domain Glossary,
  Tasks 2.1.1a, 2.2.1a, 3.1.1a) — primitive obsession, inconsistent with the plan's own `DeclKind`
  and an existing codebase precedent it doesn't follow.** The plan introduces `DeclKind` (Task
  2.1.1a: `#[derive(Debug, Clone, Copy, PartialEq, Eq)]`, no `Deserialize`) specifically to make
  declaration kinds a proper sum type, then immediately routes around it: `ContentRule` and
  `NamingRule` store kind(s) as raw `String`, requiring a `kind_name(kind) -> &str` mapping helper
  (Task 2.2.1a) to compare against them. Unlike component-name references, which get fail-fast
  Levenshtein-suggestion validation at config-load time (Story 0.1.3), **there is no validation
  anywhere in the plan that a `ContentRule.allowed_kinds` or `NamingRule.kind` string is one of the
  five real `DeclKind` values** — a typo like `"structt"` parses fine and silently makes that rule
  permanently dead, the exact failure mode Story 0.1.3 was built to prevent for component names.
  This is also a missed reuse of an existing, in-repo pattern: `Severity` and `OutputFormat`
  (`src/config.rs:9-24`) are both plain Rust enums with `#[serde(rename_all = "lowercase")]`
  `Deserialize` derives — the direct precedent for exactly this situation.
  **Remediation**: derive `Deserialize` on `DeclKind` with `#[serde(rename_all = "lowercase")]`
  (mirroring `Severity`), and type `NamingRule.kind: DeclKind` / `ContentRule.allowed_kinds:
  Vec<DeclKind>` directly. This deletes the `kind_name()` string-mapping helper entirely and gets
  typo-rejection for free from serde instead of needing new bespoke validation code.

- [ ] **`DependencyRule.may_depend_on: Option<Vec<String>>` (Task 0.1.1b) has an unspecified
  state.** Every acceptance criterion in Story 1.1.1 and the `layers`-desugar reference
  implementation (Task 0.1.2b: `may_depend_on: Some(layers[i..].to_vec())`) only ever constructs
  `Some(...)`. The plan's "deny-by-default" rule (Pattern Decisions) covers a *component with no
  `DependencyRule` at all* — it says nothing about a `DependencyRule` that exists but has
  `may_depend_on: None`. Nothing in Story 1.1.1's acceptance criteria pins whether that means
  "unconstrained" or "deny everything" — a genuinely ambiguous, type-representable state that
  type-driven design says shouldn't exist.
  **Remediation**: drop the `Option`, make it plain `may_depend_on: Vec<String>` (empty vec =
  deny-all, consistent with the already-established deny-by-default default), or, if the `None`
  vs. `Some(vec![])` distinction is actually intended, add an explicit acceptance criterion and
  test pinning its behavior before Task 1.1.1a is implemented.

- [ ] **ADR-001's own rationale for the redundant-parsing cost isn't actually committed to in the
  task spec.** ADR-001 rationale item 4 argues the parallel-trait split doesn't cost extra parsing
  because `GrammarCache` "already exists precisely so multiple checkers sharing a `Language` share
  one parse," and proposes giving the new declaration pass its own `GrammarCache`-backed walk. But
  Task 2.1.1d then defaults to **a fresh, uncached `tree_sitter::Parser` per file** (matching
  `import_graph.rs::build_go`'s existing non-cached pattern), demoting `GrammarCache` to "a valid
  alternative... pick whichever keeps the diff smaller." As specified, a batch run with both
  `component-deps` and `content-rules`/`naming-rules` configured together — exactly Phase 7's
  dogfooding scenario — parses every Go/JS file **twice**: once in `import_graph::build()`, once in
  `declarations::build()`. That's precisely the redundant-parsing cost ADR-001 claimed was already
  solved.
  **Remediation**: commit explicitly to threading one shared `GrammarCache` (or equivalent
  single-parse-per-file cache) through both `import_graph::build()` and `declarations::build()` at
  the `run_architecture_check`/batch-run call site, rather than leaving it an implementer's
  diff-size judgment call. Worth doing given requirements.md's explicit performance NFR carried
  forward into this plan ("rebuilding the import graph on every edit is too expensive").

- [ ] **Duplicated "declared-but-never-fires" audit logic, and an unexplained third asymmetry.**
  Task 1.1.3a (`ComponentDependencyChecker`'s zero-match-component advisory) and Task 3.1.2a
  (`NamingChecker`'s zero-match-rule advisory) independently implement the same pattern — scan
  declared rules/components, flag ones matching zero real graph entries. Per the
  `code-architecture-best-practices` skill's Reuse Check (the highest-value check to run *before*
  any pattern work): this is exactly the "same bug-prone logic copy-pasted across sibling
  functions" smell, each implementation getting its own chance to diverge (e.g. Task 1.1.3a's
  criterion produces one finding per component; nothing pins whether Task 3.1.2a's is per-rule or
  batched the same way). Worse, `ContentChecker` (Story 2.2.1) gets **no equivalent audit at all**
  — a third inconsistency with no stated rationale, unlike the deliberate and explicitly-justified
  asymmetry between `DependencyRule`'s deny-by-default and `ContentRule`'s "no rule = unconstrained"
  (which Story 2.2.1's acceptance criteria does explain).
  **Remediation**: extract one shared helper (e.g. `fn zero_match_advisory<T>(declared: &[T],
  matches: impl Fn(&T) -> bool, describe: impl Fn(&T) -> String) -> Option<ArchFinding>`) used by
  `ComponentDependencyChecker` and `NamingChecker`, and either add the same audit to `ContentChecker`
  in Story 2.2.1 or add one sentence explaining why content rules are exempt.

- [ ] **Phase 6's `[category]` prefix change is an effective breaking change to a public tool's
  output contract, not just an internal test-maintenance item.** Pattern Decisions frames
  `ArchFinding` as "reused unmodified," which is true at the Rust-type level — but Story 6.1.1
  changes the *message text* of the two already-shipped, released checkers (`import-cycles`,
  `layering`) by adding a `[import-cycle]`/`[layering]` prefix that didn't exist before. Task
  6.1.1c catches the *internal* test-breakage risk, but requirements.md itself notes kibitzer "is
  public/OSS with real release automation (`dist-workspace.toml`, `cliff.toml`, a Homebrew tap)."
  Any external adopter who scripted a `grep`/CI assertion against the old un-prefixed
  `"import cycle: ..."` string breaks silently on upgrade — `ArchFinding.message` is plain `String`,
  so nothing at the type level signals this to a consumer.
  **Remediation**: no code change needed, but Phase 6's changelog entry (via `cliff.toml`'s
  conventional-commit convention) should call out the message-format change explicitly as a
  behavior change, not bundle it silently into a "polish" release note.

## Nitpicks

- `component_of` (Task 0.1.2d) returns borrowed `Option<&'a str>`, but its main downstream
  consumer, `Declaration.component: Option<String>` (Task 2.1.1b), needs an owned value —
  `resolve_component` (Task 2.1.1e) will need a `.map(str::to_string)` at the boundary. Not a real
  problem, just worth the implementer knowing the conversion is expected rather than a sign
  something's wrong.
- Task 1.1.1a's prose doesn't say whether `config.effective_components()`/
  `effective_dependency_rules()` (Task 0.1.2c: each allocates a fresh `Vec` via `.chain(...).collect()`)
  are computed once per `check()` call or resolved fresh per edge. If implemented as the latter,
  that's an O(edges) redundant-allocation footgun. Worth one line in the task text: "compute once
  at the top of `check()`, before the edge loop."
- ADR-001 accepts, with reasoning, that `Check.architecture_checker` stays a flat string namespace
  spanning two trait registries. Given the same plan already introduces Levenshtein "did you mean"
  suggestions for unknown component names (Story 0.1.3), the parallel "unknown architecture checker
  'X'" error path (Task 1.2.1d) would be a natural, cheap place to reuse that same helper for a
  small UX win — not required, just a low-cost consistency opportunity already sitting in the plan.
