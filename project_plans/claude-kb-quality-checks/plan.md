# Implementation Plan: Claude Code Skill Quality Checks

**Project:** claude-kb-quality-checks  
**Date:** 2026-11-04

## Architecture Overview

Three new native checkers land in `src/checkers/`:

| Checker | File | Scope | Severity |
|---|---|---|---|
| `claude-skill-frontmatter` | `src/checkers/claude_skill_frontmatter.rs` | `**/.claude/**/*.md` (path-filtered) | Blocking (errors) / Advisory (warnings) |
| `claude-skill-xref` | `src/checkers/claude_skill_xref.rs` | same | Advisory |
| `claude-skill-dangling-symlink` | `src/checkers/claude_skill_dangling_symlink.rs` | same | Advisory |

All three are added to `default_checks()` via `core_checks()` in `src/config.rs`. A design document for the LLM-assisted `claude-skill-weak` tier is written in `docs/claude-skill-checks.md` but not implemented.

## Detailed Task Breakdown

### Task 1 — Shared path-filter utility

**File:** `src/checkers/claude_skill_common.rs`

A helper module that:
- Determines whether a given `&Path` is under a `.claude/{skills,agents,commands}` directory (the path-filter used by all three checkers instead of a complex glob)
- Computes the path-implied skill ID from a file path (`<name>` for top-level, `<ns>:<sub>` for namespaced, `None` for namespaced sub-skills that don't assert a name)
- Builds a `SkillRegistry` (a `HashSet<String>` of known skill IDs) by scanning a `.claude/skills/` root; uses `std::sync::LazyLock<Mutex<HashMap<PathBuf, SkillRegistry>>>` keyed on the `.claude/` root dir to amortize scans within a daemon session
- Exposes a `find_claude_root(file: &Path) -> Option<PathBuf>` that walks ancestors

The registry scan must handle symlinks (follow one level, do not recurse into loops), mixed `SKILL.md` / bare `.md` layouts, and the `<ns>/skills/<sub>/SKILL.md` two-level namespace.

### Task 2 — `claude-skill-frontmatter` checker

**File:** `src/checkers/claude_skill_frontmatter.rs`

Implements:
1. Parse frontmatter from `ctx.source` using a 30-line state machine (look for `---\n` fence, collect `key: value` lines, handle block scalar continuations).
2. Determine file type (skill / agent / command) from path segments.
3. Run the following checks in order, each producing a `Finding` at the frontmatter's key line:

| Check id | Condition | Severity |
|---|---|---|
| `name-mismatch` | `name:` != path-implied id (when id is known) | Blocking |
| `missing-description` | frontmatter block present but no `description:` | Advisory |
| `name-too-long` | `name:` > 64 chars | Blocking |
| `name-invalid-chars` | `name:` outside `[a-z0-9-]` | Blocking |
| `name-xml-tag` | paired XML tag in `name:` | Blocking |
| `description-too-long` | `description:` > 1,024 chars (skills only) | Blocking |
| `description-xml-tag` | paired XML tag in `description:` (skills only) | Blocking |
| `broken-frontmatter-key` | key in `BROKEN_KEYS` set (e.g. `paths:`) | Blocking |
| `unknown-frontmatter-key` | key outside documented set for this file type | Advisory |

The documented key sets (SKILL_KEYS, AGENT_KEYS, COMMAND_KEYS) are `const` slices derived from the Claude Code spec snapshot in `lint-claude-kb`.

Registering via `inventory::submit!` and in `src/checkers/mod.rs`.

**Unit tests** (`tests/claude_skill_frontmatter_test.rs`):
- Clean skill: no findings
- `name-mismatch`: frontmatter `name: foo` in a dir named `bar`
- Namespaced skill: `ns:sub` id recognized, name checked as `sub` (or omitted)
- `missing-description`: frontmatter block with only `name:`
- `name-too-long` / `name-invalid-chars` / `name-xml-tag`
- `description-too-long` / `description-xml-tag`
- `broken-frontmatter-key` (`paths:`)
- `unknown-frontmatter-key` (`version:`, `tools:`)
- No frontmatter block: 0 findings (graceful skip)

### Task 3 — `claude-skill-xref` checker

**File:** `src/checkers/claude_skill_xref.rs`

Logic:
1. Use `find_claude_root(file)` from the common module. If no root found, return `Ok(vec![])`.
2. Load `SkillRegistry` from the lazy cache (or scan and populate it).
3. Add configured `external_skills` from `configure()` options into the registry.
4. Scan `ctx.source` for the regex `` `([A-Za-z][\w:/@.-]*)` `` followed by `'?s?[ \t]+skill\b` (ported from `lint-claude-kb`).
5. For each match, if the token is not in the registry, emit a `Finding` at the match's line with message `broken-skill-ref: '<token>' is not a known skill id`.

**Configuration via `configure()`:**
```json
{"external_skills": ["security-review", "claude-in-chrome"]}
```
The `configure()` override returns a new checker instance with the extra set merged in.

**Unit tests** (`tests/claude_skill_xref_test.rs`):
- Clean prose with no backtick-skill references: 0 findings
- Valid reference (token in registry): 0 findings
- Broken reference (token not in registry): 1 finding, correct line
- External_skills allow-list suppresses a finding
- No `.claude/` root found: 0 findings (graceful)
- Namespaced `ns:sub` token recognized

### Task 4 — `claude-skill-dangling-symlink` checker

**File:** `src/checkers/claude_skill_dangling_symlink.rs`

This checker is unusual: a dangling symlink may not appear as a `.md` file (the symlink itself may not have a `.md` extension). The checker fires per `.md` file in the `.claude/` tree, but its real function is to detect dangling symlinks in the same skill's directory.

Alternatively it can be a whole-repo (batch-only) command-style check that calls `find` — but staying native is preferable.

Practical approach:
1. Fire on the per-file trigger, but on the first call within a given `.claude/` root, do a one-shot scan of the entire `.claude/{skills,agents,commands}` tree for dangling symlinks using `std::fs::read_link` + `symlink_metadata` checks.
2. Cache results (per root, per daemon session) to avoid re-scanning on every `.md` touch.
3. Report each dangling symlink as a finding at line 1 of whatever `.md` file triggered the check (best approximation; a whole-repo shape would be cleaner but adds config complexity).

Alternatively (and simpler): implement as a `batch`-triggered command check in `default_checks()` that calls the shared helper inline. Revisit if the per-file approach proves awkward.

**Unit tests:** tested with a temporary directory tree containing a dangling symlink under `.claude/skills/`.

### Task 5 — Wire into `default_checks()`

In `src/config.rs`, add a `claude_kb_checks()` function:

```rust
fn claude_kb_checks() -> Vec<Check> {
    vec![
        Check {
            message: Some("Claude Code skill/agent/command authoring violation".to_string()),
            ..native_check("claude-skill-frontmatter", Severity::Advisory,
                          &["**/.claude/**/*.md"])
        },
        native_check("claude-skill-xref", Severity::Advisory,
                    &["**/.claude/**/*.md"]),
        native_check("claude-skill-dangling-symlink", Severity::Advisory,
                    &["**/.claude/**/*.md"]),
    ]
}
```

Chain `claude_kb_checks()` into `default_checks()`.

Update `tests/config_test.rs` assertions that count `default_checks()` entries.

### Task 6 — Backtest

Run the new checkers against the dotfiles repo (which has the `.claude/skills/` tree) using `kibitzer run` and compare output against `lint-claude-kb`'s output for the same set of checks. Document discrepancies. Add the dotfiles repo to the backtest corpus entries in `docs/backtest-repos.md`.

Also run against existing corpus repos (expected: 0 findings — no `.claude/skills/` present). Record the result in `docs/claude-skill-checks-backtest.md`.

### Task 7 — LLM-assisted check design doc

Write `docs/claude-skill-checks.md` covering:
- Native checks (reference to AC-2/AC-3/AC-4 results)
- `claude-skill-weak` design: triggers (batch-only), LLM call scope (touched file + its immediate cross-refs), flag categories (vague description, monolithic split candidate, duplicate cluster), finding format, opt-in mechanism (explicit `inspect.json` entry, not `default_checks()`)
- Why it's a separate advisory pass rather than a native check

### Task 8 — Update `lint-claude-kb` doc comment

Add a note in the `lint-claude-kb` module-level docstring listing which checks are now covered natively by kibitzer and directing users to run `kibitzer check native claude-skill-frontmatter` / `claude-skill-xref` instead. Checks remaining Python-only: `oversized-no-split`, `nested-references`, `windows-path`, `removed-builtin-command`.

## Implementation Order

1. Task 1 (common module) — dependency for Tasks 2–4
2. Tasks 2 and 3 in parallel (both depend on Task 1; neither depends on the other)
3. Task 4 (dangling symlink) — independent, can overlap with 2/3
4. Task 5 (wire into `default_checks()`) — after 2, 3, 4 land
5. Task 6 (backtest) — after Task 5
6. Task 7 (design doc) — independent, can land any time
7. Task 8 (lint-claude-kb update) — after Task 5

## Risk Register

| Risk | Likelihood | Mitigation |
|---|---|---|
| Cross-file scan on every hook call is slow on large skills trees | Medium | LazyLock cache keyed on root; scan amortized across all `.md` files in a session |
| File glob `**/.claude/**/*.md` matches unexpected paths in nested repos | Low | In-checker path guard: bail unless ancestor segment is exactly `.claude` |
| `default_checks()` test count assertion fails | High (certain) | Update the count assertion in Task 5 |
| Public corpus backtest finds 0 findings (looks broken) | Certain | Document as expected behavior in `docs/claude-skill-checks-backtest.md` |
| Dangling symlink checker is awkward as per-file | Medium | Fall back to batch command check if per-file shape proves unclean |

## Adversarial Review

**Concern 1:** The skill registry scan inside a per-file `check()` could be O(N²) — N files each triggering N scans.  
**Response:** The `LazyLock<Mutex<HashMap<root, registry>>>` cache amortizes this to one scan per root per process lifetime (or per daemon session). The first file to trigger the check scans; all subsequent files in the same root read from the cache.

**Concern 2:** `**/.claude/**/*.md` will also match `**/.claude/CLAUDE.md`, `**/.claude/agents.md`, and other non-skill files.  
**Response:** The in-checker path guard (`is_under_skills`, `is_under_agents`, `is_under_commands`) correctly filters these. The checker bails early with 0 findings for files not under those directories.

**Concern 3:** The LLM-assisted design tier (Task 7) is deferred — is there a risk it never lands?  
**Response:** The design doc (Task 7) captures the architecture so it isn't lost. The native checks deliver immediate value. The LLM tier is a follow-on with its own backlog item when the team is ready to invest in it.

**Concern 4:** The `broken-frontmatter-key` check hard-codes `paths:` as the only entry — this list will rot.  
**Response:** The broken-key list is a `const` slice in the checker module with a dated comment explaining each entry and what to do when the upstream bug is fixed. The comment pattern matches `lint-claude-kb`'s existing approach.
