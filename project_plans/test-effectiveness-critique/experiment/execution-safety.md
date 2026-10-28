# Experiment execution-safety policy

Status: policy written by Task 0.3.5a; enforced by `scripts/test-effectiveness/preflight-sandbox.sh` and `scripts/test-effectiveness/secret-scan.sh` (Task 0.3.5b). The per-arm isolation modes below are the implementer's defaults for Part B13 of the G0 decision template; the owner approves or edits them at G0 (they are not a recorded decision).

## Scope

This policy governs the experiment harness and the spike scripts that execute untrusted third-party build and test code: Tasks 0.2.2a, 0.3.1a, 0.3.2a, 0.3.4a-e and Stories 3.1.2, 3.1.4, 3.2.x. Install-time scripts (`npm`, `pip`, Gradle, `cargo` build scripts), candidate test suites and mutated code all count as untrusted.

The shipped producer recipes (Tasks 2.2.2b, 2.2.8a-b) run repo test code with the caller's environment and credentials by design. They print the executes-code notice and the docs recommend running them inside the sandbox defined here, but they do not enforce the allow-list, because they run on the user's own checkout at the user's request. kibitzer itself never executes tests (ADR-001).

## Rules

1. **Read-only corpus.** The corpus checkout is bind-mounted read-only except the output directory and the tool's cache.
2. **Environment allow-list.** The sandbox process environment is an allow-list (`env -i` plus named variables). `HOME` points to a scratch directory. No variable matching `*TOKEN*`, `*SECRET*`, `*KEY*`, `*PASSWORD*`, `SSH_AUTH_SOCK`, cloud-provider credential variables (`AWS_*`, `GOOGLE_APPLICATION_CREDENTIALS`) or a password-manager agent socket (`OP_*`) is passed. `~/.ssh`, `~/.aws`, `~/.gnupg` and `~/.config/gh` of the real home are never mounted.
3. **Network.** Denied during suite and mutation runs where the exemplar's selection criterion 3 holds (no network dependency). Allowed only in a separate dependency-fetch step recorded in the results row. The fetch step runs under the same env allow-list and the same secret-free scratch `HOME`, because install-time scripts execute there (`preflight-sandbox.sh --fetch-step`).
4. **Time and memory limits.** Every run has a wall-clock timeout and a memory limit; the values come from the G0 `MutationBudget` (Part B3). Until G0 Part B exists, no arm runs.
5. **Pinned commits.** Corpus repos are pinned by commit; the results row records the SHA.
6. **Throwaway copy.** Mutation tools that rewrite sources (Stryker, mutmut, cargo-mutants, PIT) run against a throwaway copy, not the owner's checkout. After a run, `git status --porcelain` in the original checkout must be empty.
7. **Artifact scrubbing.** Logs and results committed under `project_plans/` and the triage JSONL under `docs/backtest-triage/` are scanned by `secret-scan.sh` (patterns in `secret-patterns.txt`). Install the opt-in pre-commit hook with `scripts/test-effectiveness/secret-scan.sh --install`; `project_plans/` is committed, so the hook lives in the clone's own hooks directory, not in the tree.

## Per-arm isolation table (defaults; owner approves at G0 Part B13)

| Arm | Isolation mode | Why |
|---|---|---|
| Python exemplar (coverage.py, mutmut) | `rootless-sandbox` (bubblewrap) | No network needed after the fetch step; `bwrap` is installed locally |
| Go: stapler-squad packages, kubernetes subpackages | `rootless-sandbox` | Module cache populated in the fetch step, then network denied |
| Rust: ripgrep crates (cargo-mutants) | `rootless-sandbox` | `cargo fetch` in the fetch step; build scripts run under the allow-list |
| JVM: PIT via Gradle | `container` (docker) | Gradle daemon and JDK toolchain are simpler in an image |
| JS/TS: Stryker via npx | `container` (docker) | `npm` install scripts execute in the fetch step; image keeps them off the host |
| Static-only repos (cassandra, servo, deno, vscode) | none needed | kibitzer parses files and executes nothing |

`vm` is an allowed mode but is not automated by the preflight; an arm that selects it records the VM image and the owner's own verification.

## Harness contract

- The harness refuses to start an arm without a recorded isolation mode: it exits non-zero with `isolation mode not recorded for arm <name>` and writes no row.
- Every results row carries `isolation` (one of `container`, `vm`, `rootless-sandbox`), `network` (`denied` or `fetch-step`) and `env_allowlist_hash`.
- Run `preflight-sandbox.sh --mode <mode>` before the first run of an arm; it exits 0 only when the child environment has no allow-list violation and the real home's credential directories are absent. A sandbox that exposes `~/.ssh` makes it exit non-zero naming the path.
- Every spike memo header (Tasks 0.2.2a, 0.3.1a, 0.3.2a, 0.3.4a-e) names the isolation mode used. The commits of Tasks 0.3.5a and 0.3.5b must be ancestors of the first such memo commit.
- Tasks 0.2.2a also requires `G0-decision.md` Part A line A11 (time-box) to be recorded before any install starts.
