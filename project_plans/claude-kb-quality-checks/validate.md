# Validation: Claude Code Skill Quality Checks

**Project:** claude-kb-quality-checks  
**Date:** 2026-11-04

## Plan Validation Against Requirements

### AC-1 — `claude-skill-frontmatter` in `default_checks()`

**Plan coverage:** Task 2 creates the checker; Task 5 wires it into `claude_kb_checks()` → `default_checks()`. The `**/.claude/**/*.md` glob with in-checker path filtering covers skills, agents, and commands.

**Risk:** The file-glob matches `.claude/CLAUDE.md` and other non-authoring files. The in-checker guard handles this, but it must be exercised in tests.

**Verdict:** Covered. Add an explicit test case for a file under `.claude/` that is NOT under `{skills,agents,commands}` — expect 0 findings.

### AC-2 — Frontmatter violation detection

**Plan coverage:** Task 2 lists all nine violation types with their conditions and severities. The frontmatter parser handles block scalars (needed for multi-line `description:`).

**Gap identified:** The plan does not specify what line number is reported for each finding. The `missing-description` finding has no key line to anchor to — should report line 1 (the opening `---` fence). Clarify in implementation.

**Verdict:** Covered with one clarification: `missing-description` and `unknown-frontmatter-key` report the line of the frontmatter block opener (`---`, line 1) when no key-specific line is available.

### AC-3 — `claude-skill-xref` checker

**Plan coverage:** Task 3 fully specifies the regex, registry-build logic, `configure()` for `external_skills`, and unit tests including namespaced `ns:sub` recognition.

**Risk:** The skill registry scan reads the filesystem at check time. If the checker fires on a file that was just created (and whose directory doesn't yet have sibling skills), the scan may succeed but return an empty or partial registry. This is acceptable — the check is advisory.

**Verdict:** Covered.

### AC-4 — Dangling symlink detection

**Plan coverage:** Task 4 identifies the design tension (per-file vs. batch) and proposes a one-shot-per-root cache approach. Fallback to batch command is noted.

**Concern:** A per-file checker that scans the whole skills tree on first trigger, then caches, will produce findings attached to whatever `.md` file happened to trigger the scan first — not the actual symlink paths. This is confusing in kibitzer's output format (`{file}:{line}: message` where file is a `.md` file, not the symlink).

**Recommendation:** Implement `claude-skill-dangling-symlink` as a `batch`-only check (trigger `["batch"]`) that shells out to a small helper command, or as a whole-repo native check. This avoids the misleading file attribution. Update Task 4 in the plan.

**Verdict:** Plan needs revision on `claude-skill-dangling-symlink` shape. Recommend batch-only command check for clean file attribution.

### AC-5 — Backtest

**Plan coverage:** Task 6 specifies running against the dotfiles repo and the existing public corpus, documenting results in `docs/claude-skill-checks-backtest.md`.

**Gap:** The dotfiles repo path is not in the existing `backtest-repos.md` corpus list. Task 6 should add it. The step of adding the dotfiles repo path to the corpus list must be part of the task, not assumed.

**Verdict:** Covered if Task 6 explicitly includes adding the dotfiles entry to `docs/backtest-repos.md`.

### AC-6 — Unit tests

**Plan coverage:** Tasks 2, 3, and 4 each specify a test file with an explicit list of test cases. The cases cover clean pass, each violation type, and edge cases (namespaced IDs, no frontmatter, no `.claude/` root).

**Gap:** No test for `configure()` with an `external_skills` list (AC-3). Add an explicit test case for this in Task 3's list.

**Verdict:** Covered with that addition.

### AC-7 — LLM-assisted check design

**Plan coverage:** Task 7 covers the design document content. The description matches AC-7's requirements (triggers, call budget, flag categories, finding format, opt-in mechanism).

**Verdict:** Covered.

### AC-8 — LLM-assisted check not implemented

**Plan coverage:** Task 7 explicitly produces only a design doc. No implementation tasks exist for the LLM tier.

**Verdict:** Covered.

### AC-9 — `make ready` continues to pass

**Plan coverage:** No Ansible or shell changes are introduced. The only files modified are Rust sources and the Python `lint-claude-kb` script's doc comment.

**Risk:** Adding checkers to `default_checks()` changes the count, breaking the count-assertion test in `tests/config_test.rs`. Task 5 explicitly calls this out.

**Verdict:** Covered.

### AC-10 — `lint-claude-kb` doc comment update

**Plan coverage:** Task 8 specifies adding a note listing checks now covered natively and which remain Python-only.

**Verdict:** Covered.

## Open Questions

**Q1 — Glob vs. path-filter approach:**  
The plan uses `**/.claude/**/*.md` as the file glob plus an in-checker path guard. An alternative is a single `**/*.md` glob with the guard doing all scoping. The `**/.claude/**/*.md` approach is preferable — it avoids the checker being invoked for every `.md` file in a large repo that has no `.claude/` tree at all — but confirm that kibitzer's glob engine anchors `**/.claude/**` correctly (the leading `**/` makes it match `.claude/` anywhere in the path).

**Q2 — Severity mix within one checker:**  
`claude-skill-frontmatter` produces both Blocking and Advisory findings from the same checker. The `Check` struct in `src/config.rs` has a single `severity` field applied to the whole check, but individual `Finding` structs don't carry severity — severity is at the check level. This means the checker cannot mix Blocking and Advisory findings within a single `Check` entry.

**Resolution options:**
- Split into two checks: `claude-skill-frontmatter` (Blocking, errors only) and `claude-skill-frontmatter-advisory` (Advisory, warnings like `missing-description`).
- Accept that the whole check runs at Advisory severity — findings are surfaced but do not block.
- Check how other mixed-severity checkers handle this.

This is a design decision that must be made before Task 2 implementation begins. Recommendation: split into two registered checks, both wired into `default_checks()`.

**Q3 — `invalid-skill-location` check:**  
The `lint-claude-kb` script flags a bare `<name>.md` directly under `.claude/skills/` (instead of `<name>/SKILL.md`) as an error. This check would need to fire on the `.md` file itself. The `claude-skill-frontmatter` checker, triggered per-file, can detect this condition from the file path: `parent is .claude/skills/ AND filename != SKILL.md`. Add this to Task 2's check list.

## Revised Task List

After validation:

1. **Task 1** (common module) — unchanged
2. **Task 2** (frontmatter checker) — add `invalid-skill-location` check; clarify line reporting for `missing-description`
3. **Task 2a** (advisory-only frontmatter split) — if Q2 resolution is "split", create a companion `claude-skill-frontmatter-warn` check for advisory-only findings
4. **Task 3** (xref checker) — add `configure()` unit test
5. **Task 4** (dangling symlink) — revise to batch-only command check shape
6. **Task 5** (wire into defaults) — unchanged but update count
7. **Task 6** (backtest) — add dotfiles repo to corpus list first
8. **Task 7** (design doc) — unchanged
9. **Task 8** (lint-claude-kb update) — unchanged

## Confidence Assessment

| Area | Confidence | Notes |
|---|---|---|
| Checker trait and registration | HIGH | Directly observed in codebase |
| Frontmatter parsing approach | HIGH | Direct port of working Python logic |
| Cross-file skill registry (LazyLock pattern) | MEDIUM | Pattern is standard Rust; correctness of the cache lifetime in a daemon session should be verified with a test |
| Dangling symlink (batch shape) | MEDIUM | Batch command check is simpler than per-file; correct invocation pattern needs verification |
| `default_checks()` count test | HIGH | Expected breakage, explicitly called out |
| Backtest on dotfiles repo | HIGH | Lint-claude-kb provides a known-good baseline to compare against |
| LLM-assisted tier design | MEDIUM | The design is sound in outline; the specific LLM prompt engineering for "vague description" detection needs a prototype run |
