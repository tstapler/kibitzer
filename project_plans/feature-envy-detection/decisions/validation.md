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

## Round 2: record-target and orchestrator-role suppression (2026-11-04)

Rules added in `src/feature_envy.rs`:
1. `is_record_target`: skip when the dominant foreign type declares no methods anywhere in the
   model (DTO / input record / result or summary struct). Covers parameter records and
   locally built records (`GetStatistics`, `*Result` fillers) alike.
2. `is_orchestrator_receiver`: skip when the method's own receiver ends in `ORCHESTRATOR_SUFFIXES`
   (`Repository`, `Storage`, `Store`, `Handler`, `Controller`, `Manager`, `Service`, `Registry`,
   `Listener`, `Reconciler`, `Runner`, `Executor`, `Generator`, `Collector`, `Builder`, `Scanner`,
   `Checker`, `Linker`, `Determiner`) or equals one case-insensitively (`reconciler`).

Command (debug build): `kibitzer check architecture feature-envy <dir>` on `stapler-squad` and
`kubernetes/kubernetes/pkg`.

| Build | Findings (squad + k8s pkg) |
|---|---|
| Round 1 baseline | 85 + 43 = 128 |
| + rule 1 (param-only first cut, then any no-method record) + 7 original suffixes | 12 + 25 = 37 |
| + rule 2 role suffixes above (shipped) | 2 + 8 = 10 |

### Measured FP rate — target NOT met

All judgments: one reviewer reading the first ~10-15 lines of each method; a TP needs a refactor a
reviewer would plausibly accept.

| Sample | Build | n | TP | FP | FP rate |
|---|---|---|---|---|---|
| squad + k8s pkg, all findings (N=1) | 7 original suffixes + no-method record rule | 37 | 7 | 30 | 81% |
| squad + k8s pkg, all findings | + role suffixes (tuned on the row above) | 10 | 7 | 3 | 30% (in-sample, n<32) |
| k8s `staging/` (22) + the 10 above | role suffixes, before `Options`/`Config` | 32 | 9 | 23 | 72% |
| **Held-out**: `cli/cli`, `caddyserver/caddy`, `hashicorp/consul`, every 5th of 159 findings (`cat ho_cli ho_caddy ho_consul \| awk 'NR%5==1'`) | + `Options`/`Config`/`Flags`/`Settings` (shipped) | 32 | 0-2 | 30-32 | **>= 94%** |

Commands: `kibitzer check architecture feature-envy <dir>` for each repo; counts cli 5, caddy 4,
consul 150, cert-manager 0.

**Conclusion: criterion 2 (FP <= 30% at n >= 32) is not met.** The 30% row has n=10 and its suffix
list was fitted to that same corpus. On unseen repos the rate is >= 94%, almost entirely
`hashicorp/consul` RPC/HTTP endpoint methods (`func (c *Catalog) Register(args *structs.RegisterRequest, ...)`,
`HTTPHandlers.*`) whose dominant foreign type is a request struct *with* methods, so the
no-method-record rule does not apply. Suffix lists cannot fix this honestly; the next lever is
structural (a parameter that is a request/args value consumed in a handler-shaped method), which
needs its own design. The checker must stay opt-in.

### Held-out sample record (what is and is not preserved)

- Generation: `kibitzer check architecture feature-envy <repo> > ho_<name>` for `cli/cli` (5
  findings), `caddyserver/caddy` (4), `hashicorp/consul` (150); `cat ho_cli ho_caddy ho_consul |
  awk 'NR%5==1'` over the 159 findings -> 32 sampled. Build: the shipped one (suffixes incl.
  `Options`/`Config`/`Flags`/`Settings`). Judged by one reviewer reading the method source.
- Counts: 0-2 TP, 30-32 FP, i.e. FP rate >= 94%. The **per-finding judgment list was not
  saved**, and the reason for the 0-2 range (presumably borderline cases) was not recorded, so
  the exact split is not reproducible from this file. It is deliberately not re-measured or
  re-tuned here (the sample is spent). Even at the most favourable end (2 TP), FP is 30/32 = 94%.

### Deviation from AC 0 wording

AC 0 says the dominant type is "a parameter of the flagged method" with no methods of its own.
`is_record_target` (`src/feature_envy.rs`) implements the **wider** rule: the dominant foreign
type declares no methods anywhere in the model, whether or not it is a parameter. Reason: locally
built result/summary records (`GetStatistics`, `*Result` fillers) were observed as the same FP
class as parameter records; the parameter-only first cut was replaced by the wider rule (see the
Round 2 build table). The with/without-param-gate delta was not isolated in a separate run.
Narrowing to the AC would re-admit those records. Tests: `suppresses_parameter_record_without_methods`
and `suppresses_locally_built_record_without_methods` (must not fire),
`still_flags_parameter_type_that_has_methods` (must still fire).

### Known true positives

- `ServicePortMap.Update`, `updateNodeInfoSnapshotList`: still fire.
- `transitionWithGuard` (receiver `BacklogService`): **lost, accepted.** It reads 5 fields of its
  `item` param into a `BacklogItemTransitionInput` (`PlanApproved: item.PlanApproved, ...`). That
  shape is exactly the service-receiver FP class (consul's endpoints are the same). Narrowing the
  `Service` suffix by "reads a foreign type heavily" cannot separate them: every FP reads a
  foreign type heavily. Pinned by
  `service_receiver_reading_a_param_heavily_is_a_known_accepted_loss`.
- `recoverMissingSession` (receiver `SessionHealthChecker`): **lost, accepted; the TP label is kept.**
  Source: `result.IsHealthy = false; result.Issues = append(result.Issues, ...);
  result.RecoveryAttempted = true; result.RecoverySuccess = true; result.Actions = append(...)`
  mixed with `instance.Start(false)`, `instance.TmuxAlive()`. It loses to the `Checker` suffix
  (and, if `HealthCheckResult` is dominant, to the no-method-record rule; which type was dominant
  before the change was not re-verified). It is a real recovery routine on `Instance`-heavy state,
  so this is a recall cost of the suffix rules, not a relabelled TP.

### Other checks (fresh run, this commit)

`kibitzer check backtest feature-envy` exit 0 (4818 transcripts, 1999 snapshots); `cargo fmt --all
--check` ok; `cargo clippy --all-targets` no warnings; `cargo test --workspace` all pass (after
`cargo build --workspace`, which `plugin_contract` needs).
