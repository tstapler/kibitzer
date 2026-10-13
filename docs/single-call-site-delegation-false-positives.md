# `single-call-site-delegation` check — known false positives

Tracks false-positive firings of `single-call-site-delegation`
(`src/single_call_site_delegation.rs`, with the shape test and call extraction in
`src/symbol_extract.rs` and call resolution in `src/arch_model.rs`). Check new occurrences
against this list before re-investigating from scratch. Interface-implementation misfires are
already tracked as #117 and are not repeated here, except where TypeScript makes them much larger.

## Log

### 2026-10-12 — kubernetes/stapler-squad/vscode corpus backtest — calls inside anonymous callbacks are dropped

- **Repo**: `kubernetes/kubernetes` `test/e2e/apimachinery/crd_publish_openapi.go:528`
  (`setupCRDAndVerifySchema`, called at lines 200, 242 inside `framework.ConformanceIt(func…)` closures);
  stapler-squad `web-app/src/lib/window/__tests__/useWindowShortcuts.test.ts:49` (`pressKey`, 12 call
  sites inside `it(...)` callbacks, one recorded caller).
- **Why it's a false positive**: the function has many real callers; the finding says one.
- **Mechanism** (VERIFIED, `src/symbol_extract.rs` `walk_calls`): a call site is recorded only when
  `current_caller` is `Some`, so calls with no named enclosing function (module-level
  `describe(...)`, `var _ = Describe(func(){...})`) are discarded. kubernetes: 14 of 489 by agent
  heuristic (likely an undercount).

### 2026-10-12 — vscode/stapler-squad corpus backtest — a wrapper counted as its own sole caller

- **Repo**: `microsoft/vscode` `src/vs/platform/userData/common/fileUserDataProvider.ts:96`
  (`stat(resource) { return this.fileSystemProvider.stat(...); }`); stapler-squad
  `web-app/src/lib/omnibar/detector.ts:355` (`find(name) { return this.detectors.find(...); }`).
- **Why it's a false positive**: the inner call is a method of another object; the wrapper is not
  its own caller.
- **Mechanism**: `caller_counts` (`src/single_call_site_delegation.rs:43-55`, VERIFIED) inserts
  `edge.from` with no `from != to` filter, and `resolve_one_call_edge` (`arch_model.rs:~605-643`)
  matches on the last name segment regardless of receiver type. vscode agent joined the exported
  call edges to the findings: 1032 of 4227 have the self edge as the only caller. Related
  wrong-receiver resolutions (`Promise.reject(...)` resolving to a repo method) are a lower
  bound of about 30 by agent heuristic.

### 2026-10-12 — kubernetes corpus backtest — function used as a value (callback registration)

- **Repo**: `kubernetes/kubernetes` `staging/src/k8s.io/pod-security-admission/policy/check_sysctls.go:122`
  (`sysctlsV1Dot27`, registered as `CheckPod: sysctlsV1Dot27,` at line 74; the only counted caller
  is a test); 24 `describe*` helpers in `staging/src/k8s.io/kubectl/pkg/describe/describe.go`.
- **Why it's a false positive**: inlining would break the registration.
- **Mechanism**: only `call_expression` nodes produce edges, so a bare identifier in a struct field
  or map value creates none. About 43 of 489 kubernetes findings (agent estimate).

### 2026-10-12 — kubernetes/stapler-squad/vscode corpus backtest — "delegation" is a closure wrapper or a type conversion

- **Repo**: `kubernetes/kubernetes` `test/images/agnhost/fakeregistryserver/registryserver.go:79`
  (`auth`: whole body is `return http.HandlerFunc(func(w, r) {...~15 lines...})`); stapler-squad
  `server/user_themes.go:125` (`registerUserThemesHandler`: an 18-line `s.mux.HandleFunc(..., func(){...})`).
- **Why it's a false positive**: registration with real logic in the closure, or a type conversion
  (`T(x)` parses as a call), is not a thin forwarding call.
- **Mechanism** (VERIFIED, `symbol_extract.rs::is_pure_delegation_body`): it accepts any single
  `call_expression` statement or `return <call>`, ignoring argument size and whether the callee is
  a repo function. kubernetes: 46 findings with a 20+-line body containing `func(`, and 32
  conversions (24 `http.HandlerFunc`); vscode: 591 callback bodies of 5+ lines. These are agent
  script counts. Wrappers of builtins (`Map.set`, `JSON.parse`) are a related, debatable group.

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

## Not logged here

Harness/corpus artifacts, not checker bugs: gitignored
`stapler-squad/.claude/worktrees/` copies (796 of 869 findings) and 258 vscode findings under
`/fixtures/` (verbatim copies of vscode source used as test input). Re-run counts, 2026-10-12.
