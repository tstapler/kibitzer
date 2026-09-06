# ADR-002: The architecture model cache lives in the owning process (MCP server / LSP `Backend`), not in `src/daemon.rs`

**Status**: Accepted
**Date**: 2026-08-23

## Context

Building `ArchModel` (`src/arch_model.rs`) — a whole-repo, package+symbol-level tree —
is strictly more expensive than one file's checks, and is invalidated by a change to
*any* in-scope file, not just one. `src/daemon.rs`'s existing `Cache`
(`src/cache.rs`) is per-file, per-trigger, keyed by one file's `(mtime, len)` stamp
(`Stamp` at `src/cache.rs:15-19`); `Cache::get`/`put` resolve against exactly one file
path per call. `src/lsp.rs`'s `Backend` doesn't route through the daemon at all today —
`diagnostics_for_file` calls `run_checks_for_trigger` directly.

Both the MCP server (`KibitzerServer`, `src/mcp.rs`) and the LSP server (`Backend`,
`src/lsp.rs`) are already long-lived, single-process, single-session servers: one
`kibitzer mcp` invocation serves an entire MCP session over stdio; one `kibitzer lsp`
invocation serves an entire editor session. Within either process's lifetime, rebuilding
`ArchModel` from scratch on every tool call/LSP request reintroduces exactly the
"transient, recomputed every call" problem `architecture_assessment` already has today,
which this feature exists to fix.

## Decision

Cache the built `ArchModel` in-memory, inside the process that owns it, keyed by
`(repo_root, include_private)` only — **not** by `scope`. `build_model` itself never takes
a `scope` parameter (see the implementation plan's Story 1.3.1); the CLI export path
already establishes the correct pattern of "build the full, unscoped model once, then apply
`ArchModel::filtered(scope, level)` per call" (Task 2.1.1b). The cache must mirror that:
keying per-`scope` would fragment one repo's cache into a separate rebuild for every
distinct `scope`/`package` value a caller passes across a session (e.g. an agent calling
`list_architecture_symbols` once per package it inspects) — reintroducing exactly the
"recomputed every call" cost this ADR exists to eliminate.

- `KibitzerServer` and `Backend` (`src/lsp.rs`) each hold a **single-slot** cache:
  `Mutex<Option<(ModelCacheKey, CachedModel)>>`. `KibitzerServer`'s populates lazily on the
  first `list_architecture_symbols`/`get_architecture_node` call; `Backend`'s populates via
  a background task starting at the `initialized()` LSP notification (not on the first
  `workspace/symbol` request — see the implementation plan's Epic 4.3 for why an inline
  build on the request path was rejected) and is invalidated (marked dirty, rebuilt in the
  background, not synchronously) on `did_save` for any in-scope file.
- `ModelCacheKey { repo_root: PathBuf, include_private: bool }` — `scope` and `level` are
  **not** part of the key; every caller applies `.filtered(scope, level)` to the cached,
  unscoped `Arc<ArchModel>` after a cache hit.
- A request whose `ModelCacheKey` doesn't match the slot's current key (a different
  `repo_root`, or a caller flipping `include_private`) replaces the slot outright — a
  v1-acceptable rebuild, not an unbounded cache: at most one `ArchModel` is held in memory
  per process at any time, so there's no eviction policy to design. Toggling
  `include_private` mid-session is expected to be rare (an explicit opt-in flag, not a
  default that varies per call), so the occasional eviction-and-rebuild this causes is an
  acceptable trade-off for v1's simplicity.
- `CachedModel` bundles the `ArchModel` with a `Vec<(PathBuf, Stamp)>` file-stamp set
  (reusing `cache.rs`'s existing `Stamp` shape) covering every file that went into the
  build. A cache hit requires both a matching `ModelCacheKey` and every file's stamp to
  still match — the same cheap stat-based invalidation `daemon.rs`'s per-file cache already
  uses, just applied to a whole file set instead of one file.
- This cache is **not** persisted to disk and **not** shared across separate CLI
  invocations (`kibitzer architecture export` always builds fresh — it's a one-shot
  process, so there's nothing to reuse across calls within it). It does not extend
  `src/cache.rs`'s `Cache` struct or add a new `daemon.rs` RPC verb.

## Alternatives Rejected

- **Extend `daemon.rs`'s persisted per-file `Cache` schema** to hold a whole-repo model
  entry — rejected because `Cache`'s shape (one entry per `(file_path, trigger)`,
  invalidated by that one file's stamp) doesn't fit a repo-scoped model without either
  (a) one entry per file plus a reassembly step, or (b) a single entry invalidated by any
  file touch, which defeats incrementality either way. `lsp.rs` also doesn't use
  `daemon.rs` today, so routing LSP symbol support through the daemon socket protocol
  would be new integration surface on top of new caching logic — two new things instead
  of one.
- **Recompute `ArchModel` on every call, no cache** — rejected as it directly
  reintroduces the "recomputed every call" problem the requirements doc's Problem
  Statement identifies as the core gap in today's `architecture_assessment`.
- **A new daemon RPC (`BuildArchModel`) sharing one model across CLI/MCP/LSP processes
  in the same session** — considered as the "most correct" long-term answer (one build,
  shared everywhere), but rejected for v1 as it requires new daemon wire-protocol surface,
  a new persistence/invalidation design for the daemon's `Cache`, and cross-process
  synchronization the appetite (Large, 3–6 weeks) doesn't have headroom for alongside
  7-language symbol extraction and three consumer interfaces. Flagged as a natural v2
  follow-up once the in-process cache's real hit rate is observed.

## Consequences

- A user running `kibitzer mcp` and `kibitzer lsp` against the same repo in the same
  session pays for building `ArchModel` twice (once per process), not once. Acceptable
  for v1 given the daemon-sharing alternative's cost, and consistent with `lsp.rs`
  already not sharing `daemon.rs`'s cache for diagnostics today.
- `kibitzer architecture export` always does a fresh build — there is no cross-invocation
  caching for the CLI export path, matching every other one-shot kibitzer CLI command's
  behavior today (no CLI command currently reads from `daemon.rs`'s cache).
- **Directory walk still happens on every `get_or_build` call, cache hit or not** (flagged
  across two earlier review rounds, now resolved as an accepted v1 tradeoff rather than
  left open): the caller must walk the workspace and stat each file's `Stamp` *before*
  calling `get_or_build`, so it can pass `files: &[PathBuf]` for the stamp-set comparison —
  only the parse/extract step (`build_model` itself) is skipped on a cache hit, not the
  walk. This is intentional: a stat-only directory walk is IO-cheap relative to per-file
  tree-sitter parsing, which is the expensive step this cache exists to avoid repeating.
  Caching the file list itself (or file-watcher-driven invalidation) so the walk can also
  be skipped on a hit is deferred as a v2 follow-up alongside the daemon-RPC-sharing
  alternative above, not built for v1.
