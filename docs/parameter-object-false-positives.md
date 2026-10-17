# `parameter-object` check — corpus results and known false positives

`parameter-object` (`src/parameter_object.rs`) flags a `(name, type)` parameter group of
2+ that recurs exactly across 3+ functions/methods (Fowler's *Introduce Parameter
Object*). It runs by default at Advisory severity, batch trigger only. Parameter data
exists for Go and TypeScript/TSX only (`SymbolNode::params`); other languages are skipped.

Reproduce: `kibitzer check architecture parameter-object <repo-dir>` (clone the corpus
with `scripts/clone-backtest-repos.sh`).

## Corpus run (final build)

| Repo | Findings | Notes |
|---|---|---|
| kubernetes/kubernetes | 1211 | Go; see triage below |
| microsoft/vscode | 275 | TypeScript; see triage below |
| BurntSushi/ripgrep | 0 | Rust, no param extraction |
| apache/cassandra | 0 | Java, no param extraction |
| denoland/deno | n/a | model build aborts: `stream did not contain valid UTF-8` reading a test-fixture `.js` file (`import_graph::build`, not this check) |
| servo/servo | n/a | same UTF-8 abort on a WPT fixture `.js` file |

`kibitzer check backtest parameter-object` scanned 3210 transcripts / 2396 snapshots and
produced no replayable signal: the check needs 3+ functions across the repo, which a
single reconstructed file rarely supplies. The corpus run above is the meaningful
evidence for this check.

## Triage

Method: read a 50-finding sample (25 per repo, `shuf` with a fixed seed) from the
build *before* the last three mitigations below, and classified each by eye. This is a
judgement call, not a ground-truth label set.

- Roughly 14 of 50 looked like real candidates (cohesive domain groups such as
  `host, port`, `commandLine, os, shell`, `c, name, ns`, `containerName, pod`).
- Roughly 25 of 50 were noise (classes below). The rest were ambiguous.
- That is well above the ~20% false-positive target, which is why the check stays
  Advisory. The sample predates the mitigations marked (m), so the current rate is lower
  by an amount not measured.

## False-positive classes

1. **Blank params** (`_ *v1.Pod, _ string`) grouped as if named. (m) Params named `_` or
   unnamed are dropped.
2. **Injected services** (`logService ILogService`, `ServicesAccessor`) in VS Code. (m)
   Types matching `I` + capital letter, and `ServicesAccessor`, are dropped. Heuristic;
   it will also drop a real `IFoo` data interface.
3. **Interface implementations**: the same full signature on methods of 3+ distinct
   types. (m) Skipped. Free functions implementing one callback type (e.g. 23 k8s
   `validate.*` functions sharing `fldPath, op`) are still flagged.
4. **Test and fixture code**: `_test.go`, `*.test.ts`, `*.spec.ts` and `fixtures/`
   or `testdata/` directories. (m) Skipped. Test helpers under other paths
   (`test/e2e/...`) are still flagged.
5. **Go `testing` boilerplate** (`t *testing.T`, `b *testing.B`). (m) Added to the default
   suppression list with `ctx context.Context`.

## Remaining known false positives (not mitigated)

- **Framework conventions** that repeat by contract: k8s conversion functions
  (`in *X, s conversion.Scope`), `(newObj, oldObj runtime.Object)` strategy hooks,
  `(w http.ResponseWriter, req *http.Request)`. Add them to
  `architecture.parameter_object_suppress` per repo; setting that key replaces the
  default list outright.
- **Old/new pairs** (`newPod, oldPod`): a legitimate pair, rarely a missing type.
- **Printf-style tails** (`format string, args ...interface{}`).
- **Callback parameters** (`handler (event: any) => void, node EventTarget`).
- **Overlapping clusters**: a function can belong to several reported clusters, since only
  strictly-contained subsets with the same member count are collapsed.
