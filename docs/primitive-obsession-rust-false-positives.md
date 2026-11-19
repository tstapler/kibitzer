# primitive-obsession-rust: backtest and known false positives

Advisory, `**/*.rs`. The Go checker flags parameter piles; the Rust analogue flags a named-field
struct whose fields are **all** raw primitives (`String`, integers, floats, `bool`, `char`) with
at least 3 fields, at least 2 distinct types, **two or more `String` fields** (easy to swap at a
call site) and **no `pub` field** (a `pub` field marks a plain data record). Skipped: tuple
structs, `#[cfg(test)]` modules, structs deriving `Serialize`/`Deserialize` or carrying
`#[repr(..)]`.

## Backtest (2026-11-19, release build)

Command: `find <repo> -name '*.rs' | xargs -n1 kibitzer check native primitive-obsession-rust`.
Corpus: servo (1650 files), ripgrep (110), deno (1052), and the crates.io registry checkout in
`~/.cargo/registry/src` (36,378 files, 1,212 crate versions) added after review found the first
sample (5 findings) too small to trust.

| Version | servo | ripgrep | deno | registry (distinct structs) |
|---|---|---|---|---|
| v1: all-primitive, 3+ fields, 2+ types | 8 | 1 | not run | not run |
| v2: v1 + at least one `String` | 5 | 0 | not run | 35 |
| shipped: v2 with 2+ `String`, no `pub` field | 1 | 0 | 3 | 7 (8 findings; one struct in two crate versions) |

Triage (by reading each struct; rubric: a finding is a false positive when the struct is a plain
data/wire/report/build-config record where a newtype would not prevent a realistic mix-up):

- v2 on the registry: about 30 of 35 distinct structs were false positives: `pub` data records
  (GL `ActiveUniform`, comrak AST nodes, wgpu `AllocationReport`), serde-adjacent wire types
  (JWT `Payload`, hf-hub `Metadata`), derive-macro test fixtures, one-`String` records. The v1/v2
  "1 of 5" sample was not representative; that is why the two-`String` / no-`pub` rules exist.
- Shipped, all 11 distinct findings: 9 judged true positives (e.g. proxy `Builder { all, http,
  https, no: String }`, `AppInfo { application_name, application_version, engine_name,
  engine_version }`, `FixableAction { module_name, target_version, is_major }`) and 2 judged false
  positives (`ring` build-script `Target`, deno `FunctionStats`, a profiler record):
  **2/11 = 18%** (bar: 20%).

This is the author's judgment on a small final set, not an independent labelling; the margin to
the bar is two structs. Recall on real code is low by design.

## Known false positives

- Private config/record structs with several `String`s that really are just data
  (`Target`, `FunctionStats`).
- Structs mirroring an external schema by hand without serde derives.
