# Research: no-config-fallback

## Codebase findings

### Config resolution path (verified in source)

`src/config.rs` defines three constants:
- `CONFIG_FILENAME = "inspect.json"`
- `CONFIG_DIR = ".kibitzer"` (current)
- `LEGACY_CONFIG_DIR = ".claude"` (deprecated; still read as fallback)

`find_config(start)` walks upward from `start`, checking `resolve_config_path(dir)` at each
directory, until it finds a config or exhausts the filesystem. Returns `Ok(None)` on
no-config — never errors on absence.

`find_effective_config(start)` wraps `find_config` and, on `Ok(None)`, returns
`(Config { checks: defaults + plugins, architecture: default, … }, start_dir(start))`.
This is already in production; the "fall back to built-in defaults" proposal is implemented.

### Which MCP tools use which resolver

| MCP Tool | Resolver used | Fails on no config? |
|---|---|---|
| `list_checks` | `find_effective_config` | No |
| `architecture_assessment` | `find_effective_config` | No |
| `run_checks` | `find_effective_config` | No |
| `list_architecture_symbols` | `resolve_repo_root` via `find_config` | No (falls back to git root) |
| `get_architecture_node` | same | No |
| call-graph traversal tools | same | No |
| write tools (plan/apply/etc.) | `find_config` explicit | Error if no config (intentional) |

### "Using built-in defaults" notice: absent

`list_checks` output format: `"config root: {root}\nchecks:\n{names}"`. No indication of
whether the config root has a real `inspect.json` or is just the fallback start directory.
`architecture_assessment` has no header line distinguishing defaults-only from merged runs.

The gap: `find_effective_config` does not surface whether it found a local file. The return
type is `Result<(Config, PathBuf)>` with no boolean/enum discriminant for
"loaded from disk" vs "defaults only". Adding the notice requires either:
  - (a) Adding a `bool found_local` to the return type (or a `ConfigSource` enum), or
  - (b) Checking `locate_config_path(start).is_file()` after the call (simpler, one extra
        stat, but slightly racy).

Option (a) is cleaner. `find_effective_config` returns `(Config, PathBuf, bool)` or a
`EffectiveConfig { config, root, is_local: bool }` struct. MCP callers check `is_local`
and prepend the note.

### `--config <path>` override: absent

CLI (`src/main.rs`) parses via `clap`; no `--config` flag on `run`, `check`, or `architecture`
subcommands. MCP tools (`src/mcp.rs`) have request structs with `path`/`scope` but no
`config` field.

A clean implementation would:
- Add `--config <path>` to the `clap` subcommand definitions.
- Thread it through as `Option<PathBuf>` to `find_effective_config`, which reads that file
  directly instead of walking.
- MCP: add optional `config` field to affected `*Request` structs.

The write-tool gate (`mcp.rs:1653`) explicitly uses `find_config` rather than
`find_effective_config`, and checks that the config resides in the same repo — this
security boundary must not be relaxed by a `--config` override.

### Scope/glob mismatch in `kibitzer architecture export`

`arch_export.rs:51–55`: when `--scope` is provided, `model.filtered(scope_glob, ModelLevel::Code)`
is called; on empty result with a non-empty model, it prints:
  `"no packages matched scope \"…\" under …; nothing to export"`

`ArchModel::filtered` matches scope globs against package keys. Package keys for Go are set
by `package_key_for_file` (`arch_model.rs:329–336`): if the file appears in
`import_graph.file_packages`, its import path is used (e.g., `github.com/org/repo/pkg/temporal`);
otherwise the directory relative to repo root is used (e.g., `pkg/temporal`).

**Two distinct mismatch cases**:
1. `go.mod`-indexed repos: keys are full import paths. `--scope 'pkg/temporal/**'` won't match
   `github.com/org/repo/pkg/temporal`.
2. Repos without `go.mod` or non-Go repos: keys are relative dir paths. `--scope 'pkg/temporal/**'`
   would work.

The fix: on empty-match, list the first N (say 5) actual package keys from the model so the
user can see the key format. This is a pure UX improvement — no behavior change.

### Stale doc comments (verified line numbers)

- `src/config.rs:1041`: "Like `find_config`, but never returns "nothing to do": when no
  `.claude/inspect.json` exists above `start`" — should say `.kibitzer/inspect.json`.
- `src/config.rs:569`: "not a real `.claude/inspect.json`" — same stale path.

### `docs/getting-started.md`

Lines 15–38 mention defaults and the deprecated `.claude/inspect.json` fallback. No formal
statement of the full lookup algorithm (walk order, priority rules, what "no config" means
at runtime). This is a docs gap, not a code gap.

## Risk assessment

- **AC1 (defaults notice)**: Low risk. Adding `is_local: bool` to the return of
  `find_effective_config` or returning a `ConfigSource` discriminant touches only
  `config.rs` and the three MCP callers. No behavior change.
- **AC2 (`--config` flag)**: Medium risk. Threading a new `Option<PathBuf>` through CLI
  argument parsing and all callers of `find_effective_config` requires touching `main.rs`,
  `mcp.rs`, and `config.rs`. Must not accidentally enable it for the write-tool gate.
  Needs a backtest run to confirm no behavioral regression.
- **AC3 (scope error message)**: Low risk. Pure output improvement in `arch_export.rs`.
  One additional clone of model package keys on the error path.
- **AC4/AC5 (doc/comment fixes)**: Zero risk. Comment and doc changes only.
