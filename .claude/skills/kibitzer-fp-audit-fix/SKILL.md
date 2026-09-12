---
name: kibitzer-fp-audit-fix
description: Burn down the docs/<checker>-false-positives.md backlog — audit every open Log entry, classify it against current source, dispatch one background agent per checker to fix or dismiss its entries with tests and a backtest, then synthesize the doc updates yourself. Run before shipping any PR that touches checker behavior, or periodically to clear the backlog.
---

# False-positive log audit → classify → fix

kibitzer's checkers each keep a `docs/<checker-name>-false-positives.md` log
(convention: `docs/reporting-false-positives.md`). Entries accumulate faster
than anyone fixes them, and a checker's own `## Log` section often goes stale
in a specific way: infra shipped *after* an entry was filed (the generic
`changed_lines` scoping in `src/check.rs`, `Cache::apply_grace` in
`src/cache.rs`) silently fixes the exact bug an old entry blames, but nobody
goes back to verify and remove it. This skill is the repeatable loop for
clearing that backlog without reading every entry yourself end to end.

Three roles, don't blur them:
- **You (coordinator)**: skim, group by doc, write the per-agent briefs, and
  own every doc edit and the final test run. Never let a background agent
  edit a `*-false-positives.md` file directly — concurrent agents editing the
  same doc race each other, and removing an entry is a judgment call this
  skill's own removal policy (below) says needs verification, not automation.
- **Background agents (one per doc, in parallel)**: read the doc + the
  checker's source, verify each entry against *current* code, fix what's
  genuinely broken, prove it with a test built from the entry's real example,
  and report a verdict — never touch the doc, never commit.
- **CI gate** (`cargo fmt --all --check`, `cargo clippy --workspace
  --all-targets -- -D warnings`, `cargo build --workspace`, `cargo test`, per
  `.github/workflows/ci.yml`): the thing your final synthesis pass must leave
  green before this is done.

## Step 1 — Audit: enumerate the open backlog

```sh
find docs -iname "*-false-positives.md"
```

For each file, find its unresolved entries — `grep -n '^### ' <file>` lists
every log entry; entries under a `## Fixed` or `## Resolution` heading are
historical record, not backlog (don't re-open them). Entries under `## Log`
(or a doc-specific unresolved heading) are the target. Skim each doc's
mechanism section (`## Root-cause mechanism` / `## Mechanism`) — that's the
hypothesis every entry in the file was filed against, and often the thing
that's since been fixed generically elsewhere.

Also check `docs/backtest-triage/*/*.jsonl` for `"verdict": "false_positive"`
records that haven't yet been promoted to a `docs/<checker>-false-positives.md`
entry — `docs/backtest-triage/README.md`'s own caveat says a triage verdict
there is bookkeeping, not a fix trigger, until it's written up here.

## Step 2 — Classify before delegating

Don't hand an agent a bare "go read this file." For each doc, form a
hypothesis per entry (or per cluster of entries sharing a mechanism) from
what you already know about the codebase:

- **Likely already fixed by later infra** — name the infra (e.g. `changed_lines`
  scoping, `Cache::apply_grace`) and ask the agent to confirm with a test,
  not re-derive the fix from scratch.
- **Needs a real fix** — the entry names a specific function and a proposed
  direction (many entries do — read to the end, several say "not attempted
  here, no source change made" with a concrete suggestion). Hand that
  suggestion to the agent as a starting point, not a solved problem.
- **External blocker** — `docs/reporting-false-positives.md`'s removal policy
  is explicit: an entry that blames *another* repo's own shelled-out tooling
  (not kibitzer's own checker code) cannot be closed by a kibitzer-side fix at
  all until that repo migrates. Flag these so the agent doesn't waste time
  trying to fix code that isn't kibitzer's.

Group entries by doc, not by individual entry — entries in the same doc
almost always share one source file and one mechanism, so one agent per doc
avoids duplicate reads and avoids two agents racing to edit the same
`src/*.rs` file.

## Step 3 — Fix: one background agent per doc, in parallel

Launch every doc's agent in the same message (independent source files, no
reason to serialize). Each brief must include, verbatim or adapted:

1. The doc path and a summary of your Step 2 hypotheses per entry — not just
   "read this file."
2. **Verify against current source, not the doc's claims.** Line numbers and
   even mechanisms drift; the doc is a lead, not ground truth.
3. **Fix the minimal thing**, with a regression test built from the entry's
   own real-world example (file/line/snippet it already cites) — not only a
   synthetic case. Also keep or add a true-positive test for the pattern the
   fix must NOT suppress, so narrowing/widening a heuristic doesn't trade one
   false positive for a false negative.
4. **Backtest, don't just unit-test** — this repo's `CLAUDE.md` requires it
   for any change to a check's rules/thresholds. After `cargo build
   --release`, run the changed native checker against real files: this
   repo's own source, and — if already cloned per `docs/backtest-repos.md`
   — the corpus repos, or `kibitzer check backtest <name>` against session
   transcripts. Report the exact command and output.
5. **Do not edit the false-positives doc. Do not commit or push.** Report a
   per-entry verdict: already-fixed-confirmed-with-test / fixed-by-you
   (files, tests, backtest output) / external-blocker-unchanged /
   still-open-and-why.

Use `subagent_type: general-purpose` (or `fork` only if you want the agent to
inherit this conversation's context — usually unnecessary here since each
brief is self-contained). Do not read an agent's output file yourself; wait
for its completion notification.

## Step 4 — Synthesize (you, not an agent)

Once all agents report back:

1. Update each doc yourself: move confirmed-fixed entries out per
   `docs/reporting-false-positives.md`'s removal policy — cite the fix (files
   + test names; a commit SHA once you've actually committed, not before).
   Leave external-blocker entries in place, optionally with a short note on
   what would close them. Leave still-open entries untouched.
2. Run the full CI gate yourself once, across all agents' combined changes:
   ```sh
   cargo fmt --all --check
   cargo clippy --workspace --all-targets -- -D warnings
   cargo build --workspace
   cargo test
   ```
   Fix anything broken by the combination of independent agents' edits
   (import collisions, duplicate test names) before calling this done.
3. Report a summary table (doc → entries resolved / still open / external)
   to the user and ask before committing — this repo's git-hygiene rule is
   commit only when asked, draft PRs by default.

## When to run this

- Before shipping any PR that changes a checker's rules or thresholds — the
  backlog for that checker is exactly the regression risk a reviewer will
  ask about.
- Periodically as its own maintenance pass, independent of any pending PR,
  since entries otherwise only get read when someone happens to be touching
  that checker for an unrelated reason.

## Related

- `docs/reporting-false-positives.md` — the filing and removal convention
  this skill enforces.
- `.claude/skills/kibitzer-sample-review/SKILL.md` — how *new* entries get
  filed in the first place (transcript mining). This skill is the other half
  of that loop: burning down what's already filed.
- `docs/backtesting.md`, `docs/backtest-repos.md` — the backtest discipline
  Step 3 requires before any check-behavior change counts as done.
