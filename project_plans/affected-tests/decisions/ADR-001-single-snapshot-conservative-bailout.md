# ADR-001: `affected` builds one working-tree `ArchModel`, not base+HEAD, and bails out conservatively on unresolvable deletes/renames

**Status**: Accepted
**Date**: 2026-09-22

## Context

`kibitzer affected --base <sha>` must compute the set of packages transitively
affected by a diff, including cases where the diff deletes a file or renames it
across directories. `research/pitfalls.md` (§4, "ArchModel staleness vs. a diff's
base ref") identifies that a single `ArchModel` snapshot cannot, in principle,
answer "who imported this now-deleted file" — that requires knowing the *pre-change*
graph, i.e. a second `ArchModel` built from a `<base>` snapshot (via the
`git archive <ref> | tar -x` pattern already used by
`check_native_against_git_head_repo`, `src/check.rs:1256-1304`).

`requirements.md`'s Open Questions section, updated after Phase 2 research,
explicitly permits resolving this with "the interim default... conservative:
an unresolvable rename or a base ref with no common history falls back to the
`__ALL__` bail-out sentinel rather than guessing" — i.e., a concrete design is
required, but it does not have to be the two-sided-snapshot design.

## Decision

`affected` builds exactly **one** `ArchModel`, from the current working tree
(`repo_root` as it sits on disk — the same `collect_repo_files` + `build_model`
path `arch_export.rs::run_export` already uses, no `git archive` snapshotting).

Changed-file resolution against that one model:
- `Added`/`Modified`/`Renamed` (new-side path): resolved directly via the
  working-tree `ArchModel.packages`/`file_packages` — the file exists on disk,
  so this is unambiguous, *except* when a `Modified` file's diff hunk itself
  changes its `package` clause without a `git mv` (a manual package-boundary
  refactor). That specific case is not resolved here as an ordinary in-place edit;
  it is detected separately (a cheap diff-hunk content check) and bailed out via
  Story 2.2.3, since trusting only the post-change location would silently drop
  the old package's former importers from the seed set.
- `Deleted`, and the *old-side* path of a `Renamed` entry: resolved via
  `resolve_removed_path` (Story 2.2.2) — if the old path's directory still has
  *other* files present in `ArchModel.packages` (a sibling survived), that
  package is added to the seed set as "changed" (its composition changed, even
  though this specific file didn't survive to be inspected directly). If the
  directory has **no** surviving recognized-language files (the package was
  fully removed), `affected` bails out to `AffectedResult::BailOut` rather than
  guessing whether the vanished package's former importers are still safe.

## Alternatives Considered

- **Two-sided snapshot** (build `ArchModel` for both `<base>` and `HEAD`,
  reachability = union of both): correctly resolves every delete/rename case
  without ever bailing out, and has real prior art in this codebase
  (`check_native_against_git_head_repo`'s `git archive`-into-temp-dir pattern).
  Rejected for v1: it doubles the build cost (a second full repo walk +
  tree-sitter parse) on every invocation, in tension with the Non-functional
  Requirement that `affected` be "at most comparable to `test-affected.py`'s
  current `go list -deps` walk, not slower." It is also meaningfully more code
  (a second snapshot-building path, a union-reachability step) for a case
  requirements.md already sanctions solving conservatively.
- **Reuse `ModelCache`**: rejected outright, independent of this decision — see
  `research/stack.md` §3 and `research/architecture.md` §2: `ModelCache` has no
  git-ref key and is scoped to long-lived server processes, not a one-shot CLI.

## Consequences

- A `Modified` Go file whose diff hunk changes its `package` clause without a
  `git mv` (e.g. a manual package-boundary refactor) is **not** an accepted gap of
  this design: Story 2.2.3 closes it by detecting the clause change directly on the
  diff hunk (a cheap content-level check, not a second snapshot or a full
  re-parse) and routing the file through the same conservative bail-out path as
  `PackageFullyRemoved`/`ShallowCloneOrNoCommonHistory`, rather than silently
  trusting the post-change on-disk location. This was the single-working-tree
  design's original blind spot — a `Modified` status never reaches
  `resolve_removed_path`'s delete/rename handling at all — and is now closed, not
  merely documented as a known limitation.
- A diff that deletes the last file in a package (or renames it away with no
  sibling left behind) always bails out to `__ALL__`, even in cases a
  two-sided model could have resolved precisely (e.g. the deleted package had
  zero importers, so bailing out was unnecessary). This trades some CI-time
  precision for correctness-by-construction and matches the "never silently
  under-test" bar exactly — an over-broad bail-out costs CI minutes, an
  under-broad narrowed set costs a missed test.
- `BailOutReason` (Domain Glossary) must carry which package/path triggered
  this specific case distinctly from a config-glob bail-out, so a maintainer
  can tell from the reported reason whether this pattern is firing often
  enough in practice to justify building the two-sided-snapshot alternative
  later (Story 4.1.3's real-diff comparison run is the first opportunity to
  observe this).
- If `<base>` snapshot resolution is added later (e.g. because the
  fully-removed-package bail-out fires too often against real stapler-squad
  history), it is additive: `AffectedResult`/`BailOutReason`'s shape does not
  need to change, only `resolve_removed_path`'s implementation.
