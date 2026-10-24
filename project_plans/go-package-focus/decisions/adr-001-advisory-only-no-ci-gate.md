# ADR-001: Advisory-Only Mode — No Autofix, No CI Gate

**Status**: Accepted
**Date**: 2026-10-23

## Context

The `go-package-focus` checker identifies candidate package splits based on structural
signals (coupling, LCOM, field-access clusters). These heuristics produce false positives
on legitimate large packages (shared lock/state machines, cohesive parsers, monorepo
entry-point `cmd/` packages). Enforcing a CI gate on advisory heuristics blocks engineers
for reasons that require human judgment to resolve, and there is no authoritative threshold
for "a package is too large" that holds across all codebases.

## Decision

All `go-package-focus` findings use `Severity::Advisory` only. No autofix is generated.
The checker is never registered as a blocking check in `default_checks()`. The
split/keep decision procedure (AC-8) explicitly documents this boundary.

## Alternatives Rejected

**Blocking severity with an escape hatch** — a `// kibitzer:ignore` comment on each
offending file: rejected because the escape hatch shifts cognitive load to every engineer
who hits the check, and the false-positive rate on real-world code (monorepo entry points,
generated types with many methods, shared state abstractions) is high enough that
blocking would be routine noise rather than rare exceptions.

**Separate CI-gateable dependency-policy enforcement** — flag it as a distinct check name
(`go-dependency-policy`) with blocking severity: out of scope for this feature; documented
as a future direction in the split/keep decision procedure (AC-8).

## Consequences

- Agents and engineers receive advisory findings they can act on or dismiss.
- Dependency-policy violations (layer inversion, forbidden imports) remain a separate
  blocking concern under `architecture_checker: "dependency-rules"`.
- The distinction between CI-gateable policy and advisory heuristics is documented in
  `docs/go-package-focus.md`.
