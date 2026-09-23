# Requirements: affected-tests

**Date**: 2026-09-22
**Type**: feature addition
**Complexity**: 3 — system design (new CLI surface built on existing graph infra, with a
correctness-critical bail-out contract)

## Problem Statement

Every repo that wants fast PR-time CI ends up hand-rolling "which tests does this diff
actually touch." stapler-squad's concrete instance is `scripts/test-affected.py`: walk
`go list -deps`/importers from the changed `.go` files vs. a base SHA, with a `__ALL__`
fallback for changes to `go.mod`/`go.sum`/`Makefile`/`.golangci.yml`/proto files/the
script itself (unbounded-blast-radius files). It works, but it's Go-only, lives in one
repo, has no shared test coverage of its own, and any other repo wanting the same thing
(JS/TS via `jest --changedSince` composition, etc.) starts from scratch.

kibitzer already builds most of the graph this needs for other checks: `import_graph.rs`
(directed package/module import edges, Go/TS/JS/Java/Kotlin/Python) and `arch_model.rs`
(`ArchModel.import_edges`, built on top of it, consumed today by `Architecture Export`,
`Architecture Diagram`, and the MCP `list_callers`/`list_callees`/`get_architecture_node`
tools per [src/main.rs](https://github.com/tstapler/kibitzer/blob/master/src/main.rs) and
[src/mcp.rs](https://github.com/tstapler/kibitzer/blob/master/src/mcp.rs)). No existing
kibitzer code walks a git diff against a base ref today (`grep -rn "diff --name-only"
src/*.rs` returns nothing) — `hook.rs`'s `changed_lines` machinery scopes findings to
edited line ranges within a single file's working-tree edit, not a cross-file diff
against an arbitrary base SHA, so that plumbing is new.

The problem this project solves: give kibitzer a `kibitzer affected --base <sha>`
subcommand that walks its existing import graph outward from a diff's changed files to
the affected package/test set, so stapler-squad (and any other kibitzer-adopting repo)
can delete its bespoke script and any other repo gets the same capability for free.

## Baseline

- **`scripts/test-affected.py`** (stapler-squad, referenced in this item's Context) is
  the concrete reference implementation: diff changed `.go` files vs. base SHA, walk
  `go list -deps`/importers outward, print `__ALL__` when the diff touches
  `go.mod`/`go.sum`/`Makefile`/`.golangci.yml`/proto files/the script itself, otherwise
  print the affected package list for `go test $(...)`. Landed alongside stapler-squad
  PR #704, which gates PR-time CI on this narrowed set instead of the full suite +
  coverage threshold (those now run main-only).
- **`digitalocean/gta`** and **Nx's `affected`** solve the same problem for Go and
  JS/TS monorepos respectively, per this item's description — reference prior art, not
  code kibitzer depends on.
- **kibitzer's existing batch-only report family** (`change_coupling.rs`,
  `root_cause_clusters.rs`, `hotspots.rs`, all wired under `Architecture` in
  [src/main.rs](https://github.com/tstapler/kibitzer/blob/master/src/main.rs)) is the
  closest architectural precedent: each shells out to `git log`/`git diff`, is
  Go-only or cross-language per its own scope note, and is explicitly never wired into
  `default_checks()`/hook mode — a "look here" or, in this case, "run these" report
  driven by an explicit subcommand invocation, not a per-edit pass/fail check.
- **`ArchModel`/`import_graph.rs`** already has everything needed for the "walk the
  graph outward" half of the problem (package-level import edges, `file_packages` for
  mapping a changed file to its package key) but nothing for the "diff against a base
  ref" half, and no existing consumer computes reverse-reachability (importers of a
  changed package) — `import_graph.rs`'s `edges_from` only walks forward (a package's
  own imports), per its doc comment.

## Users / Consumers

kibitzer's own maintainer (single user today, tstapler), initially via stapler-squad's
CI (`build.yml`, replacing `scripts/test-affected.py`'s invocation). Any other
kibitzer-adopting repo wanting the same PR-time-CI narrowing is a secondary consumer of
the same subcommand, unmodified.

## Success Metrics

- `kibitzer affected --base <sha>` runs against stapler-squad and produces a package
  list that a CI wrapper can feed to `go test $(...)`, computing the same affected set
  `scripts/test-affected.py` currently does for a representative sample of real
  historical stapler-squad diffs (a backtest-style comparison, not just fixture tests —
  matching this repo's own "no native checker ships without a backtest" bar from this
  file's own CLAUDE.md).
- A diff touching an unbounded-blast-radius input (lockfile, build config, the affected
  logic itself) yields an explicit "run everything" sentinel, never a narrowed set that
  silently under-tests — same guarantee `test-affected.py`'s `__ALL__` fallback gives
  today.
- stapler-squad's `scripts/test-affected.py` and its `build.yml` invocation are
  deletable in favor of `kibitzer affected` without changing PR-time CI's pass/fail
  behavior (validated by running both side-by-side on the same diffs before cutting
  over — the cutover itself is stapler-squad's follow-up work, out of scope here, but
  this project's acceptance bar is that the comparison is *possible*).
- Automated tests cover the graph-walk logic (given a small synthetic import graph and
  a changed-file set, the affected package set is computed correctly) and the bail-out
  triggers (each unbounded-blast-radius file category from the description fires the
  sentinel).

## Appetite

TBD — Phase 3 planning sizes this after research confirms how much of `import_graph.rs`
can be reused as-is (forward edges only) versus needs a reverse-index addition, and
scopes the Go-only v1 vs. later JS/TS support (matching `hotspots.rs`'s precedent of
shipping Go-only first per #15).

## Constraints

- Solo maintainer — no team/budget constraints, but changes must not add ongoing
  maintenance burden disproportionate to a personal tool.
- Must reuse `ArchModel`/`import_graph.rs` rather than building a second, parallel
  dependency-graph representation — kibitzer already has one "shared model all
  consumers read from" (per `arch_model.rs`'s own doc comment); a second one is exactly
  the kind of fragmentation that doc comment exists to prevent.
- Per-language pluggable, matching kibitzer's native-checker architecture — Go first
  (the immediate, concrete use case), not a Go-only permanent ceiling.
- No native checker or feature ships without being run against real-world signal per
  this repo's CLAUDE.md: a check gets `kibitzer check backtest`; this feature, having no
  transcript-shaped edit history to backtest, instead needs its affected-set output
  compared against `scripts/test-affected.py`'s output on real stapler-squad history
  (see Success Metrics) as the equivalent real-world validation step.

## Non-functional Requirements

- **Performance SLO**: must be fast enough for PR-time CI gating (the whole point is
  replacing a full-suite run) — walking the graph outward from a diff should be at most
  comparable to `test-affected.py`'s current `go list -deps` walk, not slower.
- **Scalability**: needs to handle stapler-squad's current repo size without a
  from-scratch full graph rebuild on every CI run if avoidable — kibitzer's daemon
  cache (`cache.rs`, `daemon.rs`) already exists for exactly this kind of reuse; whether
  `affected` can read from it is a research question, not assumed here.
- **Security classification**: internal/personal-use tool; the subcommand reads git
  history and source files already on disk, no new trust boundary.
- **Data residency**: no special requirements.

## Scope

### In Scope

- A `kibitzer affected --base <sha>` subcommand — **resolved in Phase 3 planning**: it lands
  as `kibitzer architecture affected --base <sha>`, nested under the existing
  `ArchitectureAction` enum alongside `Export`/`Diagram`/`ChangeCoupling`, since it's an
  `ArchModel` consumer like those siblings (see `plan.md`'s Story 3.2.1) — that:
  1. Diffs the working tree/PR branch against a base ref to get changed files.
  2. Walks kibitzer's existing `ArchModel`/`import_graph.rs` outward (reverse
     reachability — importers of a changed package, transitively) from those files to
     the affected package/module/test set.
  3. Emits either a package list (consumable by `go test $(...)`) or a bail-out
     sentinel when the diff touches an unbounded-blast-radius input (build config,
     lockfiles, codegen inputs, the affected-computation logic itself) — same shape as
     `test-affected.py`'s `__ALL__` fallback.
- Go support (the immediate stapler-squad use case).
- Automated tests for the graph-walk and each bail-out trigger category.
- A comparison run against `scripts/test-affected.py` on real stapler-squad diffs,
  documented as this feature's real-world validation step (see Success Metrics and
  Constraints).

### Out of Scope

- JS/TS (or any other language) support for `affected` itself — `import_graph.rs`
  already extracts JS/TS/Java/Kotlin/Python edges, so a later extension is cheap, but
  this project's acceptance bar is Go-only, matching `hotspots.rs`'s precedent (#15).
- Actually deleting `scripts/test-affected.py` or rewiring stapler-squad's `build.yml`
  to call `kibitzer affected` — that cutover is stapler-squad's own follow-up PR, once
  this subcommand exists and the comparison in Success Metrics passes.
- Function/test-level granularity (mapping a changed function to the specific test
  functions that exercise it) — this project targets package/module-level affected
  sets, matching `test-affected.py`'s current granularity. Issue #33's proposed
  function-level call-graph, if it lands, is a future refinement, not a prerequisite.
- Any daemon-cache-backed incremental recomputation — if research finds the daemon
  cache can be reused cheaply, great, but a from-scratch graph build per invocation is
  an acceptable v1 if the performance bar (Non-functional Requirements) is still met.

## Rabbit Holes

- **Reverse-reachability isn't just "invert `edges_from`."** A changed file needs to
  resolve to its package key (via `ImportGraph.file_packages`), then the walk needs
  every package that transitively imports *that* package — the graph as currently
  built doesn't materialize this direction. Don't let building a full reverse index
  balloon into a general-purpose graph-query engine when a straightforward BFS/DFS over
  existing edges (reversed at read time) is enough for this use case.
- **The bail-out list is a hazard if hardcoded narrowly.** `test-affected.py`'s
  `__ALL__` triggers (`go.mod`/`go.sum`/`Makefile`/`.golangci.yml`/proto files/the
  script itself) are Go/stapler-squad-specific; kibitzer's version needs a config
  surface (or at minimum a clearly documented, easily extended list) so a *different*
  adopting repo's own unbounded-blast-radius files (its own lockfile, its own build
  config) aren't silently missed. Getting this wrong doesn't fail loud — it silently
  under-tests, which is the one failure mode this whole feature exists to prevent.
- **Base-ref diffing edge cases**: merge commits, a base ref that doesn't share history
  with HEAD (force-pushed rebase), renamed/moved files changing package keys. Don't
  hand-wave these into "assume a clean linear history" without at least stating the
  assumption — `test-affected.py` may already have made calls here worth inheriting
  rather than re-deriving from scratch.
- **Output format bikeshedding.** A package list vs. JSON vs. a shell-ready
  space-separated string are all plausible; matching whatever's cheapest for a CI
  wrapper to consume (probably shell-ready, matching `test-affected.py`'s own output
  contract) avoids a redesign once stapler-squad tries to actually swap it in.

## Alternatives Considered

- **Build `affected` on `ArchModel`/`import_graph.rs`** (this project's direction) —
  reuses the one existing shared dependency-graph representation, keeps kibitzer's
  "one model, many views" pattern (per the architecture-export project's own
  documented convention in this repo's MEMORY.md); the only new work is the
  diff-to-changed-files step and the reverse-walk.
  - kibitzer's own `list_callers`/`list_callees` MCP tools already do
    function-level reverse traversal, more granular than a first version of `affected`
    needs.
- **Port `test-affected.py`'s `go list`-based approach directly, unchanged in
  language, as a new standalone kibitzer module** — lower risk short-term (proven
  logic, direct port), but abandons kibitzer's own `import_graph.rs`/tree-sitter-based
  extraction that already works cross-language, and duplicates a second Go-specific
  dependency-walk mechanism alongside the general one this repo is otherwise
  consolidating everything into. Rejected as fragmentation of the "one model" pattern.
- **Shell out to `digitalocean/gta` or a JS `nx affected`-equivalent as external
  tools, wrapped by kibitzer** — avoids reimplementation, but reintroduces exactly the
  "per-language external tool a user must separately install" problem the
  `checker-plugin-system` project was built to solve for *checks*; `affected` isn't a
  check (no SARIF/`Check.command` dispatch fits naturally), and pulling in an external
  Go dependency just to shell out to it doesn't reduce kibitzer's own maintenance
  surface the way reusing `import_graph.rs` does.

## Feasibility Risks

- The comparison-against-`test-affected.py` validation step (Success Metrics) depends
  on stapler-squad having enough real historical diffs where the two implementations'
  outputs are directly comparable — if `test-affected.py`'s Go-specific heuristics
  (module-path resolution, replace directives) diverge from `import_graph.rs`'s
  tree-sitter-based Go import extraction in ways that change *which* packages are
  affected, that divergence needs to be understood and either reconciled or explicitly
  accepted (with a documented reason) before this ships as a drop-in replacement.
- No existing kibitzer code diffs against an arbitrary base ref (only against
  `git HEAD` for the hook's before/after comparison, per `check.rs`'s
  `check_against_git_head`/`check_native_against_git_head`) — the base-ref diff step is
  net-new surface, not a small extension of something proven.
- Renamed/moved files and non-linear history (rebases, merge commits) are real
  correctness edges for a feature whose entire value proposition is "don't silently
  under-test" — getting this wrong is worse than not shipping the feature at all.

## Observability Requirements

Standard: failures (git diff failure, unresolvable base ref, graph build failure) surface
as clear CLI errors on stderr. No new metrics/alerting needed for a single-user local
tool / CI-invoked subcommand.

## Risk Control

Low-to-moderate. No production rollout; this is an additive, opt-in subcommand — nothing
in `default_checks()`/hook mode changes, and no existing behavior regresses for anyone
who never invokes `kibitzer affected`. The real risk is downstream (stapler-squad's CI
under-testing if `affected` returns a wrong narrowed set once adopted) — mitigated by the
comparison-validation step in Success Metrics happening *before* stapler-squad's own
cutover PR, and by the bail-out sentinel being conservative by design. Rollback is
`git revert` on kibitzer's side; stapler-squad simply doesn't adopt the subcommand if the
comparison step finds a divergence that can't be resolved.

## Open Questions — resolved after Phase 2 research

- **Daemon-cache reuse**: No. `arch_model::ModelCache` (`src/arch_model.rs:721-773`) is
  explicitly in-process/single-slot/no-persistence/no-daemon-RPC (per its own doc
  comment); `daemon.rs`'s protocol has no `ArchModel`-related request type at all, and
  even the existing `Architecture Export`/`Diagram` CLI paths rebuild the model from
  scratch every invocation rather than reading the cache. `affected` gets no daemon-cache
  benefit in v1 — confirmed by both `research/stack.md` and `research/architecture.md`.
  Wiring the daemon to serve models is legitimate future work, correctly kept out of
  scope per this doc's own Scope section.
- **Bail-out config surface**: A new `Config.affected: AffectedConfig { extra_bail_out_globs:
  Vec<String> }` field, mirroring the existing `Config.architecture` nested-block
  precedent (`src/config.rs:375`) and reusing `Check.scope`'s `Vec<String>` +
  `glob::matches_scope` glob-matching convention already used elsewhere in this repo —
  not a new pattern language. The field is additive-only: it's unioned with, never a
  replacement for, the hardcoded defaults (`go.mod`/`go.sum`/`**/*.proto`, all matched
  tree-wide), matching this repo's own `.claude/inspect.json`-overlays-defaults
  convention rather than a plain replace-on-set `Vec<String>` field. Build-tag-bearing
  files are a documented, accepted gap (a content-level signal, not something a path
  glob can express) rather than a defaulted entry. Per `research/architecture.md` and
  `research/features.md`.
- **Output contract**: Newline-separated plain-text package list on stdout (matching
  `git diff --name-only`/`go list`/`test-affected.py`'s own convention), with the literal
  `__ALL__` sentinel string on stdout at exit 0 for the bail-out case — not a distinct
  exit code — since `test-affected.py`'s only real CI consumer
  (`~/Programming/stapler-squad/build.yml:207-323`) already does
  `PKGS="$(python3 scripts/test-affected.py "$BASE_SHA")"` and word-splits `$PKGS`
  unquoted into `go test` args; matching this exactly minimizes stapler-squad's eventual
  cutover diff to a one-line command swap. Distinct exit codes are reserved for genuine
  errors (bad base ref, not a git repo), never for "bail out" or "nothing affected" —
  confirmed by `research/ux.md`.
- **Renamed/moved files and merge-commit diffs**: *(unresolved after Phase 2 research)*.
  `research/pitfalls.md` and `research/architecture.md` both confirm no existing kibitzer
  git-diff code handles rename detection (`-M`) or arbitrary-base `merge-base` resolution
  — `src/check.rs:1363`'s only precedent is a hardcoded `HEAD` diff — and
  `test-affected.py` itself does a plain `--name-only` diff with no rename handling either,
  so there's no existing answer to inherit. This needs fresh design in Phase 3 planning,
  tracked there as a concrete task rather than left implicit. Given the "never silently
  under-test" bar, the interim default (until explicitly designed) should be conservative:
  an unresolvable rename or a base ref with no common history falls back to the `__ALL__`
  bail-out sentinel rather than guessing.
