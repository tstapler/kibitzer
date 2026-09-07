# Real-world backtest repos

A fixed set of large, well-regarded open-source repos (plus `stapler-squad`, Tyler's
own Go monorepo) used to sanity-check a native checker against real code before/after
it ships — a complement to `docs/backtesting.md`'s transcript-based backtest, which
only covers edits kibitzer has actually seen in a Claude Code session. This list exists
so picking the corpus doesn't have to be redone (or re-argued) each time.

Cloned via `scripts/clone-backtest-repos.sh`, one shallow (`--depth 1`) clone per repo,
into `~/code/github.com/<owner>/<repo>` per the repo-placement convention in
`~/.claude/CLAUDE.md` — outside this repo, not committed here.

| Repo                     | Language(s)      | Why chosen |
|--------------------------|------------------|------------|
| `kubernetes/kubernetes`  | Go               | Largest widely-referenced real-world Go codebase; broad style variance across many contributors. |
| `apache/cassandra`       | Java             | Large, mature, well-regarded Java codebase (build tooling aside). |
| `tstapler/stapler-squad` | Go               | Tyler's own in-progress monorepo — already checked out locally (see below), not re-cloned by the script. |
| `servo/servo`            | Rust             | Large browser-engine Rust codebase, heavy real-world generics/trait use. |
| `BurntSushi/ripgrep`     | Rust             | Small, famously clean, idiomatic Rust codebase — a useful low-noise counterpoint to `servo`'s size. |
| `denoland/deno`          | Rust + TypeScript | Covers both a large Rust codebase and its own TS standard library/runtime code in one repo. |
| `microsoft/vscode`       | TypeScript       | Canonical large-scale, well-regarded TypeScript application codebase — the JS/TS pick requested 2026-09-07. |

`stapler-squad` is not cloned by the script — it's already present locally (e.g.
`~/Programming/stapler-squad`, or a `~/.stapler-squad/workspaces/*/worktrees/*` worktree
checkout); point the checker at whichever local checkout is current instead.

No Python or Kotlin exemplar is in this list yet — add one the same way if a checker
needs backtesting against those languages.

## Running a checker against the corpus

```sh
# One language's syntax-rules checker against one repo, whole tree:
find ~/code/github.com/kubernetes/kubernetes -name '*.go' -print0 \
  | xargs -0 -n1 kibitzer check native syntax-rules

# Every checker `kibitzer run` knows about (uses default_checks(), no
# .claude/inspect.json needed) against one repo:
kibitzer run ~/code/github.com/BurntSushi/ripgrep
```

Filter to one rule's findings by grepping the `[rule-id]` prefix in the message, e.g.
`grep '\[flag-argument\]'`.

To triage findings as true/false positive and track that across repeated runs
instead of re-reading the same output each time, use
`scripts/backtest-triage.py` — see `docs/backtest-triage/README.md`.
