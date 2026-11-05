# `remove-setting-method` check

Fowler's Remove Setting Method (`src/remove_setting_method.rs`, issue #57). Run with
`kibitzer check architecture remove-setting-method`.

## Mechanism

Go only (field-write edges exist only for Go). A setter is a method named `Set*`/`set*` that
writes exactly one receiver field and reads none. It is flagged when no call site names it
(callee's last segment, resolved or not), or every call site is a same-directory function
named `New*`/`new*`. Skipped: exported setters outside `internal/` packages, test files, and
types with an `implements` edge. Suppress with `// kibitzer:ignore remove-setting-method -- <why>`.

## Backtest (release build)

| Repo | Findings | Reading |
|------|----------|---------|
| kubernetes/kubernetes | 0 (44s) | no findings to triage |
| tstapler/stapler-squad | 0 | — |
| cli/cli | 0 | — |
| go-chi/chi | 0 | — |
| hashicorp/terraform | 3 | `ResourceInstanceBuilder.SetAddr/SetResourceInstanceObjectSrc/SetProviderAddr` (`internal/stacks/stackstate/state_builder.go`): `grep -rnE` over the whole repo finds no caller, so true positives (dead builder API). |

Known limit: fluent/builder setters that are called only from outside the repo are not
distinguishable; the model has no return-type information. Not in `default_checks()` — it is a
whole-repo model checker like `remove-middle-man`, run via `kibitzer check architecture`.
