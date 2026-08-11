# `markdown-link-integrity` / `doc-structure-report` checks — known false positives

Tracks confirmed false-positive firings of the two blocking markdown checks wired up
per-project via `.claude/inspect.json` (`markdown-link-integrity` running
`markdownlint-cli2 {file}`, `doc-structure-report` running `python3
scripts/doc_report.py`) on every `Edit|Write` to a `**/*.md` file. Check new occurrences
against this list before re-investigating a firing from scratch.

## Root-cause mechanism (confirmed by reading the source and a real transcript)

Same family of bug as `go-primitive-obsession` (see
[go-primitive-obsession-false-positives.md](go-primitive-obsession-false-positives.md)):
kibitzer's checks are whole-file (or, for `doc-structure-report`, whole-*repo* — its
command is `python3 scripts/doc_report.py` with no `{file}` substitution at all), not
diff-aware. `hook::run_hook` only extracts `tool_input.file_path`; `check::run_check`
shells out against the file/repo's current on-disk state regardless of what the
triggering edit actually changed.

The specific failure mode this causes for these two checks, beyond "an unrelated
pre-existing violation gets re-surfaced": **a multi-step edit sequence gets blocked
mid-way, even though the final state is valid.** Reference-style links
(`[label][ref-id]` + a separate `[ref-id]: target` definition line) are commonly
restructured across more than one `Edit` call in the same turn — e.g. replacing an
inline `[text](url)` link with a reference-style use in one edit, then adding the
matching `[ref-id]: target` definition (often in an Appendix section) in a later edit.
kibitzer whole-file-checks after *every single* `Edit`, so the first edit — which
introduces a reference-style *use* with no matching *definition* yet — gets blocked as
an "unused reference def" / "bad anchor" violation, even though the sequence as a whole
was heading somewhere valid. This reads to the user as "the hook fires on deletion
blocks," because the edit that introduces the not-yet-resolved reference is very often
also the one that deletes/shrinks the old inline-link prose it's replacing.

## Log

### 2026-08-10 — design-docs — reference link introduced before its definition

- **Repo**: `tstapler/design-docs`, file:
  `nop-self-service-project-creation/README.md`
- **Session**: `~/.claude/projects/-Users-tstapler-Documents-design-docs/dbf3af09-e4a8-47ff-97cf-25cb6bedbdda.jsonl`, `toolu_01UhBgxCh3YpJHdzhHujUERk`
- **What changed**: an `Edit` that collapsed a long paragraph (1402 chars) citing an
  inline `[Slack thread](https://example.slack.com/archives/...)` link down to a
  shorter paragraph (718 chars) ending in a reference-style use,
  `[live-conversation lead][appendix-live-conversations]`.
- **Why it fired**: both `markdown-link-integrity` and `doc-structure-report` blocked
  in the same hook run. At the moment this specific `Edit` landed, the file did not yet
  contain a matching `[appendix-live-conversations]: <target>` definition line (per
  `doc_report.py`'s own docstring: "a `[label][ref-id]` with no matching definition
  anywhere in the doc" is one of the two hard-error cases it checks) — the definition
  was added by a *later* `Edit` in the same turn.
- **Mechanism**: confirmed from `doc_report.py`'s module docstring and from the
  transcript showing the matching `[appendix-live-conversations]:` definition landing
  in a subsequent, separate `Edit` call rather than the same one. kibitzer re-checks the
  whole file synchronously after every `Edit`, so it cannot tell "mid-sequence,
  temporarily inconsistent" apart from "actually broken" — a sequence of edits that is
  valid once complete gets blocked partway through.
- **Not a pure deletion**: `old_len=1402`, `new_len=718` — net shrink, but the edit adds
  the reference-style *use* as part of the same hunk that removes the old inline link.
  No cleaner (pure-deletion-only) example of this check firing has been found yet in
  the sessions audited so far — see `checking-invocations.md` for how to look for one.
