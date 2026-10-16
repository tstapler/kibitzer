# Implementation progress: test-effectiveness-critique

Branch: `sdd/test-effectiveness-critique-impl` (from planning tip `3a8c7e6`). Not pushed. Evidence = `cargo test --no-fail-fast`, `cargo clippy --workspace --all-targets -- -D warnings`, and per-task commit messages in `git log`.

## Done

| Task / story | Evidence |
|---|---|
| 0.3.5a-b safety gate (policy, preflight, secret scan) | `scripts/test-effectiveness/test-safety.sh` "all checks passed"; shellcheck clean |
| 0.1.1a-d `docs/test-effectiveness.md` | 4/4; inventory table 32 rows; markdown-link-integrity check is weak (does not flag a broken link), `../` links resolved manually |
| 0.2.1a-e `src/test_code/` seam | 36 tests |
| 2.1.1a-b, 2.1.2a-b, 2.1.3a/c matrix, analyses, exclusions | 36 tests |
| 2.2.7a guard, 2.2.9a sidecar | 24 tests |
| 2.2.1 coverage.py, 2.2.2 Go cover (+recipe script 2.2.2b-c, 2.1.4b recipe part), 2.2.4 Stryker, 2.2.5 PIT, 2.2.7b | adapters; fixtures are real-run only for coverage.py 7.13.4 and the Go profiles; Stryker and PIT fixtures are hand-written (real runs blocked, see below) |
| 2.1.4a, 2.2.7c locator | 11 tests |
| 2.2.7d, 2.2.7e, 2.2.9c malformed corpus, mutate loop, partition check | no panics in 120k extra cases |
| 2.1.5a-d scale tests, ceiling `DEFAULT_MAX_INPUT_BYTES = 649_949_574` | measured, see commit f333fff; Stryker limiting format |
| 1.1.1a, 1.1.2a, 1.1.2c, 1.1.5a-b `test-no-assertion` Go+Python, registry-only | 25 + 9 tests; not chained into `default_checks()` |

## In progress

2.2.9b, 2.3.1a, 2.3.1e, 2.3.1k, 2.3.1h (hidden `kibitzer tests report` core).

## Blocked on owner decision

| Task | Needs |
|---|---|
| 0.2.2a (select/smoke-test exemplars, installs tools) | G0 Part A line A11 (time-box) not recorded |
| 0.3.1a, 0.3.2a, 0.3.4a-e (spikes execute third-party code) | owner-approved isolation mode per arm (B13) and A11 |
| 0.4.1 (G0), 1.3.1 (G1), 3.4.1 (G2) | owner-authored decision files |
| 1.2.1a-c, 1.2.2, 1.2.3 (backtest, triage, latency) | G0 Part A commit A0, A5 triage volume, G1 budget commit |
| 1.1.1b, 1.1.2b, 1.1.3, 1.1.4 (other languages, tautology, mock-only) | plan sequencing: after owner Go/Python triage (1.2.2) |
| All 3.x, 4.x; ship-gated 2.2.3, 2.2.6, 2.2.8, 2.3.3-2.3.5 | G0 Part B / G1 / G2 |

## Not started, unblocked (remainder wave)

2.3.1b/c/d/f/i/j renderer and messages, 2.3.2 ignore + RULES.

## Failed

None. `tests/daemon_spawn.rs` fails when run in parallel under load and passes with `--test-threads=1` (14/14); unrelated to this work.
