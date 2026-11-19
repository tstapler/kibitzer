# rust-error-context: backtest and known false positives

Advisory, `**/*.rs`. Flags a `?` inside an anyhow-returning function when the operand is a
call to a well-known foreign fallible API (file/IO, parsing, process, network, or a path under `std::`, `serde_json::`, `toml::`,
`serde_yaml::`, `tokio::fs::`, `tokio::process::`) and the chain does not end in `.context(..)`, `.with_context(..)`,
`.wrap_err(..)`, `.wrap_err_with(..)`, `.map_err(..)` or `.ok_or*(..)`.

"Anyhow-returning" means the return type names `anyhow::`/`eyre::`, or is a one-argument
`Result<T>` in a file with `use anyhow::Result` (or an `anyhow::{.., Result}` import). Not
resolved: `type Result<T> = anyhow::Result<T>` aliases defined in another file. Skipped: `?`
inside closures/async blocks (return type unknown), tests (`#[test]`, `#[cfg(test)]`, `tests/`,
`benches/`, `examples/`), calls to same-file functions that already return anyhow, writes to
`stdout`/`stderr`, and bare path calls such as `index::read(..)` that only share a name.

## Backtest (2026-11-19, release build)

Command: `find <repo> -name '*.rs' | xargs -n1 kibitzer check native rust-error-context`.
`denoland/deno` was added to the corpus for this check because servo barely uses anyhow
(0 findings), which would otherwise leave the check unmeasured.

| Version | ripgrep | deno | servo |
|---|---|---|---|
| v1: any `?` without context in an anyhow fn | 96 | 112 | 0 |
| + skip calls to same-file anyhow fns | 42 | 93 | 0 |
| shipped: only known-foreign fallible operands, not project paths/terminal writes | 1 | 12 | 0 |

v1 and v2 were rejected after triage: sampled findings were mostly `?` on project-internal
functions whose errors already carry a message (`args.searcher()?`, `check_indexing_allowed()?`),
about 8 of 28 sampled (29%), and the rest could not be told apart without cross-file resolution.
The shipped allow-list trades recall for precision: internal bare errors are not reported.

Triage of all 13 shipped findings (1 ripgrep, 12 deno): `v.parse()?`, `env::current_exe()?`,
`child.wait().await?`, `tokio::fs::read(..).await?`, `serde_json::from_slice(..)?`,
`std::fs::read_to_string(..)?`, `File::open(..)?`, `port.parse()?`, `set_nonblocking(..)?` and
similar: each loses the path/input in the error. **0/13 false positives (0%)**; bar 20%. Small
sample, and servo contributes no signal.

## Known limits

- Recall is low by design: only foreign operations on the allow-list (`FOREIGN_FALLIBLE`,
  `src/checkers/rust_error_context.rs`) are checked.
- `type Result<T> = ..` aliases from another file are not recognised.
- The allow-list matches method names regardless of receiver type, so a project method named
  `read`/`write`/`open` called as `x.read()` is flagged.
- The plan's configurable anyhow-type list (`inspect.json` options) was not built: detection is
  by return-type text and imports only.
