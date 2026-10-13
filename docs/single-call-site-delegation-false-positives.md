# `single-call-site-delegation` check — known false positives

Tracks false-positive firings of `single-call-site-delegation`
(`src/single_call_site_delegation.rs`, with the shape test and call extraction in
`src/symbol_extract.rs` and call resolution in `src/arch_model.rs`). Check new occurrences
against this list before re-investigating from scratch. Interface-implementation misfires are
tracked as #117 and are not repeated here, except where TypeScript makes them much larger.

## Log

### 2026-10-12 — kubernetes/vscode corpus backtest — function used as a value (callback registration)

- **Repo**: `kubernetes/kubernetes` `staging/src/k8s.io/pod-security-admission/policy/check_sysctls.go:122`
  (`sysctlsV1Dot27`, registered as `CheckPod: sysctlsV1Dot27,` at line 74; the only counted caller
  is a test); 24 `describe*` helpers in `staging/src/k8s.io/kubectl/pkg/describe/describe.go`.
- **Why it's a false positive**: inlining would break the registration.
- **Mechanism**: only `call_expression` nodes produce edges, so a bare identifier in a struct field
  or map value creates none. About 43 of 489 kubernetes findings (agent estimate, not re-derived).

### 2026-10-12 — vscode/stapler-squad corpus backtest — callers resolved by name only

- **Repo**: `microsoft/vscode` `src/vs/platform/actions/common/actions.ts:499`
  (`MenuRegistry.getCommand`, whose sole "caller" is `CommandsRegistry.getCommand(...)`, a different
  class); `src/vs/workbench/api/test/common/extHostExtensionActivator.test.ts:275` (`reject`, "called"
  by the global `Promise.reject`).
- **Why it's a false positive**: the counted call site belongs to a different receiver.
- **Mechanism**: `resolve_one_call_edge` (`arch_model.rs:~605-643`) matches the last name segment
  regardless of receiver type. At least ~30 in vscode (agent heuristic lower bound; the true count
  needs type information).

### 2026-10-12 — vscode corpus backtest — TS class methods are never "exported"

- **Repo**: `microsoft/vscode` `src/vs/sessions/browser/parts/titlebarPart.ts:501`
  (`doCreateAuxiliaryTitlebarPart`, overridden in two other files); RPC endpoints such as
  `src/vs/workbench/api/browser/mainThreadLanguageFeatures.ts:179` whose one caller is in another
  process.
- **Why it's a false positive**: overridden/interface methods and cross-process endpoints can't
  be inlined.
- **Mechanism**: `js_ts_is_exported` (`symbol_extract.rs:85-89`) only checks for an
  `export_statement` parent, so every public method of an exported class is a candidate. The agent
  reports 3562 of 4227 findings are methods, about 2502 apparently public (regex, unverified).
  This is the TypeScript-scale instance of #117. In kubernetes, the agent's regex found 0 of 87
  method findings with a same-named interface method in the package.

### 2026-10-12 — vscode corpus backtest — wrappers of builtin/global calls

- **Repo**: `microsoft/vscode` `src/vs/base/common/strings.ts:1157` (`JSON.parse`),
  `src/vs/editor/contrib/folding/browser/foldingRanges.ts:143` (`Map.set`).
- **Why it's debatable**: the callee isn't a repo function, so there is nothing to inline into.
  Classed needs_discussion, not a clear false positive (about 825 in vscode, agent regex estimate).
- **Mechanism**: `is_pure_delegation_body` accepts any single call, with no check that the callee
  is a repo function.

## Fixed

### 2026-10-12 — kubernetes/stapler-squad/vscode corpus backtest — four caller-counting and shape bugs

- **Symptom**: findings for functions with several callers or call sites, or whose "delegation" is
  a closure. Examples: `setupCRDAndVerifySchema` and `pressKey` (called from `describe`/`it`
  callbacks, one recorded caller); `fileUserDataProvider.stat` (its own sole caller);
  `setField` (13 call sites in one component); `auth` (`return http.HandlerFunc(func(w, r) {...})`).
- **Mechanism**: (1) `walk_calls` dropped calls with no named enclosing function, so module-level
  `describe`/`Describe(func(){...})` callbacks contributed no callers; (2) `caller_counts` counted a
  self-edge as a caller; (3) it counted distinct callers, not call sites, while the message said
  "at that call site"; (4) `is_pure_delegation_body` accepted a call wrapping a closure.
- **Fixed by**: attributing a call inside a module-level anonymous function to a synthetic caller
  per outermost callback; ignoring self-edges; counting distinct call sites (message now says
  "exactly one call site"); rejecting a body whose statement contains an anonymous function.
  Re-run with the shipped binary, same kubernetes clone (A/B against the pre-change binary):
  489 → 263 findings; 13 findings are newly surfaced, and the three checked by hand
  (`deleteResourceQuota`, `increaseClusterSize`, `expectPodSuccess`) each have exactly one call site,
  previously hidden because it sat in a dropped callback. vscode went from 4227 to 2149 across the
  whole change set (not a controlled A/B). stapler-squad was not re-measured: it is a live repo and
  the pre-change binary hung on it.

## Not logged here

Harness/corpus artifacts, not checker bugs: gitignored `stapler-squad/.claude/worktrees/` copies
(796 of 869 findings, since fixed by the walker change in `d3d6ffe`) and 258 vscode findings under
`/fixtures/` (verbatim copies of vscode source used as test input).
