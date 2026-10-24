# Go package focus

`go-package-focus` is an **advisory** check that asks one question of a Go package: do two
or more groups of types in it never reference each other? It produces review leads, never a
verdict. Package size, compile time and "SOLID scores" are not evidence of lost focus, and
no output of this check is CI-gateable (see [ADR-001](../project_plans/go-package-focus/decisions/adr-001-advisory-only-no-ci-gate.md)).

Run it: `kibitzer check architecture go-package-focus <dir>` (text), or
`kibitzer go package-focus <dir> [--mode size-only|graph-only|combined]` (JSON).
It is registered in the architecture model registry but is **not** in `default_checks()`
yet; see [Threshold decisions](#threshold-decisions).

## What it measures

Every method collapses into its receiver type's node, so a single type whose methods fall
into groups is never reported here (that is Extract Class territory, see the inventory
below). Types and free functions are then joined by:

- resolved call edges inside the package (calls from `init()` are ignored),
- field accesses that reach another type's field,
- embedding / `implements` edges,
- parameter types (a function taking `*ParseContext` belongs with `ParseContext`).

A package qualifies when it has at least 2 components that each hold at least 2 types and at
least 4 types overall. Non-`.go` files, `_test.go` files and generated files are excluded.

## Existing receiver and SOLID signals

| Signal | Source | Scope | Known coverage gaps |
| --- | --- | --- | --- |
| LCOM | `src/architecture_checks.rs` `LcomChecker` | method groups within one type | does not see whole-package responsibilities |
| God Class | `src/god_class.rs` | one type (ATFD/WMC/TCC) | syntactically typed locals only |
| Extract Class candidates | `src/extract_class.rs` | method groups within one type | methods without field accesses are left out |
| ISP fat interface | `src/isp_fat_interface.rs` | interface vs. its actual consumers | embedded/promoted methods, consumers reached through fields, containers, some returns, dot imports |
| DIP concrete coupling | `src/architecture_checks.rs` `DipConcreteCouplingChecker` | package-pair imports | concrete dependencies alone are not violations |
| Instability / abstractness | `src/architecture_checks.rs` `InstabilityChecker` | package-level `Ce`/`Ca` | counts distinct package pairs since the `fan_in_out` fix |
| OCP, LSP | none | none | not inferable statically; no check claims them |

`go-package-focus` does not duplicate these: the type super-node rule means a finding always
spans at least two types.

## Split decision checklist

Answer these before acting on a finding. Act only if every "yes" below holds.

1. Can you name the capability each component owns, in a few words?
2. Does one component own its state and dependencies (no field or lock the other reaches into)?
3. Is there a credible caller that needs only one of them? (One caller is enough; several is stronger.)
4. Are the resulting imports acyclic, in one direction?
5. Would the split avoid exporting private state or locks, and avoid clients importing both packages to do anything useful?
6. Would an in-package alternative (reorganize files, refactor a type) fix the same problem more cheaply?

Prefer `internal/` for the extracted package. Public support and module versioning are separate decisions.

## Keep-together signals

The check stays silent (or, where noted, marks the finding `[informational]`) when it sees:

- a shared parse/session context all symbols receive (cohesive large parser),
- package-level `sync`/`atomic`/`chan` variables (shared lock or state machine; silent, and the variable names are not surfaced),
- `init()` registration (the init edge is not a connection),
- a generated-file banner,
- `main` or `cmd/` packages with 5 or more distinct imports (wiring, reported `[informational]`),
- `_test.go`-only import cycles.

## Enforcement vs. advice

| Kind | Example | Gate in CI? |
| --- | --- | --- |
| Dependency policy | forbidden imports, declared layer direction (`dependency-rules`) | Yes, these are agreed facts |
| Structural advice | `go-package-focus`, instability, LCOM, God Class | No, advisory only |

An inferred responsibility boundary is a hypothesis; a forbidden import is a rule.

## Output

First line of an advisory finding:

```text
[go-package-focus] Candidate split in <pkg>: separate <A> from <B> (advisory)
```

Followed by one line per component (types, declaration count, state fields, unresolved
calls), then cross-boundary edges, import direction, required new exports, cycle risks, at
least one `contrary evidence:` line, and a `Coverage:` line. Size-only results and wiring
packages start `[go-package-focus] [informational]`.

### Programmatic output

`kibitzer go package-focus <dir>` prints a JSON array, one object per qualifying package,
ordered by package path:

| Field | Type | Meaning |
| --- | --- | --- |
| `package` | string | package path |
| `level` | `"advisory"` \| `"informational"` | |
| `hypothesis` | string | named responsibility hypothesis |
| `components[]` | object | `label`, `types[]`, `symbols[]` (declaration ids), `state_fields[]`, `external_calls[]` (unresolved callee text) |
| `cross_boundary_edges` | string[] | resolved references crossing the boundary; always empty today, because a report exists only when none cross |
| `import_direction` | string | direction a split would need |
| `required_new_exports` | string[] | symbols a split would have to export; always empty today, for the same reason |
| `cycle_risks` | string[] | cycle notes (test-file edges are ignored) |
| `contrary_evidence` | string[] | always at least one item |
| `coverage` | object | `private_symbols_skipped`, `generated_files_skipped`, `files_with_parse_errors`, `test_import_edges_excluded`, `unresolved_edges`, `notes[]` |

## Known coverage gaps

- **Return types, struct field types and interface satisfaction** are not edges. Two components can look disconnected while one returns the other's type.
- **Parameter-type matching** is a plain identifier scan; a local type name that coincides with a parameter or generic type-parameter name creates a false edge (this errs toward silence).
- **Generics**: type-parameterized code may inflate or hide edges (noted in `coverage.notes`).
- **Embedded interfaces** and promoted methods are only seen through explicit embedding edges.
- **Dynamic calls** (function values, reflection, interface dispatch) are unresolved; the count is in `coverage.unresolved_edges`.
- **Build tags / platform variants**: one configuration is modeled.
- **Incomplete history**: no change-history corroboration exists yet.
- **External consumers** of the package are unknown.
- Package-level shared state is detected from source text (`var` declarations mentioning `sync.`, `atomic.` or `chan `) and suppresses the whole package, a deliberately conservative rule.

## Threshold decisions

Provisional, not calibrated. Constants live in `src/go_package_focus.rs`.

| Constant | Value | Note |
| --- | --- | --- |
| types per component | 2 | graph-only mode uses 1 |
| minimum type nodes | 4 | the plan proposed 6, but 2 components x 2 types already implies 4, and 6 would reject the decoder/storage case |
| maximum type nodes | 60 | above it clustering is skipped and an `[informational]` note is emitted |
| size-only | 15 files or 120 symbols | informational only |

### Mode comparison

Fixtures (`testdata/dogfood-architecture/`):

| Package | Truth | size-only | graph-only | combined | Verdict |
| --- | --- | --- | --- | --- | --- |
| `mixed_concerns` | split | no finding | advisory | advisory | accept (graph, combined); size-only abstains |
| `cohesive_parser` | keep | no finding | no finding | no finding | accept (correct silence) |
| `shared_lock` | keep | no finding | no finding | no finding | accept (correct silence) |

Real corpus: `kubernetes/staging/src/k8s.io/apimachinery/pkg` (commit as cloned under
`~/code/github.com/kubernetes/kubernetes`). Totals: size-only 7 informational leads,
graph-only 50 findings, combined 8 (6 advisory, 2 "too large to cluster"). Verdicts are the
author's reading of type names and file layout, not owner-reviewed:

| Package | Mode(s) | Verdict | Rationale |
| --- | --- | --- | --- |
| `util/net` | combined | accept | interface/route probing vs. `PortRange` parsing are unrelated capabilities in separate files |
| `util/strategicpatch` | combined | reject | `kindItem`/`sliceItem` are helpers used by the patch code through locals; the edge is invisible to the model |
| `util/managedfields/internal` | combined | reject | `Manager`/`TypeConverter` are coupled through interface signatures (return types untracked) |
| `watch` | combined | abstain | `StreamWatcher` returns `Event`, an untracked edge; a `watch/stream` extraction may still be reasonable |
| `runtime/testing` | combined | abstain | test fixtures; a split is possible but low value |
| `api/validate` | combined | abstain | 832 unresolved edges and generics make the evidence weak |
| `api/validate/constraints` | graph-only only | reject | `Integer`/`Unsigned` are sibling constraint types of one capability |
| `conversion/queryparams` | graph-only only | reject | `Marshaler`/`Unmarshaler` are mirror interfaces |
| `runtime`, `apis/meta/v1` | combined | not adjudicated | too large to cluster |

Reading: graph-only is far too noisy (50 findings, sampled ones rejected); combined removes
most of that but still rejects 2 of its 3 decided cases (1 accept). Roughly **2 of 3 decided
combined findings were false positives**, far above the 20% bar the plan set, from a sample of
three decisions, which is too small to be a rate. **Do not add `go-package-focus` to
`default_checks()`** until the model tracks return/field types and the adjudication is
repeated on more packages with the owner.

## Build cost (separate axis)

`kibitzer go build-cost <dir>` runs `go build -debug-actiongraph` twice (cold, then warm)
with a private `GOCACHE`, so the shared cache is never cleared or written, and prints action,
compiled and cache-hit counts. On Go below 1.19, or above the newest tested version (1.26),
it prints a diagnostic and exits 0; the same applies when no `go` binary is on `PATH`. `-debug-actiongraph` is internal Go machinery with an
unversioned schema; the parser reads only `Mode` and `Cmd`. Do not run it from per-edit
hooks, and do not read build seconds as evidence about responsibilities. See
[ADR-002](../project_plans/go-package-focus/decisions/adr-002-build-cost-debug-actiongraph.md).

Example on a 7-action module with Go 1.26: cold 3 compiled / 0 cache hits, warm 0 compiled / 3 cache hits.

## `fan_in_out` correction

Coupling (`Ce`/`Ca`) now counts each distinct `(from, to)` package pair once; before, every
import statement in every file counted. Backtest before/after on `kubernetes/kubernetes` and
`BurntSushi/ripgrep` (release builds of `ad3202d` vs. this change):

| Check | Repo | Findings before | Findings after | Notes |
| --- | --- | --- | --- | --- |
| coupling | kubernetes | 1896 lines | 1243 | 631 distinct finding identities removed, 0 added; counts such as "imported by 25 distinct packages" were raw edge counts (17 distinct for `k8s.io/api/batch/v1beta1`) |
| coupling | ripgrep | 4 | 0 | all four were raw-edge counts (e.g. `ignore` "imported by 27") |
| instability | kubernetes | 370 | 381 | 29 identities removed, 40 added |
| dip-concrete-coupling | kubernetes | 1222 | 1241 | 173 removed, 194 added, same cause as instability |
| instability, dip-concrete-coupling | ripgrep | 8, 5 | 8, 5 | unchanged |

The added instability and DIP findings are not new problems: deduplication lowers `Ca` more
than `Ce` for packages imported from many files, so `I = Ce/(Ca+Ce)` moves across the 0.3
threshold in both directions. Example: `k8s.io/api/batch/v1beta1` had `Ca` 25 (raw) vs. 17
(distinct) and now reports `I=0.23`, in the zone of pain. This is a metric correction, not
"only removals"; the plan's expectation of removals only did not hold for the
`I`-threshold checks.

Reproduce with `scripts/fan-in-out-backtest.sh ad3202d` (builds the baseline in a throwaway worktree, prints identity counts with numbers normalized; re-run on 2026-10-24 with identical counts; the script uses `$CARGO_TARGET_DIR` for the current tree and needs several GB, so keep it off a small tmpfs). Not every change is a removal: `coupling` only loses findings, while `instability` and `dip-concrete-coupling` also gain some, all traced to the `Ca` shift above.

## Changing thresholds

Backtest first: [backtesting.md](backtesting.md) and [backtest-repos.md](backtest-repos.md).
