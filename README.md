<img src="logos/export/logo-192.png" alt="kibitzer logo" width="96" align="right">

# kibitzer

Advisory, diff-aware code and doc checks built for how AI agents actually
edit — locally, in CI, or wired into an agent's hooks. Runs a set of
built-in checks (e.g. Go primitive-obsession, Markdown link integrity)
against only the files that actually changed, with a caching daemon to keep
repeat checks fast. Ships as a CLI, an MCP server, and a Claude Code
`PostToolUse` hook — Claude Code is the first integration, not the ceiling.

## Install

```bash
# Homebrew (once the tap is set up — see dist-workspace.toml)
brew install tstapler/tap/kibitzer

# From source
cargo install --path .
```

## Usage

```bash
kibitzer run [dir] --trigger batch   # batch mode: check every file under dir
kibitzer hook                        # Claude Code PostToolUse hook, reads event off stdin
kibitzer mcp                         # serve as an MCP server over stdio
kibitzer daemon start|stop|status    # background daemon that caches check results
kibitzer check native primitive-obsession <file>   # run a single built-in check directly
kibitzer check native go-blank-imports <file>      # flag unjustified `import _ "pkg"`
kibitzer check native go-ignored-error <file>      # flag `result, _ := f()` discards
kibitzer check native go-error-context <file>      # flag bare error passthroughs (advisory)
kibitzer check native syntax-rules <file>          # run the native syntactic rule catalog (see docs/syntax-rules.md)
kibitzer check native markdown-link-integrity <file> # flag broken markdown reference links/anchors
kibitzer check list                  # list all natively implemented checkers
```

See `docs/checking-invocations.md` for how checks are wired up.

### Migrating off `markdownlint-cli2` / `scripts/doc_report.py`

If your `.claude/inspect.json` currently shells out to `markdownlint-cli2` and/or a
project-local `scripts/doc_report.py` for reference-link/anchor checking (see
`docs/markdown-link-integrity-false-positives.md` for the whole-file false-positive
issues that setup has), replace those `command` entries with a single native
`checker` entry — no npm install, no bespoke Python script:

```jsonc
// Before
{
  "checks": [
    {
      "name": "markdown-link-integrity",
      "command": "markdownlint-cli2 {file}",
      "severity": "blocking",
      "scope": ["**/*.md"],
      "triggers": ["PostToolUse", "batch"]
    },
    {
      "name": "doc-structure-report",
      "command": "python3 scripts/doc_report.py",
      "severity": "blocking",
      "scope": ["**/*.md"],
      "triggers": ["PostToolUse", "batch"]
    }
  ]
}
```

```jsonc
// After
{
  "checks": [
    {
      "name": "markdown-link-integrity",
      "checker": "markdown-link-integrity",
      "severity": "blocking",
      "scope": ["**/*.md"],
      "triggers": ["PostToolUse", "batch"],
      "message": "broken markdown link/anchor"
    }
  ]
}
```

The native checker replaces both — it covers what `doc_report.py` checked (dangling
reference-style uses, unused reference definitions) plus dead heading anchors, and it
runs in-process rather than shelling out. You can drop `markdownlint-cli2` and
`scripts/doc_report.py` entirely once this is wired up. Reference-use-before-definition
across a multi-step edit is handled by a grace period, not by disabling the check —
see the module doc comment on `MarkdownLinkIntegrityChecker` in
`src/markdown_link_integrity.rs`.

### Diff-aware scoping

When kibitzer knows which lines an edit touched (the `PostToolUse` hook,
not batch mode), a check in `.claude/inspect.json` can opt into scoping its
results to just those lines, two ways:

- `{changed_lines}` in `command`: substituted with a comma-separated list
  of 1-indexed, inclusive `start-end` ranges (e.g. `12-15,40-40`), for
  linters that support scoping their own scan.
- Automatic output filtering: if a check's command emits `{file}:{line}:
  message`-style lines (most linters do), kibitzer filters them down to
  the changed ranges and recomputes pass/fail from what survives — no
  command changes needed.

A failure kibitzer determines predates the current edit is reported as
advisory rather than blocking, even for a check configured as blocking.
For a per-file check (one with `{file}` in `command`), this means the same
violation is also present in `git show HEAD:<file>`. For a whole-repo check
(no `{file}` placeholder — it scans the whole tree), kibitzer snapshots the
tree at HEAD (via `git archive`) and reruns the check against that snapshot
to make the same determination.

### Structured output formats

A `command` check's `output_format` field tells kibitzer to parse its
stdout as a known structured shape (currently SARIF) instead of relying
solely on the exit code, so severity and finding counts survive into the
reported output. See `docs/output-formats.md`.

## Development

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

CI (`.github/workflows/ci.yml`) runs the same three commands on every push
and pull request. Releases are tag-triggered and handled by
`.github/workflows/release.yml` (generated by `cargo dist`) plus
`cliff.toml` for changelog generation.

## License

MIT — see `LICENSE`.
