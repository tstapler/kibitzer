# Validation Plan: hide-delegate-check

**Date**: 2026-09-22

## Happy Path Scenario
Given a repo with no `.claude/inspect.json`, when an agent/developer runs `kibitzer run <repo>`
against a source file containing `a.GetB().GetC().DoThing()`, then `config::default_checks()`'s
`syntax_rules_checks()` entries (already wired, zero new config) drive `SyntaxRulesChecker::check()`
to call the new `walk_chains`, which emits a `[hide-delegate]` finding on that 3-hop chain — while a
fluent-builder chain in the same file, `c.WithTimeout(5).WithRetries(3).WithHost("x")`, is correctly
left unflagged by `is_chain_suppressed`.

## Requirement → Test Mapping

| Requirement | Test File | Test Name | Type | Scenario |
|-------------|-----------|-----------|------|----------|
| AC1: wired into `default_checks()`, no `.claude/inspect.json` | `src/rules.rs` | `catalog_ids_are_unique_and_documented` (existing, line 1437) | Unit | Iterates `CATALOG` for id-uniqueness and non-empty descriptions; automatically re-validates once `hide-delegate`'s `RuleMeta` (Task 1.1.1b) is appended — **no new test needed** for this half of AC1. Confirmed the two candidate `src/config.rs` tests do *not* cover it: `default_checks_ids_are_unique`/`find_effective_config_falls_back_to_defaults_with_no_inspect_json` (config.rs:1163, 1173) assert on the per-language `Check` list (`"syntax-rules"`, `"syntax-rules-typescript"`, …), not on `rules::CATALOG`'s contents — and per the plan's own Tech Debt Disposition, `hide-delegate` adds zero new `Check` entries (it rides along inside the existing `syntax-rules*` checkers), so those two config.rs tests pass unchanged whether or not `hide-delegate` exists and give this AC no coverage. |
| AC1 (cont.) | `src/rules.rs` | `flags_hide_delegate` (Go, Task 4.2.1a) and its 6 per-language siblings (§AC5 rows below) | Unit | Each calls `walk_chains` directly — the exact function Task 1.3.1e wires into `SyntaxRulesChecker::check()` — so a passing test is direct evidence the check fires with zero project-level config, i.e. the wiring half of AC1. |
| AC1 (cont.) | `src/config.rs` | `find_effective_config_disable_removes_a_default_by_name` (existing, line 1237) | Unit (negative) | Generic, not `hide-delegate`-specific: confirms the *only* opt-out mechanism (disabling a whole `syntax-rules*` checker via `.claude/inspect.json`'s `disabled` list) works. Per the plan's Risk Control section, no `rules.rs` rule — including `hide-delegate` — has a finer-grained flag, so this existing generic test is the correct/only negative-path coverage available. |
| AC1 (cont.) | none (verification task) | Task 6.1.1a / 6.2.1b | Corpus backtest | `kibitzer run <repo>` against the real corpus with no per-repo `.claude/inspect.json` is the only true end-to-end proof of AC1 against real repos, as opposed to the in-process `walk_chains` unit tests above. |
| AC2: `MAX_CHAIN_LINKS` const, flags 3+ dot-accesses | `src/rules.rs` | `flags_hide_delegate` (Go) + 6 per-language siblings | Unit | Hop count 3 > `MAX_CHAIN_LINKS` (2) → flagged. |
| AC2 (cont.) | `src/rules.rs` | **`two_hops_not_flagged` — MISSING, see below** | Unit (negative/boundary) | Exactly 2 hops (at `MAX_CHAIN_LINKS`) must NOT be flagged. Not present anywhere in plan.md's Phase 4. |
| AC3: language scope (Go, TS, JS, Python, Java, Kotlin, Rust) | `src/rules.rs` | `flags_hide_delegate` / `ts_flags_hide_delegate` / `js_flags_hide_delegate` / `py_flags_hide_delegate` / `java_flags_hide_delegate` / `kotlin_flags_hide_delegate` / `rust_flags_hide_delegate` (Tasks 4.2.1a–4.2.7a) | Unit | **Confirmed**: this is implicit per-language scope coverage. Each test would fail if a future change silently reverted that language's `dot_chain_kinds`/`chain_step` to Phase 1's no-op defaults (`&[]`/`no_chain_step`), since `walk_chains` would then never find a hop and the finding would disappear — a passing suite of all 7 is a live regression guard against exactly the "language silently dropped" failure mode. |
| AC3 (cont.) | none (verification task) | Task 6.2.1c | Corpus backtest | Explicitly documents that Python and Kotlin have no exemplar in the current 7-repo corpus (`docs/backtest-repos.md`'s known gap) — those two languages' AC3 coverage is unit-tested but **not** corpus-validated; stated as a gap, not silently implied. |
| AC4: fluent-builder suppression heuristic (allowlist OR builder-verb prefix) | `src/rules.rs` | `go_suppresses_builder_chain`, `ts_suppresses_fluent_chain`, `js_suppresses_fluent_chain`, `py_prefix_heuristic_still_suppresses`, `java_suppresses_stream_chain`, `kotlin_suppresses_also_apply_chain`, `rust_suppresses_iterator_chain` (Tasks 4.2.1b–4.2.7b) | Unit | `rust_suppresses_iterator_chain` (Task 4.2.7b) is requirements.md's own canonical example (`data.iter().filter(...).map(...).collect()`), directly verifying AC4's text — resolved via `RUST_FLUENT_ALLOWLIST`, not literal same-return-type inference (see plan's Story 2.6.1 note that the literal "same type" reading doesn't actually apply here). |
| AC4 (cont.) | `src/rules.rs` | the 7 flagged tests above (same rows as AC2/AC3) | Unit (negative) | Non-prefix, non-allowlist hop names (`GetB`/`GetC`/`DoThing`, `getFoo`/`getBar`/`getBaz`, etc.) are *not* suppressed — proves the heuristic doesn't over-fire and exempt everything. |
| AC4 (cont.) | none (verification task) | Task 6.2.1a–6.2.1e | Corpus backtest | Rust iterator chains in `servo/servo`/`BurntSushi/ripgrep` are checked first as the most likely false-positive source (pitfalls.md §3/§5) — this is the corpus-level test of AC4's suppression heuristic, complementary to the unit tests. |
| AC5: per-language unit tests, flagged + suppressed pair | `src/rules.rs` | All 14 tests in Epic 4.2 (Tasks 4.2.1a/b – 4.2.7a/b), plus the `check_hide_delegate` harness (Task 4.1.1a) | Unit | This AC *is* Epic 4.2 verbatim — see the AC2/AC3/AC4 rows above for the individual scenarios; listed once more here for completeness since AC5 is literally "these tests exist." |
| AC6: backtested (transcripts + corpus), FP rate recorded | none (verification tasks, no `#[test]`) | Task 6.1.1a / 6.1.1b | **Corpus backtest** (not Unit/Integration) | `kibitzer check backtest syntax-rules` (+ the 6 other `syntax-rules-{lang}` names) against real Claude Code session transcripts (`~/.claude/projects/*/*.jsonl`), output filtered for `[hide-delegate]`, obviously-wrong fires noted in the PR description. |
| AC6 (cont.) | `docs/hide-delegate-false-positives.md` (created only if confirmed FPs found) | Task 6.2.1a–6.2.1e | **Corpus backtest** | `kibitzer run <repo>` across the 7-repo corpus, triaged with `scripts/backtest-triage.py`; true/false-positive counts recorded in the PR description (the literal AC6 text: "false-positive rate ... is recorded"). Any confirmed FP gets a documented fix to `fluent_allowlist`/`has_builder_verb_prefix`/`dot_chain_kinds`, budgeted as near-certain per pitfalls.md §5's precedent, before the check is considered done. |
| AC7: `RuleMeta` entry (category, id, description, severity) | `src/rules.rs` | `catalog_ids_are_unique_and_documented` (existing, line 1437) | Unit | Automatically re-validated once `hide-delegate`'s entry is appended (id-uniqueness + non-empty description) — same test as AC1's first row. |
| AC7 (cont.) | `src/rules.rs` (Task 1.1.1b) | none — manual code review only | Manual | No existing test in this codebase asserts a *specific* rule's `category`/`default_severity` field values (confirmed via grep for `Severity::Advisory` and `.category ==` across `src/rules.rs` — only the `CATALOG` literal definitions themselves match, no test-side assertions). Proposing a `hide-delegate`-only field-value test would be inconsistent with how every other `CATALOG` entry is verified (by eye against sibling entries at review time) — not proposed. |
| AC8: documented in `docs/syntax-rules.md` | `docs/syntax-rules.md` (Tasks 5.1.1a–5.1.1c) | none — grep-verifiable by a maintainer | Manual (Docs) | No automated test cross-references `docs/syntax-rules.md` against `rules::CATALOG` anywhere in this codebase today (confirmed via repo-wide grep for `syntax-rules.md` in `src/*.rs`/`tests/*.rs` — zero hits besides unrelated doc-comment mentions) — consistent with how every other `rules.rs` rule's doc entry is verified, i.e. not proposed as a new test. |

## Missing Test Design (to add to plan.md's Phase 4 before implementation)

Only one gap was found across all 8 ACs — AC2's exact-threshold boundary. Everything else in
plan.md's already-specified 18-test Phase 4 maps cleanly onto an AC (see table above); the plan's own
"18 tests" claim is accurate and does not need net-new test *functions* beyond this one.

### `two_hops_not_flagged` (Story 4.2.1, alongside `flags_hide_delegate`)
**Rationale**: AC2 requires the threshold to be an exact `> MAX_CHAIN_LINKS` (i.e. 3+) comparison,
not merely "long chains get flagged." Every planned test uses 3 or 4 hops; none exercises the
boundary at exactly 2. This mirrors the existing `five_params_is_not_long` test (`src/rules.rs:1427`,
`LONG_PARAM_LIST_COUNT`'s own boundary), which is this codebase's established convention for
threshold-boundary regression tests — `allows_shallow_nesting` (`deep-nesting`'s boundary test) is
looser (1 level, nowhere near `MAX_NESTING_DEPTH`), so `five_params_is_not_long`'s exact-boundary
shape is the closer analog for a rule defined by a small integer count.

- **Language**: Go (bare name, no `{lang}_` prefix — matches `flags_hide_delegate`'s own established
  precedent per Task 4.2.1a's note that Go's tests use bare naming, matching `flags_deep_nesting`).
- **Source**: `"package main\nfunc f(a A) {\n\ta.GetB().DoThing()\n}\n"` — 2 hops (`GetB`, `DoThing`),
  neither allowlisted nor prefix-matched.
- **Assertion**: `check_hide_delegate(Language::Go, tree_sitter_go::LANGUAGE.into(), src)` returns no
  finding containing `"[hide-delegate]"` — hop count 2 is not `> MAX_CHAIN_LINKS` (2), so `check_chain`
  must not push a `Finding` regardless of `is_chain_suppressed`.
- **Files**: `src/rules.rs`, placed next to Task 4.2.1a/b in Story 4.2.1.

## Test Stack
- **Unit**: Rust's built-in `#[test]` harness (`cargo test`), plain `assert!`/`assert_eq!` — no
  external assertion crate (`pretty_assertions`, `assert2`, etc.) is in `[dev-dependencies]`
  (`Cargo.toml`), consistent with every existing `src/rules.rs` test.
- **Integration**: Not applicable in the data-store sense — this codebase has no `tests/*.rs`
  integration test exercising any `rules.rs` rule through the full `config::default_checks()` →
  `Checker` → `Finding` pipeline (confirmed via repo-wide grep for `deep-nesting`/`long-function`,
  which hit only `src/rules.rs` and `src/mcp.rs`, never `tests/`). The closest thing to
  "integration" for this feature is the corpus backtest below.
- **Corpus backtest**: `kibitzer check backtest <checker-name>` against real Claude Code session
  transcripts (`docs/backtesting.md`) and `kibitzer run <repo>` against the public backtest corpus
  (`docs/backtest-repos.md`), triaged with `scripts/backtest-triage.py`
  (`docs/backtest-triage/README.md`) — per `CLAUDE.md`'s mandatory "Writing a new check" bar, both
  required before `hide-delegate` is considered done (AC6).

## Coverage Targets and How to Measure

| Stack | Coverage command | Target |
|---|---|---|
| Rust | `cargo llvm-cov --workspace --json --output-path coverage.json` then `covgate check coverage.json` (per `.github/workflows/ci.yml`'s `test with coverage` / `coverage gate (changed lines only)` steps) | `covgate.toml`'s diff-coverage gate: `fail-under-lines = 80`, `fail-under-regions = 75` on **lines/regions changed in this PR**, not overall repo coverage (which sits at ~88-89% per `covgate.toml`'s own comment). All of `src/rules.rs`'s new `walk_chains`/`check_chain`/`is_chain_suppressed`/per-language `chain_step` functions are changed lines under this PR and must individually clear 80%/75% diff coverage — the 14 per-language tests plus the 3 edge-case tests plus the new `two_hops_not_flagged` boundary test are what exercises them. |

- All public service methods: `walk_chains`/`check_chain`/`is_chain_suppressed`/`has_builder_verb_prefix` each get a happy path (flagged) and an error/negative path (suppressed or under-threshold) per language, per the table above.
- All external integrations: N/A — no external service/store; the corpus backtest (Task 6.2.1a-e) is the closest analog to an external-system integration test and is mandatory per AC6.
