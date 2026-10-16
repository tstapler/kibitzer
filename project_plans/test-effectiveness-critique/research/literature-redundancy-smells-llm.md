# Literature review part B: test redundancy / minimization, assertion quality, test smells, flakiness, LLM-as-judge

**Project**: test-effectiveness-critique (kibitzer). **Phase**: SDD 2 (research). **Date**: 2026-10-11.
**Scope**: the part-B question set only. Coverage-vs-effectiveness, mutation analysis, and pseudo-tested methods are another worker's part A; they appear here only where they support a part-B point.

## How to read the evidence labels

- **VERIFIED** = I opened the primary source (PDF text, arXiv abstract page, publisher/repository abstract, or a repository API record) in this session and the claim is in it. The URL is in the bibliography.
- **INFERRED** = my reasoning from verified facts; no source says it directly.
- **UNVERIFIED** = I saw it only in a search-result summary, a secondary source, or my own prior knowledge, and did not open the primary text.
- Where a PDF was pulled, I extracted it with `pdftotext` and read the relevant passages; where only an abstract was available, the claim is limited to the abstract.

## Executive summary

1. **Coverage-based "redundant test" removal is a lossy proxy on real faults.** On 1,478 real failed CI builds from 32 Java projects, a reduced suite missed at least one fault the full suite detected in up to 52.2% of failed builds under the most pessimistic fault mapping, and the usual TSR metrics (size reduction, requirements loss) did not predict that loss (Shi 2018, VERIFIED). Statement-coverage reduction cut suites by 62.9% on average yet lost up to 20.5% of killed mutants; mutant-based reduction lost none but kept 11.9 percentage points more tests (Shi 2014, VERIFIED). The same body of work also says a large share of tests never fail: an oracle that kept only future-failing tests would be ~20% of the original size (Shi 2018, VERIFIED). So the "delete a quarter with little coverage change" anecdote is plausible as a size claim and a weak guarantee as a fault-detection claim.
2. **What developers actually delete is mostly not "redundant".** In 24,431 manually confirmed deleted tests from 7 Java projects, 91.4% were obsolete (production code removed), 7% redundant (passing, production code still present), 1.6% failing; 20% of the redundant ones still reduced coverage or mutation score; one TSR family found at most 54% of the redundant ones (Bhatta et al. MSR 2025, VERIFIED). That is the closest thing to ground truth for "tests humans decided were low-value", and it is small and project-skewed (92% of deleted redundant tests came from one project, VERIFIED in the paper text).
3. **Static assertion/smell signals are cheap but weakly validated, and the smell detectors themselves are shaky.** Panichella et al. (EMSE 2022, VERIFIED) found the older detector misclassified over 70% of smells in generated tests and that several smells were ubiquitous in developer tests yet virtually never correlated with semantic or maintainability flaws. Assertion count correlates with suite effectiveness (Zhang & Mesbah 2015, abstract VERIFIED), but that is a suite-level correlation, not a per-test removability predictor.
4. **No paper I found evaluates an LLM as a judge of redundant or low-value unit tests against mechanical signals.** The nearest items are embedding-similarity minimization (LTM, VERIFIED), LLM smell detection (VERIFIED, small), and an LLM-as-judge for Gherkin acceptance-test coverage (VERIFIED, different artifact). The practitioner write-ups I found are open-source PRs/issues, and none is an LLM-driven deletion with measured outcomes. This is a real gap, and it means the anecdote has no peer-reviewed counterpart.

## 1. Test-suite minimization / reduction / prioritization

### 1.1 Definitions and the classic algorithms

- **Harrold-Gupta-Soffa (HGS, 1993)** selects a representative subset of tests giving the same coverage of a set of testing requirements by identifying and eliminating "redundant and obsolete" tests; needs only a requirement-to-test association. Abstract VERIFIED via OpenAlex record. Yoo & Harman's survey places HGS as a heuristic for the minimal hitting set problem with worst-case O(|T| * max|Ti|), and notes GE/GRE as greedy variants (VERIFIED in the survey text, 2007 preprint).
- **Redundancy definition used by the Illinois line of work**: a test t in suite T is redundant if it satisfies only requirements already satisfied by T \ {t} (Shi 2014, VERIFIED). Note this is relative to a chosen requirement set (statements, branches, killed mutants), which is exactly where false positives come from.
- **Yoo & Harman survey** (STVR 2012): minimization removes redundant tests, selection picks tests relevant to a change, prioritization orders for early fault detection. The copy I read is the 2007 preprint (DOI placeholder in its header), not the 2012 published version; secondary sources say the published version covers 159 papers, UNVERIFIED.

### 1.2 How much fault detection does reduction lose?

| Study | Setup | Finding | Status |
|---|---|---|---|
| Wong, Horgan, London, Mathur, ICSE 1995 | Test set minimization, seeded faults | Per Yoo & Harman, effect on fault detection was "insignificant" in the WHLM study | Claim VERIFIED via the survey text; the original paper's abstract was not retrievable (OpenAlex returned only a page header) |
| Rothermel, Harrold, Ostrin, Hong, ICSM 1998 | Siemens programs, 1,000 suites, HGS | "In contrast to the previous study, our results reveal that the fault detection capabilities of test suites can be severely compromised by minimization" (abstract VERIFIED). Survey adds: >50% loss for more than half of 1,000 suites, some 100% (VERIFIED in survey text) | VERIFIED |
| Rothermel et al., extended (STVR 2002) as described by Yoo & Harman | `space` program | Average fault-detection loss 8.9% for HGS reduction vs 18.1% for random reduction of the same size; HGS better than random | VERIFIED via survey text, not the original paper |
| Shi, Gyori, Gligoric, Zaytsev, Marinov, FSE 2014 | 18 projects, 261,235 tests, 3,590 commits, mutants as the oracle | Statement-coverage reduction: size cut 62.9% on average, lose up to 20.5% of killed mutants. Mutant-based reduction: no loss in killed mutants, 11.9 pp larger suites. Also proposes "inadequate reduction" (keep 95% of requirements) and evolution-aware metrics | VERIFIED (PDF) |
| Shi, Gyori, Mahmood, Zhao, Marinov, ISSTA 2018 | 1,478 failed builds, 32 GitHub/Travis Java projects, real failures | See below | VERIFIED (PDF) |
| Papadakis, Shin, Yoo, Bae, ICSE 2018 (context) | CoreBench, Defects4J | Correlation between mutation score and real fault detection is weak once suite size is controlled; "relative low prediction power" | VERIFIED (abstract); part A territory but it bounds how far "mutants killed" can be trusted as the ground truth for deletion |

**Shi et al. ISSTA 2018 numbers** (all VERIFIED from the PDF; I did not confirm which rows include or exclude "NewOnly" builds, so treat per-technique averages as the paper's Table 3 values):

- Failed-Build Detection Loss (FBDL) "can be up to 52.2%", the "most pessimistic" mapping (each failure is a unique fault). Table 3 averages for coverage-based techniques: Greedy 26.1% / 27.4% / 28.9% / 52.2% across the four mappings (S/P/C/U); mutant-based Greedy 13.1% / 14.7% / 15.5% / 36.2%.
- Average size reduction: 51.9% (coverage-based) and 61.1% (mutant-based).
- "Traditional TSR metrics are not good predictors of FBDL"; the paper's own proposed predictor, historical FBDL, is better but "still not a good predictor".
- "A large percentage of tests never failed": an oracular reduction keeping only tests that fail in future builds would be ~20% of the original suite on average and miss no regression fault.

**What it means for kibitzer (INFERRED)**: a flagged-redundant set from coverage or mutants is a hypothesis about future fault detection, and the best published evidence says its error rate on real failures is high and not predictable from the reduction metrics alone. Any report should present it as a ranked shortlist with its measured loss on the repo's own history, not a delete list.

### 1.3 What do reductions remove?

- Coverage/mutant-based reduction removes whichever tests have no unique requirement under the chosen criterion; Shi 2014 shows the criterion matters: killed-mutant requirements preserve mutant kills fully but retain more tests (VERIFIED).
- Test-suite reduction interacts badly with regression-test selection: in a C-program study, TSR "has almost always a negative impact on the effectiveness of RTS" while helping efficiency (arXiv 2207.12733, abstract VERIFIED; authors not recorded).
- Black-box/similarity reduction exists: ATM and FAST-R work from test source only; LTM (Pan, Ghaleb, Briand, TSE 2024) embeds test code with pretrained models and uses a genetic algorithm, reporting fault detection rate 0.84 vs 0.81 for ATM and ~5x faster minimization (arXiv abstract VERIFIED). Its fault detection is measured with mutants on Java projects (INFERRED from the abstract's context; I did not read the full paper's setup).

### 1.4 Bugs in test code (Vahabzadeh et al.)

Vahabzadeh, Milani Fard, Mesbah (ICSME 2015, PDF VERIFIED): mined 211 ASF projects, 5,556 test-related bug reports, manually categorized 443. Around half of the projects had bugs in test code; 97% of reported/fixed test bugs were false alarms and 3% "silent horrors" (test passes while production code is wrong); among silent horrors, assertion-related bugs were 67% (incorrect or missing assertions). Dominant false-alarm causes: semantic 25%, flaky 21%, environment 18%, resource handling 14%, obsolete tests 14%. Also: FindBugs/PMD-style tools were largely ineffective at detecting test bugs.

Implication (INFERRED): this supports "missing or wrong assertion" as a real defect class in tests, but it counts only bugs that someone reported and fixed, so it cannot give a prevalence of ineffective tests.

## 2. Redundancy detection: definitions, signals, and industrial evidence

### 2.1 Signal catalog

| Signal | Input | Cost | Validation status | Main false-positive modes |
|---|---|---|---|---|
| Coverage-equivalence redundancy (test's covered set is a subset of the rest; HGS/greedy) | Per-test coverage (statement/branch) | One instrumented run per test or per-test coverage tooling; then set-cover (cheap) | Validated as lossy: Rothermel 1998, Shi 2014, Shi 2018 (above). Koochakzadeh & Garousi (2010) state coverage-based redundancy "may suffer from large number of false-positive errors" (abstract VERIFIED) | Tests asserting different things over the same lines; regression pins; failure-message tests; coverage granularity too coarse |
| Mutant-kill-based redundancy (test kills no mutant that another test does not) | Kill matrix (test x mutant) | Highest: full mutation run with per-test kill info (Shi 2018 notes PIT failed to produce results for 5 projects and some reduction points, VERIFIED) | Better loss behavior than coverage in Shi 2014/2018 (lower FBDL) but still 13-36% FBDL; mutants are an imperfect proxy (Papadakis 2018) | Equivalent mutants; trivial mutants over-credit; subsumed mutants; mutation operator set bias |
| Similarity of test source (ATM, FAST-R, LTM embeddings) | Test code only | Low; no execution | Evaluated against mutants/fault detection on benchmark Java suites (LTM abstract VERIFIED). FAST-R found up to 54% of developer-deleted redundant tests (MSR 2025, VERIFIED) | Textually similar tests that differ in a decisive input/assertion; parameterized-test boilerplate |
| Never-failed-in-history | Run history/CI logs | Low if logs exist | Shi 2018: ~20% of suite would be the oracle set that ever fails in future builds, i.e. most tests never fail in the window (VERIFIED); but the dotnet/android audit explicitly says never-failed is "an investigation signal, not sufficient justification for removal" (maintainer issue, VERIFIED as their stated rule) | Tests guarding rare regressions; short windows |
| Developer-deletion ground truth (DelTest) | Git history + test runs | High to build | Dataset exists: 24,431 confirmed deleted tests (MSR 2025, VERIFIED) | Skewed to few projects; "redundant" = passing and production code still present, a loose definition |

### 2.2 Koochakzadeh & Garousi (2010)

Abstract (VERIFIED via OpenAlex/Semantic Scholar): coverage-based redundancy is widely used but "may suffer from large number of false-positive errors"; they propose a semi-automated methodology with manual interactive inspection and evaluate with mutation analysis on one real Java project, reporting the reduced suite keeps the same fault detection ability. One project, tester in the loop: weak external validity (INFERRED).

### 2.3 Palomba et al. "test-suite redundancy" and the Google/Meta/Microsoft question

- I did **not** find a Palomba et al. paper defining test-suite redundancy; the one Palomba result I opened is on code smells, not tests (Palomba et al., EMSE 2017, abstract VERIFIED via Semantic Scholar: 395 releases of 30 projects, smelly classes more change- and fault-prone). Treat the attribution in the task brief as UNVERIFIED/not found.
- **Industrial reports on deleting low-value tests by Google, Meta, Microsoft, Atlassian: none found.** A targeted search returned only open-source repository PRs/issues and a CI vendor blog. Google's flakiness blog (below) is about handling, not deleting. This is a named gap.
- What I did find, labeled by source quality (practitioner, not peer reviewed):
  - **dotnet/android epic #13012** (opened 2026-10-06; GitHub API record VERIFIED): "A test is low-value when its unique protection against meaningful regressions is too small relative to its cost"; decision question is "If this breaks, which retained tests detect it reliably, promptly and clearly enough?"; "There is no deletion quota"; "A test that has never failed is an investigation signal, not sufficient justification for removal"; "Weak assertions often need repair before their tests can be shrunk"; prioritizes expensive fixture/build/device tests over cheap unit cases; wants a mutation check before claiming equivalence when merging cases. It states it "does not establish lifetime failure histories" and performs no mutation proof for every assertion. I searched the body for LLM/Copilot/Claude/agent mentions and found none, so I cannot say it was LLM-assisted.
  - **ClickHouse PR #122485** (merged, 2026-09-26; GitHub API record VERIFIED): removed 111 integration test files judged low-value (rare corner cases, tests of the harness or Web UI JS, malformed-config handling, "only assert that a metric, log line or setting exists", failpoint-driven race reproductions); estimated 5.4% of integration test time, ~$4,900/week, ~2.2% of whole CI cost. The justification is cost-based, estimated from CI duration data and AWS prices; there is no coverage or mutation delta in the PR body I read. A later follow-up PR (#123165) exists with "stricter selection" (title seen in search results, not opened, UNVERIFIED). A search summary also claimed a reviewer pushed back on lost regression coverage; I did not open that thread (UNVERIFIED).
  - These two are the best available analogue to the anecdote, and both are **human-curated, cost-driven, with explicit rules** rather than a pure LLM verdict.

## 3. Assertion quality and the oracle problem

| Signal | Input | Cost | Validation status | False-positive modes |
|---|---|---|---|---|
| Assertion count / density per test or file | Static source | Trivial, hook-friendly | Suite-level: Zhang & Mesbah FSE 2015: number of assertions "strongly correlates" with suite effectiveness, 6,700 suites, 24,000 assertions, 5 Java projects (abstract VERIFIED). Kudrjavets, Nagappan, Ball (Microsoft TR 2006): higher assertion density in a file associated with statistically significant lower fault density, 2 components (abstract VERIFIED), but that is production-code assertions, not test assertions. Assert-use replication: "only limited evidence" for fewer defects in methods containing asserts (arXiv 2104.01514, abstract VERIFIED) | Correlation with suite size is a confound Zhang & Mesbah explicitly work against; parameterized/helper assertions; assertion-free tests that are legitimate (e.g., "does not throw", golden-file helpers) |
| No-assertion ("assertion-free") test | Static | Trivial | Plausible; I found no paper that validates "assertion-free test is removable"; tsDetect has smell types for related patterns, but I only verified its abstract (19 smell types, 96% precision / 97% recall on a 65-file benchmark; see §4) | Frameworks with implicit assertions (expected-exception annotations, mock verification, snapshot tests, fuzz/property harnesses) |
| Tautological assertions (assertTrue(true), x == x) | Static | Trivial | No empirical validation found (named gap). Related: GEM and MutGen papers describe weak assertions in LLM tests and cases of 100% coverage with 4% mutation score (search-summary quotes, not opened; UNVERIFIED) | Low frequency; likely high precision when it fires but low recall (INFERRED) |
| Checked coverage (dynamic slice of covered statements that influence an oracle) | Dynamic slicing of a test run | High: needs slicing instrumentation | Schuler & Zeller ICST 2011 / STVR 2013: seven open-source projects, "a sure indicator for oracle quality and even more sensitive than mutation testing" (abstract VERIFIED) | Tooling is Java-specific (INFERRED); slicing cost; not designed as a per-test deletion signal |
| Pseudo-tested methods (covered, but no test fails when body removed) | Extreme mutation run | Medium | Niedermayr et al. CSED 2016: ratio acceptable for unit tests, not system tests (abstract VERIFIED); Vera-Pérez et al. EMSE 2019: exist in all subjects, 28K+ methods, pseudo-tested methods "significantly less tested" (abstract VERIFIED). Part A's remit; it is a *method*-level signal, not a test-level one (INFERRED) | Methods whose effects are hard to observe; developers often would not pay to fix (Vera-Pérez, VERIFIED) |

**Oracle problem survey** (Barr, Harman, McMinn, Shahbaz, Yoo, TSE 2015; abstract VERIFIED): the oracle problem is distinguishing correct from incorrect behavior; survey of modelling, specifications, contract-driven development, metamorphic testing, with the human as the final oracle source. It frames *why* a mechanical signal cannot decide a test's value in general: the correct expected value is not in the code (INFERRED).

**Gaps**: I found no study that validates a *static per-test assertion-quality score* (weak-assertion, tautology, over-mocking) as a predictor of "this test can be deleted without losing fault detection". Hora & Robbes (arXiv 2602.00409, abstract VERIFIED) show agents add mocks to tests more often than non-agents (36% vs 26% of commits) and call mock-heavy tests "less effective at validating real interactions", but that is a prevalence/risk statement, not a validated effectiveness predictor.

## 4. Test smells and their validity

| Signal | Input | Cost | Validation status | False-positive modes |
|---|---|---|---|---|
| Rule-based test-smell detectors (tsDetect, JNose, Grano et al. tool) | Static (AST) | Low, hook-friendly | tsDetect authors: precision 96%, recall 97% on a 65-file benchmark with 19 smell types (Peruma et al. FSE 2020 demo, abstract VERIFIED). Independent assessment: Panichella et al. 2022 (below) finds much worse accuracy outside that setting | Eager Test and Assertion Roulette depend on thresholds; Mystery Guest and indirect testing hard statically; mocked file access |
| Specific smells (Assertion Roulette, Eager Test, Indirect Testing) as change/defect-proneness predictors | Static | Low | Spadini et al. ICSME 2018: 221 releases of 10 systems, >1M test cases, 6 smells; smelly tests more change- and defect-prone; production code tested by smelly tests more defect-prone (abstract VERIFIED). Association, not a removability signal | Confounded by test size/complexity (INFERRED; I did not check their controls) |
| Catalog definitions | n/a | n/a | van Deursen, Moonen, van den Bergh, Kok (CWI 2001) introduced the test-smell catalog; abstract VERIFIED via OpenAlex | n/a |

**Developer perception**: Tufano, Palomba, Bavota, Di Penta (ASE 2016, abstract VERIFIED): in a 19-participant study "developers generally do not recognize (potentially harmful) test smells". Spadini et al. (MSR 2020, abstract VERIFIED): "current detection rules for certain test smells are considered as too strict by the developers"; they derive severity thresholds in line with developer perception.

**Panichella, Panichella, Fraser, Sawant, Hellendoorn, EMSE 27(7):170, 2022** (PDF VERIFIED): built a hand-labeled dataset spanning hundreds of suites (developer-written plus EvoSuite and JTExpert generated), six smell types, cross-validated manual analysis; benchmarked two detectors. Findings: "the current vocabulary of test smells is highly mismatched to real concerns: multiple smells were ubiquitous on developer-written tests but virtually never correlated with semantic or maintainability flaws"; the older tool "misclassified over 70% of test smells, both missing real instances (false negatives) and marking many smell-free tests as smelly (false positives)". In the text: the older tool reports 100% recall and 88% precision in its original paper, but in this evaluation low precision gives an F-measure of 0.36 on one smell; tsDetect reaches 67% precision on EvoSuite and 96% on JTExpert for Assertion Roulette, and 73% precision / 55% recall for Eager Test. Authors recommend evaluating detection strategies "primarily in industrial contexts".

**Answer to "are smell detectors valid?" (VERIFIED + INFERRED)**: valid as *maintainability hints with known thresholds*, not as a signal of fault-detection effectiveness. For kibitzer's purpose (flag tests that do not earn their keep), smells such as Assertion Roulette or Magic Number are orthogonal; the smells that touch effectiveness (Empty Test, Unknown Test/no assertion, Redundant Assertion) are the ones with the least independent validation (INFERRED; I only have tsDetect's abstract and the Panichella text, not a per-smell validity table for those).

**Garousi & Küçük (JSS 2018) survey of smells in test code**: bibliographic record VERIFIED (OpenAlex); abstract/content not retrieved, so I make no claim about its findings. **Bavota et al. "Are test smells really harmful?"**: not opened (search failed); UNVERIFIED.

**Smells in LLM-generated tests**: arXiv 2410.10628 (abstract VERIFIED): 20,505 class-level suites from four LLMs, 972 method-level cases, 14,469 EvoSuite tests, 779,585 human-written tests; TsDetect and JNose; LLM tests "consistently manifest smells such as Assertion Roulette and Magic Number Test". arXiv 2407.00225 (abstract VERIFIED, 216,300 test cases): recurring Magic Number Test and Assertion Roulette; compilation failure up to 86%; hallucination-driven failures persist.

## 5. Flaky tests as noise for per-test signals

- **Luo, Hariri, Eloussi, Marinov, FSE 2014** (PDF VERIFIED): 201 commits fixing flaky tests in 51 projects; top three root-cause categories are Async Wait, Concurrency, Test Order Dependency. Cites Google TAP data: 1.6M test failures per day, 73K (4.56%) from flaky tests; Google reruns a failing test 10 times and labels it flaky if any rerun passes. Also notes flaky tests "may also hide real bugs".
- **Google Testing Blog, Micco 2016** (page VERIFIED): "about 1.5% of all test runs reporting a flaky result"; "almost 16% of our tests have some level of flakiness"; "84% of the transitions we observe from pass to fail involve a flaky test".
- **Parry, Kapfhammer, Hilton, McMinn, TOSEM 31(1), 2022**: repository record VERIFIED (published 2022-01-31, DOI 10.1145/3476105). Survey scope (76 papers; causes, costs, detection, mitigation) is from a search summary and UNVERIFIED; I did not open the PDF (download failed).
- **Vahabzadeh**: flaky = 21% of false-alarm test-bug root causes (VERIFIED, §1.4).
- **MSR 2025 deletion study**: authors reran each deleted redundant test three times and found no flaky tests among them (VERIFIED in text), but raise flakiness as a possible reason developers delete tests.
- **Effects on per-test signals (INFERRED, not found in any source I opened)**: a flaky test's pass/fail-based attribution (kill matrices, history signals like never-failed or FBDL) is noisy; per-test coverage varies with nondeterminism, so "unique coverage" can flip between runs; order-dependent tests invalidate per-test isolated coverage. Mitigation to propose: require N reruns, report a stability flag, and exclude tests whose per-test coverage differs between runs from deletion candidacy. I found no paper quantifying how much flakiness perturbs kill-matrix based reduction; that is a gap.

## 6. LLM-as-judge / LLM-assisted test quality and pruning (2023-2026)

| Item | What it does | Evidence | Relevance / status |
|---|---|---|---|
| LTM (Pan, Ghaleb, Briand; TSE 2024; arXiv 2304.01397) | Embeds test method source with pretrained code models, cosine similarity, GA picks the minimized suite | Abstract VERIFIED: 0.84 vs 0.81 fault detection rate vs ATM; ~5x faster; "first application of LLMs in the context of TSM" | An embedding model, not an LLM judge; no assertion/semantic reasoning. Useful as a cheap static pre-filter (INFERRED) |
| Santana et al. 2025 (arXiv 2506.07594) | GPT-4-Turbo, LLaMA 3 70B, Gemini-1.5 Pro on Python/Java; PyNose/TsDetect for initial smell detection, then LLM refactoring | Abstract VERIFIED: best detection accuracy 74.35% Python, 80.32% Java (Gemini); refactoring sometimes introduces new smells; GPT-4 and LLaMA often reduced coverage | Small; shows LLM refactoring of tests can *lower coverage* |
| Lucas et al. SBES 2024 (arXiv 2407.19261) | ChatGPT-4, Gemini Advanced, Mistral Large detect 30 smell types across 7 languages | Abstract VERIFIED: 21, 17, 15 types detected respectively | Type-coverage counts only; no precision/recall in the abstract |
| LLM-as-a-Judge for Gherkin acceptance tests (arXiv 2512.01232, AAAI 2026 workshop) | Rubric-driven scoring of acceptance-test scripts for requirements coverage | Abstract VERIFIED: 100 expert-annotated scripts, 20 model configs, reliability 85.4% to 100% first-attempt success, cost spans 175x | Different artifact (acceptance scripts vs unit tests) but the only evaluated "LLM judges test quality" setup I found with accuracy and cost numbers |
| LLM-as-a-Judge for SE roadmap (arXiv 2510.24367) | Literature review and roadmap | Abstract VERIFIED: "LLM-as-a-Judge research in SE is still in its early stages" | Confirms the gap |
| Meta TestGen-LLM (Alshahwan et al., FSE 2024 industry; arXiv 2402.09171) | LLM generates additional tests, filters: builds, passes reliably, increases coverage | Abstract VERIFIED: 75% build, 57% pass reliably, 25% increase coverage; 73% of recommendations accepted by engineers in test-a-thons | Not pruning, but it is an industrial precedent that *mechanical filters gate LLM output*, the same shape as "mechanical shortlist, then LLM adjudication" (INFERRED) |
| Oracle generation by LLM (arXiv 2601.05542, AIware 2025) | Empirical study of LLMs generating test oracles that expose failures | Abstract VERIFIED; results not stated in the abstract | Related to assertion quality, no findings I can quote |
| Agent test behavior (Hora & Robbes, arXiv 2602.00409) | 1.2M commits in 2,168 repos, agent vs non-agent | Abstract VERIFIED: agents more likely to modify tests (23% vs 13% of commits) and add mocks (36% vs 26%) | Context: agents also *create* low-value tests |

**What I did not find** (honest gaps):

1. No peer-reviewed study that asks an LLM to label individual existing unit tests as redundant/low-value and then scores the labels against coverage deltas, mutant kills, or historical fault detection.
2. No paper comparing LLM judgment with mechanical signals for deletion candidates.
3. No LLM-driven "delete a quarter of tests" write-up with measured outcomes. My search for the anecdote returned only an MSR 2025 paper and community "skills" files (test-sweep, test-cleanup) that gate deletions with coverage/mutation checks. Those skill files are low-quality sources (marketplace prompt text) and I did not open them; one search summary says one caps removal at 30% of tests, UNVERIFIED. The anecdote in requirements.md remains unsourced.

## 7. Signal summary for the candidate-signal catalog

| # | Signal | Input | Cost | Evidence strength | Fits kibitzer advisory/diff-aware model? |
|---|---|---|---|---|---|
| S1 | No assertion / tautological assertion in a test | Static | Trivial | Plausible, weakly validated (Vahabzadeh: missing/incorrect assertions dominate silent horrors, VERIFIED, but no deletion-validity study) | Yes, hook-time; measure precision on corpus |
| S2 | Assertion density per test, mock-only verification | Static | Trivial | Suite-level correlation only (Zhang & Mesbah); over-mock prevalence data (Hora & Robbes) | Yes, as a ranked hint, not a verdict |
| S3 | Test-smell rules (tsDetect-like) | Static | Low | Detectors unreliable outside original setting; smells weakly tied to semantic flaws (Panichella 2022) | Optional, low priority; risk of noise |
| S4 | Textual/embedding similarity between tests | Static | Low | Evaluated on mutant-based fault detection (LTM); recovers up to 54% of developer-deleted redundant tests (FAST-R, MSR 2025) | Yes, cheap batch report |
| S5 | Unique coverage per test (incremental coverage; redundancy by subset) | Per-test coverage | Medium; per-test attribution is the main cost | Lossy on real faults (Shi 2018 FBDL up to 52.2%); known false positives (Koochakzadeh & Garousi) | Batch/CI report, ingest-only; fits the owner's named signal #1 |
| S6 | Unique mutants killed per test (kill-matrix redundancy) | Kill matrix | High | Best of the reduction criteria (lower FBDL, Shi 2018: 13.1% vs 26.1% Greedy averages in Table 3), still not safe | Batch/sampled; fits owner's named signal #2 |
| S7 | Never failed in history | CI/run logs | Low where logs exist | Used as an oracle upper bound (~20% of suite, Shi 2018); maintainers treat as investigation signal only | Batch, repo-specific |
| S8 | Checked coverage | Dynamic slicing | High | Abstract-level validation on 7 projects | Research/optional |
| S9 | Flakiness/stability flag on each test | Repeated runs or history | Medium | Strong background evidence (Luo, Google, Vahabzadeh) | Required guard for S5-S7 (INFERRED) |

Validation-status key for catalog consumers: "plausible, weakly validated" means no source I opened tests the signal against deletion safety; "lossy" means evidence shows measurable fault-detection loss.

## 8. Findings that bear on the requirements' open questions

- **OQ2 (best validated predictor of "removable without losing fault detection")**: the best-measured criterion in what I read is killed-mutant redundancy, with FBDL still 13-36% on real failures (Shi 2018) and no loss on the mutant oracle itself (Shi 2014). Nothing is validated as *safe*. Historical failure data (S7) is the only signal the Illinois work found that improves prediction, and it is "still not a good predictor".
- **OQ3 (unique coverage vs unique mutants agree?)**: no source I opened measures the agreement per test; Shi 2018 reports the coverage-based and mutant-based reduced suites have 0.87 to 0.94 correlation in how builds are classified across algorithm pairs within a criterion (VERIFIED from PDF line, but this is across algorithms, not across criteria). The cross-criterion question stays open and is directly testable on the corpus.
- **OQ5 (static signals precise enough for hook-time default)**: the only independent accuracy evidence I opened is Panichella 2022, which is negative for smell rules and says nothing about no-assertion/tautology detection. Treat S1 as unvalidated until measured on the corpus and transcripts.
- **OQ6 (how to validate deletions)**: Shi 2018 argues for historical real failures rather than seeded faults; MSR 2025 uses coverage and mutation score deltas on developer-confirmed deletions (and DelTest is a reusable labeled set, INFERRED about reuse; I did not check the dataset's license or availability). Use both, since mutants are a weak proxy for real faults (Papadakis 2018).

## 9. Named gaps and unverified items

- No industrial (Google/Meta/Microsoft/Atlassian) report on deleting low-value tests found.
- No LLM-as-judge-on-unit-test-redundancy evaluation found; no LLM-vs-mechanical comparison.
- Palomba "test-suite redundancy" paper not located; only a code-smells paper by Palomba was opened.
- Not opened: Parry et al. PDF, Garousi & Küçük content, Bavota et al. test-smell harmfulness paper, original Wong 1995 and Rothermel 2002 papers (their findings are relayed from Yoo & Harman), Yoo & Harman 2012 published version (I read the 2007 preprint), LTM full text (abstract only), Spadini ICSME 2018 full text (abstract only), Kudrjavets full text (abstract only).
- Shi 2018 Table 3: I did not verify whether per-technique averages include or exclude NewOnly builds.
- The brave search tool failed (invalid subscription token) and the stapler-mcp daemon later refused connections; I used WebSearch plus direct downloads and the OpenAlex / Semantic Scholar APIs, which restricts me to abstracts for several items.
- `chekam.txt` and `just.txt` files exist in the shared scratchpad `pdf` folder from another worker; I did not use them.

## Bibliography

| # | Authors | Year | Venue | URL | Contribution | Verified? |
|---|---|---|---|---|---|---|
| 1 | August Shi, Alex Gyori, Suleman Mahmood, Peiyuan Zhao, Darko Marinov | 2018 | ISSTA | https://mir.cs.illinois.edu/gyori/pubs/issta18.pdf (DOI 10.1145/3213846.3213875) | TSR on 1,478 real failed builds; FBDL up to 52.2%; metrics poor predictors; ~20% oracle suite | VERIFIED (PDF) |
| 2 | August Shi, Alex Gyori, Milos Gligoric, Andrey Zaytsev, Darko Marinov | 2014 | FSE | https://mir.cs.illinois.edu/gyori/pubs/fse14reduction.pdf | Mutant-based and inadequate reduction; 62.9% size cut loses up to 20.5% killed mutants | VERIFIED (PDF) |
| 3 | Shin Yoo, Mark Harman | 2012 (preprint 2007 read) | STVR | https://www.cse.chalmers.se/~feldt/advice/yoo_2010_regression_testing_survey.pdf | Minimization/selection/prioritization survey; relays Wong and Rothermel findings | VERIFIED (preprint) |
| 4 | Mary Jean Harrold, Rajiv Gupta, Mary Lou Soffa | 1993 | TOSEM | https://doi.org/10.1145/152388.152391 | HGS heuristic for representative test set | VERIFIED (abstract via OpenAlex) |
| 5 | Gregg Rothermel, Mary Jean Harrold, Jeffery Ostrin, Christie Hong | 1998 | ICSM | https://doi.org/10.1109/icsm.1998.738487 | Minimization can severely compromise fault detection | VERIFIED (abstract); numbers via #3 |
| 6 | W. Eric Wong, Joseph Horgan, Saul London, Aditya Mathur | 1995 | ICSE | https://doi.org/10.1145/225014.225018 | Early minimization effect study | Metadata only; finding via #3 |
| 7 | Arash Vahabzadeh, Amin Milani Fard, Ali Mesbah | 2015 | ICSME | https://people.ece.ubc.ca/amesbah/resources/papers/icsme15.pdf | Bugs in test code; 97% false alarms, 3% silent horrors | VERIFIED (PDF) |
| 8 | Negar Koochakzadeh, Vahid Garousi | 2010 | Advances in Software Engineering | https://doi.org/10.1155/2010/932686 | Coverage-based redundancy false positives; tester-assisted reduction | VERIFIED (abstract) |
| 9 | Suraj Bhatta, Frank Kendemah, Ajay Jha | 2025 | MSR | https://2025.msrconf.org/details/msr-2025-technical-papers/24/Understanding-Test-Deletion-in-Java-Applications ; preprint https://hifromajay.github.io/papers/msr25.pdf | DelTest: 24,431 deleted tests; 91.4% obsolete, 7% redundant | VERIFIED (page + preprint PDF) |
| 10 | Rongqi Pan, Taher Ghaleb, Lionel Briand | 2024 | IEEE TSE | https://arxiv.org/abs/2304.01397 | LTM: embedding-based black-box minimization | VERIFIED (abstract) |
| 11 | Gunnar Kudrjavets, Nachiappan Nagappan, Thomas Ball | 2006 | MSR-TR-2006-54 | https://www.microsoft.com/en-us/research/publication/assessing-the-relationship-between-software-assertions-and-code-qualityan-empirical-investigation/ | Assertion density vs fault density at Microsoft | VERIFIED (abstract) |
| 12 | (authors not recorded; MacDonell group page) | 2017 | ISSREW | https://arxiv.org/abs/2104.01514 | Assert use and defectiveness in two industrial Java systems; limited support for earlier finding | VERIFIED (abstract); author list UNVERIFIED |
| 13 | Yucheng Zhang, Ali Mesbah | 2015 | ESEC/FSE | https://doi.org/10.1145/2786805.2786858 | Assertion count correlates with suite effectiveness | VERIFIED (abstract via OpenAlex) |
| 14 | David Schuler, Andreas Zeller | 2011/2013 | ICST / STVR | https://doi.org/10.1109/ICST.2011.32 ; https://doi.org/10.1002/stvr.1497 | Checked coverage as oracle-quality indicator | VERIFIED (abstract via OpenAlex) |
| 15 | Earl Barr, Mark Harman, Phil McMinn, Muzammil Shahbaz, Shin Yoo | 2015 | IEEE TSE | https://discovery.ucl.ac.uk/1471263/ | Oracle problem survey | VERIFIED (abstract via OpenAlex; UCL page opened) |
| 16 | Rainer Niedermayr, Elmar Juergens, Stefan Wagner | 2016 | CSED | https://arxiv.org/abs/1611.07163 | Pseudo-tested methods via extreme mutation | VERIFIED (abstract) |
| 17 | Oscar Vera-Pérez, Benjamin Danglot, Martin Monperrus, Benoit Baudry | 2019 | EMSE | https://arxiv.org/abs/1807.05030 | Pseudo-tested methods replication, 28K+ methods | VERIFIED (abstract) |
| 18 | Mike Papadakis, Donghwan Shin, Shin Yoo, Doo-Hwan Bae | 2018 | ICSE | https://doi.org/10.1145/3180155.3180183 | Mutation score vs real faults weak when controlling size | VERIFIED (abstract via OpenAlex) |
| 19 | Laura Inozemtseva, Reid Holmes | 2014 | ICSE | https://doi.org/10.1145/2568225.2568271 | Coverage not strongly correlated with effectiveness (part A) | VERIFIED (abstract via OpenAlex, partial) |
| 20 | Annibale Panichella, Sebastiano Panichella, Gordon Fraser, Anand Sawant, Vincent Hellendoorn | 2022 | EMSE 27(7):170 | https://pure.tudelft.nl/ws/portalfiles/portal/137994226/s10664_022_10207_5.pdf | Test-smell detector accuracy and validity; >70% misclassification for older tool | VERIFIED (PDF) |
| 21 | Anthony Peruma, Khalid Almalki, Christian Newman, Mohamed Mkaouer | 2020 | ESEC/FSE (demo) | https://doi.org/10.1145/3368089.3417921 | tsDetect, 19 smell types, 96% / 97% on benchmark | VERIFIED (abstract) |
| 22 | Davide Spadini, Fabio Palomba, Andy Zaidman, Magiel Bruntink, Alberto Bacchelli | 2018 | ICSME | https://research.tudelft.nl/en/publications/on-the-relation-of-test-smells-to-software-code-quality/ | Smelly tests more change/defect-prone | VERIFIED (abstract) |
| 23 | Davide Spadini, Martin Schvarcbacher, Ana-Maria Oprescu, Magiel Bruntink | 2020 | MSR | https://doi.org/10.1145/3379597.3387453 | Severity thresholds; detectors too strict per developers | VERIFIED (abstract via OpenAlex) |
| 24 | Michele Tufano, Fabio Palomba, Gabriele Bavota, Massimiliano Di Penta | 2016 | ASE | https://doi.org/10.1145/2970276.2970340 | Developers do not recognize test smells; nature of test smells | VERIFIED (abstract via OpenAlex) |
| 25 | Arie van Deursen, Leon Moonen, A. van den Bergh, Gerard Kok | 2001 | CWI report | OpenAlex record for "Refactoring test code" | Original test-smell catalog | VERIFIED (abstract via OpenAlex) |
| 26 | Vahid Garousi, Baris Küçük | 2018 | JSS | https://doi.org/10.1016/j.jss.2017.12.013 | Smells in test code survey | Metadata only; content UNVERIFIED |
| 27 | Fabio Palomba, Gabriele Bavota, Massimiliano Di Penta, Fausto Fasano, Rocco Oliveto | 2018 | EMSE | https://doi.org/10.1007/s10664-017-9535-z | Code-smell diffuseness/impact (not test smells) | VERIFIED (abstract via Semantic Scholar) |
| 28 | Qingzhou Luo, Farah Hariri, Lamyaa Eloussi, Darko Marinov | 2014 | FSE | https://huang.isis.vanderbilt.edu/cs8395/paper/flakytest.pdf | Flaky-test root causes; Google TAP flaky share | VERIFIED (PDF) |
| 29 | Owain Parry, Gregory Kapfhammer, Michael Hilton, Phil McMinn | 2022 | TOSEM 31(1) | https://eprints.whiterose.ac.uk/id/eprint/230095 | Survey of flaky tests | Record VERIFIED; content UNVERIFIED |
| 30 | John Micco | 2016 | Google Testing Blog | https://testing.googleblog.com/2016/05/flaky-tests-at-google-and-how-we.html | 1.5% of runs flaky; ~16% of tests; 84% pass-to-fail transitions flaky | VERIFIED (page) |
| 31 | Nadia Alshahwan, Jubin Chheda, Anastasia Finegenova, Beliz Gokkaya, Mark Harman, et al. | 2024 | FSE industry | https://arxiv.org/abs/2402.09171 | TestGen-LLM with mechanical filters | VERIFIED (abstract; author list from search summary) |
| 32 | E. G. Santana Jr. et al. | 2025 | arXiv | https://arxiv.org/abs/2506.07594 | LLM test smell detection and refactoring | VERIFIED (abstract) |
| 33 | Keila Lucas et al. | 2024 | SBES IIER | https://arxiv.org/abs/2407.19261 | ChatGPT-4/Gemini/Mistral detect test smells | VERIFIED (abstract) |
| 34 | (authors not recorded) | 2025 | arXiv / TSE | https://arxiv.org/abs/2410.10628 | Test smells in LLM-generated tests, 4 LLMs | VERIFIED (abstract); authors UNVERIFIED |
| 35 | (authors not recorded) | 2026 | EMSE | https://arxiv.org/abs/2407.00225 | LLM vs EvoSuite unit tests; smells; 216,300 tests | VERIFIED (abstract); authors UNVERIFIED |
| 36 | (authors not recorded) | 2025 | AAAI-26 workshop | https://arxiv.org/abs/2512.01232 | LLM-as-a-Judge for Gherkin acceptance tests | VERIFIED (abstract) |
| 37 | (authors not recorded) | 2025 | arXiv | https://arxiv.org/abs/2510.24367 | LLM-as-a-Judge for SE roadmap | VERIFIED (abstract) |
| 38 | (authors not recorded) | 2025 | AIware | https://arxiv.org/abs/2601.05542 | LLM-generated test oracles study | VERIFIED (abstract) |
| 39 | Hora, Robbes | 2026 | arXiv | https://arxiv.org/abs/2602.00409 | Agents modify tests and add mocks more often | VERIFIED (abstract); author names from search summary |
| 40 | (authors not recorded) | 2022 | arXiv | https://arxiv.org/abs/2207.12733 | Interaction of TSR and RTS (RegreTS framework) | VERIFIED (abstract); authors UNVERIFIED |
| 41 | simonrozsival (maintainer) | 2026 | GitHub issue, dotnet/android | https://github.com/dotnet/android/issues/13012 | Practitioner low-value test pruning rules | VERIFIED (GitHub API record); practitioner, not peer reviewed |
| 42 | alexey-milovidov | 2026 | GitHub PR, ClickHouse | https://github.com/ClickHouse/ClickHouse/pull/122485 | Removed 111 low-value integration test files; ~$4,900/week estimate | VERIFIED (GitHub API record); practitioner, not peer reviewed |
