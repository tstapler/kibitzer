# Implementation Plan: replace-magic-literal

**Feature**: New `replace-magic-literal` rule in `src/rules.rs`'s `SyntaxRulesChecker` — flags a non-trivial numeric/string literal repeated ≥2 times in one file, across all 8 languages `syntax-rules` already covers, unless it's the direct initializer of a named-constant-style binding referenced elsewhere.
**Date**: 2026-09-22
**Status**: Ready for implementation
**ADRs**: ADR-001-magic-literal-exclusion-and-threshold-strategy.md

---

## Domain Glossary
*(Ubiquitous language — every domain term that appears as a type, method, or variable name. Exact names here must be used consistently in code, tests, and comments.)*

| Term | Definition | Notes |
|------|-----------|-------|
| `literal_kinds` | New `LangRuleConfig` field: the per-grammar list of numeric/string literal node kinds this rule walks. | Kind-list field, same shape as existing `terminal_kinds`/`nesting_kinds`. |
| `numeric_literal_kinds` | New `LangRuleConfig` field: the subset of `literal_kinds` that are numeric (vs. string) node kinds for this grammar. | Added in the 2nd triad-review round to fix a real gap: `emit_literal_findings`'s numeric/string message label needs a concrete per-language answer, not a raw-text/quote-character heuristic (which mislabels Go/Rust raw strings and Python `r`/`b`/`f`-prefixed strings) and not an undefined "the split" with no field behind it. |
| `binding_finder` | New `LangRuleConfig` field: `fn(Node, &[u8]) -> Option<ConstBinding>`. Given any node, returns `Some(ConstBinding { name, initializer })` if that node is a named-constant-style binding whose direct initializer is a literal; `None` otherwise. | Function pointer, matches `body_finder`/`params_finder`'s existing pattern for per-grammar structural divergence. Named-struct return type, not an anonymous tuple — see `ConstBinding` below (Engineering-lens triad-review fix: an `Option<(String, Node)>` compiles silently even if a future edit swaps the two positions across 8 independent hand-written implementations). |
| `ConstBinding` | New struct: `{ name: String, initializer: Node<'tree> }`. The named-constant binding a `binding_finder` found. | Replaces the anonymous `(String, Node)` tuple everywhere it previously appeared (`binding_finder`'s return type, `LiteralCollector.bound`'s element type) — field names make a positional mix-up a compile error instead of a silent bug. |
| `LiteralCollector` | New struct: the accumulator for the two-pass walk. Holds `occurrences: HashMap<String, Vec<LiteralOccurrence>>` (raw literal text → every occurrence, each carrying its own node kind) and `bound: Vec<ConstBinding>` (every qualifying binding found). | Lives only for the duration of one `check()` call. |
| `LiteralOccurrence` | New struct: `{ node: Node<'tree>, kind: &'static str }`. One occurrence of a literal value, carrying the literal node's own `node.kind()` alongside it. | Exists so `emit_literal_findings` can label a finding "numeric" vs. "string" from the grammar's own node-kind name (e.g. is this kind present in a language's numeric-kind subset) rather than re-deriving it from the raw text's first character — the quote-character heuristic mislabels Go/Rust raw strings (no leading `"`) and Python `r"/b"/f"`-prefixed strings (leading `r`/`b`/`f`, not `"`) as "numeric" (Engineering-lens triad-review fix). |
| `walk_literals` | New fn, sibling of `walk_declarations`/`walk_blocks`. Collect-pass: recurses the whole tree, populating a `LiteralCollector`. | Does not emit `Finding`s itself — matches architecture.md's "collect-then-emit, don't fold into `walk_blocks`" recommendation. |
| `resolve_excluded_constants` | New fn: given a `LiteralCollector` and the file-wide identifier reference counts, returns the `HashSet<usize>` of initializer node ids that qualify for the named-constant exclusion. | Node ids come from `Node::id()`, stable within one parsed tree. |
| `emit_literal_findings` | New fn: emit-pass. Applies the allow-list and the excluded-node-id set, filters to groups with ≥ `MAGIC_LITERAL_MIN_OCCURRENCES` remaining occurrences, and pushes one `Finding` per qualifying literal value. | Anchors at the first remaining occurrence's line (see UX research), lists the rest inline. Derives its numeric/string label from `LiteralOccurrence.kind`, not the raw text (see `LiteralOccurrence`). |
| `MAGIC_LITERAL_MIN_OCCURRENCES` | New const, `usize = 2`. The repetition threshold. | See ADR-001 for why this ships at 2 with a backtest-gated bump path to 3, not a preemptive 3. |
| `MAGIC_LITERAL_ALLOWLIST` | New const, `&[&str]`: near-universal literal *values* exempted regardless of repeat count (`"0"`, `"1"`, `"-1"`, and empty-string forms). | Compared against a **normalized** value (delimiters/prefixes stripped by `normalize_literal_value`, Task 1.2.1e), not the raw source text — a raw-text comparison against a fixed list of `"\"\""`/`"''"` never matches Kotlin's empty `""""""`, Rust's `r""`, or Go's backtick empty raw string, which is a direct AC3 violation for those 3 languages (Engineering-lens/adversarial-review-flagged fix). |
| `is_allowlisted_literal` | New fn: membership test against `MAGIC_LITERAL_ALLOWLIST`. | |
| `collect_identifiers` | Existing fn (`src/rules.rs:898-908`), **generalized** from `HashSet<String>` to `HashMap<String, usize>` (counts, not just presence). | Reused by both `flag-argument` (via `collect_condition_identifiers`, now checking `.contains_key()`) and this rule's exclusion logic — not a new identifier walk. |
| `replace-magic-literal` | The rule id: the string used in `CATALOG`, in `Finding` messages' `[replace-magic-literal]` prefix, and in a `.kibitzer/accepted/` entry's `"rule"` field. | Matches the existing bracketed-rule-id convention (`[long-function]`, `[flag-argument]`, etc.). |

---

## Pattern Decisions

| Component | Pattern Chosen | Source | Alternative Rejected | Reason |
|-----------|---------------|--------|---------------------|--------|
| Whole-file literal walk | New third top-level walk (`walk_literals`), collect-then-emit two-pass, sibling to `walk_declarations`/`walk_blocks` | GoF (Template Method-shaped walk, matching this file's existing convention) | Fold literal-kind dispatch into the existing `walk_blocks` single-pass "resolve and emit immediately" walk | `walk_blocks` and literal-repetition have fundamentally different control-flow shapes (emit-immediately vs. decide-after-whole-tree); folding them smuggles hidden coupling into a function whose only job today is one thing (research/architecture.md §1). |
| Per-language literal/binding data | Two new `LangRuleConfig` fields (`literal_kinds` kind-list, `binding_finder` function pointer) — GoF Strategy, already this table's established idiom | GoF Strategy / PoEAA Transaction Script (mechanical per-node dispatch, not a Domain Model) | A `LiteralRule` trait implemented once per language | Inventing per-rule trait objects for one new rule breaks consistency with the other 5 rules already sharing one flat config struct, and adds indirection with no behavioral benefit at this scale (research/architecture.md §2). |
| Two-pass aggregation | `LiteralCollector` accumulator struct + explicit collect/emit function pair | PoEAA Transaction Script (procedural collect → filter → emit, no persistent domain model) | A streaming single-pass design that speculatively emits and retracts findings as more occurrences are discovered | Literal repetition is an inherent whole-file aggregate; a retraction design is materially more complex for a file-size class (≤ a few thousand lines) where the extra O(n) pass costs nothing that matters (research/architecture.md §1, §5). |
| Named-constant exclusion | Generalize existing `collect_identifiers` from `HashSet<String>` to `HashMap<String, usize>`; reuse for both `flag-argument` and this rule | Reuse over reinvention (DRY) | A new, separate identifier-reference-counting walk built just for this rule | `collect_identifiers` already exists and already serves the same "is this name referenced elsewhere" question for `flag-argument`; a counting variant is a small, backward-compatible generalization, not new capability (research/architecture.md §4). |
| Const-reference scope resolution | Same-file, non-scope-resolved identifier-text matching; a shadowed same-named binding in another function is an accepted, documented false negative | Type-driven design: deliberately *not* modeling lexical scope | True lexical scope resolution (tracking each binding's declaring block and resolving references against it) | Matches this file's own existing, documented precedent (`collect_condition_identifiers`'s doc comment, `src/rules.rs:878-880`): "a mechanical, low-false-positive heuristic, not a scope-resolving analysis." Building real scope resolution is large, novel capability disproportionate to this feature's "Small" appetite (research/build-vs-buy.md §3). |
| Generated-file noise/cost | Proactively call `file_size::is_generated`, scoped to just the new literal pass (existing 4 rules untouched) | Reuse of existing seam, applied proactively (see Tech Debt Disposition) | Ship without the guard and wait for the corpus backtest to discover the gap, as every prior checker (`duplicate-code`, `file-complexity`, `go-table-driven-test-candidate`) did retroactively | pitfalls.md §2/§5: every checker with a false-positive log hit generated code *first*, and literal repetition is *more* exposed to this than any prior checker (dense mechanical repetition by construction). The fix is a one-line reuse of an existing function — pre-empting it costs nothing. |
| Repetition threshold | Ship coded at `>=2` (AC2) as a named constant, with a backtest-gated decision (Phase 3, Story 3.2.2) to raise to `>=3` if the corpus mirrors `duplicate-code`'s overcorrection | Empirical validation over speculative pre-analysis | (a) Hardcode `>=3` preemptively without ever backtesting `>=2`; (b) make the threshold end-user-configurable from day one | (a) would silently override AC2's explicit numeric acceptance criterion without evidence — the entire point of the mandatory backtest (AC7-8) is to decide this empirically (requirements.md's own Appetite section: "derisk via backtest, not speculative pre-analysis"). (b) No other `CATALOG` rule is configurable yet (docs/syntax-rules.md:137) — scope creep against precedent. See ADR-001. |
| Python constant heuristic | Casing-convention detector: a module/class-level `assignment` whose left identifier matches `^[A-Z][A-Z0-9_]*$` | Convention-based heuristic (Python has no `const` keyword) | Skip the named-constant exclusion entirely for Python | Skipping it would make Python systematically noisier than the other 7 languages for the identical smell — an inconsistency the requirements don't call for. A casing heuristic is cheap and directionally correct even though weaker than a keyword guarantee (pitfalls.md §4). |
| JS/TS/Rust binding scope | Restrict the exclusion to `const` (JS/TS: `lexical_declaration` whose `kind` field is exactly `"const"`) and `const_item`/`static_item` (Rust) — excluding `let`/`let_declaration` | Type-driven design: treat "promoted to a named symbolic constant" as a narrower state than "any immutable local binding" | Honor the requirements' literal "const/let…-style binding" wording and exempt any `let` too | A trivial local `let x = 42` used once nearby would satisfy a naive "referenced elsewhere" check without representing Fowler's actual refactoring — this is a deliberate, documented tightening versus the requirements' looser phrasing, in the direction of fewer false negatives (pitfalls.md §4). Kotlin's `val` is kept in scope since the requirements explicitly name it and Kotlin has no separate `const` usable at local scope. |

---

## Tech Debt Disposition
*(Every hotspot/architecture violation touched by this feature, per `research/architecture.md`. "None identified" if none.)*

| Area | Existing Issue | Disposition | Justification |
|------|----------------|--------------|----------------|
| `src/rules.rs::lang_config()` (`:487-703`, 217 lines, 8 match arms — one per `Language` variant, each returning a `LangRuleConfig` struct literal) | **Already 5.4x over kibitzer's own `long-function` threshold** (`kibitzer run src --trigger batch` → `src/rules.rs:487: [long-function] body spans 217 lines (over 40)`). `research/architecture.md` §5's claim that "no hotspot/churn evidence... flags `rules.rs` as a SOLID violation or refactor candidate" is false — confirmed wrong by this same command. Epic 1.1's 8 per-language tasks (1.1.1a, 1.1.1c–1.1.1h) each add `literal_kinds`/`binding_finder` fields directly inside this match statement, adding ~16+ lines to an already-violating function — another instance of the identical violation, not a proportionate extension. | **Refactor-first.** | Per architecture-review.md's Lens 4 criterion #12, an "Extend as-is" disposition that adds another instance of a violation the target already has is a blocker, not a judgment call. Fixed here by sequencing a mechanical extraction (new Story 1.0.1, below) before any per-language field-addition task touches this function — split the 8 match arms into 8 small named constructor functions, reducing `lang_config()` itself to a thin dispatcher under the 40-line threshold. This is consistent with, not foreign to, the file's own idiom: it already factors per-language concerns into small named functions living just above `lang_config()` (`field_body` `src/rules.rs:146`, `field_params` `:150`, `go_param_identifier_count` `:154`, `kotlin_body` `:397`, `kotlin_params` `:405`). Expected post-refactor state: `lang_config()` itself ~20 lines (an 8-arm dispatch match), the 8 extracted constructor functions ~25-45 lines each (unchanged struct-literal bodies, just relocated), and `src/rules.rs`'s total line count essentially unchanged before Epic 1.1's additions (extraction adds only per-function signature/brace overhead, no new logic) — Epic 1.1 then adds its 2 fields x 8 constructor functions on top of that baseline instead of growing a single already-oversized function further. |
| `src/rules.rs` (`LangRuleConfig`/`SyntaxRulesChecker`, 2329 lines) | Also already over kibitzer's own `rust-file-size` threshold (`src/rules.rs:2329: [rust-file-size] file spans 2329 lines (over 500)`, 4.7x over) — a pre-existing condition, not one this feature meaningfully worsens. | **Extend as-is.** | The file already organizes its 5 existing rules around exactly the shape this feature needs (one shared config table, dedicated walk functions, small per-rule logic functions). Adding an 8th rule costs 2 table fields, 1 new walk, and a handful of sibling functions — proportionate to the existing pattern, not a strain on it. Splitting into per-rule modules is a legitimate future refactor once the file grows meaningfully past this addition, but doing it now would be scope creep against this feature's "Small" appetite (requirements.md). The file-size violation itself is orthogonal to — and not fixed by — the `lang_config()` refactor-first row above; that row addresses the long-function violation this feature's own tasks would otherwise compound, not the file-size violation, which this feature doesn't materially change. |
| `SyntaxRulesChecker` has no size/generated-file guard today | Not a violation per se, but a gap every sibling checker (`duplicate_code.rs`, `file_size.rs`, `go_table_driven_test.rs`, etc.) already closed via `file_size::is_generated`. | **Isolate via seam** — reuse the existing `is_generated` seam, scoped to only the new literal pass (Pattern Decisions table, "Generated-file noise/cost" row), rather than retrofitting a guard onto the 4 existing rules (out of scope for this feature — no backtest evidence they need one). | pitfalls.md §2/§5: literal repetition is the checker most exposed to generated-code noise of any rule in this file; closing the gap for the new pass only is a minimal, self-contained seam that doesn't touch the other 4 rules' behavior. |

---

## Migration Plan

Omitted — no schema or data changes. This feature is additive Rust code plus two doc edits.

## Observability Plan
- **Logs**: None beyond kibitzer's existing `Finding` output (stdout via CLI, structured via the MCP `run_checks`/`architecture_assessment` tools) — this rule produces `Finding`s through the same pipeline as its 5 `CATALOG` siblings; no new logging surface.
- **Metrics**: None. kibitzer has no per-rule metrics system; the closest analog is the backtest-triage true/false-positive ratio recorded in `docs/backtest-triage/<repo-slug>/replace-magic-literal.jsonl` (Phase 3), which is a one-time/periodic validation artifact, not a runtime metric.
- **Alerts**: None — `Severity::Advisory`, not a build/CI gate.

## Risk Control
- **Feature flag**: Not gated — ships as part of `default_checks()` once merged, matching every other `CATALOG` rule (`syntax_rules_checks()` needs zero code changes since it already registers all 8 `syntax-rules-*` checker names; the new rule rides along automatically). kibitzer has no staged-rollout mechanism for native checks.
- **Rollback procedure**: Standard revert via PR close + revert commit. The diff is self-contained (two new `LangRuleConfig` fields + per-language values, two new walk/emit functions, one `CATALOG` entry, doc edits) — reverting it removes the rule cleanly with no data/schema cleanup.
- **Staged rollout**: Full rollout on merge, but behaviorally gated by the mandatory backtest steps (Phase 3) acting as a pre-merge quality gate — per ADR-001 and Story 3.2.2's pre-committed 40% false-positive bar (not a post-hoc judgment call), if the corpus backtest reproduces `duplicate-code`'s table-driven-test overcorrection pattern, the threshold is bumped from 2 to 3 in the same PR before merge, not discovered and fixed after the fact. Story 3.2.3 makes this binding: the PR cannot open without the computed percentage stated against that bar and a non-empty triage file per corpus repo — closing the exact gap where `duplicate-code`'s own ~60% false-positive rate (`docs/duplicate-code-false-positives.md`) was logged and never acted on.

## Unresolved Questions
*(Anything still unknown at plan-approval time. Each item must be resolved before the story that depends on it starts. If none, write "None.")*

1. **Exact per-grammar representation of the literal `-1`** — stack.md flags this as genuinely unverified (whether it's a single literal node or a `unary_expression` wrapping `1`, per language). Resolved by Task 1.1.1b's `to_sexp()` probe before `MAGIC_LITERAL_ALLOWLIST` matching is finalized; must complete before Epic 1.2 (emit logic) starts.
2. **Whether tree-sitter-python 0.23.6 tokenizes an f-string as the same `string` node kind `literal_kinds` walks** (vs. a distinct interpolated-string kind) — affects whether Python f-strings are visited at all. Resolved during Task 1.1.1e by inspecting `codegen/node-types/python.json` directly; must complete before that task's `literal_kinds` value is finalized.
3. **Multi-value destructuring bindings** (`const [a, b] = [42, 42]`, Go's `a, b := 42, 42`) are a known, accepted gap: `binding_finder` implementations only handle a single-name/single-value binding shape. This is a documented false-positive risk (an already-correctly-factored destructured constant would still be flagged), not fixed in v1 — tracked for the backtest to surface real-world frequency, not resolved here.

## Dependency Visualization

```
Phase 1: Core Rule Implementation
  Epic 1.0 (refactor-first: split lang_config())
   1.0.1a (extract 8 constructor fns)
     │
   1.0.1b (verify cargo test unchanged)
     │
     ▼
  Epic 1.1 (LangRuleConfig fields, per-language)  Epic 1.2 (walk/collect/emit)  Epic 1.3 (CATALOG wiring)
   1.1.1a (struct fields)                                                              │
     │                                                                                  │
   1.1.1b (verify -1 repr) ──────────────────┐                                          │
     │                                        ▼                                         │
   1.1.1c..h (8 per-lang arms)          1.2.1a (consts + LiteralCollector)               │
     │                                        │                                         │
     └───────────────┬────────────────────────┘                                         │
                      ▼                                                                  │
              1.2.1b (walk_literals)                                                     │
                      │                                                                  │
              1.2.1c (generalize collect_identifiers)                                    │
                      │                                                                  │
              1.2.1d (resolve_excluded_constants)                                        │
                      │                                                                  │
              1.2.1e (is_allowlisted_literal + emit_literal_findings)                    │
                      │                                                                  │
              1.2.1f (wire into check(), is_generated guard)                             │
                      │                                                                  │
                      └───────────────────────────────────────────────────► 1.3.1a (CATALOG entry)
                                                                                          │
                                                                              1.3.1b (description() string)
                                                                                          │
                                                                              1.3.1c (grammar-validity test)
                                                                                          │
                                                                                          ▼
Phase 2: Testing (2.1.1a..j) ── depends on all of Phase 1
                      │
                      ▼
Phase 3: Validation
  3.1.1a → 3.1.1b (transcript backtest)      3.2.1a → 3.2.1b (corpus backtest + triage)
                      │                                      │
                      └──────────────────┬───────────────────┘
                                          ▼
                              3.2.2a (threshold decision, pre-committed 40% bar — may loop back to 1.2.1a)
                                          │
                                          ▼
                              3.2.2b (test-file scope exclusion, if triage shows fixture dominance)
                                          │
                                          ▼
                              3.2.3a (merge-blocking evidence check — PR cannot open without this)
                                          │
                                          ▼
Phase 4: Documentation (4.1.1a..c, 4.2.1a) ── depends on Phase 3's final threshold value being known
```

---

## Phase 1: Core Rule Implementation

### Epic 1.0: Refactor-first — split `lang_config()` before extending it
**Goal**: Resolve the architecture review's BLOCKER (Lens 4, tech-debt disposition): `lang_config()`
(`src/rules.rs:487-703`) is already 217 lines / 5.4x over the `long-function` threshold
(`kibitzer run src --trigger batch` → `src/rules.rs:487: [long-function] body spans 217 lines
(over 40)`); Epic 1.1's per-language field additions must land on top of a function already
brought under threshold, not stack another instance of the same violation onto it. This is a
pure mechanical extraction — no behavior change — consistent with this file's own existing idiom
of small named per-language helper functions living just above `lang_config()` (`field_body`
`src/rules.rs:146`, `field_params` `:150`, `go_param_identifier_count` `:154`, `kotlin_body`
`:397`, `kotlin_params` `:405`).

#### Story 1.0.1: Extract each match arm into its own constructor function
**As a** kibitzer maintainer, **I want** `lang_config()` reduced to a thin per-language dispatcher,
**so that** Epic 1.1 can add the two new fields without growing an already-oversized function
further, and so `lang_config()` itself drops back under the `long-function` threshold.
**Acceptance Criteria**:
- `lang_config()`'s 8 match arms (`src/rules.rs:487-703`) are each moved verbatim into a new,
  small, named function — `go_lang_config() -> LangRuleConfig`, `typescript_lang_config() ->
  LangRuleConfig`, `tsx_lang_config() -> LangRuleConfig`, `javascript_lang_config() ->
  LangRuleConfig`, `python_lang_config() -> LangRuleConfig`, `java_lang_config() ->
  LangRuleConfig`, `kotlin_lang_config() -> LangRuleConfig`, `rust_lang_config() ->
  LangRuleConfig` — with `lang_config()` itself reduced to `match lang { Language::Go =>
  go_lang_config(), Language::TypeScript => typescript_lang_config(), ... }`.
  - *Given* `src/rules.rs` after this change, *When* `kibitzer run src --trigger batch` is run
    again, *Then* `src/rules.rs:487: [long-function] ...` no longer fires (the dispatcher is
    ~20 lines; each extracted constructor function is comfortably under 40 lines, since each was
    one match arm's worth of struct-literal fields).
- The refactor is behavior-preserving: no test edits are needed.
  - *Given* the full `cargo test --lib` suite passing before this change, *When* it's run again
    after the extraction, *Then* every existing test still passes unchanged (a diff in this
    Story touches only function boundaries, not struct-literal field values).
**Files**: `src/rules.rs`

##### Task 1.0.1a: Extract the 8 match arms into named constructor functions (~15 min)
*(Sized above this plan's usual 2-5 min/task range — Engineering-lens triad review flagged this;
kept as one task rather than split into 8 because it's a single mechanical, low-risk,
copy-paste-and-rename operation with no per-arm decision-making, and splitting it would add
task-tracking overhead without reducing real risk — the risk this size raises is instead
addressed directly by Task 1.0.1b's field-by-field equivalence check below, not by task
granularity.)*
- For each of the 8 `Language` variants in `lang_config()` (`src/rules.rs:487-703`), cut that
  arm's `LangRuleConfig { ... }` struct literal into a new standalone function
  (`fn go_lang_config() -> LangRuleConfig { LangRuleConfig { ... } }`, etc.), placed near the
  other per-language helper functions already living above `lang_config()` (after
  `kotlin_params`, `src/rules.rs:405-410`). `Language::Tsx` and `Language::JavaScript`'s
  `..lang_config(Language::TypeScript)` struct-update syntax (`src/rules.rs:550`, `:555`)
  becomes `..typescript_lang_config()` in their own extracted functions. Replace the original
  match body with `match lang { Language::Go => go_lang_config(), Language::TypeScript =>
  typescript_lang_config(), Language::Tsx => tsx_lang_config(), Language::JavaScript =>
  javascript_lang_config(), Language::Python => python_lang_config(), Language::Java =>
  java_lang_config(), Language::Kotlin => kotlin_lang_config(), Language::Rust =>
  rust_lang_config() }`. No field values change.
- Files: `src/rules.rs`

##### Task 1.0.1b: Verify no behavior change (~5 min)
*(Pre-mortem Failure #5, P2: "all existing tests pass" is a behavioral check, not a field-level
one — a struct-literal field silently copied into the wrong constructor, or a stray
`..typescript_lang_config()` base misapplied to `tsx`/`javascript`, would go undetected if none
of the existing tests happen to exercise that exact field for that exact language. The temporary
equivalence probe below closes that gap, matching the pattern Task 1.1.1b already uses for the
`-1` node-shape probe.)*
- Add a temporary `#[test]` that calls `lang_config(lang)` for each of the 8 `Language` variants
  both before and after the extraction (e.g. via a throwaway git-stash/diff of `{:?}`-formatted
  output, or an inline `assert_eq!` against a captured pre-extraction snapshot) and asserts every
  field of the returned `LangRuleConfig` is identical — not just that downstream `Finding`s are
  unchanged. Delete this temporary probe once confirmed (it has no ongoing value once the
  extraction is verified — the 8 real per-language tests in Epic 2.1 are the permanent
  regression coverage).
- Run `cargo test --lib` and `kibitzer run src --trigger batch`; confirm all existing tests pass
  unchanged (no test edits) and that `src/rules.rs:487: [long-function]` no longer appears in
  the batch output.
- Files: none (verification task; the temporary probe is written and deleted within this task,
  never committed)

---

### Epic 1.1: Extend `LangRuleConfig` with literal and binding data
**Goal**: Every one of the 8 `lang_config()` arms gains verified `literal_kinds`/`binding_finder` values, following the exact per-language verification discipline stack.md already used. Every `*_binding` function below returns `Option<ConstBinding>` (the named struct defined by Task 1.1.1a, per the Domain Glossary), not an anonymous `(String, Node)` tuple — this applies uniformly to all 8 languages' binding functions even where a task's prose below doesn't spell out the exact return expression.

#### Story 1.1.1: Add struct fields and per-language node-kind/binding data
**As a** kibitzer maintainer, **I want** `LangRuleConfig` to expose which node kinds are literals and how to find a named-constant binding per language, **so that** the new rule can reuse the existing table-driven dispatch instead of hardcoding per-language logic in the walk itself.
**Acceptance Criteria**:
- `LangRuleConfig` has `literal_kinds: &'static [&'static str]` and `binding_finder: fn(Node, &[u8]) -> Option<ConstBinding>` fields, documented like their siblings. **Fixed ordering (2nd triad-review round caught this as a real forward-reference compile error in the prior draft):** `ConstBinding` and `LiteralOccurrence` are now defined in Task 1.1.1a itself — the same task that adds the two `LangRuleConfig` fields — not in Task 1.2.1a. Epic 1.1's per-language `*_binding` functions (Tasks 1.1.1c–h) can then reference `ConstBinding` without a forward reference, since Task 1.1.1a runs first within Epic 1.1 and Epic 1.1 as a whole is already sequenced before Epic 1.2 in this plan. Task 1.2.1a (Phase 1.2) now only adds the `LiteralCollector` struct and the two constants, referencing the already-defined `ConstBinding`/`LiteralOccurrence`.
  - *Given* the `LangRuleConfig` struct definition, *When* `cargo build` runs, *Then* it compiles with the two new fields present and every `lang_config()` match arm populating them (a missing field in any arm is a compile error, matching every other field's exhaustiveness guarantee).
- Each of the 8 languages' `literal_kinds` matches stack.md's verified table exactly (Go: `int_literal, float_literal, imaginary_literal, rune_literal, interpreted_string_literal, raw_string_literal`; TS/JS/Tsx: `number, string`; Python: `integer, float, string`; Java: 6 numeric kinds + `string_literal`; Kotlin: `number_literal, float_literal, string_literal, multiline_string_literal`; Rust: `integer_literal, float_literal, string_literal, raw_string_literal`).
- **Numeric-vs-string split for message labeling (2nd triad-review round: this was referenced by
  Task 1.2.1e but never actually defined anywhere — fixed by adding the `numeric_literal_kinds`
  field above).** Each language's `numeric_literal_kinds` value (a subset of its `literal_kinds`)
  is: Go `&["int_literal", "float_literal", "imaginary_literal", "rune_literal"]`; TS/JS/Tsx
  `&["number"]`; Python `&["integer", "float"]`; Java the 6 numeric kinds; Kotlin
  `&["number_literal", "float_literal"]`; Rust `&["integer_literal", "float_literal"]`.
  `emit_literal_findings` (Task 1.2.1e) labels an occurrence "numeric" if its
  `LiteralOccurrence.kind` is in `cfg.numeric_literal_kinds`, "string" otherwise.
  - *Given* Go source `` `x := "hi"` ``, *When* `walk_literals` visits the `interpreted_string_literal` node, *Then* it's recognized as a literal (present in `literal_kinds`) and its raw text `"hi"` (quotes included) is used as the map key.
**Files**: `src/rules.rs`

##### Task 1.1.1a: Define `ConstBinding`/`LiteralOccurrence` and add the two new `LangRuleConfig` struct fields (~5 min)
- Add `pub(crate) struct ConstBinding<'tree> { name: String, initializer: Node<'tree> }` and
  `pub(crate) struct LiteralOccurrence<'tree> { node: Node<'tree>, kind: &'static str }` near
  `LangRuleConfig`'s own definition (`src/rules.rs:72`), before it — defined here (not in Task
  1.2.1a) specifically so Epic 1.1's per-language `*_binding` functions (Tasks 1.1.1c–h) can
  reference `ConstBinding` without a forward reference (2nd triad-review round caught the
  original draft's ordering as a real compile error: `ConstBinding` was defined in Phase 1.2 but
  used starting in Epic 1.1).
- Add `literal_kinds`, `numeric_literal_kinds`, and `binding_finder: fn(Node, &[u8]) -> Option<ConstBinding>` fields after the existing `panic_detector` field (`src/rules.rs:143`, before the closing `}` of the struct at line 144), each with a doc comment describing its role, matching the style of `terminal_kinds`'/`body_finder`'s existing doc comments. `numeric_literal_kinds`' doc comment notes it must be a subset of `literal_kinds` (checked by Task 1.3.1c's extended grammar-validity test).
- Files: `src/rules.rs`

##### Task 1.1.1b: Verify `-1`'s per-grammar node shape (~5 min)
- Temporarily add a `#[test]` (same technique as stack.md's own `temp_probe_magic_literal_candidate_kinds`) that parses `x := -1`-equivalent source in each of the 8 grammars and prints `to_sexp()`, run via `cargo test --bin kibitzer rules::tests::temp_probe_negative_one -- --nocapture`, then delete the temporary test.
- Record the result as a one-line doc comment above `MAGIC_LITERAL_ALLOWLIST` (Task 1.2.1a) stating, per language, whether `-1` is a single literal node (allow-list matches directly) or a `unary_expression`/`negative_number`-wrapper (allow-list must match the wrapped literal's text, or the wrapper is simply never a `literal_kinds` node and `-1` silently isn't exempted — state which, per language).
- Files: `src/rules.rs` (temporary, reverted before commit)

##### Task 1.1.1c: Go — `literal_kinds`/`numeric_literal_kinds` + `go_const_binding` (~5 min)
- In `go_lang_config()` (extracted from `lang_config()`'s Go arm by Epic 1.0's Task 1.0.1a), add the `literal_kinds` value from Story 1.1.1's AC, `numeric_literal_kinds: &["int_literal", "float_literal", "imaginary_literal", "rune_literal"]`, and a new `go_const_binding` function (near the other Go-specific helpers, e.g. `go_param_identifier_count`) that matches `const_declaration` → `const_spec` children, returning `Some(ConstBinding { name: name_text, initializer: value_node })` when `value_node.kind()` is in `literal_kinds` and there is exactly one name/value pair (multi-value `const_spec`s return `None` per Unresolved Question 3).
- Files: `src/rules.rs`

##### Task 1.1.1d: TypeScript — `literal_kinds`/`numeric_literal_kinds` + `ts_js_const_binding` (~5 min)
- In `typescript_lang_config()` (extracted by Task 1.0.1a), add `literal_kinds: &["number", "string"]`, `numeric_literal_kinds: &["number"]`, and a new `ts_js_const_binding` function matching `lexical_declaration` whose `kind` field's child `.kind() == "const"` (not `"let"` — Pattern Decision "JS/TS/Rust binding scope"), then its single `variable_declarator` child's `name`/`value` fields.
- Tsx and JavaScript inherit both fields automatically via `tsx_lang_config()`/`javascript_lang_config()`'s `..typescript_lang_config()` (post-Task-1.0.1a) — no separate edit needed for them.
- Files: `src/rules.rs`

##### Task 1.1.1e: Python — `literal_kinds`/`numeric_literal_kinds` + `py_screaming_snake_binding` (~5 min)
- Before writing code, inspect `codegen/node-types/python.json` for the f-string node shape in this pinned grammar version (Unresolved Question 2) and note the finding in a one-line comment next to the new field.
- In `python_lang_config()` (extracted by Task 1.0.1a), add `literal_kinds: &["integer", "float", "string"]`, `numeric_literal_kinds: &["integer", "float"]`, and a new `py_screaming_snake_binding` function matching an `assignment` node whose `left` field is an `identifier` matching `^[A-Z][A-Z0-9_]*$` and whose `right` field is directly a `literal_kinds` node.
- Files: `src/rules.rs`

##### Task 1.1.1f: Java — `literal_kinds`/`numeric_literal_kinds` + `java_final_binding` (~5 min)
- In `java_lang_config()` (extracted by Task 1.0.1a), add the 7-kind `literal_kinds` list, `numeric_literal_kinds` as the same list minus `string_literal` (the 6 numeric kinds), and a new `java_final_binding` function matching `local_variable_declaration`/`field_declaration` nodes with a raw (unnamed-children-included) `final` child in their `modifiers`, then their `declarator` field's `variable_declarator` `name`/`value`.
- Files: `src/rules.rs`

##### Task 1.1.1g: Kotlin — `literal_kinds`/`numeric_literal_kinds` + `kotlin_val_binding` (~5 min)
- In `kotlin_lang_config()` (extracted by Task 1.0.1a), add `literal_kinds: &["number_literal", "float_literal", "string_literal", "multiline_string_literal"]`, `numeric_literal_kinds: &["number_literal", "float_literal"]`, and a new `kotlin_val_binding` function matching `property_declaration` whose first raw child is the anonymous `val` token (not `var`), then its positional `variable_declaration` (name) and sibling `expression` (initializer) — same positional-lookup style as existing `kotlin_body`/`kotlin_params`.
- Files: `src/rules.rs`

##### Task 1.1.1h: Rust — `literal_kinds`/`numeric_literal_kinds` + `rust_const_binding` (~5 min)
- In `rust_lang_config()` (extracted by Task 1.0.1a), add `literal_kinds: &["integer_literal", "float_literal", "string_literal", "raw_string_literal"]`, `numeric_literal_kinds: &["integer_literal", "float_literal"]`, and a new `rust_const_binding` function matching `const_item`/`static_item` (not `let_declaration` — Pattern Decision "JS/TS/Rust binding scope"), reading their flat `name`/`value` fields directly.
- Files: `src/rules.rs`

---

### Epic 1.2: Two-pass literal walk and finding emission
**Goal**: A collect-then-emit pass that finds repeated literals, applies the allow-list and named-constant exclusion, and emits one `Finding` per qualifying literal.

#### Story 1.2.1: Implement `LiteralCollector`, `walk_literals`, exclusion resolution, and emission
**As a** kibitzer user, **I want** a repeated non-trivial literal flagged once with every occurrence's line listed, **so that** I (or an agent) can extract a named constant without re-grepping the file.
**Acceptance Criteria**:
- A literal repeated ≥2 times (and not allow-listed, not excluded) produces exactly one `Finding`, anchored at the first occurrence, listing the rest inline.
  - *Given* Go source `func f() { a := 86400\n b := 86400\n }`, *When* `syntax-rules`'s `check()` runs, *Then* it returns a `Finding{ line: 1, message: "[replace-magic-literal] numeric literal 86400 appears 2 times (also line 2) — consider extracting a named constant" }` (plus no findings from the other 4 rules, since the function is short and shallow).
- A named-constant-bound literal whose name is referenced ≥1 time beyond its own declaration is excluded; other *raw* occurrences of the same value elsewhere in the file are still counted and still flagged if they alone reach the threshold.
  - *Given* Go source `const Timeout = 30\nfunc f() { wait(Timeout) }\nfunc g() { wait(Timeout) }\nfunc h() { sleep(30) }`, *When* `check()` runs, *Then* the `const`-bound `30` is excluded (Timeout is referenced twice beyond its declaration) but the 4th line's raw `30` is *not* separately flagged (only 1 raw occurrence remains — below `MAGIC_LITERAL_MIN_OCCURRENCES`).
- Generated files skip the new pass entirely; the existing 4 rules are unaffected.
  - *Given* a Go file beginning with `// Code generated by protoc-gen-go. DO NOT EDIT.` and containing the literal `1` repeated 50 times, *When* `check()` runs, *Then* zero `[replace-magic-literal]` findings are produced (other rules still run normally).
**Files**: `src/rules.rs`

##### Task 1.2.1a: Define `LiteralCollector` and constants (~4 min)
- `ConstBinding`/`LiteralOccurrence` are already defined by Task 1.1.1a (moved there to fix a
  forward-reference ordering issue — see Epic 1.1's Goal note) — this task only adds the
  collector struct and constants that build on them.
- Add `LiteralCollector<'tree>` (fields `occurrences: HashMap<String, Vec<LiteralOccurrence<'tree>>>`, `bound: Vec<ConstBinding<'tree>>`, both defaulting empty).
- Add `MAGIC_LITERAL_MIN_OCCURRENCES: usize = 2` and `MAGIC_LITERAL_ALLOWLIST: &[&str] = &["0", "1", "-1", ""]` (the allow-list now holds **normalized** values — empty string covers every language's empty-literal form once `normalize_literal_value` strips delimiters/prefixes in Task 1.2.1e; it is no longer `"\"\""`/`"''"` raw text, which never matched Kotlin/Rust/Go's non-basic empty forms) near the top of `src/rules.rs`, alongside `LONG_FUNCTION_LINES` etc. (`src/rules.rs:10-18`). Incorporate Task 1.1.1b's `-1` finding into the allow-list's doc comment.
- Files: `src/rules.rs`

##### Task 1.2.1b: Implement `walk_literals` (~5 min)
- Add `walk_literals` as a sibling of `walk_declarations`/`walk_blocks` (after `walk_blocks`, `src/rules.rs:766`): for each node, if `cfg.literal_kinds.contains(&node.kind())`, push `LiteralOccurrence { node, kind: node.kind() }` into `occurrences` keyed by its raw `utf8_text` (the map key stays the raw text — the allow-list normalization in Task 1.2.1e is separate from the occurrence-grouping key); separately call `(cfg.binding_finder)(node, src)` and push any `Some(ConstBinding)` into `bound`; then recurse into children (same shape as `walk_blocks`).
- Files: `src/rules.rs`

##### Task 1.2.1c: Generalize `collect_identifiers` to a counting map (~4 min)
- Change `collect_identifiers`'s (`src/rules.rs:898-908`) signature from `out: &mut HashSet<String>` / `out.insert(...)` to `out: &mut HashMap<String, usize>` / `*out.entry(text.to_string()).or_insert(0) += 1`.
- Update `collect_condition_identifiers`'s signature (`src/rules.rs:881-896`) and its one internal call site to match; update `check_declaration`'s `branched_on.contains(&name)` (`src/rules.rs:861`) to `branched_on.contains_key(&name)`.
- Files: `src/rules.rs`

##### Task 1.2.1d: Implement `resolve_excluded_constants` (~4 min)
- Add a function that, given the tree root, `src`, and `collector.bound` (`Vec<ConstBinding>`), computes identifier reference counts once (via the now-generalized `collect_identifiers`) and returns a `HashSet<usize>` of `Node::id()`s (from each `ConstBinding.initializer`) for every `bound` entry whose `name`'s count is ≥2 (declaration + ≥1 real use).
- Files: `src/rules.rs`

##### Task 1.2.1e: Implement `normalize_literal_value`, `is_allowlisted_literal`, and `emit_literal_findings` (~7 min)
- `normalize_literal_value(raw: &str, kind: &str) -> &str`: strips the language-specific delimiter/prefix so an empty-literal comparison is structural, not a fixed-string match — e.g. strip surrounding `"`/`'` for basic strings, `r"`/`r#"..."#`/`r##"..."##` for Rust raw strings, backticks for Go raw strings, `"""`/`""""""` for Kotlin multiline strings, **and Python's leading `r`/`b`/`f`/`rb`/`br`/`rf`/`fr` string-prefix letters (2nd triad-review round: the original spec covered Rust/Go/Kotlin but omitted this, which would have left `r""`/`b""` incorrectly flagged as a distinct non-empty literal from `""`)** — then return the remaining inner text (empty for an empty literal of any of these forms). This directly fixes the pre-mortem/adversarial-review finding that a fixed raw-text allow-list (`"\"\""`, `"''"`) never matches Kotlin/Rust/Go's non-basic empty forms.
- `is_allowlisted_literal(raw: &str, kind: &str) -> bool`: `MAGIC_LITERAL_ALLOWLIST.contains(&normalize_literal_value(raw, kind))`.
- `emit_literal_findings`: for each `(text, occurrences)` in the collector's `occurrences: HashMap<String, Vec<LiteralOccurrence>>`, skip if `is_allowlisted_literal(&text, occurrences[0].kind)`; retain only occurrences whose node id isn't in the excluded set; skip if the remaining count is below `MAGIC_LITERAL_MIN_OCCURRENCES`; sort by line, derive the `numeric`/`string` label from whether `cfg.numeric_literal_kinds.contains(&occurrences[0].kind)` (see Story 1.1.1's numeric-vs-string split table — not from the raw text's first character), and push one `Finding` per the message format in this story's first AC.
- Files: `src/rules.rs`

##### Task 1.2.1f: Wire the new pass into `check()` (~4 min)
- In `SyntaxRulesChecker::check()` (`src/rules.rs:732-742`), after the existing two walks, add: build a `LiteralCollector`, call `walk_literals`, call `resolve_excluded_constants`, call `emit_literal_findings` — all guarded by `if !file_size::is_generated(ctx.source) { ... }` (Pattern Decision "Generated-file noise/cost"). Add `use crate::file_size;` if not already imported in this file.
- Files: `src/rules.rs`

---

### Epic 1.3: Catalog and checker-metadata wiring
**Goal**: The new rule is discoverable via `CATALOG` and the checker's own `description()`, and the grammar-validity safety net covers its new node-kind fields.

#### Story 1.3.1: Register in `CATALOG`, update `description()`, extend the grammar-validity test
**Acceptance Criteria**:
- `CATALOG` lists `replace-magic-literal` with `Severity::Advisory`.
  - *Given* `rules::CATALOG`, *When* iterated, *Then* it contains a `RuleMeta { id: "replace-magic-literal", category: "duplication", default_severity: Severity::Advisory, .. }` entry.
- Every `literal_kinds`/binding-related node-kind string is verified by `node_kind_literals_are_valid_for_their_grammar`.
  - *Given* the test's `assert_valid` helper, *When* run against all 8 languages, *Then* it also asserts every `literal_kinds` entry is a valid named node kind (and, for the anonymous `const`/`val`/`final` tokens `binding_finder`s check directly rather than through `literal_kinds`, a parallel assertion using `id_for_node_kind(_, false)` per stack.md's documented anonymous-token caveat).
**Files**: `src/rules.rs`

##### Task 1.3.1a: Add the `CATALOG` entry (~2 min)
- Add a `RuleMeta` entry after `unreachable-code` (`src/rules.rs:59-64`): `id: "replace-magic-literal"`, `category: "duplication"`, `description: "A non-trivial numeric or string literal is repeated 2+ times in one file with no bound named constant — Fowler's Replace Magic Literal."`, `default_severity: Severity::Advisory`.
- Files: `src/rules.rs`

##### Task 1.3.1b: Update `description()`'s rule list (~1 min)
- Append `, replace-magic-literal` to the string in `SyntaxRulesChecker::description()` (`src/rules.rs:721`).
- Files: `src/rules.rs`

##### Task 1.3.1c: Extend the grammar-validity test (~5 min)
- In `node_kind_literals_are_valid_for_their_grammar`'s `assert_valid` helper (`src/rules.rs:1172-1190`), add `kinds.extend(cfg.literal_kinds);` to the existing `kinds` vec build-up. Add a second, small assertion loop (or a second helper) covering the anonymous binding tokens (`const`, `val`, `final`) each `binding_finder` checks, using `ts_lang.id_for_node_kind(token, false)` (not the existing `true`/named-only call), matching stack.md's documented caveat that these are `"named": false` in `node-types.json`.
- Add a third assertion (per language) that every entry in `cfg.numeric_literal_kinds` is also present in `cfg.literal_kinds` — `numeric_literal_kinds` is defined as a subset and this keeps that invariant compiler-adjacent instead of silently divergeable.
- Files: `src/rules.rs`

---

## Phase 2: Testing

### Epic 2.1: Unit tests per language
**Goal**: Every language gets true-positive, allow-list, and named-constant-exclusion coverage, following the existing `flags_long_function`/`ts_flags_deep_nesting`/`py_flags_long_parameter_list` naming convention.

#### Story 2.1.1: True-positive, allow-list, and exclusion tests, all 8 languages
**Acceptance Criteria**:
- Each language has at least one true-positive test and one exclusion test; Go additionally covers the allow-list and the generated-file guard (representative — not duplicated 8x, per this file's existing convention of putting the most detailed coverage on Go and lighter coverage on the rest).
  - *Given* the Go true-positive fixture from Story 1.2.1's first Given-When-Then, *When* `cargo test --lib rules::tests::flags_replace_magic_literal` runs, *Then* it passes.
  - *Given* the JS/TS `let` fixture (`let x = 42; use(x); let y = 42;` — two raw `42`s, one `let`-bound with a real second use), *When* `cargo test --lib rules::tests::ts_flags_replace_magic_literal_excludes_const_not_let` runs, *Then* it asserts the finding **does** fire (locking in Pattern Decision "JS/TS/Rust binding scope": `let` does not exempt).
**Files**: `src/rules.rs`

##### Task 2.1.1a: `flags_replace_magic_literal` (Go, ~3 min)
- Add near `flags_long_function` (`src/rules.rs:1230`): asserts the two-`86400`-literal fixture from Story 1.2.1 produces the exact expected `Finding`.
- Files: `src/rules.rs`

##### Task 2.1.1b: `flags_replace_magic_literal_allowlist` (Go, ~3 min)
- Asserts a fixture with `0`, `1`, `-1`, `""` each repeated twice produces zero `replace-magic-literal` findings.
- Files: `src/rules.rs`

##### Task 2.1.1c: `flags_replace_magic_literal_excludes_named_constant` (Go, ~4 min)
- Asserts the `const Timeout = 30` + 2 real uses + 1 unrelated raw `30` fixture from Story 1.2.1's second Given-When-Then: zero findings (the lone remaining raw occurrence is below threshold).
- Files: `src/rules.rs`

##### Task 2.1.1d: TS `const`/`let` exclusion tests (~4 min)
- `ts_flags_replace_magic_literal` (true positive) and `ts_flags_replace_magic_literal_excludes_const_not_let` (per Story 2.1.1's second Given-When-Then) near `ts_flags_deep_nesting` (`src/rules.rs:1583`).
- Files: `src/rules.rs`

##### Task 2.1.1e: Python screaming-snake-case exclusion and prefixed-empty-string allow-list test (~5 min)
- `py_flags_replace_magic_literal` and `py_flags_replace_magic_literal_excludes_screaming_snake_case` near `py_flags_deep_nesting` (`src/rules.rs:1797`).
- `py_flags_replace_magic_literal_allowlist_prefixed_empty_string` *(added — 3rd triad-review round: closes the one asymmetry the round found, where gap 1's three allow-list tests each got a dedicated regression test but the Python `r`/`b`/`f`-prefix-stripping fix in `normalize_literal_value` didn't)*: a fixture with Python's empty raw string (`r""`) repeated twice, asserting zero findings.
- Files: `src/rules.rs`

##### Task 2.1.1f: Java `final` exclusion test (~4 min)
- `java_flags_replace_magic_literal` and `java_flags_replace_magic_literal_excludes_final` near `java_flags_deep_nesting` (`src/rules.rs:1947`).
- Files: `src/rules.rs`

##### Task 2.1.1g: Kotlin `val` exclusion and empty-multiline-string allow-list test (~5 min)
- `kotlin_flags_replace_magic_literal` and `kotlin_flags_replace_magic_literal_excludes_val` near `kotlin_flags_deep_nesting` (`src/rules.rs:2089`).
- `kotlin_flags_replace_magic_literal_allowlist_empty_multiline_string` *(added — 2nd triad-review round: adversarial-review.md asked for this and it was missed the first time)*: a fixture with Kotlin's empty triple-quoted string (`""""""`) repeated twice, asserting zero findings — the concrete regression test for `normalize_literal_value`'s Kotlin-delimiter-stripping behavior.
- Files: `src/rules.rs`

##### Task 2.1.1h: Rust `const`/`let` exclusion and empty-raw-string allow-list test (~5 min)
- `rust_flags_replace_magic_literal` and `rust_flags_replace_magic_literal_excludes_const_not_let` near `rust_flags_deep_nesting` (`src/rules.rs:2227`).
- `rust_flags_replace_magic_literal_allowlist_empty_raw_string` *(added — 2nd triad-review round)*: a fixture with Rust's empty raw string (`r""`) repeated twice, asserting zero findings.
- Files: `src/rules.rs`

##### Task 2.1.1h2: Go empty-backtick-raw-string allow-list test (~3 min) *(added — 2nd triad-review round)*
- `flags_replace_magic_literal_allowlist_empty_backtick_string`: a Go fixture with an empty backtick raw string repeated twice, asserting zero findings — Task 2.1.1b's existing Go allow-list test only covers `""`, not the backtick form.
- Files: `src/rules.rs`

##### Task 2.1.1i: Generated-file guard test (~3 min)
- `flags_replace_magic_literal_skips_generated_file`: a Go fixture prefixed with `file_size::is_generated`'s marker comment, literal `1` repeated 50 times — asserts zero `replace-magic-literal` findings.
- Files: `src/rules.rs`

##### Task 2.1.1j: Empty-file and no-qualifying-literal regression test (~3 min) *(added — Engineering-lens triad review)*
- `flags_replace_magic_literal_empty_file_produces_no_findings_and_does_not_panic`: run `check()` against an empty Go source string, asserting zero findings and no panic (the empty `HashMap`/`Vec` iteration in `walk_literals`/`emit_literal_findings` is panic-safe by construction, but this locks that in as a regression test rather than leaving it merely inferred).
- `flags_replace_magic_literal_no_literals_produces_no_findings`: a Go fixture with a function body containing no literal at all (e.g. `func f(a, b int) int { return a + b }`), asserting zero `replace-magic-literal` findings (and no panic from an empty `occurrences` map).
- Files: `src/rules.rs`

##### Task 2.1.1k: Full-suite regression run (~3 min)
- Run `cargo test --lib` (not just `rules::`) to catch any regression from the `collect_identifiers` signature generalization (Task 1.2.1c) rippling into `flag-argument`'s existing tests; fix any call-site the compiler flags.
- Files: none (verification task)

---

## Phase 3: Validation

### Epic 3.1: Transcript backtest (AC7)
**Goal**: Confirm the rule fires on real repeated-literal edits and doesn't misfire pathologically, per this repo's own CLAUDE.md mandate.

#### Story 3.1.1: Run and record the transcript backtest
**Acceptance Criteria**:
- `kibitzer check backtest` has been run for all 8 `syntax-rules-*` checker names against real transcript history, and the result is recorded.
  - *Given* the built `kibitzer` binary with this feature merged, *When* `kibitzer check backtest syntax-rules --transcripts-dir ~/.claude/projects` (and the 7 other `syntax-rules-*` names) is run, *Then* its output (fire count, any `(pre-existing)` tags) is captured and triaged by hand for obvious pathological noise before proceeding to Epic 3.2.
**Files**: none (validation activity; findings recorded in this plan's PR description per Proportionality)

##### Task 3.1.1a: Run the backtest (~5 min)
- `kibitzer check backtest syntax-rules --transcripts-dir ~/.claude/projects` and the 7 other language variants; capture output.
- Files: none

##### Task 3.1.1b: Triage and record (~5 min)
- Skim the output for whether it fires on genuine repeated-literal edits vs. obvious noise; note the verdict (proceed / needs-threshold-bump) — feeds Story 3.2.2's decision.
- Files: none

### Epic 3.2: Real-world corpus backtest (AC8) and threshold decision
**Goal**: Validate at scale against the fixed backtest-repo corpus, and use the result to make the ADR-001-flagged threshold decision explicitly rather than shipping `>=2` unexamined.

#### Story 3.2.1: Run the corpus backtest and triage findings
**Acceptance Criteria**:
- The checker has been run against the corpus in `docs/backtest-repos.md`, and true/false-positive verdicts are tracked via `scripts/backtest-triage.py`.
  - *Given* `kubernetes/kubernetes` cloned per `scripts/clone-backtest-repos.sh`, *When* `kibitzer run <path-to-kubernetes>` (or `kibitzer check native syntax-rules <file>` piped from `find`) is executed, *Then* `replace-magic-literal` findings appear in the output and a representative sample is triaged into `docs/backtest-triage/kubernetes-kubernetes/replace-magic-literal.jsonl` with `verdict: true_positive` or `false_positive` per record.
**Files**: `docs/backtest-triage/<repo-slug>/replace-magic-literal.jsonl` (new, one per corpus repo touched)

##### Task 3.2.1a: Run corpus scans (~5 min)
- Run against `kubernetes/kubernetes`, `apache/cassandra`, `servo/servo`, `BurntSushi/ripgrep`, `denoland/deno`, `microsoft/vscode`, `stapler-squad` (per `docs/backtest-repos.md`'s non-prose-only entries — Rust/Go/Java/TS are the relevant languages here, not the Markdown-only repos).
- Files: none (produces raw output for Task 3.2.1b)

##### Task 3.2.1b: Triage a representative sample (~5 min)
- Feed output through `scripts/backtest-triage.py`; record verdicts, focusing first on whether table-driven-test fixtures dominate the false-positive set the way `docs/duplicate-code-false-positives.md` documents for `duplicate-code`.
- Files: `docs/backtest-triage/<repo-slug>/replace-magic-literal.jsonl`

#### Story 3.2.2: Make the threshold decision (resolves the ADR-001 gate)
**Pre-mortem note (Failure #1, P1)**: a judgment call made *after* seeing the numbers is exactly
how `duplicate-code` justified shipping at "any repeat" despite knowing table-driven tests would
dominate. This story's decision rule is therefore pre-committed and quantitative, decided now
(at planning time), not calibrated to whatever the corpus happens to show.
**Acceptance Criteria**:
- The `>=2` vs `>=3` threshold decision is made from a pre-committed numeric bar, not a post-hoc
  judgment call.
  - *Given* the triage shards from Story 3.2.1, *When* the true/false-positive ratio is computed,
    *Then* if **≥40% of triaged findings are `false_positive` with a test/fixture/table-driven
    rationale** (the pre-committed bar, fixed now — not chosen after seeing the data),
    `MAGIC_LITERAL_MIN_OCCURRENCES` is bumped from 2 to 3 in the same PR (Task 1.2.1a) and the
    corpus subset with changed verdicts is re-run; below 40%, the constant stays at 2. Either
    way the computed percentage (not just "acceptable"/"needs bump") is recorded in the PR
    description next to the 40% bar it was checked against.
  - *Given* the same triage shards, *When* test/fixture files dominate the false-positive set
    regardless of whether the 40% bar is crossed, *Then* a `!**/*_test.go`-style `scope`
    exclusion (mirroring the pattern in `docs/suppressing-checks.md`'s "Exclude one file or
    directory" section) is added to `syntax_rules_checks()`'s `replace-magic-literal` entry in
    `src/config.rs` for test-file globs across all 8 languages (`*_test.go`, `*.test.ts`,
    `*.spec.ts`, `test_*.py`, `*_test.py`, `*Test.java`, `*Test.kt`, files under `tests/`) —
    closing the exact gap `duplicate-code` left open (`docs/duplicate-code-false-positives.md`
    flagged the same test-file dominance and no scoping fix was ever landed for it).
**Files**: `src/rules.rs` (only if the threshold changes), `src/config.rs` (test-file scope
exclusion, if triage shows test/fixture dominance per the second AC above)

##### Task 3.2.2a: Compute the ratio and decide (~5 min)
- Tally triage verdicts; compute the exact false-positive percentage; apply the pre-committed
  40% decision rule above (not a fresh judgment call); if bumping, edit
  `MAGIC_LITERAL_MIN_OCCURRENCES` (Task 1.2.1a's location) and re-run the affected corpus subset
  to confirm the noise drops.
- Files: `src/rules.rs` (conditional)

##### Task 3.2.2b: Add test-file scope exclusion if warranted (~5 min)
- If Task 3.2.2a's triage shows test/fixture files dominating the false-positive set (independent
  of whether the 40% threshold-bump bar was crossed), add the 8-language test-file glob exclusion
  described in this story's second AC to `replace-magic-literal`'s `scope` in
  `src/config.rs::syntax_rules_checks()`.
- Files: `src/config.rs` (conditional)

#### Story 3.2.3: Merge-blocking backtest evidence gate (Pre-mortem Failure #2, P1)
**Pre-mortem note**: `docs/duplicate-code-false-positives.md` already documents a ~60%
false-positive rate that was logged as "needs discussion" and never acted on — evidence
gathered but not binding. This story closes that gap for this feature specifically.
**Acceptance Criteria**:
- The PR cannot merge with backtest evidence gathered but ignored.
  - *Given* Story 3.2.1's triage output, *When* the PR is opened, *Then* the PR description
    states the exact computed true/false-positive percentage next to the pre-committed 40% bar
    from Story 3.2.2, and `docs/backtest-triage/<repo-slug>/replace-magic-literal.jsonl` exists
    with at least one non-empty `verdict` field for every corpus repo listed in
    `docs/backtest-repos.md`'s non-prose-only entries — a PR description asserting the rate is
    "acceptable" with no numeric bar cited, or a missing/all-empty triage file, is treated as an
    incomplete Phase 3 and blocks proceeding to Phase 4/7-ship.
**Files**: none (a PR-description and pre-merge-checklist requirement, not a code file)

##### Task 3.2.3a: Verify triage completeness before opening the PR (~2 min)
- Confirm every corpus repo's `docs/backtest-triage/<repo-slug>/replace-magic-literal.jsonl` has
  ≥1 verdict entry, and draft the PR description's evidence sentence (percentage + 40% bar +
  bump/no-bump decision) before requesting review.
- Files: none

##### Task 3.2.3b: Put the gate in the PR body itself, not only in this plan (~1 min)
*(Engineering-lens triad review: a plan-only requirement is enforced only if a reviewer
remembers to check for it — the exact enforcement gap that let `duplicate-code`'s own 60%
false-positive finding go unacted-on. Full CI automation of this check (a GitHub Actions step
that greps the PR body / checks file existence via `gh pr checks`) is deliberately out of scope
for this Small-appetite feature — it would need its own workflow change reviewed on its own
merits, not bundled here. This task is the proportionate middle ground: make the requirement
impossible to silently skip when opening the PR, without building new CI infrastructure.)*
- When running `/sdd:7-ship` (or `gh pr create` manually), include a literal checklist line in
  the PR body: `- [ ] replace-magic-literal backtest: <X>% false-positive rate vs. the 40% bar
  (Story 3.2.2) — decision: <bump to 3 / keep at 2>`, filled in with real numbers, not left as a
  template placeholder. An unfilled or absent line is a visible, reviewable signal in the PR
  itself (not just in this plan document) that Phase 3 wasn't completed.
- Files: none

---

## Phase 4: Documentation

### Epic 4.1: `docs/syntax-rules.md`
**Goal**: The new rule is documented alongside its 5 siblings, including the suppression-guidance sentence for table-driven-test false positives.

#### Story 4.1.1: Add the rule row, per-language bullet, and suppression guidance
**Acceptance Criteria**:
- The rule table has a 6th row matching the existing 5 rows' shape.
  - *Given* `docs/syntax-rules.md`'s table (`:28-34`), *When* read after this change, *Then* it has a `replace-magic-literal` row with Category `duplication`, Default severity `advisory`, Threshold `>= 2 occurrences` (or `>= 3`, per whatever Story 3.2.2 decided), and a one-sentence description mirroring the other rows' style.
- A maintainer hitting a table-driven-test false positive is pointed at both suppression levers.
  - *Given* the doc's existing "Thresholds are fixed constants..." paragraph (`:137-138`), *When* read after this change, *Then* a new sentence immediately follows it naming the `scope` exclusion glob (`docs/suppressing-checks.md`) for a whole fixture file and a `.kibitzer/accepted/` entry keyed `"rule": "replace-magic-literal"` (`docs/accepting-findings.md`) for one specific kept repeat.
**Files**: `docs/syntax-rules.md`

##### Task 4.1.1a: Add the table row (~2 min)
- Insert a `replace-magic-literal` row into the table at `docs/syntax-rules.md:28-34`, after the `unreachable-code` row.
- Files: `docs/syntax-rules.md`

##### Task 4.1.1b: Add the per-language bullet (~4 min)
- Append a bullet to the "Per-language node kinds" list (`docs/syntax-rules.md:36-117`) summarizing each language's `literal_kinds` and binding-exclusion form (const/let-const-only/final/val/screaming-snake-case), matching the existing bullets' citation style (`node-types.json`/`to_sexp()` references).
- Files: `docs/syntax-rules.md`

##### Task 4.1.1c: Add the suppression-guidance sentence (~2 min)
- Add the sentence from this story's second AC immediately after the existing threshold paragraph (`docs/syntax-rules.md:137-138`).
- Files: `docs/syntax-rules.md`

### Epic 4.2: Reconcile AC9's `docs/suppressing-checks.md` clause
**Goal**: Confirm the requirement is actually satisfied by existing structure rather than making a redundant edit.

#### Story 4.2.1: Verify existing coverage, document the reconciliation
**Acceptance Criteria**:
- `docs/suppressing-checks.md`'s catalog-name list is confirmed to already cover this rule (no CATALOG-level rule id like `long-function` is separately listed there either — only checker-level names like `syntax-rules-*`), so no edit is needed there; this reconciliation is stated in the PR description rather than silently skipped.
  - *Given* `docs/suppressing-checks.md:3-8`'s prose list, *When* re-read after this feature ships, *Then* it still reads `syntax-rules-*` (unchanged) and the PR description explicitly notes AC9's second clause is satisfied by this existing coverage, not by a new edit.
**Files**: none (verification + PR-description note)

##### Task 4.2.1a: Verify and note (~2 min)
- Re-read `docs/suppressing-checks.md:3-8`, confirm no other individual `CATALOG` rule id appears there, and add one sentence to the PR description explaining why `replace-magic-literal` needs no separate mention.
- Files: none
