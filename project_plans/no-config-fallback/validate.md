# Validation: no-config-fallback

## Does the plan match the ACs?

| AC | Addressed by | Gap? |
|---|---|---|
| AC1: "using built-in defaults" note in list_checks / architecture_assessment / run_checks | Tasks 1 + 2 | None |
| AC2: `--config <path>` CLI + optional MCP `config` param | Tasks 4 + 8 | None |
| AC3: scope-mismatch error lists sample package keys | Task 3 | None |
| AC4: stale `.claude/inspect.json` doc comments fixed | Task 5 | None |
| AC5: config discovery documented | Task 6 | None |

## Risks not addressed in the plan

1. **`find_effective_config` signature change affects the daemon** (`src/daemon.rs`).
   The daemon may call `find_effective_config` on each request. Must audit all call sites,
   not just the three named in MCP. `grep -n find_effective_config src/` should be run
   before writing the diff.
   → **Resolution**: Make it a mechanical grep-and-update pass before coding starts.

2. **`--config` flag conflicts with per-subcommand argument namespacing in clap.**
   The plan proposes adding `--config` to the top-level `Args` struct *or* to individual
   subcommands. The existing CLI has distinct subcommands (`run`, `check`, `architecture`,
   `architecture export`, etc.) so a top-level global flag may be simpler than adding it
   to each. Verify clap's `global` argument propagation behavior.
   → **Mitigating action**: Implement as a global flag first; if clap doesn't propagate it
     through nested subcommands correctly, fall back to per-subcommand.

3. **MCP `config` field: schema impact.** Adding an optional field to the `*Request`
   structs changes the MCP tool's JSON schema (tool description parameters). This is
   backward-compatible (optional field), but the auto-generated tool description
   (`#[tool(...)]` annotation) should mention the field.
   → **Action**: Update tool description strings when adding the field.

4. **`ConfigSource::LocalFile(path)` stores the config file path, but `find_effective_config`
   currently only returns the _repo root_**, not the config file path.
   The `resolve_config_path(root)` call returns the file path; this needs to be threaded
   back out through `find_config` or derived post-hoc from the returned root.
   → **Action**: After `find_config` returns `Some((config, root))`, call
     `resolve_config_path(&root)` to derive the config file path for `ConfigSource::LocalFile`.
     This is one extra (cheap) path check.

## Checklist before implementation

- [ ] Run `grep -n "find_effective_config" src/*.rs` to enumerate all call sites
- [ ] Verify all call sites can safely ignore `ConfigSource` or handle the discriminant
- [ ] Confirm write-tool gate (`mcp.rs:1653`) does NOT use `find_effective_config`
      (confirmed: it uses `find_config` directly — safe)
- [ ] Verify `clap` global flag propagation behavior for `--config` in nested subcommands
- [ ] Ensure the note text ("no .kibitzer/inspect.json found; using built-in defaults") is
      consistent across all three MCP tools

## Go/No-go

**Go.** All ACs are covered. No AC is blocked on ambiguous requirements. No external
dependencies. Risk items are mechanical and self-resolving within the implementation.
No behavior changes to existing checks — only output additions and argument routing.
