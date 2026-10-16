# Implementation progress: test-effectiveness-critique

Branch: `sdd/test-effectiveness-critique-impl` (from planning tip `3a8c7e6`). Not pushed.

## Done

| Task / story | Evidence | Commit |
|---|---|---|
| 0.3.5a policy + per-arm isolation table | `experiment/execution-safety.md`; `scripts/test-effectiveness/test-safety.sh` policy checks pass | (see git log) |
| 0.3.5b preflight, secret patterns, secret-scan hook | `test-safety.sh` output: "all checks passed"; `shellcheck` clean | (see git log) |

## Blocked on owner decision

| Task | Needs |
|---|---|
| 0.2.2a (select/smoke-test exemplars) | G0 Part A line A11 (time-box) not recorded |
| 0.3.1a, 0.3.2a, 0.3.4a-e (spikes run third-party code) | owner-approved isolation mode per arm (B13) and A11 time-box |
| 0.4.1 (G0), 1.3.1 (G1), 3.4.1 (G2) | owner-authored decision files |
| 1.2.1a onward (transcript backtest, triage) | G0 Part A commit A0 |
| All Phase 3 (3.x), 4.x | G0 Part B / G1 / G2 |

## Failed

None.
