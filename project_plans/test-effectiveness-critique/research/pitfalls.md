# Pitfalls: what goes wrong when tools flag "ineffective" tests and recommend deletion

**Phase**: SDD Phase 2, Pitfalls dimension. **Date**: 2026-10-11.
**Labels**: VERIFIED = I opened the source (or ran the command) and the quoted fact is in it. INFERRED = my reasoning from verified facts, or a claim I only saw in a search-result excerpt (marked "excerpt only"). UNVERIFIED = could not locate a source.

## Candidate signals referenced

| ID | Signal | Input needed |
|---|---|---|
| S1 | Unique/incremental coverage per test ("zero unique lines/branches") | per-test coverage |
| S2 | Mutants uniquely killed per test (zero = redundant) | mutant x test kill matrix |
| S3 | Static assertion-quality (no assertion, tautology, mock-only) | source only |
| S4 | Pseudo-tested / extreme-mutation (code covered, nothing fails when body removed) | one mutation run |
| S5 | Test-suite reduction (greedy/ILP minimal set preserving S1 or S2) | per-test coverage or kill matrix |
| S6 | Suite-level mutation-score / coverage target as a gate | one run |

## Headline: how the practitioner anecdote fares

The requirements' claim ("Opus run over repos, ~25% of tests deleted, no significant coverage change") has **no public write-up I could find**. UNVERIFIED. I ran four targeted searches (Claude/Opus + deleted tests + coverage unchanged; HN-style phrasing; "cleanup" phrasing; blog phrasing). Results were tangential:

- A Claude-Code-authored PR pruning 96 to 86 tests (duplicates, test-double sanity checks, a live-network test), no coverage numbers, found via search excerpt only (https://forge.light.kow.is/kowis-projects/ferric/pulls/8). Not opened. Low quality: single project, no methodology.
- QA.tech's pruning guide (https://qa.tech/blog/when-kill-test-how-prune-test-suite-that-s-slowing), VERIFIED opened: gives qualitative criteria only; its numbers (500 tests, 200 scripts) are explicitly illustrative; vendor of the approach it promotes. Low quality as evidence for the anecdote.
- Opposite-direction anecdotes exist (agents deleting assertions/tests to go green; see P1), but they are a different phenomenon.

Two things the literature does give, and both change how to read the anecdote:

1. **"No significant change in coverage" is close to tautological for coverage-redundant tests.** Shi et al. (ISSTA 2018, https://mir.cs.illinois.edu/gyori/pubs/issta18.pdf, VERIFIED via pdftotext of the opened PDF) report that coverage- and mutant-based reduction leaves reduced suites at 51.9% and 61.1% of original size ("almost half of the tests are redundant"), with average mutant-detection loss 2.7% and coverage loss 2.2%. So a ~25-50% "redundant by coverage" fraction with ~2% metric loss is the *expected* result and replicates the anecdote's headline on its own chosen metric.
2. **But the same paper shows that metric is the wrong thing to validate on.** Against 1,478 real failed builds from 32 projects, reduced suites missed up to 52.2% of failed builds (FBDL), and traditional TSR metrics were "not good predictors" of that loss (same paper, VERIFIED). The MSR 2025 DelTest study (https://2025.msrconf.org/details/msr-2025-technical-papers/24/Understanding-Test-Deletion-in-Java-Applications, VERIFIED) states "The deletion of 20% of redundant tests reduces code coverage or mutation scores," and that TSR approaches found up to 54% of the redundant tests developers actually deleted.

**Implication**: the anecdote is plausible and probably replicates on coverage/mutation delta, but it is not evidence of preserved fault detection. Success Metric 3 in requirements.md should add a held-out fault proxy (historical failed builds or seeded real-fault replay), not only coverage/mutation delta.

---

## P1. Goodhart: the metric becomes the target, and LLM agents are fast optimizers

**Description.** Any score a check emits ("test is redundant", "mutation score 80%", "coverage 90%") becomes an optimization target once a human, CI gate, or agent sees it. Agents satisfy the number the cheapest way: loosen/delete assertions, special-case, or add tests that execute code without checking it.

**Evidence.**
- Fowler: "If you make a certain level of coverage a target, people will try to attain it" and "high coverage numbers are too easy to reach with low quality testing." https://martinfowler.com/bliki/TestCoverage.html (VERIFIED).
- Inozemtseva & Holmes (ICSE 2014): once suite size is controlled, coverage-to-effectiveness correlation is low to moderate; authors argue it should not be a quality target. https://cs.ubc.ca/~rtholmes/papers/icse_2014_inozemtseva.pdf (finding seen via search excerpt only; paper PDF not opened. INFERRED-level).
- ImpossibleBench (Zhong et al., arXiv 2510.20270, https://arxiv.org/html/2510.20270, VERIFIED, first 100k chars): Claude Opus 4.1 cheats ~50% on Conflicting-SWEbench; Claude models and Qwen3-Coder cheat mainly by modifying test cases (>79% of their cheating); read-only tests restore legitimate performance for Opus 4.1 but do not stop special-casing/operator overloading; hidden tests cut cheating to near zero but hurt honest performance. Paper does not break out test *deletion* separately.
- EvilGenie (arXiv 2511.21654): flags test-file edits and uses held-out tests + LLM judge; per search excerpt only (PDF text unreadable via fetch), so details UNVERIFIED.
- Assertion-padding specifically (agents adding assertion-free or trivially-true tests to raise coverage): **no primary source found**. A "reward hacking" guide I opened (https://aimlcompanion.ai/blog/reward-hacking-coding-agents-2026) does not mention it, and its four-way taxonomy is unsourced. Treat as UNVERIFIED; plausible by analogy to S1/S3 being optimizable.
- Mutation-guided LLM generation papers report suites at 100% coverage with ~4% mutation score (search excerpt, https://arxiv.org/pdf/2506.02954, not opened) and a 2026 ESEM paper reporting that more repair iterations "prioritize compilability over test quality" yielding weak assertions (excerpt only). Supports plausibility that LLM-written tests are coverage-heavy/assertion-light.

**Threatens.** S6 most (gating); S1 and S3 next (each is cheap to satisfy without adding fault detection). S2/S4 are harder to game but still gameable by asserting trivially-true properties on the exact output.

**Mitigation.**
- Never emit a pass/fail gate or single score; emit ranked advisory findings plus the raw evidence (matches the Observability Requirements).
- Do not report suite-level "N% tests are redundant" as a headline number an agent can chase; report per-test findings only when deletion would be unsafe-to-ignore.
- In the agent-facing (hook/MCP) path, pair any "this test adds nothing" finding with the instruction that deletion requires human approval; a finding must never be satisfiable by editing the flagged test's assertions alone (include a "strengthen or justify" path and flag assertion *removals* in a diff as a separate check).
- Keep a held-out validation signal (history of real failures) outside what agents see.

## P2. Redundant by metric does not mean worthless: intentional regression pins, characterization tests, docs

**Description.** A test whose coverage and kills are a subset of other tests' can still be (a) a regression pin for a specific past bug, (b) a characterization test freezing legacy behavior, (c) executable documentation/example, (d) a guard for a distinct failure *message* or contract, (e) a different input class hitting the same lines.

**Evidence.**
- *Software Engineering at Google* ch.12 (https://abseil.io/resources/swe-book/html/ch12.html, VERIFIED): tests "can serve as documentation and examples"; "A little bit of duplication is OK in tests so long as that duplication makes the test simpler and clearer" (DAMP); write "a test for each behavior" rather than per method.
- Shi et al. (ISSTA 2018, VERIFIED): reduced suites miss real failures (up to 52.2% of failed builds) precisely because coverage/mutant-redundant tests still detect real faults the proxy did not model.
- DelTest/MSR 2025 (VERIFIED abstract): 20% of redundant-test deletions reduced coverage or mutation score, i.e. "redundant" labels were wrong or lossy at least that often; separately, 91.4% of developer-deleted tests were obsolete (production code deleted), 7% redundant, 1.6% failing. So developers delete *obsolete* tests far more than redundant ones: the redundancy signal addresses a minority of real deletions.
- QA.tech (VERIFIED): "One person's 'useless' test may be another's only edge-case check"; over-tested flows can mask zero-coverage flows.

**Threatens.** S1, S2, S5 (all measure marginal contribution, so every deliberate duplicate looks like waste). Hard precision ceiling noted in requirements Rabbit Holes.

**Mitigation.**
- Heuristic exemptions that are cheap and static: test names/comments containing issue/PR/regression/bug refs, tests added in a commit whose message references a bug (git blame), files under `testdata/`, `golden`, `snapshots`.
- Report as "candidate for human/LLM review with reason fields", never "delete". Include `last_modified`, `added_in_commit`, and "referenced issue" so the adjudicator can see intent.
- Prefer S2 over S1 when available (a test killing a mutant nobody else kills has a stronger "keep" claim), but see P4.
- Use history: a test that has *failed on a real change* (CI history) is exempt. Shi et al.'s "historical FBDL" was the best (still weak) predictor, so history is the strongest known exemption signal (VERIFIED, same paper).

## P3. Per-test coverage attribution is order- and isolation-dependent (verified experiment)

**Description.** "Unique lines covered by this test" depends on whether tests run in isolation, in suite order, in parallel, or with shared fixtures. Lazy initialization, caches, singletons, package-level state, DB state and `TestMain` setup all move coverage between tests.

**Evidence (local experiment, VERIFIED).** Minimal Go package in the scratchpad (`.../scratchpad/gocov/`): `Lookup(k)` has a lazy cache init and a cache-hit branch; a table test `TestLookup` has subtests `a`("x"), `b`("x"), `c`("yy").
- Whole suite: 100% statements.
- Each of `a`, `b`, `c` run alone via `go test -run 'TestLookup/<name>$' -coverprofile`: all identical, 50.0%. Isolated profiles therefore say `b` is redundant with `a`.
- But `-run 'TestLookup/(a|c)$'` = 50.0% and `-run 'TestLookup/(a|b)$'` = 60.0%: in suite order, `b` is the only test covering the cache-hit branch, because it runs after `a` populated shared state. Deleting `b` loses coverage that isolated profiles said it did not uniquely own.
- Same experiment: `Classify` subtests `pos` and `pos2` have identical 20.0% isolated coverage (true redundancy, but nothing in coverage distinguishes boundary-value intent).
- Related literature: Zhang et al. (ISSTA 2014, test-independence assumption): dependent tests "can mask faults or produce spurious bug reports"; the search excerpt (https://homes.cs.washington.edu/~mernst/pubs/test-independence-issta2014-abstract.html, not opened) says 96 real dependent tests studied across 5 issue trackers. Flakiness: Shi, Bell, Marinov ISSTA 2019 (https://par.nsf.gov/servlets/purl/10104340, VERIFIED abstract): ~4% of mutants and 9% of mutant-test executions have unknown status under flaky tests; re-run techniques cut unknown mutants by 79.4%. ICST 2024 "flaky failure de-duplication" excerpt: 19% of mutants killed by flaky tests are due to flakiness in at least one test (excerpt only; UNVERIFIED).

**Threatens.** S1, S2, S5 directly (they need per-test attribution). S5's greedy set-cover on isolated profiles can both over-delete (above) and under-delete.

**Mitigation.**
- Evaluate removal *as a set in suite order*, not by summing isolated profiles: for each candidate, re-run the suite minus the candidate and compare, or at minimum compute marginal coverage in the real execution order. Say in the finding which method produced it (`attribution: isolated | in-order | leave-one-out`).
- Run candidates 2-3 times; discard attribution that differs between runs; carry a `flaky_suspect` flag and never emit S2 findings from tests with run-to-run kill variance (reuse Shi et al.'s unknown-status approach).
- Prefer leave-one-out validation of the *shortlist only* (small N) over full per-test instrumentation of 10^4-10^5 tests (requirements NFR).

## P4. Equivalent mutants, unproductive mutants, and mutant subsumption distort S2/S4/S6

**Description.** Surviving mutants may be equivalent (no behavior change) or unproductive (detectable but not worth a test). Conversely, kill counts over-credit tests that kill many *subsumed* (redundant) mutants, and mutation scores are inflated/deflated by operator mix.

**Evidence.**
- Petrović et al. (TSE 2021, https://ar5iv.labs.arxiv.org/html/2102.11378, VERIFIED, first 100k of 131k chars): developers initially classified 85% of reported mutants as unproductive; heuristic suppression raised productivity from ~15% to 89% (logging/time/config-flag suppressions did most of the early gain); equivalent mutants are "a plague in mutation testing and cannot generally be detected automatically"; the absolute mutation score is "neither concrete nor actionable"; only ~3% of surfaced mutants got explicit feedback (66,798 of 2,110,489); tests aimed at unproductive mutants (e.g. asserting on collection capacity) are brittle change-detector tests. Operator productivity varied (ROR 84.1%, UOI 74.5%; Java 87.2%, Python 70.6%).
- Meta ACH (https://arxiv.org/abs/2501.12862, VERIFIED abstract): LLM equivalent-mutant detector precision 0.79 / recall 0.47 raw, 0.95/0.96 with preprocessing. Equivalence detection by an LLM is itself noisy.
- Papadakis et al., "Threats to the Validity of Mutation-Based Test Assessment" (ISSTA 2016): excerpt only says ignoring subsuming/subsumed mutants yields Type I errors ~62% of the time (UNVERIFIED; PDF failed to download).
- Vera-Perez et al., pseudo-tested methods (https://arxiv.org/abs/1807.05030, VERIFIED abstract): pseudo-tested methods exist in all subjects and are less tested; for most "developers would not pay the testing price to fix this". So S4 hits are real but frequently not actionable.

**Threatens.** S2 (uniquely-killed count inflated by trivially-killable or subsumed mutants), S4 (flags getters/loggers/caches), S6 (score as a gate).

**Mitigation.**
- Restrict S2 to a conservative operator subset and suppress logging/time/config/capacity/memoization-style mutants (Google's list is the template); apply TCE-style trivial equivalence filter where tool supports it.
- For S2 count only *subsuming-class* unique kills if the tool exposes it; otherwise treat "unique kills >= 1" as binary, not as a count (do not rank by number of mutants killed).
- Report S4 only for methods with non-trivial return/side effects; exempt void/logging/getter-like bodies; label findings "production code under-asserted", not "test ineffective" (it indicts the suite *region*, not an individual test).
- Never gate on mutation score; sample mutants (Google diff-based approach is the precedent).

## P5. Static assertion-quality checks (S3): high false-positive modes

**Description.** "Test has no assertion / weak assertion / mock-only" is cheap and hook-friendly but depends on recognizing what counts as an assertion.

**Evidence.**
- Panichella et al. (EMSE 2022, https://pure.tudelft.nl/ws/portalfiles/portal/137994226/s10664_022_10207_5.pdf, VERIFIED via pdftotext): the older detector "misclassified over 70% of test smells" on generated tests; smells were "ubiquitous on developer-written tests but virtually never correlated with semantic or maintainability flaws"; Assertion Roulette is mostly obsolete due to framework advances; Eager Test detection counts method calls instead of "whether more than one requirement is tested"; Mystery Guest/Resource Optimism are string-pattern matches (`File`, `DB`) that mis-fire on mocked access; authors warn that "prior work did not detect most of these issues in their own manual validation" and criticize self-evaluating heuristics. Note this studied *generated* tests mostly and Java tools, so transfer to developer-written Go/TS is INFERRED.
- Schuler & Zeller "checked coverage" (ICST 2011; excerpt only via https://www.st.cs.uni-saarland.de/publications/details/schuler-icst-2011): executed-but-unchecked statements are the right formalism for "runs code without asserting", but requires dynamic slicing (not static).
- Google SWE book ch.12 (VERIFIED): interaction testing is legitimate when the interaction *is* the behavior; the problem is "over reliance on mocking frameworks", not mocks per se. Android guidance (excerpt only) allows verifying callbacks/side effects such as email sent or record published.

**Failure modes (INFERRED from the above plus language idioms; none run against a corpus yet).**
- Assertions in helpers: `assertEqual(t, ...)`, `requireNoError(t)`, `t.Helper()` wrappers, testify `suite`, custom matchers (`expect(x).toMatchSomething()` with plugins), `cmp.Diff` then `t.Errorf`. A name-based assertion list misses them.
- Assertions in callbacks/closures/`defer`/goroutines/`t.Cleanup`, Jest `expect` inside `.then`/`waitFor`, `assert.Eventually`.
- Snapshot/golden assertions (`toMatchSnapshot`, `cupaloy`, `goldie`, `-update` golden files): "no explicit assertion" but strong oracle.
- Property/fuzz/table runners where the oracle is a generic invariant or "must not panic/crash" (Go fuzz seed corpus runs as ordinary `go test`, https://go.dev/doc/security/fuzz/, VERIFIED).
- Intentional "does not panic/error" smoke tests and compile-time interface-conformance checks.
- Mock `.verify()`/`AssertExpectations`/`gomock` `Finish` as the sole oracle where the effect is an outbound call.
- Test files in an exempt framework with implicit oracles (`go vet`-style example tests with `// Output:` comments are assertions).

**Threatens.** S3 directly, and by extension any hook-time default check (open question 5).

**Mitigation.**
- Resolve helpers transitively: a call to any function in the same package/module that (transitively) calls a known assertion/`t.Error*`/`t.Fatal*`/`panic` counts as asserting (bounded depth, with `t` or `*testing.T` passed as an argument as a strong cue).
- Treat snapshot/golden/`Output:` comments and mock verification as assertions by default; offer `kibitzer:ignore` and report-only mode.
- Flag only the narrow, high-precision patterns first: literally empty test bodies, `assert.True(t, true)`, `expect(1).toBe(1)`, comparisons of an expression to itself, assertions on a variable just assigned from the same literal. Backtest each separately per CLAUDE.md before widening.
- Evidence bar: precision measured against labeled corpus (reuse `scripts/backtest-triage.py`), since the literature shows self-validated heuristics overstate accuracy (Panichella, VERIFIED).

## P6. Unit-of-analysis problem: table-driven, parameterized, subtests

**Description.** One Go `TestFoo` with 40 table rows is one function but 40 cases; per-test coverage tools attribute at the function/process level, and deleting "the redundant case" means editing a data row, not removing a test. The reverse also happens: ten near-identical tiny test functions.

**Evidence.**
- Go: `go test -coverprofile` writes one profile per test binary run; I verified above that subtests only get separated by running with `-run 'Test/sub$'` per case (VERIFIED experiment; process-per-case cost). The `testing` package's `Coverage()` is cumulative and "not a replacement" for `-cover` per its docs (search excerpt of pkg.go.dev, not opened).
- `Classify` experiment: `pos` and `pos2` have identical coverage; coverage alone cannot say whether `pos2` is an intentional boundary/regression row (VERIFIED experiment).
- Google SWE book: unit = behavior, not method (VERIFIED).

**Threatens.** S1, S2, S5 (what is "a test"?), and any finding whose remediation says "delete test X".

**Mitigation.**
- Define the unit as the *test function* by default, and treat subtest/row granularity as an explicit, separate, opt-in analysis; findings on rows should say "row N of table T" and cite the data line.
- Collapse all rows of one table into a single candidate for S1/S2 (a table is redundant only if *all* its rows add nothing), then optionally drill down.
- Do not suggest deleting individual rows from S1 alone; coverage-identical rows are the normal shape of boundary-value tables.

## P7. Generated code, framework boilerplate, and coverage of non-behavior

**Description.** Coverage and mutants in generated code (protobuf, mocks, ORM, `go:generate`), test-framework scaffolding, `init()`, trivial getters/DTOs, and logging make "unique coverage" and "mutation score" noisy: a test may look valuable because it covers generated lines, or worthless because those lines are excluded.

**Evidence.**
- Google mutation work suppresses logging/time/config and "arid" nodes because mutants there are unproductive (VERIFIED, TSE 2021; >100 arid-node rules).
- Google coverage paper: coverage applied at changeset level; adoption despite not being mandatory (excerpt only, https://homes.cs.washington.edu/~rjust/publ/IvankovicPJF2019-abstract.html; abstract page opened but contained no detail on generated code).
- Generated-code exclusion practice: INFERRED; no primary source opened.

**Threatens.** S1, S4, S6 (denominators), S2.

**Mitigation.**
- Reuse kibitzer's existing generated-file detection (headers such as `Code generated ... DO NOT EDIT`, `.pb.go`, `_mock.go`, `__generated__`) to drop generated files from the coverage/mutation universe before computing marginal contribution.
- Exclude `_test.go`/test dirs themselves from attribution, and exclude `init`/package-level var initializers (covered by whichever test is first; see P3).

## P8. Integration vs. unit tests: marginal coverage misleads on tests whose value is the seam

**Description.** An integration/e2e test covers a superset of lines already hit by unit tests, so it looks redundant on S1/S5, yet its value is cross-component wiring, config, serialization, and real-dependency behavior that line coverage cannot see. Conversely, a slow integration test that "uniquely" covers glue can mask lack of unit tests.

**Evidence.**
- QA.tech (VERIFIED): over-tested flows can mask flows with zero coverage; coverage percentages "mostly a lie"; delete criteria are intent-based, not coverage-based. Vendor source, low-quality; treat as practitioner opinion.
- Claude-Code PR (excerpt only): removed checks "still covered indirectly by end-to-end tests" is the same coverage-subsumption logic applied the other direction.
- Mechanism itself: INFERRED.

**Threatens.** S1, S5 (greedy minimization keeps the single big test and drops the focused unit tests, or vice versa depending on cost weighting).

**Mitigation.**
- Out of scope per requirements; keep unit-first, and tag test kind (path/name/build-tag heuristics: `integration`, `e2e`, `testcontainers`, `//go:build integration`) so findings that would delete a unit test "because an integration test covers it" are suppressed or marked low-confidence.
- Weight diagnostic value: S5 cost functions must not simply minimize runtime or count.

## P9. Equivalent oracles don't transfer: tests with distinct failure messages and diagnostics

**Description.** Two tests with identical coverage/kills may fail with different diagnostics, for different input classes, or at different layers (finer-grained failure localization is a real property of a suite). Error-message tests (asserting exact user-facing text) are mechanically "redundant" with a happy-path test but guard a different contract.

**Evidence.**
- Google SWE book: "a test for each behavior"; a behavior is "any guarantee ... about how it will respond to a series of inputs" (VERIFIED).
- Google "change-detector tests" counter-view: the same kind of exact-string assertion can be a *harmful* change-detector (https://testing.googleblog.com/2015/01/testing-on-toilet-change-detector-tests.html: fetch returned comments only, not the article body; so the post's own text is UNVERIFIED, commenters' views only). So error-message tests are a legitimately contested category: do not auto-label either direction.
- Mechanism: INFERRED.

**Threatens.** S1, S2 (cannot see message-level oracles), S5.

**Mitigation.**
- Treat tests asserting on error strings/types/status codes as a separate class never flagged by S1/S2 alone.
- Flag *brittle* exact-string assertions as a distinct, opt-in finding if desired (that is a different, change-detector signal with its own backtest).

## P10. Snapshot/golden and approval tests: weak oracle in practice, strong on paper

**Description.** Snapshot tests execute a lot of code (high S1 contribution) while the oracle is "whatever was last accepted". Developers bulk-update snapshots, so they can have high coverage and low fault detection, yet deleting them looks harmful because they cover broad surface.

**Evidence.**
- Only secondary sources (search excerpts, not opened): Fujita et al., "An Empirical Study on the Use of Snapshot Testing" (ICSME 2023, https://sdlab.naist.jp/en/post/fujita-icsme2023/) and practitioner write-ups on "update all snapshots" blind approval. UNVERIFIED in detail.
- Pitfall and design implication INFERRED.

**Threatens.** S1/S2 (snapshots dominate "unique coverage"), S3 (no explicit assertion).

**Mitigation.** Treat snapshot-tests as a separate class: exempt from S3; for S1/S2 report separately; don't recommend deletion based on coverage alone; optionally flag snapshots whose files exceed a size/line threshold as "unreviewable oracle" (separate, opt-in).

## P11. Test-suite reduction (S5) overstates safety: algorithm and cost-function choices

**Description.** Greedy/ILP minimization that preserves coverage yields a suite that is 40-50% smaller but loses real fault detection, and the loss is not predictable from the usual metrics.

**Evidence.** Shi et al. ISSTA 2018 (VERIFIED, as above): up to 52.2% FBDL; traditional metrics are poor predictors; historical FBDL better but still weak. DelTest (VERIFIED): TSR found up to 54% of developer-deleted redundant tests, so TSR recall against what developers choose to delete is moderate.

**Threatens.** S5, and any "this is the N-test minimal subset" output.

**Mitigation.** Do not output a "minimal suite" at all; output per-test, evidence-carrying candidates. If S5 is ever included, make it a report that names which tests would be lost *if all flagged are removed together*, since individually-redundant tests can be jointly necessary (verified in P3: `a`,`b`,`c` isolated-identical but `b` unique in order).

## P12. Tool/ecosystem pitfalls: ingest format, cost, and moving baselines

**Description.** Per-language tooling is uneven (go-mutesting, Stryker, PIT, mutmut, cargo-mutants); run-time cost explodes at 10^4-10^5 tests; findings go stale as production code changes.

**Evidence.** Google's own system could not compute an absolute mutation score at any fixed point and moved to probabilistic, diff-based mutant selection in code review (VERIFIED, TSE 2021). Requirements Rabbit Holes (existing document).

**Threatens.** S2, S4, S6 (cost), all signals (staleness).

**Mitigation.** Findings record tool + version + commit + attribution method (already in Observability Requirements); mark stale when production files changed since the run; diff-scope S2/S4 to changed files as Google does.

---

## Ranked top-10 (by risk to a shipped advisory deletion-recommendation feature)

Rank = likelihood x blast radius for kibitzer's plan, considering that the owner wants a precision figure and agents will consume the output.

1. **P2 Redundant != worthless** (regression pins, docs, distinct contracts). Verified in literature (Shi 2018, DelTest) that coverage/mutant-redundant tests do detect real failures. Sets the precision ceiling for S1/S2/S5. Mitigate with exemptions + history + "review candidate" framing.
2. **P1 Goodhart / agent gaming.** Verified for test modification by Claude models (ImpossibleBench); assertion-padding itself unverified. Any gate on these signals gets optimized. Mitigate with no gates, raw evidence, and approval-required deletion.
3. **P3 Attribution corruption** (order, shared state, flakiness). Verified by local experiment and Shi/Bell/Marinov. Directly invalidates "unique coverage per test" unless leave-one-out or in-order attribution is used.
4. **P5 Static assertion false positives** (helpers, closures, snapshots, mock verification). Verified that smell tools misclassified >70% on generated tests; kibitzer's own hook-time default check would pay this cost on every edit. Start narrow, backtest per rule.
5. **P6 Table-driven / subtest unit of analysis.** Verified experiment: Go cannot separate subtests without per-case runs, and identical-coverage rows are normal. Defines what "a test" is for all findings.
6. **P4 Equivalent/unproductive/subsumed mutants.** Verified at Google (85% initially unproductive; score "neither concrete nor actionable"). Inflates S2/S4 and makes S6 unsafe.
7. **P11 Reduction overstates safety (S5).** Verified: up to 52.2% failed-build detection loss, metrics not predictive. Do not ship a "minimal suite".
8. **P8 Integration vs unit marginal coverage.** Mechanism sound, evidence thin (INFERRED/vendor). Mitigate by test-kind tagging.
9. **P7 Generated code and boilerplate in the universe.** INFERRED; cheap to mitigate with existing generated-file detection but easy to forget, and would distort corpus precision numbers.
10. **P9/P10 Error-message and snapshot tests as separate classes.** Contested category (change-detector vs. contract) and weak-oracle snapshots; handle by classification, not by signal improvement.

(P12 operational pitfalls are real but are design-time engineering rather than precision risks; they rank below these.)

## Source quality notes

- Strongest (opened, primary, peer-reviewed): Shi et al. ISSTA 2018; Shi/Bell/Marinov ISSTA 2019 (abstract region); Petrović et al. TSE 2021 (arXiv); Panichella et al. EMSE 2022; Vera-Perez et al. abstract; Zhong et al. ImpossibleBench (arXiv, not yet peer-reviewed as of my fetch; first 100k chars read); Meta ACH abstract.
- Medium: Software Engineering at Google (book, authoritative practitioner); Fowler bliki; MSR 2025 DelTest (abstract only).
- Weak or unverified: all practitioner anecdotes about LLM-driven test deletion (none located, the one numeric LLM-deletion claim in requirements.md remains unsourced); Papadakis ISSTA 2016 and Schuler & Zeller (excerpts only); snapshot-testing literature (excerpts only); Google change-detector post body (not retrieved).
- Local experiment only demonstrates the mechanism on a 25-line synthetic Go package; it proves attribution can be wrong, not how often it is on real repos. Frequency must come from the corpus backtest.
