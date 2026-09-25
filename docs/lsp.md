# LSP server mode

`kibitzer lsp` speaks the [Language Server Protocol](https://microsoft.github.io/language-server-protocol/)
over stdio (via [`tower-lsp`](https://docs.rs/tower-lsp)), so any LSP-capable editor
can get kibitzer's checks as inline diagnostics instead of running `kibitzer hook`
or `kibitzer run` out of band.

## Editor setup

`kibitzer lsp` isn't tied to one editor — it's a generic stdio LSP server, so any
client that can spawn an arbitrary command works. There's no dedicated kibitzer
extension for either editor below; both use their existing generic/custom LSP
client support.

### Neovim

With [`nvim-lspconfig`](https://github.com/neovim/nvim-lspconfig), register kibitzer
as a custom server (it isn't one of lspconfig's built-in server definitions) via
`vim.lsp.config` + `vim.lsp.enable` (Neovim 0.11+):

```lua
vim.lsp.config.kibitzer = {
  cmd = { "kibitzer", "lsp" },
  filetypes = { "go", "rust", "python", "lua", "markdown" }, -- match your repo's checks
  root_markers = { ".kibitzer/inspect.json", ".git" },
}
vim.lsp.enable("kibitzer")
```

On older Neovim, use `vim.lsp.start` from an `FileType` autocmd instead:

```lua
vim.api.nvim_create_autocmd("FileType", {
  pattern = { "go", "rust", "python", "lua", "markdown" },
  callback = function()
    vim.lsp.start({
      name = "kibitzer",
      cmd = { "kibitzer", "lsp" },
      root_dir = vim.fs.dirname(vim.fs.find({ ".kibitzer/inspect.json", ".git" }, { upward = true })[1]),
    })
  end,
})
```

kibitzer publishes diagnostics and answers `textDocument/documentSymbol` /
`workspace/symbol` like any other server — no special lspconfig glue needed
beyond `cmd` and `filetypes`.

### VS Code

VS Code has no built-in generic LSP client, so pointing it at an arbitrary stdio
server requires a small extension shim. Use a generic-LSP-client extension —
e.g. [Generic LSP Client](https://marketplace.visualstudio.com/items?itemName=llllvvuu.llllvvuu-glspc)
(`llllvvuu.llllvvuu-glspc`) — configured with:

```json
{
  "glspc.serverCommand": "kibitzer",
  "glspc.serverCommandArguments": ["lsp"],
  "glspc.languageId": "go"
}
```

That extension registers one language server for one `languageId` per
installed copy — see its README for building extra copies if you need
kibitzer attached to multiple filetypes at once. kibitzer doesn't ship its
own VS Code extension initially; this is the supported path until (if ever)
a dedicated one exists.

### Scoping to relevant filetypes

Whichever client you use, the `filetypes`/`languageId` list is a client-side
optimization (only start/attach the server for files you care about) — it's
independent of `.kibitzer/inspect.json`'s own `scope` globs (see
[`docs/suppressing-checks.md`](suppressing-checks.md)), which control which
*checks* run against a given file once kibitzer is already attached. Mismatch
between the two isn't harmful, just redundant: e.g. attaching kibitzer to every
filetype when `inspect.json` scopes a check to `**/*.go` still works, since
scoped-out files simply produce no diagnostics.

## What it does

- On `textDocument/didOpen`, `didChange`, and `didSave`, it looks up the nearest
  `.kibitzer/inspect.json` via `config::find_config` and runs every in-scope check
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
