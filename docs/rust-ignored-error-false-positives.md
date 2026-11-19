# rust-ignored-error: backtest and known false positives

Advisory, `**/*.rs`. Two shapes in non-test code (`#[cfg(test)]`, `#[test]`, and files under
`tests/`, `benches/`, `examples/` are skipped):

1. `let _ = <call>;` (a possible `Result` dropped silently).
2. Bare `.unwrap()`. `.expect("msg")` is **not** flagged: the message documents the invariant.

Not flagged: `let _ = x` of a plain value; `let _ = .. .send/try_send/recv/try_recv/next/clone/
drop/from_raw(..)` and similar infallible-or-intentional callees; calls passing
`ExceptionHandling::Report`; `.unwrap()` on `lock()/try_lock()/read()/write()/join()/borrow()`, on
`write!`/`writeln!`, and on `Regex::new`/`Url::parse`-style calls whose arguments are all string literals
(`Regex::new("a+")`), not `fs::read_to_string("x")`.

## Backtest (2026-11-19, release build)

Command: `find <repo> -name '*.rs' | xargs -n1 kibitzer check native rust-ignored-error`.

| Version | ripgrep `let _` / `.unwrap()` | servo `let _` / `.unwrap()` |
|---|---|---|
| v0: every `let _ = <call>` and bare `.unwrap()` | 4 / 76 | 710 / 2046 |
| shipped | 4 / 65 | 196 / 2021 |

The exclusions came from triage of v0 samples: 14 of 20 sampled servo `let _` sites were
`sender.send(..)` (ignoring a dropped receiver is the intended idiom), and 6 of 20 sampled
ripgrep `.unwrap()` sites were `writeln!(String, ..).unwrap()` (cannot fail).

Triage of the shipped output (samples, same rubric: a finding is a false positive when the
discarded/unwrapped value cannot fail, or discarding it is the intended idiom):

- `let _ = <call>`: all 4 ripgrep sites plus 20 random servo sites (seed 5) were reviewed. Six
  servo sites were false positives (`Box::from_raw` drop, two `recv()` waits, `iterator.next()`,
  two `Call_(.., ExceptionHandling::Report)`); each class is now excluded. The 18 remaining
  sites were `remove_dir` cleanup, DOM `Collapse`/`Extend`/`SetEnd` results, `write_json_packet`,
  `url.set_username`, and similar: none judged false. The exclusions were derived from this same
  sample, so the shipped rate on unseen code is not measured; **0/18 on the residual sample**.
- `.unwrap()`: 14 ripgrep sites (post-`writeln!` exclusion) and 20 servo sites. 5 were
  invariant unwraps that are plausibly false positives (`pop().unwrap()` right after
  `last().unwrap()` guard, `iter.next().unwrap()` after a non-empty check, `get_mut(&id)
  .unwrap()` on a just-inserted key): **5/34 = 15%**.

Combined: 5/52 = 10% on the triaged samples (bar: 20%). A clippy `unwrap_used`-style rule is
inherently opinionated; the 15% counts only provable invariants, not "would you rather see
`expect`".

## Known limits

- `let _ = <call>` cannot tell `Result` from other return types without type information, so
  discards of non-`Result` values from unlisted callees still fire.
- Unwraps that follow an explicit guard are flagged; use `.expect("why")` or `kibitzer:ignore`.
- With 2021 unwrap findings on servo the check is high-volume on unwrap-heavy code bases.
- The `let _` allow-list is by callee name only (`send`, `join`, `remove`, `insert`, ..), so a
  dropped `Result` from a same-named method is missed, and the `ExceptionHandling::Report`
  exemption is specific to servo/mozjs-style bindings. Both came from backtest triage.
- Because `read`/`write` are exempt receivers (for `RwLock`), `file.read(&mut b).unwrap()` and
  `fs::read(p).unwrap()` are not flagged.
- Test detection is by attribute (`#[cfg(..test..)]`, `#[test]`, `#[rstest]`) and by path (`tests/`,
  `benches/`, `examples/`, `tests.rs`); a checkout under an ancestor directory with one of those
  names is skipped wholesale.
