# Build vs. Buy: test-effectiveness analysis

**Date**: 2026-10-11 | **Phase**: SDD 2 (research) | **Question**: build in kibitzer, or adopt/ingest existing tools?

Confidence labels: VERIFIED = I ran a command or opened the URL this session; INFERRED = from background knowledge or reasoning, not re-checked.

## Recommendation (read this first)

**Ingest the expensive dynamic parts; build the cheap algorithmic and static parts.** Concretely:

| Layer | Decision | Why |
|---|---|---|
| Mutation engines (operators, instrumentation, test-running) | **Buy / ingest** — never build (already Out of Scope) | Language-aware, years of tuning; every target language already has a maintained engine |
| Per-test coverage collection | **Ingest** (coverage.py contexts, lcov/Go profiles produced per test by the user's CI) | Instrumentation is per-language; kibitzer cannot own it |
| Analysis over kill/coverage matrices (unique coverage, greedy set cover, subsumption, ranking) | **Build in Rust** | Small, well-specified, language-agnostic once the input is a normalized matrix; keeps provenance and "no code leaves the machine" |
| Static assertion/smell detection | **Build narrowly in Rust (tree-sitter), ingest linters opportunistically** | Kibitzer already has per-file static checkers and the hook-latency budget; third-party smell detectors are Java-centric, GPL, or style-only |
| SaaS (Codecov, Develocity, Sonar, Stryker Dashboard) | **Not recommended in the mechanical path** | Violates the data-residency constraint or does not answer the question |
| Fork a "test minimizer" | **Not recommended** | Nothing mature found; research artifacts are throwaway |

The unifying seam is a **normalized test-result matrix** (test id → covered lines / killed mutants) with tool+version provenance, produced by thin adapters from PIT XML, Stryker JSON, cargo-mutants `outcomes.json`, coverage.py contexts, lcov/Go profiles. Everything downstream is kibitzer-owned.

---

## 1. Existing OSS

### 1a. Maintenance / license snapshot (all VERIFIED via `gh api repos/<r>` and `.../releases/latest`, 2026-10-11)

| Tool | Domain | License | Last push | Latest release | Stars |
|---|---|---|---|---|---|
| [hcoles/pitest](https://github.com/hcoles/pitest) (PIT) | Java mutation | Apache-2.0 | 2026-10-09 | 1.30.0 (2026-08-27) | 1873 |
| [STAMP-project/pitest-descartes](https://github.com/STAMP-project/pitest-descartes) | Java extreme/pseudo-tested mutation | **LGPL-3.0** | 2026-09-29 | 1.3.4 (2025-09-19) | 127 |
| [stryker-mutator/stryker-js](https://github.com/stryker-mutator/stryker-js) | JS/TS mutation | Apache-2.0 | 2026-10-10 | v10.0.0 (2026-08-14) | 3198 |
| [stryker-mutator/stryker-net](https://github.com/stryker-mutator/stryker-net) | C# mutation | Apache-2.0 | 2026-10-12 | n/c | 2101 |
| [sourcefrog/cargo-mutants](https://github.com/sourcefrog/cargo-mutants) | Rust mutation | MIT | 2026-10-08 | v27.1.0 (2026-06-02) | 1329 |
| [boxed/mutmut](https://github.com/boxed/mutmut) | Python mutation | BSD-3 | 2026-09-12 | 3.8.0 (2026-09-12) | 1473 |
| [avito-tech/go-mutesting](https://github.com/avito-tech/go-mutesting) | Go mutation | MIT | 2026-10-02 | v2.3.1 (2025-12-26) | 277 |
| [go-gremlins/gremlins](https://github.com/go-gremlins/gremlins) | Go mutation | Apache-2.0 | 2026-06-26 | v0.6.0 (2025-12-06) | 439 |
| [mull-project/mull](https://github.com/mull-project/mull) | C/C++ (LLVM) mutation | Apache-2.0 | 2026-09-13 | 0.34.1 | 845 |
| [agroce/universalmutator](https://github.com/agroce/universalmutator) | regex mutation, any language | NOASSERTION | 2026-05-20 | v.1.14.1 | 164 |
| [nedbat/coveragepy](https://github.com/nedbat/coveragepy) | Python coverage + contexts | Apache-2.0 | 2026-10-11 | 7.16.2 (2026-09-27) | 3416 |
| [jacoco/jacoco](https://github.com/jacoco/jacoco) | Java coverage | NOASSERTION (EPL-2.0 per project docs, INFERRED) | 2026-10-10 | v0.8.15 | 4608 |
| [tarpas/pytest-testmon](https://github.com/tarpas/pytest-testmon) | Python test selection (per-test dep DB) | MIT | 2025-12-01 | n/c | 1022 |
| [TestSmells/TSDetect](https://github.com/TestSmells/TSDetect) | Java test smells | **GPL-3.0** | 2026-06-08 | none | 10 |
| [TestSmells/TestSmellDetector](https://github.com/TestSmells/TestSmellDetector) | Java test smells | GPL-3.0 | 2022-10-01 | n/c | 80 |
| [JetBrains-Research/PyNose](https://github.com/JetBrains-Research/PyNose) | Python test smells | Apache-2.0 | 2022-02-23 | n/c | 57 |
| [jest-community/eslint-plugin-jest](https://github.com/jest-community/eslint-plugin-jest) | JS/TS test lint | MIT | 2026-10-12 | v29.16.11 (2026-10-10) | 1174 |
| [Antonboom/testifylint](https://github.com/Antonboom/testifylint) | Go testify lint | MIT | 2026-10-01 | v1.6.4 (2025-09-12) | 173 |
| [stryker-mutator/mutation-testing-elements](https://github.com/stryker-mutator/mutation-testing-elements) | mutation report JSON schema | Apache-2.0 | 2026-10-12 | v3.9.0 | 29 |

Kotlin: no dedicated engine in this list. PIT works on JVM bytecode and has a Kotlin plugin (arcmutate/pitest-kotlin, INFERRED, not opened) — treat Kotlin as "PIT with caveats", unverified.

### 1b. What each emits (the integration-critical part)

| Source | Per-test attribution? | Machine-readable output | Verified how |
|---|---|---|---|
| **PIT** | **Yes, optionally.** XML has `<killingTest>`, `<killingTests>`, `<succeedingTests>`, `<coveringTests>` per mutation. Full kill matrix needs `fullMutationMatrix=true`, documented as "partially supported feature added due to demand from the research community" and other features "not guaranteed to work correctly" | `mutations.xml` | VERIFIED: tags in [XMLReportListener.java](https://github.com/hcoles/pitest/blob/master/pitest-entry/src/main/java/org/pitest/mutationtest/report/xml/XMLReportListener.java); option text at [pitest.org/quickstart/maven](https://pitest.org/quickstart/maven/) |
| **Stryker (JS/.NET/Scala)** | **Yes.** Report schema has per-mutant `coveredBy` and `killedBy` (test IDs) plus `testFiles[].tests[]` with `id`/`name`; also `testsCompleted` | `mutation-testing-report.json`, standard schema ([schema](https://github.com/stryker-mutator/mutation-testing-elements/blob/master/packages/report-schema/src/mutation-testing-report-schema.json)) | VERIFIED: grepped the schema file (lines 43, 64, 113-135). Whether a given Stryker runner populates `killedBy`/`coveredBy` fully is INFERRED (depends on runner/coverage analysis mode). |
| **cargo-mutants** | **No per-test kill attribution** as far as the docs show: `outcomes.json` has per-mutant outcome (caught/missed/timeout/unviable) and log file, not which test killed it | `mutants.out/{mutants,outcomes}.json`, `caught.txt`, `missed.txt` | VERIFIED: [mutants.rs/mutants-out](https://mutants.rs/mutants-out.html) lists the files; absence of per-test info is INFERRED from that listing, not from reading the JSON. |
| **mutmut 3.x** | Unknown — not opened. It does map mutants to covering tests internally (INFERRED). Output is its own cache/`mutmut results`, not a standard schema | cache dir + CLI | UNVERIFIED |
| **go-mutesting / gremlins** | Unknown — not opened. Both are mutant-vs-whole-package-test-run tools; per-test kill info is unlikely (INFERRED) | stdout / JSON (gremlins has `--output`) | UNVERIFIED |
| **coverage.py contexts** | **Yes, the best per-test coverage source found.** `dynamic_context = test_function` starts a new context per test function; `--contexts` filters reports | SQLite `.coverage` + `coverage json --show-contexts` | VERIFIED: [contexts docs](https://coverage.readthedocs.io/en/latest/contexts.html) lines 146-191 |
| **JaCoCo** | Not natively. Session-based; per-test needs one execution per test or a session-per-test agent hack | `.exec` binary, XML/CSV reports | INFERRED |
| **Go coverage** | Per-run `-coverprofile`; per-test means one `go test -run '^TestX$'` per test (matches the requirements' rabbit hole) | text coverprofile | INFERRED (consistent with requirements.md) |
| **pytest-testmon** | Stores per-test file/line dependency DB in `.testmondata` | SQLite | VERIFIED existence via README; schema not opened |
| **Linters** (eslint-plugin-jest `expect-expect`, `no-conditional-expect`; Sonar S2699 "tests should include assertions") | Static, per-file | ESLint JSON / Sonar issues | rule *names* INFERRED, not opened |

### 1c. Thin ingest layer needed

1. A `TestMatrix` model: `tests: Vec<TestId>`, `units: Vec<Unit>` (line/branch id or mutant id), sparse `covers` / `kills` relation, plus `provenance { tool, version, run_at, language }` (satisfies the Observability requirement).
2. Adapters, in priority order: **Stryker JSON** (one adapter covers JS/TS/C#/Scala via a stable, versioned schema — highest leverage), **PIT XML** (Java/Kotlin), **coverage.py contexts JSON** (Python), **lcov + Go coverprofile with a user-supplied test-id convention** (per-test files), **cargo-mutants** (mutant-level only: yields mutation score and surviving mutants, not per-test kills).
3. The ingest is *file-based* (resolves Open Question 1 toward "ingest report"): kibitzer never launches mutation engines in v1; a documented recipe or optional `kibitzer test-effectiveness collect` wrapper can shell out later.

### Per-option verdicts

**Option 1: wrap mutation engines (PIT/Descartes, Stryker, cargo-mutants, mutmut, go-mutesting/gremlins)**
- Pros: only validated source of fault-detection signal; all actively maintained (see table); Stryker schema is a real cross-language standard; permissive licenses (Apache/MIT/BSD) allow redistributing adapters.
- Cons: cost and wall-clock are theirs, not ours; per-test kill attribution is uneven (Stryker/PIT yes, cargo-mutants no, Go tools unverified); PIT's full matrix is "partially supported"; Descartes is LGPL-3.0 (fine as an external tool that kibitzer only reads output from, not linked).
- **Verdict: Recommended (ingest, never embed).**

**Option 2: per-test coverage (coverage.py contexts, JaCoCo, Go)**
- Pros: coverage.py contexts is first-class and cheap to enable; unique-coverage is the owner's named signal.
- Cons: Go/JaCoCo/Rust need N runs; flaky/order-dependent tests corrupt attribution (requirements.md rabbit hole).
- **Verdict: Recommended for Python (contexts), Viable elsewhere via user-produced per-test profiles; Not recommended for kibitzer to orchestrate N-run collection in v1.**

**Option 3: test-smell detectors (tsDetect, eslint-plugin-jest, testifylint)**
- Pros: eslint-plugin-jest is healthy and MIT; cheap static signal.
- Cons: TSDetect/TestSmellDetector are GPL-3.0 (cannot port code into kibitzer's license without a decision), Java/JUnit-only (INFERRED), tiny and stale (10 / 80 stars; last release none / 2022); PyNose stale since 2022; testifylint is a style linter (`assert.Equal` arg order etc.), not an effectiveness signal. Smell validity vs. fault detection is also weakly evidenced per the requirements.
- **Verdict: Not recommended as a dependency. Viable only as "also run eslint-plugin-jest and surface its `expect-expect` family"; port the *ideas* (cite the literature) rather than the code.**

---

## 2. SaaS

| Product | What it gives | Fit |
|---|---|---|
| **Stryker Dashboard** | Free for OSS, stores `mutation-testing-report.json` (or score-only `{mutationScore: N}`) and badges ([docs](https://stryker-mutator.io/docs/General/dashboard/), VERIFIED) | Uploads report content, which includes source and test names, off-machine. Adds nothing to analysis. Not recommended in the mechanical path. |
| **Codecov** | Coverage trends; Test Analytics reports failed/flaky tests as PR comments ([docs](https://docs.codecov.com/docs/test-analytics), VERIFIED snippet) | Flakiness/failure analytics, not redundancy or kill power. Uploads coverage data. Not recommended. |
| **Develocity Predictive Test Selection** (Gradle) | Runs only tests relevant to a change, records why each was selected/skipped ([page](https://gradle.com/develocity/product/predictive-test-selection/), VERIFIED) | Optimizes *which tests to run*, not *which tests to delete*. Commercial, build-tool-locked (Gradle/Maven). Different problem. Not recommended. |
| **Launchable / CloudBees Smart Tests** | Same category as PTS (ML test selection) | INFERRED from background; not opened. Same verdict. |
| **SonarQube/SonarCloud** | Rule "tests should include assertions" (S2699) and other test rules | INFERRED; page fetch failed. Already a static lint; user's CI may have it. Could be ingested via SARIF later, low priority. |
| **Meticulous** | UI regression test generation | Out of scope (E2E/UI, test generation) |

Cost / residency / lock-in summary: every SaaS above sends coverage, test names, or reports off the machine — conflicts with the "no code leaves the machine in the mechanical path" constraint (requirements.md, Data residency). None computes redundancy or unique-kill power; they target test *selection* and *flakiness*. **Verdict: Not recommended.** Self-hosted OSS dashboards (mutation-testing-elements HTML) are fine as optional *output* targets: emitting a Stryker-schema report from kibitzer would let users view results locally.

---

## 3. LLM-written bespoke vs. battle-tested library

Rule of thumb: hand-roll things that are pure functions over a normalized matrix and are fully specified and testable by property tests; do not hand-roll anything that must understand a language's semantics or a runtime's instrumentation.

| Piece | Safe in Rust? | Rationale |
|---|---|---|
| (a) Unique/incremental coverage per test; greedy set-cover minimization | **Safe, build.** | Input: `HashMap<TestId, BitSet>`. Greedy set cover is textbook (ln n approximation), ~100 lines, deterministic, testable with property tests (union preserved; result subset-minimal). Use `roaring`/`fixedbitset`-style bitsets for 10^5 tests — small, justified dependency. Caveat: greedy order depends on tie-breaks; document a deterministic tie-break (name order) so findings are reproducible. |
| (b) Mutant subsumption graph / dominator mutants | **Safe at the matrix level, build; do not claim true subsumption.** | From a full kill matrix, "mutant A is dynamically subsumed by B if every test killing B kills A" is a set-inclusion pass, O(m²) naive, fine with bitsets plus sampling. But it is only valid *relative to the given test suite* (dynamic subsumption); true semantic subsumption and equivalent-mutant detection are undecidable/hard — do not attempt. Also depends on PIT `fullMutationMatrix` or Stryker `killedBy` actually listing *all* killers (bail-on-first-failure truncates them) — the adapter must reject or warn on single-killer reports. |
| (c) Static assertion detection per language | **Build narrowly, language by language, with tree-sitter.** | Kibitzer already does per-file static checks. "Test function with zero assertion call / only mock-verify / `assert true`-style tautology" is shallow syntactic matching, but each language needs a curated assertion-API list (testify, `t.Error`, `expect`, `assert*`, `#[should_panic]`, `assert!`) and helpers hide assertions (a call to `checkInvariants(t)`). Expect false positives on assertion helpers; mitigate with cross-function call-graph inclusion (kibitzer has `list_callees` for Go/TS/JS) and an inline-ignore. Precision must be measured on the corpus before default-on (requirements Q5). |
| Mutation operators (AOR, ROR, statement deletion, etc.) | **Reckless.** | Needs AST/bytecode-correct rewriting, compilability filtering, equivalent/trivial-mutant avoidance, build integration, timeouts. Years of tuning in the tools above. Already Out of Scope. |
| Language-aware coverage instrumentation | **Reckless.** | Per-language runtimes (JVM agent, Go cover rewriting, LLVM source-based coverage); use the native tools. |
| Test-to-code mapping | **Viable, build heuristically.** | `affected.rs` already does diff → tests; reuse. |
| Prediction (ML "predictive mutation testing") | Not recommended | Needs training data and per-project models; far beyond scope; INFERRED from literature knowledge, not re-reviewed here (see the literature worker's report). |

## 4. Fork / adapt

Searched GitHub (`gh search repos`, VERIFIED, 2026-10-11) for "test suite minimization", "test suite reduction tool", "redundant test detection", "mutant subsumption", "test redundancy coverage", "test smell detector".

| Hit | Status | Verdict |
|---|---|---|
| [anisbelkacem/Test_Suite_Minimization](https://github.com/anisbelkacem/Test_Suite_Minimization), [clasick/PyMinimize](https://github.com/clasick/PyMinimize), [FoadG/LLM-Assisted-Mutation-Testing-...](https://github.com/FoadG/LLM-Assisted-Mutation-Testing-with-Automated-Test-Suite-Minimization), [jwlin/Nemo](https://github.com/jwlin/Nemo) | 0–1 stars, single-author, research/student artifacts, no license on several; not opened beyond metadata | Not recommended as a base; consult only as reference for an algorithm |
| [tonypunnacherry/Dynamic-Mutant-Subsumption](https://github.com/tonypunnacherry/Dynamic-Mutant-Subsumption), [dzt2/SubsumeTest](https://github.com/dzt2/SubsumeTest) | 0 stars, research artifacts (2025 / 2017) | Not recommended; the algorithm is simple enough to re-derive |
| [TestSmells/TestSmellDetector](https://github.com/TestSmells/TestSmellDetector), [TSDetect](https://github.com/TestSmells/TSDetect) | GPL-3.0, Java-only (INFERRED), stale/tiny | Reference for smell definitions only |
| [pytest-testmon](https://github.com/tarpas/pytest-testmon) | MIT, maintained to 2025-12 | Not a redundancy detector; but its per-test dependency DB is a plausible *Python input source* — Viable as an ingest source if coverage.py contexts prove awkward |
| [EvoSuite](https://github.com/EvoSuite/evosuite) | LGPL-3.0, last push 2025-02-14 — effectively stalled | Out of scope (test generation) |

No "test value" analyzer close enough to fork was found. Search coverage gap: I did not search academic artifact repositories (Zenodo, paper-specific repos) systematically; the literature worker's citations may surface artifacts. **Verdict: Not recommended** (no viable base; the algorithms in §3 are small).

---

## Risks and open items

- **Licensing**: GPL-3.0 detectors cannot be ported into kibitzer without checking kibitzer's own license (not checked here). Treat smell *definitions* from papers as ideas, not code.
- **Per-test kill attribution is the weakest link**: only Stryker (schema-level) and PIT (opt-in, partial support) are verified to emit it; cargo-mutants (Rust, and kibitzer itself is Rust) is verified-by-listing to lack it. For Rust, the viable signals are static + coverage-based; test-level mutation analysis would need a different path (e.g. re-run per test, expensive). Resolve before committing to a "first-class language" list (Open Question 4).
- **Unverified**: mutmut, go-mutesting, gremlins output schemas; JaCoCo per-test; Kotlin mutation story; Sonar rule IDs; Launchable. Each needs a 10-minute read before implementation planning relies on it.
- **Fit with the central tension**: ingest-only keeps kibitzer advisory and out of the hook path, consistent with the Risk Control section; the static assertion check is the only piece that may belong in `default_checks()`.

## Sources opened or run this session

- `gh api repos/<r>` and `releases/latest` for every repo in §1a (output in session)
- https://github.com/hcoles/pitest/blob/master/pitest-entry/src/main/java/org/pitest/mutationtest/report/xml/XMLReportListener.java
- https://pitest.org/quickstart/maven/
- https://github.com/stryker-mutator/mutation-testing-elements/blob/master/packages/report-schema/src/mutation-testing-report-schema.json
- https://mutants.rs/mutants-out.html
- https://coverage.readthedocs.io/en/latest/contexts.html
- https://stryker-mutator.io/docs/General/dashboard/
- https://docs.codecov.com/docs/test-analytics
- https://gradle.com/develocity/product/predictive-test-selection/
- `gh search repos` queries listed in §4
