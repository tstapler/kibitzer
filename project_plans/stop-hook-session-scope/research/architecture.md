# Research: Architecture Analysis

## Relevant code locations

### `src/task_stop.rs`

The entire Stop-hook implementation. Key types and functions:

- **`StopOffsets`** (line 43): `BTreeMap<String, u64>` keyed by transcript path string.
  Loaded and saved via `std::fs::read_to_string` / `std::fs::write` — neither is atomic
  or locked.
- **`default_offsets_path()`** (line 65): returns `~/.cache/kibitzer/stop-offsets.json`
  (or `$XDG_CACHE_HOME/kibitzer/stop-offsets.json`). Single shared file on the machine.
- **`run_stop_hook_with_offsets_path`** (line 156): loads offsets → reads new transcript
  lines → writes updated offsets → calls `stop_hook_findings_for_files`. The load-modify-
  write is a classic TOCTOU race: no lock held across the three steps.
- **`touched_files`** (line 109): correctly scoped to only the single `transcript_path`.

### `src/hook.rs`

- `HookInput` (line 16): deserializes the Stop hook payload from stdin, including
  `transcript_path: Option<PathBuf>` (line 34).
- `run_hook` (line 165): dispatches to `crate::task_stop::run_stop_hook(&input.cwd,
  input.transcript_path.as_deref())` when `hook_event_name == "Stop"`.

## Race condition walk-through

```
t=0: Session A (consolette)  load → {transcript_A: 1000, transcript_B: 500}
t=0: Session B (stapler-sq)  load → {transcript_A: 1000, transcript_B: 500}
t=1: Session A reads transcript_A[1000..2000], computes new offset 2000
t=1: Session B reads transcript_B[500..800], computes new offset 800
t=2: Session A writes → {transcript_A: 2000, transcript_B: 500}
t=3: Session B writes → {transcript_A: 1000, transcript_B: 800}  ← A's update lost!
t=4: Session A fires Stop again → loads {transcript_A: 1000} → re-processes 1000..2000
     → touched_files finds files edited earlier in the same session that may be from
       a prior task in a different repo.
```

## Why per-session files fix this

If each session's offset is stored in its OWN file (e.g.,
`~/.cache/kibitzer/stop-offsets/<sha256-of-transcript-path>.json`), then sessions A and
B never contend on the same file. Each session reads and writes only its own per-session
file. There is no shared mutable state across sessions. The race disappears entirely
without any locking, atomicity, or coordination primitives.

## Atomic write alternative

An atomic rename (`write to tmp → rename`) would fix the clobber problem for a single
session's multi-concurrent invocations (e.g., two Stop events for the same session
within milliseconds, which is unlikely), but does NOT fix the cross-session clobber
because both sessions would still be renaming to the SAME destination path. Only a
per-session file fully eliminates cross-session contention.

## File locking alternative

`flock`/`fcntl` advisory locks would work but add OS-specific surface and require either
a separate `libc` crate call or introducing `fs2`/`file-lock` as a new dependency. Both
are ruled out by the requirements' no-new-deps constraint.

## Impact surface

The fix is entirely inside `src/task_stop.rs`:
- `default_offsets_path()` → replace with a per-session function that takes the
  transcript path and derives a deterministic filename from it.
- `StopOffsets` struct → simplify: per-session file needs only a single `u64` (the
  offset), not a BTreeMap. The BTreeMap was only needed because one file held offsets for
  all sessions.
- `StopOffsets::load` / `StopOffsets::save` → operate on the single-offset file.
- `run_stop_hook_with_offsets_path` signature → replace `offsets_path: &Path` with an
  approach where the path is derived internally from `transcript_path` (or keep the
  path injectable for tests, but derive it from a hash of the transcript path rather than
  passing the global shared path).
- Tests: update to use the new per-session path derivation; add a concurrency test.

No other file needs to change. The public API (`run_stop_hook`, called from `hook.rs`)
does not change its signature or behavior.

## Path derivation strategy

Use a truncated SHA-256 or SHA-1 of the transcript_path bytes as the filename:
`~/.cache/kibitzer/stop-offsets/<hex>.json`. The Rust standard library has no built-in
SHA-256, but `Cargo.toml` already has `sha2` as a transitive dependency or... let me check.

Actually, to avoid any new dependency, use a simple `DefaultHasher`-based approach (stable
within a binary run but not across runs — not suitable) OR use a path-safe encoding of the
transcript path itself (e.g., percent-encode or replace `/` with `_`, truncated to a
filesystem-safe length). The simplest approach is to hash the transcript path with
`std::collections::hash_map::DefaultHasher` noting that its value may differ across Rust
versions — for an on-disk key this is undesirable.

A better approach: take the transcript path's last two components
(`<project-dir>/<session-id>.jsonl`) — those are already unique per session by Claude
Code's naming convention — and sanitize them into a valid filename:
`stop-offsets/<sanitized-session-id>.json`. This is human-readable and avoids any hash.

Alternatively, store the file at:
`~/.cache/kibitzer/stop-offsets/<sanitized-transcript-basename>.json`
where the basename is the JSONL session file's own filename (which Claude Code already
makes unique per session). This is the simplest and most debuggable option.
