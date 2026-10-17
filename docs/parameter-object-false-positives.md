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
| kubernetes/kubernetes | 2410 | Go; see triage below |
| microsoft/vscode | 3231 | TypeScript; see triage below |
| BurntSushi/ripgrep | 0 | Rust, no param extraction |
| apache/cassandra | 0 | Java, no param extraction |
| denoland/deno | n/a | model build aborts: `stream did not contain valid UTF-8` reading a test-fixture `.js` file (`import_graph::build`, not this check) |
| servo/servo | n/a | same UTF-8 abort on a WPT fixture `.js` file |

`kibitzer check backtest parameter-object` scanned 3210 transcripts / 2396 snapshots and
produced no replayable signal: the check needs 3+ functions across the repo, which a
single reconstructed file rarely supplies. The corpus run above is the meaningful
evidence for this check.

## Triage

The checker includes unexported symbols (`needs_private_symbols`), since repeated groups
mostly live in internal helpers. An earlier exported-only run produced 1211 (k8s) and 275
(vscode) findings.

Method: read a 40-finding sample (20 per repo, `shuf` with a fixed seed) of the final
build and classified each by eye. This is a judgement call, not a ground-truth label set.

- About 14 of 40 looked like real candidates (e.g. `uid, gid`; `deltaTop, lineHeight,
  lineNumber`; `action, annotations, eventtype, reason`; `parentToolCallId, session`).
- About 20 of 40 were noise: (`db, sql`), (`params, signal`), (`..., level`),
  (`ctx, width`), `_`-prefixed names, callback parameters, framework hook shapes.
- The rest were ambiguous.
- The noise share is well above the ~20% false-positive target and the volume is high, so
  the check stays Advisory. A repo that finds it too loud can narrow it via
  `.kibitzer/inspect.json` (see `docs/suppressing-checks.md`) or extend
  `architecture.parameter_object_suppress`.

## False-positive classes

1. **Blank params** (`_ *v1.Pod, _ string`) grouped as if named. Params named `_` or
   unnamed are dropped.
   Functions with more than 10 usable params are skipped entirely.
2. **Injected services** (`logService ILogService`, `ServicesAccessor`) in VS Code. (m)
   Types shaped `I` + capital + lowercase (so not Go's `ID`), and `ServicesAccessor`, are
   dropped. Heuristic; it will also drop a real `IFoo` data interface.
3. **Interface implementations**: the same full signature on methods of 3+ distinct
   types. Skipped; a `ctx` parameter doesn't defeat the match. Free functions implementing one callback type (e.g. 23 k8s
   `validate.*` functions sharing `fldPath, op`) are still flagged.
4. **Test and fixture code**: `_test.go`, `*.test.ts`, `*.spec.ts` and `fixtures/`
   or `testdata/` directories. Skipped. Test helpers under other paths
   (`test/e2e/...`) are still flagged.
5. **Go `testing` boilerplate** (`t *testing.T`, `b *testing.B`). Added to the default
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
- **TypeScript rest/destructured params** (`...args`, `{a, b}`) are not extracted, unlike
  Go variadics.
- **Overlapping clusters**: a function can belong to several reported clusters, since only
  strictly-contained subsets with the same member count are collapsed.
