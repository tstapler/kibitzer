# `push-down-method` check — known false positives and tuning

`push-down-method` (`src/push_down_method.rs`) is Fowler's *Push Down Method*: a method on a
supertype whose only resolved callers are methods of exactly one direct subtype (or that
subtype's descendants). Advisory, batch-only, on by default. Source: `ArchModel.call_edges`
(Go and TS/JS only; other languages never fire) plus resolved `Extends` edges.

## What it requires

| Gate | Why |
|---|---|
| Supertype has ≥2 direct resolved subtypes | With one, the smell is `collapse-hierarchy`. |
| ≥1 resolved caller, all inside exactly one direct subtype's subtree | No callers is "no evidence", not "unused". |
| No caller on the supertype itself, in another subtype, or outside the hierarchy (free functions included) | Moving the method down would break that caller. |
| Each caller's source line goes through the calling method's own receiver (`this.m()`, `recv.m()`) | `snap.m()` on another value of the supertype would break (found on stapler-squad's `ReviewState`). An unreadable line is accepted. |
| Exported, not used as a value, not a constructor, not overridden by any direct subtype | Overrides are polymorphism; value use is an unresolved caller. |
| Test, `fixtures/`, `testdata/` files and unresolved `Extends` edges ignored | Same guard as the other hierarchy checks. |
| Private symbols included (`needs_private_symbols`) | Callers and subtypes are often unexported; an unmodeled caller blocks the finding rather than being accepted. |

## Known false-positive patterns

| Pattern | Why it fires | Status |
|---|---|---|
| Caller reaches the method through a different value of the supertype | Call edges drop the receiver. | Mitigated by the receiver check above; a chained or multi-line call is not verified. |
| Callers in unmodeled files (generated code, other languages) | Not in the call graph. | Not filtered. |
| Reflection or interface dispatch | The call edge is unresolved, so it is invisible. | Not filtered; unresolved edges are not evidence either way. |

## Backtest (release build, final)

Every hit was read against the source by one reviewer; callers were checked with `grep` over the whole repo.

**Standard corpus** (`kibitzer check architecture push-down-method <repo>`, `docs/backtest-repos.md`): **0 hits** on all
eight repos. The first run on stapler-squad produced 1 false positive (the `snap.m()` case above), which the receiver
check removed.

**Held-out repos** (not used to design any guard): argo-cd, caddy, trivy, containerd, etcd, docker/compose, syft, pack, cli, coredns (ten Go repos named before any of this check's output on them was seen). **3 hits** (caddy 2, trivy 1), all sound within
the repo, so **0/3 false positives**, with the public-API caveat above. n is tiny, so treat the rate as "not yet
contradicted", not as a measured 0%.

Also shown by unit tests and a hand-built Go fixture (two embedded subtypes, each calling a different base method),
reported through the real binary. Transcript replay: 0 findings over 1818 snapshots.

## Suppression

Add `kibitzer:ignore push-down-method -- <why>` above the flagged line, or disable the check per repo
(`disabled: ["push-down-method"]`); see `docs/suppressing-checks.md`.
