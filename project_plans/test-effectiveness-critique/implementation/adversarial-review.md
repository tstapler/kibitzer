# Adversarial Review: test-effectiveness-critique

**Date**: 2026-10-11
**Verdict**: CONCERNS (re-review 2026-10-11; original verdict was BLOCKED, see "Re-review" at the end)

Basis: plan.md, requirements.md, ADR-001..005, research/pitfalls.md (skimmed), research/stack.md (targeted greps), and spot-checks of the repo at `d76c1ab`.

## Blockers

- [ ] **B1. The deletion experiment cannot produce mutation-score deltas or the 30-seed random baseline for Go, Python, or Rust at the stated cost.** Story 3.1.2 requires `mutation_score_before/after` plus `random_baseline: [..30 values..]`. `research/stack.md` (section 2 summary and its "Open gap") records that no Go/Python/Rust tool emits a per-test kill matrix (gremlins, go-mutesting, cargo-mutants give per-mutant status only; mutmut maps tests per function). A mutation score for suite-minus-flagged therefore needs a fresh mutation run per deleted set: 1 flagged + 30 random = 31 mutation runs per arm, and more for the held-out proxy replays. Spike 0.3.2 measures only per-test coverage cost, never mutation-run cost. No task installs gremlins or go-mutesting, and Story 3.2.2 (Go arm) never names a mutation tool. The estimates (3.2.x at M+S, ~229k CU each) assume none of this. Recommendation: before G0, fix in the protocol which arms compute mutation delta from a full kill matrix (PIT/Stryker only) and which use bounded-module mutation reruns with an explicit run budget. Add mutation-run cost to spike 0.3.2. Add the missing tool-install and Go mutation-run tasks, or state that Go/Python/Rust arms report coverage and proxy only.

- [ ] **B2. Q3 cannot be answered for the languages the plan defaults to, and Story 4.2.1 pre-writes the opposite.** Q3 is resolved only by arms 3.2.4 (PIT, one sampled Java module) and 3.2.5 (Stryker, one small JS/TS project). Both exemplars are INFERRED candidates not yet chosen (Story 0.2.2). The Q4 default hypothesis makes Go and Python first-class, which have no ground-truth kill matrix. Story 4.2.1's acceptance example reads "unique coverage suffices on Go/Python", extrapolating from Java/JS with n of about 2 suites. Jaccard of the "zero-unique" sets is also the wrong statistic. A test with zero unique coverage can only have zero unique kills if its lines are covered elsewhere, but the reverse does not hold. The sets are nested by construction, so containment or precision/recall is what answers "is the cheaper signal sufficient". Recommendation: scope the Q3 answer to JVM/JS in the plan text, forbid extrapolation in 3.3.2 and 4.2.1, report containment and precision/recall with CIs, and either add a sampled Python arm (per-test mutmut reruns on one module) or label Go/Python ordering as "unvalidated default".

- [ ] **B3. The held-out proxy and baseline do not support the validity claim the plan makes (Q6).**
  - (a) Spike 0.3.1 tries replay on 20 consecutive commits of two repos. The Shi et al. data the plan leans on needed 1,478 failed builds across 32 projects to measure failed-build detection loss. Twenty commits will have zero to a handful of pre-existing-test failures, so no power exists. The plan has no fallback tree for that outcome.
  - (b) The fallback, a disjoint mutant split, is same-family and correlated with a selection made on unique kills. If the split is a plan option rather than a mandatory floor, H1/H2 can degenerate into "chosen on mutants, validated on mutants".
  - (c) "Beat a size-matched random baseline" is nearly vacuous. A set of coverage-redundant tests beats uniform random on any coverage-correlated proxy, because random sets contain unique-coverage tests. The meaningful null is a cheap-heuristic or stratified baseline (same package/kind, or smallest/fastest tests).
  - (d) With 30 seeds, the "5th percentile" is about the 1.5th order statistic. Use a rank p-value or far more seeds (cheap when computed from a matrix).
  - (e) H4's precision threshold is deferred until after triage (Unresolved Questions), so it is not pre-registered.
  Recommendation: make "select on mutant sample A, evaluate on B" a mandatory arm, and add a power analysis and a stated outcome "Q6 answered same-family only". Add a stratified/heuristic baseline and require H4's threshold at G0.

## Concerns

- [ ] **Flagged-set semantics contradict the plan's own pitfall P11.** FlaggedSet is "tests selected by one signal" and is deleted jointly in 3.1.2, but `ZeroUniqueCoverage` is computed per test (two duplicates both show zero unique coverage; jointly they are necessary). H1 would fail by construction, or pass only under the greedy reduction. Define which set is deleted (individually flagged vs greedy reduction vs joint-loss-clean) per hypothesis.
- [ ] **Owner gates are labels, not stop points.** G0/G1/G2 are bold lines after epics, with no task whose acceptance is "owner decision recorded in X". Status says "Ready for implementation". `kibitzer tests report` (Phase 2) merges to master with tiers "derived from signal class" before G2, so "keep research-only" at G2 is not available. Add explicit gate tasks with a recorded decision file, hide the subcommand (`#[command(hide)]` or an `experimental` marker) until G2, and make 3.2.x and 4.x depend on that file.
- [ ] **ADR-001 staleness is never built.** ADR-001 says findings carry the input commit and are marked stale when production files changed; no story or acceptance criterion implements it. Add to 2.3.1.
- [ ] **Scalability NFR is unaddressed.** ADR-005 hand-rolls dense `Vec<u64>` test-by-unit bitsets. At 10^5 tests x ~10^6 lines this is on the order of tens of GB. Unique coverage needs only per-unit hit counts or an inverted index. Specify the representation and add a scale test story rather than "revisit if measured".
- [ ] **Flaky detection needs repeated runs nobody budgets.** Story 2.1.3 flags `Flaky` by comparing two matrices of the same suite, but the experiment arms collect one run each. Add repeat runs to 3.2.x and 0.3.2 costs, or downgrade to "unassessed: single run".
- [ ] **Estimate realism.** Wall-clock-dominated work (toolchain installs, PIT-on-Gradle, Stryker, commit replay builds) is priced as M/S token tasks. Triage volume (1.2.2: at least 50 findings x 3 rules x up to 6 languages, about 900 verdicts, in M+S+S) looks about 2-3x under. The experiment arms in B1 are the largest miss. The overrun checkpoint (1.25x cumulative) will trip early. Re-baseline the Phase 3 rows after spikes 0.3.1/0.3.2 and say so.
- [ ] **Scope drift against requirements.** Conditional or speculative stories (4.1.2 assertion-removed, 2.3.3 `ArchModel` enrichment, 3.3.4 LLM pilot, JUnit adapter 2.2.6, `test-mock-only`, Kotlin/Rust profiles with fixtures for six languages) are built before any triage evidence. "Pursue full scope" is the owner's stated stance, so keep, but sequence them behind G1/G2 rather than in W3.
- [ ] **Success Metric 3 is partially met.** The requirements name the corpus (kubernetes, cassandra, servo, ripgrep, deno, vscode, stapler-squad). Dynamic arms cover stapler-squad, k8s subpackages, ripgrep, and three unchosen exemplars. cassandra/servo/deno/vscode get static runs only. State this explicitly in 3.3.3 limitations.
- [ ] **Attribution under Python contexts.** `coverage.py` attributes import-time and module-level lines to the empty context and fixtures to whichever test triggers them first. Unique-coverage for the first test in a module is inflated. Story 2.2.1's 3-test fixture will not expose it. Add a fixture with a module-level import and a shared fixture.

## Minors

- Story 1.2.1 cites `kibitzer check backtest ... --introduced-only`; the real flag is `--only-new` (`src/main.rs`, `CheckCommand::Backtest`). Acceptance criterion as written will fail.
- Plan line references (`src/check.rs:477`, `src/config.rs:759-769`) were not re-opened here; `default_checks()` and `scope_output_to_changed_lines` do exist (`src/config.rs`, `src/check.rs:942`).
- Story 0.3.2 "20,000-test suite yields a bounded run count" has no stated bound; pick the number at G0.
- The "5th percentile" criterion in 3.1.1 is quoted as "or the owner-approved equivalent", which defers the one number that pre-registration exists to fix.
- Unverified literature is correctly quarantined in the plan; the risk is that Story 0.1.1 AC examples quote the Shi 52.2% figure without scoping it (Java, failed-build detection) in the catalog row.

---

## Re-review (after plan repair)

**Verdict: CONCERNS.** B1, B2, B3 are resolved. No new blockers found. Checked: `plan.md` (Phase 3 stories 3.1.x-3.4.1, 0.3.1, 0.3.4, 0.4.1, 4.2.1, Repair log), ADR-003 Amendment, `requirements.md` Success Metrics / NFRs. A script confirmed all 51 stories have at least one task (107 tasks).

### Previously BLOCKED items

- **B1 (mutation cost): resolved.** Story 0.3.4 installs and times each tool, fixes T = (2 + N_nulls x K_mut) x M_s_killed x t_m, and yields a per-language verdict. Survivors are correctly not rerun. K_mut >= 19 is correct for a one-sided rank p of 0.05. Arms over budget fall back to coverage delta only, and Stories 3.1.2 and 3.2.1-3.2.3 are conditioned on the verdict. Go/Python/Rust no longer pretend to have per-test kill matrices. Estimates are still placeholders, but a re-baseline before G0 is mandated.
- **B2 (Q3): resolved.** Q3 is scoped to JVM/JS per-test; Go/Python/Rust are "unvalidated default" with extrapolation forbidden in 3.3.2 and 4.2.1 (with Given/When/Then checks). The decision statistic is directional precision/containment with Wilson lower bound. The repair's note that the sets are not nested in either direction is correct and supersedes my original "nested by construction" claim. The conclusion is a decision rule, not a pre-written sentence.
- **B3 (proxy/baseline): resolved.** The power gate (about 96 events) plus an explicit fallback tree ("Q6 answered same-family only") replaces the 20-commit spike. The P2 split is a mandatory floor. Nulls N2 (stratified) and N4 (coverage-preserving) must be beaten via rank p-value with K >= 999 from a matrix or K >= 19 for reruns. The H4 floor is pre-registered at G0. ADR-003 Amendment matches the plan. Gates are decision files that downstream stories list as preconditions, and the subcommand is hidden until G2.

### Remaining concerns (not blockers, each honestly labeled or cheap to fix at G0)

- **Subset-suite mutation runs unverified.** Spike 0.3.4's acceptance criteria run each tool on the full suite only. Nothing verifies that gremlins, mutmut and cargo-mutants can be pointed at a skip-list or subset suite, which the deletion harness needs (Task 3.1.2a covers only `go test`/pytest/cargo skip lists). The fallback is coverage-delta only, so this is not blocking, but add "run once with a restricted test set" to 0.3.4a.
- **Sequencing.** 0.3.4 and 0.3.1 run "on the named module/Python exemplar" but are scheduled in W1 in parallel with 0.2.2, which names the modules. Use stapler-squad/ripgrep first, or order 0.2.2a first. Similarly the `--by-package` threshold comes from 2.1.5 (W3) but is a G0 (W2) item; the plan handles this by recording the rule only.
- **Coverage-delta-only arms (e.g. kubernetes) have no held-out proxy**, so H2 and the mutation half of H1 are undefined there. State "H1/H2 not evaluated" explicitly in 3.1.2/3.3.3 for such arms.
- **JaCoCo.** 3.2.4 prefers JaCoCo line coverage, but no adapter story covers JaCoCo XML (2.2.3 is lcov); falls back to MutantSite labeling.
- **Estimates** for 0.3.4 (M+S for five toolchain installs) and the Phase 3 rows remain token-band placeholders; the mandated re-baseline is the control.
- UX acceptance criteria S3/S6/S7/S8/A3 remain open (already logged in the Repair log); the line references in the Repair log remain unverified.
