# Architecture Review: affected-tests
**Date**: 2026-09-22
**Verdict**: CONCERNS

## Blockers

None. The original Blocker (Story 1.2.1's `bail_out_globs` replacing hardcoded
defaults) is resolved:

- The field was renamed to `extra_bail_out_globs: Vec<String>` and is now
  documented (Domain Glossary, plan.md:55; Task 1.2.1a, plan.md:380-391) and
  tested (Task 1.2.1c, plan.md:401-405) as additive-only.
- `AffectedConfig::effective_bail_out_globs()` (plan.md:56, 383) chains
  `default_bail_out_globs()` with `self.extra_bail_out_globs` — the union, not
  the raw field, is what `compute_affected` actually checks (Task 3.1.1b step 4
  and Task 3.1.1c, plan.md:766-767, 784, explicitly calling out "not the raw
  `extra_bail_out_globs` field").
- The Pattern Decisions table (plan.md:83) states the semantics directly and
  gives the rationale distinguishing this from `Check.scope`'s legitimate
  replace-on-set convention (replacing here would silently drop the safety net
  rather than just narrow what gets linted).
- Consistency check across every remaining reference — Story 1.2.1 (plan.md:344-406),
  Story 1.2.2 (plan.md:408-467), Story 3.1.1/Task 3.1.1b-c (plan.md:718-785),
  Story 3.2.2/Task 3.2.2a-b (plan.md:846-933) — found no stale mentions of the
  old `bail_out_globs` name or replace semantics; every call site consistently
  reads `config.effective_bail_out_globs()`.
- This now matches the repo's own `.claude/inspect.json`-overlays-defaults
  convention the original Blocker cited (CLAUDE.md's "Default check catalog"
  section) — same shape as `merge_checks` (`src/config.rs:767-782`): local
  config extends, never replaces, the shipped default.

All four original Concerns were also verified as addressed in the current plan,
not just claimed:

- **kibitzer-repo-specific defaults**: `default_bail_out_globs()` (Task 1.2.2a,
  plan.md:457-461) now ships only `**/go.mod`, `**/go.sum`, `**/*.proto` — no
  `src/affected.rs`/`src/import_graph.rs`/`src/arch_model.rs` entries. Story
  1.2.2 adds an explicit acceptance criterion asserting the default list
  contains none of kibitzer's own paths (plan.md:442-453), and kibitzer's
  self-protection is repositioned as its own `extra_bail_out_globs` overlay
  entry (plan.md:448-450) rather than a shipped default.
- **Tech Debt Disposition mislabel**: relabeled from "Isolate via seam" to
  "Accept and document" (plan.md:93), with an explicit note that the relabel
  is a direct response to the architecture review finding no seam was actually
  built. Task 4.1.3a (plan.md:1062-1071) now also tries to sample a
  `replace`-directive commit from stapler-squad history, or states plainly if
  none exists, closing the original "criteria don't exercise this risk" gap.
- **Ad hoc file→package lookup**: Task 2.2.1b (plan.md:630-647) now adds
  `pub fn package_for_file(&self, path: &Path) -> Option<&str>` on `ArchModel`
  instead of a private derived `BTreeMap` in `affected.rs` — exactly the
  remediation the original review asked for, framed as consistent with the
  Tech Debt Disposition's "extend as-is" stance.
- **Missing performance validation**: Task 4.1.3b (plan.md:1073-1080) now wraps
  both tools in `time` and records wall-clock timing per commit; Task 4.1.3c
  (plan.md:1082-1090) requires a one-line verdict on whether the NFR is met and
  whether a regression pulls the daemon-cache follow-up forward. Story 4.1.3's
  acceptance criteria (plan.md:1048-1058) make this a required output of
  `docs/affected-validation.md`, not just a suggestion.

## Concerns

- **New, minor: Domain Glossary claims `effective_bail_out_globs()` is
  "deduplicated" but the specified implementation isn't.** plan.md:56 says
  "`default_bail_out_globs()` chained with `self.extra_bail_out_globs`,
  deduplicated," but Task 1.2.1a's actual code (plan.md:383) is a plain
  `.into_iter().chain(...).collect()` with no `.dedup()`/`HashSet` step, and
  Task 1.2.1c's tests (plan.md:401-405) check union content, not absence of
  duplicates. Practically low-severity — a duplicate glob just means
  `matches_any_bail_out_glob` (Task 3.1.1c) checks it twice, no wrong-answer
  risk — but the glossary entry over-promises relative to the specified code.
  **Remediation**: either add `.dedup()` (with a preceding `.sort()`, since
  `Vec::dedup` only removes consecutive duplicates) to the Task 1.2.1a snippet,
  or drop "deduplicated" from the glossary's description to match what's
  actually specified.

## Nitpicks

- `run_affected`'s match on `AffectedResult::Packages(packages) if packages.is_empty()`
  vs. the non-guarded `Packages(packages)` arm is a minor readability wart; an
  `if packages.is_empty() { .. } else { .. }` inside one arm reads more directly.
- A single `ChangeStatus::Renamed { old, .. }` entry is processed by two independent
  functions (`resolve_changed_packages` for the new-side path, `resolve_removed_path`
  for the old-side path) with no compiler-enforced link ensuring both stay in sync if
  `ChangeStatus` grows a new variant later.
- Package keys stay raw `String`/`&str` throughout (`ReverseAdjacency`,
  `AffectedResult::Packages(Vec<String>)`) rather than a `PackageKey` newtype — this
  matches `ArchModel`'s own existing convention, so it's not a regression, just worth
  naming if a newtype is ever introduced repo-wide.
- kibitzer's own default checks (`long-function` at 40 lines, `file-size` at 500
  lines) already fire heavily on `src/config.rs`, `src/main.rs`, `src/arch_model.rs`,
  and `src/import_graph.rs` (verified via `kibitzer run src --trigger batch`, e.g.
  `src/mcp.rs:538: body spans 182 lines`). `compute_affected`'s 8-step orchestration
  (Task 3.1.1b) will likely trip `long-function` once written; the plan doesn't budget
  a decomposition pass, which is mildly ironic for a code-quality tool's own new code.
