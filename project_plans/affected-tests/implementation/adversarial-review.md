# Adversarial Review: affected-tests

**Date**: 2026-09-22
**Verdict**: CONCERNS

## Blockers

(none — all three from the previous review are verifiably fixed)

- ~~Default `bail_out_globs` dropped `**/*.proto` and build-tag coverage~~ — **FIXED**.
  `default_bail_out_globs()` (Task 1.2.2a, `plan.md:458`) is now literally
  `vec!["**/go.mod".into(), "**/go.sum".into(), "**/*.proto".into()]` — the `.proto`
  entry is in the actual code, not just prose. Build-tag-bearing-file detection is
  now an explicit accepted gap with a stated reason (`plan.md:478-484`: a
  `//go:build` constraint is a content-level signal a path glob can't express), and
  is cross-referenced from the plan's Unresolved Questions section
  (`plan.md:129-135`). `requirements.md`'s resolved Open Question
  (`requirements.md:253-263`) now states the same default list (`go.mod`/`go.sum`/
  `**/*.proto`) and the same build-tag accepted-gap reasoning — no contradiction
  between the two docs remains.
- ~~All default globs were bare filenames, anchored to repo root~~ — **FIXED**.
  Verified directly against `src/glob.rs:5-33`: `glob_to_regex` anchors every
  pattern with `^...$`, and a leading `**/` compiles to `(.*/)?`, which matches
  zero-or-more path segments — so `**/go.mod` matches both a root `go.mod` and a
  nested `tools/scanner/go.mod`, while a bare `"go.mod"` would only match root.
  Every entry in the literal default list is now `**/`-prefixed. Story 1.2.2's
  acceptance criteria (`plan.md:414-436`) adds an explicit nested-path
  Given-When-Then (`tools/scanner/go.mod`), a documented rationale citing
  `glob.rs`'s anchoring behavior as pre-existing convention (not a new footgun),
  and Task 1.2.2b adds a table-driven test asserting each default matches both at
  root and at depth (`plan.md:487-492`). Grepped the whole plan for any remaining
  bare `"go.mod"`/`"go.sum"` glob *definitions*: the only bare-quoted occurrences
  left (`plan.md:734`, `plan.md:860`) are `ChangedFile.path` example *values* (a
  file that changed, e.g. root `go.mod`), not glob patterns, and are paired
  correctly with the `"**/go.mod"` glob that matched them — not a stale default.
- ~~Contradictory unresolvable-`--base` exit contract~~ — **FIXED**, and now
  consistent across all three documents:
  - `requirements.md:271-273`: "Distinct exit codes are reserved for genuine
    errors (bad base ref, not a git repo), never for 'bail out' or 'nothing
    affected'."
  - `research/ux.md:107-119` (prose) now explicitly carves out the unresolvable-
    ref/missing-git case as a hard error ("this does **not** extend to a `--base`
    ref that fails to resolve at all... these fail loud as a distinct hard
    error"), and the recommendation table (`research/ux.md:153`) agrees: "Base
    ref invalid / not a git repo | Exit 1." The previous version had the table
    and prose disagreeing with each other; that's resolved.
  - `plan.md`'s Domain Glossary (`plan.md:59,63,64`) and Story 1.1.1
    (`plan.md:186-235`) both state the hard-error framing, `BailOutReason`
    (`plan.md:750`) has exactly three variants (`GlobMatched`,
    `PackageFullyRemoved`, `ShallowCloneOrNoCommonHistory`) with no
    `UnresolvableBaseRef` variant anywhere in the document (grepped for the
    string — the only hit is inside this review's own history of the old
    finding), and `resolve_base_sha`'s described behavior (Task 1.1.1a,
    `plan.md:245-251`) explicitly disclaims mapping a failed `git rev-parse` to
    `Ok(None)`, calling out that this is a deliberate change from "an earlier
    draft of this task." Every Given-When-Then touching base-ref resolution
    (Story 1.1.1's three ACs, Story 3.2.2's third AC, Story 4.1.2's case (d),
    Task 3.2.2c) consistently asserts `Err`/non-zero-exit/empty-stdout for this
    case and never conflates it with a `BailOut` variant.

## Concerns

- **Merge-commit handling is still asserted, not designed or tested** (carried
  forward, unchanged). `requirements.md`'s Rabbit Holes section still flags merge
  commits as a correctness edge case, and Task 4.1.3a (`plan.md:1062-1071`) still
  only samples "a merge commit" as one category in the real-diff comparison — no
  Phase 1 acceptance criterion or unit test exercises a merge-commit-as-base or
  merge-commit-as-HEAD shape. Same gap as before.
- **The in-place package-identity-change gap in ADR-001 is still unaddressed**
  (carried forward, unchanged). Checked ADR-001's Consequences section
  (`decisions/ADR-001-single-snapshot-conservative-bailout.md:58-75`) directly:
  it still only names the fully-removed-package bail-out tradeoff and the
  future two-sided-snapshot option — no bullet was added for a `Modified` file
  that changes which package it logically belongs to (e.g. an edited `package`
  declaration with no `git mv`), which `resolve_changed_packages` resolves only
  to its current package and `resolve_removed_path` never sees (it's only wired
  for `Deleted`/`Renamed`-old paths, per Task 2.2.2b). Recommendation stands:
  name this explicitly as an accepted-risk bullet.
- **Story 4.1.3's real-diff comparison is still a one-time, hand-run exercise
  with no regression gate** (carried forward, unchanged). No automation re-runs
  `docs/affected-validation.md`'s comparison after this initial validation pass.
- **Resolved since last review, no longer a concern**: the previous review's
  "no task validates the NFR performance bar" concern is now addressed — Story
  4.1.3's acceptance criteria (`plan.md:1048-1058`) and Tasks 4.1.3b/4.1.3c
  (`plan.md:1073-1091`) now explicitly require wall-clock timing (`time <cmd>`)
  for both tools on the same commits, recorded in `docs/affected-validation.md`,
  with a stated verdict on whether the "not slower than `go list -deps`" NFR is
  met. Noting this as closed rather than silently dropping it.

## Minors

- The bail-out check ordering (globs before the graph walk, `plan.md:656-657`) and the cycle-safe reverse BFS (Story 2.1.2, explicit visited set, tested against `import_graph.rs`'s own two-package cycle fixture) are both well-reasoned and correctly test-covered — no notes.
- Binary files in the diff are handled only implicitly (no `file_packages` entry → silently skipped, same path as `README.md`/vendor files) rather than via an explicit acceptance criterion naming "binary file" as a case — functionally fine, but `pitfalls.md`'s explicit callout of this case deserves one named test rather than relying on the reader to infer it's covered by the "unsupported extension" path.
- Unbounded stdout on a very large affected set (flagged as a general, not repo-verified, CI pattern in `pitfalls.md` §5) is not addressed anywhere in the plan. Given the target repo scale (hundreds to low-thousands of packages per `research/stack.md` §4), this is unlikely to bite in practice — noting for awareness, not blocking.
- `ArchitectureAction::Affected` nests the subcommand as `kibitzer architecture affected`, while `requirements.md` throughout describes it as top-level `kibitzer affected`. `requirements.md` itself marks the exact name "TBD in planning," so this isn't a violation, but it's worth double-checking that stapler-squad's eventual cutover PR author (a future task, out of scope here) is aware the final invocation is the longer nested form.
