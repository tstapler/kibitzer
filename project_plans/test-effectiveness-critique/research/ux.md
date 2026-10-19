# UX research: presenting test-effectiveness findings

Phase 2 (UX) worker output. Labels: VERIFIED = I opened the source this session and the claim is in it; INFERRED = my reasoning or prior knowledge, not checked here.

Sources opened:
- Petrović, Ivanković, Fraser, Just, "Practical Mutation Testing at Scale" (Google): https://arxiv.org/html/2102.11378v2 (abstract page https://arxiv.org/abs/2102.11378; PDF unreadable via fetch)
- Petrović et al., "Does mutation testing improve testing practices?" (ICSE 2021): https://arxiv.org/html/2103.07189 (abstract https://arxiv.org/abs/2103.07189)
- Codecov PR comments: https://docs.codecov.com/docs/pull-request-comments
- PIT basic concepts: https://pitest.org/quickstart/basic_concepts/ ; PIT Maven: https://pitest.org/quickstart/maven/
- Stryker mutation-testing-elements repo README: https://github.com/stryker-mutator/mutation-testing-elements (only a one-line description came back; UI features NOT verified)
- Repo docs: `docs/output-formats.md`, `docs/accepting-findings.md`, requirements.md.

Not opened (fetch failed): Sonar rule pages (rules.sonarsource.com unreachable), Stryker mutant-states page (404), Coveralls. Statements about those are INFERRED.

---

## 1. How comparable tools present this, and what drove acceptance

### Google: surviving mutants in code review (VERIFIED, arxiv 2102.11378v2 and 2103.07189)

| Choice | Detail | Evidence of effect |
|---|---|---|
| Diff-scoped | Mutants only on changed, covered lines | Findings are actionable by the author of the change |
| Hard volume cap | One mutant per line; at most 7 x number of files per changelist, picked at random from survivors | Median 7 mutants per changelist vs 820 for traditional mutagenesis |
| Arid-node suppression | Heuristics suppress mutants on logging, monitoring, time, flag config, `flush`/`close`, etc. | Productive rate 80% to 89% as rules were added; paper says without them "users would become frustrated with non-actionable feedback and opt out of the system altogether" |
| Two-click feedback | Each finding has "Please fix" (reviewer asks author to act) and "Not useful" | Feedback exists for only 66,798 of 2,110,489 surfaced mutants (~3%), yet that was enough to drive rule tuning |
| Shown inside review as one finding type next to coverage | Not a separate score/dashboard | Authors credit "targeted mutation and careful surfacing of findings" for adoption |
| Coverage first, mutants second | Coverage "a far simpler test goal ... to act on" | Developers act on coverage first |

Outcomes: 82% of mutants with feedback labeled productive overall; Java 87.2%, Python 70.6%; operator variance (ROR 84.1%, UOI 74.5%). Over ~6 years, exposure to mutants correlated with more test hunks (r_s=0.9), lower mutant survival (r_s=-0.50) and fewer "Please fix" requests (r_s=-0.34), with coverage change flat, which the authors read as developers writing tests to kill mutants (not to raise coverage). 70% (1,043 of 1,502) of high-priority bugs were coupled to a live mutant. The paper reports anecdotal pushback on low-quality tests written only to kill mutants and "does not directly test which presentation choices caused the behavior changes."

Gaps (VERIFIED absent from text I read): no developer survey, no measured "fraction of surfaced mutants that got a test."

Takeaways for kibitzer:
- Acceptance was driven by precision control and volume capping, not by report richness.
- The only verified action Google asked of developers was "add/strengthen a test". Nothing I found shows any deployed system asking developers to delete a test from a dynamic signal. Test-removal findings are therefore a UX case with no acceptance evidence behind it (INFERRED conclusion from absence in the sources opened; I did not search for deletion-oriented tools).
- Unsound suppression heuristics are acknowledged by the authors as possibly hiding productive findings. Precision-over-recall is the accepted trade.

### Codecov PR comment (VERIFIED, docs.codecov.com)
- Header shows patch (diff) coverage and delta; body lists files; `hide_project_coverage: true` shows only diff coverage and missing lines.
- `require_changes: "uncovered_patch"` posts a comment only when the patch has uncovered lines; `coverage_drop` only on a drop. So "say nothing unless there is something" is a first-class option.
- Comment is updated in place, new comments (and email) only on first post.
- Takeaway: diff-scoped, silent-when-clean, one updated artifact. (INFERRED: this is why teams keep it enabled.) Codecov acceptance data not verified.

### PIT (VERIFIED partial, pitest.org)
- Outcome vocabulary: Killed / Survived / No coverage / Non viable / Timed out / Memory error / Run error.
- Defines test strength = killed / (killed + survived), excluding no-coverage mutants; build can fail on `testStrengthThreshold`. Output formats HTML (default), XML, CSV.
- Takeaway: an explicit denominator definition ("excluding no-coverage") and a machine-readable XML/CSV beside the human HTML are the pattern. PIT HTML layout (package > class > line-annotated source) is INFERRED from prior knowledge, not verified here.

### Stryker / mutation-testing-elements (UNVERIFIED UI)
README confirms only "a schema for mutation testing results with web components to visualise it". From prior knowledge (INFERRED): file-tree with per-file score, per-mutant status filter, source view with mutant markers, thresholds high/low/break, and a JSON schema that carries `killedBy`/`coveredBy` test ids. Relevant mainly because that schema is an ingest format kibitzer could read; that belongs to the tools-inventory worker.

### Sonar test rules (INFERRED, not opened)
Rules such as "tests should include assertions" are per-test static smell flags with a line pointer, severity and remediation text; known false-positive complaint pattern is custom assertion helpers (INFERRED). Takeaway: static "no assertion" findings need a configurable assertion-helper vocabulary, or they become noise.

---

## 2. Actionable output for "this test is removable/vacuous"

### Distinguish three finding classes (different claims, different wording)
1. **Vacuous (static)**: no assertion, tautology (`assert true`, `x == x`), mock-only verification. Claim is about the test itself. High confidence achievable at hook time. Action: strengthen or remove.
2. **Redundant (dynamic, per-test coverage)**: the test's covered lines are a subset of other tests' coverage. Claim is relative. Medium confidence; regression pins and spec-as-documentation legitimately look redundant (requirements.md Rabbit Holes).
3. **Mutant-silent (dynamic, mutation)**: kills no mutant that another test does not also kill. Strongest literature backing but needs a mutation run, equivalent-mutant noise applies.

Do not collapse these into one "ineffective test" label; the remediation and confidence differ.

### Evidence block per finding (satisfies requirements.md Observability Requirements)
```
src/foo_test.go:118: [test-redundant] TestParseEmpty covers 14 lines; 14/14 also covered by TestParse, TestParseTable
  signal: unique-coverage = 0 lines   baseline: 212 tests in package, run 2026-10-11, go1.23 cover profile (per-test via -run)
  confidence: medium (coverage-only; no mutation data)
  not a claim: this test may pin a regression or document behaviour - see "Before deleting" below
  dismiss: // kibitzer:ignore test-redundant -- pins issue #123
```
Required fields: rule id in `[brackets]` (the convention `accepting-findings.md` already requires so `kibitzer:ignore` and accepted entries can key on it), raw measured value, comparison baseline, which tests cover the same ground (name them; a reviewer can then diff them), data source and tool version, and a confidence tier.

### Confidence presentation
- Use a coarse 3-tier label (high/medium/low) derived from the signal class, not a fake-precise percentage. INFERRED rationale: until backtest precision is measured, a numeric probability would be a made-up number. Tier definitions should be tied to measured precision after triage (success metric 3).
- Say what the signal cannot see: "no mutation data", "ran with test order randomized: no/yes", "flaky-test attribution not checked".

### Dismissal
- Primary: inline `kibitzer:ignore <rule> -- <why>` above the test function (the repo's stated default; `accepting-findings.md` line 80). A reason is required there too, which doubles as the "why this test earns its keep" documentation.
- Fallback: `.kibitzer/accepted/*.json` keyed `(rule, file, line, content)`. VERIFIED limitation: it applies to native per-file checkers only and explicitly excludes whole-repo checks whose granularity is not a line. A suite-hygiene report that ingests coverage/mutation data is whole-repo and run-data dependent, so this mechanism does not obviously fit; the plan phase must either anchor findings to the test function's first line (so the existing key works) or extend the model. Flag as an open design question.
- Staleness guard: the `content` field re-surfaces the finding when the test body changes. This is desirable here because a test's redundancy depends on the rest of the suite, not just its own text. Counterpoint (INFERRED): the dynamic signal can change without the test line changing (other tests deleted), so an accepted dynamic finding should not silently re-emerge noisily; prefer ignoring quietly and only re-flagging when the test's own content changes.
- Wrong-signal path: `report_false_positive` (MCP) for "this finding is wrong" vs `kibitzer:ignore` for "correct but intentional" - same split `accepting-findings.md` already draws.

### Risk of nudging agents into deleting valuable tests
INFERRED, but concrete:
- Agents optimize toward making a check go quiet. A finding that says "removable" is satisfied by deletion, which is the cheapest way to make it disappear, and deletion also makes CI green when the test was failing. Google's anecdotal pushback on tests "written only to kill mutants" is the mirror image: gaming either direction exists (VERIFIED as anecdote only).
- Asymmetric costs: a wrongly kept test costs runtime; a wrongly deleted one silently loses a regression guard, and nothing will tell anyone.
- Mitigations to specify in design:
  1. Never phrase a finding as an instruction to delete. Phrase it as "this test adds no unique coverage/kills; decide: strengthen, merge, or remove with justification".
  2. For agents, make verification the first step and removal require stating which other test provides the guarantee.
  3. Suppress findings on tests the diff did not add or touch in agent hook mode (diff-aware): an agent should only be nudged about tests it just wrote, never prompted to prune existing human-written tests during an unrelated task. Suite-wide pruning is a human-initiated CLI report only.
  4. Never fire on a test that is currently failing or skipped (a failing test has no coverage signal worth acting on).
  5. Cap findings per run (Google's 7/file analogue) and rank by confidence.

---

## 3. Telling an AI agent how to act (hook / MCP)

Wording principles (INFERRED from the above plus kibitzer's existing advisory stance):

1. **Lead with the claim and the evidence, not the verb.** "This test executes `parse()` but contains no assertion" then the evidence line.
2. **Give a decision tree, with strengthening as the default branch for tests the agent just wrote.** For a vacuous test the agent authored: the likely fix is to add the missing assertion (the test intent exists, the oracle is missing). Delete only if the behaviour is already asserted elsewhere, and name where.
3. **Require verification before removal, in the message itself.** E.g. "Before removing: confirm the behaviour is asserted by <named test>; run the suite and compare." Provide the names of the covering tests so verification is one step.
4. **State the non-claim.** "A redundant test may be an intentional regression pin; if so, add `kibitzer:ignore test-redundant -- <reason>`." This gives the agent a legitimate quiet path that is not deletion, reducing deletion pressure.
5. **Separate "your test" from "existing test".** Wording for tests added in the current diff is directive ("strengthen this test"); for pre-existing tests it is informational and never an instruction (or not emitted in hook mode at all).
6. **Be uniform and short** - one headline line plus 1-3 evidence lines, same shape as the existing `file:line: [rule] message` convention so `scope_output_to_changed_lines` (VERIFIED in `output-formats.md` to key on `{file}:{line}: message`) keeps working. Note `output-formats.md` VERIFIES that diff scoping does not understand SARIF locations, so emit this via the native `Finding` path or the text convention, not through SARIF, if diff scoping is wanted.
7. **Don't use urgency or severity inflation.** Advisory tier only (requirements.md Risk Control).

Example hook text for an agent-added test:
```
tests/test_parse.py:41: [test-no-assertion] test_parse_empty calls parse() but makes no assertion on its result or side effects.
  Likely fix: assert on the return value or raised error. Remove only if another test already asserts this behaviour (then name it in your reply).
  Intentional? add `# kibitzer:ignore test-no-assertion -- <why>` above the def.
```

---

## 4. CLI report shape for suite hygiene runs

Dynamic signals are batch-only, never in the hook path (requirements.md NFR). Proposed subcommand output (shape only; names are placeholders):

```
kibitzer test-effectiveness ./...   (go, cover profiles from 212 per-test runs, 2026-10-11)

Suite: 212 tests | 31 flagged | est. runtime of flagged: 4.1s of 38.0s | coverage if all removed: 81.4% -> 81.4%
Rank  Test                         Signal            Value        Overlaps with             Conf    Cost(s)
1     TestParseEmpty               unique-coverage   0 lines      TestParse, TestParseTable  medium  0.01
2     TestFooNoAssert              no-assertion      static       -                          high    0.00
3     TestBarMock                  mock-only         static       -                          medium  0.20
...
Summary by signal: no-assertion 9 | mock-only 5 | zero-unique-coverage 14 | zero-unique-kills 3
Excluded (not assessed): 6 flaky/skipped, 2 failing
```
Design points:
- Sort by (confidence desc, then cost saved) so the top rows are the safest wins; show total potential saving once at top (this answers the owner's "what can I delete?" and is also the corpus-experiment metric: coverage/mutation delta on deletion).
- The "Overlaps with" column is the verification handle; without it the user must re-derive why.
- Print an explicit "Excluded / not assessed" line so absence of a flag is not read as endorsement (PIT excludes no-coverage mutants from strength for the same denominator-honesty reason; VERIFIED).
- `--format json` (and optionally SARIF) for CI/LLM adjudication; table is the default. Cap rows with `--top N`; full list in JSON. INFERRED convention, consistent with Google's cap logic (VERIFIED) applied to the human view.
- Group by file/package on request; a per-package roll-up beats a 10^4-row flat list on kubernetes-scale suites (requirements.md Scalability).
- Because the report is whole-repo, it should still honour `kibitzer:ignore` and `.kibitzer/inspect.json` scope so the same finding does not return after dismissal.

---

## 5. Job-to-be-done per consumer (requirements.md "Users / Consumers")

| Consumer | JTBD (INFERRED) | Right surface | Wording / posture | Failure mode to avoid |
|---|---|---|---|---|
| Developer/CI on a diff (`kibitzer run`) | "Tell me if the tests I'm adding in this PR don't actually test anything, before a reviewer has to." | Static signals only, on added/changed tests, diff-scoped; optional CI step that ingests coverage/mutation report for changed tests | Terse, `file:line: [rule]`, silent when clean (Codecov `uncovered_patch` analogue, VERIFIED) | Noise on tests untouched by the PR; opt-out like Google's described frustration |
| AI agent (hook / MCP `run_checks`) | "Tell me when the test I just wrote is vacuous so I fix it before claiming done." | Static signals, hook latency, only on tests in the edit | Directive for own tests: strengthen, with named alternatives; verification step; ignore comment for intentional | Pushing agent to delete or to write brittle assertions that merely silence the rule (assertion-padding) |
| Repo owner suite hygiene | "Give me a ranked list of what I can safely cut or merge, with proof, so I can approve a deletion PR." | Explicit CLI report from per-test coverage / mutation data, ranked table + JSON, runtime saved, coverage delta | Informational, evidence-heavy, names overlapping tests, includes "not assessed" | Presenting a ranked list as a delete list; no way to record "keep" decisions that survive the next run |

Cross-cutting: the "assertion-padding" risk (agent satisfies a no-assertion rule with `assert result is not None`) means static vacuity checks should also flag trivially weak assertions, or the finding just trains agents toward a cheaper vacuity. INFERRED; testable on the transcript backtest.

---

## Open UX questions for the planner
1. Anchor of a suite-level dynamic finding: first line of the test function (reuses `kibitzer:ignore` and `accepted/` keys) vs a new whole-repo dismissal path. Verified constraint: `accepted/` excludes whole-repo checks today.
2. Should hook mode ever emit findings on pre-existing tests? Recommendation: no.
3. Confidence tier thresholds - define only after corpus triage (success metric 3), do not guess.
4. Who owns the "assertion-helper vocabulary" config so custom helpers are not flagged (Sonar-style FP, INFERRED).
