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

| Pattern | Example | Status |
|---|---|---|
| Access through a defined type that is not a struct: `type unmounterDefaults flexVolumeUnmounter` | kubernetes `flexVolume.mounter` is also read by `unmounterDefaults.TearDownAt`, but the model drops accesses from a receiver that declares no fields | Not filtered. |
| Field set from a free function or through a local value rather than a method receiver | kubernetes `typesFromEnv.deviceType`/`attributeType` are assigned via `result.deviceType = ...` in `compile.go`, not from `sizeEstimator` | Not filtered (2 hits). |
| Field read by reflection / struct tags / serialization | Invisible to the model | Not filtered. |
| Access through a non-receiver variable (`x.field`, `x` a local) | Only receiver accesses are recorded | Mitigated for exported fields by the unexported-only rule. |

The checker asks for private symbols (`needs_private_symbols`): subtypes are often unexported, and without
them the first corpus pass saw only 3 of the 17 hits below.

## Backtest (release build, final)

`kibitzer check architecture push-down-field <repo>`: kubernetes 17; cassandra, nowinandroid, servo,
ripgrep, deno, vscode, stapler-squad 0. All 17 were read against the source (`grep` of every use of
the field in its package). **3 false positives of 17 = 17.6%** (the first two patterns above), by one reviewer;
n is small. The 14 plausible hits are the `flexVolume` pod fields, `downwardAPIVolume.pod`, `hostPath.pathType`,
two `localVolume` fields, four `patcher` fields (each used by exactly one of `jsonPatcher`/`smpPatcher`), and the
three `*Opts.initializeLabelAllowListsOnce` `sync.Once` fields used only by the `*Vec` types.
Earlier builds: 12 of 12 sampled hits were false positives before the unexported-only rule; one stapler-squad hit
(`ReviewState.lastAcknowledgedNs`) was removed once the supertype's own accesses blocked a finding.
Transcript replay: 0 findings over 1818 snapshots.

## Suppression

Add `kibitzer:ignore push-down-field -- <why>` above the flagged line, or disable the check per repo
(`disabled: ["push-down-field"]`); see `docs/suppressing-checks.md`.
