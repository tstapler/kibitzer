# split-loop-go: opt-in check and known false positives

`split-loop-go` is **opt-in**: it is not in `default_checks()` because the corpus backtest
measured ~92% false positives (`project_plans/split-loop-check/backtest-results.md`).
Enable it per repo by adding it to `.kibitzer/inspect.json`'s `checks` array (the general
pattern is in `docs/suppressing-checks.md`):

```json
{
  "checks": [
    {
      "name": "split-loop-go",
      "checker": "split-loop-go",
      "severity": "advisory",
      "scope": ["**/*.go"]
    }
  ]
}
```

`split-loop-go` flags a Go `for` loop whose body advances two or more accumulators that
don't reference each other. Measured on a labeled corpus sample, most findings are false
positives (1 of 12 true; see `project_plans/split-loop-check/backtest-results.md`). Known
patterns:

- **Parallel slices / lockstep builders.** `a = append(a, x)` and `b = append(b, y)` where
  `a[i]` and `b[i]` must correspond (SQL placeholders + args, names + values, inputs +
  mutations + variables). Splitting breaks the pairing — worse when ranging a map, since
  each pass gets a different order.
- **Result + diagnostics.** Values and errors/warnings/diags that come from the same call
  in the loop body.
- **Sum + count.** `total += v` with `n++`. Pure `x++`/`x += 1` tallies are already
  skipped (`tallies_only` in `src/checkers/split_loop.rs`), but `n += len(t)` alongside a
  collected list still fires.
- **Shared per-iteration work.** Accumulators fed by one expensive computation
  (a search, a parse); a split reruns it.
- **Single loop variable feeding several accumulators.** Each update mentions only the
  loop variable, so the checker sees "no direct link", yet the loop is one traversal.
- **Fused numeric kernels.** Hash lanes or SIMD-style `v1..v4` unrolled on purpose.

Suppress a single loop with a `kibitzer:ignore split-loop-go -- <why>` comment above it
(`docs/suppressing-checks.md`).

Partly mitigated: accumulators fed by the same loop element (`xs = append(xs, v)`,
`ys = append(ys, v+1)`) are linked and not flagged
(`does_not_flag_parallel_projections_of_the_same_element`). Parallel slices fed from
*different* inputs still fire, and "two independent `append` accumulators flagged" is a
required behavior, so that residual pattern is not excluded in v1.
