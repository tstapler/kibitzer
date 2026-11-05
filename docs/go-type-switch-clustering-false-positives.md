# go-type-switch-clustering: backtest and known false positives

Advisory, Go-only, `batch` granularity. Fires when 2+ methods of one type each contain a
`switch x.(type)` on the same subject string. Recommends *Replace Conditional with
Polymorphism* when `ArchModel.type_edges` already links a case type (a hierarchy exists),
otherwise *Replace Type Code with Subclasses*.

## Backtest (2026-11-04, release build, exact subject matching)

Command per repo: `kibitzer check architecture go-type-switch-clustering <repo>`
(corpus from `docs/backtest-repos.md`; clones under `~/code/github.com/`).

| Repo | Findings | Receiver-qualified subject (`x.field`, `x.f()`) | Bare identifier subject |
|---|---|---|---|
| kubernetes/kubernetes | 3 | 0 | 3 |
| tstapler/stapler-squad | 1 | 0 | 1 |
| golang/tools | 16 | 4 | 12 |
| hashicorp/terraform | 8 | 6 | 2 |
| cockroachdb/cockroach | 43 | 8 | 35 |
| **Total** | **71** | **18** | **53** |

(`go-type-switch-clustering` counts only; counted with `grep -c` on the saved output.)

`kibitzer check backtest go-type-switch-clustering` (transcript replay) was also run:
2057 snapshots checked, 0 findings. Expected: it is an ArchModel (`batch`) checker, so
single-file edit replay does not exercise it.

### True-positive floor

The AC-7 gate is "<2 true positives at exact matching ⇒ try strip-receiver normalization".
The floor is met without normalization. Verified by reading source:
`hashicorp/terraform` `internal/addrs/target_pattern.go` switches on the field `p.shape`
in two `TargetPattern` methods (lines 83 and 155) — a genuine repeated type-code switch.
The other receiver-qualified findings were classified by subject shape only, not read
individually (INFERRED).

### False-positive class: coincidental parameter names

Bare-identifier subjects (`obj`, `n`, `v`, `e`) are usually *parameters*, not a shared
field, so two methods that happen to name a parameter alike cluster spuriously. Verified
example: `kubernetes/kubernetes` `pkg/scheduler/eventhandlers.go` (`Scheduler` on `obj`,
lines 106/200/517/588) are informer-callback arguments, not a type-code field.
The checker deliberately does not normalize receiver names or parameters (conservative, per
AC-2); `same_parameter_name_is_not_normalized` pins the current behavior. Bare-identifier
findings are therefore best treated as low-confidence.

Suppress a single site with `kibitzer:ignore go-type-switch-clustering -- <why>` above the
flagged line (see `docs/suppressing-checks.md`).

## Follow-up (not done here)

Consider down-ranking findings whose subject is a bare method parameter (53 of 71 above).
Only one such site was read (the kubernetes example), so the true-positive rate within
that group is unmeasured.
