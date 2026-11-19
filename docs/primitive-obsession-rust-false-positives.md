# primitive-obsession-rust: backtest and known false positives

Advisory, `**/*.rs`. The Go checker flags parameter piles; the Rust analogue flags a named-field
struct whose fields are **all** raw primitives (`String`, integers, floats, `bool`, `char`): at
least 3 fields, at least 2 distinct types, at least one `String`. Skipped: tuple structs,
`#[cfg(test)]` modules, structs deriving `Serialize`/`Deserialize` or carrying `#[repr(..)]`.

## Backtest (2026-11-19, release build)

Command: `find <repo> -name '*.rs' | xargs -n1 kibitzer check native primitive-obsession-rust`.

| Variant | servo/servo (1650 files) | ripgrep (110 files) |
|---|---|---|
| v1: all-primitive, 3+ fields, 2+ distinct types | 8 | 1 |
| shipped: v1 + at least one `String` | 5 | 0 |

v1 total across both repos: 9.
Triage of the 9 v1 findings: the 4 numeric-only records were all false positives (`LineStep`
iterator state, `VariationAxisInformation`, `Layout1DResult`, `OfflineAudioContextOptions`); the `String`-bearing ones were 4 true positives (`PendingScroll`,
`PendingPointerMove`, `Model`, `AppInfo`) and 1 false positive (`SourcePosition`).
Shipped FP rate: **1/5 = 20%** (bar: <= 20%). The sample is small; treat the rate as an upper
bound with wide error, not a measurement.

## Known false positives

- Position/span records (`SourcePosition { source_file, line, column }`) read as primitive bundles
  but are idiomatic.
- Structs mirroring an external schema by hand (without serde derives).
