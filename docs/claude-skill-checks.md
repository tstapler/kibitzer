# Claude Code skill / agent / command checks

Advisory checks for Claude Code's own authoring trees: `.claude/{skills,agents,commands}`.
Ported from `lint-claude-kb` in `tstapler/dotfiles`. All are in `default_checks()`
(`claude_kb_checks()` in `src/config.rs`) and Advisory severity: a `Check` carries one
severity, and the script's error and warn tiers share it.

## Native checks

| Checker | Trigger | Findings |
|---|---|---|
| `claude-skill-frontmatter` | per `.md` under the three trees (path-filtered in-checker) | `name-mismatch`, `missing-description`, `name-too-long` (>64), `name-invalid-chars`, `name-xml-tag`, `description-too-long` (>1024), `description-xml-tag`, `broken-frontmatter-key` (`paths:`), `unknown-frontmatter-key`, `invalid-skill-location` (bare `.md` directly under `.claude/skills/`) |
| `claude-skill-xref` | per `.md` | `broken-skill-ref`: a backtick token immediately followed by "skill" that is not in the scanned skill-id registry or the `external_skills` allow-list |
| `claude-skill-dangling-symlink` | **batch only** | a symlink under the three trees whose target does not resolve (`node_modules`, `.venv`, `.git` skipped) |

Notes:

- Path-implied ids: `<dir>` for a top-level skill, `ns:sub` for `ns/skills/sub/SKILL.md`.
  A file with no frontmatter block, or with no `.claude/` ancestor, yields no findings.
- `external_skills` allow-list (skills that ship with Claude Code or a plugin and are not on disk):

  ```json
  {"name": "claude-skill-xref", "options": {"external_skills": ["security-review", "update-config"]}}
  ```

- The symlink check is batch-only because its subject is the symlink path. A per-file shape
  would attach the finding to whichever `.md` was edited. The runner calls it with an empty
  source for unreadable paths (`Checker::inspects_unreadable_paths`).
- Suppress with the usual `kibitzer:ignore` directive (`docs/suppressing-checks.md`).
- Python-only, not ported: `oversized-no-split`, `nested-references`, `windows-path`,
  `removed-builtin-command`.
- Not implemented: the "double-registered skill" symlink case (same content reachable as a
  top-level dir and inside a namespace) from the original issue.

Backtest results: `docs/claude-skill-checks-backtest.md`.

## Design: `claude-skill-weak` (LLM-assisted, NOT implemented)

Judgment calls a regex pass cannot make. Kept out of the native checker model because it
makes a network/LLM call, is non-deterministic, and costs money.

**Flag categories**

1. `vague-description`: a `description:` too generic to be selected by the skill picker
   (no trigger phrases, no "use when…" clause, could describe any skill).
2. `monolithic-skill`: a `SKILL.md` that should progressive-disclose into
   `references/`, `scripts/`, `assets/` (candidate when over ~300 lines with no such directory).
3. `duplicate-cluster`: near-duplicate skills that should collapse into one namespaced family.
4. `plugin-candidate`: loose skills that belong grouped in a plugin.

**Triggers.** Batch-only, never per-edit. Categories 1 and 2 run on skills touched in the
diff; 3 and 4 need the neighborhood and run only when a skill is added or renamed.

**Scope and call budget.** One call per touched skill, input limited to that file plus the
frontmatter (`name`, `description`) of skills it cross-references and of its directory siblings.
Never the whole corpus. Hard cap of N calls per run (default 10); skills beyond the cap are
reported as "not assessed", not silently skipped. Results are cached by content hash so an
unchanged skill is never re-assessed.

**Pre-filter.** Cheap deterministic signals (description under 40 chars, no "use when",
line count, name similarity) pick candidates, so most skills never reach the LLM.

**Output.** Findings use the standard `Finding` shape, line pointing at `description:` or
line 1, rule ids `claude-skill-weak/<category>`. Always Advisory.

**Opt-in.** Not in `default_checks()`. Enabled by an explicit `.kibitzer/inspect.json`
entry that names the model/provider and the call cap. Without it the check does not run and
makes no network call.

**Open question.** Prompt wording for `vague-description` needs a prototype run against
the dotfiles corpus before a false-positive rate can be claimed.
