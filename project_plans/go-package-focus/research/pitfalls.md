# go-package-focus: Known Pitfalls and Risks

## 1. Metric-change risk from `fan_in_out` deduplication

**Root cause.** `fan_in_out` (`src/architecture_checks.rs:180-187`) currently increments a counter for every `ImportEdge`, not for every distinct `(from, to)` package pair. A package that imports the same dependency across ten files produces ten fan-out counts today. Deduplication (via a `HashSet<(&str, &str)>` keyed on `(from, to)`) will collapse those to one.

**Regression surface.** Three checkers call `fan_in_out` and use its return values directly:

- `CouplingChecker` — compares raw counts against `MAX_FAN_OUT`/`MAX_FAN_IN` (both 10). In any repo where a package spreads a single dependency across multiple files, the deduplicated count may fall below 10 where the raw count exceeded it, silently dropping existing findings.
- `InstabilityChecker` (`src/architecture_checks.rs:614`) — uses `fan_in_out` on `model.import_edges` to compute Martin's I metric. Deduplication shifts I values, potentially moving packages in or out of "zone of pain" / "zone of uselessness."
- `DipConcreteCouplingChecker` (`src/architecture_checks.rs:680`) — also calls `fan_in_out` on model edges; the same I-value shift applies.

**Test fixture risk.** The three coupling unit tests (`coupling_flags_high_fan_out`, `coupling_flags_high_fan_in`, `coupling_below_threshold_has_no_findings`, in `src/architecture_checks.rs:1124-1173`) and the instability tests (lines 1734–1820) construct synthetic edges where each `(from, to)` pair is unique, so they pass identically before and after deduplication. They do **not** cover the multi-file-same-pair case; a test regression will not fire — but real-world behavior changes. Any corpus results or accepted-findings files that store specific instability/coupling counts will diverge silently.

**Mitigation.** Before landing the dedup, run `kibitzer architecture assessment` against the backtest corpus repos and compare finding counts per checker. Mark diffs as expected regressions in `scripts/backtest-triage.py`. Bump `MAX_FAN_OUT`/`MAX_FAN_IN` only if the deduplicated metric proves too noisy at the new baseline.

---

## 2. False-positive categories for package-focus analysis

**2a. Generics-induced import fan-out inflation.**  
In Go 1.18+ a package that instantiates several generic types (e.g. `slices`, `maps`, `cmp`, `constraints`) accumulates imports that represent parameter choices rather than real dependencies. The package-focus checker will see a high fan-out count for any utility package that is the instantiation site for several generics. This mimics the "spreads across many concerns" signal the checker is meant to detect, but the cause is a language affordance, not a design problem.

**2b. `init()` registration patterns.**  
Go packages that exist purely to register side effects (database drivers, codec handlers, format plugins registered via `init()`) produce import edges that have no caller relationship at the symbol level. Their fan-out is real by design — they depend on many sub-registrants — but splitting them is actively harmful. The checker should suppress findings for packages where all cross-package calls are inside `init()` functions; without that suppression, every plugin-registry package in kubernetes or Deno's Go ports will fire.

**2c. Test-only import cycles via external test packages.**  
Go allows `package foo_test` (external test packages) in `_test.go` files. These can import packages that the production `package foo` cannot, creating import edges that exist only at test time. The import-graph walker (`src/import_graph.rs`) does not currently special-case `_test.go` files: they are walked as ordinary Go files. A package-focus check that sees test-only imports will flag high fan-out on test helpers and integration-test packages that are deliberately broad. The fix is to exclude `*_test.go` files from the edges fed to the package-focus checker, or gate the checker on a `--include-test-imports` flag.

**2d. Generated code that escapes `looks_generated`.**  
`arch_model::looks_generated` (`src/arch_model.rs:300-303`) searches for "do not edit" or "code generated" case-insensitively across the whole file. Generated files without those banners (e.g. `stringer`-generated `*_string.go` files, `mockgen` output using a custom banner, vendor copies) are not skipped. A package whose many files are machine-generated will show inflated symbol counts and cross-package edges that do not reflect the real design.

**2e. Monorepo command packages.**  
In repos like `kubernetes/kubernetes`, a `cmd/kubelet` package legitimately wires together dozens of subsystems; its high fan-out is structural glue, not a focus problem. Without a configurable exemption list (or a `kibitzer:ignore` directive), the checker fires on every such entry point. The `advisory`-only constraint limits blast radius, but the false-positive rate on corpus repos will be high enough to undermine the check's credibility unless the message explicitly says "this is a likely wiring/entry-point package" when fan-out exceeds some secondary threshold.

---

## 3. Build-cache safety

**Requirement.** The prototype build-cost collector must not clear or corrupt the shared Go build cache (`GOCACHE`, typically `~/.cache/go/build`).

**Risk with `-debug-actiongraph`.** `-debug-actiongraph=<file>` is passed as a flag to `go build`/`go test` and causes the Go toolchain to write a JSON action graph to the named path. It does not by itself invalidate the cache. However:

- Passing additional flags that change what is compiled (e.g. `-race`, non-default build tags, or any flag that differs from the normal build invocation) will cause Go to produce and cache artifacts under different action IDs, occupying additional space without evicting the existing cached objects. Repeated runs with differing flags can cause cache bloat.
- If the collector shells out to `go build ./...` with flags the project does not normally use, it trains the cache on a different build configuration and may evict warm entries the developer relies on. The safe approach is to run a `go list -json ./...` (which reads but does not write the build cache) and infer package-level dependency depth from the import graph already in the arch model, rather than running a real build.
- `GOFLAGS` or `GOENV` overrides in the user's environment could cause the collector's `go build` invocation to behave unexpectedly (e.g. enabling modules proxy bypass, CGO settings).

**Mitigation.** Prefer `go list -json -deps ./...` over `go build` for the prototype. If a build invocation is genuinely needed, always set `GOCACHE=$(mktemp -d)` in the subprocess environment and clean it afterward, explicitly documented in code. Never call `go clean -cache` — that clears the caller's shared cache.

---

## 4. Go toolchain version risks for `-debug-actiongraph`

`-debug-actiongraph` is not documented in `go help build` or the Go specification. It is an internal diagnostic flag used by the Go team to inspect the build system's action graph. Known risks:

- **Format instability.** The JSON schema of the action graph output has changed between Go versions (keys added, renamed, nested differently). Code that parses it against a fixed schema will break silently on a schema addition or loudly on a field removal. Any parser must treat missing fields as optional and version-gate fields that only appear in Go 1.21+.
- **Flag removal.** Because it is undocumented, it can be removed without a deprecation notice. The collector must probe for the flag's existence (e.g. by running `go build -debug-actiongraph=/dev/null -n ./...` and checking the exit code) before relying on it, and degrade gracefully to no build-cost data rather than hard-failing.
- **Behavior differences across platforms.** On Windows, `/dev/null` does not exist; the path must be a real temp file. The Go toolchain on ARM or WASM targets may not populate the same action graph fields as on amd64 Linux.
- **`-toolexec` alternative.** `-toolexec` is a stable, documented flag (since Go 1.5) that wraps each compiler invocation. It is more portable but adds complexity: the wrapper binary must be compiled separately, creating a bootstrap dependency, and it runs in a subprocess of the Go toolchain with no shared state with the Kibitzer process. Timing data from `-toolexec` is per-tool invocation, not per package — mapping it back to packages requires parsing the compiler arguments passed to the wrapper.

---

## 5. Test fixture regression risk

**Existing test structure.** Coupling tests (`coupling_flags_high_fan_out`, etc.) use hand-constructed `ImportEdge` slices with distinct `(from, to)` pairs, so they are insensitive to the dedup change and will not catch a regression in existing behavior. Instability tests (lines 1734–1820) similarly use one edge per distinct pair.

**No golden-output fixtures.** There are no snapshot or golden files in `testdata/` that assert specific fan-out counts or instability scores. The regression risk is in the corpus backtest, not in unit tests.

**New fixture requirements for go-package-focus.** The new checker needs at minimum:

1. A fixture with a package that has multiple files all importing the same dependency — to verify that deduplication does not suppress a real high-fan-out finding.
2. A fixture with a `_test.go` file whose imports would inflate fan-out if included — to verify test-file exclusion.
3. A fixture with a `// Code generated` banner package — to verify it is excluded from package-focus analysis.
4. A fixture that exercises the `init()`-registration pattern — to verify it does not fire a false positive.

Without these, the check passes its own unit tests but fires incorrectly on the corpus repos used for backtesting.
