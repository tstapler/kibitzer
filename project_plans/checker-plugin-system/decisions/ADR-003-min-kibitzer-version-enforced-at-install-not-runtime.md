# ADR-003: `min_kibitzer_version` compatibility is enforced once, at `plugin install` time, via a hand-rolled major.minor.patch comparator — not `semver`, and not re-checked at every check run

**Status**: Accepted
**Date**: 2026-09-06

## Context

`requirements.md`'s resolved Open Questions makes version/protocol compatibility
enforcement explicitly in scope for v1: "The plugin registry/manifest carries a
minimum-kibitzer-version field; `kibitzer plugin install` refuses (or warns...) on a
version mismatch rather than silently registering an incompatible plugin."

`Cargo.lock` has no `semver` crate today (confirmed by `grep`), and none of the four
research docs (`stack.md`, `build-vs-buy.md`) identify a need for one — the comparison
this project needs is narrow: "is the running kibitzer's own `CARGO_PKG_VERSION` >= the
plugin manifest's `min_kibitzer_version`," both always simple `MAJOR.MINOR.PATCH` strings
(kibitzer's own `Cargo.toml` version has never used pre-release/build-metadata suffixes),
not general semver range satisfaction (`^1.2`, `~1.2.3`, etc.).

## Decision

1. Parse both version strings into a `(u64, u64, u64)` tuple via a small hand-rolled
   parser (split on `.`, `u64::from_str` each part, bail on anything else) and compare with
   `Ord`'s derived tuple ordering — correct by construction for multi-digit components
   (avoids the classic bug where `"0.10.0"` sorts before `"0.9.0"` under naive string
   comparison), with no dependency on `semver`'s crate for a comparison this narrow.
2. **Enforced once, at `kibitzer plugin install` time**, comparing the plugin manifest's
   `min_kibitzer_version` against `env!("CARGO_PKG_VERSION")` of the currently-running
   `kibitzer` binary. **Refuses** (not warns) on `current < min_kibitzer_version` — the
   resolved Open Question's stated policy — with a hard error naming both versions and the
   fix (`kibitzer` needs upgrading first).
3. **Not re-checked on every check run.** Once a plugin is installed, `registered_plugin_checks()`
   trusts that install-time gate; there is no runtime re-validation on every
   `find_effective_config` call. Rationale: the only way the invariant could become false
   after install is the user *downgrading* their own `kibitzer` binary while a
   newer-min-version plugin stays installed — a self-inflicted, rare edge case for a
   single-user tool, not a scenario worth paying a version-parse-and-compare cost on every
   check dispatch to guard against. `kibitzer plugin status` re-runs the same comparison
   on demand so a user who *does* hit this can diagnose it without waiting for a check to
   fail mysteriously.
4. The compatibility direction is one-way only (kibitzer's own version vs. the plugin's
   stated floor) — this project does not introduce a reciprocal "plugin protocol version"
   the plugin binary itself declares back to kibitzer (e.g. no `<plugin> --kibitzer-protocol-version`
   handshake). The dispatch surface a plugin must honor (SARIF-over-stdout,
   `{file}`/`{changed_lines}` substitution) is exactly `Check.command`'s existing, already-stable
   contract — `min_kibitzer_version` protects against a plugin built against a *newer*
   kibitzer feature (a manifest field, a SARIF extension) than the user's installed
   binary understands, not the reverse.

## Alternatives Rejected

- **`semver = "1"` crate** — the standard, correct choice for real semver range
  satisfaction (`VersionReq`/`Version`), and worth adopting the moment this project needs
  pre-release channels, build metadata, or caret/tilde ranges. Rejected for v1 as a new
  dependency solving a more general problem than the one this project actually has (a
  single floor comparison on two always-plain `MAJOR.MINOR.PATCH` strings) — revisit if a
  future plugin manifest needs richer version constraints.
- **Lexicographic string comparison** (`min_kibitzer_version.as_str() <= current`) —
  rejected outright: incorrect the moment either version reaches a two-digit component
  (`"0.9.0" > "0.10.0"` as strings), a real bug class, not a hypothetical one.
- **Warn-and-continue on an incompatible version** — rejected per the Open Question's own
  resolution; a plugin built against a `min_kibitzer_version` the running binary doesn't
  meet may rely on manifest fields or SARIF handling the running `kibitzer` doesn't know
  how to use, so registering it anyway risks a confusing downstream failure instead of one
  clear install-time error.
- **Runtime re-validation on every check dispatch** — considered for defense-in-depth
  against the kibitzer-downgrade edge case (§3 above), rejected as disproportionate cost
  (a version parse + comparison on every `find_effective_config` call, for every repo, on
  every invocation) for a failure mode that's rare, self-inflicted, and already
  diagnosable on demand via `kibitzer plugin status`.

## Consequences

- `InstalledPlugin` records the `min_kibitzer_version` the plugin declared at install time
  (not just whether the check passed), so `plugin status`/`plugin list` can display it and
  re-derive compatibility against the *currently* running binary without re-reading the
  manifest or re-downloading anything.
- If a plugin ever needs genuine semver-range expressiveness (e.g. "requires kibitzer
  1.x, incompatible with 2.x"), this ADR's comparator needs replacing with `semver` at
  that point — not a breaking change to the manifest schema itself, since
  `min_kibitzer_version` stays a plain version string either way.
