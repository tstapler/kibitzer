# Research: Stack / tooling inventory for mechanical test-effectiveness signals

**Date**: 2026-10-11 (all "last release/push" dates read from `gh api repos/<r>` and `.../releases/latest` on this date)
**Question**: per language, what tooling produces the raw data (per-test coverage, mutant kill data, smell findings, reduction/selection data), and can an ingest-only kibitzer (Rust) consume it?
**Labels**: VERIFIED = I opened the source or ran the command (cited). INFERRED = reasoned from verified facts. UNVERIFIED = not checked.

## 0. Headline findings

1. **Per-test coverage is cheap in exactly two ecosystems**: Python (coverage.py dynamic contexts: one run, per-test line sets in SQLite/JSON) and any lcov producer that emits `TN:` records. Go gets it by one `go test -run '^T$' -coverprofile` per test (compile is cached, so a loop is feasible but O(tests) process launches). JVM (JaCoCo), Rust (llvm-cov) and JS (c8/istanbul) need per-test process isolation or dump/reset hooks. See section 1.
2. **Only two mutation tools emit a real per-mutant killing-test record**: Stryker (`killedBy` / `coveredBy` test ids, mutation-testing-elements schema) and PIT (`killingTest`, `killingTests`, `succeedingTests`, `coveringTests` in XML; full matrix needs `fullMutationMatrix=true`). mutmut records per-function test mapping plus per-mutant exit code. cargo-mutants, gremlins, go-mutesting, cosmic-ray record per-mutant status only (killing test is only inside free-text logs). See section 2.
3. **Format-wise the best ingest targets are**: Stryker/mutation-testing-elements JSON (one schema, many producers), PIT `mutations.xml`, lcov (with `TN:`), Go cover profile, coverage.py JSON with `--show-contexts`. Details and ranking in sections 4 and 5.
4. **Local availability is poor for mutation tools**: none of cargo-mutants, mutmut, cosmic-ray, gremlins, go-mutesting, stryker, mvn are installed. Go, Python+coverage.py, cargo-llvm-cov, golangci-lint, node, java, gradle, kotlinc are. See section 6.

## 1. Coverage tools (per language)

| Lang | Tool (version, last release) | Output formats | Per-test attribution | Cost of per-test | Status |
|---|---|---|---|---|---|
| Python | coverage.py 7.16.2 (2026-09-27, pushed 2026-10-11) | SQLite `.coverage` (schema v7), JSON, XML, lcov, HTML | **Yes, native**: `dynamic_context = test_function` or pytest-cov `--cov-context=test`; contexts stored per file in `line_bits.numbits` | One test run (small overhead) | Active |
| Go | `go test -coverprofile` (go 1.26.4 local); `go build -cover` + `GOCOVERDIR` + `go tool covdata` for binaries | Go cover profile text (`mode:` + `file:l.c,l.c nstmt count`); covdata binary dir, convertible to text | Only by running one test at a time (`-run '^Name$'`, one profile each) | O(tests) `go test` launches; build is cached | Stdlib |
| TS/JS | c8 12.0.0 (2026-07-14), nyc 18.0.0 (2026-02-22), `node --experimental-test-coverage` | V8 JSON (`NODE_V8_COVERAGE`), istanbul `coverage-final.json`, lcov, text | No per-test in c8 itself (reporters: Istanbul-compatible). Jest/Vitest have their own coverage providers; per-test needs isolated runs | O(tests) or custom reporter | Active |
| Java | JaCoCo 0.8.15 (2026-06-05) | `.exec` binary, XML (`report.dtd`), CSV, HTML | Not built in; agent supports TCP/JMX **dump + reset** so a test listener can snapshot per test | Needs a JUnit listener per test; slows suite | Active |
| Kotlin | Kover 0.9.11 (2026-09-29, README labels it Beta); JaCoCo also usable | HTML, XML (JaCoCo-style), binary | Not built in | Same as JaCoCo | Active, beta |
| Rust | cargo-llvm-cov 0.9.1 (2026-09-06) | `--json` (llvm-cov export), `--lcov`, `--cobertura`, `--codecov`, `--text`, `--html`; nextest supported | No flag. Possible via one process per test (nextest model) with distinct `LLVM_PROFILE_FILE` | INFERRED: per-process profraw merge | Active |

Evidence:
- coverage.py contexts: <https://coverage.readthedocs.io/en/latest/contexts.html>; schema: <https://coverage.readthedocs.io/en/latest/dbschema.html> (VERIFIED, opened).
- **Ran it locally** (coverage 7.13.4, `/usr/bin/coverage`, `dynamic_context = test_function`, 3 tests): `coverage json --show-contexts` produced `{"2": [test_pos, test_neg, test_noassert], "4": [test_pos, test_noassert], "3": [test_neg]}` and `sqlite3 .coverage` showed one context row per test. VERIFIED (scratchpad `pymod/`). Note: a test with no assertion still gets coverage credit, which is exactly the signal of interest.
- **Ran it locally** (Go 1.26.4): full-suite profile vs per-test `-run` profiles gave disjoint counts per test (`TestPos` covers the non-negative branch, `TestNeg` covers the negation branch). VERIFIED (scratchpad `gomod/`). `go test -json` also gives per-test pass/fail/elapsed.
- lcov `TN:<test name>` record precedes `SF:` in a tracefile section: <https://github.com/linux-test-project/lcov/blob/master/docs/man/geninfo.rst> (VERIFIED, lines ~689-718). Per-test lcov exists in the format; whether a given producer writes `TN:` is per tool (c8/istanbul/llvm-cov: UNVERIFIED, likely empty `TN:`).
- JaCoCo dump/reset: <https://github.com/jacoco/jacoco/blob/master/org.jacoco.doc/docroot/doc/faq.html>, `agent.html` (VERIFIED: "execution data reset and dump" exist; per-test use is INFERRED).
- cargo-llvm-cov formats: <https://github.com/taiki-e/cargo-llvm-cov/blob/main/README.md> (VERIFIED).
- c8: <https://github.com/bcoe/c8/blob/main/README.md> (VERIFIED, "compatible with Istanbul's reporters").
- Kover: <https://github.com/Kotlin/kotlinx-kover/blob/main/README.md> (VERIFIED: HTML and XML reports, Beta badge).
- Go covdata: `go doc cmd/covdata` (VERIFIED, local).

## 2. Mutation tools

| Lang | Tool (version, last release, last push) | Maintenance | Report formats | Kill data granularity | Notes |
|---|---|---|---|---|---|
| Java | **PIT/pitest 1.30.0** (2026-08-27, push 2026-10-09) | Very active (1.9k stars) | HTML, **XML**, CSV (`outputFormats`) | **Per-mutant**: XML tags `killingTest`, `killingTests`, `succeedingTests`, `coveringTests`; `fullMutationMatrix=true` continues after first failure to record all failing tests ("partially supported feature added due to demand from the research community") | Incremental history file; `mutationThreshold`, `coverageThreshold` gates |
| Java | Descartes (PIT engine, extreme mutation) 1.3.4 (2025-09-19, push 2026-09-29) | Active, small | PIT formats | Same as PIT | Finds "pseudo-tested" methods |
| Java | Major 3.0.1 (site `Last-Modified` 2024-09-27) | Low activity, academic, not on GitHub | Own CSV logs | Computes "complete mutant-test detection matrix" per its site | Research-grade; kill matrix is the only listed matrix producer |
| Kotlin | PIT via Gradle plugin; `pitest-kotlin` plugin **archived** (last push 2023-03-01) | Kotlin support status UNVERIFIED beyond the archived repo | PIT formats | PIT | Arcmutate (commercial) not checked |
| TS/JS | **StrykerJS 10.0.0** (2026-08-14, push 2026-10-10) | Very active (3.2k stars) | html, **json** (mutation-testing-elements), clear-text, dashboard | **Per-mutant `killedBy[]` and `coveredBy[]` test ids** plus `testFiles` dictionary; `coverageAnalysis` default `perTest` | Incremental mode exists (`incrementalFile`) |
| Python | **mutmut 3.8.0** (2026-09-12) | Active | `.meta` JSON per source file (`exit_code_by_key`), `mutants/mutmut-stats.json` (`tests_by_mangled_function_name`) | Per-mutant exit code; **per-function** test mapping, no per-mutant killing test | v3 only mutates functions; needs fork (WSL on Windows) |
| Python | cosmic-ray 8.7.0 (2026-08-09) | Active, smaller | SQLite session (WorkItem/WorkResult: `test_outcome`, `output`, `diff`), `cr-report`, `cr-xml`, `cr-html` | Per-mutant outcome (killed/survived/incompetent); output text holds failure | Distributed execution |
| Go | **gremlins 0.6.0** (2025-12-06, push 2026-06-26) | Slow but alive | `--output <file>` JSON: per-file `mutations[{line,column,type,status}]` plus totals (`test_efficacy`, `mutations_coverage`) | Per-mutant status only (KILLED/LIVED/NOT COVERED/NOT VIABLE...) | Uses coverage to skip uncovered code; `--diff` ref supported |
| Go | avito-tech/go-mutesting v2.3.1 (2025-12-26, push 2026-10-02) | Alive fork | `json_output`, `html_output` config flags | Per-mutant killed/not | Upstream `zimmski/go-mutesting` last push 2024-07-04 (stale) |
| Go | ooze (gtramontina) push 2026-09-17, no GitHub releases | Alive | Test-embedded API (`ooze_mutation_test.go`), report format not verified | Not verified | Run as a Go test |
| Rust | **cargo-mutants 27.1.0** (2026-06-02, push 2026-10-08) | Very active | `mutants.out/`: `mutants.json`, `outcomes.json`, `caught.txt`, `missed.txt`, `timeout.txt`, `unviable.txt`, `diff/`, `logs/` | Per-mutant outcome; killing test only in per-mutant log (cargo output) | `--in-diff` targets mutants overlapping a diff; docs warn format "is subject to change" and that a diff touching only test code runs no mutants |

Evidence:
- Stryker schema: <https://github.com/stryker-mutator/mutation-testing-elements/blob/master/packages/report-schema/src/mutation-testing-report-schema.json> (VERIFIED, read in full: `killedBy`, `coveredBy`, `testsCompleted`, `statusReason`, `testFiles.*.tests[{id,name,location}]`, status enum `Killed|Survived|NoCoverage|CompileError|RuntimeError|Timeout|Ignored|Pending`). Schema package v3.9.0 (2026-07-27).
- Stryker config (`coverageAnalysis` default `perTest`, json reporter): <https://stryker-mutator.io/docs/stryker-js/configuration/> (VERIFIED).
- PIT XML tags: <https://github.com/hcoles/pitest/blob/master/pitest-entry/src/main/java/org/pitest/mutationtest/report/xml/XMLReportListener.java> (VERIFIED: tags `killingTest, killingTests, succeedingTests, coveringTests`, attrs `detected`, `status`, `numberOfTestsRun`). `fullMutationMatrix` and `outputFormats`: <https://pitest.org/quickstart/maven/> (VERIFIED).
- cargo-mutants: <https://github.com/sourcefrog/cargo-mutants/blob/main/book/src/mutants-out.md>, `in-diff.md` (VERIFIED).
- mutmut: <https://github.com/boxed/mutmut/blob/main/ARCHITECTURE.rst> and `src/mutmut/mutation/data.py` (`exit_code_by_key`), `src/mutmut/models/results.py` (`tests_by_mangled_function_name`) (VERIFIED). Format is internal, not a documented interchange format (INFERRED: "contents subject to change" is not stated for mutmut, but there is no docs promise either).
- cosmic-ray: `src/cosmic_ray/work_item.py`, `work_db.py` (SQLite via SQLAlchemy), docs `reference/cli.rst` (cr-report/cr-xml/cr-html) (VERIFIED).
- gremlins: `cmd/unleash.go` (`--output` "set the output file for machine readable results"), `internal/report/testdata/normal_output.json` (VERIFIED).
- go-mutesting: <https://github.com/avito-tech/go-mutesting/blob/master/README.md> config table (VERIFIED).
- Major: <https://mutation-testing.org/> ("computing a complete mutant-test detection matrix"; version 3.0.1) and `curl -I` Last-Modified (VERIFIED).
- Release/push dates: `gh api repos/<repo>` and `/releases/latest` run 2026-10-11 (VERIFIED).

**Per-test kill matrix summary**: full test x mutant matrix is available from PIT (with `fullMutationMatrix`) and Major; Stryker gives `killedBy` as recorded (best practice is bail on first failure, so usually one test per mutant unless configured otherwise; the schema text says so). Everything else gives mutant status only. For "mutants uniquely killed by test T" on Go/Rust/Python, one would need to rerun per test (INFERRED, expensive) or use mutmut's function-to-tests map as a proxy.

## 3. Test-smell / test-quality detectors (reuse vs rule inspiration)

| Lang | Tool | Rules relevant to *ineffective* tests | Maintenance | Reuse verdict |
|---|---|---|---|---|
| JS/TS | eslint-plugin-jest 29.16.11 (2026-10-10) | `expect-expect` (test has no assertion), `no-conditional-expect`, `no-standalone-expect`, `valid-expect`, `no-disabled-tests`, `no-commented-out-tests`, `no-identical-title`, `no-conditional-in-test`, `prefer-expect-assertions`, `no-unnecessary-assertion` | Very active | Rule inspiration (kibitzer is not an ESLint host); could ingest ESLint JSON output |
| Go | testifylint v1.6.4 (2025-09-12) | `useless-assert` (self-comparison, constant assertions), `bool-compare`, `empty`, `len`, `error-nil`, `require-error`, `suite-extra-assert-call` | Active | Inspiration; ingestable via golangci-lint JSON |
| Go | thelper v0.7.1 (2025-09-17) | helper hygiene (`t.Helper()`), not effectiveness | Maintained, tiny | Low value |
| Rust | Clippy 0.1.98 (local) | `assertions_on_constants` (warn), `bool_assert_comparison` (warn), `assertions_on_result_states` (allow), `manual_assert`, `missing_assert_message` (allow), `ignore_without_reason`, `redundant_test_prefix`, `tests_outside_test_module` (all allow) | Very active | Ingestable (`--message-format=json`); few effectiveness rules |
| Java | tsDetect (CLI + IntelliJ plugin; repo TestSmells/TSDetect-Plugin push 2026-06-08; original TestSmellDetector last push 2022-10-01, v2.2) | 21 smells from the academic catalogue (e.g. Unknown Test, Empty Test, Redundant Assertion, Ignored Test, Magic Number); CSV in/out | Research tool, low activity | Rule inspiration (catalogue); JVM dependency not worth ingesting |
| Python | PyNose (ASE 2021; PyCharm plugin; unittest only; arXiv 2108.04639) | 17 smells + "Suboptimal Assert" | Source location/maintenance UNVERIFIED | Inspiration only |
| Python | ruff flake8-pytest-style (`PT*`) | assertion-style rules | UNVERIFIED (not opened) | Not assessed |
| Multi | Sonar test rules (e.g. S2699 "Tests should include assertions") | assertion-less tests | Page not fetchable here | UNVERIFIED |

Evidence: eslint-plugin-jest `docs/rules` listing via `gh api repos/jest-community/eslint-plugin-jest/contents/docs/rules` and `expect-expect.md` (VERIFIED); testifylint README checkers table incl. `useless-assert` (VERIFIED, <https://github.com/Antonboom/testifylint>); clippy lints via `clippy-driver -W help` locally (VERIFIED); TestSmellDetector README and TSDetect-Plugin README (VERIFIED, but the 21-smell count and Java-only scope come from the testsmells.org project, UNVERIFIED here; the READMEs I opened describe a CSV-driven detector for unit tests); PyNose facts from search result summary of <https://arxiv.org/abs/2108.04639> (via WebSearch, not opened).

Observation (INFERRED): none of these measures effectiveness, they measure *syntactic* smells; they are the "static-only proxies" from the requirements' rabbit-holes list. They are the cheapest signals and the natural hook-time candidates (assertion-less test, tautological assert).

## 4. Test-suite reduction / selection / analysis

| Tool | What it provides | Maintenance (VERIFIED dates) | Relevance |
|---|---|---|---|
| pytest-testmon 2.2.0 | Python: tracks which tests touch which code, selects affected tests; stores per-test dependency data (SQLite, internal) | release 2025-12-01 | Source of per-test dependency data for Python; internal format (INFERRED) |
| coverage.py contexts | Same data, documented schema | active | Preferred Python source |
| Bazel 9.3.0 | `bazel coverage` yields per-test-target lcov; tests are hermetic and individually cacheable | 2026-10-07 | Test-*target* granularity only (INFERRED); not per test function |
| Launchable, now CloudBees Smart Tests (`cloudbees-oss/smart-tests-cli` v2.16.1, 2026-10-09; `launchableinc/cli` redirects here) | SaaS predictive test selection ingesting test-result history | active | SaaS; not mechanical/offline; out of scope as dependency |
| Develocity Predictive Test Selection | ML selection; Gradle and Maven only per compatibility docs | commercial | Not usable for corpus experiment |
| `go test -run`, `go test -json` | Per-test selection, results, timings | stdlib | Basis of Go per-test profile loop (verified above) |
| cargo-nextest | One process per test; JUnit; cargo-llvm-cov supports it | **not installed locally** | Natural per-test isolation point in Rust (INFERRED) |

Evidence: pytest-testmon repo metadata (VERIFIED via `gh api`); Bazel version (VERIFIED via `gh api`); Develocity PTS Gradle/Maven (WebSearch snippet citing <https://gradle.com/develocity/product/predictive-test-selection/> and compatibility docs; not opened); cargo-llvm-cov nextest support (README line 12, VERIFIED).

## 5. Machine-ingestable formats (what kibitzer would parse)

| Format | Signal it enables | Producers | Schema stability | Parse effort in Rust | Per-test data? |
|---|---|---|---|---|---|
| **mutation-testing-elements JSON** | per-mutant status, `killedBy`/`coveredBy` test ids, mutant location, test list | StrykerJS (+Stryker.NET, Stryker4s: not checked), and any tool that adopts the schema | Published JSON Schema, versioned (v3.9.0) | Low (serde) | Yes (test ids) |
| **PIT `mutations.xml`** | per-mutant status, killing/covering/succeeding tests, line, mutator | PIT, Descartes | Stable for years (INFERRED), tags enumerated in source | Low-medium (quick-xml) | Yes (with full matrix) |
| **lcov `.info`** | line/branch hits per file; per-test via `TN:` | c8, nyc, llvm-cov, coverage.py, gcov, JaCoCo via jacoco2lcov (lcov ships `jacoco2lcov`, `llvm2lcov`) | Text, stable, documented | Very low | Only if producer writes `TN:` or one tracefile per test |
| **Go cover profile** | per-block statement counts | `go test -coverprofile` | Stable text | Very low | One file per test run |
| **coverage.py JSON `--show-contexts`** | per-line list of test contexts | coverage.py | Documented JSON; SQLite schema versioned (v7) but "can change without major version" | Low (JSON) / medium (SQLite numbits) | **Yes** |
| JaCoCo XML | line/branch/method counters per class | JaCoCo, Kover | DTD in repo | Medium | No |
| llvm-cov export JSON / cobertura / codecov | coverage | cargo-llvm-cov | llvm-cov JSON versioned | Medium | No |
| cargo-mutants `outcomes.json` | per-mutant outcome | cargo-mutants | **Explicitly "subject to change"** | Low | No |
| gremlins JSON | per-mutant status | gremlins | Unversioned | Low | No |
| go-mutesting report.json | per-mutant killed | avito go-mutesting | Unverified | Low | No |
| mutmut `.meta` / `mutmut-stats.json` | exit code per mutant; function-to-tests | mutmut | Internal | Low | Function-level |
| cosmic-ray SQLite | per-mutant outcome | cosmic-ray | Internal (SQLAlchemy models) | Medium | No |
| JUnit XML | per-test pass/fail/time (flakiness, runtime cost) | essentially everything incl. nextest, `gotestsum` (v1.13.0), pytest | De facto | Very low | Yes |

## 6. Local availability (backtest corpus experiment)

Checked with `which` / `--version` on 2026-10-11 (VERIFIED):

| Need | Present? | Detail |
|---|---|---|
| Go toolchain | Yes | go1.26.4 (Linuxbrew); golangci-lint in `~/.local/bin` |
| Rust toolchain, clippy | Yes | cargo 1.98.1, clippy 0.1.98 |
| cargo-llvm-cov | Yes | 0.9.0 (latest is 0.9.1) |
| cargo-mutants | **No** | `cargo mutants` -> "no such command" |
| cargo-nextest | **No** | |
| Python | Yes | python3.14.7; `/usr/bin/coverage` 7.13.4 and `/usr/bin/pytest` (system Python, not the brew one: `import coverage` fails under brew python3) |
| mutmut, cosmic-ray | **No** | |
| Node/npm | Yes | node v26.8.1; **stryker, c8, nyc, jest, vitest not installed** (npx can fetch) |
| JVM | Partial | OpenJDK 25.0.2, gradle, kotlinc present; **mvn missing**; PIT would run via Gradle plugin or CLI jar |
| gremlins, go-mutesting | **No** | installable with `go install` (INFERRED) |
| cargo-dist (`dist`), covgate | Yes | release/coverage-gate tools for kibitzer itself |

Practical implication (INFERRED): without installing anything, the corpus experiment can already do **Python per-test coverage** (coverage.py contexts), **Go per-test coverage** (`-run` loop), and **Rust whole-suite coverage**. Mutation runs need installs: cargo-mutants (Rust: ripgrep), gremlins (Go: stapler-squad, kubernetes subpackages), Stryker via npx (deno/vscode TS, impractical at that scale), PIT (cassandra: huge, sampling needed).

## 7. Recommendation: ingest-only kibitzer subcommand, format priority

Rationale: the central tension in requirements.md is "who runs the tests". An ingest-only model keeps kibitzer a pure parser, never executes repo code, and matches "dynamic signals batch/CI only".

**Tier 1 (build first, covers the two owner-named signals)**
1. **mutation-testing-elements JSON** - the only language-neutral schema carrying per-test kill ids (`killedBy`, `coveredBy`, `testFiles`). One parser serves Stryker now and any future producer. Gives "mutants uniquely killed by test T" directly and, via `coveredBy`, per-test coverage over mutated sites.
2. **PIT `mutations.xml`** (with `fullMutationMatrix=true` documented as the recommended setting) - same signal for Java, and Kotlin via PIT. Could be normalized into the same internal record as (1).
3. **lcov `.info`, one tracefile per test or with `TN:`** - the universal coverage format; unique-coverage-per-test (greedy set cover / unique lines) needs only this plus a "tracefile per test" convention. Covers Rust (`cargo llvm-cov --lcov`), TS/JS (c8/nyc), Python (coverage.py `lcov`), JVM via `jacoco2lcov`.

**Tier 2**
4. **Go cover profile** (one file per test; trivial parser; Go is kibitzer's best-supported language, so its signal quality is highest per effort).
5. **coverage.py JSON with `--show-contexts`** - cheapest real per-test coverage anywhere (one run), best for the Q3 experiment (unique coverage vs unique kills agreement). Could instead be satisfied by converting contexts to per-test lcov in a helper script.
6. **JUnit XML** - per-test runtime/flakiness cost side-data; Very low effort.

**Tier 3 / defer**
- Status-only mutation outputs (cargo-mutants, gremlins, go-mutesting, cosmic-ray, mutmut): give mutation score per file or surviving mutants for the "does this suite kill anything here" question, but not per-test attribution, so they cannot rank tests. `outcomes.json` is explicitly unstable. Support cargo-mutants first among these only if a Rust-specific report is wanted.
- JaCoCo XML / llvm-cov JSON: aggregate-only, no per-test; lcov conversion covers them.
- SaaS selection tools (Smart Tests, Develocity): not mechanical/offline.

**Static smell signals** (no ingest needed, hook-time): port the cheap rules by inspiration (assertion-less test, tautological assertion, conditional expect), not by ingesting ESLint/testifylint output; consider optional ESLint/golangci JSON ingest later.

**Open gap to flag**: nothing found here emits a native per-test mutant kill matrix for Go, Rust or Python. For those languages, "uniquely killed mutants per test" requires either rerunning mutants per test (expensive, kibitzer would have to orchestrate) or accepting unique *coverage* as the proxy. That is Open Question 3 in requirements.md and the corpus experiment should measure it on Python (coverage.py contexts + mutmut's function-to-test map) and Java/TS (PIT/Stryker kill ids) where ground-truth kill ids exist.

## 8. Gaps / not verified

- Stryker.NET / Stryker4s support for the same schema: not checked.
- Whether c8/nyc/llvm-cov ever write non-empty `TN:`: not checked.
- ruff `PT` rules, Sonar S2699, Develocity PTS docs, Arcmutate, ooze report format, Major's on-disk matrix file name: not opened.
- Cost figures (O(tests) process launches) are reasoning from the verified per-test loop, not benchmarked on kubernetes-scale suites.
