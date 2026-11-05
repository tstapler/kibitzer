# go-type-switch-clustering: backtest and known false positives

Advisory, Go-only, `batch` granularity. Fires when 2+ methods of one type each contain a
`switch x.(type)` (2+ cases) on the same receiver-rooted subject. Recommends *Replace
Conditional with Polymorphism* when `ArchModel.type_edges` links a case type (a hierarchy
exists), otherwise *Replace Type Code with Subclasses*.

## Subject matching

Only subjects rooted at the method's own receiver variable count (`r.kind`, `r`,
`r.kind()`), with the receiver name normalized so `r.kind` and `c.kind` match across
methods. Parameters and locals (`obj`, `v`) are skipped: two methods naming a parameter
alike do not share a type-code field. Pinned by `same_parameter_name_does_not_cluster`.

## Backtest (2026-11-04, release build)

Command per repo: `kibitzer check architecture go-type-switch-clustering <repo>`
(corpus from `docs/backtest-repos.md`; clones under `~/code/github.com/`).

| Repo | v1: any exact subject | Shipped: receiver-rooted only |
|---|---|---|
| kubernetes/kubernetes | 3 | 0 |
| tstapler/stapler-squad | 1 | 0 |
| golang/tools | 16 | 3 |
| hashicorp/terraform | 8 | 4 |
| cockroachdb/cockroach | 43 | 4 |
| **Total** | **71** | **11** |

The v1 run matched bare parameter names as subjects; 53 of its 71 findings were of that
shape, and the one such site read (kubernetes `pkg/scheduler/eventhandlers.go`, `obj`) was
informer-callback arguments, not a type-code field. Review of the first draft flagged this,
so the shipped checker drops them.

`kibitzer check backtest go-type-switch-clustering` (transcript replay) was also run: 2057
snapshots, 0 findings — expected, since this is a `batch` ArchModel checker and
single-file edit replay doesn't exercise it.

### True-positive floor (AC 7)

Needs 2+ true positives at exact matching. Verified by grepping the source for the repeated
switch on the same field (the finding's method list names the distinct methods):

- `hashicorp/terraform` `internal/addrs/move_endpoint_module.go`: `switch … e.relSubject.(type)`
  at lines 84, 133, 171, 200 (and on `fromMatch`/`from` receivers elsewhere).
- `hashicorp/terraform` `internal/lang/globalref/reference.go`: `r.ContainerAddr.(type)` at 73, 92.
- `hashicorp/terraform` `internal/terraform/node_action_invoke.go`: `n.ActionInvocation.ActionTrigger.(type)` at 305, 321.
- `golang/tools` `go/ssa/ssa.go`: `c.Value.(type)` at 1494, 1506.
- `golang/tools` `gopls/.../completion`: `c.path[1].(type)` in keywords.go:83, snippet.go:70, statements.go:62.

Five genuine clusters, so strip-receiver normalization was not needed to meet the floor
(it is in the shipped code anyway). The remaining findings (cockroach `op.GetValue()`,
`ex.machine.CurState()`, `hint.GetValue()`, `w.desc`; terraform `p.shape`) were not
individually read.

## Deviations from requirements.md

- **Receiver normalization ships in v1.** `project_plans/type-switch-clustering/requirements.md`
  ("Rabbit Holes") asked for conservative full-string comparison. The shipped checker strips
  the method's own receiver variable so `r.kind` and `c.kind` cluster, and skips non-receiver
  subjects instead. That receiver-rooted rule is what cut findings from 71 to 11.
- **"Zero false positives on the corpus" is not proven.** 11 findings remain and only 5
  clusters were individually verified as true positives; the rest were not read.

## Known limits

- **Hierarchy detection is explicit-embedding only.** Go `type_edges` come from struct or
  interface embedding, never structural satisfaction (`arch_model.rs`), so the common shape
  (concrete types implementing an interface implicitly) yields *Type Code with Subclasses*
  even when an interface already exists. Case types are matched by bare name, not by
  package, so an unrelated same-named type can flip the recommendation.
- Package-level variable subjects are skipped.
- Cluster rule is "same subject in 2+ methods"; case-type overlap is not required.

Suppress a single site with `kibitzer:ignore go-type-switch-clustering -- <why>` above the
flagged line (see `docs/suppressing-checks.md`).
