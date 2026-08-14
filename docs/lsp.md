# LSP server mode

`kibitzer lsp` speaks the [Language Server Protocol](https://microsoft.github.io/language-server-protocol/)
over stdio (via [`tower-lsp`](https://docs.rs/tower-lsp)), so any LSP-capable editor
can get kibitzer's checks as inline diagnostics instead of running `kibitzer hook`
or `kibitzer run` out of band.

Editor-specific setup (VS Code, Neovim, etc.) isn't covered here — see
[issue #12](https://github.com/tstapler/kibitzer/issues/12).

## What it does

- On `textDocument/didOpen`, `didChange`, and `didSave`, it looks up the nearest
  `.claude/inspect.json` via `config::find_config` and runs every in-scope check
  against the file with `check::run_checks_for_trigger` — the same check-running
  core used by the `hook` and `run` subcommands — under the `lsp` trigger.
- Each `CheckResult` becomes zero or more `Diagnostic`s:
  - `Severity::Blocking` → `DiagnosticSeverity::ERROR`
  - `Severity::Advisory` → `DiagnosticSeverity::WARNING`
  - Output following the `{file}:{line}: message` convention (see
    `docs/output-formats.md` and the `command` field's doc comment in
    `src/config.rs`) becomes one diagnostic per line; output that doesn't
    collapses into a single diagnostic on line 1 so a failure is never
    silently dropped.
- On `textDocument/didClose`, diagnostics for that file are cleared.

## Known limitation: diagnostics reflect disk, not the buffer

Checks read the file off disk — shell commands substitute `{file}`, and native
checkers `std::fs::read_to_string` it — the same as `hook` and `run`. `didChange`
re-runs those same disk-based checks, so diagnostics lag behind unsaved edits
until the next save. Wiring the live editor buffer into checks (e.g. via LSP's
incremental sync) is real follow-up work, not done here.
