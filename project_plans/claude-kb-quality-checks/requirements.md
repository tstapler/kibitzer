# Requirements: Claude Code Skill/Agent/Command Quality Checks

**Project:** claude-kb-quality-checks  
**Source item:** backlog/52bbd124-20b5-47d4-a111-6b6e18fd6f1a  
**Date:** 2026-11-04

## Background

`tstapler/dotfiles` has 860+ Claude Code skill/agent/command markdown files
under `.claude/{skills,agents,commands}`. A hand-rolled stdlib-only Python
script (`stapler-scripts/lint-claude-kb`) already performs deterministic
frontmatter and cross-reference checks and found real bugs on its first run
(20 name-mismatch findings, ~46 dangling cross-references). That script is the
reference implementation; this project ports the same logic into kibitzer's
native per-file checker model and extends it with an LLM-assisted advisory
tier.

## Goal

Extend kibitzer's "advisory, diff-aware quality checks" model to cover Claude
Code's own skill/agent/command authoring format, in two tiers:

1. **Deterministic (native)** — same pattern as `markdown-link-integrity`, no
   LLM call, fires per-file, included in `default_checks()` once stable.
2. **LLM-assisted (advisory)** — judgment calls a pure regex/AST pass cannot
   make (vague descriptions, monolithic files, duplicate-skill clusters).

## Acceptance Criteria

### Tier 1 — Deterministic Checker

**AC-1.** A new native checker `claude-skill-frontmatter` is registered via
`inventory::submit!` and wired into `default_checks()`. It fires on
`.claude/skills/**/*.md`, `.claude/agents/**/*.md`, and
`.claude/commands/**/*.md` globs (or a combined `**/.claude/**/*.md` if that
is cleaner).

**AC-2.** The checker detects the following frontmatter violations, matching
the semantics in `lint-claude-kb`:
- `name:` present but does not match the path-implied skill id (kebab-case
  dirname for top-level skills, `ns:sub` for namespaced skills)
- file has a frontmatter block but is missing `description:` entirely
- `name:` over 64 characters
- `name:` contains characters outside `[a-z0-9-]`
- `description:` over 1,024 characters
- use of a `broken-frontmatter-key` (e.g. `paths:`) that Claude Code silently
  mishandles (see the documented `paths:` bug in `lint-claude-kb`)
- use of an unknown frontmatter key outside the documented set

**AC-3.** A new native checker `claude-skill-xref` is registered and wired
into `default_checks()`. It fires on the same globs. It detects a backtick
token immediately followed by the word "skill" (the pattern
`` `token` skill ``) where `token` is not in the set of known skill ids
(collected by scanning the `.claude/skills/` tree at check time) and not in
a configured `external_skills` allow-list.

**AC-4.** A new native checker `claude-skill-dangling-symlink` is registered.
It detects dangling symlinks under `.claude/{skills,agents,commands}` (skipping
`node_modules`, `.venv`, `.git` per `lint-claude-kb`'s existing exclusion
logic).

**AC-5.** Each new checker ships with a backtest against the
`backtest-repos.md` real-world corpus. The backtest result (true-positive rate
and notable false-positive patterns) is recorded in
`docs/claude-skill-checks-backtest.md`.

**AC-6.** Each new checker is covered by unit tests in `tests/` matching the
style of existing checker tests (e.g. `tests/markdown_link_integrity_test.rs`).
Tests exercise at minimum: clean pass, each violation type, and an edge case
per violation (e.g. namespaced `ns:sub` skill id recognition for
`claude-skill-frontmatter`).

### Tier 2 — LLM-Assisted Advisory Pass

**AC-7.** An LLM-assisted advisory check (`claude-skill-weak`) is designed
and documented in `docs/claude-skill-checks.md`. It is explicitly opt-in
(not wired into `default_checks()`). Design covers:
- triggering scope: when to invoke (batch mode only or also per-edit)
- LLM call budget: scoped to the touched skill file + its immediate
  cross-reference neighborhood, not the whole corpus
- flag categories: vague/generic description, monolithic SKILL.md candidate
  for progressive-disclosure split, near-duplicate skill clusters
- how findings surface (advisory severity, human-readable rationale)

**AC-8.** The LLM-assisted check is NOT implemented in this phase — only
designed and documented. Implementation is a follow-on.

### Operational

**AC-9.** `make ready` (ansible-lint + shellcheck) continues to pass after the
changes land.

**AC-10.** The existing `stapler-scripts/lint-claude-kb` Python script is
updated to note (in its own doc comment) that the native kibitzer checks now
supersede it for the checks they cover, and which checks remain Python-only
(oversized-no-split, nested-references, windows-path, removed-builtin-command).

## Out of Scope

- Implementing the LLM-assisted tier (AC-8 covers design only).
- Modifying the `stapler-scripts/lint-claude-kb` logic itself (AC-10 is a
  comment update only).
- Adding tree-sitter parsing of YAML frontmatter (regex parse matching the
  existing `lint-claude-kb` approach is sufficient).
- Cross-repo skill registry (only the local `.claude/skills/` tree is
  scanned; remote/installed plugins use the `external_skills` allow-list).
