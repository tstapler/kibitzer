# Requirements: no-config-fallback

## Item
Run without `.kibitzer/inspect.json`: fall back to built-in defaults

## Problem

In kibitzer ≤ 0.1.29, the MCP tools (`architecture_assessment`, `list_checks`) failed with
"no .claude/inspect.json found above this path" when invoked against a repo that had no
`.kibitzer/inspect.json`. This made one-off analysis of foreign repos require writing a
temporary config file into the target repo's working tree — a write that could be committed
by accident.

**Current state (HEAD)**: `find_effective_config` already resolves this for the MCP tools
that run checks (`list_checks`, `architecture_assessment`, `run_checks`). The architecture
query tools (`list_architecture_symbols`, `get_architecture_node`, call-graph traversal) use
`resolve_repo_root` which already falls back to the git root. No MCP tool fails hard on a
missing config file in the current codebase.

**Remaining gaps** (what this item is actually about now):

1. **No "using built-in defaults" notice in output.** When no config file exists, tools emit
   output identical to a config-tuned run. Callers can't tell whether a report reflects
   per-repo tuning or pylint-style defaults, and may mistake the defaults-only output for a
   fully configured run.

2. **No `--config <path>` override.** There is no way to point kibitzer at a config file
   that lives outside the repo being analyzed — useful for maintaining a shared config for
   many analyzed repos or running against repos you cannot modify.

3. **Scope/import-path mismatch in `kibitzer architecture export`.** When Go packages are
   keyed by their full module import path (e.g., `github.com/org/repo/pkg/temporal`), passing
   `--scope 'pkg/temporal/**'` matches nothing and prints "no packages matched scope …;
   nothing to export". The error message does not list the actual package keys it tried to
   match against, so it's self-diagnosing only if you already know the internal key format.

4. **Stale doc comment in `config.rs`.** The `find_effective_config` doc comment still says
   `.claude/inspect.json` instead of `.kibitzer/inspect.json` (see line 1041). The
   `find_repo_root` doc comment at line 569 has the same stale path.

5. **Docs don't state the config lookup order.** `docs/getting-started.md` mentions the
   deprecated `.claude/inspect.json` fallback but doesn't formally document the discovery
   walk order: `.kibitzer/inspect.json` takes priority at each directory level, then
   `.claude/inspect.json` as a deprecated fallback, walking up to the filesystem root.

## Acceptance Criteria

1. `list_checks`, `architecture_assessment`, and `run_checks` output includes a one-line
   note ("note: no .kibitzer/inspect.json found; using built-in defaults") when
   `find_effective_config` returns without a local config file, so callers can distinguish
   a defaults-only run from a config-merged run.

2. `kibitzer run` / `kibitzer architecture ...` CLI commands accept a `--config <path>`
   flag that loads config from an explicit file path instead of walking upward. MCP tools
   optionally accept a `config` parameter serving the same purpose.

3. `kibitzer architecture export --scope` failure message lists the first N actual package
   keys the model contains, so a scope-mismatch is self-diagnosing ("no packages matched
   scope \"pkg/temporal/**\"; model contains: github.com/org/repo/pkg/temporal, …").

4. The `find_effective_config` and `find_repo_root` doc comments in `src/config.rs` are
   corrected to reference `.kibitzer/inspect.json` instead of `.claude/inspect.json`.

5. `docs/getting-started.md` (or a new `docs/config-lookup.md` referenced from it)
   formally documents the full config discovery walk: `.kibitzer/inspect.json` →
   `.claude/inspect.json` (deprecated) at each level, walking to filesystem root; and that
   no config file is never an error, only a defaults-only run.

## Out of scope

- The architectural model itself (`ArchitectureConfig.components`, dependency rules) has no
  sensible defaults and continues to require a real config file. This item does not add
  defaults for those fields.
- `allow_write_tools` continues to require a real `.kibitzer/inspect.json` in the target
  repo (security boundary). The write-tool gate explicitly uses `find_config` not
  `find_effective_config` and must stay that way.
