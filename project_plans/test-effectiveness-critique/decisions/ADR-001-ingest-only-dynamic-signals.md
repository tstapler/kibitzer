# ADR-001: Dynamic test-effectiveness signals are ingest-only; kibitzer never runs tests

**Status**: Accepted (plan phase) | **Date**: 2026-10-11 | **Resolves**: requirements Open Question 1

## Context
Per-test coverage and mutant-kill data require executing the repo's tests, possibly thousands of times. Kibitzer is an advisory, diff-aware checker; it executes no repo code today (`research/architecture.md` section 4). The `command` check path is per-file with a 30 s timeout (`src/check.rs:36`).

## Decision
Kibitzer parses reports that CI or the owner already produced (coverage.py contexts, Go per-test cover profiles, lcov, mutation-testing-elements JSON, PIT XML, JUnit XML). It never launches test, coverage, or mutation tools. Collection recipes live as standalone scripts under `scripts/test-effectiveness/`, outside the binary's runtime path.

## Alternatives rejected
- Shell out to the project's runner from a subcommand: rejected. Cost is dominated by the user's suite, orchestration differs per language, and it breaks "advisory, not a test runner" and the "executes repo code only when explicitly invoked" security class.
- Plugin wrapper: rejected. The plugin protocol is per-file SARIF with a 30 s timeout (`src/plugin.rs:174-190`, `docs/output-formats.md`).

## Consequences
- Provenance (tool, version, run time, attribution method) must be carried in the input or supplied by flags; adapters reject inputs that cannot supply it.
- Input staleness is a first-class concern: findings carry the input's commit and are marked stale when production files changed since.
- Per-test coverage acquisition cost is the user's, which the experiment (Epic 3.2) measures per language.

## Amendment (plan repair, 2026-10-11)
- Staleness is now a story: Story 2.3.5 (input commit in each dimension's `Provenance`, `git diff --name-only <input-commit>..HEAD` intersected with the test's covered files; unknown commit caps findings at `Low`).
- Provenance is per dimension (coverage, kills), not per matrix, because a coverage input and a mutation input come from different tools and runs. Attribution (`Isolated`/`InOrder`/`LeaveOneOut`) applies to coverage only; kill inputs carry `KillCompleteness` (`Full`/`FirstKillerOnly`).
- Ingest-only consequence for tests ids: raw ids from each tool are resolved to source locations by `TestLocator` (Story 2.1.4); the Go collection recipe encodes the package path in each profile name. Unresolvable ids are excluded (`NoSource`), not guessed.
- Attribution method is carried by the input (Go `manifest.json` `mode`), else supplied by `--attribution`, else the adapter's documented default is used and printed as "assumed" with a class W warning. Adapters reject only a missing tool name or version; a missing run time or commit is a warning (plan "Attribution source", Stories 2.1.1, 2.2.1-2.2.3).
- A single input does not exclude tests: stability is a footer note ("Stability not assessed: single run"); only `Flaky` (needs two inputs) excludes.
- The report subcommand ships hidden and experimental until the G2 decision file exists (Story 2.3.1, Task 4.2.1c).

## Amendment (triad repair, iteration 1, 2026-10-11)
- "Kibitzer never runs tests" covers the binary only. The experiment scripts (spikes 0.3.2 and 0.3.4, harness 3.1.2, leave-one-out 3.1.4, arms 3.2.x) and the collection recipes do execute third-party test and build code, install-time scripts and mutated sources on the owner's machine. Story 0.3.5 adds the safety policy for that: per-arm isolation mode (container, VM or rootless sandbox), read-only corpus mount, environment allow-list with no credential or agent-socket variables, network denied during runs, per-run timeout and memory limit, throwaway copies for mutators, and a secret scan before results are committed. The harness refuses to run an arm without a recorded isolation mode and every results row carries it.
- Input files are untrusted data: Story 2.2.7 adds the size cap, depth limit, entity-safe XML handling and path-containment rules that "ingest-only" implies but did not state.
