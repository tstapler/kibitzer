# Requirements: test-effectiveness-critique

**Date**: 2026-10-11
**Type**: feature addition (new check/report "track" — research-led)
**Complexity**: 3 — system design (new signal class that needs test execution data, multiple epics, external tool integration: coverage + mutation tools across languages)

## Problem Statement

Test suites accumulate tests that add cost (runtime, maintenance, review surface, false confidence) but little
fault-detection value: tests that duplicate another test's coverage, execute code without asserting on it,
assert tautologies or only mock interactions, or kill no mutants that another test doesn't already kill.
Humans and LLM agents both add tests freely and almost never delete them, because nothing tells them
which tests are earning their keep.

A practitioner reported running Opus over their repos to judge test quality and deleting roughly a quarter of the
tests with no significant change in coverage. That is an existence proof that (a) a large fraction of a typical
suite is low-value and (b) an LLM can find it — but the LLM judgment is slow, expensive, non-reproducible, and
unauditable. The problem this project addresses: **identify what mechanical, cheap, reproducible signals exist
for "this test is ineffective", how well each is validated in the literature, and which of them kibitzer
should ship as advisory checks/reports** — so that LLM judgment is reserved for the ambiguous residue, not the
whole suite.

Kibitzer currently critiques production code and docs (`default_checks()` in `src/config.rs`) and uses
coverage only as a CI diff gate on itself (`covgate.toml`). It has no check that critiques test quality.

## Baseline

- **Today**: coverage % (line/branch) is the de-facto test-quality proxy. The literature
  (Inozemtseva & Holmes, ICSE 2014, "Coverage is not strongly correlated with test suite effectiveness") says
  this is weak once suite size is controlled. VERIFIED (full text) in
  `research/literature-effectiveness-mutation.md` (bibliography row 1 and the Q2 evidence table): across 31,000
  suites on five Java systems the correlation after size control is "low to moderate". Scope: Java only; the
  same document records no evidence for Go, Rust, TS or Kotlin.
- **Manual / LLM review**: ad-hoc Opus pass over a repo's tests, as described above. No reproducible tool,
  no ranking, no false-positive tracking.
- **Existing kibitzer surface**: static per-file checkers; `affected.rs` (diff → affected tests);
  `backtest.rs` (replay Claude Code transcripts); `docs/backtest-repos.md` real-world corpus.
  Nothing that executes tests or ingests coverage/mutation output.

## Users / Consumers

Ranked, with the recommended default below. The ranking is a **G0 owner choice** (recorded in Part A of
`decisions/G0-decision.md`, see the plan's "G0 decision template"); the plan's two surfaces (ADR-002) serve
different consumers, so the ranking also says which surface carries the product bet.

1. **AI agents** (Claude Code hook, MCP `run_checks`) that write tests and should be told when a test they just
   added is vacuous. Primary. Rationale: kibitzer's stated purpose is "advisory, diff-aware code/doc quality
   checks for AI agents, CI, and local dev" (`CLAUDE.md`), agents are the only consumer with an automatic
   behavior-change path (the hook is read on every edit), and the static surface is the only one that can
   become default-on through gate G1. Job to be done: "tell me when the test I just wrote is vacuous, so I fix
   it before claiming done" (`research/ux.md` section 5, INFERRED).
2. **The repo owner doing periodic suite hygiene** ("what can I delete?"). Secondary, and the consumer of the
   dynamic batch report; also the person who records G0-G2, so the research track is built for them. The
   practitioner anecdote in the Problem Statement describes this user.
3. **Developers and CI** running `kibitzer run` / hook mode on a diff. Tertiary: they receive the same static
   findings as agents, and the report's JSON is their archive format; no separate surface is built for them.

Demand evidence is thin: one unsourced anecdote (Feasibility Risks) and the literature. The ranking is a
recommendation, not a measurement; SM5 below is how it gets checked after ship.

**Whose adoption is the goal (iteration 3, product).** The demand and adoption evidence is single-user: the
owner's own transcripts and one anecdote. Recommended default, a **G0 owner choice** (template line A10):
this project is **owner dogfooding**; success is judged on the owner's repos and sessions, and external
adoption is not a goal or a metric. The tool is public, so others may use it, but the plan makes no claim
that agents or repos other than the owner's behave the same way, and no gate depends on external users.
Choosing "external adoption is a goal" instead adds a requirement this plan does not meet: evidence from at
least one more user or agent population before G1 promotion to `default_checks()`.

## Success Metrics

Measured against the baseline above:

1. **Literature deliverable**: a cited survey covering test-suite effectiveness, minimization/reduction,
   redundancy, mutation analysis, and test smells; every candidate signal has ≥1 primary-source citation and a
   stated validation status (empirically validated / plausible / unvalidated).
2. **Candidate-signal catalog**: each signal classified by (a) input required (static source only / per-test
   coverage / mutation results / run history), (b) cost, (c) false-positive risk, (d) whether it fits kibitzer's
   advisory diff-aware model.
3. **Empirical result on the real-world corpus** (`docs/backtest-repos.md`): for the signal(s) chosen for
   implementation, a measured answer to "if you delete the flagged tests, how much does coverage / mutation
   score move?" — the direct analogue of the practitioner's anecdote — with a precision figure from manual triage
   (`scripts/backtest-triage.py`), not just flag counts.
4. **Behavior change**: at least one shipped check or report that flags ineffective tests with a precision the
   owner accepts after triage, wired per the repo's "default check catalog" rule if it needs no repo-specific
   setup, else opt-in.
5. **Adoption leading indicator (SM5, measurable after ship, not a gate)**: the *flagged-then-revised rate*:
   of tests an AI agent adds that receive a `test-no-assertion` / `test-tautology` / `test-mock-only` finding in
   hook mode, the share whose next edit in the same session strengthens the test (adds a non-weak assertion)
   versus adds a `kibitzer:ignore`, removes the test (reported separately with and without a named covering
   test in the agent's reply, because deleting the just-added test is the cheapest way to silence a finding),
   or leaves the finding. Reported with the ignore-directive rate and
   reason diversity. The transcript backtest (plan Story 1.2.1, Task 1.2.1c) gives a pre-ship baseline; after
   G1 the hook log gives the live figure. A rate that never moves is evidence the advisory is being ignored,
   which the research track alone cannot show. The pre-ship baseline is computed on transcripts in which
   agents never saw the finding, so it is the natural strengthening rate, a floor to compare the live
   figure against, not a measurement of the advisory's effect. The target and the decision rule for what
   value holds a rule opt-in or demotes a promoted one are owner inputs recorded at G0 Part A (template
   line A8); this document proposes no rate. **Fallbacks (iteration 3):** if A8 is left unset, promoted rules
   keep their G1 verdict and SM5 is reported descriptively only (no demotion fires). Whatever A8 says, a
   post-G1 demotion rule may not fire before the live sample holds at least 30 flagged-then-observed
   agent-added tests for that rule (the same per-rule minimum A5 uses for triage; the owner may raise it).
   When the pre-ship baseline is "not estimable" (denominator below 10, Task 1.2.1c), the live rate is
   reported as a raw rate with no comparison, the demotion rule uses only the ignore-directive rate and
   reason diversity, and the adoption assumption stays UNVERIFIED.

### How SM3 and SM4 are read, and where their numbers live

- **SM4 has two distinct "met" outcomes and they are not the same success.** (i) *Shipped validated check*:
  a check or report whose precision cleared the owner's floor on triage and, for dynamic signals, whose held-out
  hypothesis (plan H2) was supported. This is a product success. (ii) *Shipped descriptive report on a null
  result*: the report ships labeled "descriptive" with the negative results documented (plan Story 0.4.1
  outcome table). That satisfies the letter of SM4 and is an **informative negative: a success of the research
  track, not of the product**. Status reports must say which of the two occurred; outcome (ii) is never
  described as "behavior change delivered".
- **Numeric thresholds are not in this file.** The SM3 and SM4 numbers (H1 equivalence margins, H2 rank-p and
  null counts, H3 and H4 Wilson floors, sample sizes, budgets) are recorded in `decisions/G0-decision.md`
  Part B by the owner before any experiment runs. The proposed defaults, which the owner approves or edits,
  are cited from `implementation/plan.md` ("G0 decision template" and hypotheses H1-H4). `G0-decision.md`
  does not exist yet; this file does not invent its contents.

## Size

Large (3–15M CU) — informational estimate, INFERRED: literature review is research-agent heavy (cheap per agent,
many agents); implementation spans checker design plus multi-language corpus runs which are the expensive part.
Full scope is the default; not a gate.

**Wall-clock blockers**: running test suites and mutation tools across the backtest corpus (kubernetes, cassandra,
servo, ripgrep, deno, vscode, stapler-squad) is slow and needs cloned repos + toolchains (owner: tstapler; device/CI
runs). Which signals to implement vs. leave as research (owner decision after Phase 2). None others.

## Why now, and where this sits on the roadmap

Only claims sourced from the repo:

- `docs/check-ideas.md` ("Evidenced candidates") records **none** after triaging 69 correction-phrase matches
  across two mining passes; its "Untested ideas" list is explicitly "not findings" and says not to implement on
  its strength alone. So no evidenced candidate outranks this one on the transcript-occurrence bar, and this
  project does not itself meet that bar yet: Story 1.2.1 measures occurrences of assertion-free and tautological
  tests in transcripts, and the constraint below allows documenting why a corpus-only signal is justified.
- The justification offered is owner-initiated plus literature: the anecdote (unsourced) and the research
  documents in `research/`. That is weaker than a confirmed occurrence and is stated as such.
- Positioning shift: this extends kibitzer from per-file static advisories to ingesting execution data
  (ADR-001) behind a hidden, experimental subcommand (ADR-002, Risk Control). The shift is deliberately gated
  (G0, G1, G2) so that it can end as "descriptive report plus documented negative result".

## Constraints

- Kibitzer is **advisory and diff-aware**, not a test runner; any signal needing test execution must say who runs
  the tests and how results get in (ingest a report vs. shell out) — this is the central architectural tension.
- Must follow `CLAUDE.md` "Writing a new check": backtest against transcripts (`kibitzer check backtest`) *and* the
  public-repo corpus before a checker is "done"; meet the "confirmed transcript occurrence" bar in
  `docs/check-ideas.md` or document why a corpus-only signal is justified.
- Multi-language reality: kibitzer supports Go/TS/JS/Java/Kotlin/Python/Rust-adjacent checks; per-language
  coverage/mutation tooling differs widely (go-mutesting, Stryker, PIT, mutmut, cargo-mutants).
- Rust, existing codebase conventions; no new heavy runtime dependency without justification.

## Non-functional Requirements

- **Performance SLO**: static signals must fit hook-mode latency (same budget as existing per-file checkers);
  dynamic signals (coverage/mutation) are batch/CI only and must never run in the hook path.
- **Scalability**: must tolerate suites with 10^4–10^5 tests (kubernetes-scale) without per-test full re-runs;
  mutation signals must state sampling/budget strategy.
- **Security classification**: internal (executes repo test code only when explicitly invoked).
- **Data residency**: no special requirements; no code leaves the machine (no LLM call in the mechanical path).

## Scope

### In Scope

- Deep literature review (academic + industry): test-suite effectiveness vs coverage, mutation testing
  (incl. Google/Facebook industrial reports, pseudo-tested methods / extreme mutation, mutant subsumption,
  predictive mutation testing), test-suite minimization and redundancy, assertion quality / oracle problem,
  test smells and their empirical validity, flaky tests, test-to-code traceability, LLM-as-judge for test quality.
- Inventory of existing tools and what each emits (per-test coverage, mutant-kill matrices, test-smell detectors)
  and whether output is machine-ingestable.
- Catalog of mechanical signals with the classification in Success Metric 2, including the two the owner named:
  **incremental (unique) coverage contributed by a test** and **number/uniqueness of mutants a test kills**.
- Design of how such signals integrate with kibitzer (new check family vs. batch report like
  `hotspots.rs`/`change_coupling.rs`), including how an LLM pass could consume the mechanical ranking.
- Implementation of the selected signal(s) after the owner's go/no-go at the end of research.

### Out of Scope

- Automatically deleting or rewriting tests (advisory only — consistent with kibitzer's advisory stance).
- Building kibitzer's own mutation engine (integrate with / ingest existing tools).
- Generating new tests.
- Judging integration/E2E/UI test effectiveness beyond what the same signals incidentally cover (unit tests first).

## Rabbit Holes

- **Per-test coverage collection** is not a standard output of most coverage tools (Go `-coverprofile` is per run;
  per-test requires a run per test or specialized tooling) — cost may dominate everything else.
- **Equivalent mutants** and mutant subsumption: raw "mutants killed" over-credits tests that kill trivial mutants;
  deduplicating requires theory + tooling.
- **Redundancy ≠ worthless**: tests redundant on coverage/mutants may be intentional regression pins, spec
  documentation, or guard different failure messages — a precision ceiling for any mechanical signal.
- **Static-only proxies** (no assertions, tautological asserts, mock-only verification) are cheap and hook-friendly
  but their validity vs. real fault detection is weakly evidenced — must be measured, not assumed.
- **Flakiness and test order dependence** corrupt per-test coverage/mutation attribution.
- **Language breadth**: a signal that works for Go may have no usable tooling in Kotlin; choose a primary language
  for implementation or accept a layered rollout.

## Alternatives Considered

- Pure LLM judgment per test (the anecdote) — high recall, no reproducibility, cost scales with suite size.
- Coverage-diff only ("delete test, re-run, did coverage move?") — cheap, but known to be a weak
  effectiveness proxy; useful as a floor signal and for the corpus experiment.
- Full mutation testing — strongest validated signal, highest cost; likely CI-batch/sampled only.
- Off-the-shelf test-smell detectors (tsDetect, etc.) — evaluate for reuse vs. porting rules.
- Hybrid: mechanical signals produce a ranked shortlist; LLM adjudicates the shortlist.

## Feasibility Risks

- Per-test attribution may require invasive per-language instrumentation kibitzer cannot own.
- Mutation tooling availability/quality is uneven across the corpus languages.
- Results on a "deleting a quarter of tests" claim may not replicate; the anecdote is unsourced — treat as a
  hypothesis to test, not a target.
- Precision may be too low for a default-on check; may land opt-in/report-only.
- **Adoption assumption (untested)**: that a finding carrying evidence and a non-claim changes what a developer
  or agent does. Redundancy can be intentional (Rabbit Holes: "Redundancy is not worthless"), so users and
  agents may learn to ignore the advisory or answer it with a filler `kibitzer:ignore` reason or a weak
  assertion. Mitigations are in the plan (Goodhart measurements in Story 1.2.1, G1's drop of the pre-filled
  directive, non-specific-reason counts in the report); SM5 is the check. Until SM5 is measured, and also
  when the baseline is "not estimable" or the live sample is below the minimum, the adoption assumption
  stays UNVERIFIED and no promotion or demotion is justified by it.

## Observability Requirements

Each emitted finding must carry: signal name, raw measured value (e.g. unique lines covered, mutants uniquely
killed), the baseline it was compared against, and tool/version that produced the input data, so triage and
false-positive reporting (`report_false_positive`, `docs/reporting-false-positives.md`) can reproduce it.
Corpus runs record flagged count, triaged precision, and coverage/mutation-score delta on deletion.

## Risk Control

Advisory-only and opt-in until backtested precision is accepted by the owner; dynamic signals gated behind an
explicit subcommand/config (never in `default_checks()` hook path). Rollback = remove from config / don't ship the
checker; no data migration. Check name registered per `docs/suppressing-checks.md` so it can be disabled/scoped.

## Open Questions

*Phase 2 status*: Q1 and Q6 answered by research (ingest-only; validate against a held-out fault proxy plus a size-matched random-deletion baseline). Q2 answered negatively (no per-test signal is validated in the literature). Q3, Q4, Q5 *(unresolved after Phase 2 research)* — carried into plan.md.

1. Where do dynamic signals live — a `kibitzer` subcommand that shells out to the project's test/coverage/mutation
   runner, or an ingest-only model consuming reports produced by CI (lcov, Go cover profiles, Stryker/PIT reports)?
2. What is the best validated predictor of "test is removable without losing fault detection" in the literature,
   and at what cost?
3. Does unique-coverage-per-test agree with unique-mutants-killed-per-test closely enough that the cheaper one
   suffices? (Testable on the corpus.)
4. Which languages get first-class support in the first implementation?
5. Can static assertion-quality signals be made precise enough to be a hook-time default check, per the
   `default_checks()` rule?
6. How should "deleted tests" be validated in the corpus experiment — coverage delta, mutation-score delta, or
   historical-bug re-detection?
