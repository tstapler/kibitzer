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

## Fixed

### 2026-08-13 — design-docs — both directions of the mechanism in one session, self-resolving each time

- **Symptom**: a reference-style link's *use* and its *definition* landing in separate
  `Edit` calls, in either order, got the earlier one blocked even though the very next
  edit resolved it (`nop-self-service-project-creation/README.md`, `tstapler/design-docs`,
  3 blocks across 41 edits) — see git history for the full original entry.
- **Mechanism**: `Cache::apply_grace` ([src/cache.rs:138](src/cache.rs#L138)) applies
  generically to every `Blocking`-severity `CheckResult`, keyed on `(file_path,
  check_name)` — not specific to native vs. shelled-out checks, and not gated on
  `design-docs` migrating off `doc_report.py`/`markdownlint-cli2` as this doc previously
  assumed. It downgrades a check's *first* failure on a file to Advisory, escalating to
  Blocking only if that same check fails again on the next touch to that file with no
  passing result in between — exactly the "fail once, pass on the immediate next edit"
  shape both directions of this entry describe. The native `markdown-link-integrity`
  checker (`src/markdown_link_integrity.rs`) also reimplements `doc_report.py`'s
  bidirectional rule — `unused_definition_findings`
  ([src/markdown_link_integrity.rs:264](src/markdown_link_integrity.rs#L264)) flags a
  dangling *definition* with no use, not just a dangling *use*, and both feed the same
  `check_name` so both get identical grace treatment.
- **Real bug found and fixed along the way**: grace state (`grace_pending`) wasn't
  actually surviving between separate `kibitzer hook` process invocations for
  diff-scoped `Edit` calls — `cache.save()` (`src/daemon.rs`, both the daemon-resident
  path and the no-daemon fallback) was only called when `changed_lines.is_none()` (i.e.
  a `Write`), never for an `Edit`, which is the shape of every entry in this doc. Fixed
  by making the save unconditional. Confirmed empirically: a test isolating
  `XDG_RUNTIME_DIR` (so it can't accidentally hit a real background daemon) failed before
  this fix and passes after it.
- **Regression-guarded** by `markdown_link_integrity_dangling_reference_then_its_definition_never_hard_blocks`,
  `markdown_link_integrity_dangling_definition_then_its_use_never_hard_blocks`, and
  `blocking_check_grace_persists_across_diff_scoped_edits_without_a_daemon`
  (`tests/hook_contract.rs`).
- **Known limit, still open**: the grace window is exactly one edit-to-that-file wide —
  a violation that survives more than one subsequent touch to the same file without an
  intervening pass still hard-blocks on the second touch, same as before this fix. The
  remaining `## Log` entries below (multi-edit gaps of 4+ tool calls, or a violation that
  never gets fixed within the session) are NOT covered by this fix — see each entry.

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
- **Status (2026-09-11 re-audit)**: still open. `Cache::apply_grace`'s window (see the
  `## Fixed` section above) only covers a violation surviving exactly one subsequent
  touch to the file; the definition here lands "a later `Edit`," not confirmed to be
  the *immediate* next one, and (per `docs/reporting-false-positives.md`'s removal
  policy) this session predates the grace-persistence bugfix's own landing time, so it
  wouldn't have applied even if the gap were narrow enough.

### 2026-08-13 — design-docs — unrelated edit blocked by a dangling reference a prior edit in the same turn introduced

- **Repo**: `tstapler/design-docs`, file: `titus-k8s-migration-brief/README.md`
- **Session**: `~/.claude/projects/-Users-tstapler-Documents-design-docs/fdb9431f-d338-4bac-9c54-a6140ca7e99a.jsonl`, `toolu_01Rwvf5b6Vg5zrNK9fapeWxW`
- **What changed**: `old_len=290`, `new_len=279`. The edit swapped one inline link's
  target and text — `[scope clarification](#scope-clarification-with-argha-2026-08-13)`
  became `[clarification thread #1](#places-that-need-clarification)` — both valid
  anchors pointing at headings that already existed in the file at the time. This edit
  did not touch reference-style links at all.
- **Why it fired**: both `markdown-link-integrity` and `doc-structure-report` blocked.
  Four tool calls earlier in the same turn (`toolu_01TiLzkUkwMKhRX6ZWcbgUgP`), an `Edit`
  had introduced 10 reference-style link *uses* (`[What is MD?][md-intro]`,
  `[MD delivery-config resources][md-resources]`, `[Titus Tasks & Jobs][compute-titus-jobs]`,
  and 7 more) with no matching `[ref-id]: target` definitions anywhere in the file yet —
  those definitions weren't added until `toolu_01WwttNagtMuwXgENgLFjZ8W`, four tool calls
  *after* the one that got blocked. The blocked edit landed in between, on an unrelated
  part of the file, and got caught by the pre-existing dangling references from the
  earlier edit.
- **Mechanism**: same whole-file/non-diff-aware re-check described above, confirmed here
  with an even cleaner example — the edit kibitzer blocked is not the edit that
  introduced the violation, nor does it touch the violating content at all. Reconstructed
  by extracting `old_string`/`new_string` for every `Edit` to this file in the session
  and grepping each `new_string` for reference-style use (`[label][ref-id]`) vs.
  definition (`^[ref-id]:`) patterns, confirming which edit added the uses (index 2 of
  11) versus which added the definitions (index 7 of 11) relative to the blocked edit
  (index 3). `doc-structure-report`'s repo-wide scope (`scripts/doc_report.py`'s `main()`
  calls `root.glob("*/README.md")` with no CLI-argument handling at all — confirmed by
  reading the script directly) means it would have re-surfaced this same violation
  regardless of which file in the repo the triggering edit touched; `markdown-link-integrity`
  here is `markdownlint-cli2 {file}` (per this repo's `.claude/inspect.json`) — file-scoped,
  not the native grace-period checker described in the Resolution section below, since
  this repo hasn't migrated off the shelled-out setup.
- **Not a pure deletion**: net shrink (`290` → `279`), but irrelevant here — the point
  isn't deletion vs. addition, it's that the edit's content has nothing to do with the
  violation that blocked it.
- **Status (2026-09-11 re-audit)**: still open, and not just a `design-docs`-migration
  gap — `Cache::apply_grace` (see the `## Fixed` section above) now confirmed to apply
  regardless of native-vs-shelled checker. But the gap here (~8 tool calls between the
  edit that introduced the dangling refs and the one that got blocked) far exceeds
  grace's one-touch window: any intervening touch to *this* file would already have
  escalated to Blocking on its second consecutive failure. Would still reproduce today.

### 2026-08-12 — design-docs — 9 blocks in one session, same file, `doc-structure-report` only

- **Repo**: `tstapler/design-docs`, file: `nkp-migration-approach/README.md`
- **Session**: `~/.claude/projects/-Users-tstapler-Documents-design-docs/55b4e258-63ad-4d83-b6c1-e4d9aeacbd94.jsonl`
- **What changed**: a long editing session (17:33–18:06) that progressively restructured
  this doc, adding ~20 reference-style link uses (`[label][ref-id]`, e.g.
  `[phasing-vs-timeline]`, `[security-parity-gap]`, `[three-classes]`) across several
  edits before their matching `[ref-id]: target` definitions were added. 9 separate
  `Edit` calls got blocked; only `doc-structure-report` fired each time (not
  `markdown-link-integrity`), consistent with the "unused reference def" check.
- **Why it fired**: representative example — `toolu_01Fmg3kS9TUE3oLxegBxYc8r` (blocked
  17:33:33) only changed section-number citations (`§4.6`/`§4.7`/`§4.8`), no
  reference-style link content at all, yet was blocked because the *immediately
  preceding* edit (`toolu_01UvgiXrqY4C15JwaSVPncpJ`) had already introduced 7 dangling
  reference uses with no definitions yet. The same pattern repeats for
  `toolu_01QgwkGGiGiyqVFUC7S5FbWx` (blocked 18:05:15, only a `§4.2` citation change).
  The remaining blocks are further edits landing while the growing set of dangling
  references was still being built up in stages — matching definitions for most of them
  weren't added until `toolu_015YQ8cffzRxduqnQ6KvfaEb` (18:04:56), well after the first
  block.
- **Mechanism**: same whole-repo, non-diff-aware `doc-structure-report` behavior already
  confirmed against `scripts/doc_report.py`'s source in the 2026-08-13 entry above (its
  `main()` ignores the CLI argument and rescans the whole repo via
  `root.glob("*/README.md")`) — not re-verified against source in this pass (this
  session's sandbox couldn't read `/Users/tstapler/Documents/design-docs` to
  double-check), but the transcript evidence (multiple edits blocked purely on
  citation-number changes with no reference-link content) matches the already-confirmed
  mechanism exactly, so it's filed here rather than re-verified from scratch.
- **Not a pure deletion**: all 9 edits in this session net-grew the file; irrelevant here
  regardless — several of the blocked edits (see above) didn't touch reference-link
  content in either direction.
- **Status (2026-09-11 re-audit)**: still open. `apply_grace`'s semantics (see the
  `## Fixed` section above) explain the pattern exactly — only the very first failure on
  this file downgrades; blocks 2–9 occur while the same (file, check) pair is still
  failing with no intervening pass, so each escalates immediately. That's grace working
  as designed, not a gap in it — but it still means a growing set of dangling references
  built up in stages blocks every edit after the first, which is the behavior this entry
  flags as surprising. Would still reproduce today.

### 2026-08-11 — design-docs — the session that first introduced the argocd-machine-access/constraint-verification refs later seen in dcb5a7eb

- **Repo**: `tstapler/design-docs`, file: `nop-self-service-project-creation/README.md`
- **Session**: `~/.claude/projects/-Users-tstapler-Documents-design-docs/fcf3c0eb-bf1f-49bd-a2eb-3e9da93bf241.jsonl` (22:26–22:27), `doc-structure-report` only, 2 blocks across 24 `Edit` calls to this file.
- **What changed**: `toolu_013xBp8xX5gSdctTbZpM8dfy` (edit 2 of 24) introduces a reference-style
  use, `[constraint verification chain][constraint-verification]`, with no definition yet.
  `toolu_01Ky6TRyrQcKiz7fDqFcrhPP` (edit 3, blocked; `old_len=33`, `new_len=1312`) then adds
  a second dangling use, `[the hard constraint][hard-constraint]`, and gets blocked —
  correctly, since both refs are still undefined at that point.
  `toolu_01CLadRay1vycbimfiJiJgCc` (edit 4, blocked; `old_len=188`, `new_len=245`) adds a
  third dangling use, `[ArgoCD machine access & project tokens][argocd-machine-access]`,
  and is blocked for the same reason. `toolu_018a3hiLr95d5FvC5z1tbzrL` (edit 5) finally adds
  `[argocd-machine-access]:` and `[constraint-verification]:` definitions, resolving those
  two; no further blocks occur for the rest of the session (edits 6–24).
- **Not fully resolved in-session — but not a false-positive miss either**: `hard-constraint`
  never gets a matching definition anywhere in this session (confirmed: the string
  `hard-constraint` appears in only two places in the whole transcript — its introduction
  at edit 3 and unchanged surrounding context in edit 4's `old_string` — and no later edit
  in this session touches it). Reading the file's current on-disk state, that sentence was
  later rewritten in some *subsequent* session to drop the reference-style link entirely
  (line 314 now reads plain prose, `the hard constraint (stated in […][argocd-machine-access])`),
  so the dangling ref was real but got cleaned up outside this transcript — consistent with
  a true (if eventually self-corrected across sessions, not within one) catch, not something
  to log as a false positive. Curiously, `doc-structure-report` did not re-block on edits 6–24
  despite this dangling ref persisting for the rest of the session — not investigated further
  here since it doesn't change the disposition of blocks 1–2 (both were correct, and both
  were resolved for their own introduced refs one edit later, matching the confirmed
  mechanism), but worth a re-check if `doc_report.py`'s blocking-vs-advisory split is
  revisited later.
- **Mechanism**: same whole-file/non-diff-aware re-check as the log entries above — the
  "use before definition" direction specifically (not the mirror-image variant from the
  `dcb5a7eb` entry). Not re-verified against `doc_report.py`'s source in this pass; reapplying
  the mechanism already confirmed against source earlier in this investigation.
- **Not a pure deletion**: both blocked edits net-grew the file substantially; each is
  self-caused (introduces the very reference that trips the check).
- **Status (2026-09-11 re-audit)**: blocks 1–2 were correct catches per the entry's own
  text, not false positives — leave as-is. The "why didn't edits 6–24 re-block on the
  still-dangling `hard-constraint`" anomaly remains unexplained: `apply_grace` keys purely
  on `(file_path, check_name)` with no per-violation identity, so a still-failing whole-file
  check *should* re-escalate on the next touch to this file regardless of which specific
  reference is dangling — this doesn't fit that model and isn't traceable to anything in
  kibitzer's own `src/cache.rs`/`src/daemon.rs`. The most likely remaining explanation lives
  in `design-docs`' own (kibitzer-external) `doc_report.py`, not confirmed. Left open per
  `docs/reporting-false-positives.md`'s policy against closing an entry on an unresolved
  open question.

### 2026-08-13 — design-docs — pure punctuation edit blocked by an earlier, uninspected edit's dangling reference

- **Repo**: `tstapler/design-docs`, file: `unified-cluster-provisioning-and-upgrades/README.md`
- **Session**: `~/.claude/projects/-Users-tstapler-Documents-design-docs/ed9d5a4c-eba2-443b-b3ad-d4639ad9ae02.jsonl`, `toolu_01LpuBy3mFvHAaW5fr1eTkXt`
- **What changed**: `old_len=332`, `new_len=328` — pure punctuation/wording cleanup (splitting
  a run-on sentence, swapping a parenthetical for an em dash). No link or reference content
  touched at all.
- **Why it fired**: both `markdown-link-integrity` and `doc-structure-report` blocked. Two
  edits earlier in the same session, `toolu_01Q29Q7mjpFX8DUxYzJANJ4Z`, had introduced a
  reference-style use, `[that doc's stale-comment correction][nop-self-service-stale-comment]`,
  with no `[nop-self-service-stale-comment]: target` definition anywhere in the file — and no
  edit in the rest of this session ever adds one (confirmed: only 5 `Edit` calls touch this
  file in the whole transcript, and none contains `nop-self-service-stale-comment]:`). The
  blocked edit landed in between, on an unrelated sentence, and got caught by that
  already-dangling reference.
- **Notable**: kibitzer only produced a `hook_success`/`hook_blocking_error` attachment for 2
  of the file's 5 `Edit` calls in this session (the one right before the block, and the block
  itself) — the edit that actually introduced the dangling reference has no hook record at
  all, so it can't be confirmed whether the hook simply wasn't invoked for it or ran and
  didn't emit an attachment. Not investigated further; doesn't change the disposition of the
  block itself.
- **Resolved later, not in this session**: the live file (`grep` run outside this session's
  transcript) now has both `[nop-self-service-stale-comment]:` and `[titus-nop-managed-cell]:`
  definitions in its References section, so the violation was real but got fixed in a later
  session, not by anything in this transcript — matching the pattern already seen in the
  `fcf3c0eb` entry above (`hard-constraint`), where a dangling ref survives past the session
  that introduced it without triggering more blocks for the rest of that session.
- **Mechanism**: same whole-file/non-diff-aware re-check as the entries above — an unrelated
  edit gets blocked by a still-unresolved violation from an earlier edit in the same session.
  Not re-verified against `doc_report.py`'s source in this pass; reapplying the mechanism
  already confirmed against source earlier in this investigation.
- **Not a pure deletion**: near-net-neutral length change (`332` → `328`); irrelevant here —
  the edit's content has nothing to do with the violation that blocked it.
- **Status (2026-09-11 re-audit)**: still open — the dangling reference here persists for
  the rest of the session with no intervening pass, so `apply_grace`'s window (one touch)
  is exceeded the same way as the `fdb9431f`/`55b4e258` entries above. Would still
  reproduce today.

### 2026-08-13 — design-docs — the edit that *resolves* its own dangling reference gets blocked, not the edit that created it

- **Repo**: `tstapler/design-docs`, file: `deployment-safety-k8s-terraform/README.md`
- **Session**: `~/.claude/projects/-Users-tstapler-Documents-design-docs/386848aa-3bca-4ae1-a09f-fa5df2279413.jsonl`, `doc-structure-report` only, 1 block across 2 `Edit` calls to this file.
- **What changed**: `toolu_013HexYUt8Tp8A7rGSmenwh5` (edit 1, passed) adds a new paragraph
  citing `[IMv2][imv2]` — a reference-style use with no `[imv2]:` definition anywhere in the
  file yet. `toolu_019KTcHEEZgwGrpWcnynTrNx` (edit 2, **blocked**) adds exactly the missing
  definition, `[imv2]: https://docs.google.com/document/d/.../edit "Infrastructure
  Management Platform v2 (go/imv2)"`, right after the file's pre-existing `[nkp-fnr]:`
  definition — i.e. this edit's own content fully resolves the only dangling reference either
  edit introduced.
- **Backwards from every other case logged here**: in the `fcf3c0eb`/`dcb5a7eb`/`ed9d5a4c`
  entries above, the edit that *introduces* a dangling ref is the one that risks blocking, and
  the edit that supplies the matching definition/use is what resolves it (and passes). Here
  it's inverted: the introducing edit (1) passed, and the completing edit (2) — which by
  itself makes the file's `imv2` reference fully consistent — is the one that got blocked.
- **Ruled out**: not a duplicate-definition problem (confirmed via the live file: exactly one
  `^[imv2]:` line, no duplicates), not the pre-existing `[tfno]` reference (already defined
  earlier in the file, unchanged by either edit), and not a `§`-citation problem (this file has
  no `§` citations at all). Checked every reference-style label in the current file
  (`grep -oE '\][a-zA-Z0-9_-]+\]'` for uses vs `^\[label\]:` for definitions) — zero dangling
  labels in the file's current state.
- **Not fully explained**: since `doc-structure-report` is a whole-file check (not scoped to
  the diff) and this file is large (~330 lines) with plenty of content neither edit touched,
  the most likely explanation is that some *other*, pre-existing reference elsewhere in the
  file was transiently dangling at the exact moment of edit 2, and got cleaned up by an
  unrelated edit in a later session — matching the "dangling ref persists past the session
  that introduced it, doesn't re-block, and is eventually fixed elsewhere" pattern already
  flagged as an open anomaly in the `fcf3c0eb` and `ed9d5a4c` entries above. Filesystem access
  needed to reconstruct the exact file state at block time (or to re-read
  `scripts/doc_report.py`'s source to confirm this theory) was not available this pass — left
  as an open question rather than asserted as fact.
- **Not a pure deletion**: both edits net-grew the file substantially.
- **Status (2026-09-11 re-audit, INFERRED not confirmed)**: best-fit explanation under
  `apply_grace`'s actual semantics (see the `## Fixed` section above): edit 1 likely
  wasn't a clean pass but a first-failure grace-downgrade on some *other*, pre-existing
  dangling reference elsewhere in this large file (which reads as "passed" in a coarse
  pass/fail transcript read) — edit 2's block is then the second-consecutive-failure
  escalation for that same other reference, not for `imv2` (which edit 2 does fully
  resolve). Can't confirm without the raw transcript attachment for edit 1 (session file
  on a macOS path, inaccessible from this audit's machine) — left open per policy rather
  than asserted.

### 2026-08-10 — design-docs — edit with no link syntax at all gets blocked, first hit of a 6-block session

- **Repo**: `tstapler/design-docs`, file: `nop-self-service-project-creation/README.md`
- **Session**: `~/.claude/projects/-Users-tstapler-Documents-design-docs/dbf3af09-e4a8-47ff-97cf-25cb6bedbdda.jsonl`,
  `toolu_01UQgAuTANP3MjLxiKv3rRd6` (20:07:33Z), `markdown-link-integrity` only (not
  `doc-structure-report`, unlike the other 5 blocks later in this same session).
- **What changed**: appends one sentence to an existing bullet about external-provisioning
  ownership (old_len=956, new_len=960 — pure text addition, nothing deleted). The added text
  contains **zero** Markdown link syntax — no `[...]`, no `[...]:`, no bare URLs — verified by
  reading the full `old_string`/`new_string` pair directly from the transcript.
- **Why it's a false positive**: the 4 `Edit` calls preceding this one in the same session
  (`toolu_01UNsTwKjrn8oBFdwDBMTtUs`, `toolu_01MdNckDjJaYSbra6x8y1kRK`, `toolu_01TLh9eHLRCNxvkXoFYcB7Ys`,
  `toolu_011bu4NLwAVQNCGbeM6fh5rn`) all passed, and none of them introduce a dangling reference
  either — one of them (`toolu_011bu4NLwAVQNCGbeM6fh5rn`) adds a link, but it's a fully-formed
  inline link (`[Slack thread](https://example.slack.com/archives/...)`), not a reference-style use. So
  nothing in this session's own edit history explains the block; it must be a pre-existing
  violation elsewhere in this large, mostly-untouched file that predates the session.
- **Not fully explained**: by the time of this investigation the file had been rewritten
  substantially in later sessions (none of the sentences from this session's edits are present
  in the current file anymore), so the exact dangling reference responsible at 20:07:33Z on
  2026-08-10 can't be reconstructed after the fact. Matches the same open anomaly already
  flagged in the `fcf3c0eb`/`ed9d5a4c`/`386848aa` entries above (a whole-file check blocking on
  content the triggering edit never touched) rather than a new mechanism.
- **Rest of the session (not logged as separate entries — true positives)**: the same session's
  remaining 5 blocks (`toolu_0124a6eu4TmDEEi2aBekWe9T`, `toolu_01Y64C65ydMERiqUYBKcmy89`,
  `toolu_01UhBgxCh3YpJHdzhHujUERk`, `toolu_01QtDYvgXaaKgNMrCphjpzkV`, `toolu_01MVMTzCDvh88NsZrw46efjt`)
  are genuine use-before-definition violations — each introduces a reference-style link
  (`[...][appendix-live-conversations]` or `[...][pr-crd-sync-no-token]`) several edits before a
  later edit in the same session (`toolu_01Qk142v33bDQvTKLYc2anFP` and `toolu_014V4Cw6CeWdQVfuTm9ZgC7F`
  respectively) supplies the matching `[label]:` definition — the same confirmed mechanism as the
  `dcb5a7eb` entry above, not a new false-positive sample.
- **Status (2026-09-11 re-audit)**: still open. This transcript (13:07 PDT, 2026-08-10)
  predates the `apply_grace` grace-persistence bugfix's own landing time (`980f09c`,
  20:42 PDT the same day) entirely, so grace didn't exist yet at the time. The root
  violation itself remains unreconstructable (per this entry's own admission) — even
  replayed today, a violation surviving this long in the transcript would already have
  exceeded grace's one-touch window regardless.

### 2026-09-11 — stapler-squad — `[SEVERITY: High]`-style bracket tag in a heading misread as an undefined shortcut reference link

- **Repo**: `tstapler/stapler-squad`, file:
  `docs/bugs/open/BUG-105-leaked-test-tmux-servers-exhaust-service-cgroup-cause-deploy-rollbacks.md:1`
  (a new file at the time; later renamed under `docs/bugs/fixed/` once the bug was
  fixed, same finding both times).
- **What changed**: wrote a new bug-report doc whose title line ended
  `... deploy rollbacks [SEVERITY: High]`, following this repo's own convention —
  `docs/bugs/open/BUG-103-...md` and `BUG-104-...md` both use the identical
  `[SEVERITY: <level>]` suffix in their `# BUG-NNN: ...` heading.
- **Why it's a false positive**: `[SEVERITY: High]` is a plain bracketed tag, not an
  attempted link of any kind — there is no corresponding `[SEVERITY: High]: <url>`
  definition anywhere in the document, nor was one ever intended.
- **Mechanism**: CommonMark's own grammar makes `[text]` with no following `(url)` or
  `[ref]` a syntactically valid *shortcut reference link* candidate — pulldown-cmark
  (which `markdown_link_integrity.rs::check_source` walks) emits it as
  `LinkType::ShortcutUnknown` whenever no matching `[label]: target` definition
  exists, and `reference_style_flags` (line ~216) classifies every `*Unknown` link
  type as dangling. There is no square-bracket syntax in CommonMark that means
  "plain literal bracketed text, definitely not a link attempt" — the ambiguity is in
  the spec itself, not a bug in this checker's own logic. This is a distinct
  mechanism from every other entry in this log (all of which are about *timing* —
  a valid reference-style link whose definition lands in a later edit — not about
  *misclassifying non-link bracket text as a link at all*), and is likely to recur
  on any doc using a `[TAG]`-style convention in a heading or plain sentence — this
  same repo's own `code:review` skill output format uses `[BLOCKER]`/`[CRITICAL]`/
  `[MAJOR]`/`[NIT]` severity tags the same way. Not independently verified against a
  broader corpus. A possible fix direction (not attempted here, no source change
  made): only treat a `ShortcutUnknown` as dangling if the bracketed text itself
  looks like a plausible reference label (e.g. a slug-like `[a-z0-9-]+` pattern, or
  no internal whitespace) rather than free-form prose/tags containing spaces and
  punctuation — `SEVERITY: High` contains both a colon and a space, unlike every
  legitimate reference label already in this corpus (`appendix-live-conversations`,
  `pr-crd-sync-no-token`).
- **Workaround applied in the session**: changed the heading to parenthesize the tag
  (`(SEVERITY: High)`) instead of bracketing it, since this was blocking a same-session
  edit and no suppression was in place — worked around rather than fixed at the
  source, logged here per this doc's own instruction not to just note a false positive
  in passing.

## Resolution: native `markdown-link-integrity` checker + grace period

`markdown-link-integrity` is no longer a `markdownlint-cli2`/`doc_report.py`
shell-command check — it's a native Rust `Checker` (`src/markdown_link_integrity.rs`),
wired via `checker: "markdown-link-integrity"` in `.claude/inspect.json` (see
`README.md`'s Usage section for the migration). The whole-file-vs-diff mismatch
described above still exists in principle (the checker still reads the whole file),
but the specific failure mode this doc logs — a valid multi-step edit getting blocked
mid-sequence — is now handled generically by `Cache::apply_grace` (`src/cache.rs`),
not by anything markdown-specific: the first time a given file+check fails under a
live per-edit trigger, it's downgraded to Advisory with a "will block if still failing
on the next edit" message, and only escalates to Blocking if the same file is still
failing on a later touch. This was verified empirically with a live `kibitzer daemon`
running: first touch → Advisory/exit 0, second still-failing touch → Blocking/exit 2.
See the module doc comment on `MarkdownLinkIntegrityChecker` for the full policy,
including the caveat that this escalation only survives across edits when a
`kibitzer daemon` is running (the recommended setup) — without one, grace state isn't
persisted for diff-scoped per-edit hook calls, so a still-failing violation stays
Advisory instead of escalating.

Both entries previously logged here (personal-wiki `[[wiki links]]`, stapler-squad GFM
task-list checkboxes) were fixed by enabling `Options::ENABLE_WIKILINKS |
Options::ENABLE_TASKLISTS` on the parser in `check_source` — see the commit that
removed these entries for the diff and its regression tests
(`wikilinks_are_not_flagged_as_undefined_references`,
`piped_wikilink_is_not_flagged_as_undefined_reference`,
`task_list_checkboxes_are_not_flagged_as_undefined_references`). `LinkType::WikiLink`
and `Event::TaskListMarker` are their own dedicated pulldown-cmark event types under
those options, so `[[Page]]`/`[[Page|Text]]` and `- [x]`/`- [ ]` no longer fall through
to plain CommonMark bracket parsing at all — they never reach the reference-style
matching logic this checker runs, rather than being special-cased around it.
