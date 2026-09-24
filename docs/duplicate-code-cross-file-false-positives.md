# `duplicate-code-cross-file` check — known false positives

Tracks confirmed false-positive firings of the `duplicate-code-cross-file` kibitzer
check (`src/checkers/duplicate_cross_file_checker.rs`), wired up via the
`PostToolUse` hook. Check new occurrences against this list before
re-investigating a firing from scratch.

## Mechanism (confirmed by reading the source)

Shares its window-hashing logic with `duplicate-code`
(`docs/duplicate-code-false-positives.md`) through `qualifying_window`/
`qualifying_windows` in `src/checkers/duplicate_code.rs`: a 6-line window
(`MIN_BLOCK_LINES`) over whitespace-trimmed lines, filtered only for blank
lines and a 60-character minimum (`MIN_BLOCK_CHARS`), fires once a window's
exact text repeats across at least 3 files (`MIN_OCCURRENCES`,
`spans_multiple_files`). No exclusion for a window that falls entirely inside
a file's leading `import`/`use` block — see issue #102.

## Log

### 2026-09-23 — tstapler/stelekit (feat/graph-creation-name-description) — shared import block flagged as duplicated logic

- **Repo**: `tstapler/stelekit` (branch `feat/graph-creation-name-description`), files:
  - `kmp/src/jvmTest/kotlin/dev/stapler/stelekit/ui/StelekitViewModelSyncStateTest.kt:7`
  - `kmp/src/jvmTest/kotlin/dev/stapler/stelekit/ui/StelekitViewModelSyncStateIntegrationTest.kt:7`
  - `kmp/src/businessTest/kotlin/dev/stapler/stelekit/git/GitSyncServiceRateLimitRetryTest.kt:6`
- **What changed**: added a new jvmTest file (`GitSetupScreenScreenshotTest.kt`) and
  extracted a shared `buildTestGitSyncService()` fixture out of the two
  `StelekitViewModelSyncState*Test.kt` files, touching their import lists in the
  process.
- **Why it's a false positive**: the flagged 6-line block is six `import`
  statements (`arrow.core.Either`/`left`/`right`,
  `dev.stapler.stelekit.db.GraphLoader`/`GraphWriter`,
  `dev.stapler.stelekit.error.DomainError`) that the three files share only
  because they use the same libraries — there is no function to "extract,"
  and each file's own compiler dictates the exact import list it needs.
- **Mechanism**: `qualifying_window`/`qualifying_windows`
  (`src/checkers/duplicate_code.rs`) have no import/use-block exclusion — see
  this doc's "Mechanism" section above and issue #102.
