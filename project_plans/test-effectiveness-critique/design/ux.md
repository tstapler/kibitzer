# UX design: test-effectiveness-critique

**Date**: 2026-10-11
**Inputs**: `requirements.md`, `research/ux.md`, `implementation/plan.md` (Stories 1.1.2-1.1.4, 2.1.3, 2.2.x, 2.3.1-2.3.4, 4.1.1, 4.2.1, 4.2.2).
**Revision**: triad repair iteration 1 (2026-10-11) applied UX blockers B1-B3 and gaps G2, G5-G12 from the triad UX lens (section 10b). Iteration 2 (2026-10-11) applies blocker B1 (the joint line) and gaps G1-G14 of the round-2 UX lens (section 10c). Iteration 3 (2026-10-11) applies round-3 blockers B1 (`section` field, `signal_mode` vocabulary) and B2 (per-class joint check, zero case) and the contract gaps (section 10d); section 9 now holds 68 criteria. Iteration 5 (round-5 repair, 2026-10-13) applies round-4 blocker B1 (`report_only` spelling) and gaps G1-G25 (section 10e, repair log at the end); section 9 now holds the count in its Total line.
**Labels**: items marked *(proposed)* are design choices not yet in the plan; items marked *(plan delta)* require a plan.md change before implementation. Thresholds and caps marked *(placeholder)* must be set from triage data (plan: no numeric tier boundary ships before triage).

## 0. Principles (apply to every surface)

1. **Advisory, never a gate.** Findings never change an exit status. Only input errors do (exit 2).
2. **Say the claim, the evidence, and the non-claim.** Every finding carries what was measured, against what, and what the signal cannot see.
3. **Never instruct deletion.** Wording is "strengthen, merge, or remove with justification". The quiet path is `kibitzer:ignore`, not deletion.
4. **Silence is not endorsement.** Reports print what was *not assessed*; hook mode stays quiet when clean (Codecov `uncovered_patch` analogue, `research/ux.md` section 1).
5. **Three finding classes, three wordings**: vacuous (static, about the test), redundant (relative, coverage), mutant-silent (relative, mutation). Never one "ineffective test" label.
6. **No color-only meaning, ASCII only, no emoji.** v1 emits no ANSI color at all, so `NO_COLOR` and non-TTY behavior are trivially satisfied and testable (UX-X2); if color is added later it must satisfy UX-X1/X2/X5 first.
7. **Summaries describe measurements, never savings.** No output (text, JSON, brief) states a summed runtime, a "safe to remove" count, or a joint figure phrased as the effect of deleting the flagged set. Runtime is per-row context only and never orders rows (triad repair B3). A figure that is zero by construction (the `JointSafe` set's joint loss, harness invariant I0) is never printed as a finding: it carries no information and reads as a green light (iteration 2, B1).
8. **One contract for every consumer.** Everything that lowers confidence (warnings), identifies provenance (header), counts what was done (summary) or states the mode (banner) is in the text body and in the JSON envelope. A consumer that reads only stdout, or only JSON, loses nothing (triad repair B1).

## 1. Surface inventory

| # | Surface | Consumer | Interactive? | Treatment |
|---|---|---|---|---|
| S1 | Hook / MCP `run_checks` finding text (static rules) | AI agent, developer | Agent reads and acts | Condensed + decision tree (section 2) |
| S2 | `kibitzer tests report` text output | Repo owner | Yes (types command, reads, decides) | Full: wireframe, flow, error table (section 3) |
| S3 | `--format json` and `--format brief` | CI, LLM/human adjudicator | No | Condensed (section 4) |
| S4 | Dismissal flows: inline ignore, `report_false_positive`, config scope | Developer, agent | Yes | Full: flows + error table (section 5) |
| S5 | Input-error / warning messages for the report | Repo owner | Yes | Full error table (section 3.4) |
| S6 | Enablement and config (`inspect.json` rules, `assertion_helpers`) | Developer | Config file | Condensed (section 6) |
| S7 | `--verbose` stderr diagnostics | Owner debugging | No | Condensed (section 7) |
| S8 | Recipe script output (`per-test-go-coverage.sh`) | Repo owner | Yes (runs script) | Condensed + errors (section 8) |

No GUI, web, or modal surfaces exist. "Click" criteria below are expressed as commands, flags, and edits.

## 2. S1: hook / MCP finding text (static rules)

### 2.1 Shape

One headline line in the repo-wide `file:line: [rule] message` convention (so `scope_output_to_changed_lines` keeps working; `docs/output-formats.md`), then exactly three indented lines when the rule has all three (likely fix, the non-claim, the quiet path); a rule may omit the likely-fix line only if no fix can be stated, never the non-claim or the quiet path.

Rules: `test-no-assertion`, `test-tautology`, `test-mock-only` (opt-in, medium).

```
tests/test_parse.py:41: [test-no-assertion] test_parse_empty calls parse() but no assertion call was found in its body or same-file helpers.
  Likely fix: assert on the return value or the raised error. Remove only if another test already asserts this behaviour (name it in your reply).
  Not claimed: that the test is worthless; it may only check that parse() does not raise.
  Intentional? add `# kibitzer:ignore test-no-assertion -- <why>` above the def and any decorators. Finding wrong? report_false_positive.
```

```
pkg/parse_test.go:88: [test-tautology] assert.Equal(t, got, got) compares `got` to itself and cannot fail.
  Likely fix: compare against an expected value or a computed result.
  Not claimed: that the rest of the test is wrong; only this comparison cannot fail.
  Intentional? add `// kibitzer:ignore test-tautology -- <why>` above the func. Finding wrong? report_false_positive.
```

```
src/OrderServiceTest.java:57: [test-mock-only] testCreate's only check is verify(repo).save(x); no return value or state is asserted. (confidence: medium - the interaction may be the behaviour)
  Likely fix: also assert on what create() returns or stores, or confirm the interaction is the contract.
  Not claimed: that the interaction is unimportant; whether it is the behaviour is your call.
  Intentional? add `// kibitzer:ignore test-mock-only -- <why>` above the method. Finding wrong? report_false_positive.
```

Every sample carries all three of: likely fix, a non-claim line (what the signal cannot see), and the quiet-path line, so Principle 2 holds in hook mode; the non-claim is its own line, not folded into the headline. The quiet-path line also names `report_false_positive` for the case where the tool is wrong, so an agent is not steered only to `kibitzer:ignore` (which hides the finding from the tool owner; section 5.2) (round 5, G10, G12). A confidence tag appears inline only when it is below `high` (so `test-mock-only` carries it and the other two static rules do not); the rule is "print confidence when it is not high", stated once here so the three samples are consistent (iteration 2, G14).

### 2.1a Quiet-path variants (iteration 2, G6)

The quiet-path line is never removed from hook output. Goodhart measurement (a) of plan Story 1.2.1 (ignore-directive rate and reason diversity) can only change its *form*, and the form is chosen once, at G1:

| Variant | Third line | When |
|---|---|---|
| V1 (default) | `Intentional? add `// kibitzer:ignore <rule> -- <why>` above the func. Finding wrong? report_false_positive.` (a complete, copyable directive) | Before G1 and when measurement (a) is not high |
| V2 | `Intentional? a kibitzer:ignore directive naming the behaviour this test pins; see docs/suppressing-checks.md. Finding wrong? report_false_positive.` (a pointer, not a copyable line) | Only if `G1-decision.md` records measurement (a) as high |

Both variants keep the `Finding wrong? report_false_positive.` clause (round 5, G12). Either way the finding is at most 4 lines, still teaches the quiet path, and still never starts with "Delete" or "Remove". V2 is the answer to boilerplate reasons: it asks for a specific reason instead of handing over a ready-made filler line, without making dismissal undiscoverable.

### 2.1b MCP `run_checks` shape (iteration 2, G14)

`run_checks` returns the same text as hook mode: one string per finding, the headline and its indented lines joined by newlines, findings in file then line order. The per-file cap line (`  +N more test-quality findings in this file (run kibitzer run <file> for all)`) is appended once, at the end of that file's block, so a finding's lines are never interleaved with it. No JSON is returned by `run_checks` for these rules.

### 2.2 Agent decision tree (printed in `docs/test-quality.md`, summarised in the 3 lines above)

```
finding on a test YOU added, or whose signature line you changed, in this diff?
 |-- yes --> does the test have an intent you can state in one sentence?
 |            |-- yes --> add the missing assertion (default branch)
 |            `-- no  --> behaviour already asserted elsewhere?
 |                         |-- yes --> remove it AND name the covering test in your reply
 |                         `-- no  --> write the assertion that the behaviour needs
 `-- no (pre-existing test) --> not emitted in hook mode (diff-scoped); never prompt an agent to prune
```

Write-time limits, accepted and measured rather than hidden (iteration 2, G7): (a) the hook never inspects the reason of an existing `kibitzer:ignore` (a suppressed finding is silent, per H4), so a filler reason gets no feedback at write time; the non-specific-reason counter and footer (D-E6) exist only in the report, and SM5 reports reason diversity. (b) Removing the just-added test silences the finding cheaply; "name the covering test" is an instruction to the agent, not something the hook can verify. SM5 therefore counts removal as its own bucket (with or without a named covering test) so the metric shows how often the cheap exit is taken.

### 2.3 Acceptance criteria / edge cases

- Emitted only for tests the diff added or whose signature line it changed (the finding anchors on the function's first line, which must fall in a changed range); unchanged tests produce no output, and an edit that removes an assertion from the body of an existing test is covered only by conditional plan Story 4.1.2 (Story 1.1.2, ADR-004).
- Silent when clean: no "0 findings" line in hook mode.
- Failing or skipped tests (`t.Skip`, `@Disabled`, `xit`) are not flagged.
- Test file generated (`Code generated ... DO NOT EDIT`) produces nothing.
- Unsupported language (no `TestCodeProfile`, e.g. a `.cs` file): checker is not run; no output, no error. `kibitzer check list` shows which languages each rule supports.
- Per-run cap *(proposed, placeholder)*: after the G1 cap value is recorded, at most that many (placeholder 5) test-quality findings per file in hook mode; before G1 the effective cap is none (UX-H9); extra are replaced by one line `  +N more test-quality findings in this file (run kibitzer run <file> for all)`.
- Message never begins with "Delete" or "Remove"; never uses urgency words ("must", "critical").
- Quiet-path line: every hook finding carries V1 or V2 of section 2.1a; none is emitted without one (UX-H1, H6, H8).
- Before the cap value is recorded at G1, hook mode collapses nothing. The flood risk this leaves (an agent adding a large test file) exists only for repos that opted a rule in via `inspect.json`, because the rules are not in `default_checks()` before G1; the cap value is a G1 deliverable (plan Story 1.3.1).

## 3. S2: `kibitzer tests report`

### 3.1 Command shape

```
kibitzer tests report [INPUT FLAGS] [OUTPUT FLAGS]

Input (at least one required; combinable):
  --coveragepy <ctx.json>      coverage.py `coverage json --show-contexts`
  --go-profiles <dir>          directory of per-test cover profiles + manifest.json
  --lcov <file>                lcov tracefile (needs TN: names for per-test data)
  --stryker <report.json>      mutation-testing-elements report
  --pit <mutations.xml>        PIT report (needs fullMutationMatrix=true)
  --junit <report.xml>         status + runtime side data (not a primary input)
  --max-input-bytes N          refuse any single input file larger than N bytes (R26); the
                               default is the built-in ceiling from plan Task 2.1.5b; G0
                               Part B2 may only tighten it

Provenance (all optional; see plan "Attribution source"):
  --attribution isolated|in-order|leave-one-out
                               how per-test coverage was produced, when the input does not say
  --kill-completeness full|first-killer|sampled|budget-limited
                               how complete the mutation run was, when no recipe sidecar says
  --input-commit <sha>         commit the input was produced at, when the input does not carry it
  --leave-one-out <loo.json>   per-test removal results for a shortlist (plan Story 3.1.4)

Output:
  --format text|json|brief     default: text
  --top N                      default: 20 (text only; json/brief are complete). The visible
                               window holds at least floor(N / R) rows of each of the R rules
                               that have findings, then fills in ranking order (section 3.2)
  --subtests                   treat table rows/subtests as separate tests
  --by-package                 roll-up table per package; --top then means rows per package
                               (section 3.2c)
  --experimental               also show signals that fail the main-list rule below, in their
                               own section under the main list, each row tagged (experimental)
  --show-suppressed            list every finding a kibitzer:ignore directive suppressed, with
                               its reason (section 5.1a); json always carries suppressed[]
  --verbose                    diagnostics on stderr
```

(`--top` default, `--by-package`, `--show-suppressed` and the exit-2 contract are *(proposed)*; `--format`, `--top`, `--subtests`, `--verbose`, `--attribution`, `--input-commit`, `--junit`, `--experimental` follow the plan.)

**Main-list rule for `--experimental` (iteration 2, G3).** Before a G2 verdict exists no signal is "marked shippable", so the rule cannot be the verdict. It is structural and uses the plan's "Confidence caps" table: a signal is in the main list when its input quality does not force the structural `Low` cap (`FirstKillerOnly`, `Sampled` or `BudgetLimited` kill data, PIT without `fullMutationMatrix`, unknown producer schema version, unknown input commit); signals that the caps force to `Low` appear only with `--experimental`, in their own section. After G2 the rule becomes the verdict: a signal with a recorded non-removed verdict (`ship-with-tiers`, `ship-report-only`, `descriptive-only`) is in the main list and rendered in the mode of its verdict, so the null-result `descriptive` report lists its rows in the main section (iteration 3: this keeps the descriptive deliverable reachable without `--experimental`); a signal with no verdict, or one the structural caps still force to `Low` even with a verdict (round 5, G4: a verdict never lifts a structural cap), is experimental. One behaviour, stated once: a structurally capped signal's rows are never shown in the main list at `low`; they are in the experimental section only, so R8 and R9 do not print "shown as low" in the main list.

**Vocabulary (round 5, G14).** The word "experimental" names four different things; they are not interchangeable:

| Where | Meaning |
|---|---|
| `mode: experimental` / banner | No G2 verdict is recorded for any shown signal (a state of validation) |
| `signal_mode: experimental` (record) | This record's signal has no G2 verdict |
| `section: experimental` (record), "Experimental signals" section | This row is structurally capped at `low` and hidden unless `--experimental` is given (a state of input quality) |
| `--experimental` flag | Show the rows of `section: experimental` |

A record can be `section: main` with `signal_mode: experimental` (before G2, uncapped signals) but never `section: experimental` with a verdict-bearing `signal_mode` unless the caps force it (after G2, a capped signal). Without `--experimental` the experimental section is absent and the summary prints one line `experimental signals not shown: test-zero-unique-kills 3 (use --experimental)`, so nothing is silently dropped.

### 3.2 Wireframe (text format, 80 columns, ASCII)

The report has four modes, each printing a banner on the first output line and carried in JSON as `mode` and `banner` (section 4): `experimental` (before G2, the plan's hidden command), `descriptive` (G2 verdict `descriptive-only`), `report_only` (`ship-report-only`), and `tiered` (`ship-with-tiers`). The wireframe below is the `experimental` mode with tiers, which is also the row layout of `tiered`; the differences per mode, including the four banner texts and what validation each points to, are in section 3.2a (banner prefix = the JSON `mode` value plus a colon, so `report_only:`), and short wireframes of the `report_only` and `tiered` heads are in section 3.2b. The main list holds 20 rows (the `--top 20` window of 28 findings: 9 `test-no-assertion`, 5 `test-mock-only` and 6 of 14 `test-zero-unique-coverage`, per the stratified window below), and the rows the wireframe skips are marked with bracketed lines that are not report output.

```
experimental: signals not yet validated (see docs/test-effectiveness.md)
kibitzer tests report
  input:    coveragepy 7.6.1 | commit 9f2c1ab | run 2026-10-11 | attribution: isolated
            pit 1.15.3 | commit 9f2c1ab | run 2026-10-11 | kill data: first killer only
  scope:    assessed 197 of 212 tests in the input (15 not assessed)
  note:     41 test functions in this repo are not in the input (partial run?); not assessed

warning: 3 test files changed since the report commit (9f2c1ab..HEAD; comment-only edits ignored); their tests are excluded as stale.
warning: full mutation matrix not enabled (only killingTest present); zero-unique-kills capped at low.

Listed in order of confidence, then rule id, then test id. Order is not a removal priority; see "Before acting".
 #  Location and rule
 1  pkg/foo_test.go:40: [test-no-assertion] TestFooNoAssert
      alone: static | conf: high | cost: 0.00s
      overlaps: -
    [rows 2-9: eight more high-confidence test-no-assertion rows; not reproduced in this wireframe]
10  pkg/bar_test.go:75: [test-mock-only] TestBarMock
      alone: static | conf: medium | cost: 0.20s
      overlaps: -
    [rows 11-14: four more test-mock-only rows; not reproduced]
15  pkg/parse_test.go:118: [test-zero-unique-coverage] TestParseEmpty
      alone: 0 unique lines | conf: medium | cost: 0.01s
      overlaps: TestParse, TestParseTable*
    [rows 16-20: five more test-zero-unique-coverage rows; not reproduced]
    ... 8 more: test-zero-unique-coverage 8 (use --top 0 or --format json for all)

Experimental signals (shown because of --experimental; not ranked with, or counted in, the list above)
  test-zero-unique-kills: 3 flagged, 1 shown
 1  pkg/zed_test.go:12: [test-zero-unique-kills] TestZed (experimental)
      alone: 0 unique mutants | conf: low | cost: -
      overlaps: TestZedAlt
    ... 2 more  (experimental section: its own window of N rows, stratified like the main list)

Summary
  flagged by rule (main list):
    test-no-assertion 9 | test-mock-only 5 | test-zero-unique-coverage 14
  tests flagged individually: 28 | of these, 3 are redundant only with other flagged tests
    (their `overlaps` line ends `(all flagged)`; a name marked * is one flagged partner)
  joint coverage check, coverage redundancy only (the 14 test-zero-unique-coverage findings of the main list;
  static rows are not part of it):
    3 lines are covered by these tests and by no test outside the set
    (14 of 14 had coverage data; per-row "alone" values are not additive; computed on the
     data the set was chosen from; this says nothing about fault detection)
  suppressed by kibitzer:ignore: 2 | dismissals with non-specific reasons: 1   (use --show-suppressed to list)

Not assessed (15): stale 9 | inputs disagree 2 | failing 2 | skipped 1 | no-data 1   (use --verbose to list)
Stability not assessed: single run
Absence of a flag does not mean a test is valuable.

Before acting on a row
  1. Open the tests named under "overlaps" and confirm they assert the same behaviour.
  2. A name marked * is also flagged. If a group of flagged tests overlaps, keep at least
     one of them; "alone" is per test and does not hold once its partner is gone.
  3. A redundant test may be a regression pin or spec; if so keep it:
       kibitzer:ignore <rule> -- <why>      (correct but intentional)
     If the finding itself is wrong, use report_false_positive (the tool is wrong).
  4. Re-run the suite after any change.

Legend
  alone    what only this test covers or kills; "0 unique lines" = no line only this test runs
  conf     high | medium | low, from signal class and input quality; not a probability
  cost     runtime from JUnit input, for context; it never affects order; "-" = no timing input
  *        overlap partner that is also flagged in this report
  inputs disagree
           unique coverage differed between two inputs of one suite; cause not determined
  stale    the test's file, or a production file it covers, changed since the input commit
           (comment-only edits do not count)
```

Not-assessed reasons print in this fixed order, nonzero ones only: `stale`, `no-source`, `inputs disagree`, `failing`, `skipped`, `generated`, `no-data`. Their sum equals the K of the `scope:` line (UX-R21); a zero total prints `Not assessed (0)`.

Row field definitions (printed by `kibitzer tests report --help`):

| Column | Meaning |
|---|---|
| Location and rule | `path:line: [rule] TestName` in the repo-wide `file:line: [rule]` convention, so the anchor is editor-clickable, `kibitzer:ignore` goes on the line above it, and `report_false_positive` has the file and line it needs. The path, line and rule id are never truncated; a long test name is printed in full and may wrap (UX-R15) |
| alone | The measured value for this test on its own (`0 unique lines`, `0 unique mutants`, `static`) |
| conf | `high` / `medium` / `low`; derived from signal class and input quality, not a probability; structural caps in the plan's "Confidence caps" (in-order attribution and no kill dimension: medium at most) |
| cost | Measured runtime from JUnit input; `-` when no timing input. Context only; it is never a sort key and no total is printed (Principle 7) |
| overlaps | Up to 2 test names that cover the same ground, then `+N`; full list in json. A name followed by `*` is itself flagged in this report. When every partner is flagged the line ends `(all flagged)`; that tag is what the summary's "redundant only with other flagged tests" count counts, so the count can be verified from the rows (round 5, G16) |

The second and third lines of each row are indented, so a 25-character rule id plus a path fits 80 columns for typical paths; 80 columns is a layout target, not a truncation rule.

Ranking key (replaces the former confidence-then-runtime order): confidence tier (`High` first), then rule id ascending, then test id ascending; the wireframe rows follow it: all nine `high` `test-no-assertion` rows come first (rows 1-9), then the `medium` rows by rule id, `test-mock-only` (rows 10-14) before `test-zero-unique-coverage` (rows 15-20), which is why the wireframe's second printed row is number 10. Runtime was removed from the key because sorting expensive tests first is, in effect, a delete order, which Principle 3 forbids (triad repair B3).

**Visible window under `--top N` (iteration 2, G9).** The key decides order, but with a flat cut a large static class (sorted first by rule id) could crowd every dynamic finding out of the window. So the window is filled in two steps: each of the R rules that has findings first receives `min(count, floor(N / R))` rows, taken in key order within the rule; the remaining slots go to the next rows in global key order; the chosen rows are then printed in global key order. The omitted line names what was cut per rule (`... 8 more: test-zero-unique-coverage 8`). This is deterministic (UX-R9) and `--top 0` / json show everything. The experimental section has its own window with the same rule and the same N (the two windows are not shared or split), and its heading states `<flagged> flagged, <shown> shown` (round 5, G17).

**Joint coverage check (iteration 2, B1; iteration 3, round-3 B2).** One check per rule class, never one number over a mix. Static rules (`test-no-assertion`, `test-mock-only`) are claims about a test body and are not part of any joint check, because a vacuous test often does cover lines, and folding its lines into a redundancy figure would make one number mean two things. The coverage-redundancy class (`test-zero-unique-coverage`) and the kill-redundancy class (`test-zero-unique-kills`) each get their own line, computed over that class's `IndividuallyFlagged` set of the main list (experimental section excluded), counting units held by at least one set member and by no test outside the set; a class with several `unit_kind` values (`line`, `mutant_site`, `mutant`) gets one line per kind, and the unit kind is named in the sentence. It is the number that is usually nonzero when flagged tests are redundant with each other, which is why it is worth printing: it shows the per-row `alone` values are not additive. It is never computed over the `JointSafe` subset in the report, because that subset has zero joint loss by construction (harness invariant I0). Tests with no data are excluded from the set and counted in the parenthesis.

**Zero case.** A measured zero (every member is covered by some test outside the set) is a real value, not a constructed one, but printed bare it reads as a green light for removing the whole set, which Principle 7 forbids. A zero is therefore never printed without its attached non-claim, on the line below: `0 lines are covered by these tests and by no test outside the set` then `This is a measurement on the data the set was chosen from; it does not show removal is safe (regression pins, untested units and failure messages are not seen).` JSON carries the sentence as `zero_note`. When a class has no findings its line is absent (not printed as zero). **No data (round 5, G3).** When `with_data` is 0 (no member of the set had coverage or kill data), no count is computed: the line reads `no coverage data for these tests; no joint check computed`, and json has `units_held_only_by_set: null`, `with_data: 0`, `zero_note: null`. A zero that is really "nothing measured" is never printed as `0 lines`.

**`redundant_only_with_flagged`.** The count of individually flagged tests whose `overlaps_with` is non-empty and whose every member also appears in `overlaps_also_flagged`. A test with at least one non-flagged partner is not counted, because its covered ground survives.

### 3.2a Mode matrix (descriptive-only and other modes)

Which parts of the output depend on the mode. `descriptive` is the null-result deliverable (plan Story 0.4.1 outcome table and Story 4.2.1), so it is specified in full.

| Output element | experimental | descriptive | report_only | tiered |
|---|---|---|---|---|
| First line banner | `experimental: signals not yet validated (see docs/test-effectiveness.md)` | `descriptive: signals not validated as removal predictors; see docs/test-effectiveness.md` | `report_only: signals measured, ordering not validated as removal advice; backing: docs/test-effectiveness.md#results` | `tiered: confidence tiers come from triaged precision on <N> arms; backing: docs/test-effectiveness.md#results` |
| Header, scope, warnings above table | yes | yes | yes | yes |
| `conf` per row, confidence-ordered list | yes | **no** (column and ranking line absent) | no | yes |
| Row order | tier, rule id, test id | test id ascending, stated in the list heading | test id ascending | tier, rule id, test id |
| "redundant only with" count and `*` partner marks | yes ("of these, N are redundant only with another flagged test") | **no** count and no "redundant only with" wording; `overlaps` and `*` kept as plain facts | no count; `*` kept | yes |
| `joint coverage check` lines (one per rule class and unit kind, over that class's `IndividuallyFlagged` set, section 3.2; never the `JointSafe` subset; a zero carries its non-claim) | yes | yes, over the listed tests of the class, "listed" wording (it is a measurement) | yes | yes |
| Runtime sum, "estimated runtime" | never | never | never | never |
| `cost` per row | yes | yes | yes | yes |
| Before acting block, Legend, Not assessed, non-endorsement sentence | yes | yes (step 2 and the word "flagged" are replaced by "listed"; the sentence reads "Absence from this list does not mean a test is valuable."; `non_claims[0]` carries the same variant) | same "listed" variant as descriptive | yes ("Absence of a flag ...") |
| JSON `confidence` key per record | present | **omitted** | omitted | present (omitted for every record whenever the top-level `mode` is `descriptive` or `report_only`, even a record whose `signal_mode` is `tiered`) |
| JSON `mode` | `experimental` | `descriptive` | `report_only` | `tiered` |
| JSON `summary` count keys | `individually_flagged`, `redundant_only_with_flagged` | `listed` (no redundant-only key) | `listed` (no redundant-only key) | `individually_flagged`, `redundant_only_with_flagged` |

**`descriptive` vs `report_only` (round 5, G15).** The layout is identical; they differ in banner, `mode` string and what they claim. `descriptive` is the null-result deliverable (the G2 verdict `descriptive-only`: the signals are not shown to predict anything; wording stays "listed" and no ranking claim). `report_only` is the verdict `ship-report-only`: the signals were measured and a precision figure exists, but the ordering is not validated as removal advice, so the banner points to the results anchor. Neither prints `conf`; if a later change makes them identical the two verdicts should be merged in the plan, not distinguished by the renderer.

Descriptive-mode wireframe:

```
descriptive: signals not validated as removal predictors; see docs/test-effectiveness.md
kibitzer tests report
  input:    coveragepy 7.6.1 | commit 9f2c1ab | run 2026-10-11 | attribution: isolated
  scope:    assessed 197 of 212 tests in the input (15 not assessed)

warning: 3 test files changed since the report commit (9f2c1ab..HEAD; comment-only edits ignored); their tests are excluded as stale.

Tests with a measured value, in test-id order (not a ranking, not a removal list)
 #  Location and rule
 1  pkg/bar_test.go:75: [test-mock-only] TestBarMock
      alone: static | cost: 0.20s
      overlaps: -
 2  pkg/foo_test.go:40: [test-no-assertion] TestFooNoAssert
      alone: static | cost: 0.00s
      overlaps: -
 3  pkg/parse_test.go:118: [test-zero-unique-coverage] TestParseEmpty
      alone: 0 unique lines | cost: 0.01s
      overlaps: TestParse, TestParseTable*
    [rows 4-20 not reproduced]
    ... 8 more: test-zero-unique-coverage 8 (use --top 0 or --format json for all)

Summary
  listed in this report, by rule: test-no-assertion 9 | test-mock-only 5 | test-zero-unique-coverage 14
  (28 tests; the rows above are the first 20 in the display window, "listed" means in the full list)
  joint coverage check, coverage redundancy only (the 14 listed test-zero-unique-coverage tests):
    3 lines are covered by these tests and by no test outside the set
    (14 of 14 had coverage data; per-row "alone" values are not additive; computed on the
     data the set was chosen from; this says nothing about fault detection)
  suppressed by kibitzer:ignore: 2 | dismissals with non-specific reasons: 1   (use --show-suppressed to list)

Not assessed (15): stale 9 | inputs disagree 2 | failing 2 | skipped 1 | no-data 1   (use --verbose to list)
Stability not assessed: single run
Absence from this list does not mean a test is valuable.

Before acting on a row
  1. Open the tests named under "overlaps" and confirm they assert the same behaviour.
  2. If a group of listed tests overlaps, keep at least one of them; "alone" is per
     test and does not hold once its partner is gone.
  3. A redundant test may be a regression pin or spec; if so keep it:
       kibitzer:ignore <rule> -- <why>      (correct but intentional)
     If the finding itself is wrong, use report_false_positive (the tool is wrong).
  4. Re-run the suite after any change.

Legend
  alone    what only this test covers or kills; "0 unique lines" = no line only this test runs
  cost     runtime from JUnit input, for context; "-" = no timing input
  *        overlap partner that is also listed in this report
  inputs disagree
           unique coverage differed between two inputs of one suite; cause not determined
  stale    the test's file, or a production file it covers, changed since the input commit
           (comment-only edits do not count)
```

### 3.2b Heads of `report_only` and `tiered` output (iteration 2, G11)

Only the first lines differ from the wireframe in 3.2; the banner names the mode and points at the validation that backs it, so the two modes are distinguishable without inferring from a missing `conf` column.

```
report_only: signals measured, ordering not validated as removal advice; backing: docs/test-effectiveness.md#results
kibitzer tests report
  input:    coveragepy 7.6.1 | commit 9f2c1ab | run 2026-10-11 | attribution: isolated
  scope:    assessed 197 of 212 tests in the input (15 not assessed)
...
Tests with a measured value, in test-id order (not a removal list)
 #  Location and rule
 1  pkg/bar_test.go:75: [test-mock-only] TestBarMock
      alone: static | cost: 0.20s
```

```
tiered: confidence tiers come from triaged precision on 2 arms; backing: docs/test-effectiveness.md#results
kibitzer tests report
...
Listed in order of confidence, then rule id, then test id. Order is not a removal priority; see "Before acting".
 #  Location and rule
 1  pkg/foo_test.go:40: [test-no-assertion] TestFooNoAssert
      alone: static | conf: high | cost: 0.00s
```

The `<N> arms` count and the anchor text are filled from `G2-decision.md` at build time (plan Story 4.2.1); the anchor must exist in `docs/test-effectiveness.md` (UX-R13 includes a check).

### 3.2c `--by-package` roll-up (iteration 2, G9)

`--by-package` replaces the flat list with one summary row per package, then the flagged rows of each package, capped per package. In this view `--top N` means rows per package (default 5 *(placeholder, set with the R22 threshold at G0)*), not a global cut; the flat `--top 20` default does not apply, so the two never conflict. Packages sort by flagged count descending, then package path. For languages without packages the package is the test file's directory. JSON stays complete and gains a `packages[]` array only under this flag.

```
 package        assessed  flagged  no-assertion  mock-only  zero-unique-coverage
 pkg/parse            40        6             2          1                     3
 pkg/foo              12        2             2          0                     0
 ... 14 more packages with 0 flagged (use --format json for all)

pkg/parse (6 flagged; showing 5)
 1  pkg/parse/foo_test.go:40: [test-no-assertion] TestFooNoAssert
      alone: static | conf: high | cost: 0.00s
      overlaps: -
    ... 4 more rows, then "... 1 more (use --top 0 for all)"
```

R22's auto-switch message quotes this per-package cap (section 3.4).

### 3.3 Interaction flow (repo owner, suite hygiene)

| Step | User does | System responds |
|---|---|---|
| 1 | Produces input with a recipe (`docs/test-effectiveness-report.md`) | Recipe prints the output path and the exact next command |
| 2 | Runs `kibitzer tests report --coveragepy ctx.json` | Prints mode banner, header (tool, version, commit, attribution), warnings, listed rows, summary, "Not assessed", "Before acting" block, legend. Exit 0. For an input that takes noticeable time to read, a `[tests] reading <file> (<size>)` progress line goes to stderr when stderr is a TTY (UX-E9) |
| 3 | Reads top rows; opens the overlapping tests | (none: names are in the row) |
| 4 | Decides per row: keep, strengthen, merge, remove | Keep: add `kibitzer:ignore` (flow D1). Wrong signal: `report_false_positive` (flow D2). Change: edit test and re-run report |
| 5 | Re-runs the report after edits, on the same input | Suppressed count rises; the dismissed test and its file-mates stay assessed, because a comment-only edit (including adding a `kibitzer:ignore` line) does not mark a file stale (section 3.4, staleness detail). A removed test disappears from the table; its row in the input then shows under `Not assessed: no-source` until the input is regenerated |
| 6 | Needs machine-readable | Re-runs with `--format json` or `--format brief` (S3) |

Exit paths: Ctrl-C at any point (report is read-only, writes nothing); `--help` available at every step; `--format json` for any user who needs to leave the text view.

Steps to first useful output: given an existing input file, 1 command. Given no input file, follow the recipe: 2 commands (recipe, then report).

Discovery (before G2): `Command::Tests` is hidden, so `kibitzer --help` does not list it. The entry points are `docs/test-effectiveness-report.md` (linked from `README.md`) and the experimental banner's pointer to `docs/test-effectiveness.md`; after a G2 unhide it appears in `--help`. This is deliberate (the plan's hidden-until-G2 risk control), not a gap.

CI flow (S3 consumer; no interactive step):

| Step | CI does | System responds |
|---|---|---|
| 1 | Runs the producer, then `kibitzer tests report ... --format json > report.json` | Exit 0 for any produced report, exit 2 only for class E; on exit 2 stdout holds the error object of section 4, so `report.json` is still valid JSON (UX-E13); a consumer tests for a top-level `error` key before reading anything else (a document with `error` is not a report). Exception: R19 (invalid `--format`) leaves stdout empty |
| 2 | Archives `report.json` | The file is self-describing: `mode`, `header`, `warnings[]`, `summary`, `findings[]`, `non_claims` (section 4) |
| 3 | Compares two archived reports | Key on `(rule, test.id)`, where `test.id` is `<repo-relative path>::<qualified name>` and so unique across packages; `header.inputs[].commit` and `summary` explain differences; deterministic order makes text diffs stable |
| 4 | Decides how to treat degradation | `warnings[]` also carries class I notices (R13, R15, R17, R22, R27), so it being non-empty means nothing by itself. A consumer treats the report as degraded when any `warnings[]` entry has `class == "W"` (equivalently `lowers.effect != "informational"`), or when `mode` is not `tiered`. The report never fails the build; a CI script that blocks on it does so by its own choice |

### 3.4 Empty, partial, stale, and error states

State classes: **(E)** error, exit 2, no table; **(W)** warning, exit 0, table printed with degraded confidence; **(I)** informational.

| # | Condition | Class | What the user sees | Exit path offered |
|---|---|---|---|---|
| R1 | No input flag given | E | `error: no input given. Supply at least one of --coveragepy, --go-profiles, --lcov, --stryker, --pit.` + link to recipes doc | Prints the recipes pointer and usage line |
| R2 | Input path does not exist | E | `error: --coveragepy ctx.json: file not found (cwd: /repo).` | Hint: `check the path; recipes: docs/test-effectiveness-report.md` |
| R3 | Input unreadable / not valid JSON/XML | E | `error: ctx.json: not valid JSON (line 1, column 1). Is this a coverage.py JSON report?` | Names the producing command (`coverage json --show-contexts`) |
| R4 | Valid JSON but wrong tool/shape | E | `error: ctx.json does not look like coverage.py output (no "meta" or "files" key). Detected: Stryker report. Use --stryker.` | Suggests the matching flag when sniffable |
| R5 | Missing tool name or version (`MissingProvenance`) | E | `error: ctx.json has no coverage.py version in meta; findings need tool and version to be reproducible. Re-run the producer with coverage.py >= <floor>.` where `<floor>` is the lowest version in the adapter's fixture allow-list (plan Story 2.2.1), filled in at runtime | Version floor comes from the allow-list, not from this document |
| R6 | coverage.py report has no test contexts | E | `error: ctx.json contains 0 test contexts. Set dynamic_context = test_function in .coveragerc and re-run.` + the two-line config | Prints exact config snippet |
| R7 | lcov with only unnamed `TN:` | W | `warning: lcov has no per-test names (TN: empty); per-test signals not available from this input. Whole-suite figures only.` Per-test rows replaced by `Not assessed: no-data N` | Suggests one-tracefile-per-test recipe |
| R8 | PIT report without full matrix (detected from the file) | W | `warning: full mutation matrix not enabled (only killingTest present); zero-unique-kills capped at low.` + the `fullMutationMatrix` setting | Kill rows are listed only in the experimental section (`--experimental`), not in the main list (section 3.1) |
| R9 | Kill data incomplete: Stryker `killedBy` all single-valued (bail on first kill), or a mutation run the recipe sidecar records as sampled or stopped at its budget (section 8) | W | `warning: kill sets look truncated (bail on first kill); unique kills over-credit tests; zero-unique-kills capped at low.` For a sampled or budget-limited run: `warning: mutation run covered 412 of 1930 mutants (stopped at budget); a test with no unique kills here may uniquely kill an unrun mutant; zero-unique-kills capped at low.` A user-supplied `--kill-completeness first-killer` fires this id (`kill-data-incomplete`, params `source: flag`), never R8, because R8 is reserved for a PIT file whose own structure lacks the full matrix (round 5, G25). | Suggests disabling bail, or raising the budget / re-running unsampled; the rows are experimental-section only |
| R10 | Report commit is an ancestor of HEAD and files changed since (comment-only edits are not changes) | W | `warning: report was produced at 9f2c1ab, HEAD is a41e7d0; N test files changed since (comment-only edits ignored). Their tests are excluded as stale.` | Suggests re-running the recipe |
| R11 | Report commit unknown (provenance lacks it), or an `--input-commit` value or input-carried commit is not a hex object id | W | `warning: report has no commit; cannot check staleness. Treat results as unverified against the current tree.` For a malformed value: `warning: --input-commit "<value>" is not a hex object id; ignored. ...` (`params.reason: not_a_hex_object_id`; the value is never passed to git) | none needed |
| R12 | Test ids in input not found in repo (renamed/deleted) | W | `warning: 14 test ids in the input have no source file here; listed under "Not assessed: no-source". They cannot be suppressed inline (they are not findings, so there is nothing to dismiss).` | `refresh the input so ids match this tree; see section 5.4` |
| R13 | Two inputs disagree on the same test (unique-coverage differs) | I | Test moves to `Not assessed: inputs disagree` (iteration 2, G10: the label states the observation, not a cause such as flakiness). The Legend says what it means ("unique coverage differed between two inputs of one suite; cause not determined"); test ids are listed under `--verbose`. The internal reason name stays `Flaky`; the printed label and the JSON reason value are `inputs_disagree` | `--verbose` lists them |
| R14 | Attribution is `in-order` rather than isolated, whether stated or assumed | W | `warning: attribution is in-order; unique-coverage can over-state redundancy when tests share setup. Treat as medium at most.` When the value is the adapter default rather than stated: `warning: attribution is in-order (assumed; pass --attribution to state it); ...`. Both carry the stable id `attribution-in-order`; the assumed case sets `params.assumed: true` and `header.inputs[].attribution_assumed: true` | Names isolated recipe |
| R15 | Language has no adapter (e.g. Kotlin dynamic) | I | Header shows `languages with dynamic data: python, go`; for a Kotlin input path `note: no adapter for Kotlin test files; static rules only (run kibitzer run).` | Points to static rules |
| R16 | Partial data: some tests lack coverage or kill data | W | `scope:` header line in the one standard form `assessed 180 of 212 tests in the input (32 not assessed)`, with every reason (stale, no-source, inputs disagree, failing, skipped, generated, no-data) on the "Not assessed" line so its parts sum to the 32; flagged rows still listed | `--verbose` lists them |
| R17 | Zero tests flagged, at least one assessed | I | `note: no tests flagged. assessed 206 of 212 tests in the input (6 not assessed).` (`note: no tests listed.` in `descriptive` and `report_only`). The full footer still prints: Not assessed line, Stability line, the non-endorsement sentence, Legend, and the `experimental signals not shown` line when applicable; the Before-acting block is omitted (no rows). If `not_assessed` is more than half of the input a second line adds `most tests were not assessed; a clean result covers only the assessed part.` Exit 0 | none needed |
| R18 | Zero tests in input | E | `error: input contains no tests; nothing to assess.` | Points to producer |
| R19 | Unknown format value (`--format xml`) | E | `error: unknown --format "xml"; expected text, json, or brief.` | Prints valid values |
| R20 | `--top` not a non-negative integer | E | `error: --top expects a non-negative integer; got "-3".` | `--top 0` means all |
| R21 | Mix of per-test and whole-suite inputs only (e.g. only `--stryker` suite-level) | W | `warning: no per-test coverage supplied; only mutation signals computed.` | none |
| R22 | Very large suite (>= threshold *(placeholder)*) | I | `note: suite has 41,000 tests; showing the first 5 per package in key order (not a priority order; --by-package; --top then counts per package). Full list: --format json.` The auto-switch changes the text view only: `json` and `brief` are unchanged by it (complete, no `packages[]`; `packages[]` appears only when `--by-package` is given explicitly), and exit status and stdout shape for scripts are the same (round 5, G20). | `--by-package` / `--format json` |
| R23 | Working tree has uncommitted changes to test or covered files | W | `warning: working tree differs from commit a41e7d0 in 2 test files (uncommitted); their tests are excluded as stale.` | `commit or stash, regenerate the input, re-run` |
| R24 | Report commit is not an ancestor of HEAD (other branch, rebase) | W | `warning: report commit 9f2c1ab is not an ancestor of HEAD a41e7d0; staleness is computed from the tree difference between them and may over-exclude.` | `regenerate the input on this branch` |
| R25 | Not a git repository | W | `warning: not a git repository; staleness not assessed. Treat results as unverified against the current tree.` | none needed |
| R27 | Zero tests assessed (every test stale, no-data, failing, skipped or otherwise excluded), input non-empty | I | `note: nothing could be assessed. assessed 0 of 212 tests in the input (212 not assessed).` followed by the Not assessed breakdown. Never worded as a clean result: "no tests flagged" is not printed. Static findings, if any, are still listed (R17 vs R27 is decided by `assessed == 0`) | Names the largest reason's fix (regenerate the input for `stale`) |
| R26 | An input file exceeds `--max-input-bytes` | E | `error: ctx.json is 3.2 GB, over the 1.0 GB input cap (--max-input-bytes).` | `raise --max-input-bytes or split the input; see docs/test-effectiveness-report.md` |

Rules for every message:
- The first line names the flag or file involved; the second line (when present) states the fix. Every class W line starts with the literal `warning:`, every class I line with `note:` and every class E line with `error:` (UX-X1, one prefix convention everywhere, including the wireframes).
- Exit 2 for class E; class E prints no table. With `--format text` it writes to stderr only and stdout is empty; with `--format json` or `brief` stdout holds the minimal error object of section 4 (so a CI job that captures stdout never gets an empty file) and the same message is on stderr as one `error:` line. R19 (invalid `--format`) cannot choose a format, so under it stdout is empty even when the user passed `--format xml` meaning json; a CI job that needs a JSON document on every exit-2 path must pass a valid `--format` value (round 5, G5).
- **Stream contract (triad repair B1/G2).** Class W and I messages are part of the report, so they are always printed in the stdout body above the table (text) or in `warnings[]` of the envelope (json/brief). With `--format json` or `brief` each warning is additionally copied to stderr as one `warning:` line, so an interactive user redirecting stdout still sees it; in `--format text` there is no stderr copy, so a terminal does not show each warning twice. A consumer that captures only stdout therefore never loses a warning, and `report > out.txt` keeps them.
- Every W line states which signal's confidence it lowered. In JSON this is `lowers`; in text the renderer appends ` (lowers: <signal>)` mechanically to any W line whose message does not itself name the signal (R10, R11, R12, R23, R24, R25 and R14 get the suffix), so text and JSON agree without hand-editing each row (round 5, G13).
- Prefix convention (round 5, G13): class W lines start `warning:`, class I lines start `note:`, class E lines start `error:`. The stderr copy under `--format json` / `brief` applies to W and I lines alike.
- The numbers in examples (counts, thresholds, the 1.0 GB cap) are illustrative; real values come from the input or from the G0-recorded parameter.
- **Exit codes (iteration 3).** 0: a report was produced. 2: class E, an input or usage condition. 1: an internal error or an IO failure writing the output (for example stdout closed); stderr carries one line `error: internal: <what>`, stdout is not promised to be valid JSON, and the code is never used for a malformed input (those are always 0 or 2).
- **Line wrapping (iteration 3).** 80 columns is the layout target for rows. The first-line banner is one unwrapped line. Every other non-row line (warnings, summary, scope, notes, hints) is soft-wrapped at 100 columns on a word boundary with a 2-space continuation indent; a path, rule id, sha or anchor is never split, so a line holding one long token may exceed 100.

**Precedence when staleness messages combine (iteration 3).** Staleness is computed once, against the input commit and the working tree together. Each stale file is counted in exactly one message: R23 when its working copy differs from HEAD (an uncommitted edit wins), otherwise R10 (the input commit is an ancestor and the file differs between it and HEAD) or R24 (it is not an ancestor). R10 and R24 are mutually exclusive by definition; R23 may accompany either. A test is `Stale` once however many messages name its files, and the scope line counts tests, not files.

**Static rules and staleness (round 5, G18).** Staleness only excludes a test from the dynamic rules (`test-zero-unique-*`), whose input is old. Static rules (`test-no-assertion`, `test-tautology`, `test-mock-only`) are computed from the working tree and are never excluded: a stale test with a static finding is listed, and the same test is still counted under `Not assessed: stale` for the dynamic signals. After an owner edits a test the static finding therefore disappears immediately when fixed, and the test stays in `Not assessed: stale` until the input is regenerated; the Not assessed line says so (`stale: regenerate the input to re-assess`), and `--verbose` lists the ids.

Staleness detail *(plan delta; extended in triad repair, G8)*: the compare is `git diff --name-only <input-commit> HEAD` for R10 and R24 and `git status --porcelain` for R23; a clean tree at the report commit produces none of R10, R23, R24, R25. The `ExclusionReason::Stale` variant covers all three. Add `ExclusionReason::Stale` to the plan's `ExclusionReason` enum (currently `Flaky`, `Failing`, `Skipped`, `Generated`, `NoData`) and `NoSource`; both are printed in "Not assessed".

**Comment-only exemption (iteration 2, G5).** A file that the name-only diff lists is stale only if its content differs once comments and whitespace are removed: the report compares the file's token stream at the input commit (`git show <sha>:<path>`, read-only) with the working copy, dropping comment nodes through the language's tree-sitter grammar. Adding, editing or removing a `kibitzer:ignore` line is therefore never a staleness event, so dismissing a finding does not take the dismissed test and its file-mates out of assessment. When no grammar is available for the file, or the old content cannot be read, the file counts as changed (conservative). R10, R23 and R24 name the count after the exemption.

**Stable ids (iteration 2, G8).** The `R<n>` labels are design-document row numbers and are not part of any contract. Every message has a stable string `id` used in JSON `warnings[].id` and `error.id`:

| R | id | R | id | R | id |
|---|---|---|---|---|---|
| R1 | `no-input` | R10 | `stale-input-commit` | R19 | `unknown-format` |
| R2 | `input-not-found` | R11 | `input-commit-unknown` | R20 | `bad-top` |
| R3 | `input-unparseable` | R12 | `ids-without-source` | R21 | `no-per-test-coverage` |
| R4 | `input-wrong-shape` | R13 | `inputs-disagree` | R22 | `large-suite-by-package` |
| R5 | `missing-provenance` | R14 | `attribution-in-order` | R23 | `dirty-working-tree` |
| R6 | `no-test-contexts` | R15 | `no-adapter-for-language` | R24 | `input-commit-not-ancestor` |
| R7 | `lcov-no-test-names` | R16 | `partial-data` | R25 | `not-a-git-repo` |
| R8 | `pit-no-full-matrix` | R17 | `nothing-flagged` | R26 | `input-over-size-cap` |
| R9 | `kill-data-incomplete` | R18 | `no-tests-in-input` | R27 | `nothing-assessed` |

### 3.5 Acceptance criteria (S2)

See section 9, IDs UX-R1..UX-R12.

## 4. S3: `--format json` and `--format brief` (condensed)

**The envelope below is the contract that Story 2.3.1 freezes as `schema_version` 1; it is specified here, before that freeze, so that nothing which lowers confidence is left to stderr (triad repair B1).**

```json
{
  "schema_version": 1,
  "format": "json",
  "mode": "experimental",
  "banner": "experimental: signals not yet validated (see docs/test-effectiveness.md)",
  "advisory": true,
  "non_claims": [
    "Absence of a flag does not mean a test is valuable.",
    "A flagged test may be a regression pin, a spec, or guard a distinct failure message.",
    "Per-test values are alone-values; they do not hold once an overlapping flagged test is gone.",
    "The joint coverage check is computed on the data the set was chosen from and says nothing about fault detection."
  ],
  "header": {
    "kibitzer_version": "0.0.0",
    "head_commit": "a41e7d0",
    "inputs": [
      {"flag": "--coveragepy", "tool": "coverage.py", "tool_version": "7.6.1",
       "commit": "9f2c1ab", "run": "2026-10-11",
       "attribution": "isolated", "attribution_assumed": false,
       "kill_completeness": null, "mutation_run": null, "leave_one_out": null},
      {"flag": "--pit", "tool": "pit", "tool_version": "1.15.3",
       "commit": "9f2c1ab", "run": "2026-10-11",
       "attribution": null, "attribution_assumed": false,
       "kill_completeness": "first_killer_only",
       "mutation_run": {"scope": "full", "mutants_in_report": 1930,
                        "mutants_total": null, "sampled_fraction": null,
                        "budget_seconds": null, "budget_exhausted": null}}
    ],
    "scope": {"tests_in_input": 212, "assessed": 197, "not_assessed": 15,
              "by_reason": {"stale": 9, "no_source": 0, "inputs_disagree": 2,
                            "failing": 2, "skipped": 1, "generated": 0, "no_data": 1},
              "repo_tests_not_in_input": 41}
  },
  "warnings": [
    {"id": "stale-input-commit", "class": "W",
     "lowers": {"signal": "coverage_derived", "effect": "excluded", "tests": 9, "files": 3},
     "params": {"input_commit": "9f2c1ab", "head_commit": "a41e7d0", "files_changed": 3},
     "message": "report was produced at 9f2c1ab, HEAD is a41e7d0; 3 test files changed since (comment-only edits ignored). Their tests are excluded as stale."},
    {"id": "pit-no-full-matrix", "class": "W",
     "lowers": {"signal": "kill_derived", "effect": "capped_low", "tests": null, "files": null},
     "params": {"input": "--pit"},
     "message": "full mutation matrix not enabled (only killingTest present); zero-unique-kills capped at low."}
  ],
  "summary": {
    "flagged_by_rule": {"test-no-assertion": 9, "test-mock-only": 5,
                        "test-zero-unique-coverage": 14},
    "individually_flagged": 28,
    "redundant_only_with_flagged": 3,
    "joint_check": [
      {"class": "coverage_redundancy", "rules": ["test-zero-unique-coverage"], "unit_kind": "line",
       "set": "individually_flagged", "set_size": 14, "with_data": 14,
       "units_held_only_by_set": 3, "zero_note": null,
       "basis": "selection data; not evidence of fault detection"}
    ],
    "experimental": {"shown": true, "flagged_by_rule": {"test-zero-unique-kills": 3}},
    "suppressed": {"total": 2, "non_specific_reason": 1, "malformed_directives": 0},
    "stability": "single_run"
  },
  "findings": [
    {
      "rule": "test-no-assertion",
      "section": "main",
      "signal_mode": "experimental",
      "input": null,
      "test": {"id": "pkg/foo_test.go::TestFooNoAssert", "file": "pkg/foo_test.go", "line": 40},
      "value": {"kind": "static",
                "evidence": "no assertion call found in body or same-file helpers"},
      "baseline": "static check over the test body",
      "attribution": null,
      "tool": "kibitzer", "tool_version": "0.0.0", "commit": "a41e7d0",
      "confidence": "high",
      "cost_seconds": 0.0,
      "overlaps_with": [],
      "overlaps_also_flagged": [],
      "leave_one_out": null
    },
    {
      "rule": "test-zero-unique-coverage",
      "section": "main",
      "signal_mode": "experimental",
      "input": 0,
      "test": {"id": "pkg/parse_test.go::TestParseEmpty", "file": "pkg/parse_test.go", "line": 118},
      "value": {"kind": "unique_coverage", "unique_units": 0, "covered_units": 14,
                "unit_kind": "line"},
      "baseline": "197 assessed tests; per-test coverage contexts",
      "attribution": "isolated",
      "tool": "coverage.py", "tool_version": "7.6.1", "commit": "9f2c1ab",
      "confidence": "medium",
      "cost_seconds": 0.01,
      "overlaps_with": ["pkg/parse_test.go::TestParse", "pkg/parse_test.go::TestParseTable"],
      "overlaps_also_flagged": ["pkg/parse_test.go::TestParseTable"],
      "leave_one_out": null
    },
    {
      "rule": "test-zero-unique-kills",
      "section": "experimental",
      "signal_mode": "experimental",
      "input": 1,
      "test": {"id": "pkg/zed_test.go::TestZed", "file": "pkg/zed_test.go", "line": 12},
      "value": {"kind": "unique_kills", "unique_mutants": 0, "killed_mutants": 7,
                "unit_kind": "mutant"},
      "baseline": "1930 mutants in report; first killer only",
      "attribution": null,
      "tool": "pit", "tool_version": "1.15.3", "commit": "9f2c1ab",
      "confidence": "low",
      "cost_seconds": null,
      "overlaps_with": ["pkg/zed_test.go::TestZedAlt"],
      "overlaps_also_flagged": [],
      "leave_one_out": null
    }
  ],
  "not_assessed": [{"test": {"id": "pkg/x_test.go::TestX", "file": "pkg/x_test.go", "line": 9},
                    "reason": "inputs_disagree"}],
  "suppressed": [{"rule": "test-zero-unique-coverage",
                  "test": {"id": "pkg/y_test.go::TestY", "file": "pkg/y_test.go", "line": 30},
                  "reason_text": "pins issue #123", "non_specific": false}]
}
```

Error object (class E with `--format json` or `brief`, iteration 2, G8): `{"schema_version": 1, "format": "json", "error": {"id": "no-input", "message": "no input given. Supply at least one of ...", "hint": "recipes: docs/test-effectiveness-report.md"}}`, exit 2, nothing else on stdout. `id` comes from the table in section 3.4. An invalid `--format` value (R19) cannot choose a format and stays stderr-only. **Discriminator (round 5, G5):** the error object has `error` and none of `mode`, `banner`, `advisory`, `non_claims`, `header`, `warnings`, `summary`; a consumer tests for a top-level `error` key first, and a document without it is a report carrying the full envelope (UX-R14 applies to reports only).

**Schema evolution policy (round 5, G6).** Within `schema_version` 1: new top-level or per-record keys, new `warnings[].id` values, new rule ids, new `lowers.signal` / `lowers.effect` / `value.kind` / `reason` values and new `header.inputs[].tool` values are additive and consumers must tolerate them (ignore unknown keys, treat an unknown enum value as the most conservative reading: unknown `lowers.effect` as `capped_low`). The version bumps to 2 only for: removing or renaming a key, changing a key's type or the meaning of an existing enum value, changing the `test.id` grammar, or changing the key-order guarantee. The freeze test (Story 2.3.1) checks that a fixture written against 1.0 still parses when unknown keys are injected.

Key order is exactly as shown for the top level and for records (`schema_version` first, then `format`; `findings`, `not_assessed` and `suppressed` last); a `joint_check` entry's key order is `class, rules, unit_kind, set, set_size, with_data, units_held_only_by_set, zero_note, basis`, as in the example (round 5, G8). The example is illustrative of a mixed repo: record `input` is the index into `header.inputs` (null for static rules), so two inputs from one tool stay distinguishable (round 5, G7). Field notes:

- `mode`, `banner`: the mode and its first-line text (section 3.2a). Two per-record fields, never interchangeable (iteration 3): `section` is `main` or `experimental` and is the only field that says which list a row belongs to, so a consumer counting main-list rows filters on `section == "main"`; `signal_mode` is the verdict state of the record's signal in the same four words as the top-level `mode`: `experimental` (no G2 verdict recorded; every row before G2, in both sections), then `descriptive`, `report_only`, `tiered` after G2 (the verdicts `descriptive-only`, `ship-report-only`, `ship-with-tiers` respectively). Top-level `mode` is the least-validated `signal_mode` among `section == "main"` rows, ordered `experimental` < `descriptive` < `report_only` < `tiered`; with no main rows it is `experimental` before G2 and the single recorded verdict after. The render path follows the top-level mode for every row.
- `advisory` is always `true` and `non_claims` is always non-empty, so a consumer that reads only JSON still sees the non-endorsement statement (triad repair G10).
- `test.id` grammar: `<repo-relative path>::<qualified name>`, so the `(rule, test.id)` key cannot collide across packages. Qualified name per language: Go `TestName` (subtests `TestName/sub` only under `--subtests`), Python `TestClass::test_name` or `test_name`, Java and Kotlin `ClassName#method`, TS/JS the `describe` chain joined with ` > ` plus the test title, Rust `module::fn_name`. A test with no unique source file keeps the raw input id as `test.id` with `file` and `line` null and appears only in `not_assessed`.
- `value.kind` is the discriminant: `unique_coverage` (`unique_units`, `covered_units`, `unit_kind` = `line` or `mutant_site`), `unique_kills` (`unique_mutants`, `killed_mutants`, `unit_kind` = `mutant`), `static` (`evidence`, one sentence naming what was and was not found). Rendering `alone:` text is derived from this object only.
- `warnings[]` carries every class W and I message of section 3.4, including the ones that also print in the text body. `id` is the stable slug of the section 3.4 table. `lowers` is a structured object: `signal` is one of `coverage_derived`, `kill_derived`, `static`, `all`; `effect` is one of `excluded`, `capped_low`, `capped_medium`, `informational`; `tests` and `files` are counts or null. `params` holds the values the message interpolates (shas, counts) so a script never parses `message`. Class E conditions produce the error object shown above, not a report.
- `header.inputs[].mutation_run` (kill inputs only; null otherwise) records how complete the mutation run was: `scope` is `full`, `sampled`, `budget_limited` or `unknown`; the other fields are copied from the recipe sidecar of section 8 and are null when the input does not say. `kill_completeness` is `full`, `first_killer_only`, `sampled` or `budget_limited`; any value other than `full` caps `test-zero-unique-kills` at `low` (R9), so a partial run never reads as complete.
- `header.scope.repo_tests_not_in_input` is the count of located test functions in the repo that are absent from the input (a partial suite run), or null when no repo scan was possible; they are outside `tests_in_input`, `assessed` and `not_assessed`, and the text report prints a `note:` line when the count is above zero.
- Enum spelling (iteration 3): CLI flags are hyphenated, JSON values are snake_case, text output echoes the flag spelling. `--attribution isolated|in-order|leave-one-out` maps to `isolated|in_order|leave_one_out`; `--kill-completeness full|first-killer|sampled|budget-limited` maps to `full|first_killer_only|sampled|budget_limited`. `warnings[].id` slugs are hyphenated, being identifiers, not enum values.
- `test.id` disambiguation (round 5, G22): when the qualified name repeats within one file (a redefined Python function, two TS `describe` chains with the same title, a parametrized `test[param]` where the raw parameter id is kept in brackets), later occurrences get a `#2`, `#3` suffix in source order, so `(rule, test.id)` is unique within a report. `overlaps_with` and `overlaps_also_flagged` hold full `test.id` strings (`<path>::<name>`), so a partner in another package or file is unambiguous; the text row prints bare names and appends the path to a name only when two partners in the row share it. `overlaps_with` lists every other test that covers a unit this test covers (coverage rules) or kills a mutant it kills (kill rules); static rules have `[]`.
- `summary.joint_check` is an array (section 3.2): one entry per rule class and `unit_kind` that has findings, each `{class, rules, unit_kind, set, set_size, with_data, units_held_only_by_set, basis, zero_note}`; `class` is `coverage_redundancy` or `kill_redundancy`, `set` is `individually_flagged` (or `listed` in `descriptive` and `report_only`), `zero_note` is the non-claim sentence when the value is 0 and null otherwise. Static rules never appear. `[]` when no dynamic rule flagged anything.
- `leave_one_out` (record) is null unless `--leave-one-out` supplied a result: `{"result": "no_loss"|"units_lost"|"not_run", "units_lost": N|null, "unit_kind": ...}`; `header.inputs[].leave_one_out` is null or `{"file", "matched", "unmatched"}`. A malformed `loo.json` is class E R3 or R4 naming `--leave-one-out`; unmatched ids appear only with `--verbose`.
- `summary.suppressed.malformed_directives` counts `kibitzer:ignore` directives above flagged tests that lack a reason and so did not apply; the affected record carries `ignore_note` (a string, key absent when there is no malformed directive) and the text row a `note:` line (UX-D9). `record_non_claims` (brief only) is an array of strings drawn from one fixed per-rule list kept next to `RULES` in code, so the vocabulary is closed and testable.
- `summary.experimental` exists only when `--experimental` was given; the main-list `flagged_by_rule`, `individually_flagged` and `joint_check` never include experimental rows.
- `suppressed[]` is always present (empty when nothing was suppressed) so a CI consumer can audit dismissals; text shows it only with `--show-suppressed` (section 5.1a).
- `summary` has no runtime total and no "estimated savings" field; `cost_seconds` is per-record context only (Principle 7).
- In `descriptive` and `report_only` mode the `confidence` key is omitted from every record, the `summary.individually_flagged` key is named `listed`, `summary.redundant_only_with_flagged` is omitted, and each `summary.joint_check[].set` reads `listed`.
- Unassessed tests appear only in the top-level `not_assessed` array; a finding record has no `not_assessed` field. The field is `rule` everywhere (plan and this document).

`brief` is the same JSON document with `"format": "brief"` and `schema_version` 1: the full envelope (`mode`, `banner`, `advisory`, `non_claims`, `header`, `warnings`, `summary`) is present exactly as in `json` (UX-R14), and the only difference is two extra keys per `findings[]` record: `record_non_claims` (named differently from the envelope's top-level `non_claims`; e.g. `"may pin a regression"`) and `verify_first` (named covering tests), for an external LLM or human adjudicator. `brief` is therefore a superset of `json`, not a smaller document; the name means "adjudication brief". The source location is already `test.file`/`test.line`, and the former constant `"action": "review"` carried no information, so neither is added (round 5, G9). The envelope is not repeated or abridged. kibitzer makes no LLM call.

Acceptance bullets:
- Every record has `rule`, `section`, `signal_mode`, `test`, `value`, `baseline`, `attribution`, `tool`, `tool_version`, `commit`, `confidence` (omitted in `descriptive`), `overlaps_with`, `overlaps_also_flagged` (Story 2.3.1); `schema_version` is the first top-level key.
- The envelope fields `mode`, `banner`, `advisory`, `non_claims`, `header`, `warnings`, `summary` are present on every output, including a zero-finding run (UX-R14).
- Output is complete (no `--top` truncation) and valid JSON even when 0 findings (`"findings": []` plus `"not_assessed": [...]`).
- Every class W condition is in `warnings[]`; with `--format json` each is also one `warning:` line on stderr; stdout contains JSON and nothing else (UX-R10).
- Key order and finding order are deterministic across runs on identical input.
- `brief` records never contain imperative deletion language in `record_non_claims` or any text field.

## 5. S4: dismissal flows

### 5.1 Flow D1: "correct but intentional" (inline ignore)

```
Finding shown -> developer decides to keep the test
   |
   v
Add directive directly above the test function (above any decorators/attributes):
   Go/Java/Kotlin/Rust/TS:  // kibitzer:ignore test-zero-unique-coverage -- pins issue #123
   Python:                  # kibitzer:ignore test-zero-unique-coverage -- pins issue #123
   |
   v
Re-run -> finding gone, summary shows "suppressed by kibitzer:ignore: N"
         (the edit touches only a comment, so the test file is not marked stale; section 3.4)
```

| Step | User does | System responds |
|---|---|---|
| 1 | Reads the finding, opens the test | The finding line number is the function's first line (anchor), so the directive goes on the line above |
| 2 | Writes directive with a reason | Next run: finding absent; suppressed count +1 |
| 3 | Omits the `-- <why>` | `kibitzer run` reports an `[ignore-syntax]` malformed-directive finding (existing behavior, `docs/suppressing-checks.md`); the target finding is not suppressed |
| 4 | Misspells the rule id | `[ignore-syntax]` reports unknown rule `test-zero-unique-covrage` with the nearest valid ids |
| 5 | Later deletes/renames the test | Directive is orphaned; `kibitzer run` surfaces `[unused-ignore]` for static rules (existing, #123) |

Dynamic rules and `[unused-ignore]` *(proposed)*: the report does **not** emit `[unused-ignore]` for dynamic rules, because a dynamic finding can vanish when the input data differs (different run, other tests removed) without the directive being wrong. Re-flagging happens only if the test's own content changes, which keeps dismissed findings quiet (`research/ux.md` section 2, staleness).

### 5.1a Auditing dismissals (iteration 2, G12)

`kibitzer tests report --show-suppressed` adds a section under the summary listing each suppressed finding, so a dismissal cannot outlive its reason unseen:

```
Suppressed by kibitzer:ignore (2)
  pkg/y_test.go:30: [test-zero-unique-coverage] TestY -- pins issue #123
  pkg/z_test.go:55: [test-zero-unique-coverage] TestZ -- intentional   (non-specific reason)
```

Each line is `path:line: [rule] TestName -- <reason as written>`, in path then line order, with the non-specific mark when D-E6 applies. In JSON the same data is the always-present `suppressed[]` array (section 4). A directive whose finding no longer appears in the input at all is not listed, because the report cannot tell a fixed test from an absent one; `kibitzer run` reports `[unused-ignore]` for static rules only (D1 step 5), and that asymmetry is accepted.

### 5.2 Flow D2: "the finding is wrong" (false positive)

| Step | User/agent does | System responds |
|---|---|---|
| 1 | Notices the claim is false (e.g. assertion lives in a helper in another file) | Finding text already points to `docs/test-quality-false-positives.md` mode list in `--verbose`/brief output |
| 2 | Calls MCP `report_false_positive` with the rule id as `check_name`, the file, and the reason fields | The tool appends the report to the local JSONL queue (`default_queue_path()`, `src/false_positive.rs`; no server is involved) and returns its existing acknowledgement (``queued false-positive report for `<rule>` at <queue path>. Run `kibitzer check false-positives list` ...``) followed by one added sentence: `This does not silence the finding; to silence it locally add kibitzer:ignore <rule> -- <why>.` (UX-A5). `check_name` is an unvalidated string in the tool (verified in `src/mcp.rs` and `src/false_positive.rs`), so report rule ids are accepted as they are; the triage doc a report drains into is named by that string by convention, and for report rules the owner folds entries into `docs/test-quality-false-positives.md` by hand |
| 3 | Needs the finding gone now | Also adds `kibitzer:ignore` (D1) or `assertion_helpers` option (D3); D2 alone does not silence |

Distinction printed in docs and in the "Before acting" block: **ignore = correct but intentional; report_false_positive = the tool is wrong.**

### 5.3 Flow D3: systematic false positive (scope/disable by config)

Add the helper to `assertion_helpers`, or disable/scope the rule in `.kibitzer/inspect.json` (`docs/suppressing-checks.md`). Result: all findings of that mode vanish; no per-test directives needed.

### 5.4 Flow D4: unanchored findings (report only)

A test that cannot be mapped to a source file and line (R12) is listed under "Not assessed: no-source", is never emitted as a finding, and has no acknowledgement path. **Decided (triad repair G9; this matches the plan's Task 2.3.2a decision, option (a)):** none is offered. Rationale: the `.kibitzer/accepted/` key is `(rule, file, line, content)` and the `content` field is its staleness guard, so a `(rule, test-id)` entry would be the only dismissal that never goes stale; the remedy is to refresh the input so ids match the tree (the R12 hint), not to suppress. Accepted cost: an owner cannot mark such a test "keep". The recurring `no-source` count is the visible residue and carries the same hint every run. Revisit trigger is in section 11.

### 5.5 Dismissal error and edge states

| # | Condition | What the user sees | Exit path |
|---|---|---|---|
| D-E1 | Directive above a decorator vs above `def` | Until task 2.3.2a verifies both placements, the finding text says `above the def and any decorators` (the placement that is accepted in both cases); if both are verified the text may say `above the test` | Doc example in the finding text |
| D-E2 | Directive with no reason | `[ignore-syntax]` malformed finding from `kibitzer run`; target finding still shown. In the report (iteration 3) the row gains `note: kibitzer:ignore above this test has no reason and was not applied (use -- <why>)` and the summary counts `malformed directives: N`, so the owner is told in the surface they are reading | Message gives the corrected form |
| D-E3 | Directive for a rule that is not enabled | No error; silently inert (matches existing inline-ignore behavior) | n/a |
| D-E4 | `report_false_positive` cannot append to its local queue (unwritable or missing directory; the tool has no server, `src/mcp.rs`) | The tool's existing text `error queuing false-positive report: <io error>` followed by one added sentence: `The report was not recorded; add kibitzer:ignore <rule> -- <why> to silence locally.` | Local ignore |
| D-E5 | Many findings dismissed identically (>= 5 same rule, same reason) | Report footer hint: `5 findings dismissed with the same reason; consider an assertion_helpers/scope option` *(proposed)*; json: `summary.suppressed.same_reason_groups` is an array of `{rule, reason_text, count}` for groups with count >= 5 (empty otherwise) | Config pointer |
| D-E6 | Dismissal reason is non-specific (matches a stoplist such as `intentional`, `ok`, `n/a`, `todo`, or is shorter than a minimum length; stoplist and length are *(placeholder)* set from the Story 1.2.1 ignore-reason data) | The directive still suppresses (it is well-formed); the summary counts it: `dismissals with non-specific reasons: N`, and the footer says `N dismissals have non-specific reasons; a reason that names the behaviour pinned helps the next reader`. The footer is derived (`summary.suppressed.non_specific_reason > 0`) and has no field of its own | Reword the reason. This is a nudge, not a rejection, so the quiet path stays quiet (Goodhart mitigation, triad repair G10) |

## 6. S6: enablement and config (condensed)

```jsonc
// .kibitzer/inspect.json
{
  "checks": {
    "test-no-assertion": { "enabled": true,
      "options": { "assertion_helpers": ["requireOK", "mustEqual"] } },
    "test-mock-only":    { "enabled": true }   // opt-in; never in default_checks()
  }
}
```

Acceptance bullets:
- Before gate G1 a repo with no `inspect.json` sees no test-quality findings; `kibitzer check list` lists the rules and their default state (`off`).
- Enabling a rule needs one edit to one file; disabling needs the same.
- Unknown option key produces a config warning naming the key and the valid keys, never a silent ignore.
- After G1 promotion, the "default catalog" docs list exactly the rules and languages the owner approved (Story 4.1.1).

## 7. S7: `--verbose` stderr diagnostics (condensed)

```
[tests] adapter: coveragepy 7.6.1 (ctx.json)
[tests] records: tests=212 units=3,904 mutants=0
[tests] rejected: 0
[tests] excluded: stale=9 inputs_disagree=2 failing=2 skipped=1 no-data=1
[tests] suppressed: kibitzer:ignore=2
[tests] elapsed: 0.42s
```

Bullets: stderr only; one line per fact, prefixed `[tests]`; includes reason for every rejected record; `--verbose` also expands "Not assessed" with test ids and reasons.

## 8. S8: recipe script output (condensed)

The recipe is the only step in this design that executes repo test code (the report itself never does: it parses files and runs `git diff`/`git status` only). It says so before it starts, shows progress, and can be resumed (triad repair G6).

```
per-test-go-coverage.sh: this runs your test suite once per test (attribution=isolated).
  It executes repo test code with your environment and credentials; run it in a
  checkout and shell you trust. Ctrl-C is safe: finished profiles are kept and
  --resume continues from them.
per-test-go-coverage.sh: 212 tests, attribution=isolated, go1.23, commit 9f2c1ab
[ 37/212] pkg/parse TestParseEmpty (0.4s) | elapsed 0:19 | eta about 1:30
...
wrote profiles/ (212 files) and profiles/manifest.json
next: kibitzer tests report --go-profiles profiles/
```

Rules: the notice prints before any test runs and is not suppressible by a quiet flag; the progress line goes to stderr, one line per test when stderr is a TTY and one line per 10% otherwise; `eta` is omitted until at least 5 tests have finished.

Errors: not in a Go module -> `error: no go.mod in <dir>; run from a module root`; `go` missing -> `error: go not found on PATH`; a test fails during isolated run -> recorded as `failing` in the manifest and mentioned in the summary line (`3 failing, excluded`); interrupted -> `interrupted at 37/212: wrote profiles/manifest.partial.json; re-run with --resume to continue, or delete profiles/ to start over` (no final `manifest.json` is written, so `kibitzer tests report` refuses a partial directory with `error: profiles/ has manifest.partial.json and no manifest.json (interrupted run); re-run the recipe with --resume`; the error names the marker file, per rule 3 of section 8.1).

### 8.1 One contract for every producer recipe (iteration 2, G13)

The Go script above is the pattern; every other producer recipe (coverage.py, lcov, PIT, Stryker, and the wrapper for the mutation tools) follows the same six rules, because the mutation recipes are the longest-running and costliest step and deserve the notice more than any:

1. **Notice before the first run**, not suppressible by a quiet flag: what will execute (repo test code, or a source-rewriting mutation tool that works on a throwaway copy), with whose environment and credentials, and the stated budget if one applies. The notice points at the experiment execution policy (plan Story 0.3.5) for runs on third-party code.
2. **Progress to stderr**: one line per test or mutant batch on a TTY and one per 10 percent otherwise; an `eta` only after at least 5 units finished.
3. **Interrupt**: finished work is kept and a `*.partial` marker says where it stopped; the final output file is not written, so the report refuses a partial input with an `error:` naming the marker.
4. **Resume** where the tool supports it (PIT history file, Stryker incremental mode, the Go profile directory); where it does not the notice says `no resume: a re-run starts over`.
5. **Budget** (`--budget-seconds N`): when exhausted the recipe stops, writes what it has, and says so: `stopped at budget: 412 of 1930 mutants run (21%); wrote partial report; kill data will be treated as budget-limited`.
6. **Sidecar**: on success, partial or budget stop the recipe writes `<output>.run.json` next to a file output, or `<output-dir>/run.json` for a directory output such as `profiles/`: `{tool, tool_version, commit, attribution, tests_total, mutants_total, mutants_run, sampled_fraction, budget_seconds, budget_exhausted, partial}`. The adapter reads it as "a value the input itself carries" (plan "Attribution source", step 1), copies it into `header.inputs[].mutation_run`, and sets `kill_completeness` to `sampled` or `budget_limited` so a partial run cannot look complete (R9).

A producer run by hand without a recipe has no sidecar: `mutation_run.scope` is `unknown`, and the owner may state it with `--kill-completeness full|first-killer|sampled|budget-limited`.

## 9. UX acceptance criteria

Each is testable by a human with a terminal.

**S1 hook / MCP text**

- UX-H1. Given an agent-added test with no assertion, the finding appears as `file:line: [test-no-assertion] ...` with a likely-fix line, a non-claim, and the `kibitzer:ignore` line; total <= 4 lines.
- UX-H2. No finding text begins with "Delete" or "Remove" and none contains "must", "critical", or "urgent".
- UX-H3. A diff that does not touch a vacuous pre-existing test produces no output for it.
- UX-H4. A file with no test-quality issues produces no test-quality output at all (no "0 findings" line).
- UX-H5. A file in a language with no test profile produces no error and no output.
- UX-H6. Copying the `kibitzer:ignore` line from the message verbatim (replacing `<why>`) suppresses that finding on the next run.
- UX-H7. After the G1 cap value is recorded, findings above the per-file cap are collapsed into exactly one `+N more` line (before G1 see UX-H9).
- UX-H8. Every hook and `run_checks` finding carries the quiet-path line in the form V1 or V2 of section 2.1a (V2 only when `G1-decision.md` records measurement (a) as high); no finding is emitted without one.
- UX-H9. Before the G1 cap value is recorded, hook mode collapses nothing (no `+N more` line, no cap); this is testable pre-G1 by adding six vacuous tests to one file and seeing six findings.
- UX-H10. Every hook and `run_checks` finding carries a non-claim line (what the signal cannot see) and its quiet-path line names both `kibitzer:ignore` and `report_false_positive`; no sample exceeds 4 lines.

**S2 report**

- UX-R1. Given a valid input file, the user reaches a ranked table with one command.
- UX-R2. Every row names the rule id, the raw value, the confidence tier (not in `descriptive` or `report_only` mode, section 3.2a), and (for dynamic signals) the overlapping tests.
- UX-R3. The header always shows tool, tool version, commit, and attribution method, and the `scope:` line in the single form `assessed N of M tests in the input (K not assessed)`, where K is the sum of every not-assessed reason including stale and no-source (UX-R21).
- UX-R4. The output always contains a "Not assessed" line with a count (0 included) and the non-endorsement sentence: "Absence of a flag does not mean a test is valuable." in `experimental` and `tiered` mode, "Absence from this list does not mean a test is valuable." in `descriptive` and `report_only` mode (R17 included).
- UX-R5. The output contains no headline percentage of "redundant" or "ineffective" tests, no pass/fail verdict, no summed runtime, and no phrase of the form "safe to remove", "savings" or "estimated runtime" in any format.
- UX-R6. Exit status is 0 when tests are flagged; exit 2 only for class E conditions in section 3.4.
- UX-R7. A "joint coverage check" line is present for every dynamic rule class that flagged a test; it is computed per class (and unit kind) over that class's `IndividuallyFlagged` set of the main list, never over a mix with static rules and never over the `JointSafe` subset (zero by construction, not printed), not by summing per-test values; it names the unit kind, the set and its size and how many members had data; it is worded as a measurement on the selection data ("covered by these tests and by no test outside the set"), never as the effect of removing the set; a measured zero always carries the attached non-claim sentence (`zero_note` in json).
- UX-R8. With 31 flagged and the default `--top 20`, the table shows 20 rows plus one line stating how many were omitted, per rule, and how to see all.
- UX-R9. Running twice on identical input produces byte-identical output (deterministic ordering and tie-breaks).
- UX-R10. With `--format json`, stdout parses as JSON and nothing else; every warning is present in `warnings[]`, and each is also one `warning:` line on stderr (a copy, not the only home).
- UX-R11. Output at 80 columns does not wrap mid-row for test names under 40 characters; longer test names are printed in full (they may wrap); no name or anchor in the text report is truncated with `...`.
- UX-R12. Each of R1-R27 produces its specified message, and the exit status matches its class.
- UX-R13. The first output line is the mode banner of section 3.2a, one of `experimental:`, `descriptive:`, `report_only:` or `tiered:`; the last two name the validation that backs them (an anchor in `docs/test-effectiveness.md` that exists). In `descriptive` mode no row has a `conf` column, no ranking line is printed, rows are in test-id order, and the output contains no `JointSafe` count, no "safe" wording and no "redundant only with" column.
- UX-R14. Every `--format json` and `--format brief` output carries `schema_version` (first), `mode`, `banner`, `advisory`, `non_claims`, `header` (tool, version, commit, attribution per input), `warnings`, and `summary` (flagged by rule, joint check, suppressed count), including when `findings` is empty; `format` is the second key, and `brief` carries the identical envelope.
- UX-R15. Each row prints `path:line: [rule] TestName` in full; the path, line and rule id are never truncated or abbreviated, and `line` equals the `test.line` of the same record in json.
- UX-R16. An overlap partner that is itself flagged is marked `*` in text and listed in `overlaps_also_flagged` in json; the Before-acting block contains the sentence that at least one of an overlapping flagged group should be kept.
- UX-R17. Rows are never ordered by runtime and no output contains a runtime total; `cost` appears per row as context only.
- UX-R18. The text output ends with a Legend that defines `alone`, `conf` (when shown), `cost`, `*`, `inputs disagree` and `stale`.
- UX-R19. Under `--top N` with R rules present, the visible window holds at least `min(count, floor(N / R))` rows of each rule, rows print in global key order, and the omitted line names the omitted count per rule.
- UX-R20. `--by-package` prints a per-package roll-up table (assessed, flagged, per-rule counts) and at most `--top` rows per package (default 5); json gains `packages[]` and stays complete; the flat `--top 20` default does not apply in this view.
- UX-R21. In the header and the breakdown, `assessed + not assessed = tests in the input`, and the parts of the `Not assessed` line (stale, no-source, inputs disagree, failing, skipped, generated, no-data) sum to K; tests present in the repo but absent from the input are reported on a separate `note:` line and in `header.scope.repo_tests_not_in_input`.
- UX-R22. Every json record carries `section` (`main` or `experimental`) and `signal_mode`; `signal_mode` uses the vocabulary of `mode` (`experimental`, `descriptive`, `report_only`, `tiered`), `experimental` meaning no G2 verdict; main and experimental rows are told apart by `section` only; top-level `mode` is the least-validated `signal_mode` among main rows in the order `experimental` < `descriptive` < `report_only` < `tiered`.
- UX-R23. `overlaps_with` and `overlaps_also_flagged` hold `test.id` strings; `redundant_only_with_flagged` counts flagged tests whose partners are all flagged; JSON enum values are snake_case spellings of the hyphenated CLI flag values (`in-order` is `in_order`, `first-killer` is `first_killer_only`).
- UX-R24. With `--leave-one-out`, a shortlisted row shows its measured result in text and `leave_one_out` in json, and a malformed result file is class E R3 or R4 naming the flag.
- UX-R25. Schema evolution (round 5, G6): within `schema_version` 1, injecting unknown keys or unknown enum values into a conforming fixture does not break the reference consumer; removing, renaming or retyping a key, or changing an enum value's meaning, requires version 2.
- UX-R26. When `with_data` is 0 for a joint-check class the text line reads `no coverage data for these tests; no joint check computed` and json has `units_held_only_by_set: null` and `zero_note: null`; a count of 0 is printed only when at least one member had data.
- UX-R27. Zero flagged (R17) and zero assessed (R27) are distinct: R17 uses the single scope form `assessed N of M tests in the input (K not assessed)`, says "listed" in `descriptive` and `report_only`, and still prints the Not assessed, Stability, non-endorsement, Legend and experimental-hidden lines; R27 never contains the words "no tests flagged".
- UX-R28. Every finding record carries `input` (index into `header.inputs`, null for static rules), and `test.id` is unique within a report: repeated qualified names within a file get `#2`, `#3` suffixes in source order.
- UX-R29. `confidence` is omitted from every record exactly when top-level `mode` is `descriptive` or `report_only`; a record's `signal_mode` is its own verdict and is informational once `confidence` is omitted. With no main rows after G2, `mode` is the least-validated recorded verdict among the signals of the run (`experimental` when none has one).
- UX-R30. The "experimental" vocabulary table of section 3.1 holds: `section: experimental` rows are only structurally capped signals (a verdict never lifts a cap), and the experimental section has its own `--top` window with a `<flagged> flagged, <shown> shown` heading.
- UX-R31. Static rules are evaluated on the working tree regardless of staleness; a stale test with a static finding is listed and also counted under `Not assessed: stale`, and the Not assessed line says to regenerate the input to re-assess.
- UX-R32. `brief` differs from `json` only by `record_non_claims` and `verify_first` per record; `record_non_claims` values come from a closed per-rule list.
- UX-R33. In text, an overlaps line ends `(all flagged)` exactly when every `overlaps_with` member is in `overlaps_also_flagged`, and the summary's "redundant only with other flagged tests" count equals the number of such rows.
- UX-R34. `joint_check` entries use the key order `class, rules, unit_kind, set, set_size, with_data, units_held_only_by_set, zero_note, basis`; `ignore_note` is a string present only on a record with a malformed directive.

**Errors, exit paths, states**

- UX-E1. Every class E message names the offending flag or file and states one concrete fix; none ends with only "failed" or "invalid".
- UX-E2. Every class W warning appears in the stdout body above the table (not buried after it, not stderr only), starts with the literal `warning:`, and states which signal's confidence it lowered (in text by the message itself or the appended ` (lowers: <signal>)`; in json by `lowers`).
- UX-E3. Given lcov with no `TN:` names, the report states "per-test coverage not available from this input" and does not print a ranking of per-test signals.
- UX-E4. Given a PIT report without a full matrix, no `zero-unique-kills` row shows a tier above `low`.
- UX-E5. Given a report whose commit differs from HEAD, tests in files changed since that commit are not flagged by dynamic rules and are counted under "Not assessed: stale" (static rules are still evaluated on the working tree, section 3.4); a change to those files that only touches comments, including adding a `kibitzer:ignore` line, does not make them stale (UX-E14). Given a dirty working tree (R23), a non-ancestor commit (R24) or a non-git directory (R25), the matching warning appears and the run still exits 0 (see UX-E10).
- UX-E6. Given an input whose test ids no longer exist in the repo, the report completes (exit 0) and counts them under `no-source`.
- UX-E7. Integration or snapshot tests with zero unique coverage are shown as `low` or listed under Not assessed, never `medium`.
- UX-E8. No error state is a dead end: every E/W message either names the flag, command, or doc to use next or states that no action is needed.
- UX-E9. When reading an input takes noticeable time (threshold *(placeholder)*, set from the Story 2.1.5 parse measurements), a `[tests] reading <file> (<size>)` line appears on stderr if stderr is a TTY; stdout is unaffected, so determinism (UX-R9) holds.
- UX-E10. Given a dirty working tree, a report commit that is not an ancestor of HEAD, and a non-git directory, the report prints R23, R24 and R25 respectively above the table, exits 0, and excludes only the tests the warning names.
- UX-E11. The collection recipe prints its executes-repo-test-code notice before the first test runs, prints per-test progress to stderr, and after an interrupt leaves `manifest.partial.json` and continues with `--resume` without re-running finished tests.
- UX-E12. With `--experimental`, signals outside the main-list rule of section 3.1 (before G2: structurally capped at `low`; after G2: no recorded verdict or still capped) appear only in a separate "Experimental signals" section with its own count line, and each of their rows ends with `(experimental)`; they are absent from the main list, from the main summary counts (`flagged_by_rule`, `individually_flagged`, the joint check), and (in json) carry `"section": "experimental"` and are counted only in `summary.experimental`. Without the flag they appear nowhere and one summary line names their count.
- UX-E13. With `--format json` or `brief`, a class E condition exits 2 with stdout holding exactly the error object of section 4 (`schema_version`, `format`, `error.id` from the section 3.4 table, `message`, `hint`) and the `error:` line on stderr; with `--format text` stdout is empty.
- UX-E14. Adding, editing or removing a `kibitzer:ignore` line, or any other comment-only or whitespace-only edit to a test file, does not mark that file's tests stale and R10/R23/R24 do not count it; a file whose comment-stripped token stream differs does.
- UX-E15. Every producer recipe (coverage.py, lcov, PIT, Stryker, mutation wrapper) prints its notice before running, shows progress, leaves a partial marker on interrupt, states whether it can resume, writes `<output>.run.json` on every exit path including a budget stop, and a budget-limited or sampled run is reported by the report as R9 with `kill_completeness` not `full`.
- UX-E16. Exit status 1 is used only for an internal error or an IO failure writing the output, with `error: internal:` on stderr; no input condition ever produces 1.
- UX-E17. CI degradation (round 5, G1): `warnings[]` includes class I notices; a consumer treats a report as degraded by `class == "W"` entries (or `lowers.effect != "informational"`) and `mode`, never by `warnings[]` being non-empty. A clean zero-flag run has `nothing-flagged` in `warnings[]` with `class: "I"`.
- UX-E18. A json/brief document with a top-level `error` key is not a report and carries none of the envelope keys; a consumer tests `error` first. R19 is the one exit-2 path with empty stdout under json.
- UX-E19. R22's auto-switch to the per-package view changes text only; `json` and `brief` are byte-identical with and without it and gain `packages[]` only under an explicit `--by-package`.
- UX-E20. Class I messages start `note:` (R13, R15, R17, R22, R27), W start `warning:`, E start `error:`; under json/brief W and I lines are each copied once to stderr.
- UX-E21. Given PIT without a full matrix or any kill input forcing a structural `low` cap (including after a G2 verdict), the signal's rows appear only in the experimental section, never in the main list at `low`; R8 and R9 text and exit-path column say so.
- UX-E22. A user-supplied `--kill-completeness first-killer|sampled|budget-limited` fires `kill-data-incomplete` (R9) with `params.source: flag`; `pit-no-full-matrix` fires only when the PIT file itself lacks the full matrix.
- UX-E23. Recipe vocabulary (round 5, G21): the sidecar and notices use `attribution`, never `mode`; the sidecar is `<output>.run.json` for a file output and `<output-dir>/run.json` for a directory; the report's partial-input error names the marker file.


**Dismissal**

- UX-D1. A user can dismiss one finding in 1 edit (add one comment line above the test) and 1 re-run.
- UX-D2. A dismissed finding does not reappear when unrelated tests are added or removed; it reappears only if the directive is moved, removed or detached from the test by an edit (inline ignores have no content guard; plan Story 2.3.2 restates this).
- UX-D3. A directive without a reason is reported as malformed and does not suppress.
- UX-D4. The report summary shows the suppressed count so dismissals are visible, not silent.
- UX-D5. Docs and the "Before acting" block state the split between `kibitzer:ignore` (intentional) and `report_false_positive` (tool wrong) in one sentence each.
- UX-D6. The report does not emit `[unused-ignore]` for dynamic rules.
- UX-D7. Dismissals whose reason matches the non-specific stoplist or minimum length still suppress, are counted in `summary.suppressed.non_specific_reason` and in the text summary, and trigger the one-line footer hint (D-E6).
- UX-D8. `--show-suppressed` lists each suppressed finding as `path:line: [rule] TestName -- <reason>` with the non-specific mark where it applies, and json always carries `suppressed[]` with the same data; neither lists a directive whose finding no longer appears.
- UX-D9. A `kibitzer:ignore` directive without a reason above a flagged test leaves the finding listed, adds a `note:` line to its row, and is counted as `malformed directives: N` in the summary and `summary.suppressed.malformed_directives` in json.
- UX-D10. Same-reason dismissals (D-E5) appear in json as `summary.suppressed.same_reason_groups` (`{rule, reason_text, count}`, count >= 5); the non-specific footer (D-E6) is derived from `non_specific_reason > 0` and has no separate field.

**Agent-specific**

- UX-A1. For a test the agent just added, the message names strengthening as the first likely fix and requires naming the covering test before removal.
- UX-A2. In hook mode no dynamic-signal finding (redundant, mutant-silent) is ever emitted; dynamic findings exist only in the report.
- UX-A3. Re-running the hook after the agent adds a trivially weak assertion (`assert x is not None`) on an otherwise vacuous test is evaluated by the transcript backtest; if observed, `test-tautology`/weak-assertion wording is revised before G1 *(validation criterion, depends on Story 1.2.1)*.
- UX-A4. The JSON and brief outputs carry `advisory: true` and the global `non_claims` array; the non-endorsement sentence (variant per UX-R4) appears in all three formats.
- UX-A5. The `report_false_positive` acknowledgement states that it does not silence the finding and names `kibitzer:ignore <rule> -- <why>` as the local way to do so; it extends the tool's existing queue acknowledgement rather than replacing it, and a queue write failure adds the same local-ignore sentence to the existing `error queuing false-positive report` text (D-E4).

**Accessibility (terminal)**

- UX-X1. Every state conveyed by color (tier, warning, error) is also conveyed by text (`high`/`medium`/`low`, `warning:`, `error:`). v1 uses no color, so this holds by construction; the criterion guards later additions.
- UX-X2. With `NO_COLOR=1` or non-TTY stdout, output contains no ANSI escape sequences.
- UX-X3. Output uses ASCII only; no box-drawing, emoji, or Unicode arrows, so screen readers and `less`/CI logs render identically.
- UX-X4. Only `--by-package` is a columnar table (plain space-aligned columns with a header row, no merged cells). The main report rows are three-line blocks (headline, `alone`/`conf`/`cost` line, `overlaps` line), each readable top to bottom as a record. There is no one-line-per-finding text mode in v1; a grep or `sort` pipeline over text uses the headline, which is the only line of a row containing `[rule]` (`grep '\[test-'` yields one line per finding), and a pipeline needing more uses `--format json`.
- UX-X5. Text contrast is not relied on: no output depends on dim/grey-only emphasis to be understood.
- UX-X6. The first-line banner is one unwrapped line; every other non-row line wraps at 100 columns on a word boundary with a 2-space continuation, and a path, rule id, sha or anchor is never split.
- UX-X7. Row blocks and the grep path are as stated in UX-X4; no text mode promises one line per finding.

Total: 10 (H) + 34 (R) + 23 (E) + 10 (D) + 5 (A) + 7 (X) = **89** UX acceptance criteria (41 original; 13 added in triad repair iteration 1: R13-R18, E9-E12, D7, A4, A5; 8 added in iteration 2: H8, R19-R21, E13-E15, D8; 6 added in iteration 3: R22-R24, E16, D9, X6; 21 added in round-5 repair: H9, H10, R25-R34, E17-E23, D10, X7; IDs are stable and never renumbered, so some lists are out of numeric order only where an ID was appended inside its group; counted by grep of `^- UX-<letter><n>.` lines). Surfaces: S1-S8 = 8 (counted by grep of the section 1 table rows).

## 10. Plan deltas this design requires

*Status: all six deltas below, and the section 11 decision on unanchored findings, are applied in `implementation/plan.md` (UX repair pass and Phase 4 repair log); the list is kept as the traceability record.*

1. Add `ExclusionReason::Stale` and `NoSource` to the enum in the plan's Domain Glossary and Story 2.1.3.
2. Add an exit-code contract (0 for findings, 2 for input errors) to Story 2.3.1.
3. Add `--by-package`, default `--top 20`, and "Before acting" block text to Story 2.3.1.
4. Add per-file cap for hook-mode findings to Story 1.1.2 (placeholder value, set after the backtest).
5. Story 2.3.2 acceptance: add D-E1 (decorator placement) as an explicit test case.
6. Provenance must include commit for R10; Story 2.1.1 currently lists commit in `Provenance` but adapters are only required to reject missing tool/version. Missing commit is a warning (R11), not an error.

## 10b. Plan deltas from triad repair iteration 1

*Status: applied in `implementation/plan.md` (see its "Triad repair log (iteration 1)") and `implementation/validation.md`.*

| # | Delta | Plan home |
|---|---|---|
| T1 | JSON envelope (`mode`, `banner`, `advisory`, `non_claims`, `header`, `warnings`, `summary`) frozen as `schema_version` 1 before the schema story closes; warnings in stdout body and `warnings[]`, stderr copy only for json/brief | Story 2.3.1, new Task 2.3.1e |
| T2 | Mode matrix and descriptive wireframe (section 3.2a); `mode` per signal | Stories 2.3.1, 4.2.1 |
| T3 | Summary reworded to a joint coverage check; runtime sum and runtime ordering removed; flagged overlap partners marked; keep-one step | Story 2.3.1, Observability Plan |
| T4 | `path:line: [rule] TestName` rows, no anchor truncation; legend; `--experimental` section | Story 2.3.1 |
| T5 | R23-R26 (dirty tree, non-ancestor, non-git, input cap) | Stories 2.3.5 (Task 2.3.5b), 2.2.7 |
| T6 | Recipe notice, progress, `--resume` | Story 2.2.2 (Task 2.2.2c) |
| T7 | `report_false_positive` acknowledgement text; non-specific-reason count (D-E6) | Story 2.3.2 (Task 2.3.2d) |

## 10c. Plan deltas from triad repair iteration 2

*Status: applied in `implementation/plan.md` (see its "Triad repair log (iteration 2)") and `implementation/validation.md`.*

| # | Delta | Plan home |
|---|---|---|
| U1 | Joint coverage check computed over `IndividuallyFlagged`, never the zero-by-construction `JointSafe` subset; `JointSafe` count replaced by "of these, N are redundant only with another flagged test" (B1) | Story 2.3.1, Glossary `HarnessInvariant` |
| U2 | Wireframe row order, experimental count line and arithmetic, `--experimental` main-list rule (G1-G3) | Story 2.3.1, Story 4.2.1 |
| U3 | Scope line and Not-assessed breakdown include stale and no-source; `repo_tests_not_in_input` note (G4) | Stories 2.3.1, 2.3.5 |
| U4 | Comment-only/ignore-directive-only edits are not staleness (G5) | Story 2.3.5 |
| U5 | Hook quiet-path line kept in variants V1/V2, never dropped (G6) | Stories 1.3.1, 4.1.1 |
| U6 | SM5 removal bucket; write-time limits stated (G7) | `requirements.md` SM5, Task 1.2.1c |
| U7 | Machine contract: `test.id` grammar, `value.kind`, stable warning ids, structured `lowers`, class E error object, `format` key and identical brief envelope, `mutation_run`, `kill_completeness` values (G8) | Stories 2.3.1, 2.2.4, 2.2.5 |
| U8 | `--top` window stratified by rule; `--by-package` wireframe and per-package `--top` (G9) | Story 2.3.1 |
| U9 | "inputs disagree" label (G10); four banners incl. `report_only` and `tiered` (G11) | Stories 2.3.1, 4.2.1 |
| U10 | `--show-suppressed` and json `suppressed[]` (G12) | Stories 2.3.1, 2.3.2 |
| U11 | Producer recipe contract: notice, progress, partial, resume, budget, sidecar (G13) | New Story 2.2.8 |
| U12 | `report_false_positive` wording matches the real local-queue tool (engineering A1) | Story 2.3.2 (Task 2.3.2d) |

## 10d. Plan deltas from triad repair iteration 3

*Status: applied in `implementation/plan.md` (see its "Triad repair log (iteration 3)") and `implementation/validation.md`.*

| # | Delta | Plan home |
|---|---|---|
| V1 | Per-record `section` (`main`/`experimental`); `signal_mode` vocabulary unified with `mode`, least-validated ordering defined (B1) | Story 2.3.1, Task 2.3.1e, Glossary `ReportMode` |
| V2 | Joint check per rule class and unit kind, zero case carries a non-claim, static rules excluded (B2) | Story 2.3.1, Task 2.3.1e |
| V3 | Wireframe row order, arithmetic and descriptive title; JSON example order | `design/ux.md` only |
| V4 | Enum spelling map, report_only summary keys, `test.id` in overlap fields, `redundant_only_with_flagged` definition, `leave_one_out` field and errors | Story 2.3.1 |
| V5 | Exit 1 for internal/IO failure; malformed-directive note in the report; R10/R23/R24 precedence; assumed-attribution id; wrap rule; brief `record_non_claims`; "added or signature-changed" wording | Stories 2.3.1, 2.3.4, 2.3.5 |

## 10e. Plan deltas from round-5 repair

*Status: applied in `implementation/plan.md` / `validation.md` (round-5 repair and round 5b).*

| # | Delta | Plan home |
|---|---|---|
| W1 | `ReportMode` JSON spelling is `report_only` (banner prefix `report_only:`); verdict names in `G2-decision.md` stay hyphenated (`ship-report-only`) | Glossary `ReportMode`, Story 2.3.1, Story 4.2.1 |
| W2 | R27 `nothing-assessed`, R17 wording, `note:` prefix for class I, ` (lowers: ...)` suffix | Story 2.3.5 |
| W3 | Record `input` key; `test.id` `#n` suffix; `same_reason_groups`; `joint_check` null when `with_data == 0`; schema evolution policy and freeze test | Story 2.3.1, Task 2.3.1e |
| W4 | Static rules not excluded by staleness | Story 2.3.5 |
| W5 | Hook text: non-claim line and `report_false_positive` in quiet-path line; pre-G1 no cap | Stories 1.1.2, 1.3.1 |
| W6 | Recipe sidecar uses `attribution`; directory-output sidecar path; partial-marker error text | Story 2.2.8 |

## 11. Unresolved flows

1. **Dismissal of unanchored dynamic findings (D4): decided, with a revisit trigger.** Option (a), keep as not-assessed with no acknowledgement path, is the decision (section 5.4 gives the rationale; plan Task 2.3.2a records it and verifies it against the code). Revisit trigger, from the plan: if Story 3.3.1 triage shows `NoSource` tests are a material share of an arm's would-be flags, reopen option (b) (a content-bearing key in `.kibitzer/accepted/`).
2. **Confidence-tier numeric boundaries** (`high`/`medium`/`low`) are intentionally undefined until corpus triage (Story 3.3.3); wording is final, thresholds are not.
3. **Hook per-file cap value** and the `--by-package` large-suite threshold are placeholders pending backtest data.

## Repair log - round 5

Blocker B1: `report-only` -> `report_only` in the mode matrix, JSON `mode` and `signal_mode` values, banner text and 3.2b head (banner prefix equals the JSON value), field notes, UX-R2, UX-R13, UX-R22 and sections 10c/10d. The G2 verdict name `ship-report-only` stays hyphenated (a decisions-file value, not a JSON enum).

| Gap | Change |
|---|---|
| G1 | CI step 4 keys on `class == "W"` / `lowers.effect`; UX-E17 |
| G2 | R17 rewritten (single scope form, "listed", full footer printed); new R27 `nothing-assessed`; UX-R27 |
| G3 | Joint check with `with_data == 0`: no count, `null`; UX-R26 |
| G4 | One behaviour: capped signals only in the experimental section, a verdict never lifts a cap; R8/R9 text; UX-E21 |
| G5 | Error-object discriminator; R19 empty stdout noted in CI flow; UX-E18 |
| G6 | Schema evolution policy (additive vs version bump); UX-R25 |
| G7 | Per-record `input` key; example finding 2 aligned with header (coverage.py), `baseline` figure fixed; UX-R28 |
| G8 | Key-order sentence scoped, `joint_check` order stated, `ignore_note` and `record_non_claims` typed; UX-R34 |
| G9 | `brief` documented as superset; dropped duplicate location and constant `action`; UX-R32 |
| G10 | Hook samples carry a non-claim line; line budget restated; UX-H10 |
| G11 | H7 scoped post-G1, pre-G1 no cap; UX-H9 |
| G12 | Quiet-path line (V1, V2) names `report_false_positive`; UX-H10 |
| G13 | Text ` (lowers: ...)` suffix; class I prefix `note:`; R15 reclassified print form; UX-E20 |
| G14 | Vocabulary table for "experimental"; UX-R30 |
| G15 | Mixed-verdict rule, confidence omission, descriptive vs report_only difference; UX-R29 |
| G16 | `(all flagged)` tag on overlaps line, summary reworded; UX-R33 |
| G17 | Experimental section has its own window and heading; UX-R30 |
| G18 | Static rules exempt from staleness; Not assessed note; UX-E5 amended, UX-R31 |
| G19 | Descriptive/report_only non-endorsement variant ("Absence from this list"), placeholder replaced with concrete text, "listed" vs window distinguished; UX-R4, UX-A4 amended |
| G20 | R22 auto-switch affects text only, wording "first 5 in key order"; UX-E19 |
| G21 | `mode` -> `attribution` in recipe notices and sidecar; sidecar path for directories; partial-marker error text; UX-E23 |
| G22 | `test.id` `#n` disambiguation; UX-R28 |
| G23 | Row blocks vs columnar stated; grep path via `[rule]` headline, no one-line mode in v1; UX-X4 amended, UX-X7 |
| G24 | D-E1 interim wording, D-E5 `same_reason_groups`, D-E6 derived footer; UX-D10 |
| G25 | `--kill-completeness` flag maps to R9 `kill-data-incomplete`, R8 only for PIT file structure; UX-E22 |

Counts: 89 criteria (was 68; +21); 8 surfaces; error/notice rows R1-R27 (was R1-R26; R27 appended).
