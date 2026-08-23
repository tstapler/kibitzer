# Phase 7.2 Adoption Proof — Findings

**Status**: Executed 2026-08-23, verification-only against a read-only clone of
`tstapler/stapler-squad`. Per Epic 7.2's scope in `plan.md`, this produced **no commit to
kibitzer's own repo** — the `.claude/inspect.json` architecture blocks used were scratch,
uncommitted, and live only in the session transcript that ran them. This document persists
those findings so they aren't lost.

## Most urgent finding: stapler-squad's own `depguard` config is currently non-functional in CI

**This is independent of kibitzer** and worth acting on regardless of whether kibitzer's
`component-deps` checker is ever adopted there.

`golangci-lint run --enable-only depguard ./...` (v2.12.2) reported **0 issues** across all three
`depguard` rules (`no_server_in_core`, `no_ent_in_services`, `no_ioutil`) in the real
`stapler-squad` repo. That result was not taken at face value — a synthetic, unambiguous violation
was injected (`server/services/zz_kibitzer_verify_probe.go` importing `session/ent`) and depguard
**still** reported 0 issues on it.

Root cause was isolated in a throwaway 3-file Go module reproduction outside stapler-squad:

- A `depguard` rule with `deny` alone fires correctly.
- The identical rule with a `files:` glob added — every form tested, including stapler-squad's
  exact literal patterns — never matches.
- `files: ["$all"]` (what `no_ioutil` uses) works fine.

**Conclusion**: `no_server_in_core` and `no_ent_in_services` are currently inert in stapler-squad's
real CI — a `depguard`/`golangci-lint` `files`-glob regression. Their "0 issues" verdict means "the
rule never ran," not "the code is clean." `no_ioutil` itself was separately confirmed to work
correctly (fires on a synthetic `io/ioutil` import) — it doesn't use a `files:` glob, which is
consistent with the isolated repro.

**Recommendation** (a suggestion, not an instruction — fixing another repo's CI is out of scope for
kibitzer's own implementation): file this as a separate issue against stapler-squad's CI, or try
upgrading/pinning `golangci-lint` to a different version and re-testing whether the `files:` glob
regression persists.

## Summary of the two migrations

Both `no_server_in_core` and `no_ent_in_services` were migrated to kibitzer's `component-deps`
schema and run for real against the stapler-squad clone. In both cases, kibitzer's literal
`Component.paths` values as written in `plan.md` matched **zero** import-graph nodes on the first
attempt, revealing a kibitzer schema-authoring gap (below). Once corrected, both migrations agree
with depguard on production code — once depguard's real exclusions and its currently-broken state
are both accounted for — and kibitzer additionally surfaces real violations depguard is currently
missing (test-file imports in 7.2.1; the negation-glob-excluded files in 7.2.2 in far larger number
than expected, plus 11 files not on the exclusion list at all).

## Story 7.2.1: `no_server_in_core`

The plan's literal config (`Component.paths: ["session/**", "config/**", "log/**"]` for `core`,
`["server/**"]` for `server`) matched 0 import-graph nodes at first. Cause: kibitzer's Go extractor
uses full module-qualified import paths as graph nodes, and a glob needs a `**/` prefix to match
anywhere in that path, not just at the start.

A second gap surfaced even after adding the `**/` prefix: `**/session/**` still doesn't match the
top-level `session` package itself, only its subpackages. A component needs **both** a bare glob
(`**/session`) **and** a `/**` glob (`**/session/**`) to fully cover a package and its
subpackages — the same two-glob idiom already used in kibitzer's own `src/run.rs` test fixtures.

With the corrected config, kibitzer found **3 real violations**, all in `_test.go` files:

- `session/storage_backlog_events_test.go:17`
- `session/host_advertisement_convergence_test.go:10`
- `session/ent_repository_backlog_events_test.go:13`

All three import `server/...` packages — confirmed genuine by reading the actual import lines, not
inferred from the tool output alone.

**Divergence from depguard, fully explained**: depguard's real config includes `"!**/*_test.go"`,
an exclusion kibitzer's `Component.paths` has no equivalent for (it doesn't support negation
globs). Once that's accounted for alongside depguard's broken/inert state (see above), the two
tools agree: **zero production-code (non-test) violations either way**. Kibitzer additionally
surfaces the 3 test-file imports that depguard would silently allow even if it were functional —
whether that's desirable is Tyler's call, not evaluated here.

## Story 7.2.2: `no_ent_in_services`

Same 0-node-match gap, same corrected-config approach:

- `services: ["**/server/services", "**/server/services/**"]`
- `ent: ["**/session/ent", "**/session/ent/**"]`

Real result: **20 files flagged**.

Against the plan's list of 10 grandfathered files (files depguard's real config excludes via
negation globs kibitzer can't reproduce), **9 of 10** were flagged as new violations:

- `error_registry.go`
- `analytics_escape_service.go`
- `analytics_escape_service_test.go`
- `workflow_service_test.go`
- `workflow_service.go`
- `backlog_service_query.go`
- `backlog_service_lifecycle.go`
- `backlog_service_triage.go`
- `session_service.go`

This is real, previously-unknown data confirming the scale of the negation-glob gap — the plan
only predicted that the gap existed, not how many of the 10 it would actually hit.

**The 10th file, `backlog_service.go`, was the one exception** — confirmed by reading the file that
it no longer imports `session/ent` directly. Its grandfather-exclusion in stapler-squad's real
config is now stale/unnecessary.

**Additionally, 11 more files were flagged that are not on the grandfather list at all**:

- `webhook_trigger_common.go`
- `session_summary_service.go`
- `session_summary_service_test.go`
- `github_webhook_handler.go`
- `github_webhook_handler_test.go`
- `generic_webhook_handler_test.go`
- `backlog_service_sync.go`
- `backlog_service_ship_status.go`
- `backlog_service_ship.go`
- `backlog_service_pipeline_mode.go`
- `backlog_service_pipeline_mode_test.go`

These are real `session/ent` imports that depguard's rule, if functional, should also be flagging —
they aren't excluded by its config either. Since depguard's rule is currently inert (see top
finding), it's silently missing all of these too, independent of the negation-glob gap.

## Story 7.2.3: `no_ioutil`

Confirmed intentionally not migrated. It's a global/unscoped ban (`files: ["$all"]`) with no source
component, and kibitzer's `DependencyRule` schema (keyed by a source component) has no equivalent
for an unscoped rule. Stays on `depguard`/`golangci-lint`.

Separately confirmed: `no_ioutil` is itself one of the working depguard rules (fires on a synthetic
`io/ioutil` import), unlike the two migrated rules — consistent with the `files:`-glob regression
theory, since `no_ioutil` doesn't use a `files:` glob.

## Bottom line

- Both migrations agree with depguard on production code, once depguard's real exclusions and its
  currently-broken state are both accounted for.
- Kibitzer's `component-deps`, once configured with corrected globs, is currently the **more
  functional** of the two checkers on stapler-squad's real codebase — depguard's equivalent rules
  are silently non-functional in its CI right now.

## Findings about kibitzer itself (separate from the stapler-squad CI bug)

1. **Glob-prefixing/pairing UX gap**: `Component.paths` globs must be written with a `**/` prefix
   to match anywhere in a module-qualified import path, and a component covering both a package and
   its subpackages needs a paired bare-glob + `/**`-glob (`**/session`, `**/session/**`), not a
   single pattern. This is a real deviation from the plan's literal schema text, which used bare
   `session/**`-style globs that matched nothing. Not fixed as part of this task — worth
   documenting or improving separately (e.g. auto-deriving the paired form, or defaulting to
   substring/anywhere matching).
2. **No exclusion-glob support**: `Component.paths` has no negation-glob mechanism, so it can't
   express depguard's `!**/*_test.go` or its 10-file grandfather list. This is a legitimate future
   scope item per the plan, confirmed real by this verification, not added here.
