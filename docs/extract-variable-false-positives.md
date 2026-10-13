# `extract-variable` rule — known false positives

Tracks false-positive firings of the `extract-variable` rule in `syntax-rules*`
(`src/checkers/rules.rs`: `walk_dense_expressions`, `expression_density`). Check new
occurrences against this list before re-investigating from scratch.

## Mechanism (confirmed by reading the source)

`expression_density` counts every call node and every non-arithmetic binary expression
in an `if`/`return`/ternary expression; the threshold is
`EXTRACT_VARIABLE_DENSITY_THRESHOLD = 10`, tuned on Go (kubernetes) only. The finding says
"nests N operators/calls", but the metric is total node count, not nesting depth.

## Log

### 2026-10-12 — corpus backtest (kubernetes, vscode, cassandra, servo, deno, ripgrep, stapler-squad) — flat chains of atomic clauses

- **Repo**: `kubernetes/kubernetes`, file
  [`staging/src/k8s.io/client-go/transport/round_trippers.go:64`](https://github.com/kubernetes/kubernetes/blob/cc213a13bd4e8f0f253087187ffd3e1264d33f3b/staging/src/k8s.io/client-go/transport/round_trippers.go#L64);
  same shape in `apache/cassandra` `equals()`/`hashCode()` and `microsoft/vscode`
  `src/vs/base/parts/ipc/common/webSocketFraming.ts:246`.
- **What the code looks like**:
  ```go
  if len(config.Impersonate.UserName) > 0 ||
      len(config.Impersonate.UID) > 0 ||
      len(config.Impersonate.Groups) > 0 ||
      len(config.Impersonate.Extra) > 0 {
  ```
- **Why it's a false positive**: four atomic, same-operator clauses with no nesting; there is
  nothing to extract into a variable (the better refactor is a helper or table).
- **Mechanism**: each clause costs 2–3 nodes, so 4–5 clauses reach 10. 140 of 189
  first-party kubernetes findings score 10–12 (agent count, from the `nests N` values).
  Share of flat chains is a regex estimate by the triage agents (about 100/189 kubernetes,
  about 600/973 vscode) and is not verified.

### 2026-10-12 — corpus backtest — fluent/builder call chains and template literals

- **Repo**: `stapler-squad` `session/ent_repository.go:1657` (`return r.client.ApprovalRule.Create().SetRuleID(...)...`,
  score 28); `microsoft/vscode` `build/gulpfile.vscode.ts:648`,
  `extensions/media-preview/src/audioPreview.ts:66` (HTML template literal with `${fn()}` interpolations);
  `apache/cassandra` `cql3/Duration.java:205` (`new Builder(..).addYears(..).addMonths(..)...build()`).
- **Why it's a false positive**: a linear pipeline or a template has no condition an extracted
  variable would explain.
- **Mechanism**: every chained call adds 1. Go's density boundaries are only
  `func_literal`/`composite_literal` (`rules.rs:914`); TS's don't include `template_string`
  (`rules.rs:~957`); Java's don't treat a builder chain specially (`rules.rs:~1064`).
  Cassandra agent counts 44 of 228 builder-chain returns and 38 more single-call-with-many-args
  returns (regex).

### 2026-10-12 — deno/servo corpus backtest — `return` of a multi-statement `async` block or `match`

- **Repo**: `denoland/deno`, `cli/module_loader.rs:1474`:
  `return deno_core::ModuleLoadResponse::Async(async move { let hook_result = match receiver.await {...}; ... })`
  — 61 lines, 8 statements, score 29. Also `cli/rt/run.rs:487/867/955`, `cli/tools/task.rs:585`,
  and servo `components/script/dom/element/create.rs:217`.
- **Why it's a false positive**: the block is its own scope with its own statements; the
  density counts everything inside it.
- **Mechanism**: `rust_lang_config()` `density_boundary_kinds` is only `closure_expression`,
  `struct_expression`, `array_expression` (`rules.rs:~1169`); `async_block` and
  `match_expression` aren't boundaries. Agent probe: a 4-statement `return wrap(async move {..})`
  flags at 10. 6 findings share this (agent script `ret.py`).

### 2026-10-12 — deno corpus backtest — enum-variant constructors and trivial accessors counted as calls

- **Repo**: `denoland/deno`, `cli/lsp/diagnostics.rs:145`: three
  `diagnostic.code == Some(NumberOrString::String("...".to_string()))` clauses (score 14);
  `BurntSushi/ripgrep` `crates/printer/src/standard.rs:728` and `json.rs:673` (both of ripgrep's
  findings): `!matches.is_empty() && matches.last().unwrap().is_empty() && matches.last().unwrap().start() >= range.end`.
- **Why it's a false positive**: three short comparisons; `Some(..)`/`Ok(..)`/`.unwrap()`/`.clone()`
  are not logic.
- **Mechanism**: `Some(x)`/`Ok(x)`/`Enum::V(x)` parse as `call_expression` and are in `call_kinds`
  (`rules.rs:~1167`); every method call weighs 1. Agent probe: 3 `Some(..)` clauses score 14, 2
  clauses don't fire.

### 2026-10-12 — kubernetes/stapler-squad corpus backtest — `t.Fatalf` assertion guards in tests

- **Repo**: `kubernetes/kubernetes` `pkg/kubelet/kubelet_pods_test.go:8135`
  (`if !ok || !s.IsTerminationRequested() || ... { t.Fatalf(...) }`); 85 of 189 first-party
  kubernetes findings are in `_test.go` files (agent count by filename).
- **Why it's a false positive**: the compare-and-fail guard is idiomatic Go; splitting it makes
  the test worse.
- **Mechanism**: `walk_dense_expressions` has no test-file exemption. Classed
  needs_discussion rather than a pure false positive.

## Not logged here

Harness/corpus artifacts, not checker bugs: the vendored 146,620-line TypeScript copy
`extensions/vscode-colorize-perf-tests/test/colorize-fixtures/test-checker.ts` (261 of 973
vscode findings), `kubernetes/vendor/` (182 of 372), and gitignored
`stapler-squad/.claude/worktrees/` copies (473 of 520). Counts re-run with
`grep` over the findings files on 2026-10-12.
