# Requirements: checker-plugin-system

**Date**: 2026-09-06
**Type**: feature addition
**Complexity**: 3 — system design (external integration: downloading a companion artifact)

## Problem Statement

kibitzer's native checkers are 100% static: every one is compiled directly into the
binary via `checker::registry()` ([src/checker.rs:146](https://github.com/tstapler/kibitzer/blob/master/src/checker.rs#L146)),
and `default_checks()` ([src/config.rs:534](https://github.com/tstapler/kibitzer/blob/master/src/config.rs#L534))
runs the whole catalog for every install, pylint-style, with no opt-out mechanism for
heavy individual checks short of `.claude/inspect.json` suppression.

An upcoming semantic "how vs why" comment checker needs an embedding model (a 2026-09-06
research spike — see `docs/comment-quality-false-positives.md` and the session's fork
report — found rule-based keyword heuristics unreliable: 8:2 false-positive-to-false-negative
ratio on real Cassandra/Servo samples, because genuine rationale like parameter contracts,
byte-format specs, and spec citations carries no hedge keyword). That model requires an
ONNX runtime (`ort`) and a downloaded model file. Baking that into `checker::registry()`
directly would give every kibitzer install — including CI environments and minimal
`cargo install`/Homebrew setups — a hard dependency on an ONNX runtime and megabytes of
model weights, even for users who never touch the semantic checker. This conflicts with
kibitzer's current zero-ML, no-network-at-runtime, single lightweight binary distribution
model (see the repo's CLAUDE.md "Cutting a release" section — `cargo-dist`/Homebrew ship
one static-ish binary per platform).

The problem this project solves: give kibitzer a way to make a heavy, optional checker
(the embedding one, and any future one with a similar profile) pluggable and separately
installable, without touching the core binary's dependency graph.

## Baseline

Today there are exactly two ways to add a check, and neither fits:

1. **Native (`Checker` trait + `registry()`)** — fully in-process, structured `Finding`
   output, zero install step, but requires compiling the dependency into the core binary.
   No runtime opt-in/opt-out short of not registering it at all.
2. **External command** (`Check.command` + optional `output_format: sarif`,
   [src/config.rs:28-77](https://github.com/tstapler/kibitzer/blob/master/src/config.rs#L28-L77),
   [docs/output-formats.md](https://github.com/tstapler/kibitzer/blob/master/docs/output-formats.md)) —
   already a fully general "shell out to an external tool, optionally parse SARIF from
   stdout" mechanism, but it's per-repo config a user hand-writes in `.claude/inspect.json`
   pointing at a command that must already be installed and on `PATH`. There is no
   concept of "install this optional companion tool for me" — a user (or kibitzer itself)
   has to separately build/fetch/place the binary and then hand-author the config entry.

Neither gives a path to "one command installs the optional heavy checker, and it's
otherwise invisible to a default install."

## Users / Consumers

kibitzer's own maintainer (single user today, tstapler), running it locally via the
`kibitzer` CLI/daemon (`src/daemon.rs`), the MCP server (`src/mcp.rs`), and the Claude
Code hook integration. No external/downstream users yet.

## Success Metrics

- The core `kibitzer` binary's `Cargo.toml` gains no new ML/heavy dependencies (no ONNX
  runtime, no model weights) from this work — the plugin mechanism itself may add small,
  lightweight dependencies (an HTTP client, a checksum crate) needed to implement
  install/verify, but nothing ML-related.
- A `kibitzer plugin install <name>` (or equivalently named) subcommand fetches a
  companion executable, places it somewhere kibitzer's check-resolution can find it,
  and wires it into the effective check config as a `command`/`output_format: sarif`
  check — without the user hand-editing `.claude/inspect.json` themselves.
- An automated test in the suite installs/loads a trivial stub plugin (always returns
  no findings, or one canned finding) and asserts it runs and its output surfaces
  correctly through kibitzer's normal check-dispatch path.
- A manual CLI walkthrough (install → run → see the stub's output) is captured (e.g. in
  the PR description) as a live demonstration, not just the automated test.
- Uninstalling/never installing the plugin leaves kibitzer's default behavior and
  `default_checks()` catalog completely unchanged.

## Appetite

TBD — Phase 3 planning sizes this after research determines the real cost of the chosen
approach (distribution/versioning mechanics are the biggest unknown, not the check-dispatch
plumbing, since that already exists via `Check.command`).

## Constraints

- Solo maintainer — no team/budget constraints, but changes must not add ongoing
  maintenance burden disproportionate to a personal tool.
- Must not break the existing `cargo-dist`/Homebrew release pipeline (see CLAUDE.md
  "Cutting a release") — the core binary stays the single artifact that flow produces.
- No silent network access: kibitzer currently makes no network calls at check-run time;
  any plugin download must be an explicit, user-initiated action (a subcommand invocation),
  never triggered implicitly by a normal check run.

## Non-functional Requirements

- **Performance SLO**: not specified — a plugin's own runtime cost is the plugin's
  concern, not this mechanism's.
- **Scalability**: not applicable — single-user local tool.
- **Security classification**: internal/personal-use tool, but a downloaded-and-executed
  companion binary is still a real trust boundary — Feasibility Risks below flags this
  for research/planning to address (checksum verification at minimum).
- **Data residency**: no special requirements.

## Scope

### In Scope

- The plugin **mechanism**: install/register/run/uninstall lifecycle for an optional
  external checker, built on top of (not replacing) the existing `Check.command` +
  `output_format: sarif` dispatch path ([src/config.rs](https://github.com/tstapler/kibitzer/blob/master/src/config.rs),
  [docs/output-formats.md](https://github.com/tstapler/kibitzer/blob/master/docs/output-formats.md)).
- A generic extension point other future optional/heavy checks could also use — not
  special-cased to only the embedding checker.
- One trivial stub example plugin (e.g. always-no-findings, or one canned finding) that
  proves the install → register → run path end-to-end. Its own logic is intentionally
  throwaway.
- An automated test exercising the stub plugin through the mechanism, plus a manual
  demo.
- Converting `Cargo.toml` into a real multi-member Cargo workspace and adding a new
  `crates/kibitzer-stub-plugin/` package, plus the matching `dist-workspace.toml` entry,
  so the stub plugin publishes as its own independently downloadable GitHub Release
  asset via the existing `cargo-dist` pipeline (see "Cutting a release" in CLAUDE.md).
- A minimum-kibitzer-version field in the plugin manifest/registry, enforced (install
  refuses or warns) by `kibitzer plugin install`.

### Out of Scope

- The actual embedding-based "how vs why" semantic detection logic (comment-vs-signature
  similarity scoring, the real ONNX model, `fastembed`/`ort` integration). That is
  separate, later work that will be the mechanism's first *real* consumer once this
  ships.
- Any plugin marketplace/discovery UI, plugin signing/publisher-identity verification
  beyond basic artifact integrity, or supporting third-party-authored plugins from
  outside this repo's own release artifacts.
- Changing or generalizing the existing native `Checker` trait/`registry()` path — this
  project targets the external/optional path only.

## Rabbit Holes

- **Cross-platform binary distribution** for the companion plugin (per-OS/arch builds,
  akin to `cargo-dist`'s own per-target artifacts) could balloon scope if treated as a
  first-class multi-platform release pipeline right away — research/plan should look at
  whether reusing the existing `cargo-dist` machinery (a second binary target in the same
  workspace/release) is cheap enough, versus deferring true multi-platform support.
- **Version/protocol compatibility** between the core binary and an independently
  versioned plugin binary (a plugin built against an older SARIF/protocol expectation) —
  needs at least a stated compatibility policy, even if enforcement is minimal for v1.
- **Trust/integrity of a downloaded executable** — even for a single-user tool, silently
  running an unverified downloaded binary is a real risk; at minimum a checksum check
  against a known-good manifest should be considered, without over-building a full
  signing infrastructure for a personal tool.
- **Where installed plugins live and how they're discovered** (a `~/.cache/kibitzer/plugins/`-style
  directory, an entry auto-added to `.claude/inspect.json` vs. a separate plugin-registry
  file kibitzer reads independently) has real design surface — don't let it default to
  whatever's fastest to type without checking it composes with the existing
  `find_effective_config()`/`.claude/inspect.json` overlay convention.

## Alternatives Considered

- **Extend `Check.command`/SARIF** (chosen direction per this interview) — lowest risk,
  reuses proven dispatch infra, keeps the core binary's dependency graph untouched;
  "separately installable" reduces mostly to a distribution/install-subcommand problem.
- **dylib/cdylib + FFI in-process loading** — tighter integration (could return real
  `Finding` structs, no process-spawn overhead) but Rust has no stable ABI across
  compiler versions, making a plugin built once and loaded later fragile against kibitzer
  or Rust toolchain upgrades; rejected as disproportionate risk for a personal tool.
- **WASM plugin runtime** (e.g. `wasmtime`) — sandboxed and portable, addresses the trust
  concern well, but adds a real new runtime dependency to the *core* binary (a WASM
  engine to host plugins), which is exactly the dependency-weight problem this project
  exists to avoid; could be revisited later if the process-boundary approach proves
  insufficient.
- **Cargo feature flag producing an alternate build** — simplest code-wise, but doesn't
  achieve "separately installable/downloadable after the fact": it's a build-time choice
  requiring two published release artifacts and doesn't let a user add the capability to
  an already-installed kibitzer without a full reinstall/swap.

## Feasibility Risks

- Reusing `output_format: sarif` may prove too lossy for a semantic checker's eventual
  needs (e.g. confidence scores, the specific comment/identifier pair that triggered a
  finding) — research should confirm SARIF's schema can carry what a future embedding
  checker will actually want to report, or identify the gap now.
- The install subcommand needs *something* to fetch from — this project doesn't build a
  real embedding plugin, so the stub's "distribution" story (a real GitHub Release
  artifact vs. a fixture built as part of the test) needs a concrete, unglamorous answer
  in planning, not hand-waved as "TBD" through implementation.

## Observability Requirements

Standard request logging sufficient — a plugin install/run failing should surface a
clear error to the user (stderr / existing kibitzer error paths); no new metrics/alerting
needed for a single-user local tool.

## Risk Control

Not needed — low risk. No production rollout; changes are additive (a new opt-in
subcommand and check-registration path) and don't alter default behavior for anyone who
never installs a plugin. Rollback is `git revert` if something's wrong before it ships.

## Open Questions — resolved after Phase 2 research

- **Where installed plugins live and register**: a separate kibitzer-owned registry file
  (`$XDG_DATA_HOME/kibitzer/plugins/registry.json`, per
  `project_plans/checker-plugin-system/research/architecture.md`) — not auto-written into
  `.claude/inspect.json`, which `docs/suppressing-checks.md` treats as strictly
  user-authored. `find_effective_config()` chains the registry's synthesized `Check`
  entries alongside `default_checks()`; per-repo opt-out reuses the existing
  disabled-by-name suppression.
- **Real downloadable artifact required**: yes. The stub plugin must be a genuine GitHub
  Release asset, not a local test fixture. Research (`research/stack.md`,
  `research/pitfalls.md`) confirmed this requires converting `Cargo.toml`'s currently-empty
  `[workspace]` table into a real multi-member workspace (a new `crates/kibitzer-stub-plugin/`
  package) plus a second `"cargo:crates/..."` entry in `dist-workspace.toml` — cargo-dist
  bundles multiple `[[bin]]` targets in one package into a single release archive by
  default, so a separate package is required for a separate artifact. This is now
  explicitly **in scope** (added to Scope below) and should be sized accordingly in
  Phase 3 planning.
- **Version/protocol compatibility enforced in v1**: yes. The plugin registry/manifest
  carries a minimum-kibitzer-version field; `kibitzer plugin install` refuses (or warns,
  per Phase 3's design) on a version mismatch rather than silently registering an
  incompatible plugin. Now explicitly **in scope**.
