# split-loop-go: known false positives

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
  skipped, but `n += len(t)` alongside a collected list still fires.
- **Shared per-iteration work.** Accumulators fed by one expensive computation
  (a search, a parse); a split reruns it.
- **Single loop variable feeding several accumulators.** Each update mentions only the
  loop variable, so the checker sees "no direct link", yet the loop is one traversal.
- **Fused numeric kernels.** Hash lanes or SIMD-style `v1..v4` unrolled on purpose.

Suppress a single loop with a `kibitzer:ignore split-loop-go -- <why>` comment above it
(`docs/suppressing-checks.md`).

Not fixed in v1: "two `append` accumulators flagged" is a required behavior, so the
parallel-slice pattern can't be excluded without changing it.
