---
name: kibitzer-corpus-fp-triage
description: Run a checker against the real-world backtest corpus (docs/backtest-repos.md), triage every NEW finding via scripts/backtest-triage.py, promote confirmed false positives into docs/<checker>-false-positives.md, then hand off to kibitzer-fp-audit-fix. The front half of the false-positive loop that skill fixes.
---

# Kibitzer corpus false-positive triage

Closes the front half of the false-positive loop: `kibitzer-fp-audit-fix` fixes
what's already logged in `docs/<checker>-false-positives.md`, but nothing sweeps
the real-world corpus (`docs/backtest-repos.md`) for NEW false positives and gets
them into that doc in the first place. This skill is that front half.

Contrast with `kibitzer-sample-review`, which mines *this machine's* live Claude
Code session transcripts for real hook firings — that only finds what happens to
fire naturally. This skill instead sweeps the fixed corpus of cloned OSS repos on
demand, which is broader and repeatable — the right tool right after tuning a
rule, or when chasing one reported symptom class rather than waiting for a real
session to reproduce it.

## Step 1 — Run the checker against the corpus, per repo

For each corpus repo relevant to the checker's language(s) — `docs/backtest-repos.md`'s
table; don't run a Go checker against `servo/servo` — confirm it's actually cloned
(`scripts/clone-backtest-repos.sh`, or point at the local `stapler-squad` checkout
per that doc's note), then:

```sh
python3 scripts/backtest-triage.py run \
  --repo-slug kubernetes-kubernetes \
  --repo-dir ~/code/github.com/kubernetes/kubernetes \
  --checker comment-quality-go --glob '*.go'
```

Repeat once per (checker, corpus-repo) pair. This prints counts by verdict plus a
`NEW (untriaged)` count — the reading list for Step 2. Pass `--rule` to scope to
one rule id (e.g. `over-commented`) when chasing one specific symptom a user
reported, instead of triaging the whole checker cold.

This covers per-file native checkers only (`checker::registry()`). For a checker
that runs once over the whole repo instead — anything under
`check::lookup_any_architecture_checker` (`kibitzer check architecture <name> <dir>`),
including the diagram/export/change-coupling commands — see Step 1b instead.

## Step 1b — Whole-repo (architecture) checks

Some checks aren't per-file at all — they run once over an entire repo (see
`check::lookup_any_architecture_checker` / `architecture_checks.rs`). `backtest-triage.py
run` supports these via `--mode architecture` (no `--glob`, one invocation instead of a
per-file loop):

```sh
python3 scripts/backtest-triage.py run \
  --repo-slug kubernetes-kubernetes \
  --repo-dir ~/code/github.com/kubernetes/kubernetes \
  --checker import-cycle --mode architecture
```

This only produces usable NEW/triaged diffs for checkers whose findings carry a real
`file`/`line` — currently `import-cycle` and `layering`. Everything else in that
registry (`coupling`, `package-size`, `instability`, `dip-concrete-coupling`, and any
future package-level metric) reports at package granularity with no file or line at
all, so the `(file, line, rule)` triage-store key doesn't fit them — `--mode
architecture` will faithfully run them but every finding will parse as untracked
(no match ever lands in the store). For those, skip the triage store and review by
hand instead:

```sh
kibitzer check architecture instability ~/code/github.com/kubernetes/kubernetes
```

Read the output directly, apply Step 2's same true/false-positive/needs-discussion
judgment per package, and jump straight to Step 3 for anything confirmed — cite the
package name and a `tree`-style permalink (`.../tree/<sha>/<path>`, not a line anchor)
instead of a `blob`+`#L<n>` one.

`change-coupling` is different again: per its own doc comment in `main.rs`, it's
"a look-here prioritization report, not a per-edit pass/fail check" with no verdict
concept — nothing to triage as true/false-positive. Run it (`kibitzer architecture
change-coupling <dir>`) and skim for pairs that are obviously wrong (e.g. two files
that only co-occur because of a repo-wide mechanical change, not real coupling); log
those as a `needs_discussion`-style note directly in Step 3's doc if worth recording,
otherwise there's nothing to do here.

`diagram`/`export` (`kibitzer architecture diagram|export <dir>`) aren't checks at
all — no findings, no verdicts. Treat running them against a corpus repo as a feature
smoke test, not a triage pass: confirm the command completes without error and the
output (Mermaid text / JSON model) looks structurally sane for a repo this size. If
something crashes or the output is obviously broken, that's a bug report, not a
false-positive log entry — file it the normal way, not through this skill's doc.

## Step 2 — Triage every NEW finding

For each NEW finding, read the actual source with enough surrounding context — a
function's full body and comment, not just the flagged line — and classify:

- **`true_positive`** — the check is right; nothing to log.
- **`false_positive`** — the check fired on something it shouldn't have. Trace the
  *mechanism* in the checker's source before marking this — `docs/reporting-false-positives.md`'s
  bar is identifying the mechanism, not just the symptom. Don't guess from the
  message alone.
- **`needs_discussion`** — a real hit whose value as a lint is genuinely debatable
  (an idiomatic style choice that mechanically matches the rule).

Record every verdict, not just the false positives — the triage store's
already-triaged bookkeeping only stays useful if the whole NEW set gets a verdict
each run, not a cherry-picked subset:

```sh
python3 scripts/backtest-triage.py mark \
  --repo-slug kubernetes-kubernetes --repo-dir ~/code/github.com/kubernetes/kubernetes \
  --file pkg/kubelet/foo.go --line 42 --rule over-commented \
  --verdict false_positive --note "one-line mechanism summary"
```

Always pass `--repo-dir` — it stamps the commit SHA and a GitHub permalink
automatically, so nobody re-derives either by hand later.

Triaging several corpus repos is exactly the kind of independent, fan-out work a
background agent per repo is good for — dispatch one `general-purpose` agent per
(checker, repo) pair with Steps 1–2 as its brief, each reporting its
`false_positive`/`needs_discussion` verdicts (source snippet + mechanism) back to
you rather than writing anywhere itself. Synthesize their reports yourself before
Step 3 — same reasoning as `kibitzer-fp-audit-fix`'s Step 4: promoting a verdict
into a permanent doc is a judgment call, not something to parallelize blindly.

## Step 3 — Promote confirmed false positives into the doc

A `false_positive` verdict in `docs/backtest-triage/*.jsonl` is bookkeeping only —
per that store's own README, it doesn't change checker behavior and isn't itself
a filed report. Only a write-up in `docs/<checker-name>-false-positives.md` is
(`docs/reporting-false-positives.md`'s convention). For each confirmed false
positive — or one entry per shared mechanism, if several corpus instances trace
to the same root cause, not one entry per instance — append a `## Log` entry
using that doc's template, adapted for a corpus source instead of a session
transcript:

```markdown
### <date> — <repo-slug> corpus backtest — <one-line summary>

- **Repo**: `<owner>/<repo>`, file `<path>:<line>` — permalink from the triage record.
- **What the code looks like**: the actual flagged snippet, not just a description.
- **Why it's a false positive**: why the finding doesn't apply here.
- **Mechanism**: the specific source-level reason the check fired anyway, citing
  the function/file — from Step 2's investigation, not re-guessed here.
```

Do not fix anything yet, and do not edit or remove any existing entries.

## Step 4 — Hand off to `kibitzer-fp-audit-fix`

Once the doc has new `## Log` entries, invoke the `kibitzer-fp-audit-fix` skill to
do the actual fixing — its Step 1 audit picks up exactly what Step 3 here just
wrote. Don't duplicate that skill's fix loop here.

## When to run this

- Cold, periodically, as its own maintenance sweep — the corpus is fixed (shallow
  clones, never `git pull`ed per `docs/backtest-repos.md`), so a checker's own
  behavior is the only thing that can make a previously-clean pass surface new
  findings.
- Scoped to one `--rule` right after tuning that rule, as the backtest step
  `CLAUDE.md` already requires before landing a check-behavior change — this
  skill is a more structured way to run that backtest and keep the resulting
  triage instead of re-reading raw output every time.

## Related

- `docs/backtest-triage/README.md` — the triage-store convention and its caveats
  (a live, non-frozen corpus repo like `stapler-squad` needs its triage results
  re-checked against drift; a re-clone at a new commit invalidates old verdicts).
- `docs/backtest-repos.md` — which repos exist, why, and how to clone them.
- `.claude/skills/kibitzer-fp-audit-fix/SKILL.md` — the fix half this hands off to.
- `.claude/skills/kibitzer-sample-review/SKILL.md` — the transcript-sourced
  sibling of this skill (live session firings instead of a corpus sweep).
