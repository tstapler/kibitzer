# `extract-variable` rule — known false positives

Tracks false-positive firings of the `extract-variable` rule in `syntax-rules*`
(`src/checkers/rules.rs`: `walk_dense_expressions`, `expression_nesting`). Check new
occurrences against this list before re-investigating from scratch.

## Mechanism (confirmed by reading the source)

The rule measures nesting depth of operator/call nodes (`expression_nesting`); the threshold
is `EXTRACT_VARIABLE_NESTING_THRESHOLD = 5`. A flat same-operator `&&`/`||` run and a receiver
call chain count as one level; Rust UpperCamelCase constructor calls are not counted.

## Log

(none open)

## Fixed

### 2026-10-12 — corpus backtest (kubernetes, vscode, cassandra, servo, deno, ripgrep, stapler-squad) — node-count metric

- **Symptom**: ~2,200 findings, mostly flat `&&`/`||` guard chains of atomic clauses (e.g.
  `kubernetes` `staging/src/k8s.io/client-go/transport/round_trippers.go:64`), fluent builder and
  method-chain returns (stapler-squad `session/ent_repository.go`, cassandra `cql3/Duration.java`,
  vscode `build/gulpfile.vscode.ts`), template literals (vscode `audioPreview.ts`), Rust
  `Some(..)`/`Ok(..)`/`.unwrap()` clauses (deno `cli/lsp/diagnostics.rs`, ripgrep
  `crates/printer/standard.rs`), `return` of a multi-statement `async` block (deno
  `cli/module_loader.rs`), and `t.Fatalf` assertion guards in tests.
- **Mechanism**: `expression_density` summed every call and non-arithmetic binary node, so a flat
  list of 4–5 trivial clauses reached the threshold of 10 (tuned on Go only) while the message
  claimed the expression "nests" N operators.
- **Fixed by**: replacing the sum with nesting depth (`expression_nesting`) at threshold 5, and
  not counting Rust UpperCamelCase constructor calls. Re-run over the same corpus with the shipped
  binary: findings fell from ~2,200 to 219 (cassandra 27, kubernetes 22, vscode 165, servo 2,
  deno 1, stapler-squad 2, ripgrep 0), and none of the files named above produce a finding.
  Residual: 96 of the 165 vscode findings are under `/fixtures/` (verbatim source copies used as
  test input, a corpus-scope issue, not a checker bug).
