# Requirements: stop-hook-session-scope

**Date**: 2026-10-17
**Type**: bugfix
**Complexity**: 2 — targeted fix to a shared-state race condition; no new abstraction needed

## Problem Statement

The `kibitzer hook` Stop-mode pass ("re-check files touched this task") bleeds edits
from concurrent Claude Code sessions. When two or more sessions run simultaneously and
both fire their Stop hooks, the shared `~/.cache/kibitzer/stop-offsets.json` file is
read-modify-written without any locking. The second writer clobbers the first writer's
updated offset entry. On the next Stop event for the clobbered session, its transcript
is re-processed from the stale (old) offset, re-evaluating transcript content from an
earlier task window — including file edits from a completely different repo if the
session spanned multiple tasks. The result is findings for files the current task never
touched.

Concrete evidence from the filed report:

```
kibitzer re-checked 48 file(s) touched this task (unscoped — ...):
/Users/tstapler/.stapler-squad/repos/github.com/tstapler/stapler-squad/config/config.go:
  primitive-obsession: ...
```

…surfaced inside a consolette session that never touched `config.go`.

## Root Cause (VERIFIED by reading `src/task_stop.rs`)

`StopOffsets::save` uses `std::fs::write` (non-atomic, no locking) and
`StopOffsets::load` uses `std::fs::read_to_string` (no locking). Two concurrent
sessions each load the same file, compute a new offset for their respective
`transcript_path` key, and both write back — the later write silently discards the
earlier one's updated entry. `StopOffsets` is a single global BTreeMap
`{ transcript_path → byte_offset }` at `~/.cache/kibitzer/stop-offsets.json`.

The code already correctly scopes `touched_files()` to the single `transcript_path`
from the Stop hook payload (this was not always the case, but the current code at
`src/task_stop.rs:156–186` is correct on that axis). The remaining bug is the shared
mutable file without concurrency control.

## Users / Consumers

kibitzer's own maintainer (single user today, tstapler). The Stop hook fires
automatically on every Claude Code session's Stop event on any machine running kibitzer.

## Success Metrics

1. Two concurrent Stop hook invocations that each update different `transcript_path`
   entries never lose either update — verified by a test that races two concurrent
   `run_stop_hook_with_offsets_path` calls.
2. After a concurrent-write scenario, neither session re-reports findings already
   reported in a prior Stop run.
3. The findings report for session A never contains a file from a repo that
   session A's transcript never mentioned.
4. Per-session offset state survives a concurrent write from a completely unrelated
   session without corruption.

## Acceptance Criteria

1. `StopOffsets` persistence is redesigned so that concurrent writes from different
   sessions cannot clobber each other — specifically, no single shared file contains
   entries for multiple sessions.
2. Each session's offset state is stored in a file whose path is uniquely derived from
   the `transcript_path` — no coordination between sessions is needed at read or write
   time.
3. Existing tests continue to pass; a new concurrency regression test covers two
   simultaneous calls with distinct transcripts and confirms both offsets are persisted.
4. The per-session offset file is cleaned up (or at minimum does not grow unboundedly)
   as old session transcripts age out — a documented garbage-collection strategy, even
   if not implemented immediately, must be written in the plan.
5. No behavioral change for a single-session, non-concurrent use of the Stop hook.
6. The fix is pure-Rust, no new dependencies — file locking via `flock`/`fcntl` would
   add OS-specific surface; per-session files avoid that entirely.

## Appetite

Small — 1–2 days at most. The change is isolated to `src/task_stop.rs` and its tests;
no new abstractions required, no daemon protocol changes, no config surface changes.

## Constraints

- Solo maintainer.
- No new crate dependencies — the fix must use only Rust's std library and the crates
  already in `Cargo.toml`.
- Must not change the externally visible behavior of the Stop hook (output format,
  advisory-only exit code, `additionalContext` schema).
- Old-format `stop-offsets.json` entries must be ignored or migrated without error —
  the new per-session path scheme will produce a different filename, and an unreachable
  stale `stop-offsets.json` is acceptable (it can be deleted by hand or left harmlessly
  in place since it will never be consulted again).

## Non-functional Requirements

- **Performance**: no observable change. A per-session file lookup (`O(1)` path
  derivation, one `read_to_string`, one `write`) is faster than the BTreeMap-over-all-
  sessions approach anyway.
- **Security**: offset files are in `~/.cache`, same trust level as today.

## Out of Scope

- Garbage-collecting old per-session offset files (nice-to-have, not a blocker).
- Fixing any other global-shared-state issue in kibitzer.
- Adding a `session_id` field to `hook-log.jsonl` (mentioned in the bug report as a
  separate signal gap, but this fix doesn't rely on `hook-log.jsonl` at all).
