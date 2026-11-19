# Plan: no-config-fallback

## Overview

Five targeted changes — one API extension, two output improvements, one new CLI flag, and
doc/comment fixes. No new files needed. No schema changes. No backtest required (no
checker logic changes — only output text and argument routing).

---

## Task 1 — Extend `find_effective_config` to surface config source (AC1)

**File**: `src/config.rs`

Add a `ConfigSource` enum:
```rust
pub enum ConfigSource {
    LocalFile(PathBuf),  // a real .kibitzer/inspect.json was found
    Defaults,            // no config file; built-in defaults only
}
```

Change `find_effective_config` return type from `Result<(Config, PathBuf)>` to
`Result<(Config, PathBuf, ConfigSource)>`. On the `None` branch, return
`ConfigSource::Defaults`. On the `Some` branch, return
`ConfigSource::LocalFile(config_path)` where `config_path` is the resolved file.

Update all call sites to destructure the third element. The three MCP callers
(`list_checks`, `architecture_assessment`, `run_checks`) use it to prepend a note.
CLI callers in `main.rs` can ignore it (the `_` pattern).

Estimated changes: `config.rs` (~25 lines), `mcp.rs` (3 call sites, ~10 lines each).

---

## Task 2 — Add "using built-in defaults" note to MCP output (AC1)

**File**: `src/mcp.rs`

In `list_checks`:
```
// Before the check listing:
if matches!(source, ConfigSource::Defaults) {
    prepend "note: no .kibitzer/inspect.json found; using built-in defaults\n"
}
```

In `architecture_assessment`:
```
// First line of `lines`:
if matches!(source, ConfigSource::Defaults) {
    lines.insert(0, "note: no .kibitzer/inspect.json found; using built-in defaults".into());
}
```

In `run_checks`: prepend the note to the rendered report string when `source` is `Defaults`.

---

## Task 3 — Improve `arch export` scope-mismatch error message (AC3)

**File**: `src/arch_export.rs`

After the `model.filtered(scope_glob, ModelLevel::Code)` call returns an empty result with
a non-empty model, change:
```rust
// Before:
format!("no packages matched scope \"{scope_glob}\" under {}; nothing to export", ...)

// After:
let examples: Vec<&str> = model.packages.keys()
    .take(5)
    .map(String::as_str)
    .collect();
let hint = if examples.is_empty() {
    String::new()
} else {
    format!(" (model package keys: {}{})",
        examples.join(", "),
        if model.packages.len() > 5 { ", …" } else { "" })
};
format!("no packages matched scope \"{scope_glob}\" under {}; nothing to export{hint}", ...)
```

---

## Task 4 — Add `--config <path>` CLI flag (AC2)

**File**: `src/main.rs`, `src/config.rs`

Add a `fn load_config_from_path(path: &Path) -> Result<(Config, PathBuf)>` to `config.rs`
that reads the given file directly (no walk, no fallback), validates it, and returns
`(config, path.parent())`.

In `main.rs`, add `--config <path>` to the top-level `clap` `Args` struct (or as a
per-subcommand arg on `run`, `check`, and `architecture` subcommands). When set, pass it
through to callers of `find_effective_config`.

Extend `find_effective_config` signature to accept `Option<&Path>` as a first arg:
`fn find_effective_config(config_override: Option<&Path>, start: &Path)`. When set, load
from that path instead of walking. The write-tool gate in `mcp.rs` uses `find_config`
directly and must not be changed.

**MCP**: Add optional `config: Option<String>` to `ListChecksRequest`,
`ArchitectureAssessmentRequest`, and `RunChecksRequest`. When present, resolve as a path
and pass to the extended `find_effective_config`.

---

## Task 5 — Fix stale doc comments in `src/config.rs` (AC4)

**File**: `src/config.rs`

- Line 569: `"not a real `.claude/inspect.json`"` → `"not a real `.kibitzer/inspect.json`"`
- Line 1041: `"when no `.claude/inspect.json` exists above `start`"` → `".kibitzer/inspect.json"`

---

## Task 6 — Document config lookup order (AC5)

**File**: `docs/getting-started.md`

Add a "Config discovery" section (or expand the existing note) explaining:
1. kibitzer walks upward from the target path.
2. At each directory it checks `.kibitzer/inspect.json` first, then `.claude/inspect.json`
   (deprecated; prints a one-time warning).
3. If neither is found anywhere, the built-in default checks run; output includes a note.
4. A `--config <path>` flag bypasses the walk entirely.

---

## Task 7 — Unit tests for defaults notice (AC1)

**File**: `src/mcp_bridge_tests.rs` (or inline in `mcp.rs`)

Add tests that:
1. Call `list_checks` against a tempdir with no `.kibitzer/inspect.json` and assert the
   output contains "using built-in defaults".
2. Call `architecture_assessment` against a tempdir with no config and assert the note
   appears.
3. Call `list_checks` against a tempdir *with* a valid config and assert the note does
   *not* appear.

---

## Task 8 — Integration test for `--config <path>` (AC2)

**File**: `src/main.rs` (integration test section or a new test file)

Write a test that:
1. Creates a tempdir with Go source but no `.kibitzer/inspect.json`.
2. Writes a config to a *separate* tempdir.
3. Runs `kibitzer run` with `--config <config-path>` pointing at the separate config.
4. Asserts it uses the specified config (e.g., checks listed match config contents, not defaults).

---

## Adversarial review

**Q: Doesn't the write-tool gate (`allow_write_tools`) need guarding against `--config`?**
Yes. The gate in `mcp.rs:1653` calls `find_config` directly and checks the config belongs
to the same repo root. The `--config` override must NOT be threaded through the write-tool
gate — only through the read-only analysis tools. The plan explicitly preserves this.

**Q: Could adding `is_local` to `find_effective_config` break callers?**
Yes — it's a breaking signature change for all direct callers. All callers are in-crate
(no public API surface to external crates), so this is safe. All call sites need mechanical
updating. The PR description should list them.

**Q: The scope-hint on `arch export` could be very long in a large repo.**
Capped at 5 keys + "…". This is acceptable. No truncation risk on the error path.

**Q: Is `ConfigSource` the right abstraction, or just `bool`?**
A `bool` (`is_defaults: bool`) suffices for the current ACs and is simpler. `ConfigSource`
carries the config file path in the `LocalFile` variant, which could be useful for the
"using built-in defaults" note to also say where it looked. Prefer `ConfigSource` for
forward-compatibility.
