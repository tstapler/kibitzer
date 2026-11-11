# Requirements: fix-toml-frontmatter-false-positive

## Background

Backlog item 504be758-5881-4de1-a6fe-f12723a8697f.

Editing a Hugo blog post (`content/blog/<slug>/index.md`) with TOML frontmatter (`+++`
delimiters) made the Stop hook report false positives on every turn:

```
index.md: markdown-link-integrity: broken markdown link/anchor
index.md:5: ["Software Engineering", "Infrastructure"] used but never defined
index.md:6: ["kotlin", "java", ...] used but never defined
index.md:7: ["kotlin coroutines blocking detection", ...] used but never defined
```

Lines 5-7 are `categories`/`tags`/`keywords` TOML arrays inside the `+++` frontmatter
block, not markdown links. An already-published post in the same repo uses identical
frontmatter.

Three distinct problems are reported:

**Problem 1 (primary)**: `check_source` (`src/checkers/markdown_link_integrity.rs:79`)
passes the entire file body to `pulldown-cmark` without first stripping or skipping the
leading `+++` or `---` frontmatter block. A TOML or YAML array like
`categories = ["Software Engineering", "Infrastructure"]` contains `["a", "b"]` which
pulldown-cmark tokenizes as a shortcut reference-link use. With no matching definition
anywhere in the document, every element is emitted as `LinkType::ShortcutUnknown` and
fires a "used but never defined" finding.

**Problem 2 (minor)**: The first line of the Stop hook report is
`index.md: markdown-link-integrity: broken markdown link/anchor`, which names no
specific link. This is the check's configured `message` field (from `core_checks()` in
`src/config.rs`) being prepended to the per-line findings by `describe()`. It is
technically correct but confusing since the user sees a "broken link" headline followed
by what looks like link labels with no link text.

**Problem 3 (independent)**: After filing `report_false_positive` for the check/file,
the Stop hook re-fired the identical finding five more times on turns that did NOT
include an edit to `index.md`. The `report_false_positive` queue (`false_positive.rs`)
is a local maintainer-drain queue — it has no runtime effect on kibitzer's check
execution or finding suppression. So filing a false positive report does not suppress
future findings.

Additionally, the Stop hook's loop-safety offset mechanism (`task_stop.rs`) saves the
offset before running checks, but if the save fails (the result is discarded via
`let _ = ...`), the offset does not advance and all previous transcript lines are
re-processed on the next Stop firing, potentially re-reporting files that were only
touched earlier in the session.

## Current State

- `check_source` (`src/checkers/markdown_link_integrity.rs:79`) enables only
  `Options::ENABLE_TASKLISTS | Options::ENABLE_WIKILINKS` and passes the full body to
  the parser with no frontmatter stripping. Confirmed by grep: no match for
  "frontmatter", "toml", "+++", or "---" in that file.
- pulldown-cmark's `Options` flags (checked in Cargo.toml) do not include a metadata-
  block option for `+++` TOML frontmatter. YAML frontmatter (`---`) is similarly
  unsupported at the parser level.
- The Stop hook offset save discards errors: `let _ = SessionOffset { offset: new_offset
  }.save(&offsets_path);` (`task_stop.rs:228`). A silent save failure means the offset
  stays at 0 or the previous checkpoint, causing re-processing.
- `report_false_positive` writes to the queue file only; nothing reads the queue at
  check-run time.

## Functional Requirements

### AC-1
When `check_source` processes a markdown file whose first non-empty line is `+++`, the
entire TOML frontmatter block (up to the matching closing `+++` on its own line) is
excluded from markdown parsing. No findings are emitted for content inside the block.

### AC-2
When `check_source` processes a markdown file whose first non-empty line is `---`, the
entire YAML frontmatter block (up to the matching closing `---` on its own line) is
excluded from markdown parsing. No findings are emitted for content inside the block.

### AC-3
Content in the markdown body after the frontmatter block is still parsed and checked
normally. A dangling reference link in the body (outside the frontmatter) is still
flagged.

### AC-4
A file with no frontmatter (`+++` or `---` does not appear on the first non-empty line)
behaves identically to today — no regression.

### AC-5
Unit tests in `src/checkers/markdown_link_integrity_tests.rs` cover:
- TOML `+++` frontmatter with array values: zero findings
- YAML `---` frontmatter with array values: zero findings
- TOML frontmatter followed by a dangling reference in the body: exactly one finding
  (the body reference), zero from the frontmatter
- A `---` line inside a fenced code block is NOT treated as frontmatter
- File with no frontmatter: existing tests still pass

### AC-6
The fix is backtested using `kibitzer check backtest markdown-link-integrity` against
Claude Code session transcripts and against the standard backtest-repos corpus
(at minimum `microsoft/vscode` and `kubernetes/kubernetes` for markdown coverage).
Before/after false-positive counts are recorded.

### AC-7
A log entry is added to `docs/markdown-link-integrity-false-positives.md` describing
the TOML/YAML frontmatter false positive, its mechanism, and linking to the fix.

### AC-8 (Stop hook offset reliability — independent of AC-1)
The Stop hook's `SessionOffset::save` failure is surfaced rather than silently
discarded. If the save fails and the offset cannot be advanced, the hook should not
re-report already-seen findings. Acceptable approaches:
  a. Log the save error to stderr (advisory only, no blocking behavior) so the failure
     is visible during debugging.
  b. Fall through to no-op (don't report any findings for this firing) when the offset
     cannot be persisted, since reporting without advancing the offset creates the
     repeating-finding problem.

## Out of Scope

- Cross-file frontmatter (front matter that continues to a second file): not a real use
  case; Hugo and Jekyll both require frontmatter in the same file.
- Parsing TOML/YAML frontmatter values to validate them: kibitzer is a markdown checker,
  not a site generator.
- Making `report_false_positive` suppress findings at runtime: the queue is intentionally
  a maintainer workflow, not a runtime suppression mechanism. The correct fix is AC-1
  (don't fire in the first place).
- General Stop hook deduplication for multiple registered Stop hooks: a separate
  follow-up if confirmed.
