# sentinel-return-rust: backtest and known false positives

Advisory, `**/*.rs`. Flags an explicit `return None` / `return Err(..)` that hand-propagates a
failure inside a function that also uses `?`. A return counts only when it is the whole arm
value or the only statement of its block, and it sits in the failure branch of a fallible value:

- a `None` / `Err(..)` match arm,
- a `let Some(..) = .. else { .. }` / `let Ok(..) = .. else { .. }` block,
- an `if let Err(..) = ..` (or `.is_none()` / `.is_err()`) branch with no `&&`/`||`.

Skipped: functions not returning `Option<_>`/`Result<_, _>`, constructor-like names (`new`,
`try_new`, `default`, `from_*`, `try_from*`), test code, and closures (their own exit scope).
The fix is `?` with `.ok_or(..)` / `.map_err(..)`.

## Backtest (2026-11-19, release build)

Command: `find <repo> -name '*.rs' | xargs -n1 kibitzer check native sentinel-return-rust`.

| Version | ripgrep (110 files) | servo/servo (1650 files) |
|---|---|---|
| v1: any explicit `return None`/`return Err(..)` in a fn that uses `?` (the criterion's literal wording) | 44 | 831 |
| v2: failure-branch returns only | 11 | 159 |
| shipped: v2 + sole-statement, `Some(`/`Ok(` let-else only, no compound conditions | 4 | 53 |

v1 was rejected: its ripgrep findings were mostly guard clauses that originate a new error or
bail out of a `let Token::Literal(c) = *t else { return None }` (no `?` form exists). Of v2's 11
ripgrep findings, 6 were false positives (log-then-return, non-`Option` let-else patterns,
compound `if let .. && ..` conditions); the shipped rules remove exactly those shapes.

Triage of the shipped output: all 4 ripgrep findings and a sample of 18 of the 53 servo findings
(every third) were each replaceable by `?` plus `.ok_or(..)` or `.map_err(..)`: **0/22 false
positives (0%)**, bar 20%. "True positive" here means "expressible with `?`", not "worth
changing": many are `let Some(x) = opt else { return Err(MyErr) }` where `ok_or` is a style
preference, so expect users to find the check low-value on code that is already consistent.

## Known limits

- The `?`-presence gate is per function, so a function whose only `?` is far from the flagged
  return still fires.
- `.ok_or(..)` conversions of `Option` to a new error are flagged; if you consider the explicit
  `else { return Err(..) }` clearer, suppress with `kibitzer:ignore sentinel-return-rust`.
