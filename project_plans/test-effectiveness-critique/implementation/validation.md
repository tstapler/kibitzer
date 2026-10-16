# Validation Plan: test-effectiveness-critique

**Date**: 2026-10-11
**Revision**: triad repair iteration 1 added cases for the UX envelope/mode/staleness contract, ingest hardening, execution safety, scale tests (parse memory, locator), the absolute hook budget, SM5 and the SM4 distinction. Iteration 2 adds cases for the non-tautological joint check, comment-only staleness, the machine contract (test ids, value kinds, stable warning ids, class E JSON, identical brief envelope), stratified `--top`, `--by-package`, `--show-suppressed`, four mode banners, the producer recipe contract, the hook quiet-path variants, parse reuse, the cap error text and the corrected `report_false_positive` behaviour; totals at the end are recounted by script. Iteration 3 adds cases for the four-format parse-scale test with lcov moved into Story 2.2.3, the run-sidecar schema and `--kill-completeness` (Story 2.2.9) with hand-written fixtures, the partition check (experiment-critical set builds and runs without lcov, junit, staleness, enrich or the recipe wrappers), the minimal R26 path, the Rust arm NoData-only rule, hex-object-id validation of commits, R10/R23/R24 precedence, per-class joint check with the zero non-claim, the `section` field and mode ordering, enum spelling, leave-one-out display, exit 1, malformed-directive feedback, the wrap rule, recipe and secret-scan scope, the PIT superset bias statement, and a per-story traceability index. Round 5 (engineering) adds 30 cases: the hidden experiment-input export and its parity and I0 rows, output hygiene for untrusted strings in text, json and brief, per-language instance registration and the hook cap home, `check list` state and the unknown-option error, the pre-G2 tier derivation, typed-value decoupling of experiment-critical adapter and exclusion stories, execution-safety ordering, G0 Part A0/A1 and B1/B2 ordering, and the N3 runtime, Python collector and Rust flakiness decisions.

## Happy Path Scenario
Given a repo checked out at commit `9f2c1ab` with a Python suite and a `coverage json --show-contexts` file (`ctx.json`, coverage.py with version and commit in `meta`) in which `test_parse_empty` covers only lines also covered by `test_parse`, when the owner runs `kibitzer tests report --coveragepy ctx.json`, then stdout shows a header with tool/version/commit/attribution, a mode banner on the first line, one listed row `path:line: [test-zero-unique-coverage]` with raw value `0 unique lines`, the overlapping test names (a flagged partner marked `*`), a confidence tier and cost, a joint coverage check line for the whole flagged set (measurement wording, no runtime total), a "Not assessed" line, the non-endorsement sentence, the "Before acting" block and a legend, and the exit status is 0. *(Static twin: an agent adds `def test_x(): parse("")` and hook mode prints one `[test-no-assertion]` finding of at most 4 lines.)*

## Requirement Index

Requirement IDs used in the mapping tables. SM = Success Metric, SC = In-Scope item, C = Constraint, NFR = Non-functional, OBS = Observability, RC = Risk Control, OOS = Out of Scope, Q = Open Question. Source: `requirements.md`.

| ID | Requirement (short) |
|---|---|
| SM1 | Literature deliverable: cited survey, every signal has >=1 primary citation and validation status |
| SM2 | Candidate-signal catalog classified by input, cost, FP risk, advisory/diff-aware fit |
| SM3 | Empirical corpus result: deletion delta (coverage/mutation) plus triaged precision (`backtest-triage.py`) |
| SM4 | Behavior change: >=1 shipped check/report, wired per the default-catalog rule (default-on only if no repo setup, else opt-in); distinguishes a shipped validated check from a descriptive report on a null result |
| SM5 | Adoption leading indicator: flagged-then-revised rate for agent-added tests, measurable after ship (not a gate) |
| SC1 | Inventory of tools and what each emits (machine-ingestable?) |
| SC2 | Integration design incl. how an LLM pass consumes the mechanical ranking |
| SC3 | Both owner-named signals implemented: incremental unique coverage and unique mutants killed |
| C1 | Advisory, ingest-only: states who runs tests and how results get in (ADR-001) |
| C2 | "Writing a new check" bar: transcript backtest plus corpus run; confirmed-occurrence bar or documented justification |
| C3 | Multi-language reality: per-language tooling differences, stated first-class set |
| C4 | No new heavy runtime dependency without justification |
| NFR1 | Static signals fit hook latency; dynamic signals never in hook path |
| NFR2 | 10^4-10^5 tests without per-test re-runs; mutation sampling/budget stated |
| NFR3 | Internal / no code leaves machine / no LLM call in the mechanical path |
| OBS1 | Each finding carries signal, raw value, baseline, tool/version |
| OBS2 | Corpus runs record flagged count, triaged precision, coverage/mutation delta |
| RC1 | Advisory-only, opt-in until precision accepted; dynamic gated behind explicit subcommand |
| RC2 | Rollback by config removal; check name registered per `docs/suppressing-checks.md` |
| OOS1 | No auto-delete/rewrite of tests; no own mutation engine; no test generation |
| Q1 | Dynamic signals ingest-only vs shell-out |
| Q2 | Best validated predictor of removability in the literature, and cost |
| Q3 | Unique-coverage vs unique-kills agreement (cheaper one suffices?) |
| Q4 | Which languages are first-class |
| Q5 | Static assertion-quality precision sufficient for `default_checks()` |
| Q6 | How "deleted tests" are validated (coverage delta, mutation delta, historical-bug re-detection) |

## Requirement -> Test Mapping

Types: Unit, Integration, Experiment (research validation: protocol check, pre-registration artifact, power gate, harness calibration, results-record check). Test files are Rust `#[cfg(test)]` modules unless a path under `scripts/` or `project_plans/` is named.

### A. Test seam and static checkers (Epic 0.2, Phase 1)

| Requirement | Test File | Test Name | Type | Scenario |
|---|---|---|---|---|
| SM4, NFR1 | `src/test_code/mod.rs` | `is_test_file_should_ReturnTrue_When_GoTestSuffix` | Unit | Happy path: `foo_test.go` true, `foo.go` false |
| SM4, NFR1 | `src/test_code/mod.rs` | `is_test_file_source_should_ReturnFalse_When_GeneratedHeader` | Unit | Error path: `// Code generated ... DO NOT EDIT` excluded |
| NFR1 | `src/test_code/mod.rs` | `test_functions_should_ReturnInlineRustTest_When_CfgTestModuleInLibRs` | Unit | Rust per-function detection with no test-file notion |
| SM4 | `src/test_code/mod.rs` | `assertion_calls_should_ReturnCalleeAndArgs_When_TestifyAssertEqual` | Unit | Happy path: one `AssertionCall{assert.Equal, 3}` |
| SM4 | `src/test_code/mod.rs` | `assertion_calls_should_ReturnEmpty_When_BodyHasNoAssertion` | Unit | Error path |
| SM4, C3 | `src/test_code/mod.rs` | `profile_should_Exist_When_LanguageIsGoPythonTsJsJavaKotlinRust` | Unit | Eight per-language profile instances (Go, Python, TypeScript, Tsx, JavaScript, Java, Kotlin, Rust) resolve; `.cs` yields `None` |
| SM4 | `src/test_code/mod.rs` | `assertion_vocabulary_should_IncludeConfiguredHelper_When_OptionSet` | Unit | `requireOK` recognised by checker and report from one key |
| SM4 | `src/test_code/mod.rs` | `assertion_vocabulary_should_ExcludeHelper_When_OptionAbsent` | Unit | Error path |
| SM4 | `src/checkers/test_quality_tests.rs` | `test_no_assertion_should_ReportZero_When_HelperAssertsViaTFatal` | Unit | FP fixture: helper assertion (Go) |
| SM4 | `src/checkers/test_quality_tests.rs` | `test_no_assertion_should_ReportZero_When_SnapshotFuzzSeedExampleOutputOrExpectedException` | Unit | FP fixture table, per language (P5/P10 modes) |
| SM4 | `src/checkers/test_quality_tests.rs` | `test_no_assertion_should_ReportOne_When_PythonBodyOnlyCallsParse` | Unit | Happy path: finding at line 1 anchored on `def` |
| SM4, Q5 | `src/checkers/test_quality_tests.rs` | `test_no_assertion_should_ReportAtFunctionLine_When_GoTestHasNoAssertion` | Unit | Evidence text names "no assertion call found in body or same-file helpers" |
| SM4, RC1 | `src/checkers/test_quality_tests.rs` | `test_no_assertion_message_should_ContainIgnoreLineAndNotStartWithDelete_When_Emitted` | Unit | ADR-004 wording |
| SM4 | `src/checkers/test_quality_tests.rs` | `test_no_assertion_should_DropFinding_When_ChangedRangeExcludesFunction` | Unit | Diff-scoped via `scope_output_to_changed_lines` |
| SM4, Q5 | `src/checkers/test_quality_tests.rs` | `test_no_assertion_should_ReportNothing_When_OnlyBodyAssertionRemovedFromExistingTest` | Unit | Documented limitation (gap to Story 4.1.2) |
| SM4 | `src/checkers/test_quality_tests.rs` | `test_no_assertion_should_SkipTest_When_SkipAnnotationOrGenerated` | Unit | Error path: `t.Skip`, `@Disabled`, `xit` |
| SM4 | `src/checkers/test_quality_tests.rs` | `test_tautology_should_Report_When_ExpectOneToBeOne` | Unit | Happy path (TS) |
| SM4 | `src/checkers/test_quality_tests.rs` | `test_tautology_should_NameOperand_When_GoAssertEqualGotGot` | Unit | Happy path (Go) |
| SM4 | `src/checkers/test_quality_tests.rs` | `test_tautology_should_ReportZero_When_ConstantComparedToComputedSide` | Unit | Error path: `assert.Equal(t, 4, Add(2, 2))` |
| SM4, Q5 | `src/checkers/test_quality_tests.rs` | `test_mock_only_should_Report_When_OnlyVerifyAfterCreate` | Unit | Happy path, "medium" text |
| SM4, Q5 | `src/checkers/test_quality_tests.rs` | `test_mock_only_should_ReportZero_When_StateAssertionAlsoPresent` | Unit | Error path |
| SM4, RC1, RC2 | `tests/test_quality_registry.rs` | `run_should_ReportNothing_When_NoInspectJsonAndAssertionFreeTest` | Integration | Default-off before G1 (runs real binary on a temp repo) |
| SM4, RC1 | `tests/test_quality_registry.rs` | `run_should_Report_When_InspectJsonEnablesTestNoAssertion` | Integration | Opt-in path |
| RC1, RC2 | `tests/test_quality_registry.rs` | `check_list_should_ListAllThreeTestQualityCheckers_When_DefaultConfig` | Integration | Reachable by name, `test-mock-only` available but not chained |
| RC2 | `src/config.rs` | `default_checks_should_NotContainTestQualityEntries_When_BeforeG1` | Unit | Guard: nothing chains `test_quality_checks()` |
| SM4, C2 | `tests/test_quality_registry.rs` | `run_should_ChainOnlyPromotedRule_When_G1ApprovesGoOnly` | Integration | Story 4.1.1: Go repo reports, Python repo does not |
| SM4 | `CLAUDE.md` / `docs/test-quality.md` | `catalog_doc_should_ListPromotedRules_When_G1Recorded` | Integration | Docs: grep `test-no-assertion` in default catalog paragraph |
| NFR1 | `src/checkers/test_quality_tests.rs` | `static_checkers_should_NotRegisterDynamicSignals_When_HookPath` | Unit | Hook path emits only `NoAssertion`/`Tautology`/`MockOnly` |
| NFR1 | `scripts/test-effectiveness/hook-latency.sh` (Story 1.2.3) | `hook_p95_should_NotExceedGoTableDrivenCandidate_When_1000SampledTestFiles` | Experiment | Latency NFR: p95 per file vs `go-table-driven-test-candidate`, recorded in FP doc appendix |
| NFR1 | `scripts/test-effectiveness/hook-latency.sh` (Story 1.2.3) | `hook_p95_should_StayWithinAbsoluteBudget_When_FullHookPathTimed` | Experiment | Absolute p95 ms on process start + diff scoping + all enabled rules + output, against the budget committed at G1 |
| NFR1 | `scripts/test-effectiveness/check-gates.sh` | `g1_budget_commit_should_PrecedeLatencyMeasurementAndVerdict_When_GitHistoryWalked` | Experiment | Pre-registration order; promotion stops without a budget commit |
| NFR1 | `src/checkers/test_quality_tests.rs` | `hook_mode_should_NotInvokeAnyIngestAdapter_When_RunChecks` | Unit | Dynamic code unreachable from hook/MCP |
| NFR1 | `src/test_code/mod.rs` | `parse_should_BeSharedAcrossThreeCheckersAndReport_When_OneFileProcessed` | Unit | Parse counter is 1 per file for the three checkers in one run, and equals the file count when the report locates and checks 1,000 files; `test_functions` never parses on its own (iteration 2, one-parse guard) |
| SM4, RC1 | `src/checkers/test_quality_tests.rs` | `hook_finding_should_CarryExactlyOneQuietPathLineOfRecordedVariant_When_AnyPromotedRule` | Unit | UX-H8: V1 copyable directive by default, V2 pointer when G1 records measurement (a) high; never zero lines, still <= 4 lines |
| C2 | `docs/check-ideas.md` check script | `backtest_record_should_ListHitsAndTenLabeledExamples_When_OnlyNewRun` | Experiment | Transcript backtest table present for each of three rules (or "fewer than 10 found") |
| C2 | `docs/check-ideas.md` check script | `occurrence_record_should_StateFoundOrNotFound_When_AssertionPaddingAndRemovalSearched` | Experiment | Gate for Story 4.1.2 recorded as a count |
| SM4, C2 | `src/checkers/test_assertion_removed.rs` | `assertion_removed_should_Report_When_EditDeletesAssertFromExistingTest` | Unit | Conditional story 4.1.2, happy path |
| SM4, C2 | `src/checkers/test_assertion_removed.rs` | `assertion_removed_should_ReportNothing_When_AssertionMovedToHelperInSameEdit` | Unit | Conditional story 4.1.2, error path |
| SM4 | `src/test_code/go.rs` | `snapshot_markers_should_IncludeCupaloyAndGoldie_When_GoProfileBuilt` | Unit | Task 0.2.1b owns the Go snapshot markers; Task 0.2.1d (Rust) touches no Go file, so the three language tasks run in parallel |
| SM4, NFR1 | `src/checker.rs` | `registry_should_ContainPerLanguageInstances_When_TestNoAssertionFactoryRegistered` | Unit | Round-5 G2: `test-no-assertion-go` ... `-rust` each report one `language()`; one `CheckerFactory` returns all; no instance parses on its own (parse counter stays 1) |
| RC2 | `src/inline_ignores/rules.rs` | `rules_should_OwnBareRuleIdByPrefix_When_PerLanguageInstancesRegistered` | Unit | `kibitzer:ignore test-no-assertion` is owned by `test-no-assertion-python` via `OwnerMatch::Prefix`; drift guard covers `test-tautology`, `test-mock-only`, `test-assertion-removed` |
| SM4, NFR1 | `src/check.rs` | `hook_cap_should_CollapseAcrossRulesAndInstances_When_CapExceededOnOneFile` | Unit | Round-5 G3: 2 + 2 + 1 findings from three rules, cap 3: three findings plus exactly one `+2 more` line; `kibitzer run` passes no cap |
| SM4 | `src/check.rs` | `hook_cap_should_NotCollapse_When_CapUnrecorded` | Unit | Error path: value unset in `G1-decision.md`, all findings emitted |
| SM4 | `tests/test_quality_registry.rs` | `check_list_should_ShowStateOffAndLanguage_When_BeforeG1` | Integration | Round-5 G11: `kibitzer check list` (`src/main.rs` `CheckCommand::List`) prints `state: off` for `test-no-assertion-go`; existing `syntax-rules-go` line keeps its old fields plus `state: on` |
| SM4 | `tests/test_quality_registry.rs` | `config_load_should_FailNamingKeyAndValidKeys_When_UnknownOptionKey` | Integration | Typo `assertion_helper` under `test-no-assertion-go` fails config load with the key and `assertion_helpers`; no check runs |

### B. Matrix, analyses, exclusions, locator (Epic 2.1)

| Requirement | Test File | Test Name | Type | Scenario |
|---|---|---|---|---|
| OBS1 | `src/test_effectiveness/matrix.rs` | `build_should_ReturnOk_When_ToolAndVersionPresent` | Unit | Happy path |
| OBS1 | `src/test_effectiveness/matrix.rs` | `build_should_ReturnMissingProvenance_When_VersionNone` | Unit | Error path |
| OBS1, Q3 | `src/test_effectiveness/matrix.rs` | `build_should_ReturnOkWithWarning_When_CommitNone` | Unit | Commit absent is a warning, not an error (UX R11) |
| OBS1, Q3 | `src/test_effectiveness/matrix.rs` | `finding_should_PrintEachDimensionsProvenance_When_CoveragePyAndPitMixed` | Unit | Per-dimension provenance, `InOrder` vs `Full` |
| NFR2 | `src/test_effectiveness/matrix.rs` | `unique_coverage_should_UsePostingListLengthOnly_When_UnitCoveredByOneTest` | Unit | Sparse inverted index, no dense grid |
| NFR2 | `src/test_effectiveness/matrix.rs` | `test_id_ordering_should_PlaceTestABeforeTestB_When_IteratedAcrossFiles` | Unit | Deterministic ordering |
| OBS1 | `src/test_effectiveness/matrix.rs` | `test_id_should_NotConvertFromMutantId_When_TypesDiffer` | Unit | Newtype safety (compile-fail doc test) |
| SC3 | `src/test_effectiveness/analysis.rs` | `unique_coverage_should_ReturnSetDifference_When_T1T2T3Example` | Unit | T1->{1}, T2->{}, T3->{4} |
| SC3 | `src/test_effectiveness/analysis.rs` | `unique_kills_should_ReturnMutantsKilledByOnlyThisTest_When_KillMatrixGiven` | Unit | Owner-named signal 2 |
| SC3 | `src/test_effectiveness/analysis.rs` | `unique_coverage_should_ReturnEmpty_When_NoTests` | Unit | Error/edge path: empty matrix |
| SC3, NFR2 | `src/test_effectiveness/analysis.rs` | `greedy_reduction_should_ReturnT1T3_When_RunTwice` | Unit | Deterministic, union-preserving |
| SC3, Q6 | `src/test_effectiveness/analysis.rs` | `joint_loss_should_ReturnUnit1_When_ABBothOnlyCoverOne` | Unit | Individually redundant, jointly necessary |
| SC3, Q6 | `src/test_effectiveness/analysis.rs` | `joint_safe_subset_should_ReturnAC_When_ABCFlaggedAndBRejected` | Unit | Report lists B as "redundant only with A" |
| SC3, Q6 | `src/test_effectiveness/analysis.rs` | `joint_safe_subset_should_RejectTest_When_MutantWouldLoseAllKillers` | Unit | Kill-dimension rule |
| SC3 | `src/test_effectiveness/analysis.rs` | `dominators_should_LabelRelativeToSuppliedSuite_When_M2SubsumesM1` | Unit | Subsumption, no equivalence claims |
| SC3 | `src/test_effectiveness/analysis.rs` | `greedy_reduction_should_PreserveUnion_When_RandomMatrices` | Unit | Property test over matrices from a seeded in-tree PRNG (std-only, ADR-005 iteration 2 amendment) |
| NFR2 | `src/test_effectiveness/analysis.rs` | `joint_safe_subset_should_BeSubsetOfIndividuallyFlagged_When_RandomMatrices` | Unit | Property test, seeded PRNG |
| NFR2 | `src/test_effectiveness/analysis.rs` | `joint_safe_subset_should_HaveZeroJointLoss_When_RandomMatrices` | Unit | The I0 invariant is asserted here and in the harness, never printed by the report (iteration 2, B1) |
| OBS1, C1 | `src/test_effectiveness/exclusions.rs` | `exclusion_should_MarkFlaky_When_UniqueCoverageDiffersAcrossTwoInputs` | Unit | Happy path |
| OBS1, C1 | `src/test_effectiveness/exclusions.rs` | `stability_should_PrintFooterNoteAndExcludeNothing_When_OnlyOneInput` | Unit | Footer "Stability not assessed: single run"; 212 of 212 assessed; no `single-run` reason; never labeled stable (consistency B1) |
| OBS1, C1 | `src/test_effectiveness/exclusions.rs` | `exclusion_reason_should_NotContainSingleRun_When_EnumInspected` | Unit | Compile-level guard: the variant set is `Flaky, Failing, Skipped, Generated, NoData, Stale, NoSource` |
| Q4, SM3 | `src/test_effectiveness/exclusions.rs` | `kind_should_CapAtLow_When_IntegrationOrSnapshotZeroUniqueCoverage` | Unit | UX-E7 core rule |
| Q6 | `src/test_effectiveness/exclusions.rs` | `candidates_should_BeOneTest_When_GoTestHas40SubtestsAndNoSubtestsFlag` | Unit | Table-driven rule |
| Q6 | `src/test_effectiveness/exclusions.rs` | `candidates_should_ExpandToSubtests_When_SubtestsFlagGiven` | Unit | Opt-in expansion |
| OBS1, C1 | `src/test_effectiveness/exclusions.rs` | `exclusion_should_MarkStale_When_TestFileChangedSinceInputCommit` | Unit | UX-E5 |
| OBS1, C1 | `src/test_effectiveness/exclusions.rs` | `exclusion_should_MarkNoSourceAndExitZero_When_14IdsResolveNowhere` | Unit | UX-E6 |
| OBS1 | `src/test_effectiveness/locate.rs` | `locate_should_ReturnLocated_When_CoveragePyContextMatchesTestFunction` | Unit | `tests.test_parse.test_empty` -> line 12 |
| OBS1 | `src/test_effectiveness/locate.rs` | `locate_should_ReturnAmbiguous_When_GoTestNameInTwoPackagesAndNoPackageEncoded` | Unit | Error path: excluded as `NoSource`, never guessed |
| OBS1 | `src/test_effectiveness/locate.rs` | `locate_should_ReturnNotFound_When_IdNamesNoFunction` | Unit | Error path |
| OBS1 | `src/test_effectiveness/locate.rs` | `go_profile_ids_should_BeDistinct_When_TwoPackagesEachHaveTestParse` | Unit | Package path in profile name |
| NFR2 | `src/test_effectiveness/scale_tests.rs` | `scale_should_FinishWithinBounds_When_100kTests1MUnits50MEntries` | Integration | `cargo test --release scale -- --ignored`; <=1.5 GB RSS, <=120 s; prints RSS and wall clock |
| NFR2, C1 | `src/test_effectiveness/scale_tests.rs` | `parse_scale_should_StayWithinMemoryBound_When_EachAdapterFormatAtTargetSize` | Integration | Task 2.1.5b, the four experiment-critical formats only (coverage.py JSON, Go profile directory, Stryker JSON, PIT XML; lcov moved to Story 2.2.3, `lcov_parse_scale_should_StayWithinMemoryBound_When_TracefileAtTargetSize` in section C) at the size giving 5e7 entries; prints RSS and wall clock per adapter; largest passing input recorded as the `--max-input-bytes` value |
| NFR2 | `src/test_effectiveness/scale_tests.rs` | `locator_scale_should_ResolveWithinBoundAndReadEachFileOnce_When_100kTestIds` | Integration | Task 2.1.5c: 10^5 ids over 10^4+ files; file reads <= file count |
| C4 | `src/test_effectiveness/scale_tests.rs` | `roaring_should_BeEvaluatedBehindSameApi_When_ScaleBoundFails` | Integration | Conditional: ADR-005 amendment recorded with measurement |
| C4 | `Cargo.toml` check (CI) | `cargo_tree_should_ShowOnlyQuickXmlAsNewDependency_When_PhaseTwoMerged` | Integration | Dependency-minimality guard (`cargo tree -e normal` diff) |
| OBS1, C1 | `src/test_effectiveness/staleness.rs` | `staleness_should_ExcludeTest_When_CoveredProductionFileChangedSinceCommit` | Unit | Happy path |
| OBS1, C1 | `src/test_effectiveness/staleness.rs` | `staleness_should_NotMark_When_NoCoveredFileChanged` | Unit | Negative path |
| OBS1, C1 | `src/test_effectiveness/staleness.rs` | `staleness_should_PrintNotAssessed_When_NotAGitRepository` | Unit | Error path: no failure |
| OBS1, C1 | `src/test_effectiveness/staleness.rs` | `finding_should_BeLowTier_When_InputCommitUnknown` | Unit | Unknown commit caps tier |
| C1, NFR3 | `tests/tests_report_cli.rs` | `staleness_should_NotExecuteRepoCode_When_DiffIntersectionComputed` | Integration | Only `git diff --name-only` is invoked (command-spy) |
| OBS1, C1 | `src/test_effectiveness/staleness.rs` | `staleness_should_ExcludeTestsAndWarnR23_When_UncommittedEditToTestFile` | Unit | Dirty tree at the input commit; exit 0 |
| OBS1, C1 | `src/test_effectiveness/staleness.rs` | `staleness_should_WarnR24AndUseTreeDifference_When_InputCommitNotAncestorOfHead` | Unit | Other-branch input; may over-exclude, warning says so |
| OBS1, C1 | `src/test_effectiveness/staleness.rs` | `staleness_should_PrintNoStalenessWarning_When_CleanTreeAtInputCommit` | Unit | None of R10, R23, R24, R25 |
| OBS1, C1 | `src/test_effectiveness/staleness.rs` | `staleness_should_NotMarkStale_When_OnlyKibitzerIgnoreCommentAdded` | Unit | UX-E14 / G5: dismissing a finding does not stale the file; R10 and R23 do not name it |
| OBS1, C1 | `src/test_effectiveness/staleness.rs` | `staleness_should_MarkStale_When_StatementAddedToTestFile` | Unit | Comment-stripped token stream differs |
| OBS1, C1 | `src/test_effectiveness/staleness.rs` | `staleness_should_CountAsChanged_When_NoGrammarOrOldContentUnreadable` | Unit | Conservative fallback |
| OBS1, C1 | `tests/tests_report_cli.rs` | `staleness_should_PassNoGitArgStartingWithDash_When_IdIsDashRf` | Integration | Task 2.3.5c: `--` before every path; the `git`-argument half moved here from Story 2.2.7 |
| NFR2, C1 | `src/test_effectiveness/scale_tests.rs` | `gocover_parse_scale_should_BoundPeakRssByMatrixNotDirectoryTotal_When_DirectoryOfProfilesAtTargetSize` | Integration | Task 2.1.5b: Go profile directory read one file at a time, 5e7 entries total |
| NFR2 | `src/test_effectiveness/adapters/guard.rs` | `built_in_ceiling_should_EqualSmallestOfFourPerFormatLargestPassing_When_ParseScaleRun` | Integration | One ceiling value safe for coveragepy, gocover, stryker and pit; lcov is not an input to it |
| SM4, C1 | `src/test_effectiveness/exclusions.rs` | `exclusion_should_MarkStale_When_InjectedChangedFilesContainsTestFile` | Unit | Round-5 G1: Story 2.1.3's stale case uses an injected `changed_files` set, no git and no `staleness.rs` (end-to-end form is `staleness_should_ExcludeTest_When_CoveredProductionFileChangedSinceCommit`) |

### C. Adapters (Epic 2.2)

| Requirement | Test File | Test Name | Type | Scenario |
|---|---|---|---|---|
| SC1, C3 | `adapters/coveragepy.rs` | `parse_should_GiveNoassertSameLinesAsPos_When_ThreeTestFixture` | Unit | Happy path; `test_neg` has unique line 3 |
| SC1, OBS1 | `adapters/coveragepy.rs` | `parse_should_ReturnMissingProvenance_When_MetaLacksVersion` | Unit | Error path |
| SC1, OBS1 | `adapters/coveragepy.rs` | `parse_should_ExcludeSetupLinesFromUnique_When_ModuleLevelImportAndSharedFixture` | Unit | Warning "unique coverage excludes N setup lines" |
| SC1, C3 | `adapters/coveragepy.rs` | `parse_should_ReturnNoTestContexts_When_DynamicContextUnset` | Unit | R6 error path |
| SC1, C3 | `adapters/gocover.rs` | `parse_should_MatchProfileBlocksAndSetIsolated_When_TwoProfileFixture` | Unit | Happy path |
| SC1, C1 | `adapters/gocover.rs` | `parse_should_Fail_When_ManifestMissing` | Unit | Error path |
| C1, NFR2 | `scripts/test-effectiveness/per-test-go-coverage.sh` | `recipe_should_WriteProfilePerTestAndManifest_When_RunInModule` | Integration | Profile name has import path; manifest has go version and commit |
| C1 | `scripts/test-effectiveness/per-test-go-coverage.sh` | `recipe_should_ExitNonZeroWithMessage_When_NoGoMod` | Integration | S8 error: "no go.mod in <dir>" |
| C1 | `scripts/test-effectiveness/per-test-go-coverage.sh` | `recipe_should_RecordFailingAndNotWriteManifest_When_InterruptedOrTestFails` | Integration | S8 error table |
| SC1 | `adapters/lcov.rs` | `parse_should_MapTestToFileLine_When_TnAndDaPresent` | Unit | Happy path |
| SC1 | `adapters/lcov.rs` | `parse_should_MarkNoData_When_AllTnEmpty` | Unit | R7: "per-test coverage not available from this input" |
| SC1, C1 | `adapters/lcov.rs` | `producers_should_BeRecordedEmpirically_When_C8NycLlvmCovRun` | Experiment | Which producers write non-empty `TN:`; recorded in docs, resolves plan open question |
| SC1, SC3 | `adapters/mutation_elements.rs` | `parse_should_AttributeUniqueKill_When_SingleKilledBy` | Unit | Happy path |
| SC1, SC3 | `adapters/mutation_elements.rs` | `parse_should_TagFirstKillerOnlyAndDowngrade_When_AllKilledBySingle` | Unit | R9, tier capped Low |
| SC1, SC3 | `adapters/mutation_elements.rs` | `parse_should_RejectShape_When_NotMutationTestingElements` | Unit | R4 error path |
| SC1, SC3 | `adapters/pit_xml.rs` | `parse_should_CreditBothKillers_When_KillingTestsAorB` | Unit | Happy path |
| SC1, Q3 | `adapters/pit_xml.rs` | `parse_should_TagCoveringTestsAsMutantSite_When_Parsed` | Unit | Never mixed with line coverage |
| SC1, SC3 | `adapters/pit_xml.rs` | `parse_should_WarnAndCapLow_When_FullMutationMatrixNotEnabled` | Unit | R8 / UX-E4 |
| SC1 | `adapters/junit.rs` | `parse_should_SetSkippedAndCost_When_SkippedTestcaseWithTime` | Unit | `TestSlow` skipped, 3.2 s |
| SC1 | `adapters/junit.rs` | `parse_should_Error_When_NotXml` | Unit | R3 error path |
| SC1, C3 | `tests/tests_report_cli.rs` | `adapter_sniff_should_SuggestMatchingFlag_When_StrykerReportPassedToCoveragepy` | Integration | R4 hint "Use --stryker" |
| SC1, C3 | `adapters/*.rs` fixtures | `fixtures_should_BeFromRealRuns_When_StrykerAndPitCommitted` | Experiment | Fixtures carry tool version and a provenance note (not hand-written) |
| C1, NFR2 | `src/test_effectiveness/adapters/*.rs` | `adapter_should_ExitTwoWithR26_When_InputOverMaxInputBytes` | Unit | Cap tested per covered adapter against the built-in ceiling of Task 2.1.5b; never skipped on master (iteration 2, S1) |
| C1, NFR2 | `src/test_effectiveness/adapters/guard.rs` | `cap_error_should_NameFileSizeAndCapAndOpenNoFileForReading_When_InputOverCap` | Unit | R26 text names file, size and cap; only `metadata` is called (oversize unreadable file fixture) |
| C1 | `scripts/test-effectiveness/check-gates.sh` | `hidden_variant_should_NotMerge_When_BuiltInCeilingNotRecorded` | Experiment | Task 2.3.1a precondition: Task 2.1.5b's measured value is in code |
| C1 | `adapters/lcov.rs`, `adapters/junit.rs` | `lcov_and_junit_should_CallSharedGuardAndSurviveMalformedFixtures_When_Parsed` | Unit | Stories 2.2.3 and 2.2.6 own their hardening (D3 split) |
| C1 | `src/test_effectiveness/adapters/junit.rs` | `parse_should_NotReadLocalFile_When_DoctypeDeclaresExternalEntity` | Unit | Entity-safe XML for JUnit as for PIT |
| SC1, SC3 | `adapters/mutation_elements.rs`, `adapters/pit_xml.rs` | `kill_dimension_should_BeBudgetLimitedAndLow_When_SidecarSaysBudgetExhausted` | Unit | R9 with mutant counts; `mutation_run.scope` is `budget_limited` |
| C1 | `scripts/test-effectiveness/mutation-run.sh` | `recipe_should_PrintNoticeBeforeProgressAndWriteSidecarOnBudgetStop_When_MutationWrapperRun` | Integration | Story 2.2.8: notice, `*.partial` on interrupt, `stopped at budget: ...`, `<output>.run.json` |
| C1 | `scripts/test-effectiveness/per-test-python-coverage.sh` | `recipe_should_WriteSidecarWithModeAndTestsTotal_When_Run` | Integration | Attribution read from the sidecar, no assumed-default warning |
| C1 | `src/test_effectiveness/adapters/coveragepy.rs` | `parse_should_ReturnClassEAndNotOverflowStack_When_JsonNestedBeyondRecursionLimit` | Unit | Recursion limit never disabled (grep guard on `disable_recursion_limit`) |
| C1 | `src/test_effectiveness/adapters/pit_xml.rs` | `parse_should_NotReadLocalFile_When_DoctypeDeclaresExternalEntity` | Unit | Entity-safe XML; asserted with an existing and a non-existing path |
| C1, NFR2 | `src/test_effectiveness/adapters/pit_xml.rs` | `parse_should_StayBoundedOrReject_When_BillionLaughsFixture` | Unit | Internal-entity expansion bomb |
| C1 | `src/test_effectiveness/adapters/*.rs` | `adapters_should_NeverPanicOrExitOtherThanZeroOrTwo_When_MalformedCorpus` | Unit | Truncated, empty, invalid UTF-8, BOM, duplicate keys, wrong types, out-of-range numbers, per format |
| C1 | `src/test_effectiveness/adapters/*.rs` | `adapters_should_NotPanicHangOrOverAllocate_When_FuzzOrPropertyRun` | Experiment | Owner-set case count per adapter; 10 s per-case limit |
| C1, OOS1 | `src/test_effectiveness/locate.rs`, `staleness.rs` | `locator_should_ReturnNoSourceAndOpenNothingOutsideRepo_When_IdIsPathTraversalOrLeadingDash` | Unit | `../../../etc/passwd::T`, `-rf::T`; no `git` argument starting with `-` |
| NFR2, SC1 | `src/test_effectiveness/scale_tests.rs` | `lcov_parse_scale_should_StayWithinMemoryBound_When_TracefileAtTargetSize` | Integration | Task 2.2.3b (ship-gated): moved out of Story 2.1.5; input over the ceiling is refused with R26 |
| C1, SC3 | `src/test_effectiveness/adapters/sidecar.rs` | `sidecar_should_SetBudgetLimitedAndCopyCounts_When_HandWrittenSidecarBudgetExhausted` | Unit | Story 2.2.9: no wrapper script involved; fixture under `testdata/test-effectiveness/sidecar/` |
| C1 | `src/test_effectiveness/adapters/sidecar.rs` | `sidecar_should_WarnAndCapLow_When_RequiredKeyMissing` | Unit | Missing required key is class W, dimension capped at `Low`; unknown keys ignored |
| C1 | `src/test_effectiveness/cli.rs` | `kill_completeness_flag_should_ResolveToLessCompleteValue_When_SidecarAndFlagDisagree` | Unit | Sidecar `full`, flag `sampled` gives `sampled` plus a warning naming both |
| C1, SC3 | `adapters/coveragepy.rs`, `adapters/gocover.rs` | `attribution_should_ComeFromHandWrittenSidecarOrManifest_When_NoWrapperScriptExists` | Unit | Experiment-critical adapters read the sidecar/manifest without any Story 2.2.8 artifact |
| C1, NFR2 | `scripts/test-effectiveness/check-partition.sh` | `experiment_critical_set_should_BuildAndRunItsTests_When_LcovJunitStalenessEnrichAndWrappersRemoved` | Integration | Task 2.2.9c: scratch copy with every `// ship-gated` module and the Story 2.2.8 scripts deleted; `cargo build` and the experiment-critical tests pass |
| SM3, OBS1 | `src/test_effectiveness/adapters/sidecar.rs` | `adapter_should_ReturnTypedMutationRunAndCapLow_When_SidecarBudgetExhausted` | Unit | Round-5 G1: Stories 2.2.4, 2.2.5, 2.2.9 assert the typed `MutationRun` and `cap: Low`; JSON `header.inputs[].mutation_run` is asserted in section D |
| NFR1 | `src/test_effectiveness/adapters/coveragepy.rs` | `adapter_should_ReturnTypedWarningIds_When_UnlistedVersionOrFixtureAttribution` | Unit | Round-5 G1: `unverified-tool-version` and `fixture-import-attribution` are typed ids with params; sentences are rendered and asserted by Task 2.3.1f |

### D. Report, CLI, suppression, brief (Epic 2.3)

| Requirement | Test File | Test Name | Type | Scenario |
|---|---|---|---|---|
| SM4, OBS1 | `tests/tests_report_cli.rs` | `report_json_should_ContainAllRequiredFields_When_CoveragepyInput` | Integration | Top level `schema_version,findings,not_assessed`; record `rule,test,value,baseline,attribution,tool,tool_version,commit,confidence,overlaps_with`; no per-record `not_assessed` |
| SM4 | `src/test_effectiveness/report.rs` | `rule_ids_should_EqualRegistryTable_When_AllSignalsEnumerated` | Unit | Every `Signal` maps to one string in the plan's Rule-id registry; drift guard and wireframe use the same strings (consistency C3) |
| SM4, SC3 | `tests/tests_report_cli.rs` | `report_should_EmitStaticRowForLocatedTest_When_NoAssertionCallFound` | Integration | Story 2.3.1: `test-no-assertion` row (value `static`) from `src/test_code/mod.rs`; present in json; none for `NoSource` tests (C4) |
| SM4 | `tests/tests_report_cli.rs` | `report_should_RankByConfidenceThenRuleThenId_And_IgnoreRuntime_When_TiersAndCostsDiffer` | Integration | Ranking key (triad B3): equal tier and rule, 0.20s and 0.01s: order follows test id, not runtime |
| OBS1 | `tests/tests_report_cli.rs` | `header_should_PrintAssumedAttributionAndWarn_When_NoAttributionFlag` | Integration | `attribution: in-order (assumed; pass --attribution to override)` plus W warning (C5) |
| OBS1 | `tests/tests_report_cli.rs` | `header_should_PrintIsolated_When_AttributionFlagIsolated` | Integration | Flag overrides default; no assumed warning |
| OBS1 | `src/test_effectiveness/tiers.rs` | `confidence_should_NotExceedStructuralCap_When_InOrderOrNoKillDimensionOrUnknownSchema` | Unit | Table-driven over the plan's "Confidence caps" (C6): in-order Medium, no-kill Medium, unknown schema version Low |
| OBS1, C1 | `tests/tests_report_cli.rs` | `adapter_should_WarnAndCapLow_When_ProducerVersionNotOnAllowList` | Integration | Pre-mortem #5: coveragepy, Stryker, PIT variants; exit 0, warning above table |
| OBS1, C1 | `adapters/*.rs` | `adapter_should_WarnPlausibility_When_UniqueCoverageShareOutsideFixtureRange` | Unit | Plausibility check on identical-coverage share and single-killer share |
| SM4 | `tests/tests_report_cli.rs` | `report_text_should_ShowRankedTableSummaryAndNotAssessed_When_ValidInput` | Integration | Happy path (anchor scenario) |
| SM4, RC1 | `tests/tests_report_cli.rs` | `report_should_Exit0_When_TestsFlagged` | Integration | Advisory only |
| SM4 | `tests/tests_report_cli.rs` | `report_should_Exit2WithUsageOnStderrAndEmptyStdout_When_NoInputFlag` | Integration | R1 error path |
| SM4 | `tests/tests_report_cli.rs` | `report_json_should_ParseOnStdoutWithWarningsArrayAndStderrCopy_When_WarningTriggered` | Integration | Stream contract: `warnings[]` in stdout JSON, one `warning:` line copy on stderr |
| SM4 | `tests/tests_report_cli.rs` | `report_text_should_PutWarningsInStdoutBodyAndNothingOnStderr_When_StdoutRedirected` | Integration | G2: `report > out.txt` keeps the warning above the table |
| OBS1, SM4 | `tests/tests_report_cli.rs` | `report_json_should_HaveFullEnvelopeInOrder_When_AnyInputIncludingZeroFindings` | Integration | `schema_version, format, mode, banner, advisory, non_claims, header, warnings, summary, findings, not_assessed, suppressed` (B1; iteration 2 adds `format` and `suppressed`) |
| OBS1, SM4 | `tests/tests_report_cli.rs` | `report_json_should_CarryStableWarningIdStructuredLowersAndParams_When_R10AndR14Trigger` | Integration | `id` is a slug (`stale-input-commit`, `attribution-in-order`), never a table row number; `lowers.signal` and `lowers.effect` are enums |
| OBS1 | `tests/tests_report_cli.rs` | `report_json_should_KeyFindingsByPathQualifiedTestId_When_SameTestNameInTwoPackages` | Integration | `test.id` is `<repo-relative path>::<qualified name>`; `(rule, test.id)` unique |
| OBS1 | `tests/tests_report_cli.rs` | `report_json_should_TagValueKind_When_StaticCoverageAndKillRulesPresent` | Integration | `value.kind` of `static`, `unique_coverage`, `unique_kills` with their fields |
| OBS1, RC1 | `tests/tests_report_cli.rs` | `report_should_PrintJsonErrorObjectOnStdoutAndExit2_When_ClassEWithFormatJson` | Integration | UX-E13: `{schema_version, format, error:{id,message,hint}}`; `error:` line on stderr; text mode keeps stdout empty |
| OBS1 | `tests/tests_report_cli.rs` | `report_json_should_CarryMutationRunAndKillCompleteness_When_SidecarGiven` | Integration | `header.inputs[].mutation_run.scope` and `kill_completeness` values |
| SM4 | `tests/tests_report_cli.rs` | `brief_should_CarryIdenticalEnvelopeAndFormatBrief_When_FormatBrief` | Integration | UX-A4, UX-R14: same envelope as json, `format: brief`, per-record extras only |
| SM4, RC1 | `tests/tests_report_cli.rs` | `report_should_PrintFlaggedPartnerStarAndOverlapsAlsoFlagged_When_PartnerIsFlagged` | Integration | UX-R16; json `overlaps_also_flagged` |
| SM4 | `tests/tests_report_cli.rs` | `report_should_NeverPrintRuntimeTotalOrSavingsWording_When_AnyFormat` | Unit | UX-R5, R17 scan: "estimated runtime", "savings", "safe to remove" |
| SM4 | `tests/tests_report_cli.rs` | `report_should_ListExperimentalSignalsInSeparateSectionTagged_When_ExperimentalFlag` | Integration | UX-E12; absent from main counts |
| SM4 | `tests/tests_report_cli.rs` | `report_should_PrintLegend_When_TextOutputHasRows` | Integration | UX-R18 |
| SM4 | `tests/tests_report_cli.rs` | `report_should_Show20RowsPlusOmittedLine_When_31Flagged` | Integration | `--top` default; omitted line names per-rule counts that sum to 11 |
| SM4 | `tests/tests_report_cli.rs` | `report_should_KeepDynamicRowsInWindow_When_40StaticRowsSortFirstAndTop20` | Integration | UX-R19: stratified window; every one of the 4 coverage rows visible |
| SM4 | `tests/tests_report_cli.rs` | `report_should_RollUpPerPackageAndCapRowsPerPackage_When_ByPackage` | Integration | UX-R20: roll-up table, default 5 rows per package, json `packages[]` complete |
| SM4, OBS1 | `tests/tests_report_cli.rs` | `report_scope_should_SumEveryReasonIncludingStaleAndNoSource_When_AnyInput` | Integration | UX-R21: assessed + not assessed = tests in input; breakdown parts sum to K |
| OBS1 | `tests/tests_report_cli.rs` | `report_should_NoteRepoTestsAbsentFromInput_When_PartialRun` | Integration | `note:` line and `header.scope.repo_tests_not_in_input`; outside assessed and not_assessed |
| SM4, RC1 | `tests/tests_report_cli.rs` | `joint_check_should_BeNonzeroAndOverIndividuallyFlagged_When_FlaggedTestsRedundantWithEachOther` | Integration | B1: A and B duplicates, each alone 0, joint 1; the set and its size are named; computed over the coverage-redundancy class only (round-3 B2) |
| SM4, RC1 | `tests/tests_report_cli.rs` | `report_should_NeverPrintJointSafeCountOrZeroByConstructionFigure_When_AnyMode` | Integration | The JointSafe subset appears in no output; I0 is harness-only |
| SM4 | `tests/tests_report_cli.rs` | `joint_check_should_ExcludeExperimentalRowsAndCountMembersWithoutCoverage_When_ExperimentalFlag` | Integration | Main-list set only; parenthesis counts coverage-data members |
| SM4 | `tests/tests_report_cli.rs` | `report_should_PrintBannerNamingBackingAnchorThatExists_When_ReportOnlyOrTiered` | Integration | UX-R13 / G11: banner text and `docs/test-effectiveness.md` anchor check |
| SM4 | `tests/tests_report_cli.rs` | `experimental_rule_should_KeepStructurallyCappedSignalsOutOfMainList_When_PreG2` | Integration | UX-E12 / G3: `FirstKillerOnly`, no full matrix, unknown schema, unknown commit go to the experimental section |
| SM4 | `tests/tests_report_cli.rs` | `report_static_rows_should_BeSubsetOfCheckNativeOutput_When_LocatedTests` | Integration | Parity with the harness's own static source |
| RC2 | `tests/tests_report_cli.rs` | `show_suppressed_should_ListReasonsAndMarkNonSpecific_When_FlagGiven` | Integration | UX-D8; json `suppressed[]` always present |
| SM4 | `tests/tests_report_cli.rs` | `report_should_PrintReadingNoticeOnStderrOnlyAndKeepStdoutIdentical_When_TtyStderr` | Integration | UX-E9; determinism preserved |
| SM4 | `tests/tests_report_cli.rs` | `report_should_ShowAllRows_When_TopZero` | Integration | Edge path |
| SM4 | `tests/tests_report_cli.rs` | `report_should_BeByteIdentical_When_SameInputRunTwice` | Integration | Determinism |
| RC1 | `tests/tests_report_cli.rs` | `help_should_HideTestsAndReportShouldPrintBanner_When_Experimental` | Integration | Hidden `Command::Tests`, first line "experimental: signals not yet validated" |
| RC1, SM4 | `tests/tests_report_cli.rs` | `help_should_ListTests_When_G2ShipsReportOnly` | Integration | Story 4.2.1c, unhide per G2 |
| RC2 | `src/main.rs` | `command_tests_should_BeRemoved_When_G2RecordsOwnerWrittenRemove` | Unit | Rollback path; only an explicit `remove` removes the variant |
| SM4, RC1 | `tests/tests_report_cli.rs` | `report_should_ShowDescriptiveBannerNoConfColumnAndTestIdOrder_When_G2DescriptiveOnly` | Integration | Null-result deliverable (pre-mortem #1): banner, no tier column, no ordering claim, no `JointSafe` wording, exit 0 |
| SM4, RC1 | `tests/tests_report_cli.rs` | `report_json_should_OmitConfidenceAndSetModeDescriptive_When_G2DescriptiveOnly` | Integration | `"mode": "descriptive"`, descriptive `banner`, no `confidence` key |
| SM4, RC1 | `tests/tests_report_cli.rs` | `report_text_should_HaveNoJointSafeSafeOrRedundantOnlyWith_When_DescriptiveMode` | Integration | B2: mode matrix, joint coverage check line still present |
| SM4, RC1 | `tests/tests_report_cli.rs` | `report_should_RenderEachModeColumnSetPerMatrix_When_FourModes` | Integration | Table test over experimental/descriptive/report_only/tiered against `design/ux.md` 3.2a |
| SM4, RC1 | `tests/tests_report_cli.rs` | `variant_should_RemainHiddenAndNotRemoved_When_G2DescriptiveOnlyNotYetUnhidden` | Integration | `descriptive-only` never deletes the variant |
| SM4, RC1 | `tests/tests_report_cli.rs` | `report_should_ListDescriptiveOnlySignalInMainListWithoutExperimentalFlag_When_G2DescriptiveOnlyAndUncapped` | Integration | Story 4.2.1 / `design/ux.md` section 3.1: rows are `section: main`, `signal_mode: descriptive`; `--experimental` holds only capped or verdict-less signals |
| SM4, RC1 | `tests/tests_report_cli.rs` | `report_banner_should_NeverContainHyphenatedReportOnly_When_AnyModeAndFormat` | Integration | Banner text and JSON `banner` use `report_only:`; the string `report-only` appears in no output (verdict name `ship-report-only` lives only in `G2-decision.md`) |
| SM4, OBS1 | `tests/tests_report_cli.rs` | `report_should_EscapeUntrustedStringsInErrorOutput_When_HiddenCommandRunsBeforeHygieneTask` | Integration | Task 2.3.1a AC: class E and R26 text paths escape input-derived strings (`input-unreadable` message with a control-character path) through the shared escape call, so no raw ESC or newline reaches stderr before Task 2.3.1j lands |
| C1, NFR2 | `scripts/test-effectiveness/check-partition.sh` | `partition_should_FindNoUseSiteOfShipGatedModules_When_ExperimentCriticalSourcesGrepped` | Integration | Task 2.2.9c: grep of `use` / `crate::` sites in experiment-critical modules for `adapters::lcov`, `adapters::junit`, `staleness`, `enrich` returns none outside `// ship-gated` modules |
| RC2 | `src/test_effectiveness/report.rs` | `footer_should_HintConfigOption_When_FiveDismissalsShareRuleAndReason` | Integration | UX D-E5; absent at four |
| RC2 | `tests/false_positives_cli.rs` | `false_positive_tool_should_AppendLocalIgnoreSentence_When_QueueWriteFails` | Integration | UX D-E4: existing `error queuing false-positive report:` text plus the appended sentence (the tool has no server) |
| RC2 | `tests/false_positives_cli.rs` | `false_positive_tool_should_KeepQueueAckAndAppendDoesNotSilence_When_RecordedOk` | Integration | UX-A5: existing reply text kept, one sentence appended; existing queue tests unchanged |
| RC2 | `tests/false_positives_cli.rs` | `false_positive_tool_should_AcceptReportRuleIdAsCheckName_When_Recorded` | Integration | `test-zero-unique-coverage` queued and listed by `kibitzer check false-positives list` |
| RC2 | `src/test_effectiveness/report.rs` | `summary_should_CountNonSpecificReasonsAndStillSuppress_When_StoplistRecorded` | Integration | UX D-E6, D7; off until stoplist recorded |
| SM4 | `tests/tests_report_cli.rs` | `brief_should_HaveNonEmptyNonClaimsVerifyFirstAndActionReview_When_NoOverlap` | Unit | Pre-mortem #10; constant `"action": "review"` |
| SC3, Q6 | `tests/tests_report_cli.rs` | `report_should_ListIndividuallyFlaggedWithRedundantOnlyWithColumn_When_ABDuplicate` | Integration | `JointSafe` headline, B "redundant only with A", no summed savings |
| SM4 | `tests/tests_report_cli.rs` | `report_should_PrintBeforeActingBlockWithIgnoreVsFalsePositiveSplit_When_RowsPresent` | Integration | UX-D5 |
| RC1, SM4 | `src/test_effectiveness/report.rs` | `report_should_NeverContainPercentRedundantOrVerdict_When_AnyInput` | Unit | UX-R5 |
| OBS1, RC2 | `src/test_effectiveness/report.rs` | `ignore_should_SuppressFindingAndCount_When_DirectiveAboveFunction` | Integration | "1 suppressed" |
| RC2, OBS1 | `src/test_effectiveness/report.rs` | `ignore_should_NotSuppress_When_DirectiveHasNoReason` | Integration | Error path |
| RC2 | `src/test_effectiveness/report.rs` | `finding_should_PrefixRuleIdInBrackets_When_TextPrinted` | Unit | `[test-zero-unique-coverage]` for `report_false_positive` keys |
| RC2 | `src/inline_ignores/rules.rs` | `rules_drift_guard_should_PassWithSignalIdsAsUnowned_When_CargoTest` | Unit | Drift guard |
| RC2 | `tests/tests_report_cli.rs` | `run_should_EmitNoUnusedIgnore_When_DirectiveForReportRuleAboveTest` | Integration | `kibitzer run` on same file |
| RC2 | `testdata/test-effectiveness/ignore-placement/` | `ignore_placement_should_MatchDocumentedOutcome_When_AboveDecoratorOrBetween` | Integration | Python/Java/Kotlin/Rust (D-E1); outcome measured not assumed |
| RC2 | `src/test_effectiveness/report.rs` | `ignore_should_PersistWhileAdjacent_When_UnrelatedTestsAddedOrRemoved` | Integration | UX-D2 restated semantics |
| RC2 | `src/test_effectiveness/report.rs` | `report_should_NeverEmitUnusedIgnoreForDynamicRules_When_FindingVanishes` | Unit | UX-D6 |
| RC2, C1 | `src/test_effectiveness/report.rs` | `nosource_test_should_NotBeFinding_When_IdUnanchored` | Unit | Decision 2.3.2a: no dismissal path |
| RC2 | `src/accepted_findings.rs` | `accepted_schema_should_RemainUnchanged_When_ReportShips` | Unit | Guard that no `(rule, test-id)` key was added |
| SC2, NFR3 | `tests/tests_report_cli.rs` | `brief_should_IncludeEvidenceNonClaimsVerifyFirst_When_FlaggedTest` | Integration | Adjudication brief contract |
| SC2, NFR3 | `tests/tests_report_cli.rs` | `brief_should_NeverContainImperativeDeletion_When_AnyRecord` | Unit | Wording lint |
| SC2, NFR3 | `docs/test-effectiveness.md` | `docs_should_StateKibitzerDoesNotCallLlm_When_AdjudicationSectionRead` | Integration | Exact sentence present |
| NFR3 | `tests/tests_report_cli.rs` | `report_should_MakeNoNetworkCalls_When_Run` | Integration | Run under network-denied sandbox, still exit 0 |
| SC2 | `src/test_effectiveness/enrich.rs` | `enrich_should_AddCallsInto_When_ResolvedRateAboveG0Threshold` | Unit | Conditional story 2.3.3 |
| SC2 | `docs/` status line check | `enrich_story_should_ReadNotBuiltWithRate_When_SpikeNoGo` | Experiment | "not built: resolved rate N%" cited from `linkage-spike.md` |
| Q4, C3 | `src/test_effectiveness/report.rs` | `header_should_StateLanguagesWithDynamicData_When_PythonAndGoInputs` | Unit | R15 |
| Q3, Q4 | `src/test_effectiveness/tiers.rs` | `ordering_header_should_SayUnvalidatedDefault_When_GoOrPythonCoverageOnly` | Unit | No extrapolation from JVM/JS |
| Q3 | `src/test_effectiveness/tiers.rs` | `ordering_header_should_CiteArms_When_Q3SupportedAndJavaCoverageOnly` | Unit | Happy path |
| Q3 | `src/test_effectiveness/tiers.rs` | `ordering_should_PreferKills_When_Q3NotSupportedAndKillDimensionPresent` | Unit | Negative-outcome path |
| SM3, SM4 | `src/test_effectiveness/tiers.rs` | `doc_table_should_NameConstantAndValueEquals_When_HighMinPrecisionChecked` | Unit | Doc checked against code constants |
| SM3 | `src/test_effectiveness/tiers.rs` | `tier_should_BeByClassAndInputQualityOnly_When_G2NotRecorded` | Unit | No numeric cut shipped before triage |
| SM3, Q3 | `src/test_effectiveness/tiers.rs` | `zero_unique_kills_should_NeverExceedLow_When_FullMatrixAbsentOrFirstKillerOnly` | Unit | UX-E4 invariant |
| RC1, C1 | `tests/tests_report_cli.rs` | `command_should_ExitTwoWithR26JsonErrorObject_When_OnlyTask231aMerged` | Integration | Minimal class E path in 2.3.1a: `{schema_version, format, error:{id:input-over-size-cap,...}}`, exit 2; Task 2.3.1c later adds the other messages without changing this one |
| OBS1, RC1 | `tests/tests_report_cli.rs` | `report_json_should_TagEachRecordSectionMainOrExperimental_When_ExperimentalFlag` | Integration | UX-R22/B1: `section` separates main from experimental rows; `signal_mode` is `experimental` on both before G2 |
| OBS1, RC1 | `tests/tests_report_cli.rs` | `report_mode_should_BeLeastValidatedSignalModeAmongMainRows_When_VerdictsMixed` | Integration | Order `experimental` < `descriptive` < `report_only` < `tiered`; experimental-section rows do not lower it |
| OBS1 | `tests/tests_report_cli.rs` | `report_json_should_UseSnakeCaseEnumsForHyphenatedFlags_When_InOrderAndFirstKiller` | Integration | `in-order` gives `in_order`, `first-killer` gives `first_killer_only`, `budget-limited` gives `budget_limited` |
| OBS1, RC1 | `tests/tests_report_cli.rs` | `report_json_should_NameCountListedAndOmitRedundantOnly_When_ReportOnlyMode` | Integration | `report_only` summary keys equal the `descriptive` ones; no `confidence` key |
| SM4, RC1 | `tests/tests_report_cli.rs` | `joint_check_should_ExcludeStaticRulesAndNotMixClasses_When_NoAssertionAndZeroUniqueCoverageBothFlagged` | Integration | 9 static plus 14 coverage findings give one entry over the 14 (B2) |
| SM4, RC1 | `tests/tests_report_cli.rs` | `joint_check_should_AttachNonClaimToZero_When_MeasuredValueIsZero` | Integration | Every flagged test is covered by a non-flagged test: the zero line is followed by the non-claim; json `zero_note` is non-null |
| SM4 | `tests/tests_report_cli.rs` | `joint_check_should_NameUnitKindAndSplitEntries_When_LineAndMutantSiteInputsMixed` | Integration | One entry per `(class, unit_kind)`; `unit_kind` present in json |
| SM4 | `src/test_effectiveness/report.rs` | `redundant_only_with_flagged_should_CountOnlyTestsWhoseEveryPartnerIsFlagged_When_OnePartnerUnflagged` | Unit | A test with one non-flagged partner is not counted |
| OBS1 | `tests/tests_report_cli.rs` | `report_json_should_HoldTestIdsInOverlapFields_When_PartnerInOtherPackage` | Integration | `overlaps_with` and `overlaps_also_flagged` are `<path>::<name>`; text appends the path only on a name clash |
| SM3, RC1 | `tests/tests_report_cli.rs` | `report_should_ExitTwoWithR3NamingFlag_When_LeaveOneOutFileMalformed` | Integration | No new R row; unmatched ids only under `--verbose` |
| RC1 | `tests/tests_report_cli.rs` | `report_should_ExitOneWithInternalError_When_StdoutClosed` | Integration | UX-E16; exit 1 never produced by a malformed input |
| SM4, RC1 | `tests/tests_report_cli.rs` | `report_should_ListFindingWithNoteAndCount_When_IgnoreDirectiveLacksReason` | Integration | UX-D9: not suppressed, `note:` line, `malformed directives: 1`, json `ignore_note` |
| SM4 | `tests/tests_report_cli.rs` | `report_should_WrapNonRowLinesAt100AndKeepBannerOnOneLine_When_LongWarning` | Rust golden | UX-X6: no token split |
| C1, NFR3 | `src/test_effectiveness/staleness.rs` | `input_commit_should_NeverReachGit_When_NotHexObjectId` | Unit | `--input-commit` of `--output=/x`, `HEAD~1; rm -rf` and an input-carried non-hex commit: no git process, R11 warning, exit 0 |
| C1 | `src/test_effectiveness/staleness.rs` | `staleness_should_CountFileInExactlyOneMessage_When_CommittedAndUncommittedChange` | Unit | R23 wins over R10; R10 and R24 never both appear |
| OBS1 | `tests/tests_report_cli.rs` | `warning_should_CarryAttributionInOrderIdWithAssumedParam_When_DefaultAttribution` | Integration | Stable id `attribution-in-order`, `params.assumed: true`, `attribution_assumed: true` |
| OOS1 | `src/test_effectiveness/report.rs` | `brief_record_should_UseRecordNonClaimsKey_When_FormatBrief` | Unit | Per-record key is `record_non_claims`; envelope `non_claims` unchanged |
| SM3, NFR2 | `tests/tests_report_cli.rs` | `export_should_ListFullUniverseAndFlaggedAndJointSafe_When_EmitFlagPassed` | Integration | Round-5 B1: 6-test fixture gives 6 `tests[]` with `package` and `kind`, 3 `individually_flagged`, 2 `joint_safe`, header with per-dimension provenance |
| SM3 | `tests/tests_report_cli.rs` | `export_should_EqualReportFindingsPerRule_When_ParityChecked` | Integration | Parity: export `individually_flagged` equals the main-section, unsuppressed `findings[]` per rule before `--top`; test-id sets identical |
| SM3 | `scripts/test-effectiveness/test_deletion_experiment.py` | `harness_should_FindEmptyJointLoss_When_I0RunsOverExportedPostingLists` | Experiment | Independent ~30-line I0 pass over the posting lists agrees with Rust `joint_safe`; a planted bad `joint_safe` fails with `harness invariant I0 violated` |
| RC1, OOS1 | `tests/tests_report_cli.rs` | `export_should_BeAbsentFromHelpAndAllFormats_When_FlagHidden` | Integration | `--help`, text, json and brief for the same run contain no `experiment`, `JointSafe` or `joint_safe`; stdout is unchanged by the flag |
| SM3 | `tests/tests_report_cli.rs` | `export_should_WriteNoFileAndExitTwo_When_NoInputFlag` | Integration | R1 error applies; no output file created |
| NFR3 | `tests/tests_report_cli.rs` | `output_should_EscapeControlBidiAndNewline_When_TestIdHostile` | Integration | Round-5 G4: id with newline, ESC and U+202E prints as one escaped line in text and json; stdout line count equals the ordinary-id run |
| NFR3 | `tests/tests_report_cli.rs` | `output_should_MarkTruncation_When_NameOver1024Chars` | Integration | 5,000-character name ends `[truncated 3976 chars]`, exit 0; names under the guard are printed in full |
| NFR3 | `tests/tests_report_cli.rs` | `header_should_ShowEscapeAsText_When_CommitHasTerminalEscape` | Integration | Input `commit` with an ANSI sequence is printed as visible escaped text |
| NFR3, OOS1 | `tests/tests_report_cli.rs` | `brief_should_CarryRecordNoticeAndSingleLineId_When_InjectionIdInput` | Integration | Brief record has constant `record_notice`, envelope `untrusted_fields`, single-line `test.id`; no line starts with the injected instruction |
| SM4, RC1 | `src/test_effectiveness/confidence.rs` | `confidence_should_ApplyCapsWithoutRaising_When_InOrderAndNoKillDimension` | Unit | Round-5 G7: base tier per class (template A4) then caps; `InOrder` gives `Medium`, unknown commit `Low`; never raised |
| RC1 | `src/test_effectiveness/confidence.rs` | `verdict_table_should_DefaultAllSignalsToExperimental_When_NoG2Decision` | Unit | `G2_VERDICTS` stub: every `signal_mode` is `experimental` pre-G2; Task 4.2.1c replaces the contents |
| RC1 | `tests/tests_report_cli.rs` | `adapter_error_should_ExitTwoWithInputUnreadable_When_NoSpecificMessage` | Integration | Generic class E fallback from Task 2.3.1a: any adapter `Err` is exit 2 with `{schema_version, format, error.id: input-unreadable}` before 2.3.1c exists |
| SM4 | `tests/tests_report_cli.rs` | `envelope_should_ListTwoWarningIds_When_AssumedAttributionAndUnlistedVersion` | Integration | Round-5 G1: envelope example uses `attribution-in-order` and `unverified-tool-version`, not R10 |

### E. Experiment: protocol, pre-registration, power gate, harness, results (Phase 0.3, 0.4, 3, gates)

These are the validation cases for the research-led epic. "Test File" is the script or artifact that performs the check; each is runnable as a CI-style check over committed artifacts, or a calibration run of the harness.

| Requirement | Test File | Test Name | Type | Scenario |
|---|---|---|---|---|
| SM1 | `scripts/test-effectiveness/check-lit.py` | `catalog_should_HaveCitationAndValidationStatusPerSignal_When_DocParsed` | Experiment | Every `Signal` row has >=1 primary link and one of validated/plausible/unvalidated |
| SM1 | `scripts/test-effectiveness/check-lit.py` | `anecdote_should_AppearOnlyUnderUnverifiedSection_When_DocSearched` | Experiment | "no public write-up located"; absent from every rationale |
| SM1 | `kibitzer check native markdown-link-integrity` | `docs_should_ReportNoLinkFindings_When_TestEffectivenessDocChecked` | Integration | Links resolve |
| SM1, Q2 | `scripts/test-effectiveness/check-lit.py` | `survey_should_CoverAllSixTopicAreas_When_HeadingsParsed` | Experiment | Effectiveness vs coverage, minimization, redundancy, mutation, test smells, LLM-as-judge |
| SM2 | `scripts/test-effectiveness/check-lit.py` | `catalog_should_HaveInputCostFpRiskAndAdvisoryFitColumnsFilled_When_DocParsed` | Experiment | No empty cell for any signal |
| SM2, Q2 | `scripts/test-effectiveness/check-lit.py` | `catalog_should_NotCiteUnverifiedSourceAsRationale_When_RationaleColumnScanned` | Experiment | UNVERIFIED sources excluded |
| SM1, Q2 | `docs/test-effectiveness.md` review checklist | `q2_answer_should_StateNoPerTestSignalValidatedAndCostOfBestLossyPredictor_When_ReadByOwner` | Experiment | Manual: Q2 answered with the Shi et al. scoped claim |
| SC1 | `scripts/test-effectiveness/check-lit.py` | `tool_inventory_should_StateMachineIngestableOrNotPerTool_When_DocParsed` | Experiment | Per-tool column present |
| C1, Q1 | `docs/adr/` check | `adr001_should_StateWhoRunsTestsAndHowResultsEnter_When_Read` | Experiment | Ingest-only decision recorded |
| C3, Q4 | `project_plans/.../experiment/mutation-cost-spike.md` | `spike_should_RecordVersionCountsWallClockAndTm_When_EachToolRunOnce` | Experiment | Per tool: gremlins, mutmut, cargo-mutants, PIT, Stryker |
| NFR2 | `scripts/test-effectiveness/test_cost_model.py` | `cost_model_should_EqualFormula_When_2PlusNnullsKmutTimesMsKilledTimesTm` | Experiment | Unit-style check on T = (2 + N x K) x M x t_m |
| NFR2, Q6 | `scripts/test-effectiveness/test_cost_model.py` | `sampling_should_ReduceMsAndScopeBeforeKmut_When_TExceedsCap` | Experiment | Order of reduction |
| NFR2, Q6 | `scripts/test-effectiveness/test_cost_model.py` | `kmut_should_NeverGoBelow19_When_CapForcesReduction` | Experiment | Floor for rank p 0.05 |
| NFR2 | `scripts/test-effectiveness/test_cost_model.py` | `leave_one_out_runs_should_NotExceedRmax_When_20000TestSuite` | Experiment | Sampling bound (min(200, 2%)) |
| NFR2, Q1 | `project_plans/.../experiment/cost-spike.md` | `spike_should_RecordProcessLaunchesCpuAndIsolatedVsInOrderDisagreement_When_GoAndPython` | Experiment | Attribution cost table |
| Q6 | `project_plans/.../experiment/proxy-spike.md` | `power_gate_should_ProjectCommitsAs96OverYield_When_YieldMeasured` | Experiment | 0.02 yield -> ~4,800 commits |
| Q6 | `project_plans/.../experiment/proxy-spike.md` | `power_gate_should_MarkP1ExploratoryAndFallToP2Headline_When_CommitsExceedBudget` | Experiment | Fallback tree; "Q6 answered same-family only" |
| Q6 | `project_plans/.../experiment/proxy-spike.md` | `spike_should_RecordWindowOfAtLeast200CommitsOrAll_When_ReplayTried` | Experiment | Event count, yield, commands recorded |
| Q6 | `project_plans/.../experiment/proxy-spike.md` | `memo_should_NameHeadlineProxyLabelAndFallbackPerArm_When_LastSectionRead` | Experiment | Per-arm headline |
| SM3, Q6 | `project_plans/.../decisions/G0-decision.md` | `g0_should_ExistAndRecordAllPreregisteredNumbers_When_Phase3Starts` | Experiment | Checklist: proxy per arm, exemplars/modules, MutationBudget, K, R_max, H1-H4 thresholds incl. H4 floor and margins, linkage threshold, three pre-registered parameter rules |
| SM3 | `scripts/test-effectiveness/check-gates.sh` | `phase3_story_should_Stop_When_G0DecisionFileAbsentOnMaster` | Experiment | Gate precondition enforcement |
| SM3, SM4 | `project_plans/.../decisions/G0-decision.md` | `g0_should_BeAuthoredFromTemplateWithEveryLineSignedOrEdited_When_PartAAndBRead` | Experiment | Template lines A1-A9, B1-B16 each carry an owner value; no `<owner fills>` marker remains; the agent does not author the file |
| SM3, SM4 | `scripts/test-effectiveness/check-gates.sh` | `requirements_should_ContainNoNumericSm3Sm4Threshold_And_PointToG0PartB_When_Grepped` | Experiment | Thresholds live in G0 Part B only |
| SM4 | `docs/test-effectiveness.md` | `sm4_status_should_StateValidatedCheckOrDescriptiveOnNullResult_When_G2Recorded` | Experiment | Informative negative is reported as research-track success, never as behavior change delivered |
| SM5 | `scripts/test-effectiveness/flagged-then-revised.py` | `metric_should_ReportDenominatorNumeratorAndFourComplementBuckets_When_RunOnTranscripts` | Experiment | Task 1.2.1c baseline; buckets are ignore-directive, removal with named covering test, removal without one, unchanged; labelled the natural strengthening rate; "denominator below 10: not estimable" when small |
| SM5 | `docs/check-ideas.md` check script | `sm5_baseline_should_RecordSessionCount_When_BacktestRun` | Experiment | Pre-ship baseline recorded; live figure read from hook log after G1 |
| OOS1, NFR3 | `project_plans/.../experiment/execution-safety.md` | `policy_should_RequireReadOnlyMountEnvAllowlistNetworkDenyTimeoutsAndPinnedCommits_When_Parsed` | Experiment | Story 0.3.5 policy checklist |
| OOS1, NFR3 | `scripts/test-effectiveness/preflight-sandbox.sh` | `preflight_should_FailNamingPath_When_SandboxExposesSshDirOrAgentSocket` | Experiment | Env allow-list and mount checks |
| OOS1 | `scripts/test-effectiveness/test_deletion_experiment.py` | `harness_should_RefuseToStart_When_IsolationModeNotRecordedForArm` | Experiment | Exit non-zero before executing repo code |
| OOS1 | `scripts/test-effectiveness/check-results.py` | `record_should_HaveIsolationNetworkAndEnvAllowlistHash_When_ArmComplete` | Experiment | Row schema |
| OOS1 | mutation arm check | `mutation_run_should_LeaveOriginalCheckoutClean_When_ThrowawayCopyUsed` | Experiment | Story 0.3.5: source-rewriting mutators ran on a throwaway copy; `git status --porcelain` empty in the original |
| NFR3 | `scripts/test-effectiveness/secret-scan.sh` | `commit_should_BeRefused_When_ResultsFileMatchesSecretPattern` | Experiment | Task 0.3.5b creates the hook and `secret-patterns.txt` (the repo has none); results artifacts scanned |
| NFR3 | `scripts/test-effectiveness/preflight-sandbox.sh` | `dependency_fetch_step_should_UseEnvAllowlistAndScratchHome_When_NetworkAllowed` | Experiment | Story 0.3.5 rule (3): install-time scripts run under the same allow-list |
| SM4, Q5 | `docs/check-ideas.md` check script | `phase1_should_StopOrProceedPerA9_When_ZeroTranscriptOccurrences` | Experiment | Story 1.2.1 zero-occurrence rule: `proceed-corpus-only` starts 1.2.2; `registry-only` holds 4.1.1 |
| SM3 | `scripts/test-effectiveness/test_deletion_experiment.py` | `harness_should_TakeStaticFlaggedSetFromCheckNative_When_ArmBuilt` | Experiment | Own static source; equals `kibitzer check native test-no-assertion-python` output (per-language instance) |
| SM3 | `project_plans/.../experiment/mutation-cost-spike.md` | `spike_should_RecordNotMeasuredAndCoverageDeltaVerdict_When_ToolInstallFails` | Experiment | Per-language task failure does not block the others |
| SM4 | `scripts/test-effectiveness/check-gates.sh` | `story_4_1_1_should_Stop_When_G1DecisionFileAbsent` | Experiment | G1 gate |
| SM4 | `scripts/test-effectiveness/check-gates.sh` | `stories_4_2_1_and_4_2_2_should_Stop_When_G2DecisionFileAbsent` | Experiment | G2 gate |
| SM3 | `scripts/test-effectiveness/check-protocol.py` | `protocol_should_EqualG0NumbersVerbatim_When_Compared` | Experiment | Thresholds, K, budgets match |
| SM3 | `scripts/test-effectiveness/check-protocol.py` | `protocol_should_CiteG0CommitHash_When_Read` | Experiment | Story 3.1.1 precondition |
| SM3 | `scripts/test-effectiveness/check-protocol.py` | `preregistration_tag_should_PrecedeFirstResultsCommit_When_GitHistoryWalked` | Experiment | Tag commit is ancestor of every `results/*.jsonl` first commit |
| SM3 | `scripts/test-effectiveness/check-protocol.py` | `threshold_edit_should_AppearUnderDeviationsWithOriginalValue_When_ProtocolDiffedAfterTag` | Experiment | No silent threshold change |
| SM3 | `scripts/test-effectiveness/check-protocol.py` | `protocol_should_StateFalsifierForEachOfH1ToH4_When_Parsed` | Experiment | Each hypothesis names its failing outcome |
| SM3 | `scripts/test-effectiveness/check-protocol.py` | `protocol_should_StatePowerSectionAndExploratoryLabelRule_When_Parsed` | Experiment | Events needed vs observed; Wilson n |
| SM3 | `scripts/test-effectiveness/check-results.py` | `results_table_should_LabelArmExploratory_When_EventsBelowThreshold` | Experiment | Power gate enforced in output |
| SM3 | `scripts/test-effectiveness/test_deletion_experiment.py` | `harness_should_UseSameSeededMutantSampleBeforeAndAfter_When_RunTwiceWithSeed7` | Experiment | Identical sampled ids; rerun set equals sampled killed |
| SM3 | `scripts/test-effectiveness/test_deletion_experiment.py` | `harness_should_NotRerunSurvivors_When_SuiteSubsetRun` | Experiment | Survivors cannot be killed by subset |
| SM3 | `scripts/test-effectiveness/test_deletion_experiment.py` | `harness_should_UseSkipListsAndLeaveSourceTreeUnmodified_When_Run` | Experiment | `git diff --quiet` after run |
| SM3 | `scripts/test-effectiveness/test_deletion_experiment.py` | `harness_should_EmitMutationNotRunLabel_When_ArmVerdictIsCoverageDeltaOnly` | Experiment | Verdict from 0.3.4 honoured |
| SM3, Q6 | `scripts/test-effectiveness/test_deletion_experiment.py` | `harness_should_RunBothJointSafeAndIndividuallyFlagged_When_Run` | Experiment | Secondary labelled "joint-loss effect" |
| SM3, Q6 | `scripts/test-effectiveness/test_nulls.py` | `nulls_should_BeSizeMatchedAndSeeded_When_N1ToN4Generated` | Experiment | Same size as flagged set; same seed -> same sets |
| SM3 | `scripts/test-effectiveness/test_nulls.py` | `n4_should_PreserveJointCoverage_When_Generated` | Experiment | Coverage-preserving null never loses a unit |
| SM3 | `scripts/test-effectiveness/test_nulls.py` | `rank_p_should_Equal1PlusRoverKPlus1_When_RnullsNotAboveFlagged` | Experiment | Formula; r=0, K=19 -> 0.05 |
| SM3 | `scripts/test-effectiveness/test_nulls.py` | `rank_p_should_ExceedAlpha_When_FlaggedSetIsItselfAnN4Draw` | Experiment | Negative control: calibration (no false win vs N4) |
| SM3, Q6 | `scripts/test-effectiveness/test_nulls.py` | `matrix_arms_should_UseK999OrMore_When_KillMatrixAvailable` | Experiment | Nulls from matrix, no reruns |
| SM3, Q6 | `scripts/test-effectiveness/heldout-proxy.py` (toy repo) | `proxy_should_DetectLossOfPlantedEssentialTest_When_KnownAnswerRepo` | Experiment | Known-answer calibration (Task 3.1.3b), sensitivity |
| SM3, Q6 | `scripts/test-effectiveness/heldout-proxy.py` (toy repo) | `proxy_should_ReportNoLoss_When_OnlyDuplicateTestDeleted` | Experiment | Known-answer, specificity |
| SM3, Q6 | `scripts/test-effectiveness/heldout-proxy.py` | `p2_should_SelectOnFamilyAAndEvaluateOnFamilyBDisjoint_When_Run` | Experiment | Groups disjoint, fixed pre-run, labelled "same-family" |
| SM3, Q6 | `scripts/test-effectiveness/heldout-proxy.py` | `p3_should_RecordUnavailableSingleTool_When_JavaPitOnly` | Experiment | "P3 unavailable: single tool" |
| SM3, Q6 | `scripts/test-effectiveness/heldout-proxy.py` | `p1_should_RecordEventCountAndWilsonInterval_When_HistoryReplayed` | Experiment | Interval reported |
| SM3 | `scripts/test-effectiveness/test_stats.py` | `wilson_lower_bound_should_MatchReferenceValues_When_KnownCounts` | Experiment | Used for H3, H4, tiers |
| SM3 | `scripts/test-effectiveness/test_stats.py` | `equivalence_should_Pass_When_90CiWithinMargins` | Experiment | H1 TOST-style rule on the held-out measure, incl. fail -> stated negative |
| SM3, Q6 | `scripts/test-effectiveness/test_deletion_experiment.py` | `harness_should_FailRunWithInvariantMessage_When_JointSafeShowsNonzeroSelectionDelta` | Experiment | I0: exit non-zero, no hypothesis row (pre-mortem #2) |
| SM3, Q6 | `scripts/test-effectiveness/check-results.py` | `row_should_BeInvariantCheckNotHypothesis_When_EvaluationDataEqualsSelectionData` | Experiment | `kind` classification; excluded from H1-H4 counts |
| SM3, Q6 | `scripts/test-effectiveness/check-results.py` | `results_should_LabelNoValidityEvidence_When_ArmIsCoverageDeltaOnly` | Experiment | `evidence: none`; never an H1 pass |
| SM3, Q6 | `scripts/test-effectiveness/check-results.py` | `results_text_should_UseValidatedOnlyOnHeldOutHypothesisRows_When_Linted` | Experiment | Lint of `docs/test-effectiveness.md` |
| SM3, Q6 | `scripts/test-effectiveness/check-results.py` | `record_should_HaveSelectionEvaluationLabelKindStability_When_SchemaValidated` | Experiment | Row schema fields |
| SM3, Q6 | `scripts/test-effectiveness/test_leave_one_out.py` | `loo_runs_should_NotExceedRmaxAndRecordSeed_When_20000TestSuite` | Experiment | Story 3.1.4 bound |
| SM3, Q6 | `scripts/test-effectiveness/test_leave_one_out.py` | `loo_should_ShowZeroIndividuallyAndSharedUnitsJointly_When_DuplicatePair` | Experiment | Known-answer: A and B duplicates |
| SM3 | `tests/tests_report_cli.rs` | `report_should_LabelLeaveOneOutOnShortlistOnly_When_LooFileSupplied` | Integration | Shortlisted findings print `leave-one-out: <n> units lost (measured)` and json `leave_one_out` is filled; others keep input attribution and a null field |
| SM3, SM4 | `project_plans/.../decisions/G0-decision.md` | `g0_should_ContainOutcomeTableBudgetStopLossAndOwnerAcceptances_When_PartBRead` | Experiment | Outcome table (5 rows), wall-clock budget, 1.5x stop-loss, SM3/SM4 acceptances (a)-(c) |
| SM3 | `project_plans/.../decisions/G0-decision.md` | `g0_part_a_should_PrecedeStories122And131_When_GitHistoryWalked` | Experiment | Part A commit is an ancestor of the first triage verdict and G1 file (consistency C7) |
| SM3 | `scripts/test-effectiveness/test_deletion_experiment.py` | `arm_should_RecordNotRunBudget_When_WallClockExceeds1Point5xProjection` | Experiment | Stop-loss (pre-mortem #3); later arms still run |
| SM3, SM4 | `docs/test-effectiveness.md` | `negative_result_section_should_ExistWithPerArmNumbersAndWithdrawnClaims_When_Read` | Experiment | `NegativeResultReport` always present; names outcome-table row |
| SM1 | `docs/test-effectiveness.md` | `inventory_table_should_HaveRowPerStackTool_And_FiveNonEmptyColumns_When_Parsed` | Experiment | Task 0.1.1d AC (consistency B2); extends `tool_inventory_should_StateMachineIngestableOrNotPerTool_When_DocParsed` |
| SM3, Q3 | `docs/test-effectiveness.md` | `q3_sentence_should_ReadNotAnsweredNEquals0_When_NoGroundTruthArm` | Experiment | Story 3.3.2 n < 2 branch (pre-mortem #4) |
| SM3, Q5 | `docs/backtest-triage/**` check | `triage_should_ReportOwnerAgreementOnBlind10Percent_When_JsonlCounted` | Experiment | Pre-mortem #9: min 30 per rule/arm, Wilson on confirmed labels |
| SM4, Q5 | `docs/check-ideas.md` check script | `goodhart_measurements_should_RecordIgnoreRateAndWeakAssertionShare_When_BacktestRun` | Experiment | Pre-mortem #7 (a) and (b); UX-A3 |
| Q5 | `src/checkers/test_quality_tests.rs` | `weak_oracle_fixtures_should_ReportZero_When_NotNoneOrLenGreaterThanZero` | Unit | Known false negatives documented, not widened |
| Q5 | `src/checkers/test_quality_tests.rs` | `no_assertion_should_ReportZero_When_UnresolvedCalleeMatchesAssertionPatternInPrecisionFirstMode` | Unit | Strict mode reports one; precision-first reports zero |
| SM3, Q3 | `scripts/test-effectiveness/test_stats.py` | `h3_should_UseDirectionalPrecisionNotJaccard_When_ZcovZkillComputed` | Experiment | Decision statistic |
| SM3 | `scripts/test-effectiveness/check-results.py` | `result_records_should_ValidateSchema_When_JsonlLoaded` | Experiment | `flagged_count`, `delta_*`, `null.N1..N4`, `rank_p.N2/N4`, `mutation_mode`, set name |
| OBS2 | `scripts/test-effectiveness/check-results.py` | `record_should_HaveFlaggedCountPrecisionAndDeltas_When_ArmComplete` | Experiment | Per-arm observability fields |
| SM3 | `scripts/test-effectiveness/check-results.py` | `python_arm_should_CountFixtureImportWarnings_When_Complete` | Experiment | Warnings recorded |
| SM3, C3 | `scripts/test-effectiveness/check-results.py` | `go_arm_should_RecordIsolatedVsInOrderChangedCount_When_Complete` | Experiment | Attribution disagreement |
| SM3, C3 | `scripts/test-effectiveness/check-results.py` | `rust_arm_should_StateAttributionNoData_When_Complete` | Experiment | Story 3.2.3 is NoData-only (round-3 E-B3); `Isolated` is rejected because its only ingest path is ship-gated lcov |
| SM3, Q3 | `scripts/test-effectiveness/check-results.py` | `jvm_and_ts_arms_should_RecordTruncationAndKillCompleteness_When_Ingested` | Experiment | Arm excluded from Q3 if `FirstKillerOnly` |
| SM3 | `scripts/test-effectiveness/check-results.py` | `flakiness_should_BeClassified_When_SecondInOrderRunPresentPerArm` | Experiment | Else arm records `stability: single_run`; nothing excluded |
| SM3, SM4 | `scripts/test-effectiveness/check-results.py` | `static_arm_should_RecordMutationAndCoverageDeltaAndUniqueCoverageShare_When_Complete` | Experiment | Story 3.2.6 |
| Q3 | `scripts/test-effectiveness/analyze_q3.py` | `q3_sentence_should_ReadNotSupported1Of2_When_OneArmBelowThreshold` | Experiment | Decision rule applied to data |
| Q3 | `scripts/test-effectiveness/analyze_q3.py` | `q3_scope_should_BeJvmAndJsOnly_When_ResultsGenerated` | Experiment | Scope sentence |
| Q3, Q4 | `scripts/test-effectiveness/check-results.py` | `results_text_should_LabelUnvalidatedDefault_When_GoOrPythonOrderingMentioned` | Experiment | Lint over `docs/test-effectiveness.md` results |
| Q3 | `scripts/test-effectiveness/analyze_q3.py` | `comparison_should_StateCoverageUnitKind_When_PitInput` | Experiment | `MutantSite` vs line |
| SM3, OBS2 | `scripts/test-effectiveness/check-results.py` | `corpus_table_should_ListEveryRequirementsRepoAsDynamicOrStaticOnly_When_Parsed` | Experiment | kubernetes, cassandra, servo, ripgrep, deno, vscode, stapler-squad |
| SM3 | `docs/backtest-triage/**` check | `triage_should_HaveAtLeast50VerdictsPerRulePerLanguageOrAll_When_JsonlCounted` | Experiment | Per rule per language |
| SM3, Q5 | `scripts/test-effectiveness/check-results.py` | `h4_floor_commit_should_PrecedeFirstTriageVerdict_When_GitHistoryWalked` | Experiment | Floor fixed before triage |
| SM3, Q5 | `scripts/test-effectiveness/check-results.py` | `g1_should_ListWilsonLowerBoundAndNPerRuleLanguage_When_Read` | Experiment | Promote only if bound >= floor |
| SM4, Q5 | `scripts/test-effectiveness/check-results.py` | `g1_should_RecordHookCapValueDerivedByG0Percentile_When_Read` | Experiment | Cap from distribution |
| SM3 | `docs/test-quality-false-positives.md` check | `fp_doc_should_ListModeExamplePermalinkAndMitigation_When_FalsePositivesTriaged` | Experiment | Per FP mode |
| SM3, Q3, Q4 | `docs/test-effectiveness.md` review checklist | `results_should_StateOutcomeOfH1ToH4WithNumbersAndArmNames_When_Read` | Experiment | No adjectives; limitations paragraph present |
| Q4 | `project_plans/.../decisions/G2-decision.md` | `g2_should_RecordPerSignalVerdictLanguageListAndQ3Outcome_When_Read` | Experiment | Q4 answered |
| SM4, RC1 | `project_plans/.../decisions/G2-decision.md` | `g2_should_DefaultToDescriptiveOnlyAndAllowRemoveOnlyWhenOwnerWritesIt_When_NoSignalValidated` | Experiment | Null-result tie-in (pre-mortem #1); rollback tie-in |
| SM3, SC2 | `project_plans/.../experiment/results/llm-pilot.jsonl` | `pilot_should_RecordDeltaForShortlistAndRandomSampleAndCostCu_When_Triggered` | Experiment | Optional, owner-triggered |
| OOS1 | `tests/tests_report_cli.rs` | `report_should_NotModifyAnyFile_When_Run` | Integration | Read-only: tree hash before == after |
| OOS1 | `tests/tests_report_cli.rs` | `report_should_PrintNoDeleteInstruction_When_AnyOutputFormat` | Unit | Wording across text/json/brief |
| OOS1, C4 | `Cargo.toml` / `src/` grep check | `crate_should_ContainNoMutationEngineOrTestRunner_When_Reviewed` | Integration | No process spawn of test/mutation tools from `src/test_effectiveness/` |
| NFR3 | `src/test_effectiveness/` grep check | `crate_should_HaveNoHttpClientImportInTestEffectivenessModule_When_Built` | Integration | Static guard on no LLM/network call |
| NFR3 | `project_plans/.../experiment/execution-safety.md` | `policy_should_StateShippedRecipesRunRepoCodeWithCallerEnvAndAreOutsideAllowlist_When_Read` | Experiment | Story 0.3.5 scope paragraph: the allow-list governs the harness; recipes print the notice and are not enforced |
| NFR3 | `scripts/test-effectiveness/secret-scan.sh` | `commit_should_BeRefused_When_TriageJsonlUnderDocsBacktestTriageMatchesSecretPattern` | Experiment | Scan scope includes committed `docs/backtest-triage/` JSONL, not only `experiment/results/` and logs |
| SM3, Q3 | `scripts/test-effectiveness/analyze_q3.py` | `results_text_should_StateCoveringTestsSupersetBias_When_PitComparisonReported` | Experiment | Story 3.3.2: sentence containing "superset" beside the JVM table; H3 on `MutantSite` flagged weakly informative |
| NFR3 | `project_plans/test-effectiveness-critique/experiment/` (git history check) | `memo_should_FollowPolicyCommits_When_FirstThirdPartyRunCommitted` | Experiment | Round-5 G5: commits of Tasks 0.3.5a and 0.3.5b are ancestors of the first memo from 0.2.2a, 0.3.1a, 0.3.2a or 0.3.4a-e, and the memo header names the isolation mode |
| NFR3 | `scripts/test-effectiveness/preflight-sandbox.sh` | `preflight_should_RunBeforeCandidateInstall_When_Task022aStarts` | Experiment | Task 0.2.2a refuses to start without a recorded A11 time-box and a passing preflight |
| SM3 | `scripts/test-effectiveness/check-gates.sh` | `g0_should_CommitA9ThresholdBeforeBacktestCount_When_HistoryWalked` | Experiment | Round-5 G12: the A0 commit (with the A9 threshold) is an ancestor of the Task 1.2.1a results commit; the threshold text names no count |
| SM3 | `scripts/test-effectiveness/check-gates.sh` | `g0_should_CommitPartB2BeforeArmRun_When_ScaleValuesRecorded` | Experiment | B11, B12 and B15 values are in the B2 commit, which is an ancestor of the first `results/*.jsonl` and follows Stories 2.1.5 and 2.3.1b |
| SM3 | `scripts/test-effectiveness/test_deletion_experiment.py` | `harness_should_RecordRuntimeSourceHarness_When_ExportCostAbsent` | Experiment | N3 runtime-matched null uses harness-recorded durations; row says `runtime_source: harness` (or `junit` when cost present) |
| NFR3 | `scripts/test-effectiveness/test_deletion_experiment.py` | `harness_should_PassIdsAsArgvNotShell_When_BuildingSkipList` | Experiment | A hostile test id with shell metacharacters is passed as an argv element or escaped selector; no shell string is built |
| SM3, C3 | `project_plans/test-effectiveness-critique/experiment/results/python.jsonl` | `python_arm_should_UseArmOwnedIsolatedCollector_When_Collected` | Experiment | `per-test-python-isolated.sh` (Task 3.2.1a) produced the isolated profiles; no dependency on ship-gated Task 2.2.8a |
| SM3, C3 | `project_plans/test-effectiveness-critique/experiment/results/rust.jsonl` | `rust_arm_should_ClassifyFlakyOnOutcomeFlipOrSuiteTotal_When_RepeatRun` | Experiment | Task 3.2.3c uses outcome flips and suite-level covered-line variance, not per-test unique coverage |

## UX Acceptance Tests

Surfaces are terminal text/JSON (no GUI), so "click" criteria are commands and edits. Tool is std-only: the binary spawned with `env!("CARGO_BIN_EXE_kibitzer")` and golden outputs compared against checked-in files (ADR-005 iteration 2 amendment; no `assert_cmd` or `insta`), or Manual. All 62 criteria from `design/ux.md` section 9 are listed (41 original, 13 added in triad repair iteration 1: R13-R18, E9-E12, D7, A4, A5, and 8 added in iteration 2: H8, R19-R21, E13-E15, D8). The former "plan gap" rows (UX-A1, A2, A3, S3, S6, S7, S8) now have acceptance criteria in `plan.md` (Phase 4 repair log); each row below cites the story that carries it.

| UX Criterion | Test File | Test Name | Tool | Steps |
|---|---|---|---|---|
| UX-H1 finding shape, <=4 lines | `src/checkers/test_quality_tests.rs` | `ux_h1_finding_should_HaveHeadlineFixNonClaimIgnore_When_AgentAddsEmptyTest` | Rust golden | Add `def test_x(): parse("")`; run hook; count lines <=4, fields present |
| UX-H2 no Delete/Remove/must/critical/urgent | `src/checkers/test_quality_tests.rs` | `ux_h2_message_should_ContainNoBannedWords_When_AllRulesEmit` | Rust lint | Regex over all rule messages and briefs |
| UX-H3 unchanged vacuous test silent | `src/checkers/test_quality_tests.rs` | `ux_h3_hook_should_BeSilent_When_DiffDoesNotTouchVacuousTest` | Rust golden | Diff touches another function |
| UX-H4 clean file silent | `tests/test_quality_registry.rs` | `ux_h4_hook_should_PrintNothing_When_NoIssues` | Rust golden | Clean test file; stdout empty, no "0 findings" |
| UX-H5 unsupported language silent | `tests/test_quality_registry.rs` | `ux_h5_hook_should_PrintNothingAndExitUnchanged_When_CsFile` | Rust golden | `.cs` with assertion-free test |
| UX-H6 ignore line copy-paste works | `tests/test_quality_registry.rs` | `ux_h6_copied_ignore_line_should_Suppress_When_WhyReplaced` | Rust golden | Paste printed directive over function; re-run; finding gone |
| UX-H7 cap collapse to one line | `src/checkers/test_quality_tests.rs` | `ux_h7_extra_findings_should_CollapseToOneMoreLine_When_NPlus3` | Rust golden | Uses G1 cap value N (cap logic present, value from G1) |
| UX-H8 quiet-path line V1 or V2, never absent | `src/checkers/test_quality_tests.rs` | `ux_h8_finding_should_HaveQuietPathLineInRecordedVariant_When_AnyRuleInHookOrRunChecks` | Rust golden | Both variants; `run_checks` returns the same text with the `+N more` line once per file |
| UX-R1 one command to ranked table | `tests/tests_report_cli.rs` | `ux_r1_report_should_ReachTable_When_OneCommandOneInput` | Rust golden | Single invocation |
| UX-R2 rows carry rule, value, tier, overlaps | `tests/tests_report_cli.rs` | `ux_r2_rows_should_ShowRuleValueTierOverlaps_When_DynamicSignal` | Rust golden | Column check |
| UX-R3 header always tool/version/commit/attribution | `tests/tests_report_cli.rs` | `ux_r3_header_should_AlwaysShowFourFields_When_AnyInput` | Rust golden | Includes "commit unknown" variant |
| UX-R4 Not assessed + non-endorsement sentence | `tests/tests_report_cli.rs` | `ux_r4_output_should_ShowNotAssessedZeroAndSentence_When_NothingExcluded` | Rust golden | Count 0 shown |
| UX-R5 no headline percent, verdict, runtime total or savings wording | `tests/tests_report_cli.rs` | `ux_r5_output_should_HaveNoPercentVerdictRuntimeTotalOrSavings_When_Flagged` | Rust lint | Regex over text, json, brief |
| UX-R6 exit 0 / exit 2 contract | `tests/tests_report_cli.rs` | `ux_r6_exit_status_should_MatchClass_When_ErrorAndWarningInputs` | Rust golden | E -> 2, W/flagged -> 0 |
| UX-R7 joint coverage check per rule class over `IndividuallyFlagged`, measurement wording, never the JointSafe zero, a zero carries its non-claim | `tests/tests_report_cli.rs` | `ux_r7_joint_line_should_EqualSetLossNotSum_When_ABDuplicate` | Rust golden | A and B each alone 0 unique units, joint 1; line names the set, its size and members with coverage data |
| UX-R8 omitted-rows line | `tests/tests_report_cli.rs` | `ux_r8_output_should_Show20PlusOmittedLine_When_31Flagged` | Rust golden | `... 11 more: <rule> <n> | ...` per-rule counts sum to 11 |
| UX-R9 byte-identical reruns | `tests/tests_report_cli.rs` | `ux_r9_output_should_BeIdentical_When_RunTwice` | Rust golden | `diff` of two runs |
| UX-R10 JSON on stdout only | `tests/tests_report_cli.rs` | `ux_r10_stdout_should_ParseAsJsonWithWarningsArrayAndStderrCopy_When_FormatJsonWithWarning` | Rust golden | `jq .` on stdout; warning in `warnings[]` and one `warning:` copy on stderr |
| UX-R11 80-col, no truncation | `tests/tests_report_cli.rs` | `ux_r11_rows_should_NotWrapUnder40AndNeverTruncateOver40_When_Rendered` | Rust golden | No `...` inside a name, path or line; json carries the same full name |
| UX-R12 each R1-R26 message and exit class | `tests/tests_report_cli.rs` | `ux_r12_message_should_MatchSpec_When_EachOfR1ToR26Triggered` | Rust table test (26 cases) | One fixture per R-row; assert text and exit |
| UX-R13 mode banner first line in all four modes; descriptive has no conf column, order claim, JointSafe | `tests/tests_report_cli.rs` | `ux_r13_first_line_should_BeBannerAndDescriptiveRowsHaveNoConf_When_ModeSet` | Rust golden | Four modes against the 3.2a matrix; `report_only` and `tiered` banners name an existing doc anchor |
| UX-R14 envelope fields on every json/brief output, `format` second, brief identical | `tests/tests_report_cli.rs` | `ux_r14_envelope_should_HaveAllFields_When_ZeroFindings` | Rust golden | Includes the zero-finding run and a brief run diffed against json envelope |
| UX-R15 `path:line: [rule] TestName` untruncated, line equals json `test.line` | `tests/tests_report_cli.rs` | `ux_r15_row_anchor_should_MatchJsonLineAndBeUntruncated_When_LongName` | Rust golden | Name over 40 chars |
| UX-R16 flagged partner marked `*` and keep-one sentence | `tests/tests_report_cli.rs` | `ux_r16_partner_should_BeStarredAndBeforeActingKeepOne_When_BothFlagged` | Rust golden | Text and json |
| UX-R17 no runtime ordering or total | `tests/tests_report_cli.rs` | `ux_r17_order_should_IgnoreCostAndNoTotalPrinted_When_CostsDiffer` | Rust golden | Equal tier, different cost |
| UX-R18 legend | `tests/tests_report_cli.rs` | `ux_r18_legend_should_DefineAloneCostStarInputsDisagreeStale_When_TextRows` | Rust golden | Footer block; descriptive legend also defines `*` |
| UX-R19 `--top` window stratified by rule | `tests/tests_report_cli.rs` | `ux_r19_window_should_HoldFloorNOverRRowsPerRule_When_StaticClassSortsFirst` | Rust golden | 40 static, 4 coverage rows, `--top 20` |
| UX-R20 `--by-package` roll-up and per-package cap | `tests/tests_report_cli.rs` | `ux_r20_by_package_should_PrintRollUpAndCapPerPackage_When_Flag` | Rust golden | Wireframe 3.2c; json `packages[]` |
| UX-R21 scope arithmetic and repo-absent tests | `tests/tests_report_cli.rs` | `ux_r21_scope_should_SumAllReasonsAndNoteAbsentRepoTests_When_PartialRunInput` | Rust golden | Stale and no-source in K; `note:` line |
| UX-E1 error names flag and one fix | `tests/tests_report_cli.rs` | `ux_e1_class_e_message_should_NameFlagAndFix_When_Triggered` | Rust lint | First line names flag/file; second line fix |
| UX-E2 warnings in stdout body above table, `warning:` prefix, name signal lowered | `tests/tests_report_cli.rs` | `ux_e2_warning_should_PrecedeTableHavePrefixAndNameSignal_When_ClassW` | Rust golden | Order, prefix and content; present with stdout captured alone |
| UX-E3 lcov without TN prints unavailable, no ranking | `tests/tests_report_cli.rs` | `ux_e3_report_should_ShowNoPerTestRanking_When_LcovHasNoTn` | Rust golden | Sentence present; no rows |
| UX-E4 PIT no matrix <= low | `tests/tests_report_cli.rs` | `ux_e4_zero_unique_kills_should_BeLow_When_PitWithoutFullMatrix` | Rust golden | Tier column |
| UX-E5 stale excluded and counted | `tests/tests_report_cli.rs` | `ux_e5_tests_should_BeStaleNotFlagged_When_CommitDiffersFromHead` | Integration (temp git repo) | Commit, edit test file, run |
| UX-E6 unknown ids -> no-source, exit 0 | `tests/tests_report_cli.rs` | `ux_e6_report_should_CompleteAndCountNoSource_When_IdsMissing` | Rust golden | Exit 0 |
| UX-E7 integration/snapshot never medium | `tests/tests_report_cli.rs` | `ux_e7_integration_test_should_BeLowOrNotAssessed_When_ZeroUniqueCoverage` | Rust golden | `tests/e2e/` path |
| UX-E8 no dead end | `tests/tests_report_cli.rs` | `ux_e8_every_message_should_NameNextStepOrNoActionNeeded_When_AllRowsLinted` | Rust lint | Regex over E/W/I strings |
| UX-E9 read-progress line on stderr when TTY | `tests/tests_report_cli.rs` | `ux_e9_reading_line_should_GoToStderrOnly_When_TtyAndLargeInput` | Rust golden | stdout unchanged |
| UX-E10 dirty tree, non-ancestor, non-git warnings | `tests/tests_report_cli.rs` | `ux_e10_report_should_WarnAndExitZero_When_DirtyTreeNonAncestorOrNonGit` | Integration (temp git repo) | R23, R24, R25 |
| UX-E11 recipe notice, progress, resume | `scripts/test-effectiveness/per-test-go-coverage.sh` | `ux_e11_recipe_should_NoticeBeforeFirstTestAndResume_When_Interrupted` | Integration | Notice order; finished profiles untouched on `--resume` |
| UX-E12 experimental rows in own section with own count | `tests/tests_report_cli.rs` | `ux_e12_experimental_rows_should_BeSeparateAndTagged_When_ExperimentalFlag` | Rust golden | Absent from main counts and joint check; `section: experimental` in json; one summary line names them without the flag |
| UX-E13 class E JSON error object | `tests/tests_report_cli.rs` | `ux_e13_error_should_BeJsonObjectOnStdout_When_FormatJsonAndClassE` | Rust golden | Exit 2, `error.id` from the stable-id table |
| UX-E14 comment-only edits are not staleness | `tests/tests_report_cli.rs` | `ux_e14_ignore_directive_edit_should_NotStaleFile_When_DismissingFinding` | Integration (temp git repo) | Add directive, re-run on same input; tests still assessed |
| UX-E15 producer recipe contract | `scripts/test-effectiveness/recipe-lib.sh` | `ux_e15_recipes_should_NoticeProgressPartialSidecar_When_AnyProducerWrapper` | Integration | Notice order, `*.partial` marker, resume statement, `.run.json` on every exit path, R9 from a budget stop |
| UX-D1 dismiss in 1 edit, 1 re-run | `src/test_effectiveness/report.rs` | `ux_d1_finding_should_Vanish_When_OneDirectiveLineAddedAndRerun` | Rust golden | Count edits = 1 |
| UX-D2 dismissal persists while directive adjacent | `src/test_effectiveness/report.rs` | `ux_d2_dismissal_should_Persist_When_UnrelatedTestsChange` | Rust golden | Restated semantics per plan |
| UX-D3 reasonless directive malformed | `src/test_effectiveness/report.rs` | `ux_d3_directive_should_BeMalformedAndNotSuppress_When_NoReason` | Rust golden | `kibitzer run` shows `[ignore-syntax]` |
| UX-D4 suppressed count visible | `src/test_effectiveness/report.rs` | `ux_d4_summary_should_ShowSuppressedCount_When_DirectivePresent` | Rust golden | "suppressed by kibitzer:ignore: N" |
| UX-D5 ignore vs report_false_positive split | `tests/tests_report_cli.rs` | `ux_d5_before_acting_and_docs_should_StateSplitInOneSentenceEach_When_Read` | Rust golden + doc grep | Both sentences present |
| UX-D6 no `[unused-ignore]` for dynamic rules | `src/test_effectiveness/report.rs` | `ux_d6_report_should_NotEmitUnusedIgnore_When_DynamicFindingAbsent` | Rust golden | Remove flagging condition, re-run |
| UX-D7 non-specific reasons counted, still suppress | `src/test_effectiveness/report.rs` | `ux_d7_nonspecific_reason_should_BeCountedAndSuppress_When_StoplistMatches` | Rust golden | Counter in text and json |
| UX-D8 suppressed findings listable with reasons | `src/test_effectiveness/report.rs` | `ux_d8_show_suppressed_should_ListFindingsWithReasons_When_FlagOrJson` | Rust golden | Non-specific mark; json `suppressed[]` always present |
| UX-A1 strengthen first, name covering test | `src/checkers/test_quality_tests.rs` | `ux_a1_message_should_ListStrengthenFirstAndRequireNamingCoveringTest_When_NoAssertion` | Rust golden | Story 1.1.2 "Agent guidance (UX-A1)" AC |
| UX-A2 no dynamic finding in hook mode | `src/checkers/test_quality_tests.rs` | `ux_a2_hook_should_NeverEmitDynamicRule_When_AnyInput` | Rust golden | Story 1.1.5 "Hook and MCP paths never emit dynamic rules (UX-A2)" AC; mirrors the NFR1 test |
| UX-A3 weak-assertion wording from backtest | `docs/check-ideas.md` | `ux_a3_weak_assertion_wording_should_BeRevisedBeforeG1_When_ObservedInTranscripts` | Manual | Story 1.2.1 Goodhart-measurements AC: record observed/not observed with session count; revise `test-tautology` wording before G1 |
| UX-A4 json and brief carry advisory flag and non-claims | `tests/tests_report_cli.rs` | `ux_a4_json_and_brief_should_CarryAdvisoryAndNonClaims_When_Emitted` | Rust golden | Sentence present in text, json, brief |
| UX-A5 false-positive acknowledgement says it does not silence | `tests/false_positives_cli.rs` | `ux_a5_ack_should_SayDoesNotSilenceAndNameIgnore_When_Recorded` | Integration | Existing queue reply kept, one sentence appended |
| UX-X1 state conveyed by text not only color | `tests/tests_report_cli.rs` | `ux_x1_tier_warning_error_should_HaveTextLabels_When_ColorOffAndOn` | Rust golden | Compare stripped outputs |
| UX-X2 no ANSI under NO_COLOR/non-TTY | `tests/tests_report_cli.rs` | `ux_x2_output_should_HaveNoAnsi_When_NoColorSetOrPiped` | Rust golden | Regex `\x1b\[` |
| UX-X3 ASCII only | `tests/tests_report_cli.rs` | `ux_x3_output_should_BeAscii_When_AllFormats` | Rust lint | Bytes < 0x80 |
| UX-X4 plain space-aligned columns | `tests/tests_report_cli.rs` | `ux_x4_table_should_HaveHeaderRowAndNoMergedCells_When_Rendered` | Rust golden | Column parse |
| UX-X5 no dim-only emphasis | `tests/tests_report_cli.rs` | `ux_x5_emphasis_should_NotRelyOnDimStyling_When_ColorOn` | Manual | Review SGR codes used in color mode; no SGR 2/90 conveying meaning |
| UX-R22 `section` and `signal_mode` vocabulary, least-validated mode order | `tests/tests_report_cli.rs` | `ux_r22_section_and_signal_mode_should_SeparateMainFromExperimental_When_ExperimentalFlag` | Rust golden | Rows tagged `section`; `mode` ordering `experimental` < `descriptive` < `report_only` < `tiered` |
| UX-R23 overlap fields hold test ids; `redundant_only_with_flagged` definition; enum spelling | `tests/tests_report_cli.rs` | `ux_r23_overlap_ids_and_enum_spelling_should_MatchContract_When_Json` | Rust golden | Cross-package partner id; `in_order`, `first_killer_only` |
| UX-R24 leave-one-out result shown and malformed file rejected | `tests/tests_report_cli.rs` | `ux_r24_leave_one_out_should_ShowRowLineAndJsonField_When_LooFileSupplied` | Rust golden | R3 naming `--leave-one-out` when malformed |
| UX-E16 exit 1 only for internal or output IO failure | `tests/tests_report_cli.rs` | `ux_e16_exit_one_should_OccurOnlyForInternalFailure_When_StdoutClosedAndMalformedInputTried` | Integration | Malformed input gives 0 or 2, never 1 |
| UX-D9 directive without reason noted in report | `tests/tests_report_cli.rs` | `ux_d9_report_should_NoteMalformedDirectiveAndCount_When_ReasonMissing` | Rust golden | Row note, summary count, `summary.suppressed.malformed_directives` |
| UX-X6 banner unwrapped, other non-row lines wrap at 100 | `tests/tests_report_cli.rs` | `ux_x6_wrap_should_KeepBannerOneLineAndWrapOthersAt100_When_LongLines` | Rust golden | No token split |
| UX-H9 pre-G1 hook mode collapses nothing | `src/checkers/test_quality_tests.rs` | `ux_h9_hook_should_ListSixFindingsAndNoMoreLine_When_SixVacuousTestsAndNoCapRecorded` | Rust golden | Six empty tests in one file; no `+N more`; cap recorded only after G1 |
| UX-H10 non-claim line and quiet path naming `kibitzer:ignore` and `report_false_positive`, <=4 lines | `src/checkers/test_quality_tests.rs` | `ux_h10_finding_should_NameBothQuietPaths_When_AnyHookOrRunChecksFinding` | Rust golden | Per promoted rule; line count <=4; both names present |
| UX-R25 schema evolution policy and freeze test | `tests/tests_report_cli.rs` | `ux_r25_consumer_should_IgnoreUnknownKeysAndEnums_When_SchemaVersionOne` | Integration | Inject unknown key and enum into fixture; rename a key and expect the freeze test to fail |
| UX-R26 joint check with `with_data == 0` is null | `tests/tests_report_cli.rs` | `ux_r26_joint_check_should_BeNullAndNoCount_When_NoMemberHasData` | Rust golden | Text no-data sentence; json `units_held_only_by_set` and `zero_note` null |
| UX-R27 R17 vs R27 distinct | `tests/tests_report_cli.rs` | `ux_r27_report_should_PrintNothingAssessedNotNoTestsFlagged_When_AllTestsNotAssessed` | Rust golden | All stale input gives R27 `nothing-assessed`; 40 of 40 assessed none flagged gives R17 scope form and full footer |
| UX-R28 per-record `input` and unique `test.id` with `#n` suffix | `tests/tests_report_cli.rs` | `ux_r28_record_should_CarryInputIndexAndUniqueTestId_When_DuplicateNamesInFile` | Integration | `input` indexes `header.inputs`, null for static; `#2`, `#3` in source order |
| UX-R29 `confidence` omitted in `descriptive` and `report_only`; mixed-verdict mode | `tests/tests_report_cli.rs` | `ux_r29_confidence_should_BeOmittedExactlyWhenModeDescriptiveOrReportOnly_When_VerdictsMixed` | Rust golden | Mode matrix; `report_only` spelling in json |
| UX-R30 experimental vocabulary, own window and heading | `tests/tests_report_cli.rs` | `ux_r30_experimental_section_should_HaveOwnTopWindowAndHeading_When_ExperimentalFlag` | Rust golden | Capped signals only in the section; `<flagged> flagged, <shown> shown` |
| UX-R31 static rules exempt from staleness | `tests/tests_report_cli.rs` | `ux_r31_static_finding_should_BeListedAndCountedStale_When_TestFileStale` | Integration (temp git repo) | Listed and counted under Not assessed: stale; regenerate note |
| UX-R32 `brief` differs from `json` only by `record_non_claims` and `verify_first` | `tests/tests_report_cli.rs` | `ux_r32_brief_should_EqualJsonPlusTwoKeys_When_SameInput` | Integration | Key-set diff per record; closed non-claim list |
| UX-R33 `(all flagged)` overlaps tag and summary count | `tests/tests_report_cli.rs` | `ux_r33_overlaps_line_should_EndAllFlagged_When_EveryPartnerFlagged` | Rust golden | Tag count equals summary count |
| UX-R34 `joint_check` key order and typed `ignore_note`, `record_non_claims` | `tests/tests_report_cli.rs` | `ux_r34_joint_check_should_UseContractKeyOrder_When_Json` | Rust golden | Order `class, rules, unit_kind, set, set_size, with_data, units_held_only_by_set, zero_note, basis` |
| UX-E17 CI degradation keys on class W and mode | `tests/tests_report_cli.rs` | `ux_e17_clean_run_should_HaveNothingFlaggedClassI_When_ZeroFlags` | Integration | CI step keyed on `class == "W"` passes on class I only |
| UX-E18 json error object discriminator, R19 empty stdout | `tests/tests_report_cli.rs` | `ux_e18_error_key_should_DiscriminateAndCarryNoEnvelopeKeys_When_ClassE` | Integration | `error` tested first; R19 stdout empty |
| UX-E19 R22 auto-switch changes text only | `tests/tests_report_cli.rs` | `ux_e19_json_and_brief_should_BeByteIdentical_When_AutoSwitchToPackageView` | Integration | Compare with and without; `packages[]` only under `--by-package` |
| UX-E20 `note:` prefix for class I and ` (lowers: ...)` suffix | `tests/tests_report_cli.rs` | `ux_e20_messages_should_UseClassPrefixAndLowersSuffix_When_TextAndJson` | Rust golden | Stderr copy once per W and I line under json |
| UX-E21 capped signals only in experimental section | `tests/tests_report_cli.rs` | `ux_e21_capped_signal_should_AppearOnlyInExperimentalSection_When_VerdictRecorded` | Integration | PIT without full matrix after a G2 verdict; R8/R9 text |
| UX-E22 `--kill-completeness` flag maps to R9, R8 only for PIT structure | `tests/tests_report_cli.rs` | `ux_e22_flag_should_FireR9WithSourceFlag_When_SampledGiven` | Integration | `params.source` flag; R8 absent |
| UX-E23 recipe vocabulary `attribution`, sidecar path, partial-marker error | `scripts/test-effectiveness/recipe-lib.sh` | `ux_e23_recipe_should_UseAttributionKeyAndDirectorySidecarPath_When_DirectoryOutput` | Integration | `out/run.json` has `attribution`, no `mode`; partial error names the marker |
| UX-D10 `same_reason_groups` in json; derived non-specific footer | `tests/tests_report_cli.rs` | `ux_d10_json_should_ListSameReasonGroups_When_FiveSameReasonDismissals` | Rust golden | `{rule, reason_text, count>=5}`; footer from `non_specific_reason > 0` |
| UX-X7 row blocks, no one-line-per-finding promise | `tests/tests_report_cli.rs` | `ux_x7_text_should_UseRowBlocksAndGrepViaRuleHeadline_When_TextMode` | Rust golden | Grep on `[rule]` headline finds each finding |

### Surface bullets not in section 9 (S3, S6, S7, S8, D-E4, D-E5)

| UX Criterion | Test File | Test Name | Tool | Steps |
|---|---|---|---|---|
| S3 JSON valid with 0 findings (`"findings": []`, `not_assessed`) | `tests/tests_report_cli.rs` | `s3_json_should_ParseWithEmptyFindings_When_NothingFlagged` | Rust golden | Story 2.3.1 "JSON contract (UX S3)" AC |
| S3 deterministic key and finding order | `tests/tests_report_cli.rs` | `s3_json_should_HaveStableKeyOrder_When_RunTwice` | Rust golden | Story 2.3.1 "JSON contract (UX S3)" AC |
| S3 `schema_version` present and first | `tests/tests_report_cli.rs` | `s3_json_should_HaveSchemaVersion_When_Emitted` | Rust golden | Story 2.3.1 "JSON contract (UX S3)" AC |
| S6 unknown option key warns, names valid keys | `src/config.rs` | `s6_config_should_WarnWithValidKeys_When_UnknownOptionKey` | Rust golden | Story 1.1.5 AC |
| S6 `check list` shows rule default state `off` before G1 | `tests/test_quality_registry.rs` | `s6_check_list_should_ShowDefaultOff_When_BeforeG1` | Rust golden | Story 1.1.5 AC |
| S7 `--verbose` stderr-only `[tests]` lines with reasons | `tests/tests_report_cli.rs` | `s7_verbose_should_WriteTestsPrefixedLinesToStderrOnly_When_Flag` | Rust golden | Story 2.3.1 "Verbose diagnostics (UX S7)" AC |
| S8 recipe prints three-line success output and next command | `scripts/test-effectiveness/per-test-go-coverage.sh` | `s8_recipe_should_PrintSummaryWroteAndNextLines_When_Success` | Integration | Story 2.2.2 "Recipe output and errors" AC |
| D-E4 `report_false_positive` queue write failure text | `tests/false_positives_cli.rs` | `false_positive_tool_should_AppendLocalIgnoreSentence_When_QueueWriteFails` | Integration | Story 2.3.2 AC (listed in section D) |
| D-E5 repeated-dismissal footer hint | `src/test_effectiveness/report.rs` | `footer_should_HintConfigOption_When_FiveDismissalsShareRuleAndReason` | Integration | Story 2.3.2 AC (listed in section D) |

## Story Traceability Index

The mapping tables above are keyed by requirement and test name, not by story id. This index gives every plan story one anchor test (section and name, each occurring in this file), so "every story has a validation row" is checkable by grep; a story's other rows sit in the same section under the story's acceptance criteria. Stories 3.2.4 and 3.2.5 share the arm-ingest row (`jvm_and_ts_arms_...`); the PIT-specific row is the `comparison_should_StateCoverageUnitKind_When_PitInput` anchor.

| Story | Section | Anchor test |
|---|---|---|
| 0.1.1 | E | `catalog_should_HaveCitationAndValidationStatusPerSignal` |
| 0.2.1 | A | `is_test_file_should_ReturnTrue_When_GoTestSuffix` |
| 0.2.2 | E | `corpus_table_should_ListEveryRequirementsRepoAsDynamicOrStaticOnly` |
| 0.3.1 | E | `power_gate_should_ProjectCommitsAs96OverYield` |
| 0.3.2 | E | `spike_should_RecordProcessLaunchesCpuAndIsolatedVsInOrderDisagreement` |
| 0.3.3 | E | `enrich_story_should_ReadNotBuiltWithRate_When_SpikeNoGo` |
| 0.3.4 | E | `spike_should_RecordVersionCountsWallClockAndTm` |
| 0.3.5 | E | `policy_should_RequireReadOnlyMountEnvAllowlistNetworkDenyTimeoutsAndPinnedCommits` |
| 0.4.1 | E | `g0_should_ExistAndRecordAllPreregisteredNumbers` |
| 1.1.1 | A | `test_no_assertion_should_ReportZero_When_HelperAssertsViaTFatal` |
| 1.1.2 | A | `test_no_assertion_should_ReportOne_When_PythonBodyOnlyCallsParse` |
| 1.1.3 | A | `test_tautology_should_Report_When_ExpectOneToBeOne` |
| 1.1.4 | A | `test_mock_only_should_Report_When_OnlyVerifyAfterCreate` |
| 1.1.5 | A | `default_checks_should_NotContainTestQualityEntries_When_BeforeG1` |
| 1.2.1 | A | `backtest_record_should_ListHitsAndTenLabeledExamples_When_OnlyNewRun` |
| 1.2.2 | E | `triage_should_HaveAtLeast50VerdictsPerRulePerLanguageOrAll` |
| 1.2.3 | A | `hook_p95_should_NotExceedGoTableDrivenCandidate_When_1000SampledTestFiles` |
| 1.3.1 | E | `g1_should_ListWilsonLowerBoundAndNPerRuleLanguage` |
| 2.1.1 | B | `build_should_ReturnMissingProvenance_When_VersionNone` |
| 2.1.2 | B | `unique_coverage_should_ReturnSetDifference_When_T1T2T3Example` |
| 2.1.3 | B | `exclusion_should_MarkStale_When_TestFileChangedSinceInputCommit` |
| 2.1.4 | B | `locate_should_ReturnLocated_When_CoveragePyContextMatchesTestFunction` |
| 2.1.5 | B | `parse_scale_should_StayWithinMemoryBound_When_EachAdapterFormatAtTargetSize` |
| 2.2.1 | C | `parse_should_GiveNoassertSameLinesAsPos_When_ThreeTestFixture` |
| 2.2.2 | C | `parse_should_MatchProfileBlocksAndSetIsolated_When_TwoProfileFixture` |
| 2.2.3 | C | `parse_should_MapTestToFileLine_When_TnAndDaPresent` |
| 2.2.4 | C | `parse_should_TagFirstKillerOnlyAndDowngrade_When_AllKilledBySingle` |
| 2.2.5 | C | `parse_should_CreditBothKillers_When_KillingTestsAorB` |
| 2.2.6 | C | `parse_should_SetSkippedAndCost_When_SkippedTestcaseWithTime` |
| 2.2.7 | C | `adapters_should_NeverPanicOrExitOtherThanZeroOrTwo_When_MalformedCorpus` |
| 2.2.8 | C | `recipe_should_PrintNoticeBeforeProgressAndWriteSidecarOnBudgetStop_When_MutationWrapperRun` |
| 2.2.9 | C | `sidecar_should_SetBudgetLimitedAndCopyCounts_When_HandWrittenSidecarBudgetExhausted` |
| 2.3.1 | D | `report_should_RenderEachModeColumnSetPerMatrix_When_FourModes` |
| 2.3.2 | D | `footer_should_HintConfigOption_When_FiveDismissalsShareRuleAndReason` |
| 2.3.3 | D | `enrich_should_AddCallsInto_When_ResolvedRateAboveG0Threshold` |
| 2.3.4 | D | `brief_record_should_UseRecordNonClaimsKey_When_FormatBrief` |
| 2.3.5 | B | `staleness_should_ExcludeTest_When_CoveredProductionFileChangedSinceCommit` |
| 3.1.1 | E | `protocol_should_EqualG0NumbersVerbatim_When_Compared` |
| 3.1.2 | E | `harness_should_UseSameSeededMutantSampleBeforeAndAfter_When_RunTwiceWithSeed7` |
| 3.1.3 | E | `proxy_should_DetectLossOfPlantedEssentialTest_When_KnownAnswerRepo` |
| 3.1.4 | E | `loo_should_ShowZeroIndividuallyAndSharedUnitsJointly_When_DuplicatePair` |
| 3.2.1 | E | `python_arm_should_CountFixtureImportWarnings_When_Complete` |
| 3.2.2 | E | `go_arm_should_RecordIsolatedVsInOrderChangedCount_When_Complete` |
| 3.2.3 | E | `rust_arm_should_StateAttributionNoData_When_Complete` |
| 3.2.4 | E | `comparison_should_StateCoverageUnitKind_When_PitInput` |
| 3.2.5 | E | `jvm_and_ts_arms_should_RecordTruncationAndKillCompleteness_When_Ingested` |
| 3.2.6 | E | `static_arm_should_RecordMutationAndCoverageDeltaAndUniqueCoverageShare_When_Complete` |
| 3.3.1 | E | `triage_should_ReportOwnerAgreementOnBlind10Percent_When_JsonlCounted` |
| 3.3.2 | E | `q3_sentence_should_ReadNotSupported1Of2_When_OneArmBelowThreshold` |
| 3.3.3 | E | `results_should_StateOutcomeOfH1ToH4WithNumbersAndArmNames_When_Read` |
| 3.3.4 | E | `pilot_should_RecordDeltaForShortlistAndRandomSampleAndCostCu_When_Triggered` |
| 3.4.1 | E | `g2_should_RecordPerSignalVerdictLanguageListAndQ3Outcome_When_Read` |
| 4.1.1 | E | `story_4_1_1_should_Stop_When_G1DecisionFileAbsent` |
| 4.1.2 | A | `assertion_removed_should_Report_When_EditDeletesAssertFromExistingTest` |
| 4.2.1 | D | `variant_should_RemainHiddenAndNotRemoved_When_G2DescriptiveOnlyNotYetUnhidden` |
| 4.2.2 | E | `stories_4_2_1_and_4_2_2_should_Stop_When_G2DecisionFileAbsent` |

## Test Stack
- **Unit**: Rust `#[test]` with checked-in golden files and a seeded in-tree PRNG for matrix properties (no `insta` or `proptest`: ADR-005 iteration 2 amendment); fixtures under `testdata/test-quality/` and `testdata/test-effectiveness/`.
- **Integration**: the built `kibitzer` binary spawned via `env!("CARGO_BIN_EXE_kibitzer")` and `std::process::Command` (precedent `tests/hook_contract.rs`; no `assert_cmd`) with temp git repos; adapter fixtures from real coverage.py, Go, Stryker, PIT runs; `--ignored` scale test run in release mode.
- **Experiment validation**: Python `pytest` for harness, null, stats, and cost-model scripts; checker scripts (`check-protocol.py`, `check-results.py`, `check-gates.sh`, `check-lit.py`) run over committed artifacts and git history; known-answer toy repo for the held-out proxy; owner-review checklists for G0/G1/G2 decision files.
- **E2E / UX**: terminal golden-output tests plus a manual checklist (UX-A3, UX-X5, owner walk-through of the Python recipe in `docs/test-effectiveness-report.md`).

## Coverage Targets and How to Measure

| Stack | Coverage command | Target |
|---|---|---|
| Rust | `cargo llvm-cov` then `covgate check coverage.json` (the repo's CI step; `covgate.toml`: 80% changed lines, 75% changed regions). `#[ignore]` scale tests do not count, so each story keeps a default-run small-N test that reaches its new lines | >=80% line on `src/test_effectiveness/**` and new checkers |
| Python (experiment scripts) | `pytest --cov=scripts/test-effectiveness` | >=80% line on harness, nulls, stats, cost model |

- All public report/analysis functions: happy path + error path covered (sections A-D).
- All input adapters: unit fixture from a real producer run plus one CLI integration test.
- Experiment epic: every pre-registered artifact (G0, protocol, tag, power memo, results records) has a mechanical check; every harness component has a calibration case with a known answer.
- UX acceptance criteria: each of the 89 criteria in `design/ux.md` section 9 has a test or manual step; the S3/S6/S7/S8 and D-E4/D-E5 bullets have cases and plan ACs.
- Null result: the outcome-table row selected at G0 Part B, the `NegativeResultReport`, and descriptive-only rendering each have a case (sections D and E).
- Hook latency: p95 per file recorded against the comparable existing checker (Story 1.2.3).
- Research-quality gates (not coverage): H1-H4 outcomes are reported with numbers whatever they are; a negative result is a valid outcome and is not a retry trigger.

## Migration Test
N/A. The plan has no schema or data migration (plan.md, "Migration Plan" omitted); rollback is config removal or deleting the hidden `Command::Tests` variant, covered by `default_checks_should_NotContainTestQualityEntries_When_BeforeG1` and `command_tests_should_BeRemoved_When_G2KeepsAllSignalsResearchOnly`.

## Requirement Coverage Summary

| Req | Cases (sample) | Req | Cases (sample) |
|---|---|---|---|
| SM1 | catalog/anecdote/link/survey checks | OBS1 | matrix provenance, report JSON fields |
| SM2 | catalog columns, no-unverified-rationale | OBS2 | `record_should_HaveFlaggedCountPrecisionAndDeltas_When_ArmComplete`, corpus table |
| SM3 | harness, nulls, proxy, triage, results checks | RC1 | default-off, hidden command, exit 0, no verdict |
| SM4 | registry, promotion, report CLI, tiers | RC2 | drift guard, ignore, rollback tests |
| SC1 | adapters, tool inventory | OOS1 | read-only report, no delete wording, no engine |
| SC2 | brief, no-LLM doc, enrich | Q1 | ADR-001 check, ingest-only guards |
| SC3 | unique coverage, unique kills, joint safe | Q2 | literature Q2 answer check |
| C1 | staleness, recipe, adapters | Q3 | analyze_q3, ordering header, PIT MutantSite |
| C2 | backtest records, occurrence record | Q4 | G2 record, languages header, profiles |
| C3 | profiles, spike per tool, arms | Q5 | no-assertion FP fixtures, G1 Wilson |
| C4 | cargo tree, roaring gate | Q6 | joint loss, proxy, power gate, P2 |
| NFR1 | latency, no dynamic in hook | NFR2 | scale test, cost model, R_max |
| NFR3 | no network, no LLM doc, no HTTP import, secret scan | SM5 | flagged-then-revised metric definition and pre-ship baseline |

**Totals** (recounted by script over the table rows after the round-5 UX-delta repair): section A 45, B 50, C 44, D 115, E 118, UX acceptance 89, surface bullets 9 (two of which repeat section D cases), so 470 rows and 468 unique cases (round 5b added four section D rows: descriptive-only in main list, banner never `report-only`, error-path escaping, `use`-site partition grep; before that 466 and 464; round-5 UX deltas: 445 and 443 at 68 UX rows; iteration 3: 415 and 413; iteration 2: 379 and 377; iteration 1: 337 and 335). Plan totals these cases trace to: 16 epics, 56 stories, 151 tasks (`implementation/plan.md`), and each of the 56 stories has an anchor in the Story Traceability Index. Every requirement ID in the Requirement Index (SM1-SM5, SC1-SC3, C1-C4, NFR1-NFR3, OBS1-OBS2, RC1-RC2, OOS1, Q1-Q6) appears in at least one row's Requirement column.
