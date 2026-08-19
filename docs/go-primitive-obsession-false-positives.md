# `go-primitive-obsession` check — known false positives

Tracks confirmed false-positive firings of the `go-primitive-obsession` kibitzer
check (`src/primitive_obsession.rs`, wired up per-project via `.claude/inspect.json`'s
`go-primitive-obsession` entry running `kibitzer check primitive-obsession {file}` on
every `Edit|Write` to a `**/*.go` file). Check new occurrences against this list before
re-investigating a firing from scratch.

## Root-cause mechanism (confirmed by reading the source)

The check does **not** diff old vs. new file content and does not look at which lines
an edit actually touched:

- `hook::run_hook` (`src/hook.rs`) reads the `PostToolUse` event, extracts only
  `tool_input.file_path`, and calls `run_checks_smart` — it never inspects the tool's
  diff/patch content, just the path.
- `check::run_check` (`src/check.rs`) shells out to the configured command
  (`kibitzer check primitive-obsession {file}`) with the file path substituted in.
- `main.rs`'s `CheckCommand::PrimitiveObsession` handler calls
  `primitive_obsession::check_file(&file)`, which does `std::fs::read_to_string(path)`
  and tree-sitter-parses the **entire current file on disk**, then walks every
  `parameter_list` in it (`primitive_obsession.rs::walk`/`check_parameter_list`).

So the check scans the whole file's current contents on every `Edit`/`Write` to any
`.go` file, and reports every matching signature anywhere in that file — regardless of
whether the just-applied edit added, removed, or left that signature untouched. The
project's `.claude/settings.json` matcher (`"matcher": "Edit|Write"`) and the check's
`scope: ["**/*.go"]` mean it fires on essentially any edit to any Go file in scope, not
just one that introduces a new same-typed-parameter signature.

## Log

### 2026-08-18 — stapler-squad-tests — ambiguous-substring fallback re-scanned whole file

- **Repo**: `tstapler/stapler-squad` (session `stapler-squad-tests`), file
  `server/services/session_service_test.go`.
- **What changed**: an `Edit` moved an identical 3-line setup block
  (`eventBus := events.NewEventBus(8)` / `svc := NewSessionService(storage, eventBus)`
  / `t.Cleanup(func() { svc.Shutdown() })`) into a new `t.Run("onDetected", ...)`
  subtest of a table that already duplicates this same boilerplate across several
  other subtests.
- **Why it's a false positive**: the checker isn't a bug here — the underlying
  diff-scoping infra (`src/hook.rs::compute_changed_lines`, `src/check.rs`'s
  `changed_lines`/git-HEAD-baseline machinery) *does* exist and normally prevents
  the whole-file rescan described above. This case bypassed it: `compute_changed_lines`
  located an `Edit`'s `new_string` by searching for it as a unique substring of the
  current file, and previously bailed to `None` (unscoped, whole-file check) whenever
  that text occurred more than once — which duplicated subtest boilerplate guarantees.
- **Fix**: `compute_changed_lines` (`src/hook.rs`) now scopes to the union of *all*
  occurrences of an ambiguous `new_string` instead of giving up and scanning the whole
  file. Covered by `unions_all_occurrences_when_new_string_is_ambiguous` and
  `duplicated_subtest_boilerplate_scopes_to_all_copies_not_whole_file` in
  `src/hook.rs`'s test module.

### 2026-08-10 — stapler-squad — deletion-only edit flagged

- **Repo**: `tstapler/stapler-squad`
- **What changed**: an `Edit` to `server/tls.go` that only *removed*
  `LoadTLSConfig(certFile, keyFile string) (*tls.Config, error)` because it had become
  dead code. No function signature was added.
- **Why it's a false positive**: nothing new was introduced for the checklist rule to
  flag — the edit was a pure deletion.
- **Mechanism**: per the whole-file rescan behavior above, the hook re-parses whatever
  is left in `server/tls.go` (and/or the sibling file also touched) after the edit and
  flags any *other* pre-existing same-typed-parameter signature still present, or in
  this case appears to fire independent of whether the specific edited hunk added or
  removed anything — the tool does not distinguish "diff added this" from "file
  contains this."

### 2026-08-10 — stapler-squad — pre-existing unchanged signatures flagged

- **Repo**: `tstapler/stapler-squad`
- **What changed**: edits to `server/tls.go` and `main.go` that did not alter the shape
  of the flagged functions.
- **Functions flagged**: `certCurrent(certFile, hashFile, want string)` and
  `LoadTLSConfig(certFile, keyFile string)` (`server/tls.go`) — both pre-existing,
  unchanged in signature shape by the diff.
- **Why it's a false positive**: these signatures were not newly introduced by the
  edit; they already existed in the file before the edit and were untouched by it.
- **Mechanism**: confirmed directly from source — `check_file` reads and parses the
  *entire current file*, not a diff. Any edit to a `.go` file that contains an
  already-flaggable signature anywhere in it will re-surface that finding on every
  subsequent `Edit`/`Write` to that file, whether or not the edit touched that
  particular function.
