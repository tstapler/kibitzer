# Backtest results

Old = installed Homebrew `kibitzer` (pre-change); new = `target/release/kibitzer` built at 6dd5ff3.

## Transcript backtest (AC9)

`kibitzer check backtest syntax-rules`, `[long-function]` lines only, deduplicated: old 187, new 187,
no difference. The transcript set has no flat-literal case, so this shows no regression, not
coverage of the new branch; the corpus run below covers it.

## kubernetes/kubernetes sample (AC10)

13,265 Go files under `pkg/`, `staging/`, `test/`, `cmd/` (vendor excluded), one
`check native syntax-rules <file>` per file. `[long-function]` findings keyed by `file:line`:
old 15,657, new 15,443. Silenced 214, newly introduced 0.

| Silenced | Where |
|---|---|
| 210 | generated `zz_generated.openapi.go` schema tables (6 files) |
| 1 | `pkg/controlplane/apiserver/aggregator.go:313` `DefaultGenericAPIServicePriorities` |
| 1 | `pkg/kubelet/images/pullmanager/mem_pullrecords_test.go:1198` `testPulledRecords` |
| 1 | `kubectl/pkg/cmd/get/get_test.go:1040` `sortTestTableData` |
| 1 | `kubectl/pkg/cmd/testing/fake.go:653` `testDynamicResources` |

Opened the four hand-written ones: each is a function that returns a single data literal, and none has
extractable logic. The 210 generated entries are `return common.OpenAPIDefinition{...}` schema tables.
Only the first lines of each hand-written function and one generated function were read, not every
generated entry.
