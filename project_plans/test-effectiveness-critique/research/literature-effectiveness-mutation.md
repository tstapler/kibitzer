# Literature review A: test-suite effectiveness, coverage, mutation analysis

Scope: Phase 2 research for `test-effectiveness-critique`. Question: which mechanical signals of "this test is ineffective / removable" have validated support, at what cost, with what false-positive modes.

Labels: **VERIFIED** = I opened the source text/abstract this session (PDF text extracted, arXiv/ar5iv, publisher or repository abstract page). **SECONDARY** = stated by a verified source about another paper; I did not open the primary. **INFERRED / UNVERIFIED** = my reasoning or search-engine summary only.

Method note: Brave search was down (HTTP 422 invalid token) and several publisher pages (ACM, IEEE, ORBilu) timed out, so some primaries could not be opened. Those are labelled SECONDARY or UNVERIFIED rather than smoothed over. Papers that could not be reached are listed in "Gaps".

Correction to the task brief: "Code coverage and test suite effectiveness: empirical study with real bugs in large systems" is Kochhar, Thung & Lo (SANER 2015), not Gopinath et al. Gopinath, Jensen & Groce's ICSE 2014 paper is "Code coverage for suite evaluation by developers" (UNVERIFIED for the Kochhar title/authors; see bibliography; VERIFIED for Gopinath's actual title: [NAU page](https://experts.nau.edu/en/publications/code-coverage-for-suite-evaluation-by-developers/)).

---

## 1. Coverage vs. effectiveness

### 1.1 What the evidence says

| Claim | Source | Status |
|---|---|---|
| Across 31,000 suites over five Java systems (up to 724 KLOC), effectiveness correlates **moderately to very highly with the number of test methods**; coverage correlates moderately-to-highly with effectiveness when size is ignored; **after controlling for size the correlation is "low to moderate"**, and "it is not generally safe to assume that effectiveness is correlated with coverage". Stronger coverage types (decision, modified condition) added little over statement coverage. | Inozemtseva & Holmes, ICSE 2014: [paper PDF](https://cs.ubc.ca/~rtholmes/papers/icse_2014_inozemtseva.pdf); abstract + conclusions quoted in [neverworkintheory review](https://neverworkintheory.org/2021/09/24/coverage-is-not-strongly-correlated-with-test-suite-effectiveness.html); DOI [10.1145/2568225.2568271](https://doi.org/10.1145/2568225.2568271) | VERIFIED |
| Their design: PIT mutants; suites are **random subsets of the project's own test methods** (sizes 3, 10, 30, 100, ...; 1,000 suites per size); a mutant the full master suite cannot kill is **assumed equivalent**, which the authors note "tends to overestimate the number of equivalent mutants". Effectiveness is therefore *relative to what the master suite already catches*. | same PDF, Sections 3.1-3.4 | VERIFIED |
| Their conclusion on use: coverage "useful for identifying under-tested parts of a program" but "should not be used as a quality target". | abstract via neverworkintheory page | VERIFIED |
| Namin & Andrews (ISSTA 2009): coverage is *sometimes* correlated with effectiveness when size is controlled; size+coverage predicts better than size alone; relationship is nonlinear, not linear. Subjects were very small C programs. | [paper PDF](https://www.depts.ttu.edu/cs/research/avesta/documents/papers/ISSTA2009/ISSTA09.pdf) (abstract and intro read); "very small C programs, largest < 6,000 LOC" is Niedermayr's characterization in [arXiv 1611.07163](https://ar5iv.org/html/1611.07163) | VERIFIED |
| Gopinath et al. (ICSE 2014): across many open-source projects, **statement coverage** (not block, branch, path) best predicts mutant kills when evaluating a *single* suite (the practitioner's situation) rather than comparing suites. Note the outcome variable is *mutant kills*, not real faults. | [NAU abstract page](https://experts.nau.edu/en/publications/code-coverage-for-suite-evaluation-by-developers/) | VERIFIED (abstract only) |
| Ahmed et al. (FSE 2016, "Can testedness be effectively measured?"): both statement coverage and mutation score have only a **weak negative correlation with future bug-fixes** per program element; but elements covered by any test see about **half as many bug-fixes** as uncovered ones. | [NAU abstract page](https://experts.nau.edu/en/publications/can-testedness-be-effectively-measured/) | VERIFIED (abstract only) |
| Chekam et al. (ICSE 2017): fault revelation is strongly tied to attainment of **strong mutation** coverage and weak for statement/branch/weak mutation; revelation improves significantly only at **high** coverage levels; argues earlier studies relied on an unreliable "clean program" assumption. | Characterized in the survey table ([Papadakis et al. 2018 survey](https://mutationtesting.uni.lu/survey.pdf), Table 2) and in [Papadakis et al. ICSE 2018 preprint](https://coinse.github.io/publications/pdfs/Papadakis2018hi.pdf) Table 1 ("strong connection between mutation score increase and fault detection at higher score levels", 61 real faults, C) | SECONDARY (primary not opened) |
| Kochhar, Thung & Lo (SANER 2015): statistically significant correlation between coverage and bug-kill effectiveness on 67 (HttpClient) and 92 (Rhino) real bugs, strength differing per system; a later Kochhar et al. 2017 study of 100 Java projects found insignificant correlation between test-suite coverage and post-release bugs. | search-engine summary only; [IEEE page](https://ieeexplore.ieee.org/document/7081877) failed to open | UNVERIFIED |
| Hemmati (QRS 2015, "How effective are code coverage criteria?"): only the bibliographic entry was found (pp. 151-156). | none | UNVERIFIED |
| Survey roll-up: studies disagree; Gligoric et al. found branch coverage strongly correlated with mutation score; Inozemtseva found weak when size is controlled. | [Papadakis et al. survey](https://mutationtesting.uni.lu/survey.pdf), Section 8 and Table 2 | VERIFIED (as the survey's statement) |

### 1.2 What survives once size is controlled?

- **VERIFIED**: a low-to-moderate residual coverage effect (Inozemtseva), "sometimes correlated" (Namin & Andrews), weak (Papadakis 2018 for mutation score, below). Size alone is the stronger predictor in the Java studies.
- **Why this matters for kibitzer (INFERRED)**: in these designs, bigger suite => more effective by construction ("adding a test case makes a suite at least as effective", Namin & Andrews intro). Any "delete the redundant tests" claim needs a **size-matched random-deletion baseline**, otherwise a mechanical signal can look good merely by removing less than random would. No paper I read evaluated a deletion signal this way for coverage; Shi et al. (Section 3) compare reduction techniques, not against random same-size removal.

### 1.3 Is *incremental coverage per test* a validated signal?

- **Not directly validated as a per-test removability predictor. GAP.** No paper I could open evaluates "unique lines covered by test t" against real-fault detection of the suite without t.
- Nearest evidence (all VERIFIED, [Shi et al., FSE 2014](https://mir.cs.illinois.edu/gyori/pubs/fse14reduction.pdf)): reducing a suite by statement coverage (keep a subset that covers the same statements; redundant = adds no unique requirement) shrinks suites by 62.9% on average but **loses up to 20.5% of killed mutants**; a reduction that preserves killed mutants loses none, at +11.9 percentage points in size; adding statement coverage as a second requirement costs +2.7 pp more. 18 projects, 261,235 tests, 3,590 commits. So "unique statement coverage = zero" is a *cheap but lossy* redundancy test relative to the mutant yardstick.
- Real-failure evidence (VERIFIED, [Shi et al., ISSTA 2018](https://mir.cs.illinois.edu/gyori/pubs/issta18.pdf)): on 1,478 failed Travis builds from 32 GitHub projects, reduced suites miss the faults of up to **52.2%** of failed builds ("Failed-Build Detection Loss"), "higher than suggested by traditional TSR metrics", and **traditional metrics are not good predictors of FBDL**. This is the strongest warning against trusting coverage- or mutant-based redundancy as a proxy for real-fault loss.
- Practical corollary (VERIFIED from Ahmed et al. above): "covered by *any* test" has some signal (half the later bug-fixes); a marginal test's *incremental* coverage was not what was measured.

---

## 2. Mutation testing as the effectiveness proxy

### 2.1 Validity

| Claim | Source | Status |
|---|---|---|
| 357 real faults, 5 Java programs, 321 KLOC (230,000 mutants): statistically significant correlation between mutant detection and real-fault detection **independent of coverage**, stronger than statement coverage's. **73%** of real faults are coupled to commonly used mutants; 10% need new/stronger operators; **27% (95 of 357) are not coupled** (e.g., algorithmic/omission faults, wrong-variant fixes). Equivalent mutants were not eliminated, so mutation scores may be underestimated. | Just et al., FSE 2014: [PDF](https://homes.cs.washington.edu/~mernst/pubs/mutation-effectiveness-fse2014.pdf) | VERIFIED |
| Caveat: Just et al. did not control for test-suite size (per later work). Papadakis et al. (ICSE 2018; CoreBench C + Defects4J Java) find the correlation is largely a size confounder: **all correlations between mutation score and real-fault detection are weak when size is controlled**; both size and mutation score matter; top-ranked suites by mutation score beat same-size random suites; **fewer than 1% of mutants** are the ones actually representing real-fault behavior. Conclusion: mutants are good *guidance*, weak *predictors*. | [ICSE 2018 abstract](https://conf.researchr.org/details/icse-2018/icse-2018-Technical-Papers/78/Are-Mutation-Scores-Correlated-with-Real-Fault-Detection-A-Large-Scale-Empirical-stu), [preprint](https://coinse.github.io/publications/pdfs/Papadakis2018hi.pdf) | VERIFIED |
| Industrial: 15 million mutants at Google; developers using mutation testing wrote more tests and left fewer surviving mutants over time; in an analysis of fixes to high-priority faults, mutants were coupled with real faults (a live mutant would have been reported before the bug). | Petrovic, Ivankovic, Fraser, Just, ICSE 2021: [arXiv 2103.07189](https://arxiv.org/abs/2103.07189v1) (abstract read via [ar5iv](https://ar5iv.labs.arxiv.org/html/2103.07189)) | VERIFIED |
| Mutation testing is "widely considered the best approach for evaluating test suite efficacy"; a fully covered, weakly tested function (deleting `buf.Append(...)` is not detected) is the motivating example. | Petrovic et al., [arXiv 2102.11378](https://ar5iv.labs.arxiv.org/html/2102.11378) intro | VERIFIED (as authors' framing) |
| Survey: random sampling of 10-60% of mutants loses roughly 26% to 6% of faults ("fault loss" per Papadakis & Malevris, quoted); Gopinath et al. found a small constant number of random mutants statistically similar to all mutants; selective-operator reduction is only up to ~13% better than random. | [Papadakis et al. survey](https://mutationtesting.uni.lu/survey.pdf), Section on mutant reduction | VERIFIED (as survey statement; primaries not opened) |

### 2.2 Subsumption and equivalent mutants (false-positive/noise sources)

- **Subsumption/redundant mutants**: mutants killed whenever others are killed "do not contribute to the test assessment process", inflate the score, and make it "hard to interpret". Survey estimates "more than 60%" chance of compromised conclusions in arbitrary experiments from the subsumed-mutant threat. VERIFIED ([survey](https://mutationtesting.uni.lu/survey.pdf), Sections 1, 9.3). Kurtz et al. (FSE 2016, [PDF](https://acikerisim.mu.edu.tr/xmlui/bitstream/handle/20.500.12809/2655/G%C3%B6k%C3%A7e.pdf?sequence=1)) show traditional selective-mutation operator sets look fine on raw scores but have **poor dominator-mutation scores** on at least some programs, i.e., "mutants killed" over-credits trivial kills. VERIFIED.
- **Equivalent mutants**: undecidable in general. Trivial Compiler Optimization (TCE) is reported to identify "at least 30% of all the equivalent mutants" and, separately, duplicate/equivalent instances for on average 21% of C and 5.4% of Java mutants; the survey gives these in different sections without reconciling them. VERIFIED (as survey statements; primaries not opened). Meta's LLM equivalence detector: precision 0.79, recall 0.47 raw, 0.95/0.96 with simple pre-processing ([ACH, arXiv 2501.12862](https://arxiv.org/abs/2501.12862)). VERIFIED (abstract).
- **Flaky tests corrupt kill attribution**: mutation scores vary 4 pp on average between repeated runs and 9% of mutant-test pairs have unknown status; strategic re-runs cut unknown mutants by 79.4% across 30 projects ([Shi, Bell, Marinov, ISSTA 2019](https://par.nsf.gov/servlets/purl/10104340)). VERIFIED.

### 2.3 Industrial deployments (cost/precision data)

| System | Design | Data | Source / status |
|---|---|---|---|
| Google, diff-based mutation | mutate only changed, covered, non-"arid" lines; at most one mutant per line; surfaced during code review | 6,000 engineers, ~30% of diffs with coverage (2018 abstract). Later: 776,740 changelists, 16,935,148 mutants generated, 2,110,489 surfaced; initial **85% unproductive** after sampling; heuristics lifted productivity "from 15% to 89%"; ~87.5% of generated mutants are killed by the initial suite; ROR mutants productive 84.1% vs UOI 74.5%. | [State of Mutation Testing at Google](https://research.google/pubs/state-of-mutation-testing-at-google/), [Practical Mutation Testing at Scale](https://ar5iv.labs.arxiv.org/html/2102.11378). VERIFIED |
| Meta ACH | few, concern-targeted mutants -> LLM writes tests that kill them | 10,795 Kotlin classes, 9,095 mutants, 571 tests; engineers accepted 73%, judged 36% privacy-relevant | [arXiv 2501.12862](https://arxiv.org/abs/2501.12862). VERIFIED (abstract). *Test generation, not test critique; no per-test redundancy analysis reported in the abstract.* |
| Predictive mutation testing (PMT) | classifier on mutant+test+coverage features predicts killed/alive without execution | 163 projects; up to 151.4x faster; AUC > 0.80 for most projects | [Meta Research page](https://research.fb.com/publications/predictive-mutation-testing/) (abstract; page lists TSE). Original ISSTA 2016 entry: Zhang et al., pp. 342-353, doi 10.1145/2931037.2931038, per the survey's reference list. VERIFIED (abstract); survey adds ">0.85 precision and recall". |

Lesson (INFERRED but strongly supported by the Google figures): **raw surviving-mutant output is mostly noise without suppression heuristics**; the engineering is in filtering (arid nodes, per-line cap, historical operator productivity), not in mutation itself.

---

## 3. Pseudo-tested methods / extreme mutation

| Claim | Source | Status |
|---|---|---|
| Definition: a method is **pseudo-tested** if it is covered but "no test case fails when the whole logic of the method is removed". Operator: strip body (void) or return fixed values (max two mutants/method for primitives/String; factory-built objects). Excludes empty methods, one-statement getters/setters, constructors, synthetic methods. Cost: "a few hours" for a medium project because mutants-per-method is capped. | Niedermayr, Juergens, Wagner, CSED 2016: [arXiv 1611.07163](https://ar5iv.org/html/1611.07163) | VERIFIED |
| Prevalence: 14 Java projects; ratio of pseudo-tested methods ranges 6%-53% for most; unit-test suites mean 11.41% (SD 6.42), system tests mean 35.48% (SD 20.60); Commons Lang 93% method coverage with <2% pseudo-tested; others (Predictor, Struts 2, LittleProxy) >50%. Conclusion: coverage is a defensible effectiveness indicator for unit tests, **not** for system tests. | same | VERIFIED |
| Replication: 21 Java projects, 28K+ methods, Descartes (PIT extension); pseudo-tested methods in **all** subjects, ratio 1%-46%; pseudo-tested ratio negatively correlated with coverage (Pearson -0.67) but "more directly impacted by the way the methods are verified"; mutation score *inside* pseudo-tested methods is significantly lower than in required methods; of 101 pseudo-tested methods reviewed with developers, **only 30 were worth additional testing** (rest: trivial, delegation, logging/precondition-style); PRs fixing seven projects' tests (e.g., a test missing its assertion in commons-codec) were all merged. | Vera-Perez, Danglot, Monperrus, Baudry, EMSE 2019: [arXiv 1807.05030](https://ar5iv.org/html/1807.05030) | VERIFIED |

Signal properties for the catalog:
- **Direct relevance**: it is the mechanical form of "test executes code but cannot detect its removal"; the reverse view is **assertion-free or assertion-weak test**. Niedermayr cites Fowler's AssertionFreeTesting as the same smell.
- **Granularity caveat**: output is per *method*, not per *test*. To blame a test you need method-to-covering-tests mapping (Niedermayr step 2: per-test method coverage by instrumentation). A test is "responsible" for a pseudo-tested method only if it covers it and is the only/strongest cover. Not evaluated in either paper (GAP).
- **False-positive modes (VERIFIED from Vera-Perez)**: trivial/delegating methods, precondition checkers, logging, hashCode/toString, non-deterministic code; developers judge ~70% not worth fixing. Equivalent-mutant risk is low for extreme mutation but nonzero (threat discussed in Niedermayr 6.1: exceptions thrown in pseudo-tested methods can still fail tests).

---

## 4. Per-test metrics derived from mutation analysis

### 4.1 What exists

| Metric | Definition | Who published / evidence | Status |
|---|---|---|---|
| **Mutant-adequate reduction (MAR)** | Keep a subset of tests that kills the same mutants as the full suite; a test is redundant if it kills no mutant the others don't. Offutt et al. proposed it ~20 years before 2014; Shi et al. evaluated it at scale. | Shi et al., FSE 2014 (18 projects, 261,235 tests). Result above: no killed-mutant loss, +11.9 pp suite size over statement-coverage reduction, and killed mutants of the reduced suite "fairly stable" across later versions (median drop <= 0.76 pp). | [PDF](https://mir.cs.illinois.edu/gyori/pubs/fse14reduction.pdf). VERIFIED |
| **Inadequate (relaxed) reduction** | Keep 95% of requirements: median size reduction rises 17.14 pp. Trades more fault loss for more deletions. | same | VERIFIED |
| **Unique kills per test** | Mutants killed by test t and by no other test (kill-matrix column set difference). | Implied by MAR / set-cover reduction; the 2014 FSE paper defines redundancy exactly as "satisfies only requirements satisfied by others" with killed mutants as requirements. I found **no paper that publishes unique-kill count as a standalone ranking metric and validates it against real faults.** | Concept VERIFIED (Shi 2014 Def. of redundancy); standalone validation = GAP |
| **Test "strength"/kill score** | Mutants a test kills / mutants it covers. | Not found as a published, validated per-test metric. PIT reports per-mutant killing test and "test strength" at the tool level (INFERRED; not verified here). | UNVERIFIED |
| **Dominator-mutant-based adequacy** | Redundancy-free score using only subsuming mutants; kills of dominators are the meaningful per-test signal. | Kurtz et al. 2016; survey Section 9.3. | VERIFIED |
| **Predicted kill matrix** | PMT classifier predicts per-(mutant,test) outcome without execution. | Zhang et al. (above). Predicts per-mutant outcomes; use as the matrix generator for redundancy was not evaluated in the abstract. | VERIFIED abstract only |

### 4.2 Evidence that such redundancy measures mislead

- Shi 2018 (VERIFIED): reduction measured against real failed builds loses up to 52.2%; traditional metrics (size reduction, seeded-fault loss) are poor predictors. So a test with zero unique kills can still be the only detector of a real future regression.
- Papadakis 2018 (VERIFIED): since only <1% of mutants resemble real-fault behavior, a test's unique kills are dominated by mutants irrelevant to real faults; a test can have many unique kills yet little real value, and the reverse.
- Flakiness (Shi 2019, VERIFIED): a single flaky kill can make a test look essential or a flaky miss make it look redundant.
- Per requirements.md rabbit hole: redundant tests may be intentional regression pins/spec documentation. No mutation-based paper I read models intent. INFERRED precision ceiling.

---

## 5. Signal catalog (per requirements Success Metric 2)

Cost classes are relative (hook-latency / batch / heavy batch), not wall-clock.

| Signal | Input required | Cost | Empirical validation | Known false-positive / noise modes | Fits kibitzer? (INFERRED) |
|---|---|---|---|---|---|
| Suite-level coverage % | per-run coverage report | Low | Weak: low-moderate after size control (Inozemtseva); sometimes (Namin); statement best for mutant kills (Gopinath); weak vs future bug-fixes (Ahmed) | Executes != asserts; uncovered-by-any-test has some signal (half the bug-fixes) | Already a CI gate; not a test critique |
| Test with **zero incremental coverage** (unique-statement set empty) | **per-test** coverage | Medium-High: one run per test or per-test instrumentation; Niedermayr-style instrumentation is run-once | Lossy as redundancy test: reduction loses up to 20.5% of killed mutants (Shi 2014); poor predictor of real-failure loss (Shi 2018). No direct validation as a per-test signal (GAP) | Regression pins, different assertions on same lines, order/state effects, flaky attribution | Ingest-only report; batch/CI |
| **Pseudo-tested method** (extreme mutation) | per-test method coverage + mutation run (extreme operator only) | Medium: <= 2 mutants/method; "a few hours" for medium project (Niedermayr); Descartes on PIT | Replicated on 21+14 projects; mutation score lower in these methods (Vera-Perez); developer judged 30/101 worth fixing | Trivial/delegating/logging/precondition methods; equivalent-ish; system tests inflate ratio | Strong fit to "executes without detecting"; method-level, needs mapping to tests |
| **Surviving mutants in changed lines** (diff-based) | line coverage + mutation of diff | Medium-High; Google reports needing arid-node suppression | Industrial: productivity 15% -> 89% with heuristics; developers wrote more tests (Petrovic 2021) | Equivalent/unproductive mutants (85% initially unproductive); logging, time, config flags | Good conceptual fit (diff-aware); cost/hook never |
| **Mutation score** (suite or class level) | mutation run | High | Valid with caveats: 73% coupling (Just); weak after size control (Papadakis 2018); good guidance | Equivalent mutants; subsumed mutants inflate; flakiness (+-4 pp); size confound | CI batch/sampled only; sampled mutants fine (survey) |
| **Unique mutants killed per test / zero-unique-kill test** (MAR redundancy) | **kill matrix** (test x mutant, with kill-all not stop-at-first-kill) | **Highest**: every mutant must run against every covering test (no early exit) | Reduction evaluated at scale (Shi 2014); but real-failure loss up to 52.2% (Shi 2018); standalone per-test validation = GAP | Subsumed/trivial mutants give spurious "unique" kills; flaky kills; intentional pins | Batch/CI; ingest-only; best used as ranking for LLM adjudication |
| **Dominator-mutant kills per test** | kill matrix + subsumption graph | Highest (needs full matrix to compute subsumption) | Theory + selective-mutation re-evaluation (Kurtz 2016); no per-test deletion study found | Subsumption computed on one suite is relative to that suite | Research-only for now |
| Predicted kill matrix | static+coverage features + trained model | Medium | PMT: AUC > 0.80 (TSE abstract); not evaluated for test deletion | Cross-project drift; errors compound when used to rank | Possible cost reducer; unvalidated for this purpose |
| Static assertion-weakness (no assertion, tautology, mock-only) | source only | Low (hook-capable) | Niedermayr/Vera-Perez show assertion-free test as concrete cause (commons-codec PR); **no study validates a static detector against kills or faults** (GAP; out of this review's reading) | Exception-expectation tests, helper assertions, framework asserts | Hook-friendly; must be measured on corpus |

---

## 6. Answers to requirements.md open questions (from this literature only)

- **OQ2 best validated predictor of "removable without losing fault detection"**: nothing is well validated against *real* fault loss. The best-characterized is **mutant-adequate reduction** (no mutant loss, evaluated at 261K tests) but Shi 2018 shows real-failure loss up to 52.2% and that traditional metrics don't predict it. VERIFIED facts; the ranking is my reading.
- **OQ3 unique coverage vs unique kills**: Shi 2014 supports "kills are a stricter requirement than statement coverage" (coverage reduction loses up to 20.5% of kills; kill-based loses 0 at +11.9 pp), so unique-coverage-zero is a *superset-ish, lossier* criterion: zero unique kills implies strong redundancy, zero unique coverage does not (INFERRED from VERIFIED numbers). Whether the cheaper one *suffices* is still to be measured on the corpus.
- **OQ5 static assertion signals**: no validation found; Niedermayr's observation (assertion-free tests are useless unless checking exceptions) gives a plausibility argument only.
- **OQ6 how to validate deletions**: literature uses mutation-score delta (Shi 2014), historical real failures (Shi 2018: Failed-Build Detection Loss), or bug-fix counts (Ahmed 2016). Recommendation (INFERRED): use mutation-score delta as primary, FBDL-style re-detection of historical failures where CI history exists, and a **size-matched random deletion baseline** (Inozemtseva's size confound) for every claim.

## 7. Gaps (honest list)

1. No primary opened for Chekam et al. ICSE 2017, Kochhar et al. SANER 2015, Hemmati QRS 2015, Zhang et al. ISSTA 2016 (abstract read via a Meta page only), Andrews et al. TSE 2006, Papadakis et al. "Mutant quality indicators"/subsumption ICSE 2016 paper (only via the survey).
2. **No validated per-test metric**: neither "unique statements per test" nor "unique kills per test" was found validated against real-fault detection of the suite-minus-test. Closest evidence is suite-level reduction (Shi 2014/2018).
3. No study found comparing kill-matrix redundancy to coverage redundancy on identical suites with a size-matched random baseline.
4. No evidence for Go/Rust/TS/Kotlin: every empirical paper above is Java (Inozemtseva, Just, Niedermayr, Vera-Perez, Shi) or C/Java (Papadakis 2018). Google's data is multi-language but aggregate. Language transfer of the false-positive rates is UNVERIFIED.
5. Tool output formats (PIT XML/CSV kill details, Stryker JSON, cargo-mutants, go-mutesting) not examined here; belongs to the tools inventory track.
6. The "delete 25% with no coverage change" anecdote has no corresponding published result in what I read; Shi 2014's 62.9% average coverage-based reduction suggests suites carry large coverage-redundant fractions, but with mutant loss up to 20.5%.

## 8. Bibliography

| # | Authors | Year | Venue | URL | Contribution (one line) | Verified? |
|---|---|---|---|---|---|---|
| 1 | Inozemtseva, Holmes | 2014 | ICSE | https://cs.ubc.ca/~rtholmes/papers/icse_2014_inozemtseva.pdf | Coverage weakly correlated with effectiveness once suite size controlled; size dominates | VERIFIED (full text) |
| 2 | Namin, Andrews | 2009 | ISSTA | https://www.depts.ttu.edu/cs/research/avesta/documents/papers/ISSTA2009/ISSTA09.pdf | Size and coverage jointly predict effectiveness; nonlinear; small C programs | VERIFIED (abstract/intro) |
| 3 | Gopinath, Jensen, Groce | 2014 | ICSE | https://experts.nau.edu/en/publications/code-coverage-for-suite-evaluation-by-developers/ | For single-suite evaluation, statement coverage best predicts mutant kills | VERIFIED (abstract) |
| 4 | Ahmed, Gopinath, Brindescu, Groce, Jensen | 2016 | FSE | https://experts.nau.edu/en/publications/can-testedness-be-effectively-measured/ | Coverage and mutation score only weakly negatively correlated with future bug-fixes | VERIFIED (abstract) |
| 5 | Chekam, Papadakis, Le Traon, Harman | 2017 | ICSE | https://orbilu.uni.lu/handle/10993/30694 | Fault revelation improves only at high strong-mutation coverage; clean-program assumption | SECONDARY (via survey and Papadakis 2018) |
| 6 | Kochhar, Thung, Lo | 2015 | SANER | https://ieeexplore.ieee.org/document/7081877 | Coverage vs real-bug detection on HttpClient/Rhino; significant, system-dependent | UNVERIFIED (search summary) |
| 7 | Hemmati | 2015 | QRS | none found | "How effective are code coverage criteria?" | UNVERIFIED (citation only) |
| 8 | Just, Jalali, Inozemtseva, Ernst, Holmes, Fraser | 2014 | FSE | https://homes.cs.washington.edu/~mernst/pubs/mutation-effectiveness-fse2014.pdf | Mutants correlate with real faults beyond coverage; 73% coupled, 27% not | VERIFIED (full text) |
| 9 | Papadakis, Shin, Yoo, Bae | 2018 | ICSE | https://coinse.github.io/publications/pdfs/Papadakis2018hi.pdf | Mutation-score/real-fault correlation weak once size controlled; <1% of mutants matter | VERIFIED (abstract + partial text) |
| 10 | Papadakis, Kintis, Zhang, Jia, Le Traon, Harman | 2018 | Advances in Computers 112 | https://mutationtesting.uni.lu/survey.pdf | Survey of mutation advances: subsumption, equivalence, sampling, PMT | VERIFIED (selected sections) |
| 11 | Kurtz, Ammann, Offutt, Delamaro, Kurtz, Gokce | 2016 | FSE | https://acikerisim.mu.edu.tr/xmlui/bitstream/handle/20.500.12809/2655/G%C3%B6k%C3%A7e.pdf?sequence=1 | Selective mutation performs poorly on dominator-mutant score | VERIFIED (abstract/intro) |
| 12 | Niedermayr, Juergens, Wagner | 2016 | CSED | https://ar5iv.org/html/1611.07163 | Defines pseudo-tested methods via extreme mutation; 6-53% prevalence | VERIFIED (full text) |
| 13 | Vera-Perez, Danglot, Monperrus, Baudry | 2019 | EMSE | https://ar5iv.org/html/1807.05030 | Replication (28K methods, Descartes); developers value ~30% of findings | VERIFIED (full text, key sections) |
| 14 | Petrovic, Ivankovic | 2018 | ICSE-SEIP | https://research.google/pubs/state-of-mutation-testing-at-google/ | Diff-based probabilistic mutation with arid-line suppression at Google | VERIFIED (abstract) |
| 15 | Petrovic, Ivankovic, Fraser, Just | 2021 | arXiv (Practical Mutation Testing at Scale) | https://ar5iv.labs.arxiv.org/html/2102.11378 | Incremental, filtered, learned mutant selection; productivity 15% -> 89% | VERIFIED (selected sections) |
| 16 | Petrovic, Ivankovic, Fraser, Just | 2021 | ICSE | https://arxiv.org/abs/2103.07189v1 | Mutation testing changes developer behavior; mutants coupled with real high-priority faults | VERIFIED (abstract) |
| 17 | Foster, Gulati, Harman, Harper, Mao, Ritchey, Robert, Sengupta | 2025 | FSE Industry | https://arxiv.org/abs/2501.12862 | Meta ACH: mutation-guided LLM test generation; LLM equivalent-mutant detector | VERIFIED (abstract) |
| 18 | Zhang et al. (Zhang, Wang, Zhang, Hao, Zang, Cheng, Zhang per survey bibliography) | 2016 / TSE version | ISSTA / TSE | https://research.fb.com/publications/predictive-mutation-testing/ | Predictive mutation testing: classifier predicts kills, up to 151.4x faster | VERIFIED (abstract); author list from survey |
| 19 | Shi, Gyori, Gligoric, Zaytsev, Marinov | 2014 | FSE | https://mir.cs.illinois.edu/gyori/pubs/fse14reduction.pdf | Mutant-based test-suite reduction vs coverage-based; evolution-aware metrics | VERIFIED (text) |
| 20 | Shi, Gyori, Mahmood, Zhao, Marinov | 2018 | ISSTA | https://mir.cs.illinois.edu/gyori/pubs/issta18.pdf | Reduced suites miss up to 52.2% of real failed builds; traditional metrics poor predictors | VERIFIED (abstract) |
| 21 | Shi, Bell, Marinov | 2019 | ISSTA | https://par.nsf.gov/servlets/purl/10104340 | Flaky tests make mutation results unstable; re-run strategy cuts unknowns 79.4% | VERIFIED (abstract) |
