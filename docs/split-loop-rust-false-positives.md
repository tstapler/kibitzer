# split-loop-rust: backtest and known false positives

Advisory, `**/*.rs`. Rust port of Go `split-loop-go` (Fowler's Split Loop), precision first: a
`for`/`while`/`loop` body that advances 2+ accumulators with no link between them. Accumulators:
`x op= e`, `x = x <binary-op> e`, `x[k] = e`, and `x.push/push_str/push_back/insert/extend/..(e)`
on a plain local. Two accumulators are one group when one reads the other (directly or through a
body local), when they sit in the same or exclusive `if`/`match` arms, or when both are fed by a
per-iteration value (the loop pattern or a body local). Not flagged: bodies with
`break`/`continue`/`return`/`?`, any use of an accumulator outside its own update (a `println!`
mention counts), nested loops/closures, tally-only counters (`n += 1`), initialisation
(`v.push(Vec::new())`, `push(0)`), and state evolution (`w = max(w / 2, 1)`).

## Backtest (2026-11-19, release build)

Command: `find <repo> -name '*.rs' | xargs -n1 kibitzer check native split-loop-rust`.

| Version | servo (1650 files) | ripgrep (110) | deno (1052) | tstapler/dunnage-hardening (412) | rust-lang/book (654) | tstapler/disk-cleanup (408) | kibitzer `src/` (242) |
|---|---|---|---|---|---|---|---|
| v1: direct port of the Go model | 2 | 0 | 2 | not run | not run | not run | not run |
| shipped | 0 | 0 | 0 | 0 | 0 | 0 | 0 |

All four v1 findings were false positives (**4/4**): two pairs of parallel arrays filled from one
tuple pattern (`keys.push(key); values.push(value)`, zipped afterwards; servo
`stylepropertymapreadonly.rs:52`, deno `rt_desktop/lib.rs:953`), one loop halving three texture
dimensions with `w = max(1, w / 2)` (servo `webgltexture.rs:614`), and one loop initialising two
`Vec::new()` buckets (deno `coverage/merge.rs:253`). The shipped rules remove exactly those
shapes: per-iteration-fed accumulators form one group, `x = f(..)` calls are state not
accumulation, and constructor/literal fills are initialisation.

**The shipped check reports nothing on about 4,500 `.rs` files**, so its false-positive rate is
0 of 0 findings (bar: <= 20%), and its recall on real code is **unmeasured**: the 14 unit tests
(positive: independent `+=`, independent `push`, `while` loop; negative: each suppression above)
are the only evidence it fires. Treat it as a conservative guard until a corpus with a true
positive turns up.

## Known limits

- Field accumulators (`self.total += x`) are ignored: only plain locals and `x[k]` count.
- Any accumulator read in a macro (`println!("{}", sum)`) makes the loop ambiguous and skipped.
