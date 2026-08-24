# Reporting a suspected false positive

kibitzer's `PostToolUse` hook (`src/hook.rs`) is one-directional by design: it
runs checks and hands findings to the agent via `additionalContext`, but
nothing captures what the agent thinks of them. When a finding looks wrong,
the only trace today is whatever the agent happens to say in its own
transcript — invisible unless a human goes looking. This doc is the
convention for turning that into something durable and reviewable.

## When to file one

File a report when a check fires on an edit that, on inspection, didn't
actually introduce (or doesn't actually contain) the problem the check
claims — not when you simply disagree with the check's judgment call on a
genuine hit. If you're unsure which this is, read the checker's source
(`src/<checker>.rs`) first; a false-positive report should identify the
mechanism, not just the symptom.

## How to file one

Each checker has (or should have) a `docs/<checker-name>-false-positives.md`
log — e.g. `docs/go-primitive-obsession-false-positives.md`,
`docs/markdown-link-integrity-false-positives.md`. To report a new one:

1. Find (or create) `docs/<checker-name>-false-positives.md` for the check
   that fired.
2. Append an entry under its `## Log` section (create that heading if the
   file is new) following this shape:

   ```markdown
   ### <date> — <repo/session> — <one-line summary>

   - **Repo**: `<owner>/<repo>` (session `<session-name>`, if known), file
     `<path>`.
   - **What changed**: what the edit actually did.
   - **Why it's a false positive**: why the finding doesn't apply to this
     edit.
   - **Mechanism** (if known): the specific source-level reason the check
     fired anyway — cite the function/file (e.g.
     `src/hook.rs::compute_changed_lines`). If you haven't traced it to a
     mechanism, say so explicitly rather than guessing.
   ```

3. Use the actual current date, not a placeholder.

Do not edit or delete existing entries you didn't investigate — append only.
A maintainer triages the log periodically and turns confirmed, high-signal
entries into fixes.

## Removing resolved entries

Once a logged entry's root cause has been fixed by a shipped commit — the
same mechanism, re-tested, no longer reproduces — delete the entry rather
than leaving it to accumulate. The commit that fixed it is the durable
record (cite it in the commit message that removes the entry); git history
for this file preserves the deleted text if it's ever needed again. Only
remove an entry once you've confirmed the fix actually covers it: re-read
the checker's current source (or re-run the backtest) and check that the
specific mechanism the entry describes is what changed, not just that
"some fix landed nearby."

Do not remove an entry just because:
- the checker it names was rewritten for unrelated reasons, if the specific
  failure mode described is still present or unverified against the new code
- a fix landed in a *different* system than the one the entry blames (e.g.
  the entry blames a repo's own shelled-out `doc_report.py`/`markdownlint-cli2`
  setup — a fix to kibitzer's native checker doesn't resolve it until that
  repo actually migrates to the native checker)
- the entry itself flags open questions or unverified corroboration (missing
  source access, "not fully explained," etc.) — resolve the open question
  first, don't drop the entry because it's inconvenient to chase down

## What this is not

This is not a suppression mechanism — filing a report does not silence the
check for anyone, including you, on a future run. If a finding is blocking
real work right now, that's a separate conversation with whoever owns the
project's `.claude/inspect.json`.
