# ADR-002: Build-Cost Collector Uses `-debug-actiongraph` with Version Gate

**Status**: Accepted
**Date**: 2026-10-23

## Context

Measuring Go compilation cost requires access to the build graph (packages compiled,
cache hits, build times). Go exposes two internal mechanisms:

1. `-debug-actiongraph <file>` (internal, JSON, available since Go 1.19): writes the
   build action graph to a file. Undocumented, schema can change, not supported on all
   toolchains.
2. `-toolexec <cmd>` (stable, documented): wraps every compiler/linker invocation;
   timing is measured by the wrapper, not built into Go itself.

The requirements specify a "prototype" with a version gate, not a production-stable
implementation. The build-cost collector must not clear the user's Go build cache
(`go clean -cache`), and must never run as part of per-edit lint hooks.

## Decision

Use `-debug-actiongraph` behind a version gate (`go version` output ≥ `go1.19`). Write
the action graph to a temp file (not `/dev/null`) for cross-platform compatibility
(Windows). Degrade gracefully when the flag is unsupported: print a diagnostic and exit 0
rather than failing with a cryptic error.

If the prototype proves reliable across Go 1.19–1.24, promote `-toolexec` as the
stable production approach in a follow-up.

## Alternatives Rejected

**`-toolexec` for the prototype**: `toolexec` requires a wrapper binary, adds subprocess
overhead, and produces timing data the wrapper must parse from its own arguments —
substantially more implementation complexity for a prototype that only needs a structural
signal. Deferred as the stable path once `-debug-actiongraph` validates the design.

**`go list -json -deps` only**: reports the dependency graph but not compilation timing or
cache-hit status. Insufficient for cost comparison (cold/warm/edited-source workflows).

## Consequences

- The CLI subcommand (`kibitzer go build-cost`) calls `go version` first; skips
  `-debug-actiongraph` if the version is below 1.19.
- The temp file path is OS-generated (`std::env::temp_dir()`), not hardcoded.
- No `go clean -cache` is issued before or after a run; the tool measures warm-cache
  or cold-cache depending on the caller's environment.
- A future stable implementation migrates to `-toolexec` without user-visible behavior
  changes.
