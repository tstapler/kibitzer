# `push-down-field` check — known false positives and tuning

`push-down-field` (`src/push_down_field.rs`) is Fowler's *Push Down Field*: a field declared on a
supertype that is accessed only from methods of exactly one direct subtype (or its descendants).
Advisory, batch-only, on by default. Source: `ArchModel.struct_field_names` plus `field_accesses` and
`promoted_field_accesses` (Go only; other languages never fire).

`promoted_field_accesses` exists because Go reads an embedded field as `a.N` on the *embedding* struct,
which `field_accesses` does not attribute to the declaring type. Without it the check could never fire
on Go: the first corpus run returned 0 hits for that reason, not because the code was clean.

## What it requires

| Gate | Why |
|---|---|
| Unexported fields only | Accesses are recorded only through a method's own receiver, so for an exported field the absence of other accessors proves nothing (the first 12 sampled hits were all exported API fields: 100% false positives). |
| Supertype has ≥2 direct resolved subtypes, all in the supertype's package | A Go subtype in another package is composition with a library type. |
| ≥1 access, all from exactly one direct subtype's subtree | No access evidence is skipped. |
| No access from the supertype's own methods or from outside the hierarchy | Moving the field down would break them (stapler-squad `ReviewState.lastAcknowledgedNs`). |
| Test, `fixtures/`, `testdata/` files and unresolved `Extends` edges ignored | Shared guard. |

## Known false-positive patterns

| Pattern | Why it fires | Status |
|---|---|---|
| Access through a non-receiver variable (`x.field` where `x` is a local) | Only receiver accesses are recorded. | Not filtered; mitigated for exported fields by the unexported-only rule. |
| Field read by reflection / struct tags / serialization | Invisible to the model. | Not filtered. |

## Backtest (release build, final)

`kibitzer check architecture push-down-field <repo>`: kubernetes 3, all other seven repos 0.
All 3 were read against the source: the unexported `initializeLabelAllowListsOnce` (`sync.Once`) on
`HistogramOpts`/`TimingHistogramOpts`/`SummaryOpts`, used only inside the `*Vec` types' methods
(`grep` over `component-base/metrics/*.go`); none is used by the plain (non-Vec) types. **0 false
positives of 3 (0%)**, n is small. Transcript replay: 0 findings over 1818 snapshots.

## Suppression

Add `kibitzer:ignore push-down-field -- <why>` above the flagged line, or disable the check per repo
(`disabled: ["push-down-field"]`); see `docs/suppressing-checks.md`.
