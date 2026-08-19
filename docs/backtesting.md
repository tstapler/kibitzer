# Backtesting checkers against real transcripts

`kibitzer check backtest <name>` runs a native checker against real historical file
edits pulled from Claude Code session transcripts (`~/.claude/projects/*/*.jsonl`),
without needing the historical file content to still exist on disk or in git. It
exists to answer two questions before a checker ships or changes:

- **Does it work?** Does it actually fire on the kinds of edits it's meant to catch?
- **Is it noisy?** How often does it fire on edits that shouldn't have tripped it —
  i.e. false positives — across a broad sample of real work?

## Usage

```sh
kibitzer check backtest primitive-obsession
kibitzer check backtest all --only-new
kibitzer check backtest duplicate-code --transcripts-dir ~/.claude/projects
```

- `name` is a checker name from `kibitzer check list`, or `all` to run every
  registered checker.
- `--transcripts-dir` defaults to `~/.claude/projects` and accepts either a
  projects root (one subdirectory per project, each full of `.jsonl` files) or
  a single project directory with `.jsonl` files directly inside it — both
  shapes are scanned in the same pass.
- `--only-new` drops findings that also fired against the file's content
  immediately before the edit — i.e. keeps only findings the edit itself
  introduced, mirroring what the live `PostToolUse` hook would actually have
  flagged (it downgrades pre-existing violations via a git-HEAD baseline
  comparison; backtest does the analogous comparison against transcript history
  instead of git, since there's no repo checkout to diff against).

Output is one line per finding: `<transcript>#<seq> <file>:<line>: [<checker>] <message>`,
tagged `(pre-existing)` when it also fired before the edit. Exit code is `1` if any
non-pre-existing finding was reported, `0` otherwise (with `--only-new` this makes it
usable as a pass/fail gate in a review workflow).

## How reconstruction works

A transcript is a JSONL file of `assistant`/`user`/etc. records. The tool never
touches the filesystem paths the transcript mentions — it only reads the JSONL
itself, replaying two things:

1. **`Read` tool results.** A whole-file `Read` (no `offset`/`limit`) returns its
   content as a `cat -n`-formatted string in the matching `tool_result` record,
   joined by `tool_use_id`. Stripping the `{n}\t` prefix off each line reconstructs
   the file's content at that point — this seeds "known content" for a path.
2. **`Write`/`Edit`/`MultiEdit` tool uses.** `Write` replaces the known content for
   a path outright (its `content` field is the new file, full stop) and is always
   treated as an unscoped rewrite — there's no "before" to compare against, so a
   `Write` snapshot's findings are never marked pre-existing. `Edit`/`MultiEdit`
   apply their `old_string`/`new_string` pairs onto the last known content for that
   path; the resulting before/after pair is what gets checked.

Only `Write`/`Edit`/`MultiEdit` produce a checkable snapshot — a bare `Read` updates
the known-content map but is never itself backtested, matching how the live hook
never fires on a `Read`.

## Caching

Results are cached persistently at `$XDG_CACHE_HOME/kibitzer/backtest-cache.json`
(or `~/.cache/kibitzer/backtest-cache.json`), keyed per transcript by its
mtime+size and the sorted set of checker names run against it. A transcript is
only re-reconstructed and re-checked when its content changed or the checker
selection differs from what produced the cached entry — everything else is a
cache hit. This is transparent and automatic; there's no flag to disable it.
Delete the cache file to force a full recompute (e.g. after a checker change
you want reflected without touching the transcripts, since the checker
selection alone doesn't distinguish "checker renamed" from "checker behavior
changed").

## Limitations (read before trusting a result)

- **No seed, no snapshot.** An `Edit` on a path with no prior `Read`/`Write` in the
  same transcript — or whose `old_string` isn't found in the last known content
  (often because an earlier tool this reconstruction doesn't model, like a shell
  command, changed the file) — is skipped and counted as "unreconstructable" in the
  summary line, not silently guessed at.
- **Windowed reads don't seed.** A `Read` with `offset`/`limit` only returns part of
  the file; using it as a seed would silently truncate the reconstructed content, so
  it's ignored.
- **`replace_all` is applied as a single replacement.** `apply_edit` does a
  first-occurrence `old_string` → `new_string` swap regardless of the edit's
  `replace_all` flag. Most edits aren't `replace_all`, so this undercounts
  duplicate replacements far less often than treating every edit as
  unreconstructable would overcount misses.
- **No cross-validation against tool success.** The reconstruction doesn't check
  whether the transcript's own `tool_result` reported an error for that `Edit`/
  `Write` (e.g. a rejected edit, a permission denial) — it assumes every tool_use it
  can apply actually happened. A failed edit that nonetheless matched its
  `old_string` would produce a spurious snapshot.
- **No true repo root.** Checkers are glob-matched against the transcript's raw
  `file_path` (often absolute), not a path relative to some detected repo root. Most
  checker globs (`**/*.go`, etc.) still match correctly since `**/` matches any
  prefix including none, but a checker with a narrower glob relying on repo-relative
  structure may not scope the way it would live.
- **Best-effort, not authoritative.** This is meant to surface trends and give
  concrete examples across a corpus of real edits, not to reproduce history exactly.
  Treat every finding as something to go look at, not as ground truth on its own.

## Workflow: validating a new or changed checker

1. Implement or modify the checker and run it against your test fixtures as usual
   (`cargo test`).
2. Run `kibitzer check backtest <name> --only-new` against your own
   `~/.claude/projects` (or point `--transcripts-dir` at a shared corpus of
   transcripts, if one exists) to see what it would have flagged across real past
   edits.
3. Review the flagged snapshots. For each:
   - **Correctly caught something real** — good, keep going.
   - **False positive** — tune the checker, or if the false positive is a known,
     accepted tradeoff, log it in `docs/<checker-name>-false-positives.md` following
     the existing convention (see `docs/go-primitive-obsession-false-positives.md`,
     `docs/markdown-link-integrity-false-positives.md`).
   - **Missed something real** — a historical edit you know should have fired but
     didn't; use it to write a new regression test for the checker directly (the
     backtest tool has no auto-repro-as-test feature; that's a manual step).
4. Iterate until the noise level is acceptable, then ship.
5. For an existing checker, re-run the backtest periodically or after any change to
   confirm it's still catching real issues without regressing on false-positive rate.
