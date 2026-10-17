# ADR-001: Per-session offset files instead of a global BTreeMap

**Status**: accepted
**Date**: 2026-10-17

## Context

`stop-offsets.json` is a single global file holding a `BTreeMap<transcript_path, u64>`.
Two concurrent Stop hook invocations race on this file without locking, causing one to
clobber the other's offset update. The result: a session re-processes already-seen
transcript content and surfaces findings from earlier tasks (possibly from other repos).

## Decision

Replace the single global file with one per-session file stored in a new subdirectory
`~/.cache/kibitzer/stop-offsets/`. The file for a session is named after the sanitized
basename of its `transcript_path` (e.g., `abcd1234.jsonl.json`). Each file stores only
a single `u64` offset; no BTreeMap needed.

## Alternatives rejected

### A. Atomic rename + single file

Write to a temp file, then `rename()` to the target. This prevents corrupt JSON from a
partial write within a single session, but does NOT fix cross-session clobber: two
sessions renaming to the same path still race. One rename wins; the other's data is lost.
Rejected: doesn't solve the problem.

### B. File locking (`flock`/`fcntl`)

Would work. Rejected because: (1) requires OS-specific syscalls or a new crate (`fs2`,
`file-lock`), violating the no-new-deps constraint; (2) advisory locking is complex to
get right (lock inheritance on fork, NFS semantics); (3) the per-session file approach
is simpler, more correct, and has no downsides.

### C. Keep the BTreeMap but use atomic rename

Same as option A at the cross-session level — still a race. The BTreeMap doesn't help.

## Consequences

- **Positive**: No cross-session contention at all. No locking code. No new deps.
  Human-readable offset filename (maps directly to session ID). Simpler storage type
  (`u64` vs. `BTreeMap<String, u64>`).
- **Neutral**: Old `stop-offsets.json` is abandoned (not deleted) after upgrade. First
  Stop event post-upgrade re-processes the transcript from offset 0 (one-time, harmless).
- **Negative**: One small file per session accumulates in `stop-offsets/`. Lazy GC
  (delete files whose transcript no longer exists) is documented but deferred to a
  follow-up. At ~100 bytes per file and a realistic rate of 5–20 sessions/day, this is
  negligible in practice (~730 KB/year maximum).
