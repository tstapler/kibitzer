# feature-envy backtest results

Command: `kibitzer check architecture feature-envy <dir>` (debug build of this branch).
Corpus: `tstapler/stapler-squad` (whole repo) and `kubernetes/kubernetes/pkg`. Go only; the
other corpus repos (Rust/Java/TS) are out of the checker's scope.

## Transcript backtest

`kibitzer check backtest feature-envy` exits 1: `no checker or syntax rule named
'feature-envy'`. The backtest harness replays edits through native per-file checkers only;
`feature-envy` is an `ArchModelChecker` (whole-repo model), so it cannot be replayed that way.
**Not run — gap, not a pass.** Follow-up: teach the backtest harness about model checkers.

## Public-corpus runs

| Build | Findings (squad + k8s pkg) |
|---|---|
| MIN_FOREIGN=3, call edges counted as-is | 480 + 580 = 1060 |
| + call edges counted only for types the field walker also saw | 166 + 95 = 261 |
| + MIN_FOREIGN=5 | 97 + 50 = 147 (12 move-function, 135 move-field) |
| + skip methods with no own signal (no own-field access and no own-type call) — shipped | 85 + 43 = 128 |

The FP samples below were taken before the last row; the no-own-signal guard removes the
zero-own helper class they were dominated by, but the post-guard rate was not re-sampled.

## sdd:6-verify follow-ups (not done, deliberately)

- Gate on a single dominant target type (Tsantalis & Chatzigeorgiou) instead of the summed
  foreign count; deviates from the criterion-2 wording, so left for a follow-up.
- Count own-type calls in `local`; same reason.
- Generic receivers (`Binding[T]`) and `New*` constructors have no dedicated test/suppression.
- `INTENTIONAL_DELEGATION_SUFFIXES` duplicates `middle_man.rs`; share it when a third user appears.

## Root cause of the first-run noise

`arch_model::resolve_one_call_edge` resolves `recv.Method()` by bare method name. A unique
`func (*noCopy) Lock()` (stapler-squad) or `StripedLockSet` (k8s) made every `mu.Lock()` look like
a foreign access to that type: 254 of the 1060 first-run findings named one of the two. The
checker now trusts a call edge only when the type-aware field walker also saw that type accessed in the
same method (`name_collision_call_edges_without_field_evidence_are_ignored`). The call-graph
resolver itself is unchanged (other consumers depend on it).

## False-positive estimate (INFERRED — judged from finding text, source not opened)

- Sample of 29 (every 9th) at the 261-finding stage: ~6 actionable, ~23 not → ~80% FP.
- Sample of 15 (every 10th) after MIN_FOREIGN=5: ~8 actionable, ~7 not → ~45% FP.
- Typical remaining FP: a store/logger/option-parsing method reading a parameter DTO or record.

The >30% rule triggered, so `FEATURE_ENVY_MIN_FOREIGN` is 5. The rate is still above 30% after
that; the checker stays opt-in (`model_registry()` only), and DTO/parameter-struct
suppression is the next lever.
