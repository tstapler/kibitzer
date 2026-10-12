# Implementation Plan: test-effectiveness-critique

**Feature**: Mechanical, reproducible signals that a test is ineffective: hook-time static vacuity checks plus an ingest-only batch report (unique coverage, unique kills, set cover, subsumption), validated by a corpus deletion experiment before anything is recommended for deletion.
**Date**: 2026-10-11
**Status**: Revised after adversarial, architecture, consistency and pre-mortem review and two triad (product, UX, engineering) passes (see "Repair log", "Phase 4 repair log", "Triad repair log (iteration 1)", "(iteration 2)" and "(iteration 3)" at the end); research-led; owner gates G0/G1/G2 are recorded-decision stories that later work depends on
**ADRs**: [ADR-001](../decisions/ADR-001-ingest-only-dynamic-signals.md), [ADR-002](../decisions/ADR-002-two-surfaces-static-checkers-and-batch-report.md), [ADR-003](../decisions/ADR-003-validation-methodology-before-default-on.md), [ADR-004](../decisions/ADR-004-static-check-scope-and-promotion-bar.md), [ADR-005](../decisions/ADR-005-matrix-algorithms-in-rust-minimal-new-dependencies.md)

Complexity 3. Repo is Rust; line references below are to `master` at `d76c1ab`, taken from `research/architecture.md`.

---

## Step 0.5: Alternatives explored (CREATIVE pass)

| Approach | Strength | Weakness | Verdict |
|---|---|---|---|
| A. Static-only: tree-sitter vacuity checkers in `default_checks()` | Hook-latency safe, serves the agent-just-wrote-a-test consumer, no new inputs | Cannot say "redundant"; validity vs. real fault detection unmeasured | Adopt as surface 1, gated |
| B. Ingest-only batch report over per-test coverage and mutation matrices | Carries the only signals with any (lossy) validation, full provenance, never in hook path | Per-test coverage must be produced by the user; parsers multiply per language | Adopt as surface 2 |
| C. Plugin wrapping external tools | Cheap to build | Per-file 30 s SARIF protocol, machine-local registry (`src/plugin.rs:174-190`, `docs/plugins.md`) | Rejected |
| D. Kibitzer-orchestrated runner (shell out, run per-test, mutate) | One command UX | Becomes a test runner; cost and flake attribution dominate; breaks advisory stance | Rejected (ADR-001) |
| E. LLM judgment per test | Highest recall (the anecdote) | Non-reproducible, unauditable, cost scales with suite | Out of the mechanical path; consumed only as adjudicator of a mechanical shortlist (Story 2.3.4) and an optional pilot (Story 3.3.4) |

Chosen: A + B, with an explicit experiment epic between them and any default-on decision. C and D rejected.

---

## Domain Glossary

| Term | Definition | Notes |
|---|---|---|
| `Test` | One executable test function, the default unit of analysis | Table rows and subtests are not separate `Test`s unless opted in (pitfall P6) |
| `TestId` | Stable identifier of a `Test`: `(repo-relative file, qualified name)` | Newtype, not raw string; adapters normalise tool-specific ids into it |
| `CoverageUnit` | A line or branch identifier a test can cover | Newtype over `(file, line)`; generated files excluded |
| `Mutant` | A single mutation identifier from a mutation tool's report | Newtype `MutantId`; never produced by kibitzer |
| `TestMatrix` | `{ coverage: Option<Dimension<CoverageUnit, CoverageAttribution>>, kills: Option<Dimension<MutantId, KillCompleteness>> }`; each `Dimension` is a sparse inverted index (unit to sorted `u32` test-ordinal posting list) with its own `Provenance` | The single seam every adapter writes and every algorithm reads. A coverage input and a mutation input may come from different tools, runs, and commits, so provenance is per dimension (architecture review concern 3) |
| `Provenance` | Tool name, tool version, run timestamp, commit, language, attribution source | Required on every `Dimension`; adapters reject inputs lacking tool + version. Run timestamp and commit are `Option` (absence is a warning, Story 2.1.1). Attribution is carried per the "Attribution source" rule below the Glossary |
| `CoverageAttribution` | Enum `Isolated` / `InOrder` / `LeaveOneOut`: how per-test coverage was produced | Sum type; printed on every coverage-derived finding (pitfall P3). Meaningless for kills, hence not shared. `LeaveOneOut` is produced only by the Story 3.1.4 script (shortlists, bounded by R_max), never by an adapter's default |
| `KillCompleteness` | Enum `Full` / `FirstKillerOnly` / `Sampled` / `BudgetLimited` | Types the PIT `fullMutationMatrix` and Stryker bail-on-first warnings; every value but `Full` caps `ZeroUniqueKills` at `Low`. `Sampled` and `BudgetLimited` are set only from a recipe sidecar (Story 2.2.8) or `--kill-completeness`, so a partial mutation run cannot look complete |
| `CoverageUnitKind` | Enum `Line` / `MutantSite` | PIT `coveringTests` is mutant-site coverage, not line coverage; every Q3 comparison states which kind each arm used |
| `TestLocator` | Resolves a `TestId` to `(file, line)` via `TestCodeProfile` + tree-sitter; returns `Located` / `Ambiguous` / `NotFound` | `NotFound` and `Ambiguous` become `ExclusionReason`s; required by kind tagging, `kibitzer:ignore` anchoring, and the brief |
| `Attribution` | Alias for `CoverageAttribution` in prose where only coverage is meant | Kept for continuity with ADRs |
| `Signal` | Enum of finding kinds: `NoAssertion`, `Tautology`, `MockOnly`, `ZeroUniqueCoverage`, `ZeroUniqueKills`, `PseudoTestedRegion` | Sum type, exhaustive match; each has a distinct rule id string, enumerated once in the "Rule-id registry" below the Glossary |
| `VacuityKind` | Sub-kind of static finding: `NoAssertion`, `Tautology`, `MockOnly` | Static-only subset of `Signal` |
| `UniqueCoverage` | Set of `CoverageUnit`s covered by test T and by no other test in the matrix | Meaningful only with its `Attribution` |
| `UniqueKills` | Set of `Mutant`s killed by T and no other test | Treated as binary (>= 1) unless subsuming-class data exists (pitfall P4) |
| `ReductionSet` | Greedy set-cover subset preserving covered units or killed mutants | Reported as "lost if all flagged are removed together", never as a minimal suite (pitfall P11) |
| `DominatorMutant` | Mutant not subsumed by another, relative to the supplied suite | Dynamic subsumption only; no equivalence claims |
| `TestKind` | Enum `Unit` / `Integration` / `Snapshot` / `Fuzz` / `Example` | Heuristic tag; integration/snapshot findings downgraded (pitfalls P8, P10) |
| `AssertionVocabulary` | Per-language set of callee names and macros counted as assertions, extended by config `options` | Lives in `src/test_code.rs`, one source of truth |
| `TestCodeProfile` | Per-language bundle: test-file globs, `test_functions(&CheckContext) -> Vec<TestFn { name, line, body, kind }>`, `assertion_calls(&TestFn) -> Vec<AssertionCall { callee, args }>`, `AssertionVocabulary`, exemption markers | The new seam (Tech Debt Disposition). Rust has no test-file notion (`#[cfg(test)]` is inline), so it is per-function only |
| `TestFinding` | Report-surface output record: `rule` (the rule id string), test id and located `file:line`, raw value, baseline, `Attribution`, `Provenance`, `ConfidenceTier`, per-row cost, overlapping tests and the subset of them that are themselves flagged | Own struct; `Finding` is not widened (ADR-002). JSON field is `rule` everywhere (plan and `design/ux.md`) |
| `ConfidenceTier` | Enum `High` / `Medium` / `Low`, derived from signal class and input quality until the experiment defines thresholds; not a probability | No numeric tier boundary ships before triage (UX research). The derivation rule is pre-registered at G0 (Story 0.4.1); the boundary values are set by Stories 3.3.1/3.3.3 and recorded at G2 (see "Pre-registered parameters" below) |
| `ExclusionReason` | Why a test was not assessed: `Flaky`, `Failing`, `Skipped`, `Generated`, `NoData`, `Stale`, `NoSource` | Printed in the report's "not assessed" line with the labels `stale`, `no-source`, `inputs disagree` (the `Flaky` variant keeps its name; the label states the observation, not a cause), `failing`, `skipped`, `generated`, `no-data`; every reason counts in the `assessed N of M tests in the input` breakdown, so the parts sum to K. A single input is NOT an exclusion: the footer prints "Stability not assessed: single run", every test stays assessable, and no tier is capped by it (consistency review B1); `Flaky` is the only stability exclusion and needs two inputs. `Stale`: the test's own file, or a production file its covered units live in, changed between the input commit and HEAD (Story 2.3.5). `NoSource`: the id resolves to no unique source file and line (`NotFound` or `Ambiguous`); such a test cannot carry `kibitzer:ignore` and is never emitted as a finding (decision in Task 2.3.2a) |
| `FlaggedSet` | Two named sets per signal per run: `IndividuallyFlagged` (per-test signal fires) and `JointSafe` (greedy, deterministic-order subset of `IndividuallyFlagged` whose joint loss of coverage units, and of kills where available, is empty) | Evaluated as a set, never per-test additively (pitfall P11). `JointSafe` is the primary unit of deletion; `IndividuallyFlagged` deleted wholesale is a pre-registered secondary that quantifies the joint-loss effect |
| `RandomBaseline` | Family of nulls, each size-matched: N1 uniform; N2 stratified by (package, `TestKind`); N3 runtime-matched plus "fastest-K" and "fewest-covered-units-K" heuristics; N4 coverage-preserving random deletion (random order, delete a test only if joint coverage loss stays empty) | ADR-003 requires beating N4 and N2, not just N1. N4 is the real competitor: it is what the cheap coverage-only heuristic achieves |
| `HeldOutFaultProxy` | Failure signal not used to select the flagged set. Tiers: (P1) real-history replay, (P2) mutant split by operator family and by module (select on family A, evaluate on family B), (P3) independent mutation tool where one exists | Labeled "real" or "same-family". P2 is a mandatory floor on every mutation-capable arm; P1 is reported only if the spike's power gate passes |
| `MutationBudget` | Per arm: sampled-mutant count M_s, module scope, wall-clock cap, random-baseline seed count K_mut; fixed at G0 from spike 0.3.4 | Arms whose projected cost exceeds the cap report coverage delta only |
| `DeletionDelta` | Change in coverage, mutation score, and proxy detection when a `FlaggedSet` is removed | Core experiment output |
| `Adjudication brief` | JSON shortlist with evidence for an LLM or human reviewer | Consumption contract only; kibitzer makes no LLM call |
| `HarnessInvariant` | Property that must hold by construction: on selection data the `JointSafe` set has exactly zero joint loss of coverage units (and of kills, on kill-matrix arms) | A sanity check on the harness, never evidence of validity; a nonzero value is a harness or joint-loss bug (pre-mortem #2). The report never prints it: a figure that is zero by construction reads as a green light, so the user-facing joint coverage check is computed over `IndividuallyFlagged` (Story 2.3.1) |
| `ReportMode` | Enum `Experimental` / `Descriptive` / `ReportOnly` / `Tiered`: the output mode, printed as the first-line banner (first two) and carried in JSON as `mode` and, per record, `signal_mode` (same four words; `experimental` means no G2 verdict is recorded for that signal) plus `section` (`main` or `experimental`, which alone separates main-list rows from `--experimental` rows). Top-level `mode` is the least-validated `signal_mode` among main-section rows, ordered `experimental` < `descriptive` < `report-only` < `tiered` | Derived from the G2 verdict per signal; `Experimental` until G2 exists. Which output elements each mode shows is the mode matrix in `design/ux.md` section 3.2a |
| `ReportEnvelope` | The part of every report that is not a finding: `format`, mode and banner, `advisory`, global `non_claims`, header (tool, version, commit, attribution per input, scope counts), `warnings[]`, `summary` (flagged by rule, joint check, suppressed counts, stability) | Specified in `design/ux.md` section 4 and frozen with `schema_version` 1 by Story 2.3.1 (Task 2.3.1e); text and JSON render the same data, so no consumer must read stderr to learn what lowered confidence |
| `NegativeResultReport` | The mandatory results section of `docs/test-effectiveness.md` recording which hypotheses were not supported, per arm, with numbers, and which claims are withdrawn | A terminal state of the project, not a failure; its content and what ships with it are fixed by the outcome table (Story 0.4.1) |

(33 terms, counted with grep over this table.)

### Rule-id registry (the one place rule ids are enumerated)

| `Signal` | Rule id string (`[rule]` in text, `rule` in JSON, key for `kibitzer:ignore` and `report_false_positive`) | Surface | Registered how |
|---|---|---|---|
| `NoAssertion` | `test-no-assertion` | hook and report | checker name via `inventory::submit!` |
| `Tautology` | `test-tautology` | hook and report | checker name via `inventory::submit!` |
| `MockOnly` | `test-mock-only` | hook (opt-in) and report | checker name via `inventory::submit!` |
| `ZeroUniqueCoverage` | `test-zero-unique-coverage` | report only | `RULES`, `OwnerMatch::Unowned` |
| `ZeroUniqueKills` | `test-zero-unique-kills` | report only | `RULES`, `OwnerMatch::Unowned` |
| `PseudoTestedRegion` | `test-pseudo-tested-region` | report only | `RULES`, `OwnerMatch::Unowned` |
| (conditional, Story 4.1.2) | `test-assertion-removed` | hook only | checker name via `inventory::submit!` |

The wireframe in `design/ux.md` prints these exact strings in its Rule column. The `RULES` drift-guard test (Story 2.3.2) iterates this table.

**Attribution source.** `CoverageAttribution` is never guessed silently. Order of resolution per adapter: (1) a value the input itself carries (Go `manifest.json` field `mode`: `isolated` or `in_order`; the recipe sidecar `<output>.run.json` of Story 2.2.8, which also carries kill-run completeness; any future producer metadata); (2) the `--attribution isolated|in-order|leave-one-out` flag; (3) the adapter's documented default, printed in the header as `attribution: in-order (assumed; pass --attribution to override)`. Defaults: coverage.py contexts = `in-order` (one pytest run with `dynamic_context = test_function` executes tests in one process in order); lcov with named `TN:` sections = `in-order` unless `--attribution isolated`; Go profiles have no default (the manifest is required, Story 2.2.2). An assumed attribution is a class W warning, so ADR-001's "attribution method must be supplied" is met by carry-or-flag-or-labelled-default, and the adapter rejects only missing tool and version.

**Confidence caps (single list; structural upper bounds, never raise a tier).** The tier boundaries from measured precision are set at G2; these caps hold regardless of them, are recorded at G0, and are the plan-level home of `design/ux.md` R8, R9, R14 and UX-E4/E7:

| Condition | Cap | Source |
|---|---|---|
| `TestKind` is `Integration` or `Snapshot` and the signal is coverage-derived | `Low` | pitfalls P8, P10; UX-E7 |
| `ZeroUniqueKills` with `KillCompleteness` other than `Full` (`FirstKillerOnly`, `Sampled`, `BudgetLimited`), or PIT without `fullMutationMatrix` | `Low` | UX-E4; R8, R9 |
| Input commit unknown | `Low` | Story 2.3.5; R11 |
| Coverage attribution is `InOrder` (including an assumed default) | `Medium` | R14: shared setup can overstate redundancy |
| Coverage-derived finding and no kill dimension supplied | `Medium` | Q3: coverage is a stand-in whose sufficiency is measured, not assumed |
| Attribution `Isolated` | no cap from attribution; label printed | ADR-003 item 4: isolated profiles are labeled and never alone justify a deletion claim |
| Unknown tool schema version (allow-list miss, Stories 2.2.1, 2.2.4, 2.2.5) | `Low` for that dimension | pre-mortem #5 |

---

## Pre-registered parameters (no value is invented in this plan)

Source: `design/ux.md` sections 2.3, 3.4, 11. Each row names the rule fixed at G0 and the experiment that supplies the value; downstream code reads the value from the named decision file, not from this plan.

| Parameter | Used by | Rule pre-registered at | Value set by | Recorded in | Until recorded |
|---|---|---|---|---|---|
| `ConfidenceTier` boundaries (`High`/`Medium`/`Low`) | Stories 2.3.1, 2.3.5, 4.2.1 | G0 (Story 0.4.1): tiers derive from measured triaged precision per signal class, lower Wilson bound; integration/snapshot capped at `Low` | Stories 3.3.1 (triage) and 3.3.3 (tier definition) | `G2-decision.md`, then constants in `src/test_effectiveness/tiers.rs` | Tier is assigned by signal class and input quality only, no numeric cut |
| Hook per-file cap on test-quality findings | Story 1.1.2 | G0 Part A (Story 0.4.1): cap is chosen from the per-file finding-count distribution of the Story 1.2.2 corpus run, using a percentile named in `G0-decision.md` | Story 1.2.2 (corpus run); informed by 1.2.1 transcript backtest | `G1-decision.md` | Cap logic present, value unset: no collapsing, all findings emitted |
| `--by-package` large-suite auto-switch threshold (R22) | Story 2.3.1 | G0 (Story 0.4.1): threshold is proposed from the Story 2.1.5 scale-run render measurements | Story 2.1.5, approved at G0 | `G0-decision.md` | No auto-switch; `--by-package` is opt-in only |
| Absolute hook latency budget (p95 ms per file, full hook path) | Stories 1.2.3, 4.1.1 | G1: the owner commits the number in the first commit of `G1-decision.md` ("budget commit"), before Story 1.2.3 measures; the relative bar (no worse than the slowest comparable checker) stays as a second condition | Owner at G1, informed by the Story 1.2.3 measurement design | `G1-decision.md` (budget commit precedes the measurement commit and the verdict commit) | No promotion: Story 4.1.1 stops without a recorded budget |
| Input size cap and parse-memory bound (`--max-input-bytes`, R26) | Stories 2.2.7, 2.1.5 | G0 Part B: the cap is the largest input the Story 2.1.5 parse-memory test parsed within the memory bound | Story 2.1.5 | `G0-decision.md` | Built-in ceiling: Task 2.1.5b's measured largest-passing input is committed as the code default before the hidden `Command::Tests` variant merges (Task 2.3.1a precondition), so adapters never read unbounded input and the cap tests run against it, never skipped; G0 may tighten it, never loosen it past the measurement |
| Non-specific dismissal-reason stoplist and minimum length (D-E6) | Story 2.3.2 | G0 Part A: derived from the reason distribution in Story 1.2.1 | Story 1.2.1 | `G1-decision.md` | Counter and hint are off |
| Progress-notice threshold for slow input reads (UX-E9) | Story 2.3.1 | G0 Part B: proposed from Story 2.1.5 parse timings | Story 2.1.5 | `G0-decision.md` | Notice printed on a TTY for every input file read |

---

## G0 decision template (ready to fill)

`decisions/G0-decision.md` is an owner decision and does **not** exist yet; this section is the template it is written from, so the owner edits proposed defaults instead of authoring numbers cold. Every default below is cited to the place in this plan that proposes it; the owner approves or edits each line and signs it (commit author or initials and date). A line left as `<owner fills>` means the plan proposes no number and the value comes from a spike. Nothing here is a recorded decision.

### Part A (static checks; needs no spike; committed before Stories 1.2.2 and 1.3.1)

| # | Decision | Proposed default | Cited from | Owner value |
|---|---|---|---|---|
| A1 | Primary persona and ranking (requirements "Users / Consumers") | 1 AI agent (hook), 2 repo owner (report), 3 developer/CI | `requirements.md` Users / Consumers | `<owner fills>` |
| A2 | H4 precision floor for a static rule to be promotable | 0.80 lower Wilson bound, n >= 50 triaged findings per rule per language | H4 in Phase 3 hypotheses | `<owner fills>` |
| A3 | Hook per-file cap: percentile of the per-file finding-count distribution | `<owner fills>` (no number proposed; value is derived at G1 from Story 1.2.2) | Pre-registered parameters row 2 | `<owner fills>` |
| A4 | Structural confidence caps | The "Confidence caps" table as written | Glossary, "Confidence caps" | accept / edit |
| A5 | Triage volume the owner commits to review, including the blind 10% (minimum 30 per rule per language) | 900 verdicts (the Story 1.2.2 estimate, likely 2-3x under). An unmade decision falls back to this figure; Story 1.2.2 stops at the committed volume | Effort estimate, Re-baseline note; Story 1.2.2 | `<owner fills>` |
| A6 | Non-specific dismissal-reason stoplist and minimum length (D-E6) | `<owner fills>` from Story 1.2.1 ignore-reason data | Pre-registered parameters | `<owner fills>` |
| A7 | SM4 interpretation: shipped validated check (product success) vs shipped descriptive report on a null result (research-track success; informative negative) | Accept the distinction as written in requirements | `requirements.md` "How SM3 and SM4 are read" | signed line |
| A8 | SM5 target and decision rule: which flagged-then-revised value, and which ignore-directive rate with non-specific reasons, holds a rule opt-in or demotes a promoted one, and over how many sessions. Note: the pre-ship baseline (Task 1.2.1c) is the natural strengthening rate of agents that never saw the finding, a floor for the live figure and not a treatment effect | `<owner fills>` (no rate proposed; none is sourced). **If unset**: promoted rules keep their G1 verdict and SM5 is reported descriptively only; no demotion fires. **Minimum live sample before a demotion rule may fire**: 30 agent-added tests with a finding, per rule (the per-rule minimum A5 uses for triage; owner may raise it). **If the pre-ship baseline is "not estimable"** (Task 1.2.1c): the live rate is reported raw with no comparison, demotion uses only the ignore-directive rate and reason diversity, and the adoption assumption stays UNVERIFIED | `requirements.md` SM5; Task 1.2.1c | `<owner fills>` |
| A9 | Rule if Story 1.2.1 finds ~zero confirmed transcript occurrences of assertion-free or tautological agent-written tests ("~zero" is the owner's threshold) | `proceed-corpus-only`: Phase 1 continues on the corpus-only justification `docs/check-ideas.md` allows (Stories 1.2.2, 1.3.1 still run); the alternative `registry-only` holds Story 4.1.1 and the TS/JS, Java/Kotlin and Rust rule tables (Tasks 1.1.1b, 1.1.2b, 1.1.3b, 1.1.4a) until a later occurrence is found | Story 1.2.1; `requirements.md` "Why now" | `<owner fills>` |
| A10 | Whose adoption is the goal: owner dogfooding, or external adoption (requirements "Users / Consumers") | `owner-dogfooding`: success is judged on the owner's repos and sessions; no gate depends on external users and no external-generalisation claim is made. The alternative `external-adoption` adds a requirement this plan does not meet (evidence from another user or agent population before G1 promotion) | `requirements.md` Users / Consumers; Product lens round 3 | `<owner fills>` |

### Part B (experiment; after spikes 0.3.1, 0.3.2, 0.3.4)

| # | Decision | Proposed default | Cited from | Owner value |
|---|---|---|---|---|
| B1 | Headline held-out proxy per arm and fallback | P1 only where the power gate passes (about 96 events for +/-0.10 at the worst-case proportion), else P2 same-family, labeled | Story 0.3.1 | `<owner fills>` per arm |
| B2 | Exemplars and named modules | Selection criteria of Story 0.2.2; candidates are INFERRED until smoke-tested | Story 0.2.2 | `<owner fills>` |
| B3 | `MutationBudget` per arm, feasibility verdict per language | `<owner fills>` from spike 0.3.4; K_mut >= 19, M_s reduced before K_mut | Story 0.3.4 | `<owner fills>` |
| B4 | K per null family | K >= 999 where nulls come from a matrix; K >= K_mut (>= 19) where each null needs a rerun | H2 | `<owner fills>` |
| B5 | R_max (leave-one-out bound) | min(200, 2% of suite) shortlisted tests, stratified by package | Story 0.3.2 | `<owner fills>` |
| B6 | H1 equivalence margin (SM3 delta) | 90% CI of the held-out mutation-score delta within +/-2.0 percentage points; line-coverage margin +/-1.0 for the I0-checked secondary figure | H1 | `<owner fills>` |
| B7 | H2 (SM3 contrast against nulls) | rank p-value <= 0.05 against N2 and N4 separately; N1 context only | H2 | `<owner fills>` |
| B8 | H3 (Q3 stand-in) | Wilson 95% lower bound on precision of Z_cov w.r.t. Z_kill >= 0.90; recall reported | H3 | `<owner fills>` |
| B9 | SM4 numeric reading for a dynamic signal | A signal is "validated" only if H2 is supported on the held-out proxy; tier boundaries come from triaged precision (lower Wilson bound), no number fixed here | Pre-registered parameters row 1; Story 3.3.3 | rule accepted / edited |
| B10 | Wall-clock budget, arm order, stop-loss | Matrix arms first (free nulls); stop-loss 1.5x the 0.3.4 projection | Story 0.4.1 | `<owner fills>` |
| B11 | Matrix scale bound | <= 1.5 GB resident, <= 120 s at 100k tests, 1M units, 5 x 10^7 entries | Story 2.1.5 | accept / edit |
| B12 | Input size cap (`--max-input-bytes` default) and parse-memory bound | Largest input parsed within the Story 2.1.5 parse-memory bound | Story 2.2.7 | `<owner fills>` |
| B13 | Experiment execution isolation mode per arm (`container`, `vm`, `rootless-sandbox`) and env allow-list | Policy of Story 0.3.5, defaults as written there | Story 0.3.5 | `<owner fills>` per arm |
| B14 | Linkage go/no-go threshold (resolved rate) | `<owner fills>` from spike 0.3.3 | Story 2.3.3 | `<owner fills>` |
| B15 | `--by-package` threshold and slow-read notice threshold | `<owner fills>` from Story 2.1.5 | Pre-registered parameters | `<owner fills>` |
| B16 | Outcome table (5 rows) and owner acceptances (a) SM3 met in part, (b) SM4 met by descriptive-only on a null result, (c) triage-volume commitment | As written in Story 0.4.1 | Story 0.4.1 | signed lines |

### Part C (recorded at G1, not G0)

The absolute hook latency budget (p95 ms per file on the full hook path) is committed in the first commit of `G1-decision.md`, before Story 1.2.3 measures (Pre-registered parameters row 4). The template line is `C1: budget = <owner fills> ms; basis: <owner fills>`; the plan proposes no number because none is sourced in the repo.


---

## Pattern Decisions

| Component | Pattern Chosen | Source | Alternative Rejected | Reason |
|---|---|---|---|---|
| Test-file/test-function detection | Seam: `TestCodeProfile` per `Language` | Isolate-via-seam (planning-prompt 3.5) | Each checker hand-rolls globs | Duplication already exists (`research/architecture.md` section 2); one vocabulary prevents divergent assertion lists |
| Static checkers | Strategy: `Checker` trait impls + per-language rule table | GoF Strategy; mirrors `syntax-rules-<lang>` / `comment-quality-<lang>` | One giant checker | Matches the `inventory::submit!` registration used by every checker module (`src/checkers/java_swallowed_interrupt.rs:126`, `src/checker.rs:248`) and the chained-list convention in `default_checks()` (`src/config.rs:759-769`) |
| Matrix storage | Inverted index: per unit, a sorted `u32` posting list of test ordinals; per-unit hit count | Scalability NFR (10^5 tests) | Dense `Vec<u64>` test-by-unit bitsets | Dense is tens of GB at 10^5 x 10^6; unique coverage needs only units with hit count 1. `roaring` is admitted only if Story 2.1.5's scale test fails |
| Batch report module | Transaction Script: pure `analyze(&TestMatrix) -> Vec<TestFinding>` plus thin `run_*` | PoEAA Transaction Script; mirrors `hotspots.rs` | Domain Model / Service Layer | Pure functions over a matrix; no persistent aggregate |
| Input adapters | Adapter (one per format) into `TestMatrix` | GoF Adapter | Per-signal readers | Isolates third-party formats; adding a producer touches one file |
| `Signal`, `Attribution`, `ConfidenceTier`, `TestKind`, `ExclusionReason` | Sum types, exhaustive matches | type-driven-design | String tags | Compiler-enforced handling; no stringly-typed rule ids outside the output boundary |
| `TestId`, `MutantId`, `CoverageUnit` | Newtypes | type-driven-design | Raw `String`/`usize` | Prevent cross-namespace confusion (a mutant id used as a test id) |
| Provenance | Parse-don't-validate constructor: `TestMatrix::new` fails without tool + version | type-driven-design | Optional fields | Observability requirement becomes unrepresentable-to-violate |
| Ranking and overlap naming | Pure functions, deterministic tie-break | ADR-005 | Randomised greedy | Reproducible findings for triage |
| Experiment harness | Script pipeline with JSONL records | existing `scripts/backtest-triage.py` convention | In-binary experiment command | Experiment code is throwaway research tooling, not product surface |
| Static enrichment from `ArchModel` | Read-only view, `is_test` derived from `file` | "One model, many views" (`research/architecture.md` section 5) | New stored field on `PackageNode` | Avoids `ModelCache` schema churn |

---

## Tech Debt Disposition

| Area | Existing Issue | Disposition | Justification |
|---|---|---|---|
| Test-file / test-function identification | No shared helper; each checker hand-rolls globs and `Test` prefix logic (`src/checkers/go_table_driven_test.rs:71,142`; the `**/*_test.go` glob on the `go-table-driven-test-candidate` entry at `src/config.rs:860-862`) | Isolate via seam | `src/test_code/mod.rs` (plus per-language files) used by all new code; migrating `go-table-driven-test-candidate` is deferred (non-goal) because refactoring existing checkers first buys nothing for this work |
| `Finding {line, message}` | Cannot carry signal/value/baseline/tool | Isolate via seam | Batch report gets `TestFinding` (ADR-002); `Finding` untouched |
| `ArchitectureAction` / `main.rs` (1182 lines; 49 commits in six months per architecture review) | Large dispatcher, high churn | Isolate via seam | Add one `Command::Tests` variant (enum `Command`, `src/main.rs:71`) and a one-line dispatch; handler lives in `src/test_effectiveness/cli.rs` so `main.rs` does not take new logic. Module declarations land in Wave 1 (Story 2.1.1) to avoid merge conflicts |
| `default_checks()` / `Checker` trait (`src/config.rs` 36 commits, `src/check.rs` 30 in six months) | Stable registry pattern | Extend as-is | New `test_quality_checks()` returns `Check` entries that nothing chains (opt-in precedent: `prose_checks()`, `src/config.rs:784`); checkers are reachable by name through `inventory::submit!`; chained into `default_checks()` only after gate G1 |
| `src/inline_ignores/rules.rs` `RULES` | `known_rule()` knows only `RULES` plus registered checker names, so a report rule id yields `[unused-ignore]` | Extend as-is | Register report `Signal` rule ids as `OwnerMatch::Unowned` (as `god-class`, `src/inline_ignores/rules.rs:97`); Story 2.3.2 |
| `is_generated` lives in `checkers::file_size` | `src/test_code.rs` (lower level) would import from `checkers` | Extend as-is | Accept the dependency for now; re-export from `test_code` so callers do not reach into `checkers` (Task 0.2.1a) |
| `ArchModel` test-to-symbol linkage | Test files not segregated; no consumer builds test-to-symbol map | Extend as-is (conditional) | Only if spike 0.3.3 shows usable `resolved` rate; add consumer, not stored field |
| Plugin protocol | Per-file SARIF, 30 s timeout | Leave alone | Not on the critical path (ADR-002) |
| Inline-ignore post-pass at `run_checker_against_source` (`src/check.rs:477`) | Batch reports bypass it | Isolate via seam | Report anchors findings on the test function's first line and applies the same `inline_ignores` scan to that source file (Story 2.3.2) |

---

## Migration Plan

Omitted: no schema or data migration. New cache/state is none; report inputs are user-supplied files.

---

## Observability Plan

- **Logs**: `kibitzer tests report` writes structured diagnostics to stderr at `--verbose`: adapter chosen, input path, record counts (tests, units, mutants), rejected records with reason, exclusions by `ExclusionReason`. Static checkers use the existing hook log (`src/hook_log.rs`).
- **Per-finding evidence** (requirements, Observability): every `TestFinding` carries `rule`, raw value, baseline description, `Attribution`, tool@version, input commit, `ConfidenceTier`, and overlapping test ids. Static findings render the measured value into `message` because `Finding` has no structured fields (`src/checker.rs:54-57`); the reason is stated in the message.
- **Metrics**: report summary (suite size, flagged per signal, excluded per reason, suppressed counts, joint coverage check; no runtime total, per-row cost only, per `design/ux.md` Principle 7); experiment records (flagged count, triaged precision, coverage / mutation / proxy delta, random-baseline delta distribution) as JSONL under `project_plans/test-effectiveness-critique/experiment/results/`. Static-checker hook latency measured by Story 1.2.3 (p95 per file vs. the comparable `go-table-driven-test-candidate` and `comment-quality-*` checkers).
- **Alerts**: none; CLI tool.
- **False-positive loop**: findings use `[rule-id]` prefixes so `report_false_positive` and `kibitzer:ignore` apply (`docs/reporting-false-positives.md`, `docs/suppressing-checks.md`).

## Risk Control

- **Feature flag**: static checkers registered in `test_quality_checks()` but not chained into `default_checks()` until a recorded G1 decision (default: off). `kibitzer tests report` merges with `#[command(hide = true)]` and an "experimental" banner on every output until a recorded G2 decision (Story 4.2.1c unhides it). Batch report is an explicit subcommand, never run implicitly. Both disableable by rule name per `docs/suppressing-checks.md`.
- **Rollback procedure**: remove the `test_quality_checks()` chain from `default_checks()` (static); stop invoking the subcommand (report), or delete the hidden variant only if the owner writes `remove` in `G2-decision.md` (the default for a null result is `descriptive-only`, which keeps the variant, Story 3.4.1). No data migration. Standard revert via PR close + revert commit.
- **Staged rollout**: G1 promotes static rules one at a time; G2 decides per dynamic signal whether it ships as report with tiers, report-only, descriptive-only (unvalidated, no ordering claims), or is removed.
- **Null or negative experiment result**: a defined terminal state, not a failure. The outcome table in Story 0.4.1 names what ships for every combination of results; the minimum guaranteed deliverable is `docs/test-effectiveness.md` with a `NegativeResultReport` plus the descriptive report (Stories 3.3.3, 4.2.1).
- **Behavioural guardrails** (pitfalls P1, P2): advisory only; no pruning nudge on pre-existing tests in hook mode; no pass/fail gate and no headline "N% redundant" score; deletion wording never imperative; `kibitzer:ignore` is the quiet path.

## Quality gates (every story's definition of done)

- CI steps as in `.github/workflows/ci.yml`: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test`.
- Diff-coverage gate: `covgate.toml` requires 80% of changed lines and 75% of changed regions covered (CI step `covgate check coverage.json`). `#[ignore]` scale tests (Story 2.1.5) and the hidden-command glue (Story 2.3.1) count against changed lines, so each Phase 2 story ships a non-ignored test that reaches its new lines; the scale tests also have a small-N variant that runs by default.
- Test tooling is std-only and adds no dev-dependencies (ADR-005, iteration 2 amendment): binary spawned via `env!("CARGO_BIN_EXE_kibitzer")`, golden files under `testdata/test-effectiveness/golden/`, a seeded in-tree PRNG for property tests and for the scale-fixture generator.
- A new native checker meets the repo's "Writing a new check" bar before it counts as done (Epic 1.2): transcript backtest and corpus run.

---

## Unresolved Questions

- [ ] **Q3**: Is the cheaper signal (zero unique coverage) a sufficient stand-in for zero unique kills? **Scope of the answer**: per-test kill matrices exist only for JVM (PIT) and JS/TS (Stryker), so the per-test comparison is answered only there (on PIT, `coveringTests` is a superset of the killers by construction and both come from one file, so the Z_cov-versus-Z_kill comparison is biased toward containment and weakly informative on that unit kind; the result states this, Story 3.3.2) (arms 3.2.4, 3.2.5, with exemplar count stated, n of 2 suites at minimum). For Go, Python, and Rust, Q3 is answered only at suite-level mutation-score delta after deleting a flagged set (H1), not per test; their default ordering ships labeled "unvalidated default" and extrapolation from Java/JS is forbidden in Stories 3.3.2 and 4.2.1. The conclusion is a hypothesis with a decision rule (Story 3.3.2), not a pre-written result — blocks the report's default signal ordering in Story 4.2.1 — owner: tstapler at G2.
- [ ] **Q4**: Which languages are first-class? Default hypothesis from research: Go and Python (locally runnable per-test coverage), Java/JS via PIT/Stryker adapters, Rust coverage-only — blocks Story 4.2.1 defaults and docs claims — resolved by the spike 0.3.4 feasibility matrix and Epic 3.2 per-arm results — owner: tstapler at G2.
- [ ] **Q5**: Is static assertion-quality precision sufficient for `default_checks()`? — blocks Story 4.1.1 — resolved by Stories 1.2.1-1.2.2 and re-measured in 3.2.6 against the H4 threshold pre-registered at G0 — owner: tstapler at G1.
- [ ] Held-out fault proxy: tiering P1/P2/P3 is fixed by this plan (see Glossary `HeldOutFaultProxy`); which tier is reported as the headline per arm is decided at G0 from spike 0.3.1's power gate — blocks Story 3.1.3 — owner: tstapler at G0.
- [ ] Pre-registered numbers (hypothesis thresholds, equivalence margins, H4 precision floor, K seeds, mutant sample size) — **required at G0**, before any experiment run; the agent proposes defaults in Story 3.1.1, the owner approves or edits them — blocks 3.1.2 onward. Tier boundaries for `ConfidenceTier` are derived afterward from measured precision (blocks 4.2.1).
- [ ] Exemplars for the dynamic arms (Python, Java module, JS/TS project) and which kubernetes subpackages and ripgrep crates are mutated — blocks 3.2.x — resolved by Story 0.2.2 against the selection criteria there — owner: tstapler confirms at G0.
- [ ] Owner acceptance at G0 that Success Metric 3 is met in part (static-only runs for cassandra, servo, deno, vscode) and that Success Metric 4's "shipped check or report" includes the descriptive report on a null result — blocks 3.3.3 and 4.2.1 — owner: tstapler at G0 (Story 0.4.1).
- [x] Anchor of dynamic findings for `kibitzer:ignore` (test function first line, per Story 2.3.2) versus `.kibitzer/accepted/` entries — DECIDED in Task 2.3.2a: anchored findings use inline `kibitzer:ignore` only; unanchored tests are `NoSource` exclusions, not findings, and get no dismissal path. Task 2.3.2a still verifies the `src/inline_ignores/` and `src/accepted_findings.rs` behaviour in code before implementation.
- [ ] Pre-registered parameters (tier boundaries, hook per-file cap, `--by-package` threshold, absolute hook latency budget, input size cap, non-specific-reason stoplist): see "Pre-registered parameters"; values blocked on the experiments named there.
- [ ] Primary persona ranking (agent, repo owner, developer/CI): recommended default is agent first (requirements "Users / Consumers"); decided at G0 Part A (template line A1) — blocks nothing in Phase 0 but fixes which surface's metric (SM5) is the headline — owner: tstapler at G0.
- [ ] Absolute hook latency budget — blocks Story 4.1.1 — owner: tstapler at G1 (budget commit precedes measurement).
- [ ] Whether `call_edges` from test files have a usable `resolved` rate — blocks Story 2.3.3 only — resolved by spike 0.3.3.
- [ ] Whether c8/nyc/llvm-cov ever write non-empty lcov `TN:` (`research/stack.md` section 8) — blocks Story 2.2.3 scope claims — resolved inside 2.2.3 by running producers.
- [ ] Stryker `killedBy` completeness depends on runner and bail setting — blocks Story 2.2.4 accuracy claims — resolved inside 2.2.4 by fixture from a real run.
- [ ] Kotlin mutation tooling status (archived `pitest-kotlin`, `research/stack.md` section 2) — blocks any Kotlin dynamic claim — owner: tstapler; default is "static checks only for Kotlin".

Sources the plan does NOT build on (UNVERIFIED in research): the unsourced "delete ~25% with Opus" anecdote, Papadakis ISSTA 2016 subsumption figure, Schuler & Zeller details beyond abstract, snapshot-testing literature (Fujita 2023), Google change-detector post body, Sonar S2699, Develocity PTS, Kochhar 2015, assertion-padding as an observed agent behavior. These are listed in `docs/test-effectiveness.md` as unverified and do not justify any design choice.

---

## Dependency Visualization

```
Phase 0 (parallel wave 1)
  0.1.1 literature doc + tool inventory (0.1.1d) ─────────────────────┐
  0.2.1 test_code seam ──┬──────────────► Phase 1 static (1.1.x) ─► 1.2.1 ─► 1.2.x ─► 1.3.1 [G1 record]
  0.2.2 corpus additions ─┤                          ▲   ▲                 │ (G1-decision.md)
  0.3.1 proxy spike ──────┤                          │   │                 ▼
  0.3.2 cost spike ───────┼─► 0.4.1 [G0 record] ─────┘   │           4.1.1 promote (per rule)
  0.3.3 linkage spike ────┤   Part A (static: H4 floor,  │                 ▲ needs G1-decision.md
  0.3.4 mutation-cost ────┤   hook-cap percentile) ──────┘ edges to 1.2.2 and 1.3.1
  0.3.5 execution safety ─┘   (policy and preflight precede any 0.3.2/0.3.4/3.x run of third-party code)
                              1.1.x ─► 1.2.1 (A3/A6 inputs) ─► G0 Part A ─► 1.2.2 ─► 1.3.1
                              Part B (experiment) ──┐   (Part A is recorded before the spikes finish;
                                                    ▼    Phase 1 evidence is not blocked behind 0.3.x)
Phase 2 EXPERIMENT-CRITICAL (parallel with Phase 1; no G0 dependency)
  2.1.1 TestMatrix ─► 2.1.2 algos ─► 2.1.3a/b/c exclusions (incl. Stale/NoSource variants)
  0.2.1 test_code seam ─► 2.1.4 locator (minimal; containment, 2.2.7c)
  2.2.9 run-sidecar schema + `--kill-completeness` flag (frozen first; adapters test against hand-written
        sidecar fixtures, so no wrapper script is needed) ─► 2.2.1 coveragepy, 2.2.2 gocover, 2.2.4 stryker,
        2.2.5 pit adapters (after 2.1.1) + harness input path
  2.2.7a size/depth guard (guard.rs only; each adapter story calls it), 2.2.7b PIT entity rules, 2.2.7d malformed
        corpus (tests over those four adapters; 2.2.7b-d follow the adapter they touch)
  2.1.5 scale tests: matrix, parse memory for the FOUR experiment-critical formats coveragepy, gocover, stryker,
        pit (records the built-in ceiling; lcov's scale test lives in ship-gated 2.2.3), locator (after 2.1.2 and the adapters)
  2.2.9c partition check: the experiment-critical set builds and its tests run with every ship-gated module removed
  2.3.1a command wiring + TestFinding + `findings[]` JSON + minimal class E path carrying R26 (id and exit 2),
        2.3.1e envelope (after 2.1.5b: hidden variant needs the built-in ceiling) ─► 3.1.2 harness input and 3.2.x arms (flagged sets via report JSON)
  1.1.2/1.1.3 checkers (registry-only) ─► 3.2.1/3.2.6 static flagged sets via `kibitzer check native`
        (the harness's own static source; it does not wait for the report's static rows, 2.3.1d)
Phase 2 SHIP-GATED (needs G2-decision.md; built only after G2, full scope retained)
  2.2.3 lcov (incl. its own parse-scale test), 2.2.6 junit (each with its own guard call and malformed
  fixtures), 2.2.8 recipe wrapper scripts for non-Go producers (they only write the 2.2.9 sidecar), 2.3.1 gated ACs (below), --by-package + R22 message and auto-switch, staleness 2.3.5
  (incl. R10/R23-R25 messages and 2.3.5c path containment), 2.3.3 enrich(cond), 2.3.4 brief
Phase 2 REMAINDER (parallel with Phase 1, not G2-gated; needed by the hidden report, not by the experiment)
  2.3.1b text renderer, 2.3.1c class E messages + exit contract, 2.3.1d static rows + verbose + ranking,
  2.3.1f class W/I messages (R7-R9, R11-R17, R21), 2.3.2 ignore + RULES
Phase 3 experiment (every 3.x story needs G0-decision.md Part B)
  3.1.1 protocol + pre-reg ─► 3.1.2 harness+nulls+invariant ─► 3.1.3 proxy impl ─► 3.1.4 leave-one-out
  3.2.1..3.2.6 arms (need experiment-critical adapters, 3.1.x) ─► 3.3.x ─► 3.4.1 [G2 record]
                                                                  (G2-decision.md)
Phase 4                                                               │
  4.1.2 (cond. on 1.2.1 evidence); 4.2.1 tiers/defaults/unhide/descriptive and 4.2.2 docs need G2-decision.md
```

Execution safety: Story 0.3.5's policy and preflight are preconditions for every story that runs third-party suites or mutation tools (0.3.2, 0.3.4, 3.1.2, 3.1.4, 3.2.x); the harness enforces it by refusing to start an arm without a recorded isolation mode.

Parallelism: Phase 1 and the experiment-critical part of Phase 2 are independent after 0.2.1 and 2.1.1 respectively; experiment arms 3.2.1-3.2.6 fan out after G0 Part B. Gates are stories (0.4.1, 1.3.1, 3.4.1) whose acceptance is an owner-authored decision file in `project_plans/test-effectiveness-critique/decisions/`; downstream stories list that file as a precondition and do not start without it. Story 0.4.1 is recorded in two parts in the same file: Part A (static-check numbers, no spike dependency) precedes Stories 1.2.2 and 1.3.1; Part B (experiment numbers) follows spikes 0.3.1-0.3.4.

**Phase 2 split (pre-mortem #8; iteration 2 D1-D3).** The experiment needs only the experiment-critical set, which now explicitly includes the report pieces the harness consumes (2.1.3c, 2.3.1a, 2.3.1e, 2.1.4) and uses the Phase 1 checkers as its own static source, so no experiment story waits on a REMAINDER or ship-gated task. Story 2.3.1 is partitioned (its core ACs close in the REMAINDER wave; its gated ACs close with their owning stories, listed in the story), and Story 2.2.7's staleness and lcov/junit parts live in the stories that create those files. The ship-gated set is the product surface whose shape depends on what the experiment finds (a null result ships a descriptive report, Story 4.2.1), so it lists `G2-decision.md` as a precondition. Nothing is cut: the owner's full-scope stance is kept, only sequenced.

---

# Phase 0: Foundations, literature, spikes

### Epic 0.1: Literature deliverable and signal catalog
**Goal**: Success Metric 1 and 2 as a cited docs artifact, honest about validation status.

#### Story 0.1.1: Write `docs/test-effectiveness.md`
**As a** repo owner, **I want** the cited survey and candidate-signal catalog in the repo, **so that** every shipped signal traces to a source and a stated validation status.
**Acceptance Criteria**:
- Each candidate signal lists input required, cost, false-positive risk, advisory-diff fit, validation status (empirically validated / plausible / unvalidated) and at least one primary citation.
  - *Given* signal `ZeroUniqueKills` in the catalog, *When* a reader opens its row, *Then* it shows input "kill matrix", validation "lossy: failed-build detection loss up to 52.2% on Java projects with mutation-/coverage-based reduction (Shi et al., ISSTA 2018; scope stated in the row)", and a link to the primary PDF.
- Sources marked UNVERIFIED in research are listed in an "Unverified / not relied on" section and appear in no rationale.
  - *Given* the Opus-deletion anecdote, *When* the doc is searched for it, *Then* it appears only under "Unverified / not relied on" with the text "no public write-up located".
- A tool inventory table (requirements In Scope item 2) states, for each existing coverage, mutation, and test-smell tool, what it emits and whether kibitzer can ingest it mechanically.
  - *Given* the finished file, *When* the "Tool inventory" table is read, *Then* it has one row per tool named in `research/stack.md` sections 1-5 (coverage.py, `go test -coverprofile`, lcov producers c8/nyc/llvm-cov/gcov, JaCoCo, gremlins, go-mutesting, mutmut, cosmic-ray, cargo-mutants, PIT, Stryker, JUnit XML, and the test-smell detectors of section 3) with non-empty cells for: language, emitted artifact and format, per-test granularity (yes/no/needs-config), machine-ingestable by kibitzer (yes: adapter story id; no: reason), and source section.
  - *Given* the row for lcov producers, *When* read, *Then* its per-test-granularity cell states "unverified until Story 2.2.3 runs the producers" rather than a claim.
- All links resolve under the existing `markdown-link-integrity` check.
  - *Given* the finished file, *When* `kibitzer check native markdown-link-integrity docs/test-effectiveness.md` runs, *Then* it reports no findings.
**Files**: `docs/test-effectiveness.md`

##### Task 0.1.1a: Signal catalog tables (S)
- Transcribe the catalog from `research/literature-effectiveness-mutation.md` section 5 and `research/literature-redundancy-smells-llm.md` section 7, reconciling the two S-numbering schemes into one table keyed by `Signal`.
- Files: `docs/test-effectiveness.md`

##### Task 0.1.1b: Bibliography with verification labels (S)
- Copy both bibliographies, dedupe, keep VERIFIED/SECONDARY/UNVERIFIED labels; add "how to read the evidence" and the corrected Kochhar/Gopinath attribution.
- Files: `docs/test-effectiveness.md`

##### Task 0.1.1d: Tool inventory table (S)
- Build the "Tool inventory" table from `research/stack.md` sections 1-5 and 8; mark every claim not re-verified in this repo as `INFERRED`; link each row to the adapter story that ingests it or state why none does.
- Files: `docs/test-effectiveness.md`

##### Task 0.1.1c: Self-review and link check (XS; runs last, after 0.1.1d)
- Re-read adversarially: every numeric claim has a cited row; run the link-integrity checker; add a one-line pointer from `README.md` docs list if one exists.
- Files: `docs/test-effectiveness.md`, `README.md`

### Epic 0.2: Seams and corpus readiness
**Goal**: Shared test-code seam; corpus able to exercise Python and a mutation-feasible JS/TS and Java project.

#### Story 0.2.1: `TestCodeProfile` seam in `src/test_code/`
**As a** checker author, **I want** one place that knows what a test file, test function, and assertion are per language, **so that** new checkers do not hand-roll globs.
**Note on paths**: the seam is a directory module (`src/test_code/mod.rs` plus one file per language) so that no single file carries six languages and the four tasks below do not serialise on one file; every other mention of `src/test_code.rs` in this plan and in `validation.md` means `src/test_code/mod.rs` (the module path `test_code` is unchanged).
**Acceptance Criteria**:
- Profiles exist for Go, Python, TypeScript/Tsx/JavaScript, Java, Kotlin, Rust.
  - *Given* `src/pkg/foo_test.go`, *When* `TestCodeProfile::for_language(Go).is_test_file(path)` is called, *Then* it returns true, and false for `src/pkg/foo.go`.
- Generated files are excluded via `file_size::is_generated` (re-exported from `test_code`).
  - *Given* a `foo_test.go` whose first line is `// Code generated by mockgen. DO NOT EDIT.`, *When* `is_test_file_source(path, source)` is called, *Then* it returns false.
- The shared function-level API exists, so the three checkers and the report do not re-walk trees or re-implement assertion extraction.
  - *Given* Go source with `func TestA(t *testing.T){ assert.Equal(t, 1, f()) }`, *When* `test_functions(ctx)` then `assertion_calls(&fn)` run, *Then* one `TestFn { name: "TestA", line, .. }` and one `AssertionCall { callee: "assert.Equal", args: 3 }` are returned.
  - *Given* Rust source with an inline `#[cfg(test)] mod tests { #[test] fn t() { assert!(x) } }` in `lib.rs`, *When* `test_functions(ctx)` runs, *Then* `t` is returned although `is_test_file(lib.rs)` is false.
- One parse per file: `test_functions` takes the tree already parsed by the dispatcher (`CheckContext { source, tree }`, `src/checker.rs:164-167`, parsed once through `GrammarCache`) and never parses on its own; the report's locator and static rows share one parse per source file (iteration 2, engineering NFR guard, so hook latency is not first measured after the build).
  - *Given* a file processed by the three test-quality checkers in one run, *When* a parse counter in the test build is read, *Then* it is 1 for that file; *Given* the report locating and checking 1,000 files, *Then* the counter equals 1,000.
- The vocabulary is extensible from one shared options key, resolved once.
  - *Given* options `{"assertion_helpers": ["requireOK"]}` under any test-quality check name in `.kibitzer/inspect.json`, *When* `AssertionVocabulary::from_repo_config(root)` is called by either a checker or the report, *Then* `contains("requireOK")` is true for both; without the option, false.
**Files**: `src/test_code/mod.rs`, `src/test_code/go.rs`, `src/test_code/python.rs`, `src/test_code/ts.rs`, `src/test_code/java.rs` (Java and Kotlin), `src/test_code/rust.rs`, `src/main.rs` (mod line), `src/checkers/mod.rs` (`pub mod` only; registration is `inventory::submit!`)

##### Task 0.2.1a: Seam core and shared option plumbing (S)
- `mod.rs`: `TestCodeProfile` trait/struct, `TestFn`, `AssertionCall`, `AssertionVocabulary` with `assertion_helpers` resolved once from `.kibitzer/inspect.json` check options (any of the three test-quality check names, merged) rather than per-check `configure(&options)` (`src/checker.rs:197-208`), the `is_generated` re-export, the parse-reuse guard test. Lands first; the three language tasks depend on it and then run in parallel.
- Files: `src/test_code/mod.rs`

##### Task 0.2.1b: Go and Python profiles with tests, including the Go snapshot markers (S)
- Go snapshot markers `cupaloy`, `goldie` live here (single owner of `go.rs`; Task 0.2.1d no longer touches it). Test-function predicates (`Test*` with `*testing.T`; `test_*` and `unittest.TestCase` methods), assertion vocabularies (`t.Error*`, `t.Fatal*`, testify `assert.*`/`require.*`, `cmp.Diff` plus `t.Errorf`; `assert`, `self.assert*`, `pytest.raises`), `test_functions` and `assertion_calls` over tree-sitter.
- Files: `src/test_code/go.rs`, `src/test_code/python.rs`

##### Task 0.2.1c: TS/JS and Java/Kotlin profiles with tests (M)
- `expect(...)` chains, `assert*`, `assertThat`, `verify`, snapshot markers (`toMatchSnapshot`), exemption marker set; `test_functions` and `assertion_calls`.
- Five grammars (TypeScript, TSX, JavaScript, Java, Kotlin), each with `test_functions`, `assertion_calls`, exemptions and tests, hence M.
- Files: `src/test_code/ts.rs`, `src/test_code/java.rs`

##### Task 0.2.1d: Rust profile (S)
- `#[test]`/`assert!`/`#[should_panic]`; Rust is per-function detection only (`#[cfg(test)]` is inline), no `is_test_file`.
- Files: `src/test_code/rust.rs`

#### Story 0.2.2: Corpus additions (Python, JS/TS, small Java)
**As a** experimenter, **I want** Python, small JS/TS and small Java exemplars in `docs/backtest-repos.md`, **so that** per-test coverage and ground-truth kill ids can be exercised (there is no Python exemplar today, `docs/backtest-repos.md`).
**Acceptance Criteria**:
- Each added repo runs its suite locally and yields per-test data with the intended tool.
  - *Given* the chosen Python exemplar cloned shallow, *When* its suite runs with `coverage.py dynamic_context = test_function`, *Then* `coverage json --show-contexts` lists at least 50 distinct test contexts.
- Docs and clone script list the repos with the "why chosen" convention.
  - *Given* `scripts/clone-backtest-repos.sh`, *When* run, *Then* the new repos land at `~/code/github.com/<owner>/<repo>`.
- Selection follows written criteria and is recorded in `docs/backtest-repos.md` with the measured values.
  - *Given* a candidate, *When* it is evaluated, *Then* the record shows each criterion below as pass/fail with the number measured.
**Files**: `docs/backtest-repos.md`, `scripts/clone-backtest-repos.sh`

**Time-box and fallbacks (pre-mortem #4)**: tool-install and exemplar-smoke attempts are capped at a wall-clock budget named in `G0-decision.md` Part B; hitting the cap records the candidate as "fail: budget". Each JVM and JS/TS language gets three candidates, and a named fallback applies if all fail: for JVM, PIT's Maven plugin or a Gradle-based project such as `google/gson`; for JS/TS, Stryker on a plain-Jest utility library. If fewer than two per-test kill-matrix suites survive, Story 3.3.2 reports "Q3 not answered: kill-primary where a kill dimension is present; coverage-only ordering labeled unvalidated everywhere" and Story 4.2.1's "not supported" branch renders it.

**Selection criteria (all required)**: (1) full suite, or the chosen module's suite, runs green and deterministically twice in a row in <= 10 min on the owner's machine; (2) >= 200 distinct test functions (Python/JS/Java) in the module under mutation; (3) no network, database, or container dependency in the selected tests; (4) the mutation tool for that language (PIT with `fullMutationMatrix`, Stryker with `coverageAnalysis: perTest` and bail off, mutmut v3) runs to completion on the module in the spike's budget; (5) the module is not predominantly generated code; (6) license permits cloning for local analysis.

**Candidate shortlist (INFERRED from general knowledge, none verified by this plan; the smoke test below is what verifies them)**: Python: `pallets/click`, `python-attrs/attrs`, `more-itertools/more-itertools`. Java (Gradle/PIT): `apache/commons-lang` (single module, JUnit 5; if it is Maven-only, verify a Gradle wrapper or PIT's Maven plugin is installable per `research/stack.md` section 6), `google/gson`. JS/TS (Stryker via npx): `date-fns` (Vitest), `colinhacks/zod` (Vitest) or a smaller utility library meeting the criteria. Existing corpus members used for dynamic arms: `stapler-squad` (Go; mutate named packages only), `kubernetes/kubernetes` (named subpackages only), `BurntSushi/ripgrep` (named crates only). The module to mutate in each repo is named in the G0 decision file.

##### Task 0.2.2a: Select and smoke-test candidates (S)
- Evaluate 2 candidates per language (3 for JVM and JS/TS) against the criteria above within the time-box (candidates are the shortlist; Java via Gradle because `mvn` is absent locally, `research/stack.md` section 6); record pass/fail with measured wall-clock (a measurement, not an estimate) and name the chosen module.
- Files: scratch notes only

##### Task 0.2.2b: Edit docs and script (XS)
- Add rows; extend script; keep the "No Python exemplar" sentence removed.
- Files: `docs/backtest-repos.md`, `scripts/clone-backtest-repos.sh`

### Epic 0.3: Risk spikes (riskiest first)
**Goal**: Retire the three unknowns that would invalidate the experiment before spending corpus-run budget.

#### Story 0.3.1: Spike: held-out fault proxy
**As a** experimenter, **I want** a feasibility verdict on three held-out proxies, **so that** the experiment does not validate on the same metric it selected on (`research/pitfalls.md` headline).
**Acceptance Criteria**:
- A memo compares (a) failed-build-detection-loss replay over historical commits (P1), (b) mutant split by operator family and module (P2), (c) fail-to-pass replay (P1 variant), and (d) independent-tool mutants where a second tool exists (P3), on cost, label ("real" vs "same-family"), and repo applicability.
  - *Given* the Python exemplar and stapler-squad, *When* approach (a) is tried over a window of at least 200 commits (or all commits if fewer), *Then* the memo records commits built, commits with a pre-existing-test failure (the "event" count), the yield (events per commit), and commands.
- The memo applies a power gate. Detecting a difference in detection-loss proportions with a 95% CI half-width of about 0.10 at the worst-case proportion needs about 96 events (1.96^2 x 0.25 / 0.10^2); the memo projects commits needed = 96 / yield per arm.
  - *Given* a measured yield of 0.02 events per commit, *When* the memo is read, *Then* it states ~4,800 commits needed and, if that exceeds the replay budget set in 0.3.4, marks P1 as "exploratory only" for that arm.
- Fallback tree is explicit: P1 passes the power gate, report P1 as the real-fault headline and P2 as the floor; otherwise P2 is the headline, labeled same-family, and the write-up states "Q6 answered same-family only" with the event counts and CIs; P1 results are still reported as exploratory with their wide intervals.
  - *Given* the memo, *When* the owner reads its last section, *Then* it names the headline proxy per language arm, its label, and the fallback taken.
**Files**: `project_plans/test-effectiveness-critique/experiment/proxy-spike.md`

##### Task 0.3.1a: Replay feasibility on Go (stapler-squad) and Python exemplar (M)
- Script a bounded commit-window replay; measure build/test success rate and fraction with a pre-existing-test failure.
- Files: `project_plans/test-effectiveness-critique/experiment/proxy-spike.md`

##### Task 0.3.1b: Power calculation and fallback decision (S)
- Compute events needed, commits needed, and replay cost per arm from 0.3.1a's yield; write the fallback tree outcome per arm.
- Files: `project_plans/test-effectiveness-critique/experiment/proxy-spike.md`

#### Story 0.3.2: Spike: attribution cost model and sampling strategy
**As a** experimenter, **I want** measured per-test and leave-one-out costs, **so that** the 10^4-10^5 test scalability NFR has a stated sampling strategy.
**Acceptance Criteria**:
- Cost table for isolated (`go test -run '^T$'` loop), in-order (single run with per-test marks), and leave-one-out on shortlists, for Go and Python.
  - *Given* stapler-squad's Go packages, *When* each method runs, *Then* the memo records process launches, total CPU seconds, and whether isolated and in-order disagree on any test's unique-coverage set.
- A sampling rule states how a suite above a size threshold is analysed, with a stated bound.
  - *Given* a 20,000-test suite, *When* the rule is applied, *Then* it yields at most R_max leave-one-out runs, where R_max is a number in the memo (proposed default: min(200, 2% of suite) shortlisted tests, stratified by package), rather than 20,000 runs.
- Repeat-run cost for flakiness detection is measured: a second in-order run per arm, and the memo states the rule "tests whose unique-coverage set differs across the two runs are `Flaky`; arms with one run carry the footer note 'Stability not assessed: single run', exclude nothing, and record `stability: single_run` in their results so triage precision can be stratified by it".
**Files**: `project_plans/test-effectiveness-critique/experiment/cost-spike.md`, `scripts/test-effectiveness/per-test-go-coverage.sh`

##### Task 0.3.2a: Measure and write memo (M)
- Reuse the verified scratchpad Go experiment pattern from `research/pitfalls.md` P3 at real-repo scale; include the repeat in-order run.
- Files: as above

#### Story 0.3.4: Spike: mutation-run cost model and arm feasibility
**As an** experimenter, **I want** measured per-mutant cost and a bounded sampling rule for each mutation tool, **so that** the plan states which languages get a mutation-delta arm and which get coverage delta only (adversarial review B1).
**Acceptance Criteria**:
- Tools are installed and exercised once each on the named module: `gremlins` (and `avito-tech/go-mutesting` if gremlins fails) for Go on a stapler-squad package; `mutmut` v3 (and `cosmic-ray` only if mutmut fails) for Python on the Python exemplar module; `cargo-mutants` for Rust on one ripgrep crate; PIT and Stryker for the JVM and JS/TS exemplars. None is installed locally (`research/stack.md` section 6).
  - *Given* each tool, *When* run on its module, *Then* the memo records version, mutants generated, mutants run, killed/survived/timeout/uncovered counts, wall-clock total, and mean wall-clock per mutant t_m.
- The memo fixes the cost model and sampling rule used by the protocol.
  - *Given* t_m for an arm, *When* the model is applied, *Then* it states the total as T = (2 + N_nulls x K_mut) x M_s_killed x t_m, where the factor 2 is full-suite baseline plus flagged-deleted run, N_nulls is the number of null families run through mutation, K_mut the seed count, and M_s_killed the sampled mutants that the full suite kills (survivors are not rerun: a subset suite cannot kill a mutant the full suite missed).
  - *Given* the model, *When* T exceeds the arm's wall-clock cap, *Then* the memo reduces M_s (random mutant sample, same sample before and after, seeded) and module scope before it reduces K_mut, and K_mut never goes below 19 (the smallest K giving a one-sided rank p-value of 0.05 as (1 + r) / (K + 1) with r = 0).
- The memo gives a feasibility matrix with one verdict per language: `mutation-delta` (PIT/Stryker: kill matrix, so all nulls are computed from the matrix at K >= 999 for free), `bounded-module mutation-delta` (T within cap), or `coverage-delta only` (T exceeds cap even at M_s = 100 and K_mut = 19).
  - *Given* kubernetes-scale Go, *When* the matrix is read, *Then* the verdict is stated with the measured T; the default hypothesis (to be confirmed or reversed by measurement) is bounded-module mutation-delta for stapler-squad packages and ripgrep crates, coverage-delta only for kubernetes and the other corpus giants.
**Files**: `project_plans/test-effectiveness-critique/experiment/mutation-cost-spike.md`

##### Task 0.3.4a: Go: install `gremlins` (or `go-mutesting`) and measure per-mutant cost (S)
- Install via `go install`; run once on the stapler-squad package named for the arm; record version, counts and t_m in a `## Go` section. Independent of 0.3.4b-e (separate toolchain, separate failures), so the five install-and-measure tasks run in parallel and one language failing does not block the others.
- Files: `project_plans/test-effectiveness-critique/experiment/mutation-cost-spike.md` (own section)

##### Task 0.3.4b: Python: install `mutmut` v3 and measure per-mutant cost (S)
- Files: `project_plans/test-effectiveness-critique/experiment/mutation-cost-spike.md` (own section)

##### Task 0.3.4c: Rust: install `cargo-mutants` and measure per-mutant cost (S)
- Files: `project_plans/test-effectiveness-critique/experiment/mutation-cost-spike.md` (own section)

##### Task 0.3.4d: JVM: PIT via Gradle with `fullMutationMatrix` on the Java exemplar module (M)
- Files: `project_plans/test-effectiveness-critique/experiment/mutation-cost-spike.md` (own section)

##### Task 0.3.4e: JS/TS: Stryker via npx with `coverageAnalysis: perTest`, bail off (M)
- Files: `project_plans/test-effectiveness-critique/experiment/mutation-cost-spike.md` (own section)

##### Task 0.3.4f: Cost model, sampling rule, feasibility matrix (S; after a-e)
- Apply the formula per arm; fix M_s, scope, K_mut, caps as proposals for G0; a language whose install task failed is recorded `not measured: <reason>` and its verdict is `coverage-delta only`.
- Files: `project_plans/test-effectiveness-critique/experiment/mutation-cost-spike.md`

#### Story 0.3.3: Spike: test-to-production linkage resolved rate
**As a** designer, **I want** the `resolved: true` rate of `call_edges` originating in test files, **so that** static enrichment (Story 2.3.3) is built only if usable.
**Acceptance Criteria**:
- Measured rate on `stapler-squad` and `kubernetes/kubernetes` Go packages.
  - *Given* the architecture model of stapler-squad, *When* edges with `file` ending `_test.go` are counted, *Then* the memo gives total, resolved, and unresolved counts and a go/no-go.
**Files**: `project_plans/test-effectiveness-critique/experiment/linkage-spike.md`

##### Task 0.3.3a: Count edges via `kibitzer architecture` export (S)
- Use the existing JSON export; filter by file suffix; record counts.
- Files: `project_plans/test-effectiveness-critique/experiment/linkage-spike.md`

#### Story 0.3.5: Experiment execution safety policy (sandbox, secrets, resource limits)
**As an** owner, **I want** a written and enforced policy for running third-party suites and mutation tools on my machine, **so that** the corpus experiment (kubernetes, cassandra, servo, ripgrep, deno, vscode and the G0 exemplars) cannot read my credentials, mutate my tree, or hang the host.
**Scope of this policy**: it governs the experiment harness and scripts. The shipped producer recipes (Tasks 2.2.2b, 2.2.8a-b) run repo test code with the caller's environment and credentials by design: they print the executes-code notice (UX-E11, UX-E15) and the docs recommend running them inside the sandbox this story defines, but they do not enforce the allow-list, because they run on the user's own checkout at the user's request. **Context**: ADR-001 keeps kibitzer itself from executing anything; the experiment scripts (Stories 0.3.2, 0.3.4, 3.1.2, 3.1.4, 3.2.x) do execute untrusted build and test code, install-time scripts (`npm`, `pip`, Gradle, `cargo` build scripts) and mutated code. This story is the engineering counterpart of the Security classification in `requirements.md`.
**Acceptance Criteria**:
- A policy file `project_plans/test-effectiveness-critique/experiment/execution-safety.md` states, per arm, the isolation mode (`container`, `vm`, or `rootless-sandbox`), and fixes these rules.
  - *Given* the policy, *When* read, *Then* it requires: (1) the corpus checkout is bind-mounted read-only except the output directory and the tool's cache; (2) the process environment is an allow-list, so `HOME` points to a scratch directory and no variable matching `*TOKEN*`, `*SECRET*`, `*KEY*`, `*PASSWORD*`, `SSH_AUTH_SOCK`, cloud-provider credential variables or a password-manager agent socket is passed; (3) network is denied during suite and mutation runs where the exemplar's selection criterion 3 holds, and allowed only in a separate dependency-fetch step recorded in the results row; that fetch step runs under the same env allow-list and secret-free `HOME` as every other step, because install-time scripts (`npm`, `pip`, Gradle, `cargo` build scripts) execute there; (4) a wall-clock timeout and a memory limit per run, with the values taken from the G0 `MutationBudget`; (5) corpus repos are pinned by commit.
- The harness refuses to run a corpus arm without a recorded isolation mode, and every results row carries an `isolation` field.
  - *Given* an arm started with no isolation mode, *When* Story 3.1.2's harness runs, *Then* it exits non-zero with `isolation mode not recorded for arm <name>` and writes no row.
  - *Given* a completed arm, *When* its JSONL record is read, *Then* it has `isolation` (one of the three modes), `network` (`denied` or `fetch-step`), and `env_allowlist_hash`.
- A preflight script verifies the policy on this machine before the first run.
  - *Given* a run launched with `SSH_AUTH_SOCK` set in the parent shell, *When* the preflight inspects the child environment, *Then* it reports the variable absent in the sandbox and exits 0; *Given* a sandbox that exposes `~/.ssh`, *Then* it exits non-zero naming the path.
- Mutation tools that rewrite sources (Stryker, mutmut, cargo-mutants, PIT) run against a throwaway copy, not the owner's checkout.
  - *Given* a completed mutation run, *When* `git status --porcelain` runs in the original checkout, *Then* it is empty.
- Secrets scrubbing applies to artifacts: logs and results committed under `project_plans/` are scanned before commit.
  - *Given* a results file containing a string matching the secret patterns this story defines, *When* the pre-commit scan runs, *Then* the commit is refused and the match is named. The repo has no secret-scan mechanism today (no pre-commit, githooks or lefthook config and no pattern file, verified by listing the repo root), so Task 0.3.5b creates both the hook and the pattern list rather than reusing one.
**Files**: `project_plans/test-effectiveness-critique/experiment/execution-safety.md`, `scripts/test-effectiveness/preflight-sandbox.sh`, `scripts/test-effectiveness/secret-patterns.txt`, `scripts/test-effectiveness/secret-scan.sh`

##### Task 0.3.5a: Write the policy and the per-arm isolation table (S)
- Files: `project_plans/test-effectiveness-critique/experiment/execution-safety.md`

##### Task 0.3.5b: Preflight script, secret-pattern list and secret-scan hook for results artifacts (S)
- Create `scripts/test-effectiveness/secret-patterns.txt` (token, key, password and cloud-credential patterns plus the `SSH_AUTH_SOCK` path shape) and a scan script wired as an opt-in `git` pre-commit hook installed by a one-line setup command (the repo has no hook framework; this story does not add one). The scan covers `project_plans/test-effectiveness-critique/experiment/results/`, logs, and the committed triage JSONL under `docs/backtest-triage/` (derived from third-party code, so it can carry their secrets or paths). Decide and state where the hook is installed, because `project_plans/` is committed.
- Files: `scripts/test-effectiveness/preflight-sandbox.sh`, `scripts/test-effectiveness/secret-patterns.txt`, `scripts/test-effectiveness/secret-scan.sh`

### Epic 0.4: Owner gate G0

#### Story 0.4.1: Record the G0 decision
**As an** owner, **I want** the experiment's pre-registered numbers and scope fixed in one file before any experiment spend, **so that** thresholds cannot move after results are seen.
**Acceptance Criteria**:
- `project_plans/test-effectiveness-critique/decisions/G0-decision.md` exists, authored by the owner from the "G0 decision template (ready to fill)" section of this plan (every proposed default there is cited to its source and each line is signed or edited by the owner) (the agent may draft proposals from spikes 0.3.1, 0.3.2, 0.3.4 and Story 3.1.1's defaults), in two parts committed separately so Phase 1 is not blocked behind the spikes:
  - **Part A (static checks; needs no spike)**: the primary persona ranking, the H4 precision floor, the hook per-file-cap percentile rule, the structural confidence caps from "Confidence caps", the non-specific-reason stoplist rule, the owner's committed triage volume (verdicts the owner will review, see blind-label rule below), the SM5 target and decision rule (template line A8) and the zero-occurrence rule for Story 1.2.1 (line A9).
  - **Part B (experiment; after spikes 0.3.1, 0.3.2, 0.3.4)**: headline proxy per arm and fallback taken (0.3.1); exemplars and named modules (0.2.2); `MutationBudget` per arm and the feasibility verdict per language (0.3.4); K seeds per null family; the sampling bound R_max (0.3.2); hypothesis thresholds for H1-H3 and the invariant I0 including equivalence margins; the linkage go/no-go threshold (0.3.3); the pre-registered rules for `ConfidenceTier` derivation and the `--by-package` threshold from Story 2.1.5 (the rule, not a value, for any parameter whose experiment has not yet run); the isolation mode per arm (Story 0.3.5); the input size cap and parse-memory bound (Story 2.2.7); the wall-clock budget below; and the outcome table below.
  - *Given* Part A committed, *When* Story 1.2.2 or 1.3.1 starts, *Then* it cites Part A's commit hash and does not wait for Part B.
  - *Given* the file, *When* Story 3.1.1 starts, *Then* it cites Part B's commit hash, and the protocol's numbers equal the file's.
- Wall-clock budget and stop-loss (pre-mortem #3): Part B records the total wall-clock hours corpus runs may occupy, an arm order by information per hour (PIT and Stryker matrix arms first, since their nulls are free from the matrix at K >= 999), and the stop-loss "after each arm, if actual wall-clock exceeds 1.5x the 0.3.4 projection, stop and record the arm as `not run: budget`". The Phase 3 rows of the Effort table carry a wall-clock column filled from 0.3.4 before Part B is recorded.
  - *Given* an arm whose measured wall-clock reaches 1.6x its projection, *When* the harness reports, *Then* the arm record says `not run: budget` and later arms still run.
- Outcome table (pre-mortem #1), fixed at Part B so a null result has a defined deliverable. `docs/test-effectiveness.md` with a `NegativeResultReport` and the literature deliverable (Success Metrics 1, 2) ship in every row; static checkers stay registered and opt-in in every row where no rule passes H4:

  | Static (H4 floor met by >= 1 rule/language) | H2 (JointSafe beats N2 and N4 on held-out proxy) | H3 (Q3 supported on JVM and JS/TS) | What ships |
  |---|---|---|---|
  | promote | pass | supported | Promoted static rules in `default_checks()` (4.1.1); report unhidden with tiers, coverage-primary ordering where Q3 supports it (4.2.1) |
  | promote | pass | not supported | As above; kills primary where present; coverage-only ordering labeled unsupported |
  | promote | fail | any | Promoted static rules; report ships descriptive-only (see below) |
  | none | pass | any | Report with tiers; static checkers stay opt-in with the FP doc |
  | none | fail | any | Descriptive-only report; static checkers stay opt-in; docs carry the negative results |

  *Descriptive-only* means: the report is unhidden under the exact label "descriptive" with a non-validation banner ("signals not validated as removal predictors; see docs/test-effectiveness.md"), prints unique coverage and joint-loss counts and the exclusions, carries no `ConfidenceTier` column, no ordering claim, and no `JointSafe` "safe to remove" wording. Removing the hidden `Command::Tests` variant happens only if the owner writes `remove` in `G2-decision.md`.
  - *Given* H2 reported "not beaten" on every arm, *When* Story 4.2.1 runs, *Then* the shipped artifact is the descriptive-only report plus `NegativeResultReport`, and Success Metric 4 is recorded as met by that report (interpretation accepted below).
- Owner acceptances recorded in Part B: (a) Success Metric 3 is met in part (dynamic arms on stapler-squad, named kubernetes subpackages, named ripgrep crates and the G0 exemplars; cassandra, servo, deno, vscode static only); (b) Success Metric 4's "shipped check or report" is satisfied by the descriptive-only report on a null result; (c) the `Part A` triage-volume commitment.
  - *Given* Part B, *When* read, *Then* each of (a), (b), (c) is a signed line (owner initials/date or commit author) not an agent draft.
- SM3 and SM4 numeric thresholds live here: Part B records H1 margins, H2 rank-p and null counts, H3 and H4 Wilson floors, and the SM4 reading (a "validated" signal needs a supported H2 on the held-out proxy). `requirements.md` carries no numbers for them.
  - *Given* `requirements.md`, *When* searched for a numeric SM3 or SM4 threshold, *Then* none is found and the text points to `G0-decision.md` Part B.
- Every Phase 3 story lists `G0-decision.md` Part B as a precondition.
  - *Given* no such file on `master`, *When* an agent picks up Story 3.1.1, *Then* it stops and reports the blocker.
**Files**: `project_plans/test-effectiveness-critique/decisions/G0-decision.md`

##### Task 0.4.1a: Part A: persona, static numbers and caps; owner records from the template (XS)
- Files: as above

##### Task 0.4.1b: Part B: draft proposals from spikes, outcome table, budget, isolation, caps, acceptances; owner records from the template (XS)
- Files: as above

---

# Phase 1: Static vacuity checkers (registry-only until G1)

### Epic 1.1: `test-quality` native checkers
**Goal**: Hook-latency static signals for the agent-just-wrote-a-test consumer, narrow and exempted per ADR-004.

**Sequencing (pre-mortem #6)**: noise is measured at one third of the build cost. Build and triage Go and Python for `test-no-assertion` first (Tasks 1.1.1a, 1.1.2a, then Story 1.2.2's Go and Python triage); Stories 1.1.3, 1.1.4 and the TS/JS, Java/Kotlin, Rust rule tables (Tasks 1.1.1b, 1.1.2b) start after that triage records the dominant false-positive modes in `docs/test-quality-false-positives.md`. All stay in scope; only the order changes.

#### Story 1.1.1: False-positive fixture suite (tests first)
**As a** maintainer, **I want** labeled fixtures for every pitfall P5/P10 failure mode, **so that** each checker is built against known-good tests first.
**Acceptance Criteria**:
- Fixtures per language cover: assertion in helper, in closure/`t.Cleanup`/`.then`, snapshot/golden, fuzz seed test, `// Output:` example, expected-exception, mock-verify oracle, "does not panic" smoke.
  - *Given* `testdata/test-quality/go/helper_assert_test.go` with `func TestX(t *testing.T){ requireOK(t, run()) }` and `requireOK` calling `t.Fatal`, *When* `test-no-assertion` runs, *Then* zero findings.
- True-positive fixtures cover empty body, no-assertion body, `assert.True(t, true)`, `expect(1).toBe(1)`, self-comparison.
  - *Given* `def test_x(): parse("")` in `empty_test.py`, *When* `test-no-assertion` runs, *Then* one finding at line 1 anchored on `def test_x`.
- A weak-oracle fixture family (`assert result is not None`, `len(x) > 0`, truthiness-only checks) is labeled "known false negative" and asserts zero findings from `test-no-assertion` and `test-tautology`, so the gap is documented and not silently widened (pre-mortem #7).
  - *Given* `def test_x(): assert parse("") is not None`, *When* both rules run, *Then* zero findings and the fixture's header comment says "known false negative: weak oracle".
**Files**: `testdata/test-quality/{go,python,ts,java,kotlin,rust}/`, `src/checkers/test_quality_tests.rs`

##### Task 1.1.1a: Go and Python fixtures plus table-driven test harness (M)
- Files: `testdata/test-quality/go/*`, `testdata/test-quality/python/*`, `src/checkers/test_quality_tests.rs`

##### Task 1.1.1b: TS/JS, Java/Kotlin, Rust fixtures (S)
- Files: `testdata/test-quality/{ts,java,kotlin,rust}/*`

#### Story 1.1.2: `test-no-assertion` checker
**As an** AI agent, **I want** to be told when a test I just wrote asserts nothing, **so that** I fix it before claiming done.
**Acceptance Criteria**:
- Flags a test function containing no call matching `AssertionVocabulary` after resolving same-file helpers to bounded depth; treats passing the framework handle to an unresolved function as asserting.
  - *Given* Go `func TestA(t *testing.T){ Parse("") }` in `a_test.go`, *When* the checker runs, *Then* one finding `[test-no-assertion]` at the `func TestA` line, with evidence text naming "no assertion call found in body or same-file helpers".
- A precision-first mode is available behind an option and measured in Story 1.2.2 against the strict mode: an unresolved call whose callee name or import path matches an extendable assertion-ish pattern (`assert|check|verify|expect|require|must`) counts as asserting.
  - *Given* Python `def test_x(): helpers.check_x(parse(""))` where `helpers` is in another file, *When* the checker runs in strict mode, *Then* one finding; *When* it runs in precision-first mode, *Then* zero findings.
- Message follows ADR-004 wording (strengthen / justify / remove with named covering test), never "delete".
  - *Given* the finding above, *When* its message is read, *Then* it contains `kibitzer:ignore test-no-assertion -- <why>` and does not start with "Delete".
- Findings suppressed by `kibitzer:ignore` and absent on tests the diff did not add or signature-change when diff-scoped (the finding is anchored on the function's first line; an edit that removes assertions from the body of an existing test is covered only by conditional Story 4.1.2).
  - *Given* a changed range excluding `func TestA`, *When* hook mode runs, *Then* the finding is dropped by `scope_output_to_changed_lines`.
  - *Given* an edit that deletes the only assertion inside the body of existing `TestA` without touching its first line, *When* hook mode runs, *Then* no `[test-no-assertion]` finding appears (documented limitation, see 4.1.2).
- Message shape (UX-H1, H2): headline `file:line: [rule] message`, then at most three indented lines (likely fix, non-claim, `kibitzer:ignore <rule> -- <why>` quiet path), 4 lines total; first word is never "Delete" or "Remove"; no "must", "critical", "urgent".
  - *Given* the Story 1.1.2 Go example finding, *When* its text is read, *Then* it is <= 4 lines, names strengthening as the first likely fix, and tells the agent to name the covering test before removing.
- Agent guidance (UX-A1): for a test the agent just added, the message lists strengthening first and states that removal is acceptable only after naming the covering test in the reply.
  - *Given* a `test-no-assertion` finding on a test added in the diff, *When* the message is read, *Then* the first "Likely fix" clause is "assert on" or "strengthen", and the removal clause contains "name it in your reply" (or the equivalent fixed phrase tested verbatim).
  - *Given* the decision tree in `docs/test-quality.md`, *When* its "pre-existing test" branch is read, *Then* it says no finding is emitted in hook mode and never prompts an agent to prune.
  - *Given* the printed `kibitzer:ignore` line with `<why>` replaced, *When* copied verbatim above the function and re-run, *Then* the finding is gone (UX-H6).
- Silent and safe states (UX-H4, H5; ux.md 2.3): a clean file produces no test-quality output (no "0 findings" line); a language with no `TestCodeProfile` produces no output and no error; skipped tests (`t.Skip`, `@Disabled`, `xit`) and generated files produce nothing.
  - *Given* a `.cs` file with an assertion-free test, *When* hook mode runs, *Then* no output and exit status unchanged.
- Per-file cap (UX-H7): at most N test-quality findings per file in hook mode, with extras collapsed into exactly one `  +N more test-quality findings in this file (run kibitzer run <file> for all)` line. N is a pre-registered parameter (see "Pre-registered parameters"): the cap mechanism ships here, its value is set from Story 1.2.2 and recorded in `G1-decision.md`; until recorded, no collapsing occurs.
  - *Given* a cap value N recorded in `G1-decision.md` and a file with N+3 findings, *When* hook mode runs, *Then* N findings plus one `+3 more` line appear.
**Files**: `src/checkers/test_no_assertion.rs`, `src/checkers/mod.rs`, `src/test_code/mod.rs`, `src/test_code/{go,python,ts,java,rust}.rs` (rule tables per language)

##### Task 1.1.2a: Go + Python implementation against fixtures (M)
- Use `tree_walk` and typed kinds (`GoKind`, Python equivalent); helper resolution within file only.
- Files: `src/checkers/test_no_assertion.rs`, `src/checkers/mod.rs`

##### Task 1.1.2b: TS/JS, Java/Kotlin, Rust rule tables (S)
- Files: `src/checkers/test_no_assertion.rs`, `src/test_code/ts.rs`, `src/test_code/java.rs`, `src/test_code/rust.rs`

##### Task 1.1.2c: Message shape, silent states, per-file cap mechanism (XS)
- Shared message builder for all three static rules (likely-fix / non-claim / ignore lines, banned-word test); cap collapse reading the value from config, unset by default; tests for UX-H1, H2, H4-H7. Value of N is not chosen here.
- Files: `src/checkers/test_quality_tests.rs`, `src/test_code/mod.rs`

#### Story 1.1.3: `test-tautology` checker
**As an** AI agent, **I want** trivially-true assertions flagged, **so that** assertion padding does not silence `test-no-assertion`.
**Acceptance Criteria**:
- Flags literal-true assertions and self-comparisons.
  - *Given* `expect(1).toBe(1)` in `a.test.ts`, *When* the checker runs, *Then* a `[test-tautology]` finding at that line.
  - *Given* Go `assert.Equal(t, got, got)`, *When* the checker runs, *Then* a finding naming the repeated operand `got`.
- Does not flag legitimately constant comparisons with a computed side.
  - *Given* `assert.Equal(t, 4, Add(2, 2))`, *When* the checker runs, *Then* zero findings.
**Files**: `src/checkers/test_tautology.rs`, `src/checkers/mod.rs` (per-language tautology shapes live in the checker file, so Tasks 1.1.2b and 1.1.3a-b do not share a language file)

##### Task 1.1.3a: Go + Python + TS/JS (S)
##### Task 1.1.3b: Java/Kotlin + Rust (S)
- Files: `src/checkers/test_tautology.rs`

#### Story 1.1.4: `test-mock-only` checker (opt-in, medium confidence)
**As a** reviewer, **I want** tests whose only oracle is a mock interaction listed, **so that** I can check the interaction is the behaviour.
**Acceptance Criteria**:
- Flags a test whose sole assertions are mock verification calls and which has no state/return assertion.
  - *Given* Java `verify(repo).save(x);` as the only statement after `service.create(x)`, *When* the checker runs, *Then* a `[test-mock-only]` finding with confidence text "medium: interaction may be the behaviour".
- Not registered into any default chain; selectable by name only.
  - *Given* default config, *When* `kibitzer check list` runs, *Then* `test-mock-only` is listed as an available native checker and `kibitzer run` on a file with a mock-only test reports nothing.
**Files**: `src/checkers/test_mock_only.rs`, `src/checkers/mod.rs`

##### Task 1.1.4a: Java/Kotlin + TS/JS verify vocab (S)
##### Task 1.1.4b: Go (`gomock`/`AssertExpectations`) + Python (`assert_called*`) (S)
- Files: `src/checkers/test_mock_only.rs`

#### Story 1.1.5: Registration without default-on
**As an** owner, **I want** the checkers reachable by name but not in `default_checks()`, **so that** G1 controls exposure.
**Acceptance Criteria**:
- `test_quality_checks()` exists and returns `Check` entries that nothing chains at `src/config.rs:759-769` (precedent: `prose_checks()`, `src/config.rs:784`); each checker module registers itself with `inventory::submit!`, and `src/checkers/mod.rs` gains only `pub mod` lines.
  - *Given* a repo with no `.kibitzer/inspect.json`, *When* `kibitzer run` executes on a file with an assertion-free test, *Then* no `[test-no-assertion]` finding appears.
  - *Given* an inspect.json enabling `test-no-assertion`, *When* the same run executes, *Then* the finding appears.
  - *Given* default config, *When* `kibitzer check list` runs, *Then* all three test-quality checkers are listed.
- Default state is visible (UX S6): `kibitzer check list` shows each test-quality rule with its default state `off` before G1 and the languages it supports.
  - *Given* default config before a G1 record, *When* `kibitzer check list` runs, *Then* the `test-no-assertion` line shows state `off` and a languages list.
- An unknown option key never passes silently (UX S6): the warning names the key and the valid keys.
  - *Given* `.kibitzer/inspect.json` with `"options": {"assertion_helper": ["x"]}` (typo) under `test-no-assertion`, *When* any run loads config, *Then* stderr contains a warning naming `assertion_helper` and listing `assertion_helpers`.
- Hook and MCP paths never emit dynamic rules (UX-A2): the `inventory`-registered test-quality checkers produce only `test-no-assertion`, `test-tautology`, `test-mock-only` (and `test-assertion-removed` if built); no rule from the "report only" rows of the Rule-id registry is reachable from `kibitzer run`, hook, or `run_checks`.
  - *Given* any input file and a repo with coverage and mutation reports present in the working tree, *When* hook mode or `run_checks` runs, *Then* no `test-zero-unique-*` or `test-pseudo-tested-region` finding appears and no adapter module is invoked.
**Files**: `src/config.rs`, `src/checkers/mod.rs`

##### Task 1.1.5a: Add `test_quality_checks()` (XS)
##### Task 1.1.5b: Integration test for default-off and opt-in (XS)
- Files: `src/config.rs`, `tests/` (new `tests/test_quality_registry.rs`)

### Epic 1.2: Evidence for static checks
**Goal**: Meet the `CLAUDE.md` "Writing a new check" bar: transcript backtest and corpus run, with precision recorded.

#### Story 1.2.1: Transcript backtest and occurrence mining
**As an** owner, **I want** to know whether agents actually write assertion-free or tautological tests, **so that** the `docs/check-ideas.md` "confirmed transcript occurrence" bar is met or the corpus-only justification is documented.
**Acceptance Criteria**:
- `kibitzer check backtest` is run per rule over `~/.claude/projects/*/*.jsonl` and counts of hits, hits introduced by the edit, and manual labels are recorded.
  - *Given* `kibitzer check backtest test-no-assertion --only-new` (flag `only_new` at `src/main.rs:311`), *When* run, *Then* a table in `docs/check-ideas.md` records hit count and at least 10 manually labeled examples (or "fewer than 10 found").
- Zero-occurrence rule: if the backtest finds ~zero confirmed occurrences of assertion-free or tautological agent-written tests (the threshold for "~zero" is the owner's, recorded at G0 Part A line A9), the finding does not auto-cancel Phase 1. The default is `proceed-corpus-only` (Stories 1.2.2 and 1.3.1 still run on the corpus-only justification `docs/check-ideas.md` allows, and the zero count is stated in the G1 record); `registry-only` is the owner's alternative and holds Story 4.1.1 and the second-wave language tables until an occurrence is found.
  - *Given* the backtest table with zero confirmed occurrences, *When* G0 Part A line A9 reads `proceed-corpus-only`, *Then* Story 1.2.2 starts and `G1-decision.md` quotes the zero count; *When* it reads `registry-only`, *Then* Story 4.1.1 stops and reports the blocker.
- Evidence for assertion-padding and assertion-removal is recorded as found / not found.
  - *Given* the transcripts, *When* searched for edits that remove an `assert` line, *Then* the count is recorded and gates Story 4.1.2.
- The SM5 flagged-then-revised rate is defined and baselined on the same transcripts (Task 1.2.1c), with the sample size.
  - *Given* the transcripts, *When* the baseline is computed, *Then* the table lists denominator, numerator, the three complement buckets and the session count, or "denominator below 10: not estimable".
- Goodhart measurements (pre-mortem #7, UX-A3) are recorded before G1: (a) the rate and reason diversity of `kibitzer:ignore` directives added in the turn immediately after a finding; (b) the share of fixes that add only `not None` / `len > 0` / truthiness assertions. If (b) is observed, the `test-tautology` weak-assertion wording is revised before G1 and the revision is cited.
  - *Given* the transcripts, *When* searched for assertions added in the turn after a `test-no-assertion` finding, *Then* the table in `docs/check-ideas.md` lists counts for (a) and (b), or "not observed in N sessions", with the session count.
**Files**: `docs/check-ideas.md`

##### Task 1.2.1a: Run backtests and label (M)
##### Task 1.2.1b: Write findings into `docs/check-ideas.md` (S)
- Files: `docs/check-ideas.md`

##### Task 1.2.1c: Define and baseline the flagged-then-revised rate (SM5) (S)
- Write the metric definition (denominator: tests an agent added in a session that received a test-quality finding in hook mode or in a backtest replay; numerator: the next edit of that test strengthens it with a non-weak assertion; the complement is split into four buckets: ignore-directive, removal with a named covering test in the agent's reply, removal without one, and unchanged; removal is its own pair of buckets because deleting the just-added test is the cheapest way to silence a finding, and the hook cannot verify a named covering test, so the transcript reader checks the reply text), compute the pre-ship baseline over the transcripts and label it the natural strengthening rate of agents that never saw the finding (a floor for the live figure, not a treatment effect; the target and decision rule are G0 Part A line A8), and record how the live figure will be read from the hook log (`src/hook_log.rs`) after G1. Not a gate; it is the post-ship adoption indicator in `requirements.md` (SM5).
- Files: `docs/check-ideas.md`, `scripts/test-effectiveness/flagged-then-revised.py`

#### Story 1.2.2: Corpus run and triage
**As an** owner, **I want** per-rule, per-language precision from real code, **so that** G1 is a measured decision.
**Acceptance Criteria**:
- Each rule runs over the whole corpus tree and a stratified sample is triaged with `scripts/backtest-triage.py`.
  - *Given* `find ~/code/github.com/kubernetes/kubernetes -name '*_test.go' -print0 | xargs -0 -n1 kibitzer check native test-no-assertion`, *When* run, *Then* flagged count per repo is recorded and at least 50 findings (or all, if fewer) per rule per language have a `true_positive`/`false_positive`/`needs_discussion` verdict in `docs/backtest-triage/<repo>/test-no-assertion.jsonl`.
- Go and Python are triaged first (see Epic 1.1 sequencing); the strict and precision-first resolution modes (Story 1.1.2) are both measured so G1 can choose per language.
  - *Given* the Go and Python triage, *When* the record is read, *Then* it shows true/false-positive counts for both modes and the dominant false-positive mode.
- Verdict reliability (pre-mortem #9): the owner independently labels a seeded random 10% of triaged findings (minimum 30 per rule per language, or all if fewer), blind to the agent's verdict; agreement is reported; Wilson bounds in G1 are computed on owner-confirmed labels, or on agent labels with the measured disagreement stated alongside. Triage volume is capped at the volume committed in G0 Part A.
  - *Given* the triage JSONL, *When* the record is read, *Then* each blind-labelled row carries `owner_verdict` and the summary states the agreement rate.
- The per-file finding-count distribution of the corpus run is recorded per rule per language, as the input to the hook per-file cap (rule pre-registered at G0 Part A).
  - *Given* the corpus run output, *When* the triage record is read, *Then* it lists the count of files with 1, 2, ... findings and the percentile named in `G0-decision.md`.
- False-positive modes are documented in `docs/test-quality-false-positives.md` following the `docs/*-false-positives.md` pattern.
  - *Given* a triaged false positive (helper assertion in another file), *When* the doc is read, *Then* the mode, an example permalink, and the mitigation are listed.
**Files**: `docs/backtest-triage/**`, `docs/test-quality-false-positives.md`

##### Task 1.2.2a: Run and sample (M)
##### Task 1.2.2b: Triage verdicts (S)
##### Task 1.2.2c: Write FP doc and iterate rules on found modes (S)
- Files: as above plus `src/checkers/test_no_assertion.rs` if a mode needs a fix

#### Story 1.2.3: Hook-latency measurement
**As an** owner, **I want** per-file timing, **so that** the NFR (same budget as existing checkers) is evidenced.
**Acceptance Criteria**:
- p95 per-file check time on the corpus is no worse than the slowest existing per-file test-related checker.
  - *Given* 1,000 sampled test files, *When* `kibitzer check native <rule>` is timed per file, *Then* the p95 value is recorded beside `go-table-driven-test-candidate`'s.
- An absolute budget also holds: p95 milliseconds per file on the full hook path (process start, diff scoping, all enabled test-quality rules, output), compared with the budget the owner committed in the budget commit of `G1-decision.md` before this story ran. A relative bar alone could pass while the hook is slow for every checker.
  - *Given* the budget commit precedes the measurement commit in git history, *When* the p95 on the full hook path is recorded, *Then* the record states pass or fail against the budget and the machine used; *Given* no budget commit, *Then* the story stops and reports the blocker.
**Files**: `docs/test-quality-false-positives.md` (appendix)

##### Task 1.2.3a: Time (relative and absolute, full hook path) and record (XS)

### Epic 1.3: Owner gate G1

#### Story 1.3.1: Record the G1 decision
**As an** owner, **I want** a per-rule, per-language promote / keep-opt-in / drop decision on file, **so that** Story 4.1.1 is mechanically gated.
**Acceptance Criteria**:
- Precondition: `G0-decision.md` Part A is committed (H4 floor and cap percentile; no dependency on spikes 0.3.x).
- `project_plans/test-effectiveness-critique/decisions/G1-decision.md` lists each rule and language with the triaged precision (lower Wilson bound, n, owner-vs-agent agreement), the H4 floor from `G0-decision.md` Part A, the strict vs precision-first mode chosen per language, and the verdict (Q5). The verdict is per language, not per rule.
  - *Given* the file, *When* Story 4.1.1 starts, *Then* it chains only rules whose verdict is "promote" and cites the file's commit.
- The file records the two Goodhart measurements from Story 1.2.1; if (a) shows boilerplate ignore reasons, the hook-mode quiet-path line changes from the copyable variant V1 to the pointer variant V2 of `design/ux.md` section 2.1a. The quiet-path line is never removed from hook output (UX-H1, H6, H8): dropping it would make dismissal undiscoverable and push agents to the cheaper exit of deleting the test.
  - *Given* measurement (a) recorded as high, *When* Story 4.1.1 lands, *Then* hook-mode text carries the V2 pointer line and no copyable directive, `kibitzer run` text keeps V1, and every finding still has exactly one quiet-path line.
- The file's first commit records the absolute hook latency budget (the "budget commit"), before Story 1.2.3 measures; the verdict commit records pass or fail against it, and a rule that fails the budget is not promoted.
  - *Given* `G1-decision.md` history, *When* walked, *Then* the budget commit is an ancestor of the Story 1.2.3 measurement commit and of the verdict commit.
- The file also records the hook per-file cap value N (Story 1.1.2), derived by the G0-registered rule from Story 1.2.2's distribution, and the non-specific-reason stoplist (Story 2.3.2).
  - *Given* the file, *When* Story 1.1.2's cap is configured, *Then* N equals the recorded value.
**Files**: `project_plans/test-effectiveness-critique/decisions/G1-decision.md`

##### Task 1.3.1a: Compile precision table; owner records verdicts (XS)

---

# Phase 2: Ingest-only batch report (parallel with Phase 1)

### Epic 2.1: Matrix model and algorithms
**Goal**: The normalized `TestMatrix` and pure analyses (ADR-005).

#### Story 2.1.1: `TestMatrix`, newtypes, `Provenance`
**As a** developer, **I want** one validated matrix type, **so that** adapters and algorithms are decoupled.
**Acceptance Criteria**:
- Each dimension (coverage, kills) is built separately and rejects inputs missing tool or version.
  - *Given* a builder with `Provenance { tool: "coverage.py", version: None }`, *When* `build()` is called, *Then* it returns `Err(MissingProvenance)`.
- A missing input commit is not an error: `Provenance.commit` is `Option`, and its absence is a warning (UX R11), not a rejection.
  - *Given* a builder with tool and version but `commit: None`, *When* `build()` is called, *Then* it returns `Ok` and the report prints "warning: report has no commit; cannot check staleness. Treat results as unverified against the current tree." above the table.
- Attribution is never silently guessed (see "Attribution source" under the Glossary): carried by the input, else `--attribution`, else the adapter's documented default printed as assumed.
  - *Given* a coverage.py input and no `--attribution`, *When* the report runs, *Then* the header reads `attribution: in-order (assumed; pass --attribution to override)` and a class W warning states that in-order caps coverage-derived tiers at `Medium`.
  - *Given* the same input with `--attribution isolated`, *When* the report runs, *Then* the header reads `attribution: isolated` and the assumed-attribution warning is absent.
- A matrix may hold coverage from one tool and kills from another, each with its own provenance and attribution/completeness.
  - *Given* a coverage dimension from `coverage.py` (`InOrder`) and a kill dimension from PIT (`Full`), *When* a finding is rendered, *Then* a coverage-derived finding prints coverage.py's tool, version, and `InOrder`, and a kill-derived finding prints PIT's and `Full`.
- Storage is the sparse inverted index (posting lists of `u32` test ordinals per unit), not a dense bitset.
  - *Given* a unit covered by exactly one test, *When* `unique_coverage` runs, *Then* the unit is found from its posting list length alone, without materializing any test-by-unit grid.
- Ids are newtypes with stable ordering.
  - *Given* tests `b_test.go::TestB` and `a_test.go::TestA`, *When* iterated, *Then* `TestA` precedes `TestB`.
**Files**: `src/test_effectiveness/mod.rs`, `src/test_effectiveness/matrix.rs`, `src/main.rs`

##### Task 2.1.1a: Types, inverted index, per-dimension builder (S)
##### Task 2.1.1b: Unit and property tests (S)
- Files: `src/test_effectiveness/matrix.rs`

#### Story 2.1.2: Analyses
**As an** owner, **I want** unique coverage, unique kills, greedy set cover, subsumption, and joint-loss, **so that** the report can rank tests with evidence.
**Acceptance Criteria**:
- Unique coverage equals set difference against union of others.
  - *Given* tests T1 covers {1,2,3}, T2 covers {2,3}, T3 covers {3,4}, *When* `unique_coverage` runs, *Then* T1 -> {1}, T2 -> {}, T3 -> {4}.
- Greedy cover is deterministic and union-preserving.
  - *Given* the same three tests, *When* `greedy_reduction` runs twice, *Then* both return {T1, T3} and the union equals {1,2,3,4}.
- Joint loss is computed for a set, not per test, and the flagged set is always evaluated as a set.
  - *Given* tests A and B that both cover only {1} and no other test covers 1, *When* `joint_loss({A,B})` runs, *Then* it returns {1} (each looks redundant alone; jointly necessary).
  - *Given* the same A and B plus C covering {2} with other coverage of 2, *When* `joint_safe_subset` runs over the individually flagged {A,B,C} in test-id order, *Then* it returns {A,C} (A is kept in the deletion set, B is rejected because its addition makes the joint loss non-empty), and the report lists B as "redundant only with A".
  - *Given* a kill dimension, *When* `joint_safe_subset` runs, *Then* the same rule applies over killed mutants (a test is added only if no mutant loses all its killers).
- Subsumption relative to suite only.
  - *Given* mutant m1 killed by {T1,T2} and m2 killed by {T1}, *When* `dominators` runs, *Then* m2 is a dominator of m1 (every test killing m2 kills m1) and the output labels it "relative to supplied suite".
**Files**: `src/test_effectiveness/analysis.rs`

##### Task 2.1.2a: Unique coverage/kills, greedy cover, joint loss, `joint_safe_subset` (M)
##### Task 2.1.2b: Subsumption + property tests (union preserved; result subset-minimal) (S)
- Files: `src/test_effectiveness/analysis.rs`

#### Story 2.1.3: Exclusions and stability
**As an** owner, **I want** failing, skipped, flaky (printed as `inputs disagree`), generated, stale, no-source and integration/snapshot tests handled explicitly, **so that** absence of a flag is not read as endorsement.
**Acceptance Criteria**:
- Tests that disagree across repeated inputs get `ExclusionReason::Flaky` and no dynamic finding; with a single input, stability is a footer note, not an exclusion (consistency review B1): every test stays assessable, no tier is capped by it, and the footer says so.
  - *Given* two matrices for the same suite where `TestC`'s unique coverage differs, *When* analysed together, *Then* `TestC` is excluded as `Flaky` and listed in "not assessed".
  - *Given* one matrix only and 212 assessable tests, *When* analysed, *Then* the footer reads "Stability not assessed: single run", 212 tests are assessed, the "Not assessed" line has no `single-run` reason, and no test is labeled stable.
- Tests under `TestKind::Integration`/`Snapshot` are never flagged by coverage alone.
  - *Given* `tests/e2e/login_test.go::TestLogin` with zero unique coverage, *When* analysed, *Then* it is reported as `Low` tier or excluded with reason, not flagged `Medium`.
- Table-driven tests are one `Test`.
  - *Given* Go `TestParse` with 40 subtests, *When* analysed, *Then* one candidate `TestParse` appears and none for subtests unless `--subtests` is passed.
- `ExclusionReason` includes `Stale` and `NoSource`; both are printed in "Not assessed" (UX-E5, E6).
  - *Given* a report produced at commit `9f2c1ab` and a test file changed in `9f2c1ab..HEAD`, *When* analysed, *Then* the tests in that file are not flagged and are counted under "Not assessed: stale".
  - *Given* 14 test ids in the input that resolve to no source file (`NotFound` or `Ambiguous`), *When* analysed, *Then* the report completes with exit 0, counts them under "Not assessed: no-source", and emits no finding for them.
**Files**: `src/test_effectiveness/exclusions.rs`, `src/test_code/mod.rs`

##### Task 2.1.3a: Stability and exclusion model (S)
##### Task 2.1.3b: Kind tagging via `TestCodeProfile` and path/name heuristics (S)
- Files: as above

##### Task 2.1.3c: `Stale` and `NoSource` variants and report counts (XS)
- Add both variants to `ExclusionReason`; exhaustive-match fixes; both print in "Not assessed" with counts (0 included); unit tests for UX-E5, E6.
- Files: `src/test_effectiveness/exclusions.rs`

#### Story 2.1.4: `TestLocator`: TestId to source location
**As a** report author, **I want** every `TestId` resolved to a file and line, **so that** kind tagging, `kibitzer:ignore` anchoring, and the adjudication brief have a location (architecture review concern 2).
**Acceptance Criteria**:
- Resolution uses `TestCodeProfile::test_functions` over the repo and the adapter's raw id form (Go profile name plus package, coverage.py dotted context, PIT class and method, Stryker name plus `testFiles`).
  - *Given* a coverage.py context `tests.test_parse.test_empty` and a repo with `tests/test_parse.py::test_empty` at line 12, *When* located, *Then* the result is `Located(tests/test_parse.py, 12)`.
  - *Given* Go profile `TestParse` present in two packages with no package encoded, *When* located, *Then* the result is `Ambiguous([pkgA/..., pkgB/...])` and the test is excluded as `NoSource` rather than guessed.
  - *Given* an id naming no test function, *When* located, *Then* `NotFound` and `ExclusionReason::NoSource`.
- The Go recipe encodes the package import path in each profile file name (or `manifest.json` maps profile to package), so Go ids are unambiguous at kubernetes scale.
  - *Given* `profiles/` produced by the recipe for two packages each with `TestParse`, *When* parsed, *Then* two distinct `TestId`s result.
**Files**: `src/test_effectiveness/locate.rs`, `scripts/test-effectiveness/per-test-go-coverage.sh`

##### Task 2.1.4a: Locator with per-adapter id forms and tests (S)
##### Task 2.1.4b: Recipe change and `NoSource` exclusion plumbing (S)
- Files: as above

#### Story 2.1.5: Scale tests for the matrix, the parsers and the locator
**As an** owner, **I want** synthetic scale tests at the NFR size covering the algorithms, the input parsers and the `TestLocator`, **so that** the 10^5-test claim is measured end to end rather than assumed (engineering triage: the algorithm test alone left parse memory and locator cost unmeasured).
**Acceptance Criteria**:
- Unique coverage, greedy cover, and `joint_safe_subset` complete on a synthetic matrix of 100,000 tests and 1,000,000 units with 5 x 10^7 non-zero entries within the proposed bound (<= 1.5 GB resident, <= 120 s on the dev machine; the owner may revise the bounds at G0).
  - *Given* the generated fixture (seeded), *When* `cargo test --release scale -- --ignored` runs, *Then* it passes and prints peak RSS and wall-clock.
- If the bound fails, `roaring` is evaluated as a replacement behind the same API and ADR-005 is amended with the measurement.
- Parse memory, restricted to the four experiment-critical formats (coverage.py JSON, Go per-test profile directory, Stryker JSON, PIT XML): each adapter parses a seeded synthetic input of the size that yields 5 x 10^7 matrix entries within the same memory bound, or the story records that the adapter must stream and the DOM-style read is replaced. The Go adapter is exercised on a directory of per-test profile files plus `manifest.json` read one file at a time, so its peak RSS is bounded by the matrix, not by the directory total. lcov is deliberately not here: `lcov.rs` is ship-gated Story 2.2.3, which owns its own parse-scale test (Task 2.2.3b), so this experiment-critical story never waits on G2.
  - *Given* generated inputs for the four formats at that size, *When* `cargo test --release parse_scale -- --ignored` runs, *Then* it prints peak RSS and wall-clock per adapter and fails any adapter over the bound; the built-in ceiling recorded for `--max-input-bytes` (Story 2.2.7) is the smallest of the four per-format largest-passing inputs, so one value is safe for all four.
- `TestLocator` scale: resolving 10^5 test ids against a repo tree with 10^5 test functions across at least 10^4 files completes within a stated bound (proposal: same RSS bound as the matrix test; wall-clock bound set by the owner at G0), reading each source file at most once.
  - *Given* a generated tree and a generated id list, *When* `cargo test --release locator_scale -- --ignored` runs, *Then* it prints peak RSS, wall-clock and the count of file reads, and the count of file reads is at most the number of files.
**Files**: `src/test_effectiveness/matrix.rs`, `src/test_effectiveness/scale_tests.rs`

##### Task 2.1.5a: Fixture generator and ignored scale test (S)
- Files: as above

##### Task 2.1.5b: Parse-memory test for the four experiment-critical formats (coveragepy, gocover, stryker, pit), streaming decision and built-in ceiling (S)
- Files: `src/test_effectiveness/scale_tests.rs`, `src/test_effectiveness/adapters/`

##### Task 2.1.5c: `TestLocator` scale test over a generated 10^5-test tree (S)
- Files: `src/test_effectiveness/scale_tests.rs`, `src/test_effectiveness/locate.rs`

### Epic 2.2: Adapters
**Goal**: Tier 1 (experiment-critical) and Tier 2 inputs per `research/stack.md` section 7.

#### Story 2.2.1: coverage.py contexts JSON adapter
**Acceptance Criteria**:
- Parses `coverage json --show-contexts` into per-test covered units.
  - *Given* the verified 3-test fixture (`test_pos`, `test_neg`, `test_noassert`), *When* parsed, *Then* `test_noassert` covers the same lines as `test_pos` on the shared function and `test_neg` has unique line 3.
- Missing coverage.py version is rejected.
  - *Given* a JSON file whose `meta` lacks `version`, *When* parsed, *Then* the adapter returns `Err(MissingProvenance)` naming `coverage.py`.
- Import-time and fixture lines do not inflate a test's unique coverage (coverage.py attributes module-level lines to the empty context and fixture lines to whichever test first triggers them).
  - *Given* a fixture with module-level `import big_module` and a shared `@pytest.fixture` called first by `test_a` and then by `test_b`, *When* parsed, *Then* lines in the empty context are treated as covered by all tests (not unique to anyone), fixture-body lines are not credited as unique coverage of the first test, and the output carries the warning "fixture/import attribution: unique coverage excludes N setup lines".
- Attribution defaults to `InOrder` (assumed) per "Attribution source"; `--attribution isolated` overrides it, and a sidecar (Story 2.2.9 schema, hand-written fixture in tests) carrying `mode` supplies it without the assumed-default warning.
- Guard call: the adapter calls the shared guard of Task 2.2.7a before parsing; the call is written here, not retrofitted by Story 2.2.7.
- Schema/format drift (pre-mortem #5): the adapter holds an allow-list of coverage.py `meta.version` ranges it was fixtured against; a version outside the list parses with a class W warning "unverified coverage.py version X; confidence capped at low for this dimension" and a plausibility check warns when the fraction of tests with identical coverage, or of tests with zero unique coverage, is outside the range recorded with the fixture.
  - *Given* a JSON whose `meta.version` is not in the allow-list, *When* parsed, *Then* the report completes with exit 0, the warning is printed above the table, and every coverage-derived finding is `Low`.
**Files**: `src/test_effectiveness/adapters/coveragepy.rs`, `testdata/test-effectiveness/coveragepy/` (each fixture directory records the producer version; includes a hand-written `*.run.json` sidecar)

##### Task 2.2.1a: Adapter + fixture (S)
##### Task 2.2.1b: Error cases (XS)

#### Story 2.2.2: Go per-test cover-profile directory adapter + collection recipe
**Acceptance Criteria**:
- Reads a directory `profiles/<TestName>.out` plus a manifest of attribution method.
  - *Given* `profiles/TestPos.out` and `profiles/TestNeg.out` from the verified Go fixture, *When* parsed, *Then* each test's covered statement blocks match the profile and `Attribution = Isolated`.
- Guard call: the adapter calls the shared guard of Task 2.2.7a before reading each profile file and `manifest.json`.
- A recipe script produces such a directory; the binary does not run it.
  - *Given* `scripts/test-effectiveness/per-test-go-coverage.sh ./...`, *When* run in a module, *Then* it writes one profile per `Test*` (file name includes the package import path, resolving the missing-package ambiguity; see Story 2.1.4) and a `manifest.json` with go version, commit, and `mode` (`isolated` or `in_order`), which is the attribution source for this adapter.
- Recipe output and errors (UX S8): on success the script prints three lines: `per-test-go-coverage.sh: <N> tests, mode=<mode>, <go version>, commit <sha>`, `wrote profiles/ (<N> files) and profiles/manifest.json`, and `next: kibitzer tests report --go-profiles profiles/`.
  - *Given* a run in a directory without `go.mod`, *When* the script runs, *Then* it exits non-zero with `error: no go.mod in <dir>; run from a module root`.
  - *Given* `go` missing from `PATH`, *When* the script runs, *Then* it exits non-zero with `error: go not found on PATH`.
  - *Given* a test that fails during an isolated run, *When* the script finishes, *Then* the manifest records it as `failing` and the summary line says `<k> failing, excluded`.
  - *Given* the script interrupted mid-run, *When* it exits, *Then* no final `manifest.json` is written, `manifest.partial.json` records the finished tests, and the message says `interrupted at <k>/<N>: wrote profiles/manifest.partial.json; re-run with --resume to continue, or delete profiles/ to start over`; the report refuses a directory without `manifest.json` (`error: profiles/ has no manifest.json (partial run?)`).
- Long-running and executes-code notices (UX-E11, G6): before any test runs the script prints that it executes repo test code with the caller's environment and credentials, and per-test progress with an `eta` after at least 5 tests (to stderr, one line per test on a TTY, one per 10% otherwise).
  - *Given* a run, *When* the first output lines are read, *Then* the notice precedes the first `[k/N]` progress line and is not removed by any quiet flag.
  - *Given* an interrupted run followed by `--resume`, *When* it completes, *Then* finished tests are not re-run (the profile file's modification time is unchanged) and the final `manifest.json` exists.
**Files**: `src/test_effectiveness/adapters/gocover.rs`, `scripts/test-effectiveness/per-test-go-coverage.sh`

##### Task 2.2.2a: Parser + fixture (S)
##### Task 2.2.2b: Recipe script (in-order and isolated modes) (S)
##### Task 2.2.2c: Executes-code notice, progress, `--resume` and partial manifest (S)

#### Story 2.2.3: lcov adapter (`TN:` or one tracefile per test)
**Acceptance Criteria**:
- Parses `TN:`/`SF:`/`DA:` sections.
  - *Given* a tracefile with `TN:test_a` and `DA:10,1`, *When* parsed, *Then* `test_a` covers `(file, 10)`.
- Empty `TN:` with a single tracefile yields `ExclusionReason::NoData` for per-test signals.
  - *Given* one tracefile whose sections all start `TN:` with no name, *When* parsed, *Then* every test is `NoData` and the report prints "per-test coverage not available from this input".
- Attribution: named `TN:` sections default to `InOrder` (assumed); one tracefile per test produced by the user's own loop is declared with `--attribution isolated`.
  - *Given* a tracefile with named `TN:` sections and no `--attribution`, *When* parsed, *Then* the header says `attribution: in-order (assumed)`.
- Hardening (split from Story 2.2.7, which is experiment-critical and so cannot own a ship-gated adapter): the adapter calls the shared guard of Task 2.2.7a before parsing and ships truncated, empty, invalid-UTF-8, duplicated-section and out-of-range fixtures under `testdata/test-effectiveness/malformed/lcov/`.
  - *Given* an lcov file over the input cap or any malformed fixture, *When* parsed, *Then* the result is the R26 class E error or `Ok` with warnings, never a panic.
- Parse-scale (moved here from Story 2.1.5, which is experiment-critical and cannot own a ship-gated adapter): the adapter parses a seeded synthetic tracefile of the size that yields 5 x 10^7 entries within the Story 2.1.5 memory bound, or is rewritten to stream.
  - *Given* a generated lcov file at that size and the built-in ceiling of Task 2.1.5b, *When* `cargo test --release lcov_parse_scale -- --ignored` runs, *Then* it prints peak RSS and wall-clock and fails over the bound; an input over the ceiling is refused with R26 rather than parsed.
**Files**: `src/test_effectiveness/adapters/lcov.rs`, `src/test_effectiveness/scale_tests.rs`, `testdata/test-effectiveness/malformed/lcov/`

##### Task 2.2.3a: Adapter, guard call and malformed fixtures, plus empirical check of which producers write `TN:` (S)
##### Task 2.2.3b: lcov parse-scale test and streaming decision (S)

#### Story 2.2.4: mutation-testing-elements (Stryker) JSON adapter
**Acceptance Criteria**:
- Reads `killedBy`, `coveredBy`, `testFiles`.
  - *Given* a mutant with `killedBy: ["t1"]`, *When* parsed, *Then* `t1` has `UniqueKills` containing that mutant if no other test is listed.
- Warns when every killed mutant lists exactly one killer (bail-on-first).
  - *Given* a report where all 40 killed mutants have a single `killedBy`, *When* parsed, *Then* the kill dimension is tagged `KillCompleteness::FirstKillerOnly`, the output states "kill sets truncated; unique kills over-credit tests", and `ZeroUniqueKills` is downgraded to `Low`.
- Schema drift (pre-mortem #5): an allow-list of `schemaVersion` values it was fixtured against; an unlisted version parses with a class W warning and caps the kill dimension at `Low`; a plausibility check (share of killed mutants with exactly one killer outside the fixture-recorded range) warns.
  - *Given* a report with `schemaVersion` not on the list, *When* parsed, *Then* exit 0, a warning above the table, and every kill-derived finding is `Low`.
- Guard call: the adapter calls the shared guard of Task 2.2.7a before parsing (written here, not retrofitted).
- A recipe sidecar (`<report>.run.json`, schema frozen by Story 2.2.9) or `--kill-completeness` marks a sampled or budget-stopped run, so a partial run cannot look complete. This AC is tested with a hand-written sidecar fixture; no wrapper script is needed.
  - *Given* a sidecar with `budget_exhausted: true`, *When* parsed, *Then* the kill dimension is `BudgetLimited`, R9 prints with the mutant counts, `header.inputs[].mutation_run.scope` is `budget_limited` and every kill-derived finding is `Low`.
**Files**: `src/test_effectiveness/adapters/mutation_elements.rs`, `testdata/test-effectiveness/stryker/` (fixture directory records the producer version)

##### Task 2.2.4a: Schema structs + fixture from a real Stryker run (S)
##### Task 2.2.4b: Truncation detection (S)

#### Story 2.2.5: PIT `mutations.xml` adapter
**Acceptance Criteria**:
- Reads `killingTest`, `killingTests`, `succeedingTests`, `coveringTests`; `coveringTests` is stored as `CoverageUnitKind::MutantSite` (coverage of the mutated site, not of lines) and never mixed with line coverage in a comparison.
  - *Given* a mutation element with `<killingTests>A|B</killingTests>`, *When* parsed, *Then* both A and B kill it.
  - *Given* a PIT input, *When* a Q3 comparison is built, *Then* the report states the coverage unit kind used.
- Rejects or warns when `fullMutationMatrix` was not enabled (only `killingTest` present).
  - *Given* a report whose killed mutations all have only `<killingTest>` and empty `<killingTests>`, *When* parsed, *Then* the output warns "full mutation matrix not enabled" and `ZeroUniqueKills` is capped at `Low`.
- Format drift (pre-mortem #5): an allow-list of PIT versions the fixture covers (version read from the report or `--pit-version`); an unlisted version warns and caps the dimension at `Low`; fixtures exist per available major version.
  - *Given* a PIT report from a version not on the list, *When* parsed, *Then* exit 0 with a warning and `Low` kill-derived findings.
- Guard call: the adapter calls the shared guard of Task 2.2.7a before parsing; PIT entity and DOCTYPE rules are Task 2.2.7b.
- A recipe sidecar (`<report>.run.json`, schema frozen by Story 2.2.9) or `--kill-completeness` marks a sampled or budget-stopped run (same rule, hand-written fixture and test as Story 2.2.4).
- PIT `coveringTests` is a superset of the killers by construction (a test that kills a mutant necessarily covers its site), and both come from one file. Any Z_cov-versus-Z_kill comparison on PIT (H3, Story 3.3.2) is therefore biased toward containment and weakly informative; the result text says so (Task 3.3.2b).
**Files**: `src/test_effectiveness/adapters/pit_xml.rs`, `Cargo.toml` (`quick-xml`), `Cargo.lock`, `testdata/test-effectiveness/pit/` (fixture directory records the producer version)

##### Task 2.2.5a: Dependency + adapter (S)
##### Task 2.2.5b: Fixture from a real PIT run, error cases (S)

#### Story 2.2.6: JUnit XML side-data adapter
**Acceptance Criteria**:
- Reads per-test status and time for exclusions and runtime-cost estimates.
  - *Given* `<testcase name="TestSlow" time="3.2"><skipped/></testcase>`, *When* parsed, *Then* `TestSlow` gets `ExclusionReason::Skipped` and cost 3.2 s.
- Hardening (split from Story 2.2.7): the adapter calls the shared guard of Task 2.2.7a, reads XML without entity expansion or external fetch (same rules as Story 2.2.7's PIT AC), and ships malformed fixtures under `testdata/test-effectiveness/malformed/junit/`.
  - *Given* a JUnit file with a DOCTYPE declaring an external entity, *When* parsed, *Then* no file outside the input is read.
**Files**: `src/test_effectiveness/adapters/junit.rs`, `testdata/test-effectiveness/malformed/junit/`

##### Task 2.2.6a: Adapter, guard call, entity-safe XML and fixtures (S)

#### Story 2.2.7: Ingest hardening for the experiment-critical adapters and the locator
**As an** owner, **I want** every experiment-critical adapter to treat its input file as untrusted, **so that** a malformed, hostile or merely enormous coverage or mutation file cannot exhaust memory, hang the run, read local files or crash it (ADR-005 makes parser choice a security surface: `serde_json` and `quick-xml` are the only parsers).
**Scope split (iteration 2, D3)**: this story is experiment-critical, so it covers `coveragepy.rs`, `gocover.rs`, `mutation_elements.rs`, `pit_xml.rs` and `locate.rs` only. The ship-gated `lcov.rs` and `junit.rs` call the same guard and ship their own fixtures in Stories 2.2.3 and 2.2.6, and path and `git`-argument containment in `staleness.rs` (created by Task 2.3.5a) is Task 2.3.5c. The guard helper (Task 2.2.7a) is `guard.rs` only and lands before any adapter merges; each adapter's story (2.2.1, 2.2.2, 2.2.4, 2.2.5) writes its own call to it, so nothing is retrofitted into files that do not exist yet. Tasks 2.2.7b-d add tests and rules over an adapter and follow the adapter story that creates it.
**Acceptance Criteria**:
- A size cap applies before parsing, with a class E message. The cap value is the built-in ceiling recorded by Task 2.1.5b (Pre-registered parameters row 5), which G0 Part B may only tighten.
  - *Given* an input over `--max-input-bytes`, *When* any covered adapter is invoked, *Then* it exits 2 with the R26 message naming the file, its size and the cap, and reads no more than the file's metadata (asserted by running against an unreadable file of the oversize length: the metadata call succeeds and no `open` for reading occurs).
  - *Given* the built-in ceiling is not yet recorded, *When* the hidden `Command::Tests` variant is about to merge, *Then* Task 2.3.1a stops and reports the blocker; the cap tests are therefore never skipped on master.
- Nesting depth is bounded.
  - *Given* a JSON input nested deeper than `serde_json`'s default recursion limit, *When* parsed, *Then* the adapter returns a class E "not valid JSON (nesting too deep)" error and does not overflow the stack; the recursion limit is never disabled (a grep check on `disable_recursion_limit` is part of the test).
- PIT XML is parsed without entity expansion or any external fetch.
  - *Given* a `mutations.xml` with a DOCTYPE declaring an external entity that points at a local file path, *When* parsed, *Then* no file outside the input is read (asserted with a path that does not exist and one that does), the DOCTYPE is rejected or ignored, and the outcome is a class E error or a parse with the entity left unexpanded.
  - *Given* a "billion laughs" internal-entity fixture, *When* parsed, *Then* peak memory stays within the bound of Story 2.1.5 or the adapter returns a class E error.
- Malformed inputs never panic.
  - *Given* a corpus of truncated, empty, wrong-encoding (invalid UTF-8, BOM), duplicated-key, wrong-type and out-of-range-number fixtures for each covered format, *When* each adapter parses each, *Then* the result is `Ok` with warnings or a class E error and never a panic or a non-2 non-0 exit (exit 1 is reserved for an internal or output IO failure, `design/ux.md` section 3.4, never for an input condition).
  - *Given* a seeded mutate-the-fixtures loop (ADR-005 iteration 2 amendment: std-only, no `cargo fuzz`) of a documented case count per adapter (number set by the owner; the plan fixes none), *When* it completes, *Then* there is no panic, hang over a 10 s per-case limit, or allocation over the memory bound.
- Test ids, paths and names read from input are data: they are never interpolated into a shell command or a path join without a repo-root containment check (the `git`-argument half is Task 2.3.5c).
  - *Given* an input test id of `../../../etc/passwd::T`, *When* the locator runs, *Then* the id resolves to `NoSource` and no path outside the repo root is opened.
**Files**: `src/test_effectiveness/adapters/guard.rs`, `src/test_effectiveness/adapters/pit_xml.rs` (Task 2.2.7b only), `src/test_effectiveness/locate.rs`, `testdata/test-effectiveness/malformed/`

##### Task 2.2.7a: Shared guard helper (size cap from metadata, depth policy) (S)
- Files: `src/test_effectiveness/adapters/guard.rs` (the four adapters call it from their own stories)

##### Task 2.2.7b: PIT XML entity and DOCTYPE rules, billion-laughs bound (S)
- Files: `src/test_effectiveness/adapters/pit_xml.rs`

##### Task 2.2.7c: Locator path containment and `NoSource` for hostile ids (XS)
- Files: `src/test_effectiveness/locate.rs`

##### Task 2.2.7d: Malformed-input fixture corpus and seeded mutate-the-fixtures harness (S)
- Files: `testdata/test-effectiveness/malformed/`, `src/test_effectiveness/adapters/guard.rs` (test module)

#### Story 2.2.8: Recipe contract for non-Go producers (notice, progress, partial, resume, budget, sidecar)
**As a** repo owner, **I want** every producer recipe to behave like the Go one, **so that** the long, costly mutation and coverage runs warn before executing repo code, show progress, survive interruption and say when they stopped early.
**Acceptance Criteria**:
- Each recipe wrapper (`scripts/test-effectiveness/per-test-python-coverage.sh`, `scripts/test-effectiveness/mutation-run.sh <pit|stryker|mutmut|cargo-mutants|gremlins>`) follows the six rules of `design/ux.md` section 8.1.
  - *Given* any wrapper, *When* run, *Then* the executes-code (or mutates-a-throwaway-copy) notice precedes the first progress line and is not removed by a quiet flag.
  - *Given* a mutation wrapper interrupted at 37 of 212 units, *When* it exits, *Then* a `*.partial` marker exists, the final output file does not, and the message says whether `--resume` is supported for that tool.
  - *Given* `--budget-seconds N` exhausted, *When* the wrapper stops, *Then* it writes the partial report and `<output>.run.json` with `budget_exhausted: true`, `mutants_run`, `mutants_total` and prints `stopped at budget: <run> of <total> mutants run (<pct>%)`.
- The wrappers write the sidecar whose schema, reader and `--kill-completeness` flag are frozen in the experiment-critical Story 2.2.9; the adapters (Stories 2.2.1, 2.2.2, 2.2.4, 2.2.5) read it as input-carried provenance, tested with hand-written fixtures, so this ship-gated story owns only the wrapper scripts. lcov recipes are documented loops that write the same sidecar.
  - *Given* a coverage.py recipe run, *When* its sidecar is read, *Then* it records tool, version, commit, mode and `tests_total`, and the report's header shows `attribution` from it without an assumed-default warning.
**Files**: `scripts/test-effectiveness/per-test-python-coverage.sh`, `scripts/test-effectiveness/mutation-run.sh`, `docs/test-effectiveness-report.md` (recipe sections)

##### Task 2.2.8a: Shared notice, progress, partial-marker and sidecar helper, plus the coverage.py wrapper (S)
- Files: `scripts/test-effectiveness/recipe-lib.sh`, `scripts/test-effectiveness/per-test-python-coverage.sh`

##### Task 2.2.8b: Mutation wrapper with budget, resume where supported, and sidecar (S)
- Files: `scripts/test-effectiveness/mutation-run.sh`

#### Story 2.2.9: Run-sidecar schema and `--kill-completeness` flag (experiment-critical)
**As an** experimenter, **I want** the `<report>.run.json` sidecar schema and the `--kill-completeness` flag frozen before the adapters that read them, **so that** the experiment-critical adapters (2.2.1, 2.2.2, 2.2.4, 2.2.5) are tested with hand-written sidecars and never wait on the ship-gated wrapper scripts of Story 2.2.8 (round-3 engineering E-B2).
**Acceptance Criteria**:
- One reader parses the sidecar of `design/ux.md` section 8.1 rule 6 (`tool`, `tool_version`, `commit`, `mode`, `attribution`, `tests_total`, `mutants_total`, `mutants_run`, `sampled_fraction`, `budget_seconds`, `budget_exhausted`, `partial`) with a schema version field; unknown keys are ignored, a missing required key is a class W warning and the dimension caps at `Low`.
  - *Given* a hand-written sidecar with `budget_exhausted: true`, `mutants_run: 412`, `mutants_total: 1930`, *When* read next to a Stryker or PIT report, *Then* `kill_completeness` is `budget_limited` and `header.inputs[].mutation_run` copies the values.
- `--kill-completeness full|first-killer|sampled|budget-limited` is parsed and applied when no sidecar says; a sidecar and a flag that disagree resolve to the less complete value with a class W warning.
  - *Given* a sidecar `full` and `--kill-completeness sampled`, *When* run, *Then* the dimension is `sampled` and a warning names both.
- The sidecar JSON schema is committed under `testdata/test-effectiveness/sidecar/` with valid and invalid examples, and is the single reference the Story 2.2.8 wrappers are written against.
**Files**: `src/test_effectiveness/adapters/sidecar.rs`, `src/test_effectiveness/cli.rs` (flag parse), `testdata/test-effectiveness/sidecar/`

##### Task 2.2.9a: Sidecar schema, reader and fixtures (S)
- Files: `src/test_effectiveness/adapters/sidecar.rs`, `testdata/test-effectiveness/sidecar/`

##### Task 2.2.9b: `--kill-completeness` flag and precedence with the sidecar (XS)
- Files: `src/test_effectiveness/cli.rs`

##### Task 2.2.9c: Partition check script (XS; experiment-critical)
- `scripts/test-effectiveness/check-partition.sh` copies the tree to a scratch directory, deletes every ship-gated module and script (`adapters/lcov.rs`, `adapters/junit.rs`, `staleness.rs`, `enrich.rs`, the Story 2.2.8 wrappers; their `mod` lines carry a `// ship-gated` marker so the script can drop them), then runs `cargo build` and the experiment-critical test selection. It proves the experiment never waits on lcov, junit, staleness or the wrappers.
- Files: `scripts/test-effectiveness/check-partition.sh`

### Epic 2.3: CLI, output, suppression
**Goal**: `kibitzer tests report` with evidence-carrying findings.

#### Story 2.3.1: `kibitzer tests report`
**As a** repo owner, **I want** a ranked, evidence-carrying report, **so that** I can decide what to cut or merge.
**Acceptance Criteria**:
- Text table (default) and `--format json`; `--top N`; summary lines for flagged by rule, excluded, suppressed, and the joint coverage check. There is no runtime total in any format (Principle 7).
  - *Given* `kibitzer tests report --coveragepy ctx.json --format json`, *When* run, *Then* the top level is the `ReportEnvelope` of `design/ux.md` section 4 in this key order: `schema_version`, `format`, `mode`, `banner`, `advisory`, `non_claims`, `header`, `warnings`, `summary`, `findings`, `not_assessed`, `suppressed`; and each record in `findings` has `rule` (a string from the Rule-id registry), `section` (`main` or `experimental`; the only field that separates main-list rows from `--experimental` rows), `signal_mode` (`experimental` = no G2 verdict recorded for the signal, else `descriptive`, `report-only` or `tiered`; never used to tell main from experimental rows), `test` (`id` as `<repo-relative path>::<qualified name>` so `(rule, test.id)` is unique across packages, `file`, `line`), `value` (an object with a `kind` discriminant: `unique_coverage`, `unique_kills` or `static`, per `design/ux.md` section 4), `baseline`, `attribution`, `tool`, `tool_version`, `commit`, `confidence` (omitted in descriptive mode), `cost_seconds`, `overlaps_with` and `overlaps_also_flagged` (both lists of full `test.id` strings, so a cross-package partner is unambiguous; the text renderer prints bare names and adds the path only when two partners in one row share a name), `leave_one_out` (null unless `--leave-one-out` supplied a result for this test: `{"result": "no_loss"|"units_lost"|"not_run", "units_lost": N, "unit_kind"}`). There is no per-record `not_assessed` flag: unassessed tests appear only in the top-level `not_assessed` array with their reason.
- Envelope contract is frozen before the schema is declared stable (UX-R14, B1): `ReportEnvelope` fields exist and are populated from the same data the text renderer prints, so text and JSON cannot diverge, and Task 2.3.1e, which sets `schema_version` 1 only when it merges, lands before Tasks 2.3.1b and 2.3.1d render from the envelope.
  - *Given* a run that triggers R10 and R14, *When* `--format json` runs, *Then* `warnings[]` has two entries each with a stable string `id` (`stale-input-commit`, `attribution-in-order`; never the design-table row number), `class`, a structured `lowers` (`signal` and `effect` enums plus counts) and `params` and `message`, `header.inputs[0]` has `tool`, `tool_version`, `commit`, `attribution` and `attribution_assumed`, and `summary` has `flagged_by_rule`, `joint_check`, `suppressed` and `stability`.
  - *Given* a run with zero findings, *When* `--format json` runs, *Then* the envelope fields are all present and `findings` is `[]`.
  - *Given* `--format brief`, *When* run, *Then* the document has `"format": "brief"` and the same envelope as `--format json` (identical `mode`, `banner`, `advisory`, `non_claims`, `header`, `warnings`, `summary`), and each record additionally carries `record_non_claims` (named so it cannot be confused with the envelope's top-level `non_claims`), `verify_first` and `"action": "review"`.
  - *Given* the CLI flag `--kill-completeness first-killer`, `budget-limited` or `--attribution in-order`, *When* the JSON is read, *Then* the values are the snake_case enums `first_killer_only`, `budget_limited`, `in_order` (flags are hyphenated, JSON is snake_case; text output echoes the flag spelling), one mapping table in `design/ux.md` section 4.
  - *Given* mode `report-only` or `descriptive`, *When* `--format json` runs, *Then* `summary` names the count `listed` (not `individually_flagged`), omits `redundant_only_with_flagged`, and every record omits `confidence`.
  - *Given* a class E condition with `--format json` or `brief`, *When* run, *Then* stdout is exactly `{"schema_version", "format", "error": {"id", "message", "hint"}}`, exit status is 2 and the `error:` line is on stderr; with `--format text` stdout is empty.
- JSON contract (UX S3): valid and complete with zero findings, deterministic, versioned.
  - *Given* an input where nothing is flagged, *When* run with `--format json`, *Then* stdout parses as JSON with `"findings": []` and a `not_assessed` array.
  - *Given* the same input run twice, *When* the two JSON outputs are diffed, *Then* key order and finding order are identical.
  - *Given* any `--format json` output, *When* read, *Then* `schema_version` is present as the first key.
  - *Given* the JSON contract, *When* the Story 2.3.4 brief and a CI consumer fixture parse it, *Then* neither needs stderr to learn the mode, a warning, the provenance or the suppressed count.
- The report computes static signals for located tests (consistency review C4, amends ADR-002 decision 3): for each test the `TestLocator` resolves, it runs `test_functions` and `assertion_calls` from `src/test_code/mod.rs` with the shared `AssertionVocabulary` and emits `test-no-assertion` / `test-mock-only` rows as the wireframe shows; tests with no located source get no static row. The experiment does not depend on this report path: Stories 3.2.1 and 3.2.6 take the `NoAssertion` flagged set from the Phase 1 checker via `kibitzer check native test-no-assertion` over the arm's test files (the harness's own static source), and a parity test asserts that the report's static rows equal that output for located tests.
  - *Given* a located test with no assertion call and a coveragepy input, *When* run, *Then* a `[test-no-assertion]` row appears with value `static` and no attribution caveat, and the same record is present in `--format json`.
- Ranking key (one definition; triad repair B3 removed runtime as a key because sorting costly tests first is a delete order): confidence tier (`High` first), then rule id ascending, then test id ascending; `cost` is printed per row for context and never affects order. The heading reads "Listed in order of confidence, then rule id, then test id. Order is not a removal priority; see \"Before acting\"." In `descriptive` and `report-only` mode rows are in test-id order and the heading says so.
  - *Given* two flagged tests of equal tier and rule with runtimes 0.20s and 0.01s, *When* ranked, *Then* the order follows test id, not runtime.
  - *Given* a high-confidence `test-no-assertion` row, a medium `test-zero-unique-coverage` row and a medium `test-mock-only` row, *When* ranked, *Then* the order is no-assertion, mock-only, zero-unique-coverage (the wireframe order).
  - *Given* any text, json or brief output, *When* searched for "estimated runtime", "savings", "safe to remove" or a sum of `cost`, *Then* none is found (UX-R5, R17).
- Flags: `--attribution isolated|in-order|leave-one-out` and `--input-commit <sha>` (Stories 2.1.1, 2.3.5), `--junit <report.xml>` (Story 2.2.6, side data, not counted as a primary input for the R1 check), `--max-input-bytes` (Story 2.2.7), `--kill-completeness full|first-killer|sampled|budget-limited` (Story 2.2.8, when no sidecar says), `--show-suppressed` (Story 2.3.2), and `--experimental` (shows the signals outside the main-list rule of `design/ux.md` section 3.1 in their own section with its own count line, each row tagged `(experimental)`, absent from the main list and counts, UX-E12; before G2 the rule is the structural `Low` caps of "Confidence caps", after G2 it is the verdict, Story 4.2.1).
  - *Given* only `--junit report.xml`, *When* run, *Then* exit 2 with the R1 message (no per-test coverage or kill input).
- Verbose diagnostics (UX S7): `--verbose` writes one `[tests]`-prefixed line per fact to stderr only (adapter and input path, record counts, rejected records with reason, exclusions by reason, suppressed count, elapsed) and expands "Not assessed" with test ids and reasons; without it stderr carries only class E/W messages.
  - *Given* `--verbose` and a run with 2 tests whose inputs disagree, *When* run, *Then* stderr contains `[tests] excluded: inputs_disagree=2` and stdout is byte-identical to the run without `--verbose` except for the expanded "Not assessed" listing.
- Output states "not assessed" count with reasons and the non-endorsement sentence (UX-R4).
  - *Given* 212 tests of which 9 are stale, 2 disagree across inputs, 2 failing, 1 skipped and 1 no-data, *When* run, *Then* the `scope:` line reads `assessed 197 of 212 tests in the input (15 not assessed)`, the output includes "Not assessed (15): stale 9 | inputs disagree 2 | failing 2 | skipped 1 | no-data 1" (nonzero reasons in the fixed order; `Not assessed (0)` when the total is 0) and the sentence "Absence of a flag does not mean a test is valuable."
  - *Given* a repo with 41 test functions absent from a partial-run input, *When* run, *Then* a `note:` line states the count and `header.scope.repo_tests_not_in_input` is 41, and neither is part of `assessed` or `not_assessed`.
- Exit-code contract (UX-R6, R12): exit 0 whenever a report is produced, including when tests are flagged and when warnings (class W) are printed; exit 1 only for an internal error or an IO failure writing the output (stderr `error: internal: ...`; never for an input condition, so a malformed input is always 0 or 2); exit 2 only for input errors (class E: no input flag, missing or unreadable file, wrong shape, missing tool or version, no test contexts, zero tests, bad `--format`, bad `--top`, input over the size cap). Class E prints no table: with `--format text` it goes to stderr and stdout is empty; with `--format json` or `brief` stdout holds the minimal error object of `design/ux.md` section 4 (UX-E13) and stderr carries one `error:` line. Class W and I messages are part of the report: stdout body above the table in text, `warnings[]` in json and brief, with a one-line `warning:` copy on stderr for json and brief only (UX-R10, E2).
  - *Given* flagged tests, *When* run, *Then* exit status is 0.
  - *Given* stdout closed before the report is written, *When* run, *Then* exit status is 1 and stderr starts `error: internal:`.
  - *Given* `kibitzer tests report` with no input flag, *When* run, *Then* exit status is 2, stderr starts `error: no input given. Supply at least one of --coveragepy, --go-profiles, --lcov, --stryker, --pit.`, and stdout is empty.
  - *Given* `--format json` and an input that triggers a warning, *When* run, *Then* stdout parses as JSON, the warning is present in `warnings[]`, a one-line copy is on stderr, and exit status is 0.
  - *Given* `--format text` and the same input, *When* run with stdout redirected to a file, *Then* the file contains the `warning:` line above the table and stderr is empty.
- Output controls (UX 3.1, 3.2, 3.2c): `--top N` default 20 (`--top 0` means all; text only, json and brief are complete). The window is stratified by rule so a large static class cannot crowd out the dynamic findings: each of the R rules with findings first gets `min(count, floor(N / R))` rows, the remaining slots follow global ranking order, and rows print in global order; the omitted line names the per-rule counts (UX-R8, R19). `--by-package` prints a roll-up table and at most `--top` rows per package (default 5, placeholder), and the flat `--top 20` default does not apply in that view (UX-R20). The large-suite auto-switch (R22) is gated on a pre-registered threshold (see "Pre-registered parameters"); until recorded the switch is off.
  - *Given* 31 flagged tests and no `--top`, *When* run, *Then* 20 rows plus "... 11 more: <rule> <n> | ..." with per-rule counts summing to 11 (UX-R8).
  - *Given* 40 `test-no-assertion` rows sorting before 4 `test-zero-unique-coverage` rows and `--top 20`, *When* run, *Then* the window contains at least 4 coverage rows (`floor(20 / 2) = 10`, capped at the 4 that exist) and 16 static rows.
- "Before acting" block is printed at the end of text output with four steps: open the overlapping tests; if an overlap partner is itself flagged (marked `*`) keep at least one of the group, because "alone" is per test; a redundant test may be a regression pin, keep it with `kibitzer:ignore <rule> -- <why>`; re-run after any change. It states the split: `kibitzer:ignore` means correct but intentional, `report_false_positive` means the tool is wrong (UX-D5). A Legend (alone, conf, cost, `*`, inputs disagree, stale) follows it (UX-R18).
  - *Given* any text output with at least one row, *When* read, *Then* the block is present, contains both one-sentence statements and the keep-one-of-a-flagged-group step, and the Legend defines `alone`, `cost`, `*`, `inputs disagree` and `stale`.
- Rows are `path:line: [rule] TestName` in the repo-wide convention, with `alone`, `conf`, `cost` and `overlaps` on indented lines; overlap partners that are themselves flagged carry `*` (UX-R15, R16).
  - *Given* a flagged `TestParseEmpty` at `pkg/parse_test.go:118` whose partner `TestParseTable` is also flagged, *When* run, *Then* the row's first line is `pkg/parse_test.go:118: [test-zero-unique-coverage] TestParseEmpty`, the overlaps line is `overlaps: TestParse, TestParseTable*`, and json has `overlaps_also_flagged: ["TestParseTable"]`.
- Header and warnings (UX-R3, E2, R21): header always shows tool, tool version, commit, and attribution method and the `scope:` line `assessed N of M tests in the input (K not assessed)`, where K sums every reason including stale and no-source; every class W warning starts with `warning:`, is printed in the stdout body above the table, and states which signal's confidence it lowered.
- The report prints a joint coverage check per rule class, never one number over a mix (round-3 UX-B2): static rules (`test-no-assertion`, `test-mock-only`) are vacuity claims about a test body and are excluded from it; the coverage-redundancy class (`test-zero-unique-coverage`) and the kill-redundancy class (`test-zero-unique-kills`) are checked separately, and a class with several `unit_kind` values (`line`, `mutant_site`, `mutant`) gets one entry per kind. Each entry is computed over that class's `IndividuallyFlagged` set of the main list (never the zero-by-construction `JointSafe` subset), is worded as a measurement on the selection data ("N lines are covered by these tests and by no test outside the set"), names the set size and how many members had data, and is not the effect of deleting them or a sum of per-test values (UX-R7). `summary.joint_check` is an array of `{class, rules, unit_kind, set, set_size, with_data, units_held_only_by_set, basis, zero_note}`; it is `[]` when no dynamic rule flagged anything. No "% redundant" or pass/fail verdict (UX-R5). `JointSafe` is checked by the harness invariant I0 and unit tests, not in report output.
  - *Given* tests A and B that both cover only unit 1 and are the only tests covering it, both flagged `test-zero-unique-coverage`, *When* run, *Then* each row's `alone` is 0 unique lines and the coverage-redundancy entry reads `1 lines are covered by these tests and by no test outside the set`.
  - *Given* a flagged set none of whose members is redundant with another, *When* run, *Then* the entry reports the true (possibly nonzero) value and no line says the set is safe.
  - *Given* a flagged coverage-redundancy set whose measured joint value is 0 (every member is covered by some non-flagged test), *When* run, *Then* the line prints `0 lines are covered by these tests and by no test outside the set` immediately followed by the attached non-claim `This is a measurement on the data the set was chosen from; it does not show removal is safe (regression pins, untested units and failure messages are not seen).`, and json carries the same sentence in `zero_note`; the zero is never printed without it.
  - *Given* a run with 9 `test-no-assertion` findings and 14 `test-zero-unique-coverage` findings, *When* run, *Then* `joint_check` has one entry over the 14 coverage findings and none over the 9 static findings, and the number is not a mix.
  - *Given* a flagged test, *When* `redundant_only_with_flagged` is computed, *Then* it counts the individually flagged tests whose `overlaps_with` is non-empty and every member of it appears in `overlaps_also_flagged` (a test with at least one non-flagged partner is not counted).
- Mode banner and mode matrix (UX-R13, triad repair B2, iteration 2 G11): the first output line is the banner of the mode, one of the four texts in `design/ux.md` section 3.2a; `report-only` and `tiered` banners name the validation that backs them (an anchor in `docs/test-effectiveness.md` that must exist); the elements each mode shows are the matrix in `design/ux.md` section 3.2a, implemented by one render path keyed on `ReportMode` (Task 4.2.1d completes the descriptive branch).
  - *Given* main-section rows whose signals have verdicts `tiered` and `descriptive`, *When* run, *Then* top-level `mode` is `descriptive` (least validated present; order `experimental` < `descriptive` < `report-only` < `tiered`) and the render path follows that mode for every row.
  - *Given* mode `descriptive`, *When* the text report runs, *Then* there is no `conf` column, no ordering claim, no `JointSafe` count and no "redundant only with" column, and json omits `confidence`.
- Slow-read notice (UX-E9): when stderr is a TTY and an input is being read, a `[tests] reading <file> (<size>)` line is written to stderr; stdout is unchanged.
  - *Given* the same input with and without a TTY stderr, *When* stdout is diffed, *Then* it is byte-identical.
- Wrap rule for non-row lines (UX-X6): the first-line banner is one unwrapped line (so `head -1` returns it whole); every other non-row line (warnings, summary, scope, notes) is soft-wrapped at 100 columns on a word boundary with a 2-space continuation indent, and a path, rule id, sha or anchor is never split.
  - *Given* a 135-column warning, *When* the text report runs, *Then* no non-banner line exceeds 100 columns unless it holds one unsplittable token, and the `warning:` prefix starts only the first line.
- Malformed directive feedback (UX-D9): a `kibitzer:ignore` directive above a flagged test that lacks a reason does not suppress; the row gains the line `note: kibitzer:ignore above this test has no reason and was not applied (use -- <why>)`, the summary counts `malformed directives: N`, and json has `summary.suppressed.malformed_directives` and a per-record `ignore_note`.
  - *Given* a flagged test under `// kibitzer:ignore test-zero-unique-coverage` with no reason, *When* the report runs, *Then* the row is listed with the note line and the summary count is 1.
- Leave-one-out display (UX-R24, Story 3.1.4): with `--leave-one-out <loo.json>` a shortlisted test's row gains `leave-one-out: <n> units lost (measured)` or `leave-one-out: no loss (measured)`, json fills `leave_one_out`, and `header.inputs[].leave_one_out` records the file, `matched` and `unmatched` counts; a `loo.json` that is not valid JSON or not the expected shape is class E R3 or R4 naming the flag; unmatched ids are listed only under `--verbose`.
  - *Given* a `loo.json` with a result for a flagged test, *When* run, *Then* the row shows the line and json has the field; *Given* a malformed `loo.json`, *Then* exit 2 with R3 naming `--leave-one-out`.
- Determinism and terminal hygiene (UX-R9, R11, X1-X5): identical input gives byte-identical output; ASCII only; v1 emits no ANSI color at all; path, line and test name are printed in full in text and json (a long name may wrap; nothing is truncated with `...`).
  - *Given* the same input run twice, *When* outputs are diffed, *Then* they are identical.
- The subcommand ships hidden and labeled experimental until the G2 decision is recorded (Story 4.2.1c), so `descriptive-only` and `remove` both stay available at G2.
  - *Given* the merged binary, *When* `kibitzer --help` runs, *Then* `tests` is not listed (`#[command(hide = true)]` on `Command::Tests`), and *When* `kibitzer tests report ...` runs, *Then* the first output line is "experimental: signals not yet validated (see docs/test-effectiveness.md)".
- Output reports the flagged set per `FlaggedSet` definition in `experimental` and `tiered` mode: the summary gives the individually-flagged count and "of these, N are redundant only with another flagged test (marked `*`)", and rows whose partner is flagged carry the `*` mark and a "redundant only with: <test>" note; no count is presented as a number of tests that can go (Principle 7) and the `JointSafe` size is not printed. `descriptive` mode prints neither the count nor the note.
  - *Given* tests A and B that duplicate each other, *When* run in `experimental` mode, *Then* B is listed as "redundant only with A" (the harness's `joint_safe_subset` keeps A), the summary count is 1, and the summary does not add their individual values or any runtime.
**Files**: `src/test_effectiveness/cli.rs`, `src/test_effectiveness/report.rs`, `src/main.rs` (`Command::Tests`, hidden)

**Closure partition (iteration 2, D1/D2).** The story is not one wave. *Experiment-critical tasks* (the harness reads the report's JSON): 2.3.1a and 2.3.1e, which close in the experiment-critical wave and require Task 2.1.5b's built-in input ceiling before the hidden variant merges. Task 2.3.1a also carries the minimal class E path: the `{schema_version, format, error}` object, exit 2 and the R26 message with id `input-over-size-cap`, so Story 2.2.7's cap tests exercise a real message in the experiment-critical wave; Task 2.3.1c adds the other class E messages without changing R26's id or shape. *Remainder tasks* (close in the remainder wave, no gate): 2.3.1b, 2.3.1c, 2.3.1d, 2.3.1f. *Gated ACs*, each closed with its owning story and not counted against this story's closure: `--junit` and cost (Story 2.2.6); R10 and R23-R25 messages, `Stale` counts and the stale scope numbers (Story 2.3.5); `--experimental` post-G2 rule, the `report-only`/`tiered` banners and `descriptive` branch (Story 4.2.1); `--leave-one-out` (Story 3.1.4); `--by-package`, R22 and the auto-switch (ship-gated, Task 2.3.1g); `--kill-completeness` and the sidecar reader (Story 2.2.9, experiment-critical; the wrapper scripts are Story 2.2.8); `--show-suppressed` (Story 2.3.2). Until a gated AC's owner lands, the corresponding flag is accepted and ignored with a `warning:` naming the story, so the schema is complete from the first merge.

##### Task 2.3.1a: Command wiring + `TestFinding` + `findings[]` JSON + minimal class E path with R26 (M; experiment-critical)
- Files: `src/test_effectiveness/cli.rs`, `src/test_effectiveness/report.rs`, `src/main.rs`
##### Task 2.3.1b: Text renderer and summary (S)
- Files: `src/test_effectiveness/report.rs`
##### Task 2.3.1d: Static rows for located tests, `--verbose`, ranking key and stratified `--top` window (M)
- Deterministic ordering; static `test-no-assertion`/`test-mock-only` rows via `src/test_code/` (one shared parse); `[tests]` stderr lines; the ranking key above (no runtime); stratified window; tests for UX S3, S7, R19. Files: `src/test_effectiveness/report.rs`, `src/test_effectiveness/cli.rs`

##### Task 2.3.1e: `ReportEnvelope`, `ReportMode` and the mode matrix (M; experiment-critical; precedes the schema freeze)
- Build the envelope (`format`, `mode`, `banner`, `advisory`, `non_claims`, `header`, `warnings[]`, `summary`) once and render text and json from it; `schema_version` is set to 1 only when this task merges; legend and Before-acting text from one source; stream contract for warnings; per-class joint check (including the zero-case non-claim and `unit_kind`), `section`/`signal_mode` fields and the least-validated mode ordering, enum spelling map, report-only summary keys; tests for UX-R13, R14, R17, R18, R22, R23, E2. Files: `src/test_effectiveness/report.rs`, `src/test_effectiveness/cli.rs`

##### Task 2.3.1c: Class E messages and the exit-code contract, with the JSON error object (S)
- Implement the 9 remaining class E messages R1-R6 and R18-R20 from `design/ux.md` section 3.4 (R26 landed in Task 2.3.1a; first line names the flag or file, second line the fix) with their stable ids, wire exit 2, the exit-1 internal-failure path (UX-E16), the `--format json`/`brief` error object (UX-E13), and one test per message (UX-R12, E1, E8). Files: `src/test_effectiveness/cli.rs`, `src/test_effectiveness/report.rs`

##### Task 2.3.1f: Class W/I messages from input quality, with stable ids and structured `lowers` (S)
- Implement the 11 messages R7-R9, R11-R17 and R21 (input and attribution quality, partial data, nothing flagged), each with its `id`, `lowers` enums and `params`, and one test per message. R10 and R23-R25 are Task 2.3.5a/b; R22 is Task 2.3.1g. Files: `src/test_effectiveness/report.rs`

##### Task 2.3.1g: `--by-package` roll-up, per-package `--top`, R22 message and auto-switch (S; ship-gated)
- Roll-up table and `packages[]`; the auto-switch stays off until the G0 threshold is recorded (UX-R20). Files: `src/test_effectiveness/report.rs`

#### Story 2.3.2: `kibitzer:ignore` and false-positive integration
**Acceptance Criteria**:
- A finding anchored at the test function's first line is suppressed by a directive above it.
  - *Given* `// kibitzer:ignore test-zero-unique-coverage -- pins issue #123` above `func TestParseEmpty`, *When* the report runs, *Then* `TestParseEmpty` is not listed and the summary shows "1 suppressed".
- Rule ids appear as `[rule-id]` so `report_false_positive` keys apply.
  - *Given* a text finding, *When* printed, *Then* its first token after `file:line:` is `[test-zero-unique-coverage]`.
- The report's rule ids are registered in `RULES` so the directive is not itself reported by `kibitzer run`.
  - *Given* `// kibitzer:ignore test-zero-unique-coverage -- pins issue #123` above a test, *When* `kibitzer run` executes on that file, *Then* no `[unused-ignore]` or unknown-rule finding is emitted for it.
  - *Given* the drift-guard test named in the `RULES` doc comment, *When* `cargo test` runs, *Then* it passes with every report-only row of the "Rule-id registry" (`test-zero-unique-coverage`, `test-zero-unique-kills`, `test-pseudo-tested-region`) present as `OwnerMatch::Unowned`, and every static row resolvable through `known_rule()` by its registered checker name.
- Decorator/attribute placement is an explicit test case (UX D-E1): a directive above the decorators/attributes and a directive between the decorators and `def`/`func` are each exercised for Python (`@pytest.mark...`), Java/Kotlin (`@Test`), and Rust (`#[test]`). Each placement is recorded as suppressing or not; the finding text and `docs/test-effectiveness-report.md` state the placement that works.
  - *Given* a Python test `@pytest.mark.slow` over `def test_x()` with a directive on the line above the decorator, *When* the report runs, *Then* the outcome (suppressed, or not suppressed with the finding text saying "above the decorators"/"directly above the def") matches the documented placement.
- Dismissal semantics (UX-D1..D4, D6): one comment line plus one re-run dismisses a finding; a directive without a reason is reported by `kibitzer run` as `[ignore-syntax]` and does not suppress; the report summary shows "suppressed by kibitzer:ignore: N"; the report does not emit `[unused-ignore]` for dynamic rules, because a dynamic finding can vanish when the input differs without the directive being wrong.
  - *Given* a directive with no `-- <why>`, *When* the report runs, *Then* the target finding is still listed.
- A dismissed finding stays dismissed while the directive remains adjacent to the test. Inline ignores have no content guard (only `.kibitzer/accepted/` does, `docs/accepting-findings.md`), so UX-D2's "reappears only if the test's own source changes" is restated as "reappears only if the directive is moved, removed, or detached from the test by an edit".
  - *Given* a directive above `TestParseEmpty`, *When* unrelated tests are added or removed and the report re-runs on new input, *Then* `TestParseEmpty` is not re-listed unless its own data newly flags it under a different rule.
- Repeated-dismissal hint (UX D-E5): when five or more findings of the same rule were dismissed with the same reason string, the footer prints `5 findings dismissed with the same reason; consider an assertion_helpers/scope option` (N is the actual count), once.
  - *Given* five directives `kibitzer:ignore test-zero-unique-coverage -- pins regression` in the repo, *When* the report runs, *Then* the footer line appears once with the count; with four such directives it does not appear.
- Non-specific dismissal reasons (UX D-E6, D7; Goodhart mitigation): a well-formed directive whose reason matches the stoplist or minimum length recorded at G1 still suppresses, is counted in `summary.suppressed.non_specific_reason` and in the text summary, and triggers a one-line footer hint. Until the stoplist is recorded the counter and hint are off.
  - *Given* a stoplist containing `intentional` and a directive `-- intentional`, *When* the report runs, *Then* the finding is suppressed, the summary reads `dismissals with non-specific reasons: 1`, and json `summary.suppressed.non_specific_reason` is 1.
- `report_false_positive` acknowledgement text (UX-A5). The tool has no server: `report_false_positive` in `src/mcp.rs` appends a record to a local JSONL queue (`default_queue_path()`, `src/false_positive.rs`) and returns `queued false-positive report for `<check_name>` at <path>. Run `kibitzer check false-positives list` ...`; `check_name` is an unvalidated string (verified by Grep: it is read only to format the reply and the triage entry), so report rule ids work as `check_name` with no registry change. The change here is one added sentence on the success reply.
  - *Given* a successful call with `check_name` `test-zero-unique-coverage`, *When* the response text is read, *Then* it still begins with the existing `queued false-positive report for` text and also contains `does not silence the finding` and `kibitzer:ignore <rule> -- <why>`.
  - *Given* the existing `tests/false_positives_cli.rs` queue tests, *When* `cargo test` runs, *Then* they still pass (the queue format and `list` output are unchanged).
- False-positive tool failure (UX D-E4): the real failure is a queue write error, not an unreachable server. The existing text is `error queuing false-positive report: <io error>`; one sentence is appended telling the caller how to silence locally.
  - *Given* a queue path that cannot be written, *When* `report_false_positive` is called, *Then* the returned text starts with `error queuing false-positive report:` and ends with `The report was not recorded; add kibitzer:ignore <rule> -- <why> to silence locally.`
- Dismissal audit (UX-D8): `--show-suppressed` lists each suppressed finding as `path:line: [rule] TestName -- <reason as written>` (non-specific reasons marked) and json `suppressed[]` is always present with the same data.
  - *Given* two suppressed findings, one with reason `intentional` on the stoplist, *When* run with `--show-suppressed`, *Then* both are listed with their reasons and the second carries the non-specific mark; *When* run without the flag, *Then* text shows only the count and json still has `suppressed[]` with two entries.
**Files**: `src/test_effectiveness/report.rs`, `src/inline_ignores/rules.rs` (add `Signal` rule ids to `RULES` as `OwnerMatch::Unowned`, like `god-class`), `src/inline_ignores/` (scan consumer), `src/mcp.rs` (reply sentences)

**Decision (task 2.3.2a): unanchored findings are not findings; no `(rule, test-id)` extension of `.kibitzer/accepted/`.** A test whose id resolves to no unique source file and line (`NoSource`: JUnit-only ids, renamed or deleted tests, dynamically generated tests) is listed under "Not assessed: no-source" and never emitted as a finding, so there is nothing to dismiss and no dismissal path is built. Rationale, checked against `docs/accepting-findings.md` and `docs/suppressing-checks.md`:
1. The `.kibitzer/accepted/` key is `(rule, file, line, content)`, and `content` is its staleness guard (an entry silently stops applying when the line's text changes). A `(rule, test-id)` key has no line text to guard against, so extending the schema would add the only entry type that never goes stale, which is the opposite of the directory's stated purpose ("the original judgment call needs a fresh look").
2. Its Scope section limits it to findings with a specific line and excludes whole-repo checks because "this directory's `(rule, file, line, content)` key doesn't fit them". An unanchored test finding is the same shape of problem. Its stated role is a "fallback for locations that cannot hold a comment, such as generated files": those still have a file and line, which these do not.
3. Inline ignores and `accepted/` are both applied by the existing per-file pipeline (`src/check.rs`) keyed on a source file; the report has no such file for these tests.
4. The remedy is in the input, not in suppression: refresh or regenerate the input so ids match the tree (the R12 warning says this). An id that never maps (JUnit-only, dynamic generation) is out of the signal's reach by construction and says so as "not assessed", which the plan's "silence is not endorsement" stance already requires.
Accepted cost: an owner cannot explicitly mark such a test as "keep". Revisit trigger (no number invented): if Story 3.3.1 triage shows `NoSource` tests are a material share of an arm's would-be flags, reopen option (b) with a schema that carries a content-bearing key. Task 2.3.2a must verify points 1-3 against `src/inline_ignores/` and `src/accepted_findings.rs` before closing; if the code contradicts the docs, amend this decision in the Repair log.

##### Task 2.3.2a: Read `src/inline_ignores/` and `src/accepted_findings.rs`, confirm the decision above, apply scan to anchored source files, test (M)
##### Task 2.3.2b: Register report rule ids in `RULES`, satisfy drift guard, test no `[unused-ignore]` (S)
##### Task 2.3.2c: Decorator/attribute placement tests and finding-text wording (XS)
- Files: `src/test_effectiveness/report.rs`, `testdata/test-effectiveness/ignore-placement/`

##### Task 2.3.2d: Repeated-dismissal hint, non-specific-reason count, `--show-suppressed`, and the `report_false_positive` reply sentences (XS)
- Add the D-E5 footer hint, the D-E6 counter and hint, the `--show-suppressed` listing and `suppressed[]`, and the two appended sentences to the existing replies in `report_false_positive` (`src/mcp.rs`, the queue path in `src/false_positive.rs` unchanged). Add the report rule ids to the doc note that names the triage target (for report rules the queue entry is folded into `docs/test-quality-false-positives.md` by hand, because the drain format is keyed on `check_name`). Files: `src/test_effectiveness/report.rs`, `src/mcp.rs`, `tests/false_positives_cli.rs`

#### Story 2.3.3: Static enrichment via `ArchModel` (conditional on spike 0.3.3)
**Acceptance Criteria**:
- Gated: implemented only if spike 0.3.3 records a usable resolved rate.
  - *Given* spike result "resolved >= threshold agreed at G0", *When* the report runs with `--enrich`, *Then* each finding adds "calls into: pkg.Func (resolved)".
- If the spike says no-go, this story is closed as "not built" with the number cited.
  - *Given* spike result "resolved rate 31% on `_test.go` edges", *When* the story is reviewed, *Then* its status line reads "not built: resolved rate 31% (linkage-spike.md)".
**Files**: `src/test_effectiveness/enrich.rs`

##### Task 2.3.3a: `is_test` view derived from `file`, join (S)
##### Task 2.3.3b: Tests and doc line (S)

#### Story 2.3.4: Adjudication brief contract
**Acceptance Criteria**:
- `--format brief` emits per-candidate JSON for an external LLM or reviewer: test source location, evidence, overlapping tests, non-claims.
  - *Given* a flagged `TestParseEmpty`, *When* `--format brief` runs, *Then* the record includes `{"test": ..., "evidence": {...}, "record_non_claims": ["may pin a regression"], "verify_first": ["TestParse"]}`.
- `record_non_claims` and `verify_first` are mandatory and non-empty on every record, every record carries the constant `"action": "review"`, and no schema field, key, or value uses "delete" or "remove" wording (pre-mortem #10).
  - *Given* a flagged test with no overlapping test, *When* `--format brief` runs, *Then* `verify_first` is still non-empty (it names the test's own source location and the instruction to find a covering test), and `record_non_claims` contains at least "may pin a regression".
  - *Given* text, json, and brief output for any input, *When* scanned for imperative verbs `delete`, `remove`, `drop`, `prune`, *Then* none appears outside the explicit non-claim sentences quoted from the "Before acting" block.
- Docs state kibitzer makes no LLM call.
  - *Given* `docs/test-effectiveness.md`, *When* searched for "LLM", *Then* the adjudication section contains the sentence "kibitzer does not call an LLM; the brief is input for one".
**Files**: `src/test_effectiveness/report.rs`, `docs/test-effectiveness.md`

##### Task 2.3.4a: Brief format (S)
##### Task 2.3.4b: Usage section in docs (XS)

#### Story 2.3.5: Input staleness (ADR-001)
**As a** repo owner, **I want** findings marked stale when production code changed since the input was produced, **so that** an old coverage run is not read as current evidence.
**Acceptance Criteria**:
- Each dimension's `Provenance` carries the input commit (from the input or `--input-commit`); if absent (a warning, not an error: Story 2.1.1), the report prints "input commit unknown" and every finding is `Low` tier.
  - *Given* a coverage input with commit `abc123` and a `HEAD` where a test's own file, or a file defining its covered units, differs from `abc123`, *When* the report runs, *Then* that test is excluded as `ExclusionReason::Stale` (not flagged, counted under "Not assessed: stale") and the warning "report was produced at abc123, HEAD is <sha>; N test files changed since. Their tests are excluded as stale." appears above the table.
  - *Given* no production file in the unit set changed, *When* run, *Then* no stale mark appears.
- Staleness is computed by `git diff --name-only <input-commit> HEAD` (and `git status --porcelain` for the working tree) intersected with the files of the test's covered units; no repo code is executed.
  - *Given* a non-git directory, *When* run, *Then* the report prints R25 (`warning: not a git repository; staleness not assessed. ...`) above the table and exits 0 rather than failing.
- Commit ids are validated before any `git` invocation: `--input-commit` and any commit read from an input must match `^[0-9a-fA-F]{7,64}$` (a hex object id, never a ref name or option) and are passed after `--end-of-options` or a `--` separator (Task 2.3.5d).
  - *Given* `--input-commit "--output=/tmp/x"` or `--input-commit "HEAD~1; rm -rf"`, *When* run, *Then* no `git` process is started with it, exit status is 0 and the R11 warning names the value as `not a hex object id`; a commit read from the input with the same shape is treated as unknown (R11).
- Precedence when staleness messages combine (UX-E10): each stale file is counted in exactly one message: R23 if its working copy differs from HEAD, otherwise R10 (input commit is an ancestor and the file differs between it and HEAD) or R24 (not an ancestor). R10 and R24 are mutually exclusive; R23 may accompany either. A test is `Stale` once, whatever the number of messages naming its files.
  - *Given* a file changed in a commit since the input commit and also uncommitted, *When* run, *Then* R23 counts it and R10 does not.
- This story owns the R10, R23, R24 and R25 messages (with stable ids `stale-input-commit`, `dirty-working-tree`, `input-commit-not-ancestor`, `not-a-git-repo`), the `Stale` counts in the scope line and `Not assessed` breakdown, and the `git`-argument and path containment of Task 2.3.5c.
- Comment-only exemption (iteration 2, G5): a file the name-only diff lists counts as changed only if its token stream differs once comments and whitespace are removed (`git show <sha>:<path>` against the working copy, comments dropped through the file's tree-sitter grammar; when no grammar is available or the old content cannot be read the file counts as changed). Adding, editing or removing a `kibitzer:ignore` line must never make the test and its file-mates stale, because that would take every dismissed test out of assessment.
  - *Given* an input at commit `abc123` and a working tree whose only change to `a_test.go` is a new `// kibitzer:ignore test-zero-unique-coverage -- pins issue #123` line above `TestA`, *When* the report runs, *Then* `a_test.go`'s tests are not `Stale`, R10 and R23 do not name the file, and `TestA` is suppressed.
  - *Given* a change to `a_test.go` that adds a statement, *When* the report runs, *Then* its tests are `Stale`.
- Dirty working tree (R23, UX-E10): uncommitted changes to a test file or a covered production file are treated as changed since the input commit.
  - *Given* an input at commit `abc123` equal to HEAD and an uncommitted edit to `a_test.go`, *When* run, *Then* tests in `a_test.go` are `Stale`, R23 names the file count, and exit status is 0.
  - *Given* a clean tree at the input commit, *When* run, *Then* none of R10, R23, R24, R25 appears.
- Non-ancestor commit (R24, UX-E10): when the input commit is not an ancestor of HEAD, staleness is computed from the tree difference and may over-exclude, which the warning says.
  - *Given* an input commit on another branch, *When* run, *Then* R24 is printed above the table, the files that differ between the two trees are treated as changed, and exit status is 0.
**Files**: `src/test_effectiveness/staleness.rs`, `src/test_effectiveness/report.rs`

##### Task 2.3.5a: Commit capture, diff intersection, rendering, tests (S)
##### Task 2.3.5b: Dirty-tree and non-ancestor cases (R23, R24) with temp-repo tests (XS)
- Files: `src/test_effectiveness/staleness.rs`

##### Task 2.3.5d: Hex-object-id validation of `--input-commit` and input-carried commits, precedence of R10/R23/R24 (XS)
- Files: `src/test_effectiveness/staleness.rs`

##### Task 2.3.5c: Comment-only exemption and `git`/path containment (S)
- Token-stream compare with comments dropped (shared parse via `test_code`), the comment-only and `kibitzer:ignore`-only tests above, and containment: no `git` argument starting with `-` (a `--` separator before every path), no path join outside the repo root, moved here from Story 2.2.7 because `staleness.rs` is created by Task 2.3.5a.
  - *Given* an input test id of `-rf::T`, *When* the staleness code runs, *Then* no `git` argument starting with `-` is passed and the id resolves to `NoSource`.
- Files: `src/test_effectiveness/staleness.rs`

---

# Phase 3: Experiment (research core)

All Phase 3 stories require `decisions/G0-decision.md` Part B (Story 0.4.1).

**Scope statement**: this is an exploratory validation study on a small, convenience-selected set of repositories (named at G0; at most three to four dynamic suites per language), not a population-level estimate. Results are reported as per-arm effect sizes with intervals; no claim generalises beyond the named arms, and Go/Python/Rust per-test kill agreement is not claimed at all (Q3).

Hypotheses (pre-registered in 3.1.1 with numbers fixed at G0; the defaults below are proposals):
**Circularity guard (pre-mortem #2).** `JointSafe` is by definition the subset whose joint loss of coverage units (and of kills, where a kill dimension exists) is empty, so the coverage delta on the data the set was selected from is zero by construction, and so is the N4 null. A zero there carries no information about fault detection. Only a measure that `JointSafe` does not preserve may be called validation. Every results row therefore states `selection_data` (what the flagged set was chosen on) and `evaluation_data` (what the delta was measured on), and `label` (`same-family` or `real`); a row whose evaluation data equals its selection data is a harness check, never a hypothesis outcome.

- **I0 (harness invariant, not a hypothesis)**: on selection data the `JointSafe` set has exactly zero coverage-unit loss, and exactly zero loss of killed mutants on kill-matrix arms. A nonzero value is a harness or joint-loss bug and fails the run. Reporting "I0 holds" is never described as validity evidence.
- **H1** "the `JointSafe` set of a signal can be deleted with equivalent-to-zero change on a held-out measure it does not preserve" (the anecdote, made falsifiable). The held-out measure is, per arm: (i) mutation-score delta over the identical sampled mutants where the set was selected on coverage only (mutation kills are not preserved by a coverage-selected set); (ii) on kill-matrix arms, the delta over family-B mutants and module-set-B when selection used family A and module-set A (P2); (iii) P1 real-history detection loss only where the power gate passes. Pass: two-sided equivalence, 90% CI of the held-out delta within +/-2.0 percentage points for mutation score (the proposed line-coverage margin of +/-1.0 applies to the secondary, I0-checked coverage figure only). An arm with coverage delta only (no mutation or proxy run) is reported "no validity evidence; H1 and H2 not evaluated", never as an H1 pass. Failure of the margin is a stated negative result, not a retry trigger.
- **H2** "the `JointSafe` set loses less on the held-out proxy than size-matched nulls N2 and N4", evaluated on the held-out proxy only (P2 same-family, P1 if powered), never on selection data. Pass: rank p-value (1 + #{null loss <= flagged loss}) / (K + 1) <= 0.05 against each of N2 and N4 separately, with K >= 999 where nulls are computed from a matrix and K >= K_mut (>= 19) where each null needs a mutation rerun; N1 is reported for context and passing N1 alone is not a pass. If a "5th percentile" is quoted it is derived from the rank p-value, not read off 30 seeds. Because N4 is coverage-preserving like the flagged set, p-values near 0.5 are an expected and reportable "not beaten N4" outcome; it is a stated negative result and selects a row of the Story 0.4.1 outcome table.
- **H3** "zero-unique-coverage tests are a sufficient stand-in for zero-unique-kill tests" — evaluated only on arms with a per-test kill matrix. Pass: precision of Z_cov with respect to Z_kill (share of zero-unique-coverage tests that also have zero unique kills; equivalently the containment of Z_cov in Z_kill) with Wilson 95% lower bound >= 0.90, and recall reported alongside. The two sets are not nested in either direction in general (a test with zero unique coverage can still uniquely kill a mutant on a line others cover but do not assert on), so a symmetric Jaccard is not the decision statistic.
- **H4** "static vacuity rules have precision >= the owner floor". Floor proposed: 0.80 lower Wilson bound over >= 50 triaged findings per rule per language (floor fixed at G0, not after triage).

### Epic 3.1: Protocol and harness
**Goal**: A pre-registered, reproducible deletion experiment per ADR-003.

#### Story 3.1.1: Protocol document and pre-registration
**Acceptance Criteria**:
- States invariant I0 and hypotheses H1-H4 with thresholds taken verbatim from `G0-decision.md` Part B, metrics, seeds per null family and arm, mutation budget per arm (0.3.4), sample sizes for triage, exclusion rules, and what outcome would falsify each.
  - *Given* H2, *When* the protocol is read, *Then* it states the rank p-value rule, K per null family, and which null families must be beaten, matching `G0-decision.md`.
  - *Given* H1, *When* the protocol is read, *Then* it names the held-out measure per arm class (coverage-selected: mutation delta on sampled mutants; matrix: family-B/module-B; replay if powered) and states that the selection-data coverage delta is I0, not H1.
- Every results-row schema field `selection_data`, `evaluation_data`, and `label` is defined, and a row with identical selection and evaluation data is classified `invariant_check`, never `hypothesis`.
  - *Given* a results row whose evaluation data is the coverage of the selection run, *When* classified, *Then* its `kind` is `invariant_check` and it is excluded from the H1-H4 outcome counts.
- Includes a power section: for the real-history proxy, events needed vs. events observed (0.3.1b); for triage precision, n per rule per language for a stated Wilson interval half-width.
  - *Given* the section, *When* read, *Then* it states that arms below the event threshold are labeled "exploratory" in every results table.
- The protocol is committed before any harness run; deviations go in a dated "Deviations" section, never an edit to a threshold.
  - *Given* a threshold change after the first run, *When* reviewed, *Then* it appears under "Deviations" with the reason and the original value retained.
**Files**: `project_plans/test-effectiveness-critique/experiment/protocol.md`

##### Task 3.1.1a: Write and self-review before any run (S)
##### Task 3.1.1b: Power and sample-size section (S)
##### Task 3.1.1c: Commit protocol, tag the pre-registration commit (XS)

#### Story 3.1.2: Deletion harness
**Acceptance Criteria**:
- Given a repo, a flagged list, and a mutant sample seed, the harness runs full suite vs suite-minus-flagged via skip lists (no source edits) and writes JSONL.
  - *Given* ripgrep and 20 flagged tests, *When* the harness runs, *Then* one JSONL record has `coverage_before/after`, and, when the arm's feasibility verdict (0.3.4) is a mutation verdict, `mutation_score_before/after` over the identical mutant sample; otherwise `mutation: "not run: coverage-delta arm"`.
- Flagged-set sources are fixed: dynamic flagged sets come from `kibitzer tests report --format json` `findings[]` (experiment-critical Tasks 2.3.1a and 2.3.1e) and `JointSafe` is computed by the harness from the same matrix; static flagged sets come from the Phase 1 checkers via `kibitzer check native` (its own source, so no arm waits on the report's remainder or gated tasks).
  - *Given* a Python arm, *When* the harness builds its static flagged set, *Then* it equals the `kibitzer check native test-no-assertion` output over the arm's test files, and the report's static rows for located tests are a subset of it.
- Same mutant sample is used before and after, and only mutants killed by the full suite are rerun (a subset suite cannot kill a survivor).
  - *Given* seed 7, *When* run twice, *Then* the sampled mutant ids are identical and the rerun set equals the sampled killed mutants.
- Null families N1-N4 (Glossary `RandomBaseline`) are generated size-matched with seeded RNG; each record stores the observed flagged loss and the null losses, and the rank p-value per family.
  - *Given* a record, *When* read, *Then* it has `null.N1..N4` with K values each (K from `G0-decision.md`), and `rank_p.N2`, `rank_p.N4`.
  - *Given* an arm with a kill matrix, *When* nulls are evaluated, *Then* they are computed from the matrix (no reruns) at K >= 999.
- Both `JointSafe` and `IndividuallyFlagged` deletions are run; the latter is labeled "secondary: joint-loss effect".
  - *Given* a record, *When* read, *Then* it has `set: "joint_safe"` and `set: "individually_flagged"` entries with their sizes.
- Execution safety (Story 0.3.5): the harness runs each arm inside the isolation mode recorded for it and refuses to start without one; each row records `isolation`, `network` and `env_allowlist_hash`.
  - *Given* an arm with no recorded isolation mode, *When* the harness starts, *Then* it exits non-zero with `isolation mode not recorded for arm <name>` before executing any repo code.
  - *Given* a harness run, *When* `printenv` is run inside the sandbox by the preflight, *Then* no variable outside the allow-list is present.
- Invariant I0 is checked on every run and fails the run when violated; selection and evaluation data are recorded per row.
  - *Given* a `joint_safe` set whose coverage-unit loss on selection data is nonzero, *When* the harness runs, *Then* it exits non-zero with `harness invariant I0 violated` and writes no hypothesis row.
  - *Given* a coverage-delta-only arm, *When* its record is written, *Then* it carries `evidence: "none"` and the text `no validity evidence; H1 and H2 not evaluated`, not an equivalence verdict.
  - *Given* any record, *When* read, *Then* it has `selection_data`, `evaluation_data`, `label` (`same-family` or `real`), `kind` (`invariant_check` or `hypothesis`), and `stability` (`assessed` or `single_run`).
**Files**: `scripts/test-effectiveness/deletion-experiment.py`, `project_plans/test-effectiveness-critique/experiment/results/`

##### Task 3.1.2a: Runner + skip-list adapters for go test, pytest, cargo test (M)
##### Task 3.1.2b: Random baseline and JSONL (S)
##### Task 3.1.2c: Stratified, runtime-matched, and coverage-preserving nulls plus rank p-values (S)
##### Task 3.1.2d: Sandbox wrapper, env allow-list and `isolation` row fields (S)

#### Story 3.1.3: Held-out proxy implementation
**Acceptance Criteria**:
- Implements the proxies fixed at G0, always including the P2 floor.
  - *Given* the chosen proxy "historical failure replay", *When* run on the Python arm, *Then* output records detection retained for the full suite vs suite-minus-flagged on the same commits, with the event count and Wilson interval.
  - *Given* P2 on any mutation-capable arm, *When* run, *Then* the flagged set is selected from mutants of operator-family group A and module set A, and evaluated on family group B and module set B (groups fixed in the protocol before any run), and the record labels the result "same-family".
- Where a second independent mutation tool exists for the language (P3), selection uses one tool's kills and evaluation the other's; otherwise this is recorded as "P3 unavailable".
  - *Given* Java with PIT only, *When* run, *Then* the record states "P3 unavailable: single tool".
**Files**: `scripts/test-effectiveness/heldout-proxy.py`

##### Task 3.1.3a: Implement chosen proxy (S)
##### Task 3.1.3b: Validate on a known-answer toy repo (S)
##### Task 3.1.3c: Operator-family / module split (P2) and P3 pairing (S)

#### Story 3.1.4: Leave-one-out attribution for shortlists
**As an** experimenter, **I want** exact leave-one-out unique coverage for a bounded shortlist, **so that** `CoverageAttribution::LeaveOneOut` has a producer and the isolated vs in-order disagreement can be resolved on the tests that matter (ADR-003 item 4).
**Acceptance Criteria**:
- A script runs the suite with one shortlisted test removed (skip list, no source edits) per shortlisted test and records the coverage units lost relative to the full suite; the shortlist is the `IndividuallyFlagged` zero-unique-coverage set plus a seeded sample of unflagged tests, capped at R_max from `G0-decision.md`.
  - *Given* a 20,000-test suite and R_max = min(200, 2% of suite) = 200, *When* the script runs, *Then* it performs at most 200 removals and records `shortlist_size` and the sampling seed.
  - *Given* tests A and B that duplicate each other, *When* run with the shortlist {A, B}, *Then* each removal shows zero units lost individually, and the jointly-removed run (A and B together) shows the shared units lost, matching `joint_loss({A,B})`.
- Output is a JSON file consumable by the harness and by `--attribution leave-one-out` as a per-test "units lost on removal" map with a `mode: leave_one_out` field; the report prints `attribution: leave-one-out (shortlist only; N of M tests)` for findings backed by it.
  - *Given* the output passed with `--coveragepy ctx.json --leave-one-out loo.json`, *When* the report runs, *Then* shortlisted findings print `leave-one-out` and non-shortlisted findings keep the input's attribution.
**Files**: `scripts/test-effectiveness/leave-one-out.py`, `project_plans/test-effectiveness-critique/experiment/results/`

##### Task 3.1.4a: Removal runner for go test, pytest, cargo test (M)
##### Task 3.1.4b: Output format, shortlist sampling, R_max bound test (S)

### Epic 3.2: Corpus arms
**Goal**: Run signals per language inside the isolation mode of Story 0.3.5 (no arm runs without a recorded mode); wall-clock bound by toolchains (see blockers). Each arm states its feasibility verdict from spike 0.3.4 and runs a second in-order collection so `Flaky` can be judged (otherwise the arm records `stability: single_run` and nothing is excluded for it). Arms run in the order fixed at G0 Part B (information per hour) and obey its stop-loss. Only the JVM and JS/TS arms have per-test kill matrices; Go, Python, and Rust arms have suite-level mutation score only (when the verdict allows) and so answer Q3 at suite level only.

#### Story 3.2.1: Python arm (coverage.py contexts; mutmut bounded-module suite-level mutation if 0.3.4 allows)
**Acceptance Criteria**:
- Per-test coverage, flagged sets for `ZeroUniqueCoverage` and `NoAssertion`, deletion deltas, nulls, proxy; module and tool as named in `G0-decision.md`.
  - *Given* the Python exemplar, *When* the arm completes, *Then* `results/python.jsonl` has one record per signal and set (`joint_safe`, `individually_flagged`) with `flagged_count`, `delta_*`, `null.N1..N4`, `rank_p.N2`, `rank_p.N4`, and `mutation_mode` (`bounded_module` or `not_run`).
- Fixture and import attribution warnings from the coverage.py adapter are counted.
**Files**: `project_plans/test-effectiveness-critique/experiment/results/python.jsonl`

##### Task 3.2.1a: Collect per-test coverage (in-order and isolated), run harness (M)
##### Task 3.2.1b: Record and sanity-check (S)
##### Task 3.2.1c: Repeat in-order run and flakiness classification (S)

#### Story 3.2.2: Go arm (stapler-squad named packages; kubernetes subpackages coverage-only unless 0.3.4 says otherwise)
**Acceptance Criteria**:
- Same as 3.2.1 with `go test -run` isolated vs in-order attribution disagreement counted; mutation via `gremlins` (or `go-mutesting`) on named packages only, per 0.3.4.
  - *Given* stapler-squad, *When* complete, *Then* the record states how many tests changed `UniqueCoverage` between `Isolated` and `InOrder`, and the mutation verdict used.
**Files**: `.../results/go.jsonl`

##### Task 3.2.2a: Run (M)
##### Task 3.2.2b: Record (S)
##### Task 3.2.2c: Repeat in-order run and flakiness classification (S)

#### Story 3.2.3: Rust arm (ripgrep named crates; coverage + bounded-crate cargo-mutants if 0.3.4 allows)
**Acceptance Criteria**:
- Whole-suite coverage and suite-level mutation delta; per-test attribution is `NoData` in this arm (decision, round-3 E-B3): the only Rust per-test coverage ingest path is the ship-gated lcov adapter (Story 2.2.3), so allowing `Isolated` would make the experiment depend on a ship-gated story (cycle G2 <- 3.2.3 <- 2.2.3 <- G2). The arm contributes the static flagged set (`test-no-assertion`, Story 3.2.6) and the suite-level H1 mutation delta; per-test Rust coverage signals are a post-G2 follow-up once lcov ships. Rationale for NoData-only over pulling a minimal lcov reader into the experiment-critical set: Q3 cannot be answered for Rust either way (no per-test kill matrix, Q3 scope above), the Q4 default hypothesis already treats Rust as coverage-only, and a minimal reader would still need the guard, malformed fixtures and the empirical `TN:` check, enlarging the critical set for a signal the experiment cannot validate.
  - *Given* ripgrep, *When* complete, *Then* the record states attribution `NoData` explicitly, the static flagged set and the mutation verdict.
**Files**: `.../results/rust.jsonl`

##### Task 3.2.3a: Install tools, run (M)
##### Task 3.2.3b: Record (S)
##### Task 3.2.3c: Repeat run and flakiness classification (S)

#### Story 3.2.4: JVM arm (PIT full matrix; the per-test Q3 ground truth, with the JS/TS arm)
**Acceptance Criteria**:
- Kill matrix with `fullMutationMatrix=true` on the module named at G0 (criteria in Story 0.2.2); both unique-coverage and unique-kills computed, with the coverage unit kind stated (PIT `coveringTests` is `MutantSite`; line coverage from JaCoCo is used for the line-coverage comparison if available, else the comparison is labeled mutant-site).
  - *Given* the sampled Java module, *When* PIT output is ingested, *Then* both signals are recorded for every test, with precision, recall, and containment of Z_cov in Z_kill and Wilson intervals.
**Files**: `.../results/jvm.jsonl`

##### Task 3.2.4a: Run PIT via Gradle on bounded module (M)
##### Task 3.2.4b: Ingest and record (S)

#### Story 3.2.5: JS/TS arm (Stryker; Q3 ground truth)
**Acceptance Criteria**:
- Same as 3.2.4 on the JS/TS exemplar named at G0 (Stryker with per-test coverage analysis and bail disabled so `killedBy` is complete), with truncation check from 2.2.4; if only `FirstKillerOnly` data is obtainable, the arm is excluded from the Q3 decision and reported as such.
  - *Given* the exemplar, *When* ingested, *Then* the record states whether `killedBy` was truncated and which `KillCompleteness` was used.
**Files**: `.../results/ts.jsonl`

##### Task 3.2.5a: Run Stryker via npx (M)
##### Task 3.2.5b: Ingest and record (S)

#### Story 3.2.6: Static-rules arm
**Acceptance Criteria**:
- Deleting tests flagged by `test-no-assertion` and `test-tautology` across all arms: coverage and mutation delta, and share that still contribute unique coverage.
  - *Given* flagged vacuous tests in the Python arm, *When* deleted, *Then* mutation-score delta and unique-coverage loss are recorded (a vacuous test may still contribute coverage).
**Files**: `.../results/static.jsonl`

##### Task 3.2.6a: Run per arm (M)
##### Task 3.2.6b: Record (S)

### Epic 3.3: Triage, analysis, write-up
**Goal**: Turn runs into decisions.

#### Story 3.3.1: Manual triage of dynamic findings
**Acceptance Criteria**:
- Stratified sample per signal per arm triaged for intent (regression pin, spec doc, distinct contract) via `scripts/backtest-triage.py`.
  - *Given* 40 `ZeroUniqueCoverage` flags in the Go arm, *When* triaged, *Then* a verdict and reason is stored for a stratified sample (size from protocol) in `docs/backtest-triage/<repo>/test-zero-unique-coverage.jsonl`.
- Verdict reliability (pre-mortem #9): intent verdicts are judgment calls, so the owner independently labels a seeded random 10% (minimum 30 per signal per arm, or all if fewer), blind to the agent verdict; agreement is reported; `ConfidenceTier` precision inputs use owner-confirmed labels or state the measured disagreement; triage volume does not exceed the G0 commitment.
  - *Given* the triage JSONL for an arm, *When* read, *Then* blind-labelled rows carry `owner_verdict` and the aggregate states the agreement rate and the n used for each Wilson bound.
**Files**: `docs/backtest-triage/**`

##### Task 3.3.1a: Sample and triage (M)
##### Task 3.3.1b: Aggregate precision tables (S)

#### Story 3.3.2: Q3 agreement analysis
**Acceptance Criteria**:
- Reports, per ground-truth arm (JVM, JS/TS only), precision, recall, and directional containment of Z_cov in Z_kill with Wilson 95% intervals, rank correlation of per-test unique-coverage and unique-kill counts, the coverage unit kind used, and the cost ratio. Jaccard may be shown for reference but is not the decision statistic.
  - *Given* arms 3.2.4 and 3.2.5, *When* analysed, *Then* a table gives precision, recall, and containment per suite with n and intervals.
- The conclusion is a decision rule applied to data, not a pre-written sentence: "supported for arm X" iff the Wilson lower bound on precision >= the H3 threshold in `G0-decision.md`; otherwise "not supported for arm X"; with two arms, "supported" requires both. Scope is stated as JVM and JS/TS only.
  - *Given* results where one arm clears the threshold and the other does not, *When* the sentence is generated, *Then* it reads "not supported (1 of 2 arms)" and names both.
- If fewer than two per-test kill-matrix suites were obtained (Story 0.2.2 fallbacks exhausted), the sentence is "Q3 not answered (n = k suites): kill-primary where a kill dimension is present; coverage-only ordering labeled unvalidated everywhere".
  - *Given* zero ground-truth arms, *When* the sentence is generated, *Then* it reads "Q3 not answered (n = 0 suites)" and the header text in Story 4.2.1 uses the "not supported" branch.
- The PIT bias is stated: where Z_cov is built from PIT `coveringTests` (unit kind `MutantSite`), the results text says "coveringTests is a superset of killingTests by construction; containment on this unit kind is biased upward and weakly informative"; the JS/TS arm is the unbiased comparison.
  - *Given* the JVM arm's results text, *When* searched for "superset", *Then* the sentence is present next to the table.
- Go, Python, and Rust are reported as "unvalidated default" with the suite-level H1 mutation delta where run; no sentence extrapolates the JVM/JS result to them.
  - *Given* the results section, *When* searched for "Go" and "Python", *Then* each mention of ordering carries "unvalidated default" and no sentence says the coverage signal suffices for them.
**Files**: `docs/test-effectiveness.md` (results section)

##### Task 3.3.2a: Analysis script (S)
##### Task 3.3.2b: Results text, including the PIT `coveringTests` superset bias statement (S)

#### Story 3.3.3: Results write-up and confidence tiers
**Acceptance Criteria**:
- Each hypothesis H1-H4 has a stated outcome with the numbers; tiers defined from measured precision. Invariant I0 is reported in a separate line ("harness invariant held on N arms") and never counted as validation.
  - *Given* arm results, *When* the results section is read, *Then* H1 reads e.g. "held-out mutation delta X (CI), P2 family-B delta Y, proxy loss Z vs N2 rank p, N4 rank p" with arm names, selection data and evaluation data named, not adjectives.
- Only held-out outcomes (H1 and H2 on P2/P1, never on selection data) may be worded "validated"; every other row is worded "invariant check", "no validity evidence", or "exploratory".
  - *Given* the results section, *When* searched for "validated", *Then* each hit is on a row with `kind: hypothesis`, `evaluation_data` different from `selection_data`, and `label` stated.
  - *Given* a coverage-delta-only arm, *When* its row is read, *Then* it says "no validity evidence", not "H1 passed".
- A `NegativeResultReport` section is always present: for each of H1-H4, per arm, "supported" or "not supported" with the numbers, the claims withdrawn (e.g. "the 25%-removable anecdote is not supported on arms X, Y"), and what ships per the Story 0.4.1 outcome table. A null result is an accepted terminal state; the section is not omitted when everything passes (it then states "no hypothesis was unsupported").
  - *Given* H2 not beaten for N4 on every arm, *When* the section is read, *Then* it names every arm, the rank p-values, the withdrawn claim, and the outcome-table row selected.
- Includes a limitations paragraph: Java-skewed literature, same-family proxies labeled, exploratory arms labeled, per-test Q3 answered on at most two suites.
- States corpus coverage explicitly: dynamic arms cover stapler-squad, named kubernetes subpackages, named ripgrep crates, and the G0-named Python, Java, and JS/TS exemplars; cassandra, servo, deno, and vscode receive static runs only (Success Metric 3 is met in part, as accepted by the owner in `G0-decision.md` Part B).
  - *Given* the results section, *When* read, *Then* a table lists each requirements-named corpus repo with "dynamic" or "static only".
**Files**: `docs/test-effectiveness.md`

##### Task 3.3.3a: Write (M)
##### Task 3.3.3b: Adversarial self-review of every claim (S)
##### Task 3.3.3c: `NegativeResultReport` section and outcome-table row selection (S)

#### Story 3.3.4: Optional LLM-adjudication pilot (owner-triggered)
**Acceptance Criteria**:
- On one arm, an LLM judge labels a random sample and the mechanical shortlist; compare precision and delta; recorded with cost.
  - *Given* the Python arm shortlist and an equal-size random sample, *When* both are judged and deleted, *Then* the record shows each set's delta and the judging cost in CU.
**Files**: `project_plans/test-effectiveness-critique/experiment/results/llm-pilot.jsonl`

##### Task 3.3.4a: Pilot (M)

### Epic 3.4: Owner gate G2

#### Story 3.4.1: Record the G2 decision
**As an** owner, **I want** a per-signal ship/keep-research decision on file, **so that** unhiding the report and wiring defaults are mechanically gated.
**Acceptance Criteria**:
- `project_plans/test-effectiveness-critique/decisions/G2-decision.md` records, per dynamic signal, one of: `ship-with-tiers`, `ship-report-only`, `descriptive-only` (the default for any signal whose H2 was not supported, or whose arms were all `no validity evidence`), or `remove` (accepted only when the owner writes the word `remove` for that signal); the Story 0.4.1 outcome-table row selected; first-class language list (Q4); Q3 outcome as worded by Story 3.3.2 and whether accepted.
  - *Given* a verdict of `descriptive-only` for every signal, *When* Story 4.2.1c runs, *Then* the `tests` subcommand is unhidden in descriptive mode (banner, no tiers, no ordering claim), and the hidden variant is NOT removed.
  - *Given* a verdict `remove` written by the owner for every signal, *When* Story 4.2.1c runs, *Then* the `Command::Tests` variant is removed, the docs keep the `NegativeResultReport`, and the Rollback procedure is the executed path.
- Stories 4.2.1 and 4.2.2 list this file as a precondition.
**Files**: `project_plans/test-effectiveness-critique/decisions/G2-decision.md`

##### Task 3.4.1a: Compile results table; owner records decision (XS)

---

# Phase 4: Gated promotion and docs

### Epic 4.1: Static promotion

#### Story 4.1.1: Promote accepted static rules to `default_checks()`
**Acceptance Criteria**:
- Precondition: `decisions/G1-decision.md` exists. Only rules it marks "promote" are chained; others stay opt-in.
  - *Given* G1 approves `test-no-assertion` for Go only, *When* `kibitzer run` executes on a Go repo with no inspect.json, *Then* it reports the rule; on a Python repo it does not.
- Quiet-path line preserved (UX-H1, H6, H8): promotion keeps exactly one quiet-path line in every hook and `run_checks` finding, in the variant `G1-decision.md` records (V1 copyable directive by default, V2 pointer if measurement (a) was high); this story must not drop it.
  - *Given* each promoted rule's finding in hook mode, *When* its text is read, *Then* it contains exactly one quiet-path line of the recorded variant and is at most 4 lines.
- Docs: `docs/test-quality.md`, `CLAUDE.md` "Default check catalog" list updated, FP doc linked.
  - *Given* the updated `CLAUDE.md`, *When* grepped for `test-no-assertion`, *Then* it appears in the default catalog paragraph.
**Files**: `src/config.rs`, `docs/test-quality.md`, `CLAUDE.md`

##### Task 4.1.1a: Chain per approved rule/language (S)
##### Task 4.1.1b: Docs and catalog update (S)
##### Task 4.1.1c: Regression run over corpus confirms flagged counts match triage (XS)

#### Story 4.1.2: Assertion-removal-in-diff check (conditional on 1.2.1 evidence)
**Acceptance Criteria**:
- Built only if transcript evidence found; otherwise closed with the count.
  - *Given* Story 1.2.1 recorded >= 1 confirmed occurrence, *When* an edit removes `assert x == 1` from an existing test, *Then* a `[test-assertion-removed]` finding appears at the test line.
**Files**: `src/checkers/test_assertion_removed.rs`

##### Task 4.1.2a: Diff-vs-HEAD comparison via existing baseline machinery (S)
##### Task 4.1.2b: Fixtures and backtest (S)

### Epic 4.2: Report defaults and documentation

#### Story 4.2.1: Wire tiers and defaults from measured data
**Acceptance Criteria**:
- Precondition: `decisions/G2-decision.md` exists.
- `ConfidenceTier` boundaries are the pre-registered parameter from "Pre-registered parameters": the derivation rule was fixed at G0, the values come from Stories 3.3.1/3.3.3 and are recorded in `G2-decision.md` before this story starts; this story invents no number. UX-E4 and E7 hold: `zero-unique-kills` from a PIT report without a full matrix never shows above `low`; integration/snapshot tests are `low` or Not assessed, never `medium`.
- `ConfidenceTier` thresholds are constants in code (`src/test_effectiveness/tiers.rs`) and `docs/test-effectiveness.md` cites the constant names; a test checks the doc table against the constants in that direction (the doc is checked, the code does not parse prose).
  - *Given* results table "ZeroUniqueKills with full matrix: precision P", *When* the unit test runs, *Then* the doc row for tier `High` names the constant `HIGH_MIN_PRECISION` and its value equals the constant.
- Default signal ordering follows the measured Q3 outcome per language, with unmeasured languages labeled.
  - *Given* Q3 recorded as "supported for JVM and JS/TS", *When* the report runs on a Java coverage-only input, *Then* the header states the coverage signal is primary and cites the arms.
  - *Given* the same Q3 outcome, *When* the report runs on a Go or Python coverage-only input, *Then* the header states "ordering is an unvalidated default for this language" and does not cite the JVM/JS result as support.
  - *Given* Q3 recorded as "not supported", *When* the report runs on any input with a kill dimension, *Then* kills are primary where present and coverage-only runs state that coverage-only ordering is unsupported.
- Banners (iteration 2, G11): the four mode banners are the texts in `design/ux.md` section 3.2a; the `report-only` and `tiered` banners are filled from `G2-decision.md` (arm count) and point at an anchor of `docs/test-effectiveness.md` that must exist, checked by a test.
  - *Given* G2 `ship-with-tiers` for `ZeroUniqueCoverage` on 2 arms, *When* the report runs, *Then* the first line is `tiered: confidence tiers come from triaged precision on 2 arms; backing: docs/test-effectiveness.md#results` and the anchor resolves.
- Unhide: the subcommand is unhidden and the "experimental" banner removed only for signals G2 marks `ship-with-tiers` or `ship-report-only`; from then on `--experimental` holds the signals G2 did not mark `ship-*`.
  - *Given* G2 "ship report-only" for `ZeroUniqueCoverage`, *When* `kibitzer --help` runs, *Then* `tests` is listed and unshipped signals remain behind `--experimental`.
- Descriptive-only mode (the null-result deliverable, Story 0.4.1): for any signal G2 marks `descriptive-only`, the report prints the banner "descriptive: signals not validated as removal predictors; see docs/test-effectiveness.md", shows unique-coverage and joint-loss counts and the exclusions, has no `ConfidenceTier` column, no ranking-order claim (rows sorted by test id), and no "safe" or `JointSafe` removal wording; JSON carries `"mode": "descriptive"` and `confidence` is omitted.
  - *Given* G2 `descriptive-only` for `ZeroUniqueCoverage`, *When* `kibitzer tests report --coveragepy ctx.json` runs, *Then* the first line is the descriptive banner, there is no Conf column, rows are in test-id order, and the Before-acting block still prints.
  - *Given* the same G2 and `--format json`, *When* run, *Then* each record lacks `confidence`, the top level has `"mode": "descriptive"`, the `banner` field holds the descriptive banner, `advisory` is true, `non_claims` is non-empty, and exit status is 0.
  - *Given* the same G2, *When* the text output is searched for `JointSafe`, "safe", "redundant only with" and a `conf` column header, *Then* none appears; the "joint coverage check" line is present with measurement wording (the mode matrix in `design/ux.md` section 3.2a is the acceptance reference).
**Files**: `src/test_effectiveness/report.rs`, `src/test_effectiveness/tiers.rs`, `src/main.rs`

##### Task 4.2.1a: Constants and doc-check test (S)
##### Task 4.2.1b: Ordering and header text (S)
##### Task 4.2.1c: Unhide per G2, or remove the hidden variant only on an owner-written `remove` (XS)
##### Task 4.2.1d: Descriptive-only rendering mode (S)

#### Story 4.2.2: User docs
**Acceptance Criteria**:
- `docs/test-effectiveness-report.md` documents inputs, recipes, outputs, exclusions, and the "not a delete list" stance; `docs/suppressing-checks.md` lists the rule names.
  - *Given* a new user, *When* following the recipe for Python, *Then* they can produce `ctx.json` and run the report without other documents.
- The doc lists the tool versions each adapter is fixtured against and the refresh procedure (pre-mortem #5); the producer-run fixtures are refreshed in this story.
  - *Given* `docs/test-effectiveness-report.md`, *When* read, *Then* it has a table of adapter, tested producer versions, and fixture path, and each fixture directory's recorded version matches its row.
**Files**: `docs/test-effectiveness-report.md`, `docs/suppressing-checks.md`, `README.md`

##### Task 4.2.2a: Report doc with recipes (S)
##### Task 4.2.2b: Cross-links and self-review (S)
##### Task 4.2.2c: Refresh producer-run fixtures and record versions (S)

---

## Effort estimate

Bands and weights INFERRED from `ESTIMATION.md`; recalibrate after the first implementer run. CU = raw x (1 + 4 x output share). Raw uses class midpoints XS 20k, S 55k, M 130k.

| Story | Tasks | Cost class | Raw tokens | Output share | Est. CU | Wall-clock blocker |
|---|---|---|---|---|---|---|
| 0.1.1 | 4 | S+S+S+XS | ~185k | 25% | ~370k | none |
| 0.2.1 | 4 | S+S+M+S | ~295k | 12% | ~437k | none |
| 0.2.2 | 2 | S+XS | ~75k | 10% | ~105k | repo clones, toolchain installs |
| 0.3.1 | 2 | M+S | ~185k | 8% | ~244k | historical-commit builds; power gate |
| 0.3.2 | 1 | M | ~130k | 8% | ~172k | per-test suite runs |
| 0.3.3 | 1 | S | ~55k | 8% | ~73k | none |
| 0.3.4 | 6 | S+S+S+M+M+S | ~480k | 8% | ~634k | mutation tool installs per language (5 parallel tasks); per-mutant timing runs |
| 0.3.5 | 2 | S+S | ~110k | 12% | ~163k | container or VM availability; secret-scan hook |
| 0.4.1 | 2 | XS+XS | ~40k | 10% | ~56k | owner decisions G0 Part A and Part B |
| 1.1.1 | 2 | M+S | ~185k | 15% | ~296k | none |
| 1.1.2 | 3 | M+S+XS | ~205k | 12% | ~303k | none |
| 1.1.3 | 2 | S+S | ~110k | 12% | ~163k | none |
| 1.1.4 | 2 | S+S | ~110k | 12% | ~163k | none |
| 1.1.5 | 2 | XS+XS | ~40k | 12% | ~59k | none |
| 1.2.1 | 3 | M+S+S | ~240k | 8% | ~317k | owner label review |
| 1.2.2 | 3 | M+S+S | ~240k | 8% | ~317k | corpus clone/run; owner triage review |
| 1.2.3 | 1 | XS | ~20k | 8% | ~26k | none |
| 1.3.1 | 1 | XS | ~20k | 10% | ~28k | owner decision G1 |
| 2.1.1 | 2 | S+S | ~110k | 12% | ~163k | none |
| 2.1.2 | 2 | M+S | ~185k | 12% | ~274k | none |
| 2.1.3 | 3 | S+S+XS | ~130k | 12% | ~192k | none |
| 2.1.4 | 2 | S+S | ~110k | 12% | ~163k | none |
| 2.1.5 | 3 | S+S+S | ~165k | 12% | ~244k | synthetic scale runs |
| 2.2.1 | 2 | S+XS | ~75k | 12% | ~111k | none |
| 2.2.2 | 3 | S+S+S | ~165k | 12% | ~244k | none |
| 2.2.3 | 2 | S+S | ~110k | 12% | ~163k | none |
| 2.2.4 | 2 | S+S | ~110k | 12% | ~163k | Stryker real-run fixture |
| 2.2.5 | 2 | S+S | ~110k | 12% | ~163k | PIT real-run fixture |
| 2.2.6 | 1 | S | ~55k | 12% | ~81k | none |
| 2.2.7 | 4 | S+S+XS+S | ~185k | 12% | ~274k | none |
| 2.2.8 | 2 | S+S | ~110k | 12% | ~163k | producer reruns for sidecar checks |
| 2.2.9 | 3 | S+XS+XS | ~95k | 12% | ~141k | none |
| 2.3.1 | 7 | M+S+S+M+M+S+S | ~610k | 12% | ~903k | none |
| 2.3.2 | 4 | M+S+XS+XS | ~225k | 12% | ~333k | none |
| 2.3.3 | 2 | S+S | ~110k | 12% | ~163k | conditional on 0.3.3 |
| 2.3.4 | 2 | S+XS | ~75k | 20% | ~135k | none |
| 2.3.5 | 4 | S+XS+S+XS | ~150k | 12% | ~222k | none |
| 3.1.1 | 3 | S+S+XS | ~130k | 25% | ~260k | owner approval (G0 record) |
| 3.1.2 | 4 | M+S+S+S | ~295k | 12% | ~437k | none |
| 3.1.3 | 3 | S+S+S | ~165k | 12% | ~244k | none |
| 3.1.4 | 2 | M+S | ~185k | 8% | ~244k | removal runs on shortlists, bounded by R_max |
| 3.2.1 | 3 | M+S+S | ~240k | 6% | ~298k | suite + bounded-module mutation run time |
| 3.2.2 | 3 | M+S+S | ~240k | 6% | ~298k | suite + per-test run time; gremlins on named packages |
| 3.2.3 | 3 | M+S+S | ~240k | 6% | ~298k | cargo-mutants install and bounded-crate run |
| 3.2.4 | 2 | M+S | ~185k | 6% | ~229k | PIT full-matrix run |
| 3.2.5 | 2 | M+S | ~185k | 6% | ~229k | Stryker run |
| 3.2.6 | 2 | M+S | ~185k | 6% | ~229k | reruns across arms |
| 3.3.1 | 2 | M+S | ~185k | 8% | ~244k | owner review of triage samples |
| 3.3.2 | 2 | S+S | ~110k | 10% | ~154k | none |
| 3.3.3 | 3 | M+S+S | ~240k | 25% | ~480k | owner decision G2 |
| 3.3.4 | 1 | M | ~130k | 10% | ~182k | owner trigger, LLM spend |
| 3.4.1 | 1 | XS | ~20k | 10% | ~28k | owner decision G2 |
| 4.1.1 | 3 | S+S+XS | ~130k | 15% | ~208k | owner decision G1 |
| 4.1.2 | 2 | S+S | ~110k | 12% | ~163k | conditional on 1.2.1 |
| 4.2.1 | 4 | S+S+XS+S | ~185k | 12% | ~274k | owner decision G2 record |
| 4.2.2 | 3 | S+S+S | ~165k | 25% | ~330k | producer re-runs for fixture refresh |

- **Totals** (recounted with grep after triad repair iteration 3: `grep -c "^### Epic"`, `"^#### Story"`, `"^##### Task"`, and the Raw and Est. CU columns summed over the 56 table rows by script): 16 epics, 56 stories, 143 tasks (iteration 2 left 16 / 55 / 138; iteration 3 added Story 2.2.9 and Tasks 2.2.3b, 2.2.9a-c, 2.3.5d, and re-sized Tasks 0.2.1c, 2.3.1d, 2.3.1e from S to M); raw ~8.9M tokens (8,935k; was 8,540k); ~13.1M CU before verification (13,050k; was 12,464k).
- **Verification multiplier** x1.5-2 (story-level spec and quality review, `6-verify`, CI reruns): ~19.6-26.1M CU. Size band: **XL** (above 15M CU) at every multiplier in the range, and already XL for the multiplier's lower bound; the iteration-1 text that said "Large, upper edge" misapplied the bands (informational, not a gate). Planning overhead for this phase is additional.
- **Critical path (agent waves)**: W1 {0.1.1, 0.2.1a, 0.2.2, 0.3.1-0.3.5, 2.1.1} -> W2 {0.2.1b-d, 0.4.1b (G0 Part B), 1.1.1, 1.1.2a, 2.1.2, 2.1.3a-c, 2.2.9a-b (sidecar schema and flag, first), 2.2.7a (guard), 2.2.1/2.2.2/2.2.4/2.2.5 (experiment-critical), 2.2.7b-d} -> W3 {1.2.1 (A6 input), 0.4.1a (G0 Part A, committed after 1.2.1 and before 1.2.2 starts), 1.2.2 Go/Python triage, 1.1.2b-1.1.5, 2.1.4, 2.1.5 (Task 2.1.5b records the built-in input ceiling), 3.1.1 after 0.4.1b} -> W4 {2.3.1a and 2.3.1e (experiment-critical; need 2.1.5b), 1.2.x, 1.3.1 (G1 record), 3.1.2, 3.1.3, 3.1.4, remainder 2.3.1b-d/f and 2.3.2} -> W5 {3.2.1-3.2.6 fan-out, 3.3.x} -> W6 {3.4.1 (G2 record), ship-gated Phase 2 set (2.2.3 incl. 2.2.3b, 2.2.6, 2.2.8, 2.3.1g, 2.3.3-2.3.5), 4.x}; 2.2.9c (partition check) runs with the experiment-critical set in W4. Serialising points are files, not labels: `G0-decision.md` Part A before 1.2.2/1.3.1, Part B before any 3.x story, `G1-decision.md` before 4.1.1, `G2-decision.md` before 4.2.1/4.2.2 and before the report is unhidden.
- **Duration** is not stated in hours or days. It is two things read together: (1) the agent-wave critical path above (six waves, each bounded by its slowest story), and (2) the longest chain of wall-clock blockers, which sets the calendar and is: G0 Part A (owner) -> spikes 0.3.1, 0.3.2, 0.3.4 (tool installs and per-mutant timing runs on tstapler's machine) -> G0 Part B (owner) -> arms 3.2.x, each bounded by its `MutationBudget` T and ordered matrix-arms-first with a 1.5x stop-loss -> triage review 3.3.1 (owner) -> G2 (owner). Phase 1 and the experiment-critical part of Phase 2 run beside that chain and are not on it.
- **Re-baseline (required)**: Phase 3 rows (3.1.x, 3.2.x, 3.3.x) and the two triage rows (1.2.2, 3.3.1; about 900 verdicts for 1.2.2 per the adversarial review, likely 2-3x under) are priced from token bands only and exclude wall-clock. After spikes 0.3.1, 0.3.2, and 0.3.4 report measured mutation and replay cost, re-baseline those rows before G0 Part B is recorded; the figures above are a pre-spike placeholder. Each Phase 3 arm row gains a wall-clock-hours figure from `mutation-cost-spike.md` (T per arm, Story 0.3.4) in the re-baselined table; the arm's stop-loss is 1.5x that figure (Story 0.4.1).
- **Overrun checkpoint**: if spend exceeds 25% over the table for completed stories (about 1.25x the CU shown cumulatively), pause and report the delta and cause. Informational.
- **Wall-clock blockers (owners)**:
  - G0, G1, G2 owner decisions, recorded as `decisions/G{0,1,2}-decision.md` (Stories 0.4.1, 1.3.1, 3.4.1): tstapler.
  - Cloning and building corpus repos, installing cargo-mutants, gremlins (or go-mutesting), mutmut, Stryker (npx), PIT via Gradle (none present locally per `research/stack.md` section 6; installed in Tasks 0.3.4a-e): tstapler's machine / CI.
  - Mutation-run wall-clock per arm, bounded by the `MutationBudget` from spike 0.3.4 (T = (2 + N_nulls x K_mut) x M_s_killed x t_m); arms whose T exceeds the cap run coverage delta only.
  - Historical-commit replay builds for the real-history proxy, sized by 0.3.1b's power calculation.
  - Suite, per-test, and mutation runs on the corpus (kubernetes, cassandra scale): device/CI time; sampled per Story 0.3.2.
  - Manual triage review (1.2.2, 3.3.1): tstapler spot-checks verdicts the agent records.
  - Owner-triggered LLM pilot (3.3.4): tstapler approves spend.
- Basis: bands and weights INFERRED; recalibrate after the first implementer run.


---

## Repair log

Repair pass against `adversarial-review.md` (B1-B3 + concerns) and `architecture-review.md` (6 concerns + nitpicks). ADR-001, ADR-003, ADR-004, and ADR-005 were amended (each carries an "Amendment" section). Code facts were checked with grep against the working tree: `enum Command` at `src/main.rs:71`; `only_new` flag at `src/main.rs:311`; registration by `inventory::submit!` (e.g. `src/checkers/java_swallowed_interrupt.rs:126`, collected at `src/checker.rs:248`); `prose_checks()` at `src/config.rs:784`; `RULES` with `OwnerMatch::Unowned` entries (`god-class`) at `src/inline_ignores/rules.rs:84-99`.

| Item | Change made | Status |
|---|---|---|
| B1 mutation cost | New Story 0.3.4 (tool install + per-mutant timing + cost model T = (2 + N_nulls x K_mut) x M_s_killed x t_m + feasibility matrix); `MutationBudget` glossary term; survivors-not-rerun reduction; K_mut floor 19 with rank-p justification; per-language verdict (PIT/Stryker mutation-delta from matrix, Go/Python/Rust bounded-module or coverage-delta only, kubernetes-scale default coverage-only); Stories 3.1.2 and 3.2.1-3.2.3 conditioned on verdict; estimate rows updated and re-baseline required | Mitigated: the feasible set is decided by a measurement not yet taken |
| B2 Q3 | Q3 scoped to JVM/JS per-test and suite-level H1 for Go/Python/Rust; pre-written conclusion removed in Story 3.3.2 and 4.2.1 (decision rule, "unvalidated default" label, no extrapolation); Jaccard replaced by precision/recall/containment with Wilson intervals (sets are not nested in either direction, so directional precision is the decision statistic); PIT `coveringTests` typed `MutantSite`; exemplar selection criteria and named candidate shortlist in Story 0.2.2 (candidates INFERRED, confirmed at G0) | Mitigated: n is at most two suites, stated; Go/Python per-test Q3 remains unanswerable by design |
| B3 proxy | H1-H4 pre-registered with proposed numbers fixed at G0 (`G0-decision.md`, Story 0.4.1; Story 3.1.1 deviations rule); 20-commit replay replaced by a >= 200-commit window plus power calculation (~96 events for +/-0.10) and an explicit fallback tree ("Q6 answered same-family only"); nulls N1-N4 incl. stratified, runtime-matched, coverage-preserving; rank p-value with K >= 999 (matrix) / >= 19 (rerun) replaces "5th percentile of 30"; P2 operator-family and module split mandatory, P3 independent tool where available | Mitigated: real-fault validity still depends on the replay yield; same-family floor guarantees an answer, not a real-fault one |
| Flagged-set vs joint loss | `FlaggedSet` split into `IndividuallyFlagged` and `JointSafe`; `joint_safe_subset` in Story 2.1.2; H1/H2 evaluated on `JointSafe`, individual-set deletion as labeled secondary; report shows "redundant only with" | Resolved |
| Gates as stop points | Stories 0.4.1, 1.3.1, 3.4.1 produce `G0/G1/G2-decision.md`; downstream stories list them as preconditions; dependency diagram updated; `Command::Tests` merges hidden + experimental banner, unhidden or removed by Task 4.2.1c | Resolved |
| ADR-001 staleness | Story 2.3.5 | Resolved |
| Scalability / bitsets | Inverted index with sorted `u32` posting lists; Story 2.1.5 scale test (100k tests, 1M units, bounds proposed); `roaring` only if it fails; ADR-005 amended | Resolved (bounds are proposals until measured) |
| Flaky needs repeat runs | `SingleRun` exclusion; repeat in-order runs in Tasks 3.2.1c/3.2.2c/3.2.3c and spike 0.3.2 | Resolved |
| Estimates | Table updated for added/changed stories; totals recounted by grep; mandatory re-baseline note for Phase 3 and triage rows | Mitigated: still token-band priced until spikes report |
| Scope drift | Conditional stories remain; sequencing is unchanged except gates are now enforced by files. Owner stance "pursue full scope" kept | Accepted |
| Success Metric 3 | Explicit "dynamic vs static only" table required in Story 3.3.3 | Resolved |
| coverage.py attribution | AC and warning in Story 2.2.1 (empty context, fixture first-use) | Resolved |
| `--introduced-only` | Replaced by `--only-new` (Story 1.2.1) | Resolved |
| Inline-ignore "consumer only" | Story 2.3.2 + Task 2.3.2b: register `Signal` ids in `RULES` as `OwnerMatch::Unowned`; no-`[unused-ignore]` AC; drift-guard AC | Resolved |
| TestId to source | Story 2.1.4 `TestLocator`, `NoSource` exclusion, Go recipe encodes package path | Resolved |
| Per-dimension provenance | `TestMatrix` dimensions with own provenance, `CoverageAttribution`, `KillCompleteness`, `CoverageUnitKind` | Resolved |
| Shared test-function API | `test_functions` / `assertion_calls` in Story 0.2.1, Task 0.2.1d; one shared `assertion_helpers` resolution | Resolved |
| ADR-004 "touched" | Reworded to "added or signature-changed" in ADR-004 and Story 1.1.2 (body-edit gap cross-referenced to Story 4.1.2) | Resolved |
| Registration | Stories 1.1.5 and Pattern Decisions say `inventory::submit!`; `src/checkers/mod.rs` gets `pub mod` only | Resolved |
| `Commands` -> `Command` | Fixed in Tech Debt Disposition and Story 2.3.1 | Resolved |
| Nitpicks | `is_generated` dependency noted (re-export); doc-sync test direction reversed (doc cites code constants); churn counts added; Rust per-function note added; module declarations in Wave 1 | Resolved |
| Minor: Shi 52.2% scoping | Story 0.1.1 AC row now carries Java / failed-build-detection scope | Resolved |
| Minor: line refs | `src/check.rs:477` and `src/config.rs:759-769` were not re-opened in this pass (taken from the reviews, which verified them) | Unverified here |

### UX repair pass (applies `design/ux.md` section 10 deltas and section 11 unresolved flows)

| Item | Change made | Status |
|---|---|---|
| UX delta 1: `Stale`, `NoSource` | Glossary `ExclusionReason` gains `Stale` and `NoSource` (`NoSource` replaces `Unlocatable` everywhere in this plan; ADR-001 line 23 still says "unlocatable" prose and was left unedited); Story 2.1.3 ACs + Task 2.1.3c; Story 2.3.5 now excludes stale tests rather than only marking them | Resolved |
| UX delta 2: exit-code contract | Story 2.3.1 AC: exit 0 for any produced report, exit 2 for class E only, errors/warnings on stderr; Task 2.3.1c implements the R1-R22 message table at the time; R23-R26 were added in iteration 1 and the contract is now R1-R26, split across Tasks 2.3.1c, 2.3.1f, 2.3.1g and 2.3.5a/b in iteration 2 | Resolved |
| UX delta 3: `--by-package`, `--top 20`, Before-acting block | Story 2.3.1 ACs (output controls, Before-acting block, header/warnings above table, determinism, ASCII/NO_COLOR) | Resolved; `--by-package` auto-switch threshold is a pre-registered parameter |
| UX delta 4: hook per-file cap | Story 1.1.2 ACs (message shape, silent states, cap mechanism) + Task 1.1.2c; value NOT chosen | Mechanism resolved; value pending Story 1.2.2, recorded at G1 |
| UX delta 5: D-E1 decorator placement | Story 2.3.2 AC + Task 2.3.2c | Resolved (outcome of the placement test is measured, not assumed) |
| UX delta 6: commit provenance | Story 2.1.1 AC: missing commit is a warning (R11), not `MissingProvenance` | Resolved |
| Unanchored-finding dismissal (ux.md 11.1) | Decision in Story 2.3.2 / Task 2.3.2a: option (a), `NoSource` tests are not findings, no `(rule, test-id)` key in `.kibitzer/accepted/`. Rationale: that directory's key is `(rule, file, line, content)` with `content` as the staleness guard, and its Scope section excludes findings that lack a line (`docs/accepting-findings.md`, Scope and `content` sections; `docs/suppressing-checks.md`, directive-placement section). Task 2.3.2a still verifies against `src/inline_ignores/` and `src/accepted_findings.rs`; docs were read, code was not re-opened in this pass | Decided (docs-verified); code check pending in 2.3.2a |
| UX-D2 wording | Inline ignores have no content guard (only `accepted/` does), so "reappears only if the test's own source changes" was restated in Story 2.3.2 | Corrected |
| Tier thresholds, hook cap, `--by-package` threshold | New "Pre-registered parameters" section; Story 0.4.1 records the rules at G0; Stories 1.2.2, 1.3.1, 4.2.1 carry the supply/recording; no numbers invented | Pending experiments |
| Counts | Recounted by grep: 16 epics, 51 stories, 107 tasks (was 103); estimate rows 1.1.2, 2.1.3, 2.3.1, 2.3.2 updated; raw ~6.8M, ~9.9M CU, ~14.8-19.7M with verification multiplier | Resolved |
| UX acceptance coverage | ux.md's 41 UX criteria at the time (54 after iteration 1, 62 after iteration 2) are mapped by prefix into Stories 1.1.2 (H), 2.1.3/2.3.1/2.3.5 (R, E), 2.3.2 (D), 4.2.1 (E4, E7); UX-A1..A3 and S3/S6/S7/S8 surface bullets are not yet turned into ACs | Closed by the Phase 4 repair log below (A1 in 1.1.2, A2 and S6 in 1.1.5, A3 in 1.2.1, S3 and S7 in 2.3.1, S8 in 2.2.2, D-E4/D-E5 in 2.3.2) |

### Phase 4 repair log (applies `pre-mortem.md` and `consistency-review.md`)

Edited artifacts: this plan, ADR-001, ADR-002, ADR-003, ADR-004, `design/ux.md`, `implementation/validation.md`. Counts recounted with grep after the pass: 16 epics, 52 stories, 116 tasks, 31 glossary terms.

| Source | Item | Change made | Status |
|---|---|---|---|
| Pre-mortem P1 #1 | Null/negative result has no deliverable | Story 0.4.1 outcome table (static x H2 x H3 mapped to shipped artifacts) fixed at G0 Part B; `NegativeResultReport` glossary term and Task 3.3.3c (always-present section); descriptive-only report mode (Story 4.2.1, Task 4.2.1d); Story 3.4.1 verdicts `ship-with-tiers`/`ship-report-only`/`descriptive-only`/`remove`, with `remove` only on an owner-written word; Risk Control rollback reworded; Success Metric 4 interpretation put to the owner at G0 | Resolved in plan; owner accepts the SM4 interpretation at G0 Part B |
| Pre-mortem P1 #2 | H1 circular by construction | Circularity guard paragraph; invariant I0 (`HarnessInvariant`) split from H1; H1 redefined on held-out measures (mutation delta on sampled mutants for coverage-selected sets; family-B/module-B on matrix arms; P1 if powered); H2 evaluated on the held-out proxy only; coverage-delta-only arms reported "no validity evidence"; results rows carry `selection_data`, `evaluation_data`, `label`, `kind`; Stories 3.1.1, 3.1.2, 3.3.3 ACs | Resolved in plan; whether any arm passes is an experiment outcome |
| Consistency B1 | `SingleRun` contradicts one-command report | `SingleRun` removed from `ExclusionReason`; footer note "Stability not assessed: single run" that excludes nothing and caps nothing (Glossary, Stories 0.3.2, 2.1.3, Epic 3.2 goal); experiment rows carry `stability` | Resolved |
| Consistency B2 | Tool inventory has no story | Task 0.1.1d and an AC in Story 0.1.1 (one row per tool in `research/stack.md` sections 1-5 with five required columns) | Resolved |
| Consistency C1 | ADR-001 says `Unlocatable` | ADR-001 amendment edited to `NoSource` | Resolved |
| Consistency C2 | JSON field `rule` vs `signal` | `rule` chosen everywhere; Story 2.3.1 AC lists `schema_version`, `rule`, `commit`; per-record `not_assessed` boolean removed in favour of the top-level array (it contradicted the "assessed finding" meaning); `design/ux.md` section 4 aligned | Resolved |
| Consistency C3 | Rule-id strings | "Rule-id registry" table under the Glossary (including `test-zero-unique-kills`, `test-pseudo-tested-region`, `test-assertion-removed`); wireframe Rule column prints full ids; drift guard AC iterates the table | Resolved |
| Consistency C4 | ADR-002 decision 3 vs shared `test_code.rs` | ADR-002 decision 3 amended; Story 2.3.1 AC: report computes static rows for located tests and exports the `NoAssertion` flagged set used by Stories 3.2.1 and 3.2.6 | Resolved |
| Consistency C5 | Attribution source undefined | "Attribution source" rule under the Glossary (carry, then `--attribution`, then printed assumed default); ACs in Stories 2.1.1, 2.2.1, 2.2.2 (manifest `mode`), 2.2.3; ADR-001 amended | Resolved |
| Consistency C6 | ux.md caps not in plan; ADR-003 vs R14 | Single "Confidence caps" table (in-order Medium, no-kill-dimension Medium, plus the existing Low caps); ADR-003 amendment item 8 reconciles isolated vs in-order; recorded at G0 Part A | Resolved |
| Consistency C7 | G0 edges to 1.2.2/1.3.1 | Story 0.4.1 split into Part A (no spike dependency) and Part B; dependency diagram edges added; Stories 1.2.2 and 1.3.1 cite Part A | Resolved |
| Consistency C8 | SM3 no owner acceptance | Story 0.4.1 owner acceptances (a)-(c); Unresolved Questions entry | Resolved |
| Consistency C9 | `LeaveOneOut` has no producer | Story 3.1.4 (script, shortlist bounded by R_max, `--leave-one-out` input, `leave-one-out` label); `CoverageAttribution` glossary note | Resolved |
| Consistency N1-N5 | Nitpicks | N1 `[ignore-syntax]` in ux.md; N2 ux.md 2.3 wording aligned to "added or signature-changed"; N3 ranking key, `--input-commit`, `--junit`, `--experimental`, `--attribution`, `--leave-one-out` now in both docs; N4 all surface bullets have ACs; N5 ADR-004 "transitive" reworded to "bounded-depth" to match Story 1.1.2. Scope-drift items stay in (owner stance), now sequenced by the Phase 2 split | Resolved |
| UX gaps | UX-A1, A2, A3; S3 x3; S6 x2; S7; S8; D-E4, D-E5 | ACs added: A1 in Story 1.1.2; A2 and S6 in 1.1.5; A3 and Goodhart measurements in 1.2.1; S3 and S7 in 2.3.1 (Task 2.3.1d); S8 recipe output and errors in 2.2.2; D-E4 and D-E5 in 2.3.2 (Task 2.3.2d) | Resolved |
| Pre-mortem P2 #3 | Corpus runs infeasible | G0 Part B wall-clock budget, arm order by information per hour, 1.5x stop-loss `not run: budget`; wall-clock figure per arm in the re-baselined Effort rows | Resolved in plan; the figures come from spike 0.3.4 |
| Pre-mortem P2 #4 | Tool-install or exemplar failure | Story 0.2.2 time-box, third JVM/JS candidate, named fallbacks; Story 3.3.2 n < 2 sentence | Resolved |
| Pre-mortem P2 #5 | Adapter drift | Version allow-lists, W-class warning with `Low` cap, plausibility checks, per-fixture version records in Stories 2.2.1, 2.2.4, 2.2.5; fixture refresh Task 4.2.2c | Resolved |
| Pre-mortem P2 #6 | Static noise | Epic 1.1 re-sequenced (Go and Python through 1.2.2 triage first); precision-first unresolved-callee mode in Story 1.1.2, measured in 1.2.2; G1 per language | Resolved |
| Pre-mortem P2 #7 | Goodhart | Weak-oracle fixture family (1.1.1); measurements (a) and (b) in 1.2.1; G1 records them and drops the pre-filled directive from hook output if (a) is high | Resolved |
| Pre-mortem P2 #8 | Spend front-loaded on product surface | Phase 2 split into experiment-critical, ship-gated (needs G2), and remainder in the dependency diagram and critical path | Resolved |
| Pre-mortem P2 #9 | Agent-assigned precision labels | Owner blind 10% (min 30) relabel in Stories 1.2.2 and 3.3.1; Wilson bounds on confirmed labels; triage volume capped at the G0 Part A commitment | Resolved |
| Pre-mortem P3 #10 | Brief used as delete list | Story 2.3.4: mandatory non-empty `non_claims` and `verify_first`, `"action": "review"`, banned-verb scan across text/json/brief | Resolved |
| Deferred | none of the preventions deferred; two items are data-dependent, not deferred decisions | (a) wall-clock figures per arm and the Part A/Part B numbers are filled from spikes and the owner, not by this plan; (b) code facts in the Repair log row "Minor: line refs" remain unverified here (no code was opened in this pass, by instruction) | Open by nature |

### Triad repair log (iteration 1)

Applies the product, UX and engineering lenses of the first triad readiness pass. Edited artifacts: `requirements.md`, this plan, `design/ux.md`, `implementation/validation.md`, ADR-001 and ADR-005 (amendments). No code, and `decisions/G0-decision.md` was not created: it remains an owner decision, and this plan carries the ready-to-fill "G0 decision template" instead. Counts recounted with grep after the pass: 16 epics, 54 stories, 127 tasks, 33 glossary terms, 54 UX acceptance criteria in `design/ux.md` section 9; validation row totals are in `validation.md`.

| Source | Item | Change made | Status |
|---|---|---|---|
| UX B1 | JSON envelope lacked warnings, provenance, summary, mode; warnings were stderr-only | `ReportEnvelope` and `ReportMode` glossary terms; `design/ux.md` section 4 rewritten with the envelope and key order; Story 2.3.1 ACs (envelope contract, stream contract, text redirect case); Task 2.3.1e lands before 2.3.1d freezes `schema_version` 1; warnings in stdout body and `warnings[]`, stderr copy for json and brief only | Resolved in design and plan; implementation pending |
| UX B2 | Descriptive-only mode had no wireframe, conflicted with UX-R2 and R7 | `design/ux.md` section 3.2a (mode matrix, descriptive wireframe); UX-R2, R7 made mode-conditioned; UX-R13; Story 2.3.1 mode AC; Story 4.2.1 ACs name the matrix as the acceptance reference | Resolved in design and plan |
| UX B3 | Summary and ranking read as a delete list (Principle 3) | Principle 7 added; runtime total and runtime sort key removed everywhere (ranking is tier, rule id, test id); "joint effect if ALL removed" replaced by a measurement-worded joint coverage check; flagged overlap partners marked `*` and `overlaps_also_flagged`; "keep at least one of a flagged group" added to Before-acting; UX-R5, R7, R16, R17; Observability Plan metric reworded | Resolved in design and plan |
| UX G2 | Warning stream contradiction; `!` vs `warning:` | Stream contract in section 3.4; one `warning:` prefix everywhere; UX-E2, UX-R10 reworded | Resolved |
| UX G5 | Rows lacked `file:line`; names truncated at 40 | Rows are `path:line: [rule] TestName`, nothing truncated; UX-R11, R15 | Resolved |
| UX G6 | No progress notice, no executes-repo-code notice, no resume | Recipe notice, progress and `--resume` (Story 2.2.2, Task 2.2.2c); slow-read notice (UX-E9); UX-E11 | Resolved in design and plan; threshold for the read notice is a pre-registered parameter |
| UX G7 | `--experimental` rows unspecified | Separate "Experimental signals" section, `(experimental)` tag, `signal_mode`; UX-E12 | Resolved |
| UX G8 | Dirty tree, non-ancestor, non-git cases | R23-R25, Story 2.3.5 ACs, Task 2.3.5b; UX-E10 | Resolved |
| UX G9 | No acknowledgement path for no-source tests | Explicit decision (none offered, with rationale) restated in `design/ux.md` section 5.4 and 11, matching Task 2.3.2a; revisit trigger kept | Decided, not a path |
| UX G10 | Non-claim missing from JSON; Goodhart | `advisory` and global `non_claims` in the envelope; `report_false_positive` acknowledgement text (UX-A5); non-specific-reason counter and hint (D-E6, UX-D7), stoplist deferred to G1 | Resolved in design and plan; stoplist value pending Story 1.2.1 |
| UX G11, G12 | Legend; placeholders and labels | Legend block and UX-R18; R5 floor from the adapter allow-list; R13 label explained in the Legend; `scope:` line in one form; "no color in v1" stated | Resolved |
| Product P1 | Primary persona unranked | Ranked list with rationale in `requirements.md`; G0 template line A1 (owner choice, recommended default agent first) | Resolved as recommendation; owner confirms at G0 Part A |
| Product P2 | SM4 satisfiable by a null result without saying so | `requirements.md` "How SM3 and SM4 are read": validated check vs descriptive report on a null result (informative negative, research-track success); template line A7 | Resolved |
| Product P3 | SM3/SM4 numbers unrecorded | Stated that thresholds live in G0 Part B; proposed defaults cited from H1-H4 in the template; Story 0.4.1 AC | Resolved as template; numbers await the owner |
| Product P4 | No adoption leading metric | SM5 flagged-then-revised rate in `requirements.md`; Task 1.2.1c baselines it | Defined and baselined pre-ship; live figure measurable only after G1 |
| Product P5 | Adoption risk unnamed | Feasibility Risks entry in `requirements.md`, UNVERIFIED until SM5 | Named, not retired |
| Product P6 | Roadmap rationale missing | "Why now" section in `requirements.md`, limited to what `docs/check-ideas.md` and the research files support; states that the transcript-occurrence bar is not yet met | Resolved as an honest rationale; demand evidence remains one unsourced anecdote |
| Product P7 | Inozemtseva & Holmes labeled UNVERIFIED | Relabeled VERIFIED (full text) with scope, after checking `research/literature-effectiveness-mutation.md` bibliography row 1 | Resolved |
| Eng E1 | Hook latency only relative | Absolute p95 ms budget on the full hook path, committed in the first commit of `G1-decision.md` before measurement (Stories 1.2.3, 1.3.1; Pre-registered parameters row 4) | Resolved as a parameter; the number is the owner's |
| Eng E2 | No parser hardening | Story 2.2.7 (size cap, depth, XXE-safe PIT, malformed-input corpus, path containment), ADR-005 amendment | Resolved in plan |
| Eng E3 | No sandbox or secrets guidance for experiment arms | Story 0.3.5 (policy, preflight, secret scan), Task 3.1.2d (`isolation` row fields, refusal without a mode), Epic 3.2 goal, ADR-001 amendment | Resolved in plan |
| Eng E4 | Scale test covered algorithms only | Story 2.1.5 extended: parse-memory per adapter format and `TestLocator` at 10^5 tests (Tasks 2.1.5b, 2.1.5c) | Resolved in plan |
| Eng E5 | Phase 3 rows priced token-only | Unchanged: the existing re-baseline requirement stands; new rows are priced by the same bands | Open by nature |
| Totals | Counts and estimate | Recounted by grep and by summing the Raw and Est. CU columns | Resolved |

Remaining after this iteration: (1) every `<owner fills>` line of the G0 template, the G1 budget commit and the G2 verdicts are owner decisions; (2) the default numbers (hook cap value, tier boundaries, `--by-package` and slow-read thresholds, input cap, non-specific-reason stoplist) still wait on the named experiments; (3) the code facts carried in the Repair log row "Minor: line refs" are still unverified in the planning artifacts; (4) UX acceptance for the new surface elements is specified but not exercised until implementation, so these are design-level closures, not shipped behavior.

### Triad repair log (iteration 2)

Applies the round-2 UX, engineering and product lenses (analyses in `/home/tstapler/.claude/jobs/70315fb5/tmp/triad-{ux,engineering,product}-r2.txt`). Edited artifacts: `requirements.md` (SM5), this plan, `design/ux.md`, `implementation/validation.md`, ADR-005 (amendment). No code. `decisions/G0-decision.md` was not created. Code facts re-verified by Grep/Read on `d76c1ab`: `report_false_positive` is a local JSONL queue append (`src/mcp.rs:762-785`, `src/false_positive.rs`), `check_name` is unvalidated; `CheckContext { source, tree }` at `src/checker.rs:164-167`; `[dev-dependencies]` holds only `kibitzer-stub-plugin`; `covgate.toml` exists (80/75); no hook, pre-commit or secret-pattern file exists; `src/config.rs:860-862` is the `**/*_test.go` glob (867-875 was a Java comment). Counts recounted by grep and by summing the Effort table: 16 epics, 55 stories, 138 tasks, 33 glossary terms, 62 UX criteria, 379 validation rows.

| Source | Item | Change made | Status |
|---|---|---|---|
| UX B1 | Joint line tautological (JointSafe is zero by construction) and read as a delete green light | Line computed over `IndividuallyFlagged` of the main list, naming set, size and members with coverage data; the JointSafe zero is never printed (harness invariant I0 and unit tests only); "jointly checked: 25" replaced by "of these, N are redundant only with another flagged test"; descriptive mode uses the same line over the listed set; Principle 7, UX-R7, mode matrix, Story 2.3.1 ACs, Glossary `HarnessInvariant` | Resolved in design and plan |
| UX G1 | Wireframe row order broke the ranking rule | Wireframe reordered (high first, then rule id), ranking AC added | Resolved |
| UX G2, G3 | Experimental rows in main counts; `--experimental` undefined pre-G2 | Main-list rule: structural `Low` caps pre-G2, the verdict post-G2; experimental section has its own count line and `summary.experimental`; arithmetic fixed (8 more, not 28) | Resolved; the pre-G2 rule is a design choice the owner may revise |
| UX G4 | Stale and no-source missing from scope and breakdown | Scope line and Not-assessed parts sum to K, fixed reason order, `repo_tests_not_in_input` note | Resolved |
| UX G5 | Dismissal (comment edit) marked tests stale | Comment-only exemption via comment-stripped token stream; AC in Story 2.3.5, Task 2.3.5c, UX-E14 | Resolved; falls back to "changed" when no grammar |
| UX G6 | Plan dropped the pre-filled ignore line in hook output | Quiet-path line never removed: V1 copyable, V2 pointer if G1 records measurement (a) high; Stories 1.3.1 and 4.1.1 ACs, UX-H8 | Resolved |
| UX G7 | Filler reasons and test deletion unmeasured at write time | SM5 gains removed-with/without-named-test buckets (`requirements.md`, Task 1.2.1c); hook write-time limits stated as accepted | Mitigated: hook cannot verify a named covering test |
| UX G8 | Machine contract holes | `test.id` grammar (path-qualified), `value.kind`, stable warning slugs, structured `lowers`, class E error object, `format` key and identical brief envelope, `mutation_run` and `kill_completeness` values, `suppressed[]` | Resolved in design and plan |
| UX G9 | `--top` hid whole classes; no `--by-package` wireframe | Rule-stratified window, per-rule omitted line, wireframe 3.2c, per-package `--top` | Resolved |
| UX G10, G11 | "flaky" asserts a cause; report-only and tiered had no banner | Label `inputs disagree` (variant name kept); four banners backed by an existing doc anchor; wireframe 3.2b | Resolved |
| UX G12 | Dismissals not auditable | `--show-suppressed`, json `suppressed[]` (UX-D8) | Resolved; vanished findings are not listed (stated) |
| UX G13 | Recipe UX only for Go | New Story 2.2.8 and `design/ux.md` section 8.1 (notice, progress, partial, resume, budget, sidecar) | Resolved in design and plan; wrapper scripts unbuilt |
| UX G14 | Minor items; stale counts | Confidence-tag rule, MCP shape, descriptive `*`, legend, R1-R26 and criteria counts in earlier log rows corrected | Resolved |
| Eng D1 | Experiment consumed REMAINDER pieces | 2.1.3c, 2.3.1a, 2.3.1e moved into the experiment-critical set; harness static flagged sets come from `kibitzer check native` (own source); diagram edges added | Resolved |
| Eng D2 | Story 2.3.1 could not close in its wave | Closure partition: experiment-critical, remainder and gated ACs with named owner stories; flags accepted-and-warn until their owner lands | Resolved |
| Eng D3 | Story 2.2.7 owned gated files | Scoped to the four experiment-critical adapters and locator; lcov/junit hardening in 2.2.3/2.2.6, staleness containment in Task 2.3.5c; built-in ceiling recorded by Task 2.1.5b before the hidden variant merges | Resolved |
| Eng sizing | Oversize tasks 2.2.7a, 0.3.4a, 2.3.1c, 0.2.1a-d | 2.2.7 split into a-d; 0.3.4 per language a-f; 2.3.1c split into c, f, g (R10 and R23-R25 with 2.3.5); 0.2.1 as `src/test_code/{mod,go,python,ts,java,rust}.rs` | Resolved |
| Eng A1 | D-E4/UX-A5 described a server | Reworded to the real local queue behaviour; report rule ids work as `check_name` (verified); sentences appended to existing replies | Resolved |
| Eng A2 | Test tooling not in repo | ADR-005 amendment: std-only pattern, seeded PRNG, checked-in goldens, no new dev-dependencies | Resolved; owner may prefer the crates (recorded as a new decision) |
| Eng A3, A4 | covgate not mentioned; no secret-scan mechanism | "Quality gates" section; Task 0.3.5b creates the hook, patterns and scan | Resolved |
| Eng S1 | Unbounded input before the cap exists | Built-in ceiling from Task 2.1.5b is a precondition of the hidden variant; fetch step under the env allow-list | Mitigated: the ceiling's value is measured, not chosen here |
| Eng effort, NFR | Size band wrong; duration; parse reuse; mis-citation | XL stated; duration as wave path plus longest blocker chain; one-parse AC and test; `src/config.rs:860-862` | Resolved |
| Eng validation | Rows for 0.3.5 throwaway copy, 2.2.7 cap text | Rows added or renamed | Resolved |
| Product | SM5 target, baseline reading, zero-occurrence rule, triage default | Template lines A5 default, A8, A9; SM5 text; Story 1.2.1 AC | Resolved as owner-input placeholders; no numbers invented |

Remaining after this iteration: the `<owner fills>` lines (A3, A6, A8, A9 threshold, B1-B15), the numbers waiting on experiments, and the unexercised implementation of all new UX elements (design-level closures). The pre-G2 main-list rule (structural caps) and the by-package default of 5 rows are proposals the owner may change at G0.

### Triad repair log (iteration 3)

Applies the round-3 UX, engineering and product lenses (analyses in `/home/tstapler/.claude/jobs/70315fb5/tmp/triad-{ux,engineering,product}-r3.txt`). Edited artifacts: `requirements.md`, this plan, `design/ux.md`, `implementation/validation.md`. No code, no ADR change, and `decisions/G0-decision.md` was not created. Counts recounted by grep and by a script over the Effort and validation tables: 16 epics, 56 stories, 143 tasks, 33 glossary terms, 68 UX criteria, 415 validation rows (413 unique cases).

| Source | Item | Change made | Status |
|---|---|---|---|
| Eng E-B1 | Task 2.1.5b scale-tested lcov (ship-gated) and omitted gocover | 2.1.5b restricted to coveragepy, gocover, stryker, pit; ceiling is the smallest per-format largest-passing input; lcov scale test is Task 2.2.3b in ship-gated Story 2.2.3; diagram and validation rows fixed | Resolved |
| Eng E-B2 | Adapters 2.2.1/2.2.2/2.2.4/2.2.5 depended on the sidecar and flag owned by ship-gated 2.2.8 | New experiment-critical Story 2.2.9 (sidecar schema, reader, `--kill-completeness`, hand-written fixtures); 2.2.8 keeps only the wrapper scripts; adapter ACs cite 2.2.9 | Resolved |
| Eng E-B3 | Story 2.2.7 needed R26 from remainder Task 2.3.1c; Rust arm could depend on ship-gated lcov (cycle) | Minimal class E path with R26 (id, JSON error object, exit 2) moved into experiment-critical Task 2.3.1a; Task 2.3.1c adds the other 9 messages. Story 3.2.3 is NoData-only (rationale in the story: Q3 is unanswerable for Rust either way, and a minimal reader would still need guard, fixtures and the `TN:` check) | Resolved by choosing NoData-only; per-test Rust coverage is a post-G2 follow-up |
| Eng gaps | Sizing | Tasks 0.2.1c, 2.3.1d, 2.3.1e re-sized S to M; Effort rows recomputed (raw 8,935k, CU 13,050k, x1.5-2 gives ~19.6-26.1M CU, still XL) | Resolved |
| Eng gaps | Go snapshot markers shared by 0.2.1b and 0.2.1d | Markers moved to 0.2.1b; 0.2.1d is `rust.rs` only; tautology shapes stay in the checker file so 1.1.2b and 1.1.3 do not share a language file | Resolved |
| Eng gaps | 2.2.7a guard-only vs retrofit | 2.2.7a is `guard.rs` only; each adapter story writes its own call; 2.2.7b-d follow the adapter they touch | Resolved |
| Eng gaps | PIT `coveringTests` superset bias | Stated in Story 2.2.5, Q3, and a Story 3.3.2 AC and task | Resolved (stated; the comparison stays weakly informative) |
| Eng gaps | Commit strings reach `git` | Task 2.3.5d and AC: hex object id validation of `--input-commit` and input-carried commits, R11 warning on a bad value | Resolved |
| Eng gaps | Recipe scope; secret-scan scope | Story 0.3.5 states the allow-list governs the harness, shipped recipes run repo code with caller env and only print the notice; Task 0.3.5b scan covers `docs/backtest-triage/` JSONL | Resolved as scope statement; recipes are not enforced |
| Eng gaps | Partition not asserted; no per-story traceability | Task 2.2.9c `check-partition.sh` plus a validation row; "Story Traceability Index" in `validation.md` (56 anchors) | Resolved |
| Eng gaps | Stale file lists | Stories 1.1.2, 1.1.3, 2.1.3 and tech-debt rows point at `src/test_code/mod.rs` and per-language files | Resolved |
| UX B1 | `signal_mode` could not tell main from experimental rows | Per-record `section` (`main`/`experimental`); `signal_mode` vocabulary unified with `mode`, `experimental` = no verdict; least-validated order `experimental` < `descriptive` < `report-only` < `tiered`; UX-R22, UX-E12 | Resolved. Design choice flagged: after G2 a `descriptive-only` verdict keeps its rows in the main list (so the null-result report is reachable without `--experimental`) |
| UX B2 | Joint check zero case and mixed classes | Per rule class and `unit_kind`, static rules excluded, zero always carries a non-claim (`zero_note`), `summary.joint_check` is an array; UX-R7 | Resolved |
| UX gaps | Wireframe order and arithmetic, descriptive title, JSON order | Wireframes show rows 1, 10, 15 with bracketed elisions; 9+5+6 = 20 window; descriptive wireframe lists all three classes with a neutral title and ascending test id; JSON example reordered | Resolved |
| UX gaps | Contract holes | Enum spelling map, report-only summary keys, `test.id` in overlap fields, `redundant_only_with_flagged` defined, `leave_one_out` field and errors (reusing R3/R4), exit 1, malformed-directive note, R10/R23/R24 precedence, assumed attribution (R14 variant, same id), `record_non_claims`, wrap rule, "added or signature-changed" wording; UX-R22-R24, E16, D9, X6 | Resolved |
| Product | A8 fallback; minimum live sample; "not estimable" baseline; adoption scope | A8 and requirements SM5: unset keeps G1 verdicts and SM5 descriptive; 30 tests per rule minimum (A5's figure, owner may raise); not-estimable baseline leaves the assumption UNVERIFIED; new line A10 (`owner-dogfooding` default, G0 owner choice) | Resolved as defaults; A8, A10 values are owner inputs |

Remaining after this iteration (iteration 3 is the last permitted): the `<owner fills>` lines (A3, A6, A8 rate, A9 threshold, A10, B1-B15), numbers waiting on experiments, wrapper scripts and all new UX elements unexercised (design-level closures), and the flag that the 30-test minimum for demotion reuses A5's triage figure rather than a sourced number.
