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
| Supertype must have a role: at least one field or method | `type ExtraConfig struct{}` is an extension hook, not a place to pull state into (2 kubernetes false positives). |
| Same-named siblings skipped; tests, `fixtures/`, `testdata/`, `output_tests/` (codegen golden output) and unresolved edges ignored | Copies and test data, not siblings. |

## Known false-positive patterns (these remain)

| Pattern | Example | Status |
|---|---|---|
| "Completed" wrapper beside its source (`RecommendedConfig` and `completedConfig` both embed `Config`) | kubernetes `apiserver/pkg/server` | Not filtered. |
| Snapshot/DTO types that mirror an entity on purpose | stapler-squad `Instance` vs `InstanceSnapshot` (43 fields, 1 finding) | Not filtered. |
| A shared service singleton embedded for dependency access | docker/compose `imagePuller`/`upSession` both embed `*composeService` and each hold `project`/`mu`; hoisting per-project state into the shared service is wrong | Not filtered. |
| A carrier type whose name says it holds nothing | coredns `object.Empty` (2 findings) | Not filtered. |
| Versioned API types mirrored across `v1`/`v1beta*` | Reported once per version; the duplication is real but dictated by the wire format | Counted as true positives. |

## Backtest (release build, final; `kibitzer check architecture pull-up-field <repo>`)

Every finding was read against the source by one reviewer.

**Standard corpus** (`docs/backtest-repos.md`): 19 findings (kubernetes 18, stapler-squad 1); cassandra,
nowinandroid, servo, ripgrep, deno, vscode 0. 2 false positives (the `completedConfig` wrapper and the stapler-squad
DTO mirror): **2/19 = 10.5%**. This number is optimistic: the empty-supertype and `output_tests/` guards were
added after reading the earlier 23-finding run of this corpus, and they removed 4 of its 6 false positives.

**Held-out repos** (not used to design any guard): argo-cd, caddy, trivy, containerd, etcd, docker/compose, syft, pack, cli, coredns (ten Go repos named before any of this check's output on them was seen). 15 findings (coredns 5, cli 3, syft 3,
docker/compose 2, argo-cd 1, containerd 1; the rest 0). 3 false positives (compose `composeService`, coredns `Empty` x2):
**3/15 = 20.0%**, right at the bar, n is small. This is the honest estimate. All held-out repos are Go; there was no
Java/Kotlin/TS field data to evaluate.

Combined 5/34 = 14.7%. History: the first build gave 1116 kubernetes findings, the Go same-package and majority guards
cut that to 23 with 6 wrong (26%), and the two guards above gave the current figures. Transcript replay: 0 findings
over 1818 snapshots.

## Suppression

Add `kibitzer:ignore pull-up-field -- <why>` above the flagged line, or disable the check per repo
(`disabled: ["pull-up-field"]`); see `docs/suppressing-checks.md`.
