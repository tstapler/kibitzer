# split-loop-rust: backtest and known false positives

Advisory, `**/*.rs`. Rust port of Go `split-loop-go` (Fowler's Split Loop), precision first: a
`for`/`while`/`loop` body that advances 2+ accumulators with no link between them. Accumulators:
`x op= e` (also `x[k] op= e`) and `x = x <binary-op> e` on a plain local; `push`/`insert` calls and plain indexed stores (`x[k] = e`) are not accumulators. Two accumulators are one group when one reads the other (directly or through a
body local), when they sit in the same or exclusive `if`/`match` arms, or when both are fed by a
per-iteration value (the loop pattern or a body local). Not flagged: bodies with
`break`/`continue`/`return`/`?`, any use of an accumulator outside its own update (a `println!`
mention counts), nested loops/closures, tally-only counters (`n += 1`), constant-step updates
(`off += CONST`), predicate folds (`ok &= a == b`), and state evolution (`w = max(w / 2, 1)`).

## Backtest (2026-11-19, release build)

Command: `find <repo> -name '*.rs' | xargs -n1 kibitzer check native split-loop-rust`.
Corpus: servo (1650 files), ripgrep (110), deno (1052), `tstapler/dunnage-hardening` (412),
`rust-lang/book` (654), `tstapler/disk-cleanup` (408), kibitzer `src/` (242), and the crates.io
registry checkout in `~/.cargo/registry/src` (36,378 files). The registry was added after review
noted the first shipped version found nothing, leaving recall and precision unmeasured.

| Version | servo | ripgrep | deno | registry (distinct sites) |
|---|---|---|---|---|
| v1: direct port of the Go model | 2 | 0 | 2 | 16 (22 findings) |
| v2: + item-fed accumulators one group, `x = f(x)` is state, constructor fills are init | 0 | 0 | 0 | 5 findings |
| shipped: + only `op=`/`x = x <op> e` accumulate (no `push`/index stores), constant-step and `&=`/`\|=` predicate folds excluded, module paths ignored | 0 | 0 | 0 | 2 findings |

Triage by reading the loop:

- v1: 4/4 false positives on servo and deno (parallel arrays from one tuple pattern, texture
  dimension halving, two `Vec::new()` buckets); 14 of 16 registry sites were false positives for
  the same reasons plus `geo`'s `sum_x`/`sum_y` (struct-pattern shorthand fields not seen as loop
  inputs) and `sysinfo`'s `offset += CONST` step.
- v2 left `ryu` (`vm_is_trailing_zeros &= ..`, `vr_is_trailing_zeros &= ..`: predicate folds
  driven by shared loop state), `sysinfo` (`offset += libc::CPU_STATE_MAX as isize`) and `libm`.
- Shipped: the only findings are `libm` `jn.rs:136` and `jnf.rs:81` (`a *= i; b *= temp` computing
  `n!` and `(x/2)^n` in one `while`): the same function in f64 and f32, judged one true positive
  site. **0 false positives out of 2 findings (1 distinct site)** across about 40,000 `.rs`
  files; bar: <= 20%.

With so few findings the rate carries wide uncertainty and recall is low by design: collection
builders (`push`, indexed stores) are never accumulators, because on this corpus every such pair
read as parallel arrays rather than separate jobs.

## Known limits

- Field accumulators (`self.total += x`) are ignored: only plain locals count.
- Any accumulator read in a macro (`println!("{}", sum)`) makes the loop ambiguous and skipped.
