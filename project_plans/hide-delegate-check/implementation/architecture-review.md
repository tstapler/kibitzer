# Architecture Review: hide-delegate-check
**Date**: 2026-09-22
**Verdict**: CONCERNS

## Constitution Check

`docs/adr/ADR-000-architecture-constitution.md` does not exist in this repository
(`docs/adr/` itself does not exist — confirmed via `ls docs/adr`). No constitution to check
plan elements against; no violations to report.

## Source-grounding note

Before applying the four lenses, the plan's structural claims about `src/rules.rs` were
checked against the real source (not just plan.md prose):

- `LangRuleConfig` (`src/rules.rs:72-144`), `walk_declarations`/`walk_blocks`
  (`src/rules.rs:745-766`), `check_block_for_unreachable` (`src/rules.rs:774-`),
  `SyntaxRulesChecker::check()`/`description()` (`src/rules.rs:715-743`), `CATALOG`
  (`src/rules.rs:34-65`), `kotlin_body`/`kotlin_params` (`src/rules.rs:397-409`),
  `no_panic_detector`/`go_panic_detector`/`rust_panic_detector`, and the Go/Tsx/JavaScript
  `lang_config()` arms were all read directly.
- Every per-language node-kind/field claim in plan.md (Go `selector_expression{operand,field}`/
  `call_expression{function,arguments}`; Java `method_invocation{object,name,arguments}`/
  `field_access{object,field}`; Kotlin `navigation_expression`/`call_expression` positional
  children; Rust `field_expression{value,field}`/`call_expression{function,arguments}`/
  `scoped_identifier{path,name}`; Python `attribute{object,attribute}`/`call{function,arguments}`;
  TS `member_expression{object,property,optional_chain}`/`call_expression{function,arguments}`)
  was independently re-derived from the vendored `codegen/node-types/*.json` files and matches
  plan.md exactly, field-for-field.
- `config.rs::syntax_rules_checks()` (`src/config.rs:717-738`) confirmed: 8 `Check` entries all
  pointing at `SyntaxRulesChecker`, verifying AC1's "zero `config.rs` changes" claim.
- `docs/suppressing-checks.md` confirmed: suppression levers key on checker `name`
  (`syntax-rules*`), not individual rule id — verifying the plan's Risk Control claim that
  `hide-delegate` can only be disabled by disabling the whole per-language checker.
- kibitzer's `architecture_checker`s (`import-cycles`/`coupling`, `.claude/inspect.json:108-116`)
  and `file-complexity` native check are Go/TS/JS-scoped only (`file-complexity`'s `scope` is
  `**/*.go`) — neither applies to this Rust-only change, consistent with the task brief's
  guidance to skip `kibitzer architecture export` here and rely on direct source reading instead.

Conclusion: the plan is unusually well-grounded — every specific technical claim checked came
back accurate. No fabricated node kinds, no stale line references, no misdescribed existing
structure.

## Blockers

None.

## Concerns

- [ ] **Epic 1.2 (`LangRuleConfig` field additions) — `LangRuleConfig` is a growing "fat"
  config struct, an emerging Interface Segregation concern — recommend a scoped follow-up, not a
  block on this PR.** The struct already holds fields for 5 unrelated rules
  (`long-function`/`deep-nesting`/`long-parameter-list`/`flag-argument`/`unreachable-code`); this
  plan adds 3 more (`dot_chain_kinds`, `chain_step`, `fluent_allowlist`) for a 6th, each requiring
  a no-op default in every one of the 7 `lang_config()` match arms regardless of whether that
  rule is relevant to a given language (Task 1.2.1c). Every consumer of `LangRuleConfig` — in
  principle including future rules that need none of the chain-related fields — now carries them.
  This is consistent with existing precedent (`body_finder`/`bool_param_finder`/`terminal_kinds`
  already establish "per-rule data lives in the table"), and the Tech Debt Disposition table
  correctly identifies "Extend as-is" as the right call for *this* change (a 6th occupant of an
  established pattern, not a fresh violation). But ISP's "many focused interfaces > one fat
  interface" argument only gets stronger with each addition, and nothing in the plan names a
  threshold at which this pattern should be revisited (e.g., splitting `LangRuleConfig` into
  composable per-rule sub-structs, or a `HashMap<&str, Box<dyn RuleConfig>>`-style registry).
  **Remediation**: not a change to this PR. Recommend a one-line note in `CLAUDE.md`'s "Default
  check catalog" section or a tracked backlog item: "if a 7th/8th native rule needs its own
  `LangRuleConfig` fields, evaluate splitting the struct into per-rule sub-configs before adding
  more flat fields" — so the decision isn't re-litigated from scratch by whoever lands rule #7.

- [ ] **Design Position 7 / `docs/check-ideas.md`'s "confirmed transcript occurrence" bar —
  explicitly bypassed, correctly disclosed, but worth flagging as a policy gap, not just an
  architecture one.** `CLAUDE.md`'s "Writing a new check" section and `docs/check-ideas.md`'s own
  stated bar require "2+ independent occurrences in real session transcripts" before a new native
  checker idea is considered validated. `hide-delegate` isn't in `check-ideas.md` at all — it
  arrived via `requirements.md`'s backlog item and GitHub issue #44 instead, and plan.md's Design
  Position 7 states this candidly rather than hiding it. The mandatory Phase 6 corpus/transcript
  backtest is the only evidence gate this check gets before merge, which is consistent with how
  `duplicate-code` shipped previously (cited precedent). **Remediation**: none required in
  plan.md itself — this is already handled about as well as it can be short of not shipping the
  check. Flagging only so the Phase 6 backtest is treated as load-bearing, not a formality: if it
  comes back noisy, the plan's own "narrow v1 scope" fallback (Risk Control section) should
  actually be exercised, not skipped under time pressure.

## Nitpicks

- Tech Debt Disposition's "confirmed not a hotspot" claim for `src/rules.rs`
  (research/architecture.md §7) is a qualitative argument (pattern precedent, zero new
  abstractions), not output from an actual churn×complexity hotspot-analysis tool run. Sanity
  check performed during this review: `git log --oneline -- src/rules.rs` shows 15 commits over
  the file's lifetime (2329 lines) — a moderate, expected-given-its-role churn signal, not
  alarming, and kibitzer's own `file-complexity` native check doesn't cover Rust (`**/*.go`
  scope only) so this can't currently be machine-verified in-repo either way. Not blocking — the
  specific change here is pure data-only additions mirroring 5x existing sibling-rule precedent,
  which is a low-risk shape regardless of the file's aggregate hotspot status.
- `ReceiverOrigin` enum and `ChainHopCount(usize)` newtype rejections (Pattern Decisions table)
  both hold up on inspection: `ChainHopCount` would wrap a purely-local counter never crossing a
  function/module boundary beyond a single comparison against `MAX_CHAIN_LINKS`, so it buys no
  real safety beyond the existing `usize` convention every sibling threshold already uses;
  `ReceiverOrigin` would have variants (`Stranger`/`LocalNew`) that can never be populated with
  confidence from a syntactic-only AST, which is a sum type whose extra states exist only to be
  permanently unreachable-with-confidence — correctly identified as not a real type-safety win.
- `is_chain_suppressed`'s "every hop must independently satisfy allowlist-or-prefix" semantics
  correctly and deliberately leaves `new Builder().setX().setY().build()` unsuppressed (the
  `build` terminal hop matches neither list) — documented as a known gap in both plan.md Story
  3.1.1 and ADR-001's Consequences section, with the corpus backtest named as the mechanism to
  decide whether it's worth a follow-up. No action needed; noting only because it's the single
  most likely real-world false-positive source and is worth double-checking gets genuine
  attention during the Phase 6 triage, not just a checkbox.
