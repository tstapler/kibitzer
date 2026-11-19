# file-complexity-rust: backtest and known false positives

Advisory, `**/*.rs`. Same semantics as Go `file-complexity`: fires (one finding per function,
sharing one aggregate message) once a file has at least 3 `function_item`s whose McCabe
complexity exceeds 10. Counted: `if`/`if let`, `for`, `while`, `loop`, each `match` arm except
the last (catch-all), `&&`, `||`. Closures sum into the enclosing function. `#[cfg(test)]`
modules and `#[test]` functions are skipped, as are generated files. A single over-threshold
function does not fire, matching the Go check's file-level intent.

## Backtest (2026-11-19, release build)

Command: `find <repo> -name '*.rs' | xargs -n1 kibitzer check native file-complexity-rust`.

| Repo | Before (no Rust coverage) | After: complex functions / files flagged |
|---|---|---|
| BurntSushi/ripgrep (110 files) | 0 | 19 functions / 6 files |
| servo/servo (1650 files) | 0 | 292 functions / 61 files |

Triage: complexity is a deterministic count, so a false positive here means the function is not
actually over the threshold. All 19 ripgrep functions and a random 20 of the then-299 servo functions (a nested `fn` was later no longer double-counted in its parent, leaving 292)
(seed 1) were recounted with an independent regex count of branch tokens over the function body:
all 39 were above 10 (0 false positives, **0/39 = 0%**; bar 20%). Hand check of
`crates/core/main.rs:113` (reported 11): 11 branch points counted by eye.

## Known limits

- `?` is not counted as a branch (it is an early return, but counting it would flag most
  error-propagating code).
- Large `match` dispatch functions (servo `script` DOM bindings) are flagged: they are complex by
  the metric even when each arm is trivial.
- Macro-generated bodies are not visible to tree-sitter and are not counted.
