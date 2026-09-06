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

## Known limitation: cold-cache latency and pruning in workspace symbol search

`kibitzer lsp` also implements `textDocument/documentSymbol` ("Outline"/"Go to
Symbol in File") and `workspace/symbol` ("Go to Symbol in Workspace"), backed by
the same architecture model the `architecture export`/`architecture diagram`
commands and the MCP `list_architecture_symbols`/`get_architecture_node` tools
use. Three things about `workspace/symbol` specifically are worth knowing:

- **Cold-cache/first-call latency.** The whole-repo index `workspace/symbol`
  searches is built in the background starting when the client's `initialized`
  notification arrives, not on the first `workspace/symbol` request. If that
  first request in a session arrives before the background build finishes, it
  returns a single synthetic result — `"⏳ kibitzer: still indexing this
  workspace — try again shortly"` — instead of real matches. That's not a hang,
  an error, or a sign the picker is broken; just retry the search a moment
  later. Every subsequent search in the same `kibitzer lsp` session is fast,
  since the index is built once and reused.
- **`textDocument/documentSymbol` vs. `workspace/symbol` pruning asymmetry.**
  Symbol search in your editor's Outline (per-file) includes private/unexported
  symbols, since you're already looking at that file; workspace-wide symbol
  search (Go to Symbol in Workspace) defaults to the public surface only.
- **No `possibly_pruned`/`exists_but_pruned` equivalent for
  `workspace/symbol`.** Unlike the MCP query tools, `workspace/symbol` has no
  field distinguishing "no results" from "results exist but were pruned":
  it defaults to public symbols only and returns no results for a
  private-only match, indistinguishable from a true non-match. Use the MCP
  `list_architecture_symbols` tool with `include_private: true`, or `kibitzer
  architecture export --include-private`, for a definitive check. This is a
  documented limitation, not a bug to fix: verified against this repo's
  actual pinned dependency versions (`Cargo.toml`'s `tower-lsp = "0.20.0"`,
  `lsp-types 0.94.1` per `Cargo.lock`), `lsp-types` 0.94.1 does define a
  3.17-spec `WorkspaceSymbol.data: Option<LSPAny>` extension field, but
  `tower-lsp` 0.20.0's `LanguageServer::symbol` trait method signature is
  hardcoded to the legacy `Result<Option<Vec<SymbolInformation>>>` shape
  (`tower-lsp-0.20.0/src/lib.rs:1155-1162`), and `SymbolInformation` has no
  `data`/vendor-extension field — so there is no clean protocol-level signal
  available at this dependency version without a `tower-lsp` upgrade, which is
  out of scope for this feature.
