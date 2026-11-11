# `pull-up-field` check — known false positives and tuning

`pull-up-field` (`src/pull_up_field.rs`) is Fowler's *Pull Up Field*: ≥2 direct sibling subtypes
declare a field with the same name **and** the same type text, and the supertype does not declare it.
One finding is reported per sibling set, listing up to five fields (two mirror structs sharing 43
fields is one design question). Advisory, batch-only, on by default. Source: `ArchModel.struct_field_names`
and `struct_field_types` (Go structs only today; other languages never fire).

## What it requires

| Gate | Why |
|---|---|
| Same field name and same non-empty type text | Name-only matches (`name`, `id`, `mu`) are noise. |
| More than half of the supertype's direct subtypes declare it | 3 of dozens under a mixin (`TypeMeta`, `lazyMetric`) is not a missing field. The first build had 5 of 12 sampled hits wrong largely for this reason. |
| Go subtypes must be in the supertype's package | Embedding a foreign type (`metav1.TypeMeta`) is composition; the first build reported 1116 hits in kubernetes. |
| Same-named siblings skipped; tests/fixtures/unresolved edges ignored | Copies, not siblings. |

## Known false-positive patterns (these remain)

| Pattern | Example | Status |
|---|---|---|
| "Completed" wrapper beside its source (`Config` and `completedConfig` both embed a base) | kubernetes `kube-apiserver/app`, `apiserver/pkg/server` (3 hits) | Not filtered. |
| Snapshot/DTO types that mirror an entity on purpose | stapler-squad `Instance` vs `InstanceSnapshot` (43 fields, 1 finding) | Not filtered. |
| Generated or codegen test data | kubernetes `validation-gen/output_tests/...` (2 hits) | Not filtered (`output_tests` is not an ignorable directory). |
| Versioned API types mirrored across `v1`/`v1beta*` | Reported once per version; the duplication is real but the shape is dictated by the wire format | Counted as true positives. |

## Backtest (release build, final; `kibitzer check architecture pull-up-field <repo>`)

kubernetes 22, stapler-squad 1; cassandra, nowinandroid, servo, ripgrep, deno, vscode 0. All 23 findings
read against the source: 17 plausible, **6 false positives = 26%** (3 wrapper, 2 generated, 1 DTO mirror),
by one reviewer. **This is above the 20% bar**; it ships with the threshold (majority of subtypes) and the
categories above documented, as the acceptance criterion allows. Every false positive is in Go; no
Java/Kotlin/TS field data exists yet. Transcript replay: 0 over 1818 snapshots.

## Suppression

Add `kibitzer:ignore pull-up-field -- <why>` above the flagged line, or disable the check per repo
(`disabled: ["pull-up-field"]`); see `docs/suppressing-checks.md`.
