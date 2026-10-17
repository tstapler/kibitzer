# Research: Stack Inventory

## Relevant crates (Cargo.toml)

Checked via `grep` on the workspace `Cargo.toml`. Key findings:

- **`serde` / `serde_json`**: already present, used by `StopOffsets` today — no change needed.
- **`sha2` / `sha1` / any hash crate**: NOT present in `Cargo.toml`. Confirming the
  no-new-dep constraint: we cannot use a SHA-based filename without adding a crate.
- **`std::collections::hash_map::DefaultHasher`**: available in std, but its output is
  explicitly not stable across Rust compiler versions (the doc says it "may change in
  future releases"). Writing its output to disk and reading it back after a binary upgrade
  would produce silently wrong (stale) offsets. Ruled out for on-disk keys.

## Chosen approach: sanitized basename of transcript_path

Claude Code names transcript files `<session-id>.jsonl` where the session ID is a UUID-
like string unique per session. The filename component of `transcript_path` is already
globally unique per session on the machine.

Sanitize: replace path separators and any character outside `[a-zA-Z0-9._-]` with `_`.
Maximum length: 255 bytes (filesystem limit); the UUID-like names Claude Code uses
(`<hex>.jsonl`) fit well under this.

Resulting per-session offset file path:
`~/.cache/kibitzer/stop-offsets/<sanitized-basename>.json`

Example:
- `transcript_path`: `/home/user/.claude/projects/-home-user-code-consolette/abcd1234.jsonl`
- Offset file: `~/.cache/kibitzer/stop-offsets/abcd1234.jsonl.json`

This is human-readable, needs no external crate, and the uniqueness guarantee is
inherited from Claude Code's own session-ID uniqueness.

## Per-session file format

The current `StopOffsets` type stores a BTreeMap to share one file across sessions.
With one file per session, the stored value is just a single `u64` offset. A minimal
JSON: `{"offset": 12345}` or even the plain integer string `12345\n`.

To minimize churn, keep the existing `serde_json` serialization but with a new
single-field struct:

```rust
#[derive(Debug, Default, Serialize, Deserialize)]
struct SessionOffset {
    offset: u64,
}
```

## Garbage collection

Per-session files accumulate at one per Claude Code session. A typical interactive day
might create 5–20 sessions. After a year: ~1000–5000 files, each a few bytes — trivially
small. GC strategy (documented, not necessarily implemented in this PR):

- Delete per-session offset files whose corresponding transcript JSONL no longer exists
  on disk (the session was deleted by `claude` or pruned by OS cleanup).
- Can be done lazily: on each Stop hook invocation, after updating the current session's
  offset, glob `stop-offsets/*.json` and delete any whose transcript is gone.
- Lazy GC is safe because the offset file is only ever consulted when the transcript
  still exists (the hook gets `transcript_path` from Claude Code, which only fires when
  the transcript is active).

Immediate GC implementation is out of scope for this PR per requirements; it is
documented here as a known follow-up.
