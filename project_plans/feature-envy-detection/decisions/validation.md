# feature-envy backtest results

Command: `kibitzer check architecture feature-envy <dir>` (debug build of this branch).
Corpus: `tstapler/stapler-squad` (whole repo) and `kubernetes/kubernetes/pkg`. Go only; the
other corpus repos (Rust/Java/TS) are out of the checker's scope.

## Transcript backtest

`kibitzer check backtest feature-envy` used to exit 1 (`no checker or syntax rule named
'feature-envy'`): `backtest::resolve_target` only accepted native checkers, although
`run_backtest` already replayed model checkers. `resolve_target` now also accepts any
`lookup_model` name (`src/backtest.rs`, test `resolve_target_should_Accept_When_NameIsAModelChecker`).

Real run, debug build of this branch: exit 0, "scanned 4815 transcript(s), checked 1989
snapshot(s), 1215 edit(s) unreconstructable", **0 findings**. Expected: each snapshot is modelled
as a single file, and feature-envy needs a Go type plus its accessors, so it rarely fires on a
lone edit. The transcripts therefore say nothing about noise; the corpus sample below does.

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

## False-positive rate (MEASURED sample, 2026-11-04, shipped build)

Re-ran `kibitzer check architecture feature-envy` on both corpus dirs: 85 + 43 = 128 findings
(matches the table). Sample: every 4th finding (`awk 'NR%4==1'` over stapler-squad then k8s pkg
output) = 32 findings, 22 stapler-squad and 10 k8s. For each I read the first ~15 lines of the
method source and judged it a true positive only if moving the logic onto the dominant type is a
refactor a reviewer would plausibly accept.

- True positives: 4 of 32 (`transitionWithGuard`, `recoverMissingSession`, `ServicePortMap.Update`,
  `updateNodeInfoSnapshotList`).
- False positives: 28 of 32 = **87.5% FP**. Dominant classes: persistence/mapper methods copying a
  parameter record into an ORM builder (`Create`, `UpsertRule`, `Record`), logging/event hooks, RPC
  handlers and controllers orchestrating several collaborators.
- Caveat: one reviewer, partial source, n=32; the interval is wide, but nowhere near 30%.

The >30% rule triggered, so `FEATURE_ENVY_MIN_FOREIGN` is 5 (already applied). The rate is still
far above 30%; the checker stays opt-in (`model_registry()` only). Next levers: suppress
parameter-record/DTO targets, and methods whose own receiver is a repository/store/handler.
