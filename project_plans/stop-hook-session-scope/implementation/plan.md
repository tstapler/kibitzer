# Plan: stop-hook-session-scope

**Status**: planning
**Date**: 2026-10-17

## Problem summary

`~/.cache/kibitzer/stop-offsets.json` is a single shared file read-modify-written by
every concurrent Stop hook invocation without any locking. Concurrent sessions clobber
each other's offset updates, causing a session to re-process already-seen transcript
content and surface findings for files from earlier tasks (possibly in other repos).

## Design

Replace the single global BTreeMap offset file with **one per-session file**, keyed by
a sanitized basename of `transcript_path`. No locking needed: each session owns exactly
one file and never contends with another session.

### New type

```rust
#[derive(Debug, Default, Serialize, Deserialize)]
struct SessionOffset {
    offset: u64,
}
```

### New offset-file path derivation

```rust
fn session_offsets_path(transcript_path: &Path) -> PathBuf {
    let base = transcript_path
        .file_name()
        .unwrap_or_else(|| transcript_path.as_os_str());
    let safe: String = base
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' })
        .collect();
    // Truncate to 200 chars to stay well within 255-byte filename limit after appending .json
    let safe = if safe.len() > 200 { &safe[..200] } else { &safe };
    let dir = crate::cache::default_cache_path()
        .parent()
        .map(|p| p.join("stop-offsets"))
        .unwrap_or_else(|| std::env::temp_dir().join("kibitzer-stop-offsets"));
    dir.join(format!("{safe}.json"))
}
```

### Updated `run_stop_hook_with_offsets_path`

The function signature changes to remove the injected `offsets_path` and instead derive
it from `transcript_path`. For testability, keep an inner helper that accepts an explicit
`offsets_dir: &Path` so tests can redirect to a scratch directory.

```rust
// Public (production) entry point
pub fn run_stop_hook(cwd: &Path, transcript_path: Option<&Path>) -> Result<ExitCode> {
    let offsets_dir = default_offsets_dir();
    run_stop_hook_impl(cwd, transcript_path, &offsets_dir)
}

// Testable inner entry point
fn run_stop_hook_impl(
    cwd: &Path,
    transcript_path: Option<&Path>,
    offsets_dir: &Path,
) -> Result<ExitCode> { ... }
```

## Stories

### 3.1 Replace `StopOffsets` BTreeMap with `SessionOffset` scalar

- Remove the `StopOffsets` struct and its `entries: BTreeMap<String, u64>`.
- Add `SessionOffset { offset: u64 }` with `load(path)` / `save(path)` using
  `serde_json`, same error-tolerance as the old implementation.
- Derive `session_offsets_path(transcript_path)` → per-session file in
  `~/.cache/kibitzer/stop-offsets/<sanitized-basename>.json`.

### 3.2 Update `run_stop_hook_with_offsets_path` to use per-session file

- Load `SessionOffset` from the per-session path (or default to `{ offset: 0 }`).
- After reading new lines, save the updated `SessionOffset` to the same path.
- No BTreeMap load/merge/save cycle — just a single-key read and write.

### 3.3 Update call site in `hook.rs` (if signature changed)

- `hook.rs:173` calls `crate::task_stop::run_stop_hook(...)` — confirm signature is
  unchanged; only the internal implementation changes.

### 3.4 Update tests

- Replace `run_stop_hook_with_offsets_path(cwd, transcript, &offsets_file)` calls in
  `task_stop.rs` tests with the new `run_stop_hook_impl(cwd, transcript, &offsets_dir)`
  signature (passing a scratch *directory*, not a file path).
- Verify `StopOffsets::load` / `save` tests no longer apply; delete them.
- Add a **concurrency regression test**: spawn two threads each calling
  `run_stop_hook_impl` with distinct transcripts in the same `offsets_dir`; assert both
  per-session offset files exist and have the correct offsets after both threads complete.

### 3.5 Document GC strategy (comment only, no implementation)

- Add a `// TODO(gc)` comment in `session_offsets_path` describing the lazy GC strategy:
  on each Stop invocation, after writing the current session's offset, glob
  `stop-offsets/*.json` and remove any whose transcript JSONL no longer exists.
- Out of scope for this PR; tracked as a follow-up.

## Migration

Old `stop-offsets.json` is silently abandoned. It will never be consulted again (the new
code looks for per-session files, not the old map). The only side effect: on the first
Stop hook run after the upgrade, a session re-processes its transcript from offset 0.
For most sessions that means re-evaluating an already-seen window, which at worst
surfaces advisory findings that have already been seen. Acceptable one-time behavior;
no user action required.

## Risk

**Low.** The change is entirely within `src/task_stop.rs`. No public API change. No
config change. No daemon protocol change. The hook's output format (`additionalContext`
JSON) is unchanged. A failed write to the per-session file degrades the same way as
today: the next Stop run re-processes an already-seen window (the stale-offset behavior
the fix is designed to eliminate, but no worse than the current state).

## Acceptance gate

`cargo test` green. A new test in `task_stop.rs::tests` explicitly verifies that two
concurrent invocations writing to the same `offsets_dir` do not lose either session's
offset.
