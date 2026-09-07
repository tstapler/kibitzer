# ADR-001: Convert `Cargo.toml` into a real multi-member Cargo workspace, publish the stub plugin as a second `cargo-dist` App under a lockstep version

**Status**: Accepted
**Date**: 2026-09-06

## Context

`requirements.md`'s Success Metrics require a `kibitzer plugin install <name>` subcommand
that fetches "a companion executable" and requires (per the resolved Open Questions) that
the stub plugin be a genuine, independently downloadable GitHub Release asset — not a
local test fixture standing in for one.

`Cargo.toml:1-3` today is:

```toml
# Opt out of the parent stapler-scripts workspace so this package can be built independently.
# Remove this when kibitzer is added to the workspace members list.
[workspace]
```

— an empty `[workspace]` table whose only job is disconnecting `kibitzer` from the parent
`stapler-scripts` Cargo workspace one directory up; it is not yet a real multi-member
workspace. `dist-workspace.toml:1-2` separately declares `members = ["cargo:."]`, cargo-dist's
own (distinct) workspace concept.

`research/stack.md` §5 and `research/pitfalls.md` §2 (reading cargo-dist's ["More Complex
Workspaces" guide](https://axodotdev.github.io/cargo-dist/book/workspaces/workspace-guide.html))
confirm: cargo-dist bundles multiple `[[bin]]` targets *within one package* into a single
release archive — adding `src/bin/kibitzer-stub-plugin.rs` inside the existing `kibitzer`
package would ship the stub binary inside `kibitzer`'s own tarball/Homebrew formula, which
directly violates the Success Metric "uninstalling/never installing the plugin leaves
kibitzer's default behavior... completely unchanged" (a binary silently present in
`$(brew --prefix)/bin` isn't unchanged, even inert). Only a **separate Cargo package** gets
its own independent cargo-dist "App" — separate archives, installers, and (potentially)
version.

## Decision

1. Turn `Cargo.toml`'s `[workspace]` into a real multi-member workspace:
   ```toml
   [workspace]
   members = [".", "crates/kibitzer-stub-plugin"]
   ```
   The existing `kibitzer` package's `Cargo.toml` (name, deps, `[profile.dist]`) is
   otherwise untouched — it becomes workspace member `"."`, matching `dist-workspace.toml`'s
   existing `"cargo:."` entry.
2. Add `crates/kibitzer-stub-plugin/` as a new, minimal package (own `Cargo.toml`,
   `src/main.rs`) with **zero dependency on the `kibitzer` package itself** — it's a
   throwaway binary proving the distribution mechanism, not a real checker, so it needs
   no shared code.
3. Add `"cargo:crates/kibitzer-stub-plugin"` to `dist-workspace.toml`'s `[workspace]
   members` list, alongside the existing `"cargo:."`.
4. **Version lockstep, not per-package tags**: both packages carry the same
   `[package].version` and are released together under the repo's existing single
   `git tag vX.Y.Z` convention (CLAUDE.md "Cutting a release"). `research/pitfalls.md` §2
   flags that cargo-dist's documented multi-App model supports independent per-package
   version tags (e.g. `kibitzer-stub-plugin-v0.1.0`) as an alternative — rejected here (see
   below) because it would change the repo's release process for every future release, not
   just plugin ones, which is disproportionate to what this project needs.
5. Validate empirically before implementation is considered done (not assumed from the
   package-separation model alone, per `pitfalls.md`'s explicit warning about
   [axodotdev/cargo-dist#1740](https://github.com/axodotdev/cargo-dist/issues/1740), open
   feature-resolution behavior across multiple binaries in one dist-managed workspace):
   `cargo build --workspace` succeeds, and `dist plan` (a read-only dry run against the
   installed `cargo-dist-version = "0.32.0"`, no tag pushed) shows two independent Apps
   — `kibitzer` and `kibitzer-stub-plugin` — each producing per-target archives for the
   same tag, before this ADR's decision is treated as confirmed rather than assumed.

## Alternatives Rejected

- **Bundle a second `[[bin]]` inside the existing `kibitzer` package**
  (`src/bin/kibitzer-stub-plugin.rs`) — simplest to write, but confirmed by both
  `stack.md` and `pitfalls.md` to ship the stub inside the core binary's own release
  archive/Homebrew formula, which directly fails the "default install unchanged"
  success metric. Rejected outright, not just as a style preference.
- **Independent per-package version tags** (`kibitzer-stub-plugin-v0.1.0` alongside
  `kibitzer-v0.1.14`) — cargo-dist's documented model for Apps that evolve at different
  cadences. Rejected for v1: the stub plugin's own code is intentionally throwaway and
  will rarely change independently of the core binary; a second tagging scheme is
  ongoing process overhead ("must not add ongoing maintenance burden disproportionate to
  a personal tool" — requirements.md Constraints) that lockstep versioning avoids
  entirely. Revisit only if a real (non-stub) plugin crate is added later with its own
  release cadence.
- **Move the `kibitzer` package itself under `crates/kibitzer/`** (a "proper" workspace
  reshuffle where the root is purely virtual) — technically cleaner, but touches every
  existing path reference (`Cargo.toml`'s own comment already flags this as a bigger,
  likely out-of-scope reshuffle) for no benefit this project needs; rejected as scope
  creep beyond what "add a plugin" requires.

## Consequences

- `cargo build` (no `--workspace` flag) from the repo root continues to build only the
  `kibitzer` package by default, matching today's single-crate developer workflow —
  confirmed by Cargo's default behavior of building the workspace's "current package"
  (`.`) when invoked from its own directory, not every member.
- A future *real* (non-stub) plugin crate — e.g. the eventual embedding-based checker —
  has a concrete, working precedent to copy for its own `crates/<name>/` package and
  `dist-workspace.toml` entry, rather than re-deriving this from scratch.
- If `dist plan`'s dry run (step 5 above) reveals cargo-dist 0.32.0 does *not* cleanly
  support two lockstep-versioned Apps releasing from one tag as expected, the fallback is
  prefixed per-package tags (the rejected alternative above) — this ADR's lockstep
  decision is contingent on that empirical check, not a irreversible commitment made
  before verifying it against the actual installed cargo-dist version.

## Update (2026-09-06, adversarial-review.md BLOCKER resolution)

Adversarial review flagged that this ADR's second-App design, combined with
`dist-workspace.toml`'s workspace-wide `installers = ["shell", "homebrew"]` and
`tap = "tstapler/homebrew-tap"`, would by default also publish a second, permanent,
public Homebrew formula (`kibitzer-stub-plugin.rb`) for a component this project calls
"throwaway" — never addressed by the original decision above.

Resolution: `installers` is documented `[package-local]` (overridable per-package via
`[package.metadata.dist]`) as of cargo-dist 0.0.3, confirmed still true at the exact
pinned `cargo-dist-version = "0.32.0"` by reading `book/src/reference/config.md` at
GitHub tag `v0.32.0` in `axodotdev/cargo-dist`. `tap` and `publish-jobs` are
`[global-only]` and stay workspace-wide, but that's inert here: `publish-jobs =
["homebrew"]` only publishes formulas some App's `installers` list actually generated.
So `crates/kibitzer-stub-plugin/Cargo.toml` now carries its own `[package.metadata.dist]`
with `installers = ["shell"]`, overriding the workspace default for that package only —
`kibitzer` keeps both installers and its one Homebrew formula; `kibitzer-stub-plugin`
gets only a shell (`curl | sh`) installer and a plain GitHub Release archive, with no
Homebrew formula ever generated or published for it. No tradeoff was accepted and no
part of this ADR's original decision (real workspace, lockstep versioning, zero
dependency on `kibitzer`) changed — this is an additive scoping fix, tracked in
`plan.md` Story 1.2.1 / Task 1.2.1a.
