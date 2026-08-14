# `go-blank-imports` check — known false positives

Tracks confirmed false-positive firings of the `go-blank-imports` kibitzer check
(`src/go_blank_imports.rs`), wired up via `kibitzer check native go-blank-imports
{file}`. Check new occurrences against this list before re-investigating a firing
from scratch.

## Mechanism (confirmed by reading the source)

The check walks the tree-sitter parse for every `import_spec` whose `name` field
is a `blank_identifier` (i.e. `import _ "pkg"`), and considers it justified only
if a `comment` node sits on the same row (a trailing comment) or the row
immediately above (a leading comment) — see `collect_blank_imports`. A comment
row that coincides with a *different* import spec's own row is excluded from the
leading-comment lookup (`leading_comment_rows`), so one import's trailing
comment can't accidentally justify the next import down.

## Documented scope gaps

- **Comment two or more lines above the import.** A block comment separated
  from the import by a blank line is not detected as justification — only the
  immediately adjacent row counts. Move the comment to be adjacent, or expect a
  firing.
- **A doc comment on the `import (` block itself**, rather than on the specific
  blank-import line, does not justify any individual entry in the group.
- **Aliased non-blank imports** (`import f "fmt"`) and normal named imports are
  never flagged — only `name: (blank_identifier)` triggers the check.

## Log

No confirmed false-positive firings logged yet. When one occurs, add an entry
here in the same format as `docs/go-primitive-obsession-false-positives.md`:
repo, what changed, why it's a false positive, and the mechanism (traced back to
source, not guessed).
