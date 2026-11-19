# Backtest: claude-skill-* checks

Run 2026-11-04 with a debug build of this branch, one `kibitzer check native <name> <file>`
per `.md` under `.claude/{skills,agents,commands}` (`.claude/` trees only).

## Dotfiles repo (`~/dotfiles`, 534 files linted by `lint-claude-kb`)

Per-rule counts, native vs `stapler-scripts/lint-claude-kb`:

| Rule | Native | `lint-claude-kb` | Note |
|---|---|---|---|
| `invalid-skill-location` | 1 | 1 | `.claude/skills/CHANGELOG.md` |
| `description-too-long` | 1 | 1 | `code-new-project` (1502 chars) |
| `unknown-frontmatter-key` | 1 | 1 | `github/skills/pr-ship` (`prompt:`) |
| `broken-skill-ref` | 14 | 7 | all 7 Python hits reproduced; the extra 7 are `security-review`, `claude-in-chrome`, `update-config` (built-in skills), which the script hides via a hard-coded `EXTERNAL_SKILLS` set. Native needs them in the `external_skills` option. |
| dangling symlink | 0 | 0 | none present in dotfiles. Positive cases covered by `tests/claude_skill_cli.rs` (real `kibitzer run`, git and walk paths) and `src/check/dangling_symlink_tests.rs` (nested roots, all three kinds, loop links, subdir scope) |
| `oversized-no-split` | n/a | 11 | Python-only |

Result: every covered finding the script reports is reproduced; the only difference is the
allow-list, which is configuration, not behavior.

## Public corpus (`docs/backtest-repos.md`)

| Repo | `.md` under `.claude/` | Findings |
|---|---|---|
| `kubernetes/kubernetes`, `servo/servo`, `BurntSushi/ripgrep`, `microsoft/vscode` | 0 | 0 (nothing in scope, as expected) |
| `denoland/deno` | 6 | 0 |
| `apache/cassandra` | 63 | 11 |
| `tstapler/stapler-squad` | 63 | 3 |

The expectation of 0 held only for repos with no skill tree. Two repos do ship one, so the
findings were triaged by hand:

- `apache/cassandra`: 10x `unknown-frontmatter-key` (`version:`; Claude Code ignores it, `metadata:` is the documented home) and
  1x `broken-skill-ref` (`codebase-analysis`, no such skill in the tree). Judged true positives, advisory in nature.
  The `version:` volume is the main noise risk: if it proves unwelcome, add `version` to the allowed keys.
- `tstapler/stapler-squad`: 2x `invalid-skill-location` (bare `.md` under `.claude/skills/`,
  never discovered) and 1x `broken-skill-ref` (`pm-product-management`). True positives.

## Found while reviewing

An architecture review caught that the dangling-symlink check never fired through
`kibitzer run`: the file walkers drop broken symlinks, and the 0-finding backtest row could
not reveal it. Fixed by `check::dangling_claude_symlinks` (bounded to `.claude/{skills,agents,commands}`)
and a dispatch only to checkers with `inspects_unreadable_paths()`. Reproduced before and after in a
scratch git repo containing `.claude/skills/ghost -> ../nowhere`.

## Transcript backtest (`kibitzer check backtest <name>`, `~/.claude/projects`)

| Checker | Transcripts scanned | Snapshots checked | Findings |
|---|---|---|---|
| `claude-skill-frontmatter` | 4802 | 56 | 1 (`invalid-skill-location` on dotfiles `.claude/skills/CHANGELOG.md`, a true positive) |
| `claude-skill-xref` | 4802 | 56 | 0 |
| `claude-skill-dangling-symlink` | 4800 | 74 | 0 (batch-only; edits cannot create a finding) |

1242 edits were unreconstructable in each run. The first dangling-symlink run errored when a
live session's transcript vanished mid-scan; a rerun succeeded. Only 56-74 snapshots means this
is weak evidence about per-edit noise, not a clean bill.

## Whole-tree `kibitzer run`

A plain `kibitzer run` over `~/dotfiles` or `~/dotfiles/.claude` with every default check times
out (280s / 250s). To get whole-tree output through the real runner, `kibitzer run` was pointed at
a scratch dir holding a symlink `.claude -> ~/dotfiles/.claude` and a `.kibitzer/inspect.json`
whose `disabled` list names every other registered checker (`kibitzer check list`). It finished
with exit 0. Caveats: the scratch dir has no git, so the walker also entered
`.claude/worktrees/agent-*` (nested repo copies, normally gitignored); those hits were excluded.
A few architecture checks ignore `disabled` and still printed unrelated lines.

Result, excluding `.claude/worktrees/`: 14 `broken-skill-ref`, 1 `description-too-long`,
1 `invalid-skill-location`, 1 `unknown-frontmatter-key`, 5 `missing-description`, 0 dangling
symlinks. The first four match the per-file loop above exactly.

The 5 `missing-description` hits are a deliberate difference from `lint-claude-kb`: the files
(`advanced-memory-architecture`, `basic-context-optimization`, `debug-lost-in-middle`,
`edge-case-context-poisoning`, `multi-agent-coordination`) declare `description: ''`. The script
counts the quotes as a value; native unquotes first, so an empty string is an empty
description, which is as undiscoverable as a missing one.

## Not run

- `scripts/backtest-triage.py` tracking.
- Known limitation: `claude-skill-xref` scans raw text, so a "`foo` skill" phrase inside a
  fenced code block is flagged, as `lint-claude-kb` also does. Not triaged for how often this
  produces false positives.
