# Validation Plan: inline-ignore-syntax

**Date**: 2026-10-07

**Amendment 1 (2026-10-07)**: single marker (`kibitzer:ignore` only; `kibitzer:false-positive` is a near-miss). The SM-3 and IS-4 listing rows and the two-marker rows were removed; any remaining "both markers" wording reads as "the marker". See plan.md Amendment 1.

## Happy Path Scenario
Given a Go file where `kibitzer run` and the PostToolUse hook report a `[flag-argument]` finding that the author has judged acceptable (today only dismissible through a hand-written `.kibitzer/accepted/*.json` entry that goes stale on line drift), when the author adds one line `// kibitzer:ignore flag-argument -- legacy API, callers pinned` directly above the flagged line, then the finding no longer appears in hook, MCP `run_checks` or `kibitzer run` output, stays suppressed after 5 lines are inserted above it, and the `kibitzer run` footer reports `1 findings suppressed inline`.

*Anchor test*: `run_checker_against_source_should_DropFinding_When_WholeLineIgnoreAbove` (+ `ignore_should_SurviveEdits_When_LinesInsertedAbove`).

## Test Stack
- **Unit**: Rust built-in `#[test]` in `#[cfg(test)] mod tests` (`src/inline_ignores.rs`, `src/checkers/inline_ignore.rs`, `src/check.rs`, `src/hook.rs`, `src/mcp.rs`, `src/config.rs`, `src/accepted_findings.rs`, `src/checkers/comment_quality.rs`), `assert_eq!`, `tempfile` dirs, `GrammarCache::new().parse` for tree-sitter fixtures. Type names follow the plan glossary (`Line`, `Directive`, `DirectiveParse`, `MalformedReason`, `RuleId`, `Reason`, `InlineIgnoreContext`).
- **Integration**: real checkers via `crate::checker::run_checker_configured` (in-crate), plus binary-level CLI tests in `tests/inline_ignore_cli.rs` (new, same style as `tests/false_positives_cli.rs` and `tests/hook_contract.rs`) spawning `kibitzer run`, `kibitzer hook` against temp repos.
- **E2E / UX**: manual checklist (Task 4.1.1d). `design/ux.md` is absent (research has `research/ux.md` only), so there is no UX Acceptance table; the user-facing surfaces (hook footer, MCP hint, CLI footer) are covered by the integration rows below.

Naming: `subject_should_Expected_When_Condition`.

## Requirement → Test Mapping

Requirement IDs are derived from `requirements.md` sections. `SM` = Success Metric, `CON` = Constraint, `NFR` = Non-functional, `IS` = In Scope, `RH` = Rabbit Hole, `OQ` = Open Question.

Note: requirements.md Success Metrics were rewritten as outcomes (re-surfacing rate, inline vs `accepted/` share, fp-marker triage). The original capability checks (SM-1 to SM-4 below) remain as acceptance tests; the outcome metrics are measured by the manual rows at the end of the mapping. **Round 3**: the baseline replay moved from the end of the plan to Phase 0 (a go/no-go gate before any implementation), so its rows are now the first manual rows; Phase 0 produces no `src/` code and therefore no unit tests.

| Requirement | Test File | Test Name | Type | Scenario |
|-------------|-----------|-----------|------|----------|
| SM-1: one comment dismisses a finding in hook/MCP/run/daemon/LSP | src/check.rs | run_checker_against_source_should_DropFinding_When_WholeLineIgnoreAbove | Unit | Happy path: ignore above line 10 drops `[flag-argument]` at 10, line 30 stays, `passed == false` |
| SM-1 | src/check.rs | run_checker_against_source_should_Pass_When_AllFindingsCovered | Unit | Happy path: ignores above both findings, `passed == true`, `combined` empty |
| SM-1 | src/check.rs | run_checker_against_source_should_KeepFinding_When_DirectiveTwoLinesAbove | Unit | Error path: ignore not adjacent, finding stays |
| SM-1 | src/check.rs | run_checker_against_source_should_KeepFinding_When_DirectiveMalformed | Unit | Error path: malformed ignore never suppresses |
| SM-1 | tests/inline_ignore_cli.rs | kibitzer_run_should_OmitFinding_When_IgnoreAbove | Integration | `kibitzer run <file>` prints no `[flag-argument]` line; removing the comment brings it back |
| SM-1 | tests/inline_ignore_cli.rs | kibitzer_hook_should_OmitFinding_When_IgnoreAbove | Integration | PostToolUse payload for a covered Go edit emits no `additionalContext` for that finding |
| SM-1 | src/mcp.rs | run_checks_should_OmitFinding_When_IgnoreAbove | Integration | MCP `run_checks` on a covered file reports 0 findings for the rule |
| SM-1 | src/check.rs | run_checks_for_trigger_should_DropFinding_When_IgnoreCoversChangedLine | Integration | Shared entry used by daemon and LSP (`run_checks_for_trigger`) applies the filter |
| SM-1 | src/check.rs | check_native_against_git_head_should_HonorIgnore_When_IgnorePresentAtHead | Integration | HEAD baseline in `Apply` mode treats a covered finding as suppressed, not pre-existing noise |
| SM-2: suppression survives edits elsewhere | src/check.rs | ignore_should_SurviveEdits_When_LinesInsertedAbove | Unit | Happy path: 5 lines inserted at line 1, ignore now row 15, finding line 16, still suppressed |
| SM-2 | src/check.rs | ignore_should_StopSuppressing_When_FindingMovesAwayFromComment | Unit | Error path: a blank line inserted between ignore and statement re-surfaces the finding |
| SM-2 | tests/inline_ignore_cli.rs | kibitzer_run_should_StaySuppressed_When_FileEditedElsewhere | Integration | Run before and after prepending a license header; both runs silent |
| SM-4: zero regressions in `.kibitzer/accepted/` | src/accepted_findings.rs | existing `accepted_findings` test module (all tests) | Unit | `cargo test accepted` passes unchanged; `extract_rule` (:105) not modified |
| SM-4 | src/check.rs | accepted_entry_should_StillSuppress_When_NoInlineDirective | Unit | Happy path: `accepted/` entry alone drops the finding, as before |
| SM-4 | src/check.rs | accepted_entry_should_NotBeAffected_When_InlineDisabled | Unit | Error path: `mode: Disabled` leaves accepted filtering identical |
| SM-4 | src/accepted_findings.rs | accepted_findings_should_DeserializeUnchanged_When_InlineFieldSkipped | Unit | `#[serde(skip)]` `inline` field does not change JSON load of existing files |
| SM-4 | tests/inline_ignore_cli.rs | kibitzer_run_should_ApplyBothSuppressors_When_InlineAndAcceptedMatchSameFinding | Integration | Both suppress independently; output clean |
| CON-1: works for every supported comment syntax | src/inline_ignores.rs | scan_code_comments_should_ParseDirective_When_EachLanguageAllGrammar (table: Go, TS, TSX, JS, Python, Java, Kotlin, Rust; one fixture per `Language::ALL`) | Unit | Happy path per grammar |
| CON-1 | src/inline_ignores.rs | scan_markdown_should_ParseDirective_When_HtmlCommentOutsideFence | Unit | `<!-- kibitzer:ignore em-dash-overuse -- quoted source -->` at row 5 |
| CON-1 | src/inline_ignores.rs | scan_leading_comments_should_ParseDirective_When_ShellHashComment | Unit | `.sh` `# kibitzer:ignore file-size -- vendored` |
| CON-1 | src/inline_ignores.rs | scan_leading_comments_should_ReturnNothing_When_MarkerInsideEchoString | Unit | Error path: `echo "# kibitzer:ignore a -- b"` yields zero |
| CON-1 | src/inline_ignores.rs | parse_comment_line_should_StripLeader_When_SlashDoc_Hash_BlockStar_HtmlOpen | Unit | `//`, `///`, `//!`, `#`, `/*`, `/**`, `*`, `<!--` and trailing `*/`, `-->` |
| CON-1 | src/inline_ignores.rs | scan_code_comments_should_ReportBlockCommentLineRow_When_DirectiveOnInnerLine | Unit | Java `/*\n * kibitzer:ignore long-method -- generated\n */` at line 10 gives `start_line == 11` |
| CON-1 | src/inline_ignores.rs | scan_code_comments_should_ConvertRowsToOneBased_When_TreeSitterRowSix | Unit | tree-sitter row 6 gives `Line(7)` |
| CON-1 | src/inline_ignores.rs | parse_comment_line_should_Tolerate_When_CrlfAndTabs | Unit | CRLF and tab separators |
| CON-2: reason is required | src/inline_ignores.rs | parse_comment_line_should_ReturnMalformedMissingReason_When_NoDashDash | Unit | Error path: `// kibitzer:ignore flag-argument` |
| CON-2 | src/inline_ignores.rs | parse_comment_line_should_ReturnMalformedMissingReason_When_ReasonEmptyAfterSeparator | Unit | Error path: `-- ` with only whitespace |
| CON-2 | src/inline_ignores.rs | reason_new_should_Reject_When_EmptyOrBlank | Unit | `Reason::new("  ")` errs (parse, don't validate) |
| CON-2 | src/inline_ignores.rs | parse_comment_line_should_ReturnMalformed_When_EmDashSeparator | Unit | Error path: em dash separator rejected (ASCII `--` only) |
| CON-2 | src/inline_ignores.rs | parse_comment_line_should_ReturnValid_When_ReasonPresent | Unit | Happy path: reason trimmed, shared across comma list |
| CON-3: native per-file checkers only | src/config.rs | default_checks_should_ScopeInlineIgnoreToGrammarExtensionsAndMd_When_Read | Unit | Globs are `Language::ALL` extensions plus `*.md`, not `**/*` |
| CON-3 | src/check.rs | run_command_check_should_NotApplyInlineIgnores_When_ShellOutCommandCheck | Unit | Error path: a `command` check's output is not filtered by a covering directive |
| CON-3 | src/check.rs | architecture_check_should_NotApplyInlineIgnores_When_WholeRepoCheck | Unit | Error path: architecture findings unaffected by an ignore comment |
| CON-3 | src/check.rs | kibitzer_check_native_should_BypassInlineIgnores_When_RunDirectly | Integration | `kibitzer check native <name> <file>` prints raw finding (corpus workflow), documented |
| CON-4: suppressions stay reviewable | src/inline_ignores.rs | apply_inline_ignores_should_KeepFinding_When_RuleIsMetRule | Unit | Error path: `kibitzer:ignore ignore-syntax -- x` cannot suppress `[ignore-syntax]` (also `unused-ignore`, `ignore-volume`) |
| CON-4 | src/checkers/inline_ignore.rs | inline_ignore_should_EmitVolumeFinding_When_FiveValidDirectives | Unit | Happy path: finding at 5th directive row (line 40), message `5 inline ignores in this file tell the user you are silencing this many checks here, and either fix the code or ask the user whether a check is wrong` |
| CON-4 (UX r2 gap 6) | src/checkers/inline_ignore.rs | inline_ignore_should_EmitVolumeOncePerFile_When_SevenValidDirectives | Unit | Exactly one `[ignore-volume]`, at the 5th directive's row; none at the 6th or 7th |
| CON-4 | src/checkers/inline_ignore.rs | inline_ignore_should_EmitNothing_When_FourValidDirectives | Unit | Error path: below threshold, zero `[ignore-volume]` |
| CON-4 | src/checkers/inline_ignore.rs | inline_ignore_volume_should_SurviveDiffScoping_When_ChangedLinesIsFifthDirectiveRow | Integration | `changed_lines = Some(&[(40,40)])` keeps the volume finding |
| CON-4 (P1-2) | src/inline_ignores.rs | reason_new_should_Reject_When_OneWordOrEqualsRuleId | Unit | `-- needed` gives `WeakReason(TooShort)`; `-- flag-argument`, `-- Flag Argument`, and a two-word echo give `WeakReason(RuleEcho)`; none suppress; `-- legacy API, callers pinned` accepted |
| CON-4 (P1-2) | src/checkers/inline_ignore.rs | inline_ignore_should_RenderSeparateMessages_When_ReasonTooShortVersusRuleEcho | Unit | `-- needed` renders `reason 'needed' is too short to explain the code. Write: ... <the concrete constraint that makes this code acceptable>`; `-- flag-argument` renders `reason repeats the rule id instead of saying why ...`; the two strings differ, neither contains `at least two words`, both contain `concrete constraint`, no em dash |
| CON-4 (P1-2) | src/inline_ignores.rs | apply_inline_ignores_should_CountBlocking_When_BlockingCheckSuppressed | Unit | `SuppressionCounts` total/blocking split |
| CON-4 (P1-2) | tests/inline_ignore_cli.rs | kibitzer_run_should_PrintBlockingShareInFooter_When_BlockingFindingSuppressed | Integration | Footer `3 findings suppressed inline (1 from blocking checks) ...`; zero parts omitted; no split |
| CON-4 (P1-2) | src/inline_post_pass.rs | run_checks_for_trigger_should_EmitBlockingSuppressedAdvisory_When_AddedDirectiveSilencesBlocking | Integration | `.md` directive in `changed_lines` over a `markdown-link-integrity` finding yields `[blocking-suppressed] ...; tell the user you silenced a blocking check and why, so they can confirm it` advisory (text names the user, does not contain `confirm this is intended`), exit code unaffected |
| CON-4 (P1-2) | src/inline_post_pass.rs | run_checks_for_trigger_should_EmitNoBlockingAdvisory_When_DirectiveOutsideChangedLinesOrAdvisoryCheck | Unit | Error path |
| CON-4 (P1-2) | src/inline_ignores.rs | apply_inline_ignores_should_KeepFinding_When_RuleIsBlockingSuppressed | Unit | `blocking-suppressed` in `META_RULES` |
| CON-4 (UX r3 gap 7) | n/a | success ack rows removed | n/a | The one-line ack (old `hook_should_EmitOneLineAck_*`, `hook_ack_should_BeBounded_*`, `hook_should_EmitNoAck_*` tests) was DEFERRED as beyond the ask; the hook stays silent on a passing run, covered by `hook_should_EmitNoContext_When_RunPasses` |
| NFR-1: no measurable slowdown on hook path | src/inline_ignores.rs | apply_inline_ignores_should_UseNoScanOrHash_When_NoKibitzerSubstring | Unit | 5,000-line Go file, 1,000 calls: `scan_memo.scans == 0`, `hash_calls == 0`, no parser constructed (structural, CI-stable) |
| NFR-1 | n/a | ratio test removed | n/a | Wall-clock ratio tests (this row and the rerun-latency row) were removed after verify as flaky on loaded CI; the counter test above and the Phase 4 measurements in `docs/backtest-triage/inline-ignore-results.md` carry the evidence |
| NFR-1 (hook-path raw rerun) | src/check.rs | run_checks_for_trigger_should_PerformNoRawRerun_When_EditTouchesNoDirectiveRow | Integration | Rerun counter stays 0 |
| NFR-1 | src/inline_ignores.rs | apply_inline_ignores_should_NotScan_When_FindingsEmpty | Unit | Checker returned nothing for a file with a marker; scan counter stays 0 |
| NFR-1 | src/inline_ignores.rs | scan_memo_should_ScanOnce_When_ThirtyCheckersShareOneContext | Integration | One `InlineIgnoreContext`, marker file, 30 consecutive calls with findings; `scan_memo.scans == 1` |
| NFR-1 | src/inline_ignores.rs | scan_memo_should_Rescan_When_ContentHashChangesOrPathDiffers | Unit | Edited source rescans and no longer suppresses; single entry replaced, never grows |
| NFR-1 | src/inline_ignores.rs | scan_directives_should_ConstructNoParser_When_SourceLacksKibitzer | Unit | 10,000-line input, unused `GrammarCache` stays unused |
| IS-1: syntax (rule id, required reason, same/next line, optional marker) | src/inline_ignores.rs | parse_comment_line_should_ReturnIgnoreKind_When_KibitzerIgnore | Unit | Happy path: Go `// kibitzer:ignore flag-argument -- legacy API, callers pinned` at line 7 gives full `Directive` |
| IS-1 | src/inline_ignores.rs | parse_comment_line_should_ReturnAllRules_When_CommaList | Unit | `a,b -- why` yields two `RuleId`s, one shared reason |
| IS-1 | src/inline_ignores.rs | rule_id_new_should_Reject_When_PlaceholderOrUppercase | Unit | Error path: `<rule>` and `Foo_Bar` not valid `[a-z0-9-]+` |
| IS-1 | src/inline_ignores.rs | parse_comment_line_should_ReturnNotADirective_When_RustdocPlaceholder | Unit | `/// kibitzer:ignore <rule> -- <why>` is prose |
| IS-1 | src/inline_ignores.rs | scan_code_comments_should_ReturnNothing_When_MarkerInStringLiteral | Unit | Error path: `let s = "// kibitzer:ignore x -- y";` yields zero |
| IS-1 | src/inline_ignores.rs | scan_code_comments_should_RecordWholeLineFalse_When_CommentTrailsCode | Unit | `x := f() // kibitzer:ignore a -- b` gives `whole_line == false` |
| IS-1 | src/inline_ignores.rs | covers_should_BeTrue_When_FindingOnLineBelowWholeLineDirective | Unit | `end_line: 9`, finding at 10 true, at 11 false |
| IS-1 | src/inline_ignores.rs | covers_should_BeTrueOnlyOwnRow_When_TrailingComment | Unit | Trailing comment at 9 covers 9, not 10 |
| IS-2: filter in shared path used by all entry points | src/check.rs | run_checker_against_source_should_CallApplyInlineIgnores_When_BetweenCheckerAndFlatten | Unit | Seam applies before text flatten; `Disabled` returns raw (`mode: Disabled` keeps finding) |
| IS-2 | src/check.rs | run_checker_against_source_should_ReturnRawFindings_When_ModeDisabled | Unit | Error path: opt-out path |
| IS-2 | src/inline_ignores.rs | apply_inline_ignores_should_ReturnInputUnchanged_When_FindingsEmptyOrNoSubstring | Unit | Early returns |
| IS-2 | src/inline_ignores.rs | apply_inline_ignores_should_CountDropped_When_CounterPresent | Unit | Counter incremented by dropped count only |
| IS-2 | src/inline_ignores.rs | inline_ignore_context_should_KeepSeparateCounts_When_TwoRunsConcurrent | Integration | Two contexts cover 2 and 3 findings; counters read 2 and 3 |
| IS-2 | src/check.rs | check_native_against_git_head_should_NotChangeCounter_When_BaselineReplay | Integration | Counter stripped for HEAD replay |
| IS-2 | src/check.rs | run_checks_for_trigger_should_PickUpFilter_When_DaemonOrLspCall | Integration | Same filter on the daemon/LSP route |
| IS-3: malformed ignore reported, not a silent no-op | src/checkers/inline_ignore.rs | inline_ignore_should_Report_When_MissingReason | Unit | `8: [ignore-syntax] kibitzer:ignore flag-argument has no reason. Write: kibitzer:ignore flag-argument -- <why this is acceptable>` |
| IS-3 | src/checkers/inline_ignore.rs | inline_ignore_should_Report_When_MissingRule | Unit | `[ignore-syntax] kibitzer:ignore needs a rule id. Write: kibitzer:ignore <rule> -- <why>` |
| IS-3 | src/checkers/inline_ignore.rs | inline_ignore_should_SuggestRule_When_UnknownRuleNearKnown | Unit | `unknown rule 'flag-arg' - did you mean 'flag-argument'?` |
| IS-3 | src/inline_ignores.rs | parse_comment_line_should_ReturnEmDashSeparator_When_EmOrEnDash | Unit | U+2014 and U+2013 give `Malformed(EmDashSeparator)`, never `MissingReason` |
| IS-3 | src/checkers/inline_ignore.rs | inline_ignore_should_TellUseAsciiDoubleHyphen_When_EmDashSeparator | Unit | Message is `use ASCII '--' ... Write: kibitzer:ignore <rule> -- <why>`, ASCII only |
| IS-3 | src/inline_ignores.rs | parse_comment_line_should_HandleRuleListEdges_When_CommaVariants | Unit | `a,b` valid; `a,a` dedups; `a, b`, `a,,b`, `a,`, `,a` give `BadRuleList`; `a,<rule>` is `NotADirective` |
| IS-3 | src/checkers/inline_ignore.rs | inline_ignore_should_ReportEachUnknownElementAndStillSuppress_When_CommaListMixesKnownAndUnknown | Unit | `flag-argument,flag-arg`: finding dropped, one did-you-mean for `flag-arg` |
| IS-3 | src/checkers/inline_ignore.rs | inline_ignore_should_EmitNothingForUnknownRule_When_NoNearMatch | Unit | `made-up-rule` yields no `[ignore-syntax]` (surfaced by `[unused-ignore]` with the not-a-known-rule wording instead, rows below) |
| IS-3 | src/checkers/inline_ignore.rs | inline_ignore_findings_should_BeAdvisory_When_FileHasBlockingChecks | Unit | `[ignore-syntax]` never sets exit code 2 |
| IS-3 | src/checkers/inline_ignore.rs | inline_ignore_should_Report_When_NearMissMarker | Unit | `'kibitzer: ignore' not recognized; use 'kibitzer:ignore'` |
| IS-3 | src/checkers/inline_ignore.rs | inline_ignore_messages_should_ContainNoEmDash_When_AllReasonsRendered | Unit | `em-dash-overuse` is a default check |
| IS-3 | src/inline_ignores.rs | parse_comment_line_should_ReturnNotADirective_When_ProseMentionsKibitzer | Unit | `// see kibitzer: allow list in docs`, `// the kibitzer:ignore syntax is documented` produce no finding |
| IS-3 (UX r2 gap 1) | src/inline_ignores.rs | parse_comment_line_should_ReturnNotAtCommentStart_When_DirectiveFollowsOtherText | Unit | `// TODO kibitzer:ignore flag-argument -- legacy API, callers pinned` and `// legacy: kibitzer:ignore flag-argument -- legacy API, callers pinned` give `Malformed(NotAtCommentStart)`, never `NotADirective`, and do not suppress |
| IS-3 (UX r2 gap 1) | src/inline_ignores.rs | parse_comment_line_should_StayNotADirective_When_LaterMentionLacksFullGrammar | Unit | Error path (prose stays quiet): `// the kibitzer:ignore syntax is documented` (no rule list/`--`) and `// see kibitzer:ignore <rule> -- <why>` (placeholder) remain `NotADirective` |
| IS-3 (UX r2 gap 1) | src/checkers/inline_ignore.rs | inline_ignore_should_Report_When_DirectiveNotAtCommentStart | Unit | `8: [ignore-syntax] kibitzer:ignore must start the comment; it was found after other text and suppresses nothing. Write it as its own comment: kibitzer:ignore flag-argument -- <why>`; ASCII only; the original `[flag-argument]` finding still prints (integration: `kibitzer run` shows both lines) |
| IS-3 | tests/inline_ignore_cli.rs | kibitzer_run_should_PrintBothLines_When_MalformedIgnoreAboveRealFinding | Integration | `[ignore-syntax]` and original `[flag-argument]` both print |
| IS-3 | src/inline_ignores.rs | known_rules_should_CoverEveryStaticRulePrefix_When_DriftGuardScansCheckerSources | Unit | Each `"[<id>]` literal in `src/checkers/*.rs` and `src/single_call_site_delegation.rs` (outside `#[cfg(test)]`) is in `KNOWN_RULES` or a registered checker name |
| IS-3 | src/check.rs | apply_inline_ignores_should_Suppress_When_RuleAbsentFromKnownRules | Unit | `[some-new-rule]` finding with a directive naming it still drops (table is advisory) |
| IS-3 | src/check.rs | inline_ignore_should_YieldPassingEmptyResult_When_BinaryFile | Integration | PNG-like bytes `\x89PNG\r\n\x1a\n\xff\xfe`: passing, empty result |
| IS-3 | src/check.rs | inline_ignore_should_YieldPassingEmptyResult_When_InvalidUtf8GoFile | Integration | Invalid UTF-8 `.go` file |
| IS-3 | src/check.rs | other_checker_should_KeepFailedResult_When_ReadError | Unit | Error path: existing read-error semantics unchanged for non-`inline-ignore` checkers |
| IS-3 | tests/inline_ignore_cli.rs | kibitzer_run_should_PrintNoInlineIgnoreOutput_When_DirHasBinaryFiles | Integration | `kibitzer run <dir>` with a PNG and bad-UTF-8 file: no `inline-ignore` line, exit status unaffected |
| IS-5: docs updated | tests/inline_ignore_cli.rs | docs_should_ContainNoNoInlineStance_When_GrepRun | Integration | `rg "no inline\|no inline/per-line\|still no inline" docs/ CLAUDE.md` returns zero matches |
| IS-5 | tests/inline_ignore_cli.rs | docs_suppressing_checks_should_ContainGrammarAndAnchorTable_When_Read | Integration | Section lists grammar, both markers, same-or-above scope, per-checker anchor table, meta-rule exclusion |
| IS-5 (UX r2 gap 9) | tests/inline_ignore_cli.rs | docs_should_StateHookStaleIgnoreGap_When_Story222NotShipped | Integration | `docs/suppressing-checks.md` says the hook reports an unused ignore only inside the edited lines and that stale ignores elsewhere are not reported in the hook until Story 2.2.2 ships (or names `kibitzer run` as the audit if 2.2.2 shipped in the same PR) |
| IS-5 | tests/inline_ignore_cli.rs | docs_examples_should_NotSelfSuppress_When_KibitzerRunOnDocs | Integration | `kibitzer run docs/` emits no `[unused-ignore]` or `[ignore-syntax]` from fenced examples |
| IS-5 | src/hook.rs | hook_footer_should_CarrySyntaxWithLeader_When_FailedCheckOnGoPyMd | Unit | `// kibitzer:ignore <rule> -- <why>` for `.go`, `#` for `.py`, `<!-- ... -->` for `.md` |
| IS-5 | src/hook.rs | hook_footer_should_UseStructuredAnchor_When_SinglePrefixlessChecker | Unit | `primitive-obsession` (no `[rule]` prefix) and `markdown-link-integrity` produce the checker name as rule, from `CheckResult.inline.first_anchor()`, not parsed output |
| IS-5 | src/hook.rs | hook_footer_should_AnchorOnFirstShownFinding_When_EarlierFindingScopedOutOrAccepted | Unit | Finding at 20 outside `changed_lines` (or removed by `accepted/`) plus a shown finding at 5: the hint says `above line 5`, never 20 |
| IS-5 | src/hook.rs | hook_footer_should_StayWithinCharBudget_When_LongestCase | Unit | Whole footer <= 638 chars (`MAX_FOOTER_CHARS`, pinned as 423 baseline + 215 by `hook_footer_budget_should_BeBaselinePlus215_When_Pinned`), anchor present, `Rules:` list truncated (not dropped) at 6; doc links present while they fit |
| IS-5 (UX r2 gap 11) | src/hook.rs | hook_footer_should_DropReportingLinkFirstAndKeepAnchor_When_BudgetExceeded | Unit | With the budget artificially lowered: `reporting-false-positives.md` link is dropped first, then turn-off prose, then the `suppressing-checks.md` link; the syntax line and first-finding anchor are present at every step |
| IS-5 (UX r3 gap 2) | src/hook.rs | hook_footer_should_ListRuleIds_When_SecondFindingHasNoBracketPrefix | Unit | Results `flag-argument` (prefixed) then `primitive-obsession` (no prefix): footer contains `Rules: flag-argument, primitive-obsession` from `InlineOutcome::rule_ids()`; an older-binary result with empty `inline` omits the `Rules:` line and falls back to the generic `<rule>` form |
| IS-5 | src/hook.rs | hook_footer_example_should_ParseAsValidDirective_When_PlaceholdersFilled | Unit | For Go, Python, Markdown, Rust leaders and the generic form, the example round-trips through `parse_comment_line` to `Valid` |
| IS-5 | src/hook.rs | hook_should_EmitNoContext_When_RunPasses | Unit | Error path: passing run with no matched changed-line directive emits nothing (existing behavior kept; the only exception is the ack row under CON-4) |
| IS-5 | tests/hook_contract.rs | hook_blocking_stderr_should_MentionKibitzerIgnore_When_ExitCode2 | Integration | Blocking failure stderr includes `kibitzer:ignore` |
| IS-5 (UX r3 gap 3) | tests/hook_contract.rs | hook_blocking_stderr_should_IncludeInlineIgnoreRepairText_When_MalformedDirectiveAlongsideBlockingFinding | Integration | Blocking `markdown-link-integrity` finding plus a malformed `kibitzer:ignore` in the same file: exit 2, stderr contains the `[ignore-syntax] ... Write: ...` line; same for `[unused-ignore]` and `[blocking-suppressed]` results (every failing `inline-ignore` result is printed after the blocking ones) |
| IS-5 | src/mcp.rs | get_info_instructions_should_MentionKibitzerIgnore_When_Read | Unit | Updated `get_info_instructions_*` test |
| IS-5 | src/mcp.rs | run_checks_should_EndWithSyntaxHint_When_FindingsPresent | Unit | Last line is the one-line syntax with the file's leader |
| IS-5 | src/mcp.rs | run_checks_should_OmitSyntaxHint_When_ZeroFindings | Unit | Error path |
| IS-5 | src/inline_ignores.rs | comment_leader_should_MatchLanguage_When_GoPyMdAndFallback | Unit | Leader by `Language::for_path`, `.md`, fallback `#` |
| RH-1 (P1-1) | src/inline_ignores.rs | anchor_conformance_should_DropFinding_When_DirectiveOnReportedLineOrRowAbove (table over every checker in `default_checks()`) | Integration | Per checker: real checker fires on its `ANCHOR_FIXTURES` source; whole-line directive on row N-1 and same-row directive on row N each drop the finding |
| RH-1 (P1-1) | src/inline_ignores.rs | anchor_conformance_should_HaveFixtureOrExemption_When_DefaultCheckerAdded | Unit | Every `default_checks()` name has a fixture or an `ANCHOR_EXEMPT` entry with a reason; new checker without one fails the build |
| RH-1 (P1-1) | src/hook.rs | hook_hint_should_NameExactAnchorRow_When_FindingAtLine20 | Unit | `put this on its own line directly above line 20 (or at the end of line 20): // kibitzer:ignore flag-argument -- <why>` (no bare "line 19"); head-of-file wording for `file-size`/`file-complexity` |
| RH-1 (P1-1) | src/inline_post_pass.rs | run_checks_for_trigger_should_EmitUnusedIgnoreAdvisory_When_AddedDirectiveMatchesNothingInChangedLines | Integration | Directive at row 12 in `changed_lines`, finding at 20: advisory says `Move the comment to the line directly above line 20 (or the end of line 20)`, contains "Move" and not "place"/"line 19"; comment at 19: silent |
| RH-1 (P1-1) | src/inline_post_pass.rs | run_checks_for_trigger_should_SkipRawRerun_When_NoDirectiveRowInChangedLines | Unit | Rerun counter stays 0 |
| RH-1: finding line vs comment placement (multi-line constructs) | src/inline_ignores.rs | file_size_ignore_should_Suppress_When_DirectiveInFirstTenLines | Integration | Real `file-size` on a 900-line Go file, ignore on row 1, finding at 900 dropped |
| RH-1 | src/inline_ignores.rs | file_size_ignore_should_NotSuppress_When_DirectiveBelowRowTen | Integration | Error path |
| RH-1 | src/inline_ignores.rs | file_complexity_ignore_should_DropOnlyOneFinding_When_DirectiveAboveFirstFunction | Integration | 3 complex functions, ignore above the first: 2 remain; ignore on line 1: 0 remain |
| RH-1 | src/inline_ignores.rs | covers_should_BeTrue_When_FindingLineZeroNormalizedToOne | Unit | `Finding.line == 0` becomes `Line(1)`, directive on line 1 covers |
| RH-1 | src/inline_ignores.rs | duplicate_code_ignore_should_Suppress_When_AboveLastOccurrence | Integration | Duplicates at 5 and 40; ignore above 40 drops, above 5 keeps |
| RH-1 | src/inline_ignores.rs | over_commented_ignore_should_Suppress_When_AboveFirstLeadingCommentRow | Integration | Real `comment-quality-go`, finding at row 20 |
| RH-1 | src/inline_ignores.rs | commented_out_code_ignore_should_Suppress_When_AboveThatRow | Integration | Per-row anchor |
| RH-2: cross-file checks report two locations | src/inline_ignores.rs | duplicate_cross_file_ignore_should_CoverOnlyOwnFile_When_IgnoreInAGoOnly | Integration | `a.go` clean, `b.go` still reports |
| RH-3: comment-in-string false matches | src/inline_ignores.rs | scan_directives_should_NotListMarker_When_InRustStringAndMarkdownFence | Unit | String literal and fenced block both ignored (also indented code block) |
| RH-3 | tests/inline_ignore_cli.rs | kibitzer_run_should_NotSuppressOrReport_When_MarkerOnlyInStringLiteralsOfOwnRepo | Integration | Self-run on this repo: nothing suppressed or reported by string-literal markers |
| RH-4: Markdown comments and prose checks | src/inline_ignores.rs | markdown_link_integrity_ignore_should_Suppress_When_NamedByCheckerName | Integration | Real checker, `[foo] used but never defined`, `<!-- kibitzer:ignore markdown-link-integrity -- placeholder -->` above; also "defined but never used" and "file does not exist" variants |
| RH-4 | src/inline_ignores.rs | markdown_link_integrity_ignore_should_NotMatch_When_DirectiveNamesRefId | Unit | Error path: directive naming `foo` does not match |
| RH-4 | src/markdown_text.rs | prose_checks_should_BeIdentical_When_DirectiveHtmlCommentAdded | Unit | `repetitive-sentence-structure` and `missing-paragraph-break` findings equal with and without the comment |
| RH-5: checkers without a `[rule-id]` prefix | src/check.rs | apply_inline_ignores_should_MatchCheckerName_When_NoBracketPrefix | Unit | `primitive-obsession` finding dropped by directive naming `primitive-obsession` |
| RH-5 | src/inline_ignores.rs | rule_matches_should_BeFalse_When_DynamicPrefixCheckerAndBracketMatch | Unit | `DYNAMIC_PREFIX_CHECKERS` never match by `[x]` prefix |
| RH-6: precedence with `accepted/` and comment-quality interplay | src/check.rs | inline_should_RunBeforeAccepted_When_BothMatch | Unit | Order: inline then `drop_accepted_findings` |
| RH-6 | src/checkers/comment_quality.rs | comment_quality_should_EmitNothing_When_DirectiveCommentContainsCodeLikeText | Unit | `// kibitzer:ignore flag-argument -- see foo(bar) and x = y`: no `commented-out-code` / `verbose-comment` |
| RH-6 | src/checkers/comment_quality.rs | over_commented_should_NotCount_When_KibitzerCommentAddedAtThresholdMinusOne | Unit | No `[over-commented]` |
| RH-6 | src/checkers/comment_quality.rs | other_checkers_should_BeUnperturbed_When_ValidIgnoreAdded | Integration | `duplicate-code`, `syntax-rules-go`, `em-dash-overuse` findings identical modulo line shift |
| OQ-1 | src/inline_ignores.rs | parse_comment_line_should_ReturnNearMiss_When_SpaceAfterColonOrSynonym | Unit | `kibitzer: ignore`, `disable`, `allow`, `suppress`, `false_positive`, and `kibitzer:false-positive` (Amendment 1: not a directive, never suppresses) |
| OQ-2: same line, next line, or both | src/inline_ignores.rs | covers_should_AcceptSameRowAndRowBelow_When_WholeLineDirective | Unit | Both |
| OQ-2 | src/inline_ignores.rs | covers_should_RejectRowBelow_When_TrailingDirective | Unit | Trailing covers own row only |
| OQ-3: unused/stale ignores reported | src/inline_ignores.rs | unused_ignores_should_Report_When_NoMatchingFinding | Unit | `<file>:<row>: [unused-ignore] kibitzer:ignore flag-argument suppresses nothing - remove it` |
| OQ-3 | src/inline_ignores.rs | unused_ignores_should_BeEmpty_When_DirectiveCoversRawFinding | Unit | Used ignore not reported |
| OQ-3 | src/inline_ignores.rs | unused_ignores_should_BeEmpty_When_AlsoShadowedByAccepted | Unit | Judged against raw findings |
| OQ-3 (DEFERRED, Task 2.2.2b) | src/run.rs | unused_rerun_should_FindShadowedFinding_When_AcceptedEntryShadowsIt | Integration | Covering ignore plus matching `accepted/` entry: no `[unused-ignore]`; the rerun uses `raw_findings_for_check`, which never applies `accepted/` |
| OQ-3 (DEFERRED, Task 2.2.2b) | src/run.rs | unused_ran_checkers_should_DeriveFromFirstPassCheckNames_When_CheckSkipped | Unit | Check skipped by size/trigger/config is absent from `CheckResult.check_name` set, so its rules are not judged |
| OQ-3 | src/inline_ignores.rs | unused_ignores_should_SkipRule_When_OwningCheckerDisabledOrDidNotRun | Unit | Disabled in `inspect.json`, over `MAX_NATIVE_CHECK_BYTES`, or excluded by trigger |
| OQ-3 | src/inline_ignores.rs | unused_ignores_should_FailOpen_When_RuleOwnershipUnknown | Unit | Unknown ownership not judged |
| OQ-3 | src/inline_ignores.rs | unused_ignores_should_JudgeByCheckerName_When_MarkdownLinkIntegrity | Unit | Used `markdown-link-integrity` ignore not reported |
| OQ-3 (UX r2 gap 4) | src/inline_ignores.rs | unused_ignores_should_PointToCheckList_When_RuleIsNeitherKnownNorCheckerName | Unit | `made-up-rule` (no near match) yields `[unused-ignore] 'made-up-rule' is not a known rule or checker; run 'kibitzer check list' to see valid names`; text contains `kibitzer check list` and not `remove it`; a known rule with no finding still yields `suppresses nothing - remove it` |
| OQ-3 (UX r3 gap 5, Task 2.2.2e) | tests/inline_ignore_cli.rs | kibitzer_run_should_PointToCheckList_When_IgnoreNamesUnknownRule | Integration | End to end in `kibitzer run` with Task 2.2.2b NOT shipped: typo rule with no near match prints the not-a-known-rule line from first-pass data; a rule outside `KNOWN_RULES` that really suppressed a finding prints nothing |
| Cache hit keeps inline outcome (plan Task 1.2.2b1c) | src/cache.rs | cache_roundtrip_should_PreserveInlineOutcome_When_ResultHasFirstAnchorAndDropped | Unit | `put`/`save`/`load`/`get` returns the same `first_anchor` and `dropped` |
| Cache compat (Task 1.2.2b1c) | src/check.rs | check_result_should_DeserializeWithEmptyInline_When_CacheJsonLacksInlineKey | Unit | Old `cache.json` entry loads; `inline` is the empty outcome (same style as the `findings` back-compat test at `src/check.rs:3029`) |
| Cache hit hint (Task 1.2.2b1c) | src/daemon.rs | handle_run_checks_should_ReturnFirstAnchor_When_ResultServedFromCache | Integration | Second unscoped request is a cache hit and still carries `inline.first_anchor` |
| Cache version stamp (eng r3 gap 7, Task 1.2.2b1c) | src/cache.rs | cache_load_should_DiscardEntries_When_StoredKibitzerVersionDiffers | Unit | `cache.json` stamped `0.0.0`, and one with no version key, load as empty; one stamped with `CARGO_PKG_VERSION` loads intact; `save` stamps the current version |
| Shown-findings outcome (eng r3 gap 3, Task 1.2.2b1b) | src/check.rs | run_native_check_should_ExcludeScopedOutAndAcceptedFindings_When_ComputingShown | Unit | `changed_lines = Some(&[(5,5)])`, findings at 5 and 20, `accepted/` entry for a third: `inline.shown` has only the finding at 5; `inline.kept` still has the line-20 finding (pre-scoping) |
| Structured raw path (eng r3 gap 1, Task 2.2.3a) | src/check.rs | raw_findings_for_check_should_ReturnStructuredFindings_When_DirectiveWouldSuppress | Unit | Covered file: `raw_findings_for_check` returns the finding with rule resolved by `anchor_rule`; no text parsing involved (test feeds a checker whose message contains a misleading `[x]` in the middle) |
| Raw rerun unscoped (eng r3 gap 2, Task 2.2.3a) | src/inline_post_pass.rs | post_pass_should_NameRowTwenty_When_ChangedLinesIsRowTwelveOnly | Integration | Directive at 12, finding at 20, `changed_lines = Some(&[(12,12)])`: the advisory names row 20 while the first-pass `combined` lacks the row-20 finding (diff-scoping at `src/check.rs:473` hid it) |
| Post-pass seam (eng r3 gap 6, Task 2.2.0a) | src/inline_post_pass.rs | post_pass_should_ReturnNothing_When_ChangedLinesNoneOrNoKibitzerSubstring | Unit | One call in `run_checks_for_trigger`; empty result for `changed_lines == None` or a file without the substring; no file read in the second case beyond the single gated read |
| OQ-3 fallback (Task 2.2.3c) | src/inline_post_pass.rs | run_checks_for_trigger_should_EmitUnusedIgnore_When_UnownedRuleOnWrongRowAndSurvivingFindingExists | Integration | Rule absent from `KNOWN_RULES`, directive row 12, surviving finding row 20: advisory names row 20 |
| OQ-3 fallback | src/inline_post_pass.rs | run_checks_for_trigger_should_StaySilent_When_UnownedRuleDirectiveIsInDroppedList | Unit | Directive that matched (in `inline.dropped`) is used |
| OQ-3 fallback (documented limit) | src/inline_post_pass.rs | run_checks_for_trigger_should_PointToCheckList_When_UnownedRuleHasNoSurvivingFindingAndNoRanCheckMatch | Unit | Pins the limit: no evidence any checker owns the rule, so the not-a-known-rule advisory naming `kibitzer check list` (not silent, not `remove it`) |
| OQ-3 fallback | src/inline_post_pass.rs | run_checks_for_trigger_should_EmitUnusedIgnore_When_UnownedRuleEqualsRanCheckName | Unit | `suppresses nothing - remove it` (a real checker name, so removal is the right repair) |
| NFR-1 spike (Task 2.2.3d) | manual | hook-path raw rerun measurement right after 2.2.3a | Manual | Median of 5, absolute ms and ratio recorded; decision rule in the plan chooses rerun / narrowed rerun / first-pass-only before 2.2.3c and 2.2.3b are built |
| OQ-3 (Story 2.2.3, core) | src/inline_post_pass.rs | run_checks_for_trigger_should_EmitUnusedIgnoreWithNearestRow_When_AddedDirectiveOnWrongRow | Integration | Directive at row 12, finding at row 20, `changed_lines=[(12,12)]`: `12: [unused-ignore] ... the finding is at line 20. Move the comment to the line directly above line 20 (or the end of line 20)`; at row 19: silent |
| OQ-3 | src/inline_post_pass.rs | run_checks_for_trigger_should_EmitNoUnusedIgnore_When_ChangedLinesScoped | Integration | `changed_lines = Some(&[(1,3)])`, unused ignore at row 50, nothing in any result |
| OQ-3 (DEFERRED, Task 2.2.2b) | tests/inline_ignore_cli.rs | kibitzer_run_should_ReportUnusedIgnore_When_UnscopedRunAndNoFinding | Integration | End to end |
| Raw mode / backtest hygiene (SM-1, plan Story 2.2.1) | tests/inline_ignore_cli.rs | kibitzer_run_should_PrintCoveredFinding_When_NoInlineIgnoresFlag | Integration | Flag shows raw; default hides |
| Raw mode | tests/inline_ignore_cli.rs | kibitzer_run_should_PrintFooterWithCount_When_ThreeSuppressed | Integration | Last line `[kibitzer] 3 findings suppressed inline (...) (rerun with --no-inline-ignores to see them)`; absent at 0 (blocking and false-positive split asserted in the CON-4 P1-2 rows) |
| Raw mode (UX r3 gap 4) | tests/inline_ignore_cli.rs | kibitzer_run_should_PrintSyntaxHintOnce_When_AtLeastOneFindingReported | Integration | One `[kibitzer] to dismiss a finding you judged acceptable: <comment> kibitzer:ignore <rule> -- <why> ...` line when a finding is reported; absent on a clean run |
| Config | src/config.rs | language_extensions_should_BePubCrate_When_ConfigBuildsInlineIgnoreGlobs | Unit | `Language::extensions` visible to `config.rs`; globs built from it |
| Raw mode | src/check.rs | check_native_against_git_head_should_ReportPreexisting_When_NoInlineIgnoresAndIgnoreAtHead | Integration | Current and HEAD both raw |
| Raw mode | src/backtest.rs | backtest_should_ReturnRawFindings_When_FixtureHasCoveringIgnore | Integration | `run_checker_with_cache` bypasses the filter |
| Problem falsifiability, Phase 0 gate (requirements Success Metrics, plan Tasks 0.1.1-0.1.3) | manual | `scripts/resurface-baseline.py` over `kibitzer check backtest all --only-new` output, before any `src/` change | Manual | `R`, `F`, `A`, `G`, break-even `p*`, 20 hand-classified cases and the PROCEED / SHRINK / STOP decision recorded in the gate note; states token cost is inferred, not measured. Gate rule: `G < 2%` stop, `2% <= G < 5%` shrink, `G >= 5%` proceed |
| Marker-choice dry run (plan Task 0.1.2, assumptions B and C) | manual | fresh agent given the draft clause and the 20 classified cases | Manual | Agreement with the hand label at least 70% and class (c) at least 10% of addressable cases, else collapse to one marker before building |
| Footer net-token check (UX r3 gap 1; plan Tasks 0.1.1, 4.1.1c) | manual | break-even `p* = (E x c_footer) / (S x c_re)` with replay `E`, `S` and the measured footer length | Manual | `p*` recorded pre-build and re-computed with the final footer; above 0.5 means cut the footer to the generic line |
| Counterfactual coverage (plan Task 4.1.1d) | manual | synthetic directive inserted at the reported row of each reconstructable classified case | Manual | n of N dropped; bounds the achievable reduction |
| Post-ship outcomes (requirements Roadmap Fit, plan Task 4.1.1f) | manual | 30-day review checklist, owner tstapler, due release tag date + 30 days | Manual | Re-surfacing rate of dismissed findings only (fixed ones excluded) vs baseline, `[blocking-suppressed]` advisories per 100 directives reviewed, inline vs `accepted/` share, `list --inline` backlog vs fixtures, risky-assumption samples A/B/C; kill criteria applied | Pooled across repos/sessions; fewer than 20 directives or 30 dismissed findings at day 30 is INCONCLUSIVE (extend once to day 60), never a pass or a kill (plan Task 4.1.1f).
| Binary corpus (plan Story 4.1.1) | manual | `kibitzer check backtest inline-ignore` and corpus `kibitzer run` triage | Manual | Counts recorded in PR; zero false `[ignore-syntax]` / `[unused-ignore]`; zero read-error lines on corpus binaries |

## UX Acceptance Tests
Omitted: no `design/ux.md` exists. The user-facing surfaces (hook footer, MCP hint, CLI footer, error messages) are asserted in the mapping above. One manual end-to-end check remains (Task 4.1.1d): add a covering ignore to a fixture Go file, confirm `kibitzer hook` (PostToolUse payload), MCP `run_checks`, and `kibitzer run` all omit the finding; remove it and confirm it returns; a malformed ignore prints an actionable one-line fix with no dead end.

## Migration
N/A (plan: no schema or data changes; `.kibitzer/accepted/` format untouched). No `migration_should_be_reversible` test.

## Coverage Targets and How to Measure

| Stack | Coverage command | Target |
|---|---|---|
| Rust | `cargo tarpaulin --out Stdout` (scope with `--packages kibitzer`; also `cargo test` and `cargo clippy` as the ship gate) | ≥80% line on `src/inline_ignores.rs` and `src/checkers/inline_ignore.rs` |

- All public (`pub(crate)`) functions in `inline_ignores`: happy path + error path covered.
- All external integrations (hook, MCP, CLI, git HEAD baseline): unit-tested and at least one binary-level integration test.
- Per CLAUDE.md "Writing a new check": the `inline-ignore` checker is also backtested (`kibitzer check backtest inline-ignore`) and run against the public corpus (`docs/backtest-repos.md`) before landing.
