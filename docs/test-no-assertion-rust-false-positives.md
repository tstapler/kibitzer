# test-no-assertion-rust: backtest and known false positives

Advisory, `**/*.rs`. Reuses the `test-no-assertion` checker with the Rust `TestCodeProfile`
(`src/test_code/rust.rs`): flags a `#[test]` / `#[tokio::test]` fn whose body and same-file
helpers contain no assertion macro or call. `#[ignore]`, `#[should_panic]` and snapshot macros
(`assert_snapshot!`, `expect![`) exempt a test.

## Rust-specific tuning

| Change | Why |
|---|---|
| Helper lookup resolves by last `.`/`::` segment, depth 4 (others: 2) | `SearcherTester::new(..).test()` and `test_sniff -> _sup -> _classification -> _full -> _with_flags` chains |
| Macros named `assert*`, `debug_assert*`, `test_*` count | `synstructure::test_derive!` |
| `assert` anywhere in a macro's token tree counts | `match_domstring_ascii!(s, "abc" => assert!(true), ..)` |

## Backtest (2026-11-19, release build)

Command: `find <repo> -name '*.rs' | xargs -n1 kibitzer check native test-no-assertion-rust`.

| Repo (.rs files) | v0: profile only | + method/path resolution | + depth 4, macros |
|---|---|---|---|
| BurntSushi/ripgrep (110) | 39 | 5 | 5 |
| servo/servo (1650) | 82 | 82 | 15 |

Triage of all 20 final findings: 1 false positive (`test_validate_classifier`: the oracle is
`.validate().expect(..)`), 19 true positives (smoke tests: `println!`/`unwrap()` only,
`test_exit` shutdown tests, compile-time trait checks such as `is_supported::<T>()`).
**FP rate 1/20 = 5%** (bar: 20%). The v0 numbers are not a rate: 34 of ripgrep's 39 were
`glue.rs` builder tests (`.test()` asserts inside the same-file `SearcherTester`).

## Known limits

- `.expect(..)` / `.unwrap()` on a validating call is not counted as an oracle.
- Helpers defined in another file are unresolved, so a test that only calls them is flagged.
