# `remove-setting-method` check

Fowler's Remove Setting Method (`src/remove_setting_method.rs`,
[issue #57](https://github.com/tstapler/kibitzer/issues/57)). Run with
`kibitzer check architecture remove-setting-method`.

## Mechanism

Go only (field-write edges exist only for Go). A setter is a method named `Set*`/`set*`
(uppercase after the prefix) that writes exactly one receiver field, reads no field, and makes
no calls. It is flagged when no call site names it (callee's last segment, resolved or not), or
every call site is a same-directory `New*`/`new*` function whose name contains the setter's
type name (or is bare `New`/`new`).

Skipped: exported setters outside an `internal/` path segment, test files, and types with an
`implements` edge. Callers match by name alone, so a same-named setter on another type
suppresses the finding (fewer findings, never more).

Known limits: guard-only setters (`if v == "" { return }; x.f = v`) and method values
(`register(x.setName)`) are invisible to the model; a fluent builder setter used only outside
the repo is indistinguishable from an unused one.

## Suppression

`kibitzer:ignore` directives are **not** applied to this check (architecture checks ignore them
except `encapsulate-variable`; see `suppressing-checks.md`). Scope or disable it through
`.kibitzer/inspect.json`. It is not in `default_checks()`: its false-positive rate is not yet
established on real code (see below), so it stays opt-in via `kibitzer check architecture`.

## Backtest (release build, final code)

| Repo | Findings | Time | Reading |
|------|----------|------|---------|
| kubernetes/kubernetes | 0 | 46s | — |
| tstapler/stapler-squad | 0 | 11s | — |
| cli/cli | 0 | 5s | — |
| go-chi/chi | 0 | <1s | — |
| caddyserver/caddy | 0 | 2s | — |
| hashicorp/terraform | 3 | 10s | `ResourceInstanceBuilder.SetAddr/SetResourceInstanceObjectSrc/SetProviderAddr` (`internal/stacks/stackstate/state_builder.go`); `grep -rnE` over the whole repo finds no caller, so true positives (dead builder API). Name-based grep would miss reflection callers. |

`kibitzer check backtest remove-setting-method` (transcript replay) scanned 4802 transcripts,
2030 snapshots checked, with no findings.

Not established: the "only called from constructors" branch fired on none of these repos, so it
is covered by unit tests only. The sample (3 findings, one pattern) is too small to claim a
false-positive rate. `cli/cli`, `go-chi/chi` and `caddy` are not in `docs/backtest-repos.md`;
they were added as extra Go repos.
