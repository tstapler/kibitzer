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
entries into fixes (see e.g. the `2026-08-18` entry in
`docs/go-primitive-obsession-false-positives.md`, fixed by commit `065f6ef`).

## What this is not

This is not a suppression mechanism — filing a report does not silence the
check for anyone, including you, on a future run. If a finding is blocking
real work right now, that's a separate conversation with whoever owns the
project's `.claude/inspect.json`.
