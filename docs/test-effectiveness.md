# Test effectiveness: literature, signal catalog, and tool inventory

This page records what the literature says about mechanical signals that a test
is ineffective or removable, how well each signal is validated, and which
existing tools produce the data such a signal needs. It backs the
`test-effectiveness-critique` project. Planning artifacts live under
[`project_plans/test-effectiveness-critique/`](../project_plans/test-effectiveness-critique/requirements.md);
the sources of every claim below are the research files
[`literature-effectiveness-mutation.md`](../project_plans/test-effectiveness-critique/research/literature-effectiveness-mutation.md)
(part A), [`literature-redundancy-smells-llm.md`](../project_plans/test-effectiveness-critique/research/literature-redundancy-smells-llm.md)
(part B), and [`stack.md`](../project_plans/test-effectiveness-critique/research/stack.md).

Every signal here is advisory. None is validated as a safe deletion criterion,
and this page makes no claim that any flagged test can be removed without losing
fault detection.

## How to read the evidence

Evidence labels are carried over unchanged from the research files:

- **VERIFIED**: the researcher opened the source (PDF text, arXiv or publisher
  abstract, or a repository record) and the claim is in it. "Abstract" means the
  claim is limited to the abstract.
- **SECONDARY**: stated by a verified source about another paper; the primary
  was not opened.
- **INFERRED**: reasoning from verified facts; no source says it directly.
- **UNVERIFIED**: seen only in a search summary or prior knowledge. These
  sources appear only under [Unverified / not relied on](#unverified--not-relied-on).

Validation status of a signal uses three words, plus one qualifier:

- **empirically validated**: a study opened by the researcher tests the signal
  against faults or a fault proxy.
- **plausible**: a mechanism and indirect evidence exist, but no study tests the
  signal as a predictor of removability.
- **unvalidated**: no supporting study was found.
- **lossy**: a study measured the signal and found measurable fault-detection
  loss when it is used for deletion.

Findings that shape every row below:

- Across 31,000 suites over five Java systems, effectiveness correlates with the
  number of test methods; after controlling for suite size the coverage
  correlation is low to moderate (Inozemtseva and Holmes, ICSE 2014; VERIFIED,
  [part A section 1.1](../project_plans/test-effectiveness-critique/research/literature-effectiveness-mutation.md)).
  Any deletion claim therefore needs a size-matched random-deletion baseline
  (INFERRED, part A section 1.2).
- Correlation between mutation score and real-fault detection is weak once suite
  size is controlled, and fewer than 1% of mutants represent real-fault behavior
  (Papadakis et al., ICSE 2018; VERIFIED, part A section 2.1). Mutants are
  guidance, not a ground truth for deletion.
- Only Java and C subjects appear in the empirical papers read; transfer to Go,
  Rust, TypeScript, and Kotlin is UNVERIFIED (part A section 7, gap 4).
- No study found validates a per-test metric (unique coverage or unique kills)
  against the real-fault detection of the suite minus that test (part A
  section 7, gap 2).

Attribution correction: the paper "Code coverage and test suite effectiveness:
empirical study with real bugs in large systems" is Kochhar, Thung, and Lo
(SANER 2015), not Gopinath et al. Gopinath, Jensen, and Groce's ICSE 2014 paper
is "Code coverage for suite evaluation by developers" ([NAU abstract page](https://experts.nau.edu/en/publications/code-coverage-for-suite-evaluation-by-developers/);
VERIFIED for the Gopinath title, part A header). The Kochhar paper itself is
UNVERIFIED and is listed below, not relied on.

## Signal catalog

One row per candidate signal. The first table holds the six signals that map to
kibitzer's `Signal` enum (rule ids per the plan's Rule-id registry); the second
holds candidates that have no kibitzer rule. "A" rows are part A section 5
(`A1` to `A9` in table order); "S" rows are part B section 7 (`S1` to `S9`). The
two numbering schemes overlap, so the "Source rows" column reconciles them.

Cost classes are relative: trivial and low fit a hook, medium and high are batch
or CI. "Advisory-diff fit" is INFERRED in both source files.

### Signals with a kibitzer rule id

| Signal | Rule id | Source rows | Input required | Cost | False-positive risk | Advisory-diff fit | Validation status | Primary citation |
|---|---|---|---|---|---|---|---|---|
| `NoAssertion` | `test-no-assertion` | A9, S1 | Source only (static) | Trivial | Exception-expectation tests, helper assertions, framework implicit assertions (snapshot, fuzz, mock verification) | Hook-friendly; precision must be measured on a corpus | plausible. Missing or incorrect assertions are 67% of "silent horror" test bugs (VERIFIED), and assertion-free tests are a concrete cause of pseudo-tested code (VERIFIED); no study validates a static detector against kills or faults | Vahabzadeh, Milani Fard, Mesbah, ICSME 2015: [PDF](https://people.ece.ubc.ca/amesbah/resources/papers/icsme15.pdf); Niedermayr et al., CSED 2016: [arXiv 1611.07163](https://ar5iv.org/html/1611.07163) |
| `Tautology` | `test-tautology` | A9, S1 | Source only (static) | Trivial | Low frequency; likely high precision with low recall (INFERRED) | Hook-friendly; precision must be measured on a corpus | unvalidated. No empirical validation found (named gap); the nearest source covers incorrect assertions as a defect class | Vahabzadeh, Milani Fard, Mesbah, ICSME 2015: [PDF](https://people.ece.ubc.ca/amesbah/resources/papers/icsme15.pdf) (closest related; does not test tautology detection) |
| `MockOnly` | `test-mock-only` | S2 | Source only (static) | Trivial | Legitimate interaction tests; mock-heavy tests that still assert on real state | Opt-in ranked hint, not a verdict | unvalidated. Agents add mocks in 36% of commits versus 26% for non-agents (VERIFIED, abstract), a prevalence and risk statement rather than an effectiveness predictor | Hora and Robbes, 2026: [arXiv 2602.00409](https://arxiv.org/abs/2602.00409) |
| `ZeroUniqueCoverage` | `test-zero-unique-coverage` | A2, S5 | Per-test coverage | Medium to high (one run per test, or per-test instrumentation) | Regression pins, different assertions over the same lines, order and state effects, flaky attribution | Ingest-only batch or CI report | lossy: statement-coverage reduction cuts suites 62.9% on average but loses up to 20.5% of killed mutants (Shi et al., FSE 2014; 18 Java projects); no direct validation as a per-test signal | Shi et al., FSE 2014: [PDF](https://mir.cs.illinois.edu/gyori/pubs/fse14reduction.pdf); Koochakzadeh and Garousi, 2010: [DOI](https://doi.org/10.1155/2010/932686) |
| `ZeroUniqueKills` | `test-zero-unique-kills` | A6, S6 | Kill matrix (test by mutant, all killers recorded, not stop-at-first-kill) | Highest (every mutant runs against every covering test) | Subsumed or trivial mutants give spurious "unique" kills; flaky kills; intentional regression pins | Ingest-only batch or CI report; best used as a ranking for adjudication (INFERRED) | lossy: failed-build detection loss up to 52.2% on Java projects with mutation-/coverage-based reduction (Shi et al., ISSTA 2018; scope stated in the row). Scope: 1,478 failed Travis builds, 32 GitHub Java projects; 52.2% is the coverage-based Greedy figure under the most pessimistic fault mapping, and the mutant-based Greedy figure is 36.2% (VERIFIED, part B section 1.2). Per-test unique-kill count is not validated as a standalone ranking (gap) | [Shi et al., ISSTA 2018 (primary PDF)](https://mir.cs.illinois.edu/gyori/pubs/issta18.pdf); [Shi et al., FSE 2014](https://mir.cs.illinois.edu/gyori/pubs/fse14reduction.pdf) |
| `PseudoTestedRegion` | `test-pseudo-tested-region` | A3 | Per-test method coverage plus an extreme-mutation run | Medium (at most 2 mutants per method; "a few hours" for a medium project) | Trivial, delegating, logging, and precondition methods; system tests inflate the ratio | Good conceptual fit to "executes without detecting"; method-level, so blaming a test needs a method-to-test mapping | empirically validated at the method level: pseudo-tested methods exist in all 21 replication subjects (ratio 1% to 46%), but developers judged only 30 of 101 reviewed methods worth more testing. Blaming a specific test is not evaluated in either paper | Niedermayr et al., CSED 2016: [arXiv 1611.07163](https://ar5iv.org/html/1611.07163); Vera-Perez et al., EMSE 2019: [arXiv 1807.05030](https://ar5iv.org/html/1807.05030) |

### Candidates with no kibitzer rule

| Signal | Source rows | Input required | Cost | False-positive risk | Advisory-diff fit | Validation status | Primary citation |
|---|---|---|---|---|---|---|---|
| Suite-level coverage percent | A1 | Per-run coverage report | Low | Executing code is not asserting on it; "covered by any test" still carries signal (about half the later bug-fixes of uncovered elements) | Already a CI gate; not a test critique | empirically validated as weak: low to moderate after size control | Inozemtseva and Holmes, ICSE 2014: [PDF](https://cs.ubc.ca/~rtholmes/papers/icse_2014_inozemtseva.pdf); Ahmed et al., FSE 2016: [abstract](https://experts.nau.edu/en/publications/can-testedness-be-effectively-measured/) |
| Surviving mutants in changed lines | A4 | Line coverage plus mutation of the diff | Medium to high | Equivalent and unproductive mutants (85% initially unproductive at Google); logging, time, config flags | Conceptual fit (diff-aware); never in a hook | empirically validated in industry: productivity from 15% to 89% with suppression heuristics | Petrovic et al.: [arXiv 2102.11378](https://ar5iv.labs.arxiv.org/html/2102.11378); [ICSE 2021](https://arxiv.org/abs/2103.07189v1) |
| Mutation score (suite or class) | A5 | Mutation run | High | Equivalent and subsumed mutants inflate it; flakiness moves it about 4 points between runs; size confound | CI batch or sampled only | empirically validated with caveats: 73% of 357 real faults coupled to mutants (Just et al.), weak after size control (Papadakis et al.) | Just et al., FSE 2014: [PDF](https://homes.cs.washington.edu/~mernst/pubs/mutation-effectiveness-fse2014.pdf); Papadakis et al., ICSE 2018: [preprint](https://coinse.github.io/publications/pdfs/Papadakis2018hi.pdf) |
| Dominator-mutant kills per test | A7 | Kill matrix plus subsumption graph | Highest | Subsumption computed on one suite is relative to that suite | Research only | plausible: theory plus a selective-mutation re-evaluation; no per-test deletion study found | Kurtz et al., FSE 2016: [PDF](https://acikerisim.mu.edu.tr/xmlui/bitstream/handle/20.500.12809/2655/G%C3%B6k%C3%A7e.pdf?sequence=1) |
| Predicted kill matrix | A8 | Static and coverage features plus a trained model | Medium | Cross-project drift; errors compound when ranking | Possible cost reducer | unvalidated for this purpose: AUC above 0.80 for predicting mutant outcomes; not evaluated for test deletion | Zhang et al., predictive mutation testing: [Meta Research](https://research.fb.com/publications/predictive-mutation-testing/) |
| Assertion density per test | S2 | Source only | Trivial | Parameterized and helper assertions; legitimate assertion-free tests | Ranked hint | plausible: suite-level correlation only (6,700 suites, 5 Java projects), not a per-test predictor | Zhang and Mesbah, FSE 2015: [DOI](https://doi.org/10.1145/2786805.2786858) |
| Test-smell rules (tsDetect-like) | S3 | Source (AST) | Low | The older detector misclassified over 70% of smells in the Panichella evaluation; smells often do not track semantic flaws | Optional, low priority; noisy | unvalidated for effectiveness: valid as maintainability hints only | Panichella et al., EMSE 2022: [PDF](https://pure.tudelft.nl/ws/portalfiles/portal/137994226/s10664_022_10207_5.pdf) |
| Textual or embedding similarity between tests | S4 | Test source only | Low | Similar text with a decisive input or assertion difference; parameterized boilerplate | Cheap batch report | plausible: recovers up to 54% of developer-deleted redundant tests (FAST-R); embedding minimization evaluated against mutant-based fault detection | Bhatta et al., MSR 2025: [preprint](https://hifromajay.github.io/papers/msr25.pdf); Pan, Ghaleb, Briand, TSE 2024: [arXiv 2304.01397](https://arxiv.org/abs/2304.01397) |
| Never failed in history | S7 | CI or run logs | Low where logs exist | Tests guarding rare regressions; short windows | Batch, repo-specific | plausible as an upper bound only: an oracle suite of tests that ever fail in future builds is about 20% of the original; maintainers treat it as an investigation signal | Shi et al., ISSTA 2018: [PDF](https://mir.cs.illinois.edu/gyori/pubs/issta18.pdf); [dotnet/android issue 13012](https://github.com/dotnet/android/issues/13012) (practitioner) |
| Checked coverage | S8 | Dynamic slicing of a test run | High | Java-specific tooling (INFERRED); slicing cost | Research or optional | plausible: abstract-level validation on seven projects | Schuler and Zeller, ICST 2011: [DOI](https://doi.org/10.1109/ICST.2011.32) |
| Flakiness or stability flag per test | S9 | Repeated runs or history | Medium | Not a deletion signal itself; a guard for coverage, kill, and history signals | Required guard (INFERRED) | empirically validated as background: about 1.5% of Google test runs report a flaky result, mutation scores shift about 4 points between runs | Luo et al., FSE 2014: [PDF](https://huang.isis.vanderbilt.edu/cs8395/paper/flakytest.pdf); Micco, 2016: [Google Testing Blog](https://testing.googleblog.com/2016/05/flaky-tests-at-google-and-how-we.html); Shi, Bell, Marinov, ISSTA 2019: [PDF](https://par.nsf.gov/servlets/purl/10104340) |

## Tool inventory

One row per tool in `research/stack.md` sections 1 to 5. The "Source section"
column names the section of [`stack.md`](../project_plans/test-effectiveness-critique/research/stack.md)
that holds the claim. The researcher opened each source on 2026-10-11, but none
of these claims were re-verified in this repo, so each row is marked `INFERRED`
in the Evidence column, except rows the research itself marks UNVERIFIED.
"Needs-config" means per-test attribution is possible only with extra setup,
described in the cell. Adapter story ids refer to
[`plan.md`](../project_plans/test-effectiveness-critique/implementation/plan.md)
Stories 2.2.1 to 2.2.6; the ingest-only design means kibitzer never runs the
tool.

| Tool | Language | Emitted artifact and format | Per-test granularity | Machine-ingestable by kibitzer | Source section | Evidence |
|---|---|---|---|---|---|---|
| coverage.py | Python | `.coverage` SQLite (schema v7), JSON, XML, lcov, HTML | yes (native: `dynamic_context = test_function`; JSON via `--show-contexts`) | yes: Story 2.2.1 (JSON with contexts) | 1 | INFERRED |
| `go test -coverprofile` | Go | Go cover profile text (`mode:` line plus `file:l.c,l.c nstmt count`) | needs-config (one `-run '^Name$'` invocation and profile per test) | yes: Story 2.2.2 (profile-per-test directory with manifest) | 1 | INFERRED |
| lcov producers c8, nyc, llvm-cov, gcov | TypeScript, JavaScript, Rust, C, C++ | lcov `.info` text; per-test only when a `TN:` record is written | unverified until Story 2.2.3 runs the producers | yes: Story 2.2.3 (`TN:` or one tracefile per test) | 1, 5, 8 | INFERRED |
| JaCoCo | Java | `.exec` binary, XML (`report.dtd`), CSV, HTML | needs-config (agent dump and reset per test via a JUnit listener) | no: aggregate counters only; reachable through lcov conversion (`jacoco2lcov`) under Story 2.2.3 | 1, 5 | INFERRED |
| Kover | Kotlin | HTML, XML (JaCoCo-style), binary | no (not built in) | no: aggregate only; same lcov conversion route as JaCoCo; Kover is labelled Beta | 1, 5 | INFERRED |
| cargo-llvm-cov | Rust | llvm-cov export JSON, lcov, cobertura, codecov, text, HTML | needs-config (one process per test with distinct `LLVM_PROFILE_FILE`; nextest model) | no for the JSON export (aggregate only); lcov output via Story 2.2.3 | 1, 5 | INFERRED |
| Descartes (PIT engine) | Java | PIT formats (XML, CSV, HTML) | yes (same per-mutant records as PIT) | yes: Story 2.2.5 | 2 | INFERRED |
| Major | Java | Own CSV logs | yes (documented as a complete mutant-test detection matrix) | no: own CSV format, research-grade, no adapter story | 2 | INFERRED |
| PIT (pitest) | Java, Kotlin via Gradle plugin | HTML, XML (`mutations.xml`), CSV | yes (`killingTest`, `killingTests`, `succeedingTests`, `coveringTests`; full matrix needs `fullMutationMatrix=true`) | yes: Story 2.2.5 | 2, 5 | INFERRED |
| `pitest-kotlin` plugin | Kotlin | PIT formats | yes (as PIT) | no: plugin archived (last push 2023-03-01); Kotlin dynamic claims are blocked on the plan's open Kotlin item | 2 | INFERRED |
| StrykerJS | TypeScript, JavaScript | mutation-testing-elements JSON (`killedBy[]`, `coveredBy[]`, `testFiles`), HTML, clear-text, dashboard | yes (`coverageAnalysis` defaults to `perTest`; bail-on-first-failure usually leaves one killer per mutant) | yes: Story 2.2.4 | 2, 5 | INFERRED |
| mutmut | Python | `.meta` JSON per source file (`exit_code_by_key`), `mutmut-stats.json` (`tests_by_mangled_function_name`) | needs-config (per-function test mapping only, no per-mutant killer) | no: internal format with no interchange promise; status-only mutation output is deferred | 2, 5 | INFERRED |
| cosmic-ray | Python | SQLite session (`WorkItem`, `WorkResult`), `cr-report`, `cr-xml`, `cr-html` | no (per-mutant outcome only) | no: internal SQLAlchemy models; status only | 2, 5 | INFERRED |
| gremlins | Go | `--output` JSON: per-file `mutations[{line, column, type, status}]` plus totals | no (per-mutant status only) | no: status only, unversioned; cannot rank tests | 2, 5 | INFERRED |
| go-mutesting (avito-tech fork) | Go | `json_output` and `html_output` config flags | no (killed or not per mutant) | no: status only; report schema unverified | 2, 5 | INFERRED |
| ooze | Go | Test-embedded API run as a Go test; report format not verified | no (not verified) | no: report format unverified | 2 | UNVERIFIED |
| cargo-mutants | Rust | `mutants.out/`: `mutants.json`, `outcomes.json`, `caught.txt`, `missed.txt`, `timeout.txt`, `unviable.txt`, `diff/`, `logs/` | no (killing test only in free-text per-mutant logs) | no: status only, and `outcomes.json` is documented as subject to change | 2, 5 | INFERRED |
| eslint-plugin-jest | TypeScript, JavaScript | ESLint findings (JSON output possible); rules such as `expect-expect` | no (static rule findings) | no: rule inspiration only; kibitzer is not an ESLint host, native checks in Stories 1.1.2 to 1.1.4 | 3 | INFERRED |
| testifylint | Go | golangci-lint findings; rules such as `useless-assert` | no (static rule findings) | no: rule inspiration only; optional golangci JSON ingest deferred | 3 | INFERRED |
| thelper | Go | golangci-lint findings (helper hygiene, `t.Helper()`) | no (static) | no: not an effectiveness signal | 3 | INFERRED |
| Clippy | Rust | `--message-format=json` diagnostics; lints such as `assertions_on_constants` | no (static) | no: rule inspiration only; few effectiveness lints | 3 | INFERRED |
| tsDetect | Java | CSV in and out; 21 smell types from the academic catalogue (the count comes from the testsmells.org project and is UNVERIFIED in the research) | no (static) | no: rule inspiration only; JVM dependency not worth ingesting | 3 | INFERRED |
| PyNose | Python | PyCharm plugin findings; 17 smells plus "Suboptimal Assert" (unittest only) | no (static) | no: inspiration only; source and maintenance status not checked | 3 | UNVERIFIED |
| ruff flake8-pytest-style (`PT*`) | Python | Assertion-style lint findings | no (static) | no: not assessed in the research | 3 | UNVERIFIED |
| Sonar test rules (S2699) | Multiple | Assertion-less test findings | no (static) | no: not assessed in the research | 3 | UNVERIFIED |
| pytest-testmon | Python | Per-test dependency data in an internal SQLite store | yes (tracks which tests touch which code) | no: internal format; coverage.py contexts are the preferred source | 4 | INFERRED |
| Bazel `bazel coverage` | Multiple | Per-test-target lcov | needs-config (test-target granularity, not per test function) | no: target granularity only; lcov route via Story 2.2.3 | 4 | INFERRED |
| Launchable (CloudBees Smart Tests) | Multiple | SaaS predictive test selection from test-result history | no | no: SaaS, not mechanical or offline | 4 | INFERRED |
| Develocity Predictive Test Selection | Java (Gradle, Maven) | ML test selection | no | no: commercial, Gradle and Maven only | 4 | UNVERIFIED |
| `go test -run` and `go test -json` | Go | Per-test pass, fail, and elapsed events (JSON stream) | yes (per test) | no for `-json` as a coverage source; it is the basis of the Go per-test profile loop (Story 2.2.2) | 4 | INFERRED |
| cargo-nextest | Rust | One process per test; JUnit XML | yes (process-per-test isolation point) | no directly; JUnit side-data via Story 2.2.6, and it enables per-process llvm-cov profiles | 4 | INFERRED |
| JUnit XML | Multiple (pytest, gotestsum, nextest, JVM) | Per-test pass, fail, and time XML | yes (pass, fail, and time only) | yes: Story 2.2.6 (side-data for flakiness and runtime cost) | 5 | INFERRED |

Open gap carried from `stack.md` section 8: whether c8, nyc, or llvm-cov ever
write a non-empty lcov `TN:` is not checked. Story 2.2.3 resolves it by running
the producers; until then the lcov row makes no per-test claim. Nothing found
emits a native per-test kill matrix for Go, Rust, or Python (`stack.md`
section 7).

## Bibliography

Merged from part A section 8 and part B bibliography, deduplicated (Shi 2014,
Shi 2018, Inozemtseva and Holmes, Papadakis 2018, Niedermayr, and Vera-Perez
appear in both). Labels are the research files' own. Sources labelled UNVERIFIED
are not listed here; they are in the next section.

| Authors | Year | Venue | Link | Contribution | Label |
|---|---|---|---|---|---|
| Inozemtseva, Holmes | 2014 | ICSE | [PDF](https://cs.ubc.ca/~rtholmes/papers/icse_2014_inozemtseva.pdf) | Coverage weakly correlated with effectiveness once suite size is controlled | VERIFIED (full text) |
| Namin, Andrews | 2009 | ISSTA | [PDF](https://www.depts.ttu.edu/cs/research/avesta/documents/papers/ISSTA2009/ISSTA09.pdf) | Size and coverage jointly predict effectiveness; small C programs | VERIFIED (abstract and intro) |
| Gopinath, Jensen, Groce | 2014 | ICSE | [abstract](https://experts.nau.edu/en/publications/code-coverage-for-suite-evaluation-by-developers/) | For single-suite evaluation, statement coverage best predicts mutant kills | VERIFIED (abstract) |
| Ahmed, Gopinath, Brindescu, Groce, Jensen | 2016 | FSE | [abstract](https://experts.nau.edu/en/publications/can-testedness-be-effectively-measured/) | Coverage and mutation score weakly negatively correlated with future bug-fixes | VERIFIED (abstract) |
| Chekam, Papadakis, Le Traon, Harman | 2017 | ICSE | [record](https://orbilu.uni.lu/handle/10993/30694) | Fault revelation improves only at high strong-mutation coverage | SECONDARY (via the survey and Papadakis 2018) |
| Just, Jalali, Inozemtseva, Ernst, Holmes, Fraser | 2014 | FSE | [PDF](https://homes.cs.washington.edu/~mernst/pubs/mutation-effectiveness-fse2014.pdf) | Mutants correlate with real faults beyond coverage; 73% coupled, 27% not | VERIFIED (full text) |
| Papadakis, Shin, Yoo, Bae | 2018 | ICSE | [preprint](https://coinse.github.io/publications/pdfs/Papadakis2018hi.pdf) | Mutation score and real-fault correlation weak once size is controlled | VERIFIED (abstract and partial text) |
| Papadakis, Kintis, Zhang, Jia, Le Traon, Harman | 2018 | Advances in Computers 112 | [PDF](https://mutationtesting.uni.lu/survey.pdf) | Survey: subsumption, equivalence, sampling, predictive mutation | VERIFIED (selected sections) |
| Kurtz, Ammann, Offutt, Delamaro, Kurtz, Gokce | 2016 | FSE | [PDF](https://acikerisim.mu.edu.tr/xmlui/bitstream/handle/20.500.12809/2655/G%C3%B6k%C3%A7e.pdf?sequence=1) | Selective mutation performs poorly on dominator-mutant score | VERIFIED (abstract and intro) |
| Niedermayr, Juergens, Wagner | 2016 | CSED | [arXiv](https://ar5iv.org/html/1611.07163) | Pseudo-tested methods via extreme mutation; 6% to 53% prevalence | VERIFIED (full text) |
| Vera-Perez, Danglot, Monperrus, Baudry | 2019 | EMSE | [arXiv](https://ar5iv.org/html/1807.05030) | Replication on 28K+ methods; developers value about 30% of findings | VERIFIED (full text, key sections) |
| Petrovic, Ivankovic | 2018 | ICSE-SEIP | [abstract](https://research.google/pubs/state-of-mutation-testing-at-google/) | Diff-based probabilistic mutation with arid-line suppression at Google | VERIFIED (abstract) |
| Petrovic, Ivankovic, Fraser, Just | 2021 | arXiv | [ar5iv](https://ar5iv.labs.arxiv.org/html/2102.11378) | Practical mutation testing at scale; productivity 15% to 89% | VERIFIED (selected sections) |
| Petrovic, Ivankovic, Fraser, Just | 2021 | ICSE | [arXiv](https://arxiv.org/abs/2103.07189v1) | Mutation testing changes developer behavior | VERIFIED (abstract) |
| Foster et al. | 2025 | FSE Industry | [arXiv](https://arxiv.org/abs/2501.12862) | Meta ACH: mutation-guided LLM test generation; LLM equivalent-mutant detector | VERIFIED (abstract) |
| Zhang et al. | 2016 | ISSTA / TSE | [Meta Research](https://research.fb.com/publications/predictive-mutation-testing/) | Predictive mutation testing; up to 151.4x faster | VERIFIED (abstract); author list from the survey |
| Shi, Gyori, Gligoric, Zaytsev, Marinov | 2014 | FSE | [PDF](https://mir.cs.illinois.edu/gyori/pubs/fse14reduction.pdf) | Mutant-based versus coverage-based reduction | VERIFIED (PDF) |
| Shi, Gyori, Mahmood, Zhao, Marinov | 2018 | ISSTA | [PDF](https://mir.cs.illinois.edu/gyori/pubs/issta18.pdf) | Reduced suites miss up to 52.2% of failed builds; metrics poor predictors | VERIFIED (PDF) |
| Shi, Bell, Marinov | 2019 | ISSTA | [PDF](https://par.nsf.gov/servlets/purl/10104340) | Flaky tests destabilize mutation results; re-run strategy cuts unknowns 79.4% | VERIFIED (abstract) |
| Yoo, Harman | 2012 (2007 preprint read) | STVR | [preprint](https://www.cse.chalmers.se/~feldt/advice/yoo_2010_regression_testing_survey.pdf) | Minimization, selection, prioritization survey | VERIFIED (preprint) |
| Harrold, Gupta, Soffa | 1993 | TOSEM | [DOI](https://doi.org/10.1145/152388.152391) | HGS heuristic for a representative test set | VERIFIED (abstract via OpenAlex) |
| Rothermel, Harrold, Ostrin, Hong | 1998 | ICSM | [DOI](https://doi.org/10.1109/icsm.1998.738487) | Minimization can severely compromise fault detection | VERIFIED (abstract); numbers via Yoo and Harman |
| Wong, Horgan, London, Mathur | 1995 | ICSE | [DOI](https://doi.org/10.1145/225014.225018) | Early minimization study | SECONDARY (metadata only; finding relayed by Yoo and Harman) |
| Vahabzadeh, Milani Fard, Mesbah | 2015 | ICSME | [PDF](https://people.ece.ubc.ca/amesbah/resources/papers/icsme15.pdf) | Bugs in test code; 97% false alarms, 3% silent horrors | VERIFIED (PDF) |
| Koochakzadeh, Garousi | 2010 | Advances in Software Engineering | [DOI](https://doi.org/10.1155/2010/932686) | Coverage-based redundancy false positives | VERIFIED (abstract) |
| Bhatta, Kendemah, Jha | 2025 | MSR | [preprint](https://hifromajay.github.io/papers/msr25.pdf) | DelTest: 24,431 deleted tests; 91.4% obsolete, 7% redundant | VERIFIED (page and preprint PDF) |
| Pan, Ghaleb, Briand | 2024 | IEEE TSE | [arXiv](https://arxiv.org/abs/2304.01397) | LTM: embedding-based black-box minimization | VERIFIED (abstract) |
| Kudrjavets, Nagappan, Ball | 2006 | MSR-TR-2006-54 | [record](https://www.microsoft.com/en-us/research/publication/assessing-the-relationship-between-software-assertions-and-code-qualityan-empirical-investigation/) | Assertion density versus fault density (production-code assertions) | VERIFIED (abstract) |
| Zhang, Mesbah | 2015 | ESEC/FSE | [DOI](https://doi.org/10.1145/2786805.2786858) | Assertion count correlates with suite effectiveness | VERIFIED (abstract via OpenAlex) |
| Schuler, Zeller | 2011 / 2013 | ICST / STVR | [DOI](https://doi.org/10.1109/ICST.2011.32) | Checked coverage as an oracle-quality indicator | VERIFIED (abstract via OpenAlex) |
| Barr, Harman, McMinn, Shahbaz, Yoo | 2015 | IEEE TSE | [UCL page](https://discovery.ucl.ac.uk/1471263/) | Oracle problem survey | VERIFIED (abstract) |
| Panichella, Panichella, Fraser, Sawant, Hellendoorn | 2022 | EMSE 27(7):170 | [PDF](https://pure.tudelft.nl/ws/portalfiles/portal/137994226/s10664_022_10207_5.pdf) | Test-smell detector accuracy and validity | VERIFIED (PDF) |
| Peruma, Almalki, Newman, Mkaouer | 2020 | ESEC/FSE (demo) | [DOI](https://doi.org/10.1145/3368089.3417921) | tsDetect: 19 smell types, 96% precision and 97% recall on a 65-file benchmark | VERIFIED (abstract) |
| Spadini, Palomba, Zaidman, Bruntink, Bacchelli | 2018 | ICSME | [record](https://research.tudelft.nl/en/publications/on-the-relation-of-test-smells-to-software-code-quality/) | Smelly tests more change- and defect-prone | VERIFIED (abstract) |
| Spadini, Schvarcbacher, Oprescu, Bruntink | 2020 | MSR | [DOI](https://doi.org/10.1145/3379597.3387453) | Severity thresholds; detectors too strict per developers | VERIFIED (abstract via OpenAlex) |
| Tufano, Palomba, Bavota, Di Penta | 2016 | ASE | [DOI](https://doi.org/10.1145/2970276.2970340) | Developers do not recognize test smells | VERIFIED (abstract via OpenAlex) |
| van Deursen, Moonen, van den Bergh, Kok | 2001 | CWI report | OpenAlex record for "Refactoring test code" | Original test-smell catalog | VERIFIED (abstract via OpenAlex) |
| Luo, Hariri, Eloussi, Marinov | 2014 | FSE | [PDF](https://huang.isis.vanderbilt.edu/cs8395/paper/flakytest.pdf) | Flaky-test root causes; Google TAP flaky share | VERIFIED (PDF) |
| Micco | 2016 | Google Testing Blog | [post](https://testing.googleblog.com/2016/05/flaky-tests-at-google-and-how-we.html) | About 1.5% of runs flaky; about 16% of tests have some flakiness | VERIFIED (page) |
| Alshahwan et al. | 2024 | FSE industry | [arXiv](https://arxiv.org/abs/2402.09171) | TestGen-LLM with mechanical filters | VERIFIED (abstract); author list from a search summary |
| Santana et al. | 2025 | arXiv | [arXiv](https://arxiv.org/abs/2506.07594) | LLM test smell detection and refactoring | VERIFIED (abstract) |
| Lucas et al. | 2024 | SBES IIER | [arXiv](https://arxiv.org/abs/2407.19261) | ChatGPT-4, Gemini, Mistral detect test smells | VERIFIED (abstract) |
| (authors not recorded) | 2025 | arXiv / TSE | [arXiv](https://arxiv.org/abs/2410.10628) | Test smells in LLM-generated tests | VERIFIED (abstract); authors UNVERIFIED |
| (authors not recorded) | 2026 | EMSE | [arXiv](https://arxiv.org/abs/2407.00225) | LLM versus EvoSuite unit tests; smells | VERIFIED (abstract); authors UNVERIFIED |
| (authors not recorded) | 2025 | AAAI-26 workshop | [arXiv](https://arxiv.org/abs/2512.01232) | LLM-as-a-Judge for Gherkin acceptance tests | VERIFIED (abstract) |
| (authors not recorded) | 2025 | arXiv | [arXiv](https://arxiv.org/abs/2510.24367) | LLM-as-a-Judge for software engineering roadmap | VERIFIED (abstract) |
| (authors not recorded) | 2025 | AIware | [arXiv](https://arxiv.org/abs/2601.05542) | LLM-generated test oracles study | VERIFIED (abstract) |
| (authors not recorded) | 2022 | arXiv | [arXiv](https://arxiv.org/abs/2207.12733) | Interaction of test-suite reduction and regression test selection | VERIFIED (abstract); authors UNVERIFIED |
| (authors not recorded) | 2017 | ISSREW | [arXiv](https://arxiv.org/abs/2104.01514) | Assert use and defectiveness in two industrial Java systems | VERIFIED (abstract); authors UNVERIFIED |
| Hora, Robbes | 2026 | arXiv | [arXiv](https://arxiv.org/abs/2602.00409) | Agents modify tests and add mocks more often | VERIFIED (abstract); author names from a search summary |
| simonrozsival (maintainer) | 2026 | GitHub issue, dotnet/android | [issue 13012](https://github.com/dotnet/android/issues/13012) | Practitioner low-value test pruning rules | VERIFIED (GitHub API record); practitioner, not peer reviewed |
| alexey-milovidov | 2026 | GitHub PR, ClickHouse | [PR 122485](https://github.com/ClickHouse/ClickHouse/pull/122485) | Removed 111 low-value integration test files; cost-driven | VERIFIED (GitHub API record); practitioner, not peer reviewed |

## Unverified / not relied on

These sources and claims are UNVERIFIED in the research. They appear in no
rationale and justify no design choice above.

| Item | Why it is here | Source |
|---|---|---|
| Practitioner anecdote of running Opus over repositories and deleting roughly a quarter of the tests with no significant change in coverage | no public write-up located. It is the project's motivating anecdote, recorded in `requirements.md`; the searches in part B section 6 found no peer-reviewed or published counterpart | [`requirements.md`](../project_plans/test-effectiveness-critique/requirements.md); part B section 6 |
| Kochhar, Thung, Lo, SANER 2015 (coverage versus real bugs on HttpClient and Rhino) | Search-engine summary only; the IEEE page failed to open | Part A section 1.1 and bibliography row 6 |
| Hemmati, QRS 2015, "How effective are code coverage criteria?" | Bibliographic entry only | Part A section 1.1 |
| Papadakis et al., ICSE 2016 subsumption figure | Reached only through the survey; plan lists it as not built on | Part A section 7, gap 1; plan "Sources the plan does NOT build on" |
| Schuler and Zeller details beyond the abstract | Only the abstract was read | Plan "Sources the plan does NOT build on" |
| Snapshot-testing literature (Fujita et al., ICSME 2023) | Search excerpts only | [`pitfalls.md`](../project_plans/test-effectiveness-critique/research/pitfalls.md) |
| Google "change-detector tests" post body | The fetch returned comments only, not the article text | [`pitfalls.md`](../project_plans/test-effectiveness-critique/research/pitfalls.md) |
| Sonar rule S2699 | Page not fetchable | `stack.md` section 3 |
| Develocity Predictive Test Selection | Search snippet only, not opened | `stack.md` section 4 |
| Garousi and Kucuk, JSS 2018 (smells in test code survey) | Metadata only; content not retrieved | Part B bibliography row 26 |
| Parry et al., TOSEM 2022 (flaky-test survey) content | Record verified, PDF not opened | Part B section 5 |
| Palomba "test-suite redundancy" paper | Not located; the Palomba paper opened concerns code smells | Part B section 2.3 |
| Assertion padding as an observed agent behavior | No source opened | Plan "Sources the plan does NOT build on" |

## Negative-result report

Results are pending the experiment. This section is reserved for the mandatory
per-arm record of which hypotheses were not supported, with numbers, and which
claims are withdrawn; no results are recorded here yet.
