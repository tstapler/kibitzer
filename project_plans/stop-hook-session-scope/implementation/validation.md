# Validation: stop-hook-session-scope

**Adversarial review pass — 2026-10-17**

## Claims in the plan that could be wrong

### Claim 1: "Sessions A and B never contend on the same file"
**Verdict: TRUE**, provided Claude Code gives each session a unique `transcript_path`
basename. This is a hard assumption. Confirmed by inspecting Claude Code's transcript
naming convention documented at `src/backtest.rs:2` (`~/.claude/projects/*/*.jsonl`) —
each session writes to its own JSONL file under the project's directory, with a UUID-
like session ID as the basename. The basename is unique per session.

**Edge case**: Two worktrees of the same repo might use the same session ID if symlinked
or if Claude Code reuses session IDs across projects. UNVERIFIED — the code does not
inspect the transcript name format in any normative way. However, even if this edge case
occurs, the worst outcome is the old bug (offset clobber between two sessions that share
a project and happen to have the same session file basename) — not a new regression. The
fix is strictly better or equal in all cases.

### Claim 2: "The sanitized basename is filesystem-safe to 255 bytes"
**Verdict: TRUE for typical Claude Code session files.** UUID-like names (`<hex>.jsonl`)
are 16–36 chars + `.jsonl` ≤ 42 chars. With `.json` suffix appended = 47 chars. Well
under 200-char truncation limit and 255-byte filesystem limit.

**Edge case**: If `transcript_path` has no filename component (e.g., a directory path),
`file_name()` returns `None` and the code falls back to `as_os_str()`, which for a
directory path includes path separators. The sanitizer replaces `/` with `_`, so `_home_
_user_.jsonl` is valid. Not a regression — just an unusual key.

### Claim 3: "First Stop run post-upgrade re-processes from offset 0 — harmless"
**Verdict: MOSTLY TRUE.** Re-processing from offset 0 means re-evaluating the whole
session's transcript. For a short session, this is a trivially small re-run. For a very
long session (many hours, thousands of edits), it could produce a large list of
"touched files" and re-run checks on all of them.

**Risk mitigation**: The Stop hook already has `predates_git_head` suppression — findings
already at HEAD are filtered out, so re-reporting finds only those newly introduced THIS
task, same as normal behavior. The re-run is noisier on a long session's first post-
upgrade Stop event, but not wrong.

**Not a regression** relative to the current bug, where a race ALSO causes re-processing
from an old offset. This is a known, one-time upgrade effect; the race condition produced
this unpredictably on every run.

### Claim 4: "No public API change"
**Verdict: TRUE.** `hook.rs:173` calls `crate::task_stop::run_stop_hook(&input.cwd,
input.transcript_path.as_deref())` — this signature is unchanged. The internal helper
`run_stop_hook_with_offsets_path` used only in tests changes its signature
(offset file path → offsets directory), but it's `fn` not `pub fn`, so only in-crate
tests are affected.

### Claim 5: "Existing tests continue to pass"
**Verdict: REQUIRES CAREFUL TEST MIGRATION.** Tests currently call
`run_stop_hook_with_offsets_path(cwd, transcript, &scratch_offsets_file)` with a file
path. After the refactor, the injectable param is a DIRECTORY (not a file). Tests must
be updated to pass a scratch directory. This is mechanical but non-trivial: 4 test
cases reference `&offsets` as a file path. The test author must ensure all 4 are updated
before `cargo test`.

### Claim 6: "Concurrency regression test is feasible in Rust's std test framework"
**Verdict: TRUE** but with caveats. `std::thread::spawn` + `JoinHandle::join` works
for a two-thread concurrency test. The test needs:
1. Two distinct transcript files with pre-written content.
2. Two threads each calling `run_stop_hook_impl` with the same `offsets_dir` but
   different `transcript_path`.
3. After both threads join, assert both per-session offset files exist at the expected
   paths.

The test does NOT prove absence of races under high contention (only under 2 concurrent
threads). Sufficient for the stated requirement.

## Gaps / open questions

1. **Lazy GC not in scope**: Per-session files accumulate. The plan documents this as a
   known follow-up. Acceptable — the accumulation rate is negligible for a personal tool.
2. **No test for the actual cross-session clobber bug**: The plan adds a concurrency
   test that verifies both offsets are persisted, but does NOT reproduce the original
   bug (finding from session B appearing in session A's report). This is intentional —
   reproducing the original bug requires live transcripts and real check-running, which
   is impractical in a unit test. The architectural proof (per-session files, no shared
   state) is sufficient.
3. **Migration of old `stop-offsets.json`**: Not done. Users who upgrade will see a
   one-time re-processing on the next Stop event per session. Document in CHANGELOG.

## Adversarial failure modes considered

- **What if `transcript_path` is the same for two different Claude Code sessions?**
  Unlikely by design, but if it happened, the two sessions would share a per-session
  offset file — same race as before, but only between those two specific sessions (not
  all sessions). Not worse than today.
- **What if the `stop-offsets/` directory can't be created?** Same behavior as today —
  the offset falls back to 0, re-processing from the beginning. Acceptable degradation.
- **What if the per-session offset file is corrupt (e.g., partial write)?**
  `SessionOffset::load` falls back to `default()` (offset=0) on any parse error — same
  as `StopOffsets::load` does today. No regression.

## Verdict

Plan is sound. All major claims hold. Risks are low and bounded. Proceed to implementation
with the one note: update ALL 4 test call sites in `task_stop.rs` that reference the
injectable offset path, not just the obvious ones.
