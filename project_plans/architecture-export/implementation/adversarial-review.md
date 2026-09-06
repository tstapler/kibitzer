# Adversarial Review: architecture-export

**Date**: 2026-08-24
**Verdict**: CLEAN

## Blockers

Both previously-open blockers are resolved.

- [x] **Parse-error handling** — resolved. `PruningSummary.files_with_parse_errors: Vec<PathBuf>`
  is defined (Task 1.1.1c), Story 1.3.1's AC and Task 1.3.1c name the real check
  (`tree.root_node().has_error()`), specify the exact behavior (skip the file's extraction
  entirely, record its path, `build_model` still returns `Ok(_)` never `Err`/panic), and Task
  1.3.1e adds a GWT test for it. Epic 4.2's divergence (`document_symbol` returns whatever
  extracts cleanly instead of skipping the whole file) is justified with real reasoning, not
  just asserted: a single open file with one syntax typo shouldn't blank the whole editor
  Outline panel, versus `build_model`'s whole-repo-export correctness bar — and the note
  explicitly scopes the divergence to that one handler (no `PruningSummary` to record into).
- [x] **Epic 4.3 synchronous inline build** — resolved by a genuine redesign, verified airtight.
  Story 4.3.0 spawns the whole-repo build via `tokio::task::spawn_blocking` at `initialized()`
  and requires (with a named test) that the handler returns *before* the build completes.
  `did_save` rebuilds are likewise spawned in the background, never inline, and a concurrent
  `symbol` call during a rebuild is required to keep serving the pre-rebuild `Ready` snapshot.
  Story 4.3.1's `symbol` handler only matches on `IndexState` (`Building` → synthetic
  "still indexing" result, `Ready` → filter the snapshot, `Failed` → `Ok(None)`) and Task
  4.3.1a states explicitly it "never builds or blocks itself." No code path in the current
  plan has an LSP request trigger a synchronous whole-repo `build_model` call inline. The
  Dependency Visualization diagram was actually redrawn (not just described in prose) — it now
  shows Epic 4.3.0/4.3.1 as separate boxes under Phase 4 with explicit edges ("needs Epic 1.4's
  cache", "reads Epic 4.3.0's IndexState; never builds inline itself").

## Concerns

All five previously-open concerns are resolved.

- [x] **Cache shape ambiguity** — resolved consistently everywhere it's referenced (Pattern
  Decisions table, Story 1.4.1, Task 3.1.1d, Story 4.3.0, and ADR-002 itself): single-slot
  `Mutex<Option<(ModelCacheKey, CachedModel)>>`, keyed by `(repo_root, include_private)` only.
  The "thrashes on every scope change" failure mode the original concern raised is designed
  away, not just accepted — `scope`/`level` were removed from the key entirely and are applied
  via `.filtered()` on the cached unscoped model after a hit, so varying `scope` never evicts
  the cache. ADR-002 was updated in lockstep with the plan (checked directly) — no more
  sketch/plan divergence.
- [x] **C4 diagram notation inconsistency** — resolved. Pattern Decisions table and Task 2.2.1c
  now consistently specify `graph TD`/`subgraph`, explicitly **not** real Mermaid
  `C4Component`/`C4Dynamic` notation, with the GitHub-rendering rationale stated inline (GitHub's
  built-in Mermaid renderer doesn't support the C4 extension, which would defeat the diagram's
  PR-paste purpose). No remaining reference in the plan calls this "C4Component notation."
- [x] **No performance target** — a "Performance Target" section now exists (<5s, "well under
  5 seconds", on kibitzer's own repo) plus Task 1.3.1f's benchmark. The target itself is not
  unrealistic — tree-sitter parsing plus a shallow symbol walk over a few dozen files is
  routinely sub-second, so 5s is generous, not tight. See Minors below for a factual accuracy
  issue in how this section states its comparison basis.
- [x] **Missing Python `SKIP_DIRS` task** — resolved. Task 5.1.2e now exists, extends
  `SKIP_DIRS` with `__pycache__`/`.venv`/`venv`/`.tox`, and is reflected in the Summary of
  new/changed files table.
- [x] **No MVP/fallback cut point** — resolved. The new "MVP Cut Point" section states Phases
  1–4 are independently shippable (every requirements.md success metric is satisfied without
  Phase 5) and Phase 5 is the first and only scope to cut under appetite pressure, backed by
  the Dependency Visualization diagram already showing Phase 5 depends only on Phase 1.

## Minors

- (repair-introduced) The new "Performance Target" section states kibitzer's own repo has
  "~90 source files across Phase 1's Go/TS/Tsx/JS languages." Directly counted
  (`find . -name '*.go' -o -name '*.ts' -o -name '*.tsx' -o -name '*.js'`, excluding
  `.git`/`target`/`node_modules`): **29 files**, not ~90. This doesn't threaten the <5s target's
  feasibility (fewer files only makes it easier), but the stated basis for Task 1.3.1f's
  benchmark, and the claimed parity with `architecture_assessment`'s "same repo" comparison, is
  factually wrong as written — worth a one-line correction before the benchmark task is
  implemented against a false expectation of scale.
- (repair-introduced) Story 4.3.0's `did_save` handling is only specified for when
  `index_state == Ready`. It doesn't say what happens if `did_save` fires while
  `index_state == Building` (the initial background build is still in flight, e.g. an editor
  auto-save landing in the first few seconds after connect). If the in-flight build already
  read a stale copy of that file before the save landed, and the save-triggered rebuild path
  only fires from `Ready`, the just-transitioned-to-`Ready` index could miss that edit until
  the *next* save. Narrow window given the <5s target, but the state machine doesn't name the
  case.
- No acceptance criteria/tests exist for a zero-supported-language repo via the MCP tools or
  LSP handlers — only CLI export (Story 2.1.1) is explicitly tested for this case. Still
  unaddressed after the repair (verified via grep — "zero-supported-languages" only appears in
  the CLI's Task 2.1.1b).
- The deferred barrel-file (`index.ts`) re-export resolution issue (Unresolved Questions) is
  still reasonable to defer at package-edge granularity, but the repair didn't add anything
  addressing the noted nuance: a `SymbolNode`'s `file`/`line` pointing at a re-exporting barrel
  rather than the defining file is a more visible, user-facing wrong answer now that
  symbol-level data is exposed than the pre-existing package-edge case was.
- requirements.md's Rabbit Holes section names "single-method interfaces" as an open
  minimization question; the plan still resolves private/public and generated-code pruning
  concretely but never mentions single-method interfaces — not scoped out explicitly, just
  silently absent from `PruneConfig`, unchanged by the repair.
