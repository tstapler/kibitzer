# Implementation Plan: affected-tests

**Feature**: `kibitzer architecture affected --base <sha>` — a new CLI subcommand that
diffs the working tree against a base ref, walks `ArchModel.import_edges` in reverse
from the changed files' packages, and emits either a newline-separated affected-package
list or the `__ALL__` bail-out sentinel, so stapler-squad (and any kibitzer-adopting
repo) can replace `scripts/test-affected.py`.
**Date**: 2026-09-22
**Status**: Ready for implementation
**ADRs**: [ADR-001](../decisions/ADR-001-single-snapshot-conservative-bailout.md) — single
working-tree `ArchModel` snapshot with conservative bail-out on unresolvable
deletes/renames, rather than a base+HEAD two-sided snapshot.

---

## Step 0.5 — Alternatives considered (creative pass)

Three high-level shapes for the whole feature were compared before committing:

1. **Build on `ArchModel`/`import_graph.rs` + hand-rolled reverse BFS** (chosen).
   Strength: reuses kibitzer's one existing shared dependency-graph representation —
   zero new dependencies, stays inside the "one model, many views" convention every
   research doc converged on independently (stack.md, architecture.md, build-vs-buy.md).
   Weakness: reverse-reachability and base-ref diffing are both genuinely new code
   (no existing helper to extend), so there's no shortcut to "just call an existing
   function."
2. **Port `test-affected.py`'s `go list -json -test ./...` forward-scan approach**
   (checking each package's own declared deps against the changed set) as a new
   Go-specific module, bypassing `import_graph.rs` entirely.
   Strength: it's the proven, already-running reference implementation — lowest risk
   of behavioral surprises relative to what stapler-squad's CI already does today.
   Weakness: duplicates a second, Go-only dependency-walk mechanism alongside the
   general cross-language one this repo is otherwise consolidating everything into
   (`import_graph.rs` already extracts Go/TS/JS/Java/Kotlin/Python) — exactly the
   fragmentation `requirements.md`'s Constraints section rules out.
3. **Shell out to `digitalocean/gta`** (or a similar external tool) as a wrapped
   subprocess, matching kibitzer's plugin-checker precedent.
   Strength: avoids reimplementing graph-walk logic at all.
   Weakness: `affected` isn't a per-file `Check` — there's no `Check.command`/SARIF
   dispatch shape it fits — and pulling in an external Go binary a user must install
   separately doesn't reduce kibitzer's own maintenance surface the way reusing
   `import_graph.rs` does; rejected in `requirements.md`'s own Alternatives Considered
   section already.

Approach 1 was chosen; approaches 2 and 3 are recorded above and in the Pattern
Decisions table below with their specific rejected sub-choices.

---

## Domain Glossary

| Term | Definition | Notes |
|------|-----------|-------|
| `AffectedConfig` | New `Config.affected` nested struct (`src/config.rs`) holding `extra_bail_out_globs: Vec<String>` — repo-specific blast-radius globs *added* to the hardcoded `default_bail_out_globs()`, never replacing them. | Mirrors `Config.architecture: ArchitectureConfig` (`src/config.rs:375`) for nesting shape; the additive-only merge itself mirrors this repo's own `.claude/inspect.json`-overlays-defaults convention (`merge_checks`, `src/config.rs:767-782`), not `Check.scope`'s replace-on-set `Vec<String>` semantics. |
| `extra_bail_out_globs` | The `Vec<String>` field inside `AffectedConfig`; unioned with `default_bail_out_globs()` by `AffectedConfig::effective_bail_out_globs()`, then matched against changed-file paths via the existing `glob::matches_scope`. | Renamed from an earlier `bail_out_globs` design that replaced the defaults outright — rejected per the architecture review's Blocker: a maintainer adding `proto/**` for their own repo must not silently lose `go.mod`/`go.sum` protection they never re-typed. |
| `effective_bail_out_globs` | `AffectedConfig` method: `default_bail_out_globs()` chained with `self.extra_bail_out_globs`, deduplicated. This, not the raw `extra_bail_out_globs` field, is what `compute_affected` actually checks changed files against. | New in `src/config.rs`, alongside `AffectedConfig`. |
| `ChangeStatus` | Enum describing how one path changed between `base` and the working tree: `Added`, `Modified`, `Deleted`, `Renamed { old: PathBuf }`. | Parsed from `git diff --name-status -M`'s status codes (`A`, `M`, `D`, `R###`). |
| `ChangedFile` | One diff record: `{ path: PathBuf, status: ChangeStatus }`. | Produced by `diff_changed_files`. |
| `resolve_base_sha` | Helper: runs `git rev-parse <base>` once, up front, pinning `--base` to a concrete SHA before any other git call. Returns `Result<String>` — an unresolvable `--base` (typo, unfetched branch) is a **hard error** (`Err`), not a bail-out; see Story 1.1.1. | Guards against a moving branch name mid-run (`research/pitfalls.md` §3). |
| `diff_changed_files` | Helper: shells out to `git diff --name-status -M <base_sha>...HEAD`, unions `git diff --name-status HEAD` (uncommitted tracked edits) and `git ls-files --others --exclude-standard` (untracked new files), matching `test-affected.py`'s working-tree-inclusive scope. | New code — no existing kibitzer helper diffs against an arbitrary ref (`research/stack.md` §1, `research/pitfalls.md` §3). |
| `ReverseAdjacency` | `BTreeMap<String, Vec<String>>` mapping a package key `to` every package key `from` that imports it — the inverted view of `ArchModel.import_edges`. | Built once per invocation by `build_reverse_adjacency`; mirrors `mcp.rs`'s `build_call_adjacency` shape (`src/mcp.rs:314-341`) applied to import edges instead of call edges. |
| `resolve_removed_path` | Helper: for a `Deleted` path or a `Renamed` entry's old-side path, checks whether that path's directory still has surviving files present in the working-tree `ArchModel.packages`. Returns the surviving package key, or signals the package was fully removed. | See ADR-001. |
| `AffectedResult` | Sum type returned by `compute_affected`: `Packages(Vec<String>)` or `BailOut(BailOutReason)`. | A sealed enum, not a struct with an `Option`/`bool` pair — makes "both a list and a bail-out reason" or "neither" unrepresentable (Type-driven design). An unresolvable `--base` ref is neither of these — it's a hard `Err`, since no diff was ever computed (see Story 1.1.1). |
| `BailOutReason` | Enum naming *why* a bail-out fired: `GlobMatched { glob: String, path: PathBuf }`, `PackageFullyRemoved { path: PathBuf }`, `PackageClauseChanged { path: PathBuf }` (Story 2.2.3 — a `Modified` file's diff hunk changed its `package` line without a `git mv`), `ShallowCloneOrNoCommonHistory`. Does **not** include an "unresolvable base ref" variant — that case is a hard error (`compute_affected` returns `Err`, not `Ok(BailOut(_))`), per Story 1.1.1. | Printed to stderr alongside the `__ALL__` sentinel on stdout — a debuggability win flagged in `research/features.md` §6 as not costing anything extra to add. |
| `affected::compute_affected` | Pure(-ish) library function: `compute_affected(repo_root: &Path, base: &str, config: &AffectedConfig) -> Result<AffectedResult>`. Orchestrates diffing, resolution, bail-out checks, and the reverse BFS. | New module `src/affected.rs`. |
| `run_affected` | Thin CLI handler in `src/main.rs`, alongside `run_change_coupling`/`run_hotspots`. Calls `compute_affected`, formats stdout, always `ExitCode::SUCCESS` on a successful computation (list or bail-out are both "successful answers"); a genuine `Err` (bad repo, git failure other than an unresolvable base ref) propagates as a non-zero exit with a stderr message. | Deliberately deviates from the `ArchitectureAction` family's "always exit 0, report don't gate" framing only in that its stdout is load-bearing for a CI consumer — the exit-code contract itself (0 on success incl. bail-out, nonzero only on hard error) does not change. |
| `ArchitectureAction::Affected` | New `clap` subcommand variant: `kibitzer architecture affected --path <dir> --base <ref>`. | Grouped with `Export`/`Diagram` in the enum (both are `ArchModel` consumers), per `research/architecture.md` §3. |
| `reverse_bfs` | Helper: given a `ReverseAdjacency` and a seed `BTreeSet<String>`, does a visited-set-guarded BFS outward, returning the full transitive closure. | Must carry an explicit visited set from day one — `find_cycles`/Tarjan SCC (`src/architecture_checks.rs:423-488`) proves this graph has no acyclicity invariant to lean on (`research/pitfalls.md` §2). |

---

## Pattern Decisions

| Component | Pattern Chosen | Source | Alternative Rejected | Reason |
|-----------|---------------|--------|---------------------|--------|
| Git diffing (`diff_changed_files`, `resolve_base_sha`) | Transaction Script — a plain procedural shell-out-and-parse function, no domain object | PoEAA (Fowler) | Wrapping git access behind a `GitRepository`/`DiffSource` trait or Repository-pattern abstraction | Two call sites, no test double is needed (tests spin up a real git repo in a tempdir, matching `change_coupling.rs`'s own test convention); an abstraction layer here would be speculative generality with no second implementation ever planned. |
| Reverse-reachability traversal | Hand-rolled BFS over a `BTreeMap`-based `ReverseAdjacency`, consuming `ArchModel.import_edges` directly | Build-vs-buy research (`research/build-vs-buy.md` §1); matches `mcp.rs`'s `build_call_adjacency`/`traverse_call_edges` precedent | `petgraph::Graph` | Would require converting `ImportGraph`'s `BTreeMap`/`Vec<ImportEdge>` into a second, parallel graph representation — exactly the fragmentation `requirements.md`'s Constraints section forbids — for a problem size (hundreds to low-thousands of package nodes, plain reachability, no weights) that doesn't need petgraph's algorithms. |
| Per-language traversal dispatch | None — a single Go-only code path, no dispatch layer | N/A | GoF Strategy pattern (one `AffectedLanguageStrategy` per language) | Only one language (Go) exists in scope; a Strategy interface with a single implementation is indirection with no current second case to justify it. Revisit when JS/TS support is added (Out of Scope here) — `import_graph.rs`'s existing per-language dispatch inside `build()` is the right seam to extend then, not a new layer on top of `affected`. |
| `AffectedResult` / `BailOutReason` | Sum type (Rust `enum`), not a struct with `Option<Vec<String>>` + `bool bail_out` | Type-driven design | A single struct `{ packages: Vec<String>, bail_out: bool, reason: Option<String> }` | The struct shape allows illegal states (`bail_out: true` with a non-empty `packages`, or `bail_out: false` with a `reason` set) that a caller must remember to treat as impossible; the enum makes them unrepresentable at compile time. |
| Changed-file → package resolution for deletes/renames (`resolve_removed_path`) | Conservative bail-out on "package fully removed," single working-tree snapshot | ADR-001 | Two-sided (`base` + `HEAD`) snapshot reachability union, using `check.rs`'s `git archive`-into-temp-dir pattern | Doubles the per-invocation build cost (a second full repo walk + tree-sitter parse), in tension with the NFR that `affected` be no slower than `test-affected.py`'s `go list -deps` walk; `requirements.md`'s Open Questions section explicitly sanctions a conservative bail-out as an acceptable concrete v1 answer. |
| Bail-out glob matching | Reuse `glob::matches_scope` directly against each `ChangedFile.path` | Existing repo convention (`src/glob.rs:42`), reused by `Check.scope` | A bespoke `BailOutMatcher` type wrapping/duplicating glob logic | `matches_scope` already has exactly the right signature (`rel_path: &str, scopes: &[String]) -> bool`) and semantics (gitignore-style, `!`-negation supported); wrapping it adds a layer with no behavior difference. |
| `Config.affected` config surface | Nested Value Object, `pub affected: AffectedConfig`, `#[serde(default)]` | `research/architecture.md` §4, mirroring `Config.architecture: ArchitectureConfig` (`src/config.rs:375`) | Reusing `Check.scope`'s `Vec<String>` directly as a top-level `Config` field, or bolting `bail_out_globs` onto `ArchitectureConfig` | `Check.scope` scopes *checks* to files, a different concern from `affected`'s own blast-radius policy; `ArchitectureConfig` is about layering/component/naming rules, not this subcommand — a same-shaped-but-separate nested struct keeps the concerns apart the way `architecture`/`checks`/`disabled` already sit side by side in `Config`. |
| Bail-out glob merge semantics | Additive union: `AffectedConfig.extra_bail_out_globs` is *appended* to the hardcoded `default_bail_out_globs()` via `effective_bail_out_globs()`, never substituted for it | This repo's own `.claude/inspect.json`-overlays-defaults convention (`CLAUDE.md`'s "Default check catalog" section), mirroring `merge_checks`'s local-overlays-defaults shape (`src/config.rs:767-782`) | A plain `bail_out_globs: Vec<String>` field using ordinary `#[serde(default = "...")]` replace-on-set semantics, matching `Check.scope`'s convention | Replace semantics would let a maintainer adding a repo-specific glob (e.g. `proto/**`) silently drop the hardcoded `go.mod`/`go.sum` protection they never re-typed — exactly the "silently under-test" failure mode this feature exists to prevent (architecture review Blocker). `Check.scope` can safely use replace semantics because an empty/narrower scope only changes which files get *linted*; here it would change which files get *silently skipped from the safety net*, a materially worse failure mode. |
| CLI dispatch (`run_affected`) | Pure fn (`compute_affected`) + thin CLI handler (`run_affected`), same split as `run_change_coupling`/`change_coupling::analyze` | PoEAA Transaction Script (the handler), existing repo convention (`src/main.rs:659-679`) | A stateful `AffectedService` object encapsulating repo/config/cache | No cross-call state exists (one-shot CLI invocation); every sibling `ArchitectureAction` handler is a pure fn + thin wrapper, and a service object would be inconsistent ceremony. |

---

## Tech Debt Disposition

| Area | Existing Issue | Disposition | Justification |
|------|----------------|--------------|----------------|
| `arch_model.rs` / `import_graph.rs` | None — `research/architecture.md` §5 found no SOLID/Clean/DDD violation in the sections `affected` touches; size (1969/2723 lines) comes from breadth across languages/features, not tangling. | **Extend as-is.** | `import_edges: Vec<ImportEdge>` is already a stable, multiply-consumed data shape (checkers, `fan_in_out`, the diagram renderer); `affected` adds one more consumer (a reverse-adjacency builder + BFS) and reuses `build_model_from_files`/`collect_repo_files` unchanged, per the "one model, many views" convention this repo's own MEMORY.md names as load-bearing. |
| Go import extraction (`import_graph.rs`'s `find_go_mod_upward`, no `replace`-directive support; no build-tag/`//go:build` awareness) | Pre-existing limitation of `import_graph.rs`, not introduced by this feature — see `research/pitfalls.md` §1. Could cause `affected` to under-detect an importer in a multi-module repo using local `replace` directives, or over-include (safer direction) a build-tag-gated file. | **Accept and document.** `affected` inherits whatever `import_graph.rs` currently produces; fixing the underlying extraction gap is a change to shared graph-building code used by every other consumer (checkers, `Export`, `Diagram`), well outside this project's scope. (Relabeled from an earlier "isolate via seam" disposition per the architecture review: no seam/adapter is actually built anywhere in Phases 1-4, so "accept and document" is the label that matches what the plan does.) | Re-architecting `find_go_mod_upward`/build-tag handling is a separate project with its own blast radius across every `ArchModel` consumer, not something to bundle into `affected`'s own PR. Documented here (and in Unresolved Questions below) so it isn't silently forgotten. Task 4.1.3a additionally tries to sample a `replace`-directive case from real history so this risk gets at least one real-world data point rather than staying purely theoretical. |

---

## Observability Plan
- **Logs**: none beyond kibitzer's existing convention — `anyhow::bail!`-style
  messages (command, exit status, stderr) on genuine git/I-O failures, printed to
  stderr by `main()`'s default `Termination` impl, matching `git_toplevel`
  (`src/hotspots.rs:38-52`). A bail-out additionally prints its `BailOutReason` to
  stderr as a one-line note (stdout stays pure `__ALL__` for the CI consumer).
- **Metrics**: none — single-user local tool / CI-invoked subcommand, no
  metrics/alerting infrastructure exists or is warranted (matches
  `requirements.md`'s Observability Requirements verbatim).
- **Alerts**: none.

## Risk Control
- **Feature flag**: none needed — `affected` is a net-new, opt-in subcommand under
  `kibitzer architecture affected`; nothing in `default_checks()`/hook mode changes,
  so no existing behavior can regress for a caller who never invokes it.
- **Rollback procedure**: `git revert` on kibitzer's side. stapler-squad's own
  cutover (deleting `scripts/test-affected.py`, rewiring `build.yml`) is explicitly
  out of scope for this project and happens only after the Story 4.1.3 comparison
  passes — so there is no production dependency to roll back on this repo's side.
- **Staged rollout**: none — a single-maintainer CLI tool with no user population
  to stage across. The comparison-validation step (Story 4.1.3) is the de facto
  staging gate before any adopting repo (stapler-squad) is asked to switch over.
- **Consumer contract (empty-affected-set)**: `run_affected` intentionally emits
  pure-empty stdout (no sentinel, no trailing newline) when
  `AffectedResult::Packages(vec![])` — matching `test-affected.py`'s existing,
  already-shipped contract exactly (Story 3.2.2). Any CI wrapper consuming this via
  the shell shape `PKGS="$(kibitzer architecture affected ...)"; go test $PKGS`
  **must** guard with `[ -z "$PKGS" ] && exit 0` (or equivalent) before invoking
  `go test $PKGS` — an unquoted, unguarded `$PKGS` word-splits to nothing and
  `go test` silently falls back to testing whatever package `cwd` resolves to,
  masking an intentional "nothing affected" skip as a narrow, misleading pass. This
  is a hard integration requirement for any adopting `build.yml`, not an
  implementation detail internal to kibitzer — see Story 3.2.2's acceptance criteria
  for the exact shell shape this is tested against. This requirement is not left as
  planning-doc-only prose: it is also stated directly in the `Affected` variant's
  clap doc comment (Task 3.2.1a), which is what `--help` renders for anyone about to
  wire this into their own `build.yml`; the runtime stderr note for this exact case
  uses a distinct `AFFECTED:`-prefixed, greppable message (Task 3.2.2a) rather than
  kibitzer's ordinary `[kibitzer] ...` note style, precisely because this is the one
  state where a missing guard is invisible on an otherwise-green CI run; and Story
  4.1.3's `docs/affected-validation.md` records this same scenario so it is
  documented as a concrete comparison case, not only a runtime behavior.

## Unresolved Questions

None blocking implementation. Two known, pre-existing limitations are inherited
(not introduced) from `import_graph.rs` and are explicitly accepted, not fixed,
by this project's Tech Debt Disposition above:
- Go `replace`-directive resolution gap (`import_graph.rs`'s `find_go_mod_upward`)
  can under-detect an importer in a multi-module repo — accepted risk, documented,
  follow-up ticket if it fires in practice (observable via `research/pitfalls.md`'s
  guidance to watch for it during Story 4.1.3's real-diff comparison).
- No build-tag (`//go:build`) awareness — can over-include (safe direction) or, in
  the reverse case, miss a platform-specific edge — accepted risk, same reasoning.
  The same gap also applies to `default_bail_out_globs()` (Task 1.2.2a): a
  `//go:build`-gated file is a content-level signal a path glob can't express, so
  no "build-tag-bearing files" entry exists in the default bail-out list either —
  a documented, accepted gap, not a silent drop of the category
  `requirements.md`'s resolved Open Question named.

The renamed/moved-file question flagged as unresolved in `requirements.md` is
resolved by this plan (ADR-001 + Story 2.2.2): conservative bail-out via
`resolve_removed_path` whenever a package is fully removed by a delete or rename.

---

## Dependency Visualization

```
Phase 1: Foundations
  Epic 1.1 (git diffing)         Epic 1.2 (config surface)
  1.1.1 resolve_base_sha    1.2.1 AffectedConfig + Config.affected field
       |                          |
  1.1.2 diff_changed_files  1.2.2 default bail_out_globs
       |                          |
       +-------------+------------+
                      |
Phase 2: Graph reverse-reachability
  Epic 2.1 (BFS)                 Epic 2.2 (resolution)
  2.1.1 build_reverse_adjacency  2.2.1 resolve_changed_packages (Added/Modified/Renamed-new)
       |                          |
  2.1.2 reverse_bfs (cycle-safe) 2.2.2 resolve_removed_path (Deleted/Renamed-old, bail-out)
       |                          |
       |                    2.2.3 detect_package_clause_change (Modified, bail-out)
       |                          |
       +-------------+------------+
                      |
Phase 3: CLI surface
  Epic 3.1 (orchestration)       Epic 3.2 (CLI wiring)
  3.1.1 AffectedResult/BailOutReason + compute_affected
       |                          |
       +----------------> 3.2.1 ArchitectureAction::Affected + dispatch
                                  |
                           3.2.2 run_affected stdout/exit-code contract
                      |
Phase 4: Validation
  Epic 4.1
  4.1.1 synthetic-graph unit tests (BFS/resolution)
  4.1.2 bail-out trigger tests (each category)
  4.1.3 comparison run vs. test-affected.py on real stapler-squad diffs
```

---

## Phase 1: Foundations

### Epic 1.1: Base-ref diffing
**Goal**: Produce a `Vec<ChangedFile>` for an arbitrary `--base` ref, resolved once to
a concrete SHA, with rename detection and working-tree/untracked inclusion — the
"new plumbing" every research doc confirmed doesn't exist yet.

#### Story 1.1.1: Resolve `--base` to a concrete SHA before any other git call
**As a** CI wrapper invoking `kibitzer architecture affected --base main`, **I want**
the base ref pinned to one SHA up front, **so that** a branch moving mid-run (someone
pushes to `main` while the job runs) can't silently shrink or distort the diff.

**Exit contract for an unresolvable `--base` (reconciled)**: a `--base` ref that
doesn't exist, a `git` binary that isn't installed, or a `repo_root` that isn't a
git working tree at all are all **hard errors**: non-zero exit, a clear message on
stderr, and *nothing* on stdout (not `__ALL__`, not a package list) — because no
diff was ever computed, so neither answer means anything, and a CI wrapper needs to
be able to tell "something is broken" apart from "everything is affected." This is
distinct from a diff that *was* successfully computed but touches an
unbounded-blast-radius file (Story 1.2.2) or can't be safely narrowed for some other
in-diff reason (Story 1.1.2's shallow-clone case) — those remain the `__ALL__`
bail-out, exit 0. An earlier draft of this plan, `research/ux.md`'s own prose (its
recommendation table already agreed with the hard-error framing), and
`requirements.md`'s resolved Open Question disagreed with each other on exactly this
point; all three now state the same hard-error/bail-out split (see this story's
tasks below, and the corresponding edit to `research/ux.md` §3).

**Acceptance Criteria**:
- Given a valid git repo with `base = "main"` resolving to SHA `abc1234`, when
  `resolve_base_sha(repo_root, "main")` runs, then it returns `Ok("abc1234".to_string())`
  and every subsequent git call in the same `compute_affected` invocation uses
  `"abc1234"`, never `"main"` again.
  - *Given* a tempdir git repo with `main` at commit `abc1234`, *When*
    `resolve_base_sha` is called with `base = "main"`, *Then* the returned SHA equals
    the output of `git rev-parse main` captured at call time.
- Given `base` doesn't resolve (typo, unfetched branch, e.g. `"totally-not-a-ref"`),
  when `resolve_base_sha` runs, then it returns `Err` — a **hard error**, not a
  bail-out — since `git rev-parse` failing means no diff can even be attempted; this
  propagates out of `compute_affected` uncaught (via `?`), and `run_affected` exits
  non-zero with the ref name in the stderr message and nothing on stdout.
  - *Given* a tempdir git repo with no ref named `totally-not-a-ref`, *When*
    `resolve_base_sha` is called with that base, *Then* it returns `Err` whose
    message contains `"totally-not-a-ref"`.
  - *Given* a CLI invocation `kibitzer architecture affected --base does-not-exist-ref`
    against a real repo, *When* it runs, *Then* the process exits non-zero, stdout
    capture is exactly `""`, and stderr contains a message naming
    `does-not-exist-ref`.
- Given `repo_root` is not inside a git working tree at all, when `resolve_base_sha`
  (or an earlier `git rev-parse --show-toplevel` check) runs, then `compute_affected`
  returns `Err` (hard failure, exit 1, stderr) — not a bail-out — since there is
  nothing to compute at all, matching `git_toplevel`'s existing pattern
  (`src/hotspots.rs:38-52`).
  - *Given* `repo_root = /tmp/not-a-git-repo` (no `.git`), *When* `compute_affected`
    is called, *Then* it returns `Err` whose message contains `"not a git repository"`
    (or the equivalent `git` stderr), and `run_affected` exits non-zero with nothing
    on stdout.

**Files**: `src/affected.rs` (new)

##### Task 1.1.1a: Add `git_toplevel`-style repo-root check + `resolve_base_sha` (~5 min)
- Create `src/affected.rs` with module doc comment stating scope (Go-only v1,
  batch-only, never wired into `default_checks()`/hook mode — matching the
  `ArchitectureAction` family's documented convention).
- Add `fn repo_toplevel(repo_root: &Path) -> Result<PathBuf>` (reuse the
  `git rev-parse --show-toplevel` shell-out pattern from `src/hotspots.rs:38-52`,
  `bail!` with command+status+stderr on failure).
- Add `fn resolve_base_sha(repo_root: &Path, base: &str) -> Result<String>`: runs
  `git rev-parse <base>`, returns `Ok(sha.trim().to_string())` on success, `bail!`s
  with a message naming `base` and including `git`'s stderr on a non-zero exit, and
  propagates an `Err` on a `Command::new` spawn failure (git binary missing). Unlike
  an earlier draft of this task, a non-zero `git rev-parse` exit is **not** mapped to
  `Ok(None)`/a caller-side bail-out — it's a hard error straight through, per this
  story's reconciled exit contract above.
- Files: `src/affected.rs`

##### Task 1.1.1b: Unit tests for `resolve_base_sha` (~5 min)
- Add `#[cfg(test)] mod tests` to `src/affected.rs` with a tempdir git-repo test
  helper mirroring `change_coupling.rs`'s `commit_touching`-style helper
  (`src/change_coupling.rs:424-432`): init a repo, write+commit a file.
- Test: resolving an existing branch/SHA returns `Ok(_)` matching `git rev-parse`
  directly.
- Test: resolving a nonexistent ref returns `Err` whose message names the ref.
- Files: `src/affected.rs`

#### Story 1.1.2: Compute the changed-file list with rename detection
**As a** the `affected` computation, **I want** every changed path since `base`,
annotated with `Added`/`Modified`/`Deleted`/`Renamed`, including uncommitted and
untracked changes, **so that** a plain rename doesn't look like "delete an entire
package, add a disconnected one."

**Acceptance Criteria**:
- Given a repo where `base_sha` and `HEAD` differ only by `a/foo.go` being renamed
  to `b/foo.go` with no content change, when `diff_changed_files(repo_root, base_sha)`
  runs with `-M` rename detection, then it returns exactly one `ChangedFile { path:
  "b/foo.go", status: Renamed { old: "a/foo.go" } }` — not a `Deleted("a/foo.go")` +
  `Added("b/foo.go")` pair.
  - *Given* a tempdir git repo with commit `abc1234` containing `a/foo.go`, and a
    second commit renaming it to `b/foo.go` with `git mv`, *When*
    `diff_changed_files(repo_root, "abc1234")` runs, *Then* the returned `Vec<ChangedFile>`
    has length 1 and its one entry's `status` is `Renamed { old: PathBuf::from("a/foo.go") }`.
- Given uncommitted tracked edits and an untracked new file exist in the working
  tree in addition to committed changes since `base`, when `diff_changed_files` runs,
  then all three sources are unioned into one `Vec<ChangedFile>` (matching
  `test-affected.py:73-84`'s three-source union), with no duplicate entries for a
  path that appears in more than one source.
  - *Given* commit `abc1234` as base, a committed change to `x/x.go` since then, an
    uncommitted edit to `y/y.go`, and a new untracked file `z/z.go`, *When*
    `diff_changed_files(repo_root, "abc1234")` runs, *Then* the result contains
    exactly one `ChangedFile` each for `x/x.go` (`Modified`), `y/y.go` (`Modified`),
    and `z/z.go` (`Added`).
- Given the repository is shallow-cloned and `base_sha` predates the shallow
  boundary, when `diff_changed_files` runs, then it detects the `git diff`/
  `merge-base` failure and returns a value `compute_affected` maps to
  `AffectedResult::BailOut(BailOutReason::ShallowCloneOrNoCommonHistory)`.
  - *Given* a repo created via `git clone --depth 1` where `base_sha` is not in the
    shallow history, *When* `diff_changed_files` runs, *Then* it returns `Ok(None)`
    — an ambiguous-but-not-erroneous case that resolves toward the `__ALL__`
    bail-out (unlike `resolve_base_sha`'s unresolvable-ref case, which is a hard
    error per Story 1.1.1: here `base_sha` itself resolved fine, it's specifically
    the shared-history computation that's ambiguous) — rather than an `Err` or a
    partial/wrong file list.

**Files**: `src/affected.rs`

##### Task 1.1.2a: Define `ChangeStatus`/`ChangedFile` types (~3 min)
- Add `#[derive(Debug, Clone, PartialEq, Eq)] pub enum ChangeStatus { Added, Modified, Deleted, Renamed { old: PathBuf } }`.
- Add `#[derive(Debug, Clone, PartialEq, Eq)] pub struct ChangedFile { pub path: PathBuf, pub status: ChangeStatus }`.
- Files: `src/affected.rs`

##### Task 1.1.2b: Implement `diff_changed_files` (merge-base + rename detection) (~5 min)
- `fn diff_base_head(repo_root: &Path, base_sha: &str) -> Result<Option<Vec<ChangedFile>>>`:
  runs `git diff --name-status -M <base_sha>...HEAD` (triple-dot, merge-base form,
  matching `test-affected.py`'s own semantics per `research/stack.md` §1); parses
  `A`/`M`/`D`/`R<score>` status-code lines (tab-separated: `R100\told\tnew`); returns
  `Ok(None)` on non-zero exit (ambiguous, e.g. no common history) rather than `Err`.
- Files: `src/affected.rs`

##### Task 1.1.2c: Implement working-tree/untracked union + `diff_changed_files` entry point (~5 min)
- `fn diff_working_tree(repo_root: &Path) -> Result<Vec<ChangedFile>>`: runs
  `git diff --name-status HEAD` (tracked, uncommitted) and
  `git ls-files --others --exclude-standard` (untracked, all `Added`), both parsed
  the same way as Task 1.1.2b.
- `pub fn diff_changed_files(repo_root: &Path, base_sha: &str) -> Result<Option<Vec<ChangedFile>>>`:
  calls `diff_base_head`, returns early with `Ok(None)` if that returns `None`;
  otherwise unions with `diff_working_tree`'s result, deduplicating by `path`
  (a path present in both keeps the base-diff entry, since it carries rename info the
  working-tree diff wouldn't).
- Files: `src/affected.rs`

##### Task 1.1.2d: Unit tests for rename detection + three-source union (~5 min)
- Test: a pure rename (no content change) between two commits yields one `Renamed`
  entry, not delete+add.
- Test: committed + uncommitted + untracked changes union into one deduplicated list.
- Test: a shallow-clone/no-common-history repo (simulate via `git init --depth`-style
  fixture, or a repo with two unrelated root commits and `base_sha` from the other
  history) returns `Ok(None)`.
- Files: `src/affected.rs`

### Epic 1.2: Bail-out configuration surface
**Goal**: A `Config.affected.extra_bail_out_globs` field, mirroring `Config.architecture`
for nesting and this repo's own overlay-not-replace convention for merge semantics,
with sane, portable Go-ecosystem defaults, so a repo's own unbounded-blast-radius
files (lockfiles, codegen inputs) are never silently missed — and a repo-supplied
addition never silently drops one of those defaults either.

#### Story 1.2.1: Add `AffectedConfig` and wire it into `Config`
**As a** kibitzer maintainer configuring a repo's `.claude/inspect.json`, **I want**
an `affected.extra_bail_out_globs` block, **so that** I can extend the default
bail-out list with repo-specific codegen inputs/build config without forking
kibitzer *and without silently losing the hardcoded defaults* — matching this
repo's own `.claude/inspect.json`-overlays-defaults convention (CLAUDE.md's "Default
check catalog" section), not a replace-on-set field.

**Acceptance Criteria**:
- Given no `.claude/inspect.json` overrides `affected` at all, when `Config` is
  deserialized from an empty/minimal JSON object, then
  `config.affected.effective_bail_out_globs()` equals the hardcoded default list
  (Story 1.2.2).
  - *Given* `.claude/inspect.json` containing `{}`, *When* `config::find_config` loads
    it, *Then* `config.affected.effective_bail_out_globs()` contains `"**/go.mod"` and
    `"**/go.sum"`.
- Given `.claude/inspect.json` sets `"affected": {"extra_bail_out_globs": ["proto/**"]}`,
  when `Config` is deserialized, then `config.affected.extra_bail_out_globs ==
  vec!["proto/**"]` **and** `config.affected.effective_bail_out_globs()` contains
  *both* `"proto/**"` **and** every entry of `default_bail_out_globs()` — the
  repo's list is *added to*, never substituted for, the hardcoded default. This is
  the architecture review's Blocker fix: a maintainer adding `proto/**` protection
  must not silently lose `go.mod`/`go.sum` protection they never re-typed.
  - *Given* that JSON, *When* deserialized, *Then*
    `config.affected.effective_bail_out_globs().len() == default_bail_out_globs().len() + 1`
    and the result still contains `"**/go.mod"`.
- Given `kibitzer schema` runs, when the JSON schema is generated, then
  `AffectedConfig`'s shape (including `extra_bail_out_globs`'s doc comment, which
  states its additive-only semantics) appears in the output without a separate
  schema-generation code path to maintain.
  - *Given* `Command::Schema` is invoked, *When* the schema JSON is produced, *Then*
    it contains a `"affected"` property whose schema was derived from `AffectedConfig`'s
    `JsonSchema` derive, not hand-written.

**Files**: `src/config.rs`

##### Task 1.2.1a: Define `AffectedConfig` struct (~3 min)
- Add, near `ArchitectureConfig` (`src/config.rs:165`):
  `#[derive(Debug, Clone, Deserialize, JsonSchema)] pub struct AffectedConfig { #[serde(default)] pub extra_bail_out_globs: Vec<String> }`.
- Add `impl AffectedConfig { pub fn effective_bail_out_globs(&self) -> Vec<String> { default_bail_out_globs().into_iter().chain(self.extra_bail_out_globs.iter().cloned()).collect() } }`
  — this, not the raw field, is what `compute_affected` checks changed files
  against (Task 3.1.1c).
- Doc comment on the field: what the list is for, that it's matched via
  `glob::matches_scope` against every `ChangedFile.path`, and — explicitly, since
  this is the opposite of `Check.scope`'s replace-on-set convention — that entries
  here are *added to* `default_bail_out_globs()`, never replace it; use `disabled`-
  style config (not built here — out of scope) if a default ever needs suppressing.
- Files: `src/config.rs`

##### Task 1.2.1b: Wire `Config.affected` field + `Default` impl (~3 min)
- Add `#[serde(default)] pub affected: AffectedConfig` to `Config` (`src/config.rs:371-381`).
- Add `impl Default for AffectedConfig { fn default() -> Self { Self { extra_bail_out_globs: Vec::new() } } }`
  (needed for the "no `.claude/inspect.json` at all" path in `run_affected`, since
  `Config` itself has no blanket `Default`; `effective_bail_out_globs()` still
  returns the full hardcoded list in this case since it's computed, not stored).
- Files: `src/config.rs`

##### Task 1.2.1c: Unit tests for config deserialization + defaulting (~4 min)
- Test: empty-object JSON deserializes to an empty `extra_bail_out_globs`, and
  `effective_bail_out_globs()` equals `default_bail_out_globs()` exactly.
- Test: an explicit `extra_bail_out_globs` list is *unioned with*, not a replacement
  for, `default_bail_out_globs()` in `effective_bail_out_globs()`'s output.
- Files: `src/config.rs`

#### Story 1.2.2: Ship sane, portable Go-ecosystem default bail-out globs
**As a** a repo adopting `affected` with zero configuration, **I want** the common
unbounded-blast-radius files already covered — anywhere in the tree, not just at
repo root — **so that** I get `test-affected.py`-equivalent safety on day one.

**Acceptance Criteria**:
- Given the hardcoded defaults, when a root-level `go.mod`/`go.sum`/`*.proto` file
  changes, **or the same filename changes in a nested directory** (e.g.
  `tools/scanner/go.mod` in a multi-module repo), then `matches_scope` returns
  `true` against `default_bail_out_globs()`. Every default glob is written with a
  `**/` prefix (`**/go.mod`, `**/go.sum`, `**/*.proto`) specifically so it matches
  at any depth, not just repo root — `src/glob.rs:5-33`'s `glob_to_regex` anchors
  every pattern with `^...$` and has no implicit path-prefix wildcarding, so a bare
  `"go.mod"` compiles to `^go\.mod$` and matches *only* a root-level file. This is
  `glob.rs`'s existing, unchanged matching convention (already relied on by
  `Check.scope` elsewhere in this repo) — not a new footgun `affected` introduces;
  the fix here is simply to write every default pattern in the tree-wide form that
  convention requires. (This is a real, repo-confirmed gap: stapler-squad itself has
  independent `go.mod` files at `tuitest/go.mod`, `tools/scanner/go.mod`, and
  `tools/lint/go.mod` in addition to its root module.)
  - *Given* `ChangedFile { path: "go.sum", .. }`, *When* checked against
    `default_bail_out_globs()` via `glob::matches_scope`, *Then* the result is `true`.
  - *Given* `ChangedFile { path: "tools/scanner/go.mod", .. }` (a nested module, not
    at repo root), *When* checked the same way, *Then* the result is `true`.
  - *Given* `ChangedFile { path: "api/v1/service.proto", .. }`, *When* checked the
    same way, *Then* the result is `true` — restoring the `**/*.proto` entry
    `requirements.md`'s own resolved Open Question named explicitly (codegen inputs
    like `.pb.go` are typically gitignored and invisible to a plain `.go`-file diff,
    so a proto-only change must still trigger `__ALL__`).
- Given a changed file with no special meaning (e.g. `internal/widget/widget.go`),
  when checked against `default_bail_out_globs()`, then the result is `false` — the
  default list must not be so broad it defeats the feature's purpose.
  - *Given* `ChangedFile { path: "internal/widget/widget.go", .. }`, *When* checked,
    *Then* the result is `false`.
- Given the default list ships in *any* adopting repo (not just kibitzer's own),
  when inspected, then it contains no kibitzer-repo-specific paths (`src/affected.rs`
  etc.) — those would be inert, always-`false`-matching entries in every other
  adopter's tree, exactly the kind of non-portable, repo-specific entry Task 1.2.2a
  already argues against for stapler-squad's `Makefile`/`.golangci.yml`. Kibitzer's
  own self-protection for `src/affected.rs`/`src/import_graph.rs`/`src/arch_model.rs`
  is instead this repo's *own* dogfooding config, added via `extra_bail_out_globs`
  in kibitzer's own `.claude/inspect.json` (Story 1.2.1's overlay), not baked into
  the shipped default.
  - *Given* `default_bail_out_globs()`'s output, *When* inspected, *Then* it
    contains no entry naming `src/affected.rs`, `src/import_graph.rs`, or
    `src/arch_model.rs`.

**Files**: `src/config.rs`

##### Task 1.2.2a: Implement `default_bail_out_globs()` (~3 min)
- `fn default_bail_out_globs() -> Vec<String> { vec!["**/go.mod".into(), "**/go.sum".into(), "**/*.proto".into()] }`
  — a short, documented, easily extended, *portable* default (no kibitzer-specific
  or stapler-squad-specific paths — see the acceptance criteria above and the
  architecture review's Concern) per `research/features.md` §5's synthesis.
- Doc comment above the fn covers three things:
  1. Why each entry is there (module graph blast radius, codegen-input blast
     radius) and why every entry uses a `**/` prefix rather than a bare filename —
     `src/glob.rs`'s `glob_to_regex` anchors patterns with `^...$` and does not
     implicitly match at any depth, so a bare `"go.mod"` would only ever match a
     root-level file (a real gap in a multi-module repo, confirmed against
     stapler-squad's own `tools/scanner/go.mod`). This is `glob.rs`'s existing
     convention across every consumer, not something new introduced here.
  2. That kibitzer's own affected-logic source files are deliberately *not* in this
     shipped default (they'd be inert noise for every other adopter) — kibitzer
     dogfoods that protection via its own `.claude/inspect.json`'s
     `affected.extra_bail_out_globs`, per Story 1.2.1's overlay.
  3. That "build-tag-bearing files" (`//go:build` directives) are a documented,
     accepted gap, not silently dropped: a `//go:build` constraint is a *content*
     signal inside a file, not something a path glob can express (unlike
     `go.mod`/`.proto`, there's no filename convention reliable enough to glob-match
     — plenty of ordinary, non-build-tagged `.go` files also match `*_linux.go`-style
     suffixes). This is the same category of gap already accepted for
     `import_graph.rs`'s lack of build-tag awareness in the Tech Debt Disposition
     table above; detecting it would require parsing file contents during the
     bail-out check, which the design deliberately keeps path-only for performance
     (Non-functional Requirements) and simplicity. Also noted in Unresolved
     Questions below.
- Files: `src/config.rs`

##### Task 1.2.2b: Unit tests for default-glob coverage and non-over-broadness (~4 min)
- Table-driven test: each default-list entry matches its representative path at
  repo root *and* at a nested depth (e.g. both `go.mod` and `tools/scanner/go.mod`
  match `**/go.mod`); an unrelated `.go` file path does not match; no entry names a
  kibitzer-specific source path.
- Files: `src/config.rs`

---

## Phase 2: Graph reverse-reachability

### Epic 2.1: Reverse adjacency + BFS over `ArchModel.import_edges`
**Goal**: Given a seed set of changed package keys, compute every package that
transitively imports them — the "invert `edges_from`" operation no existing kibitzer
code performs (`research/stack.md` §4, `research/architecture.md` §1).

#### Story 2.1.1: Build a reverse-adjacency index from `ArchModel.import_edges`
**As a** the `affected` computation, **I want** a `to -> Vec<from>` map built once per
invocation, **so that** the BFS doesn't do an `O(edges)` linear scan per visited node.

**Acceptance Criteria**:
- Given `ArchModel.import_edges` contains `[ImportEdge{from:"example.com/app/a",
  to:"example.com/app/b",..}, ImportEdge{from:"example.com/app/c", to:"example.com/app/b",..}]`,
  when `build_reverse_adjacency(&edges)` runs, then the resulting map has key
  `"example.com/app/b"` mapping to a `Vec` containing both `"example.com/app/a"` and
  `"example.com/app/c"` (order not asserted; a `BTreeSet`/sorted-`Vec` avoids
  nondeterministic test flakiness).
  - *Given* those two edges, *When* `build_reverse_adjacency` runs, *Then*
    `map.get("example.com/app/b")` contains exactly `{"example.com/app/a", "example.com/app/c"}`.
- Given a package with no importers at all (e.g. `"example.com/app/d"`, a leaf node
  in the `to` position of no edge), when queried, then `map.get("example.com/app/d")`
  returns `None`, not an empty `Vec` — `reverse_bfs` must treat a missing key the same
  as an empty importer set.

**Files**: `src/affected.rs`

##### Task 2.1.1a: Implement `build_reverse_adjacency` (~4 min)
- `fn build_reverse_adjacency<'a>(edges: &'a [ImportEdge]) -> BTreeMap<&'a str, BTreeSet<&'a str>>`:
  groups `edges` by `to`, collecting `from` values into a `BTreeSet` (dedupes
  multi-edge pairs — e.g. two import statements in the same package importing the
  same dependency — and gives deterministic iteration order for tests).
- Files: `src/affected.rs`

##### Task 2.1.1b: Unit tests for reverse-adjacency construction (~4 min)
- Test: multiple importers of one package all appear.
- Test: a package with zero importers is absent from the map (not present with an
  empty value).
- Test: two edges with the same `(from, to)` pair (duplicate import statements)
  produce one entry, not two.
- Files: `src/affected.rs`

#### Story 2.1.2: Cycle-safe reverse BFS from a seed set
**As a** the `affected` computation, **I want** the full transitive closure of
importers from a seed set, **so that** an indirect dependent three hops away is still
included, without infinite-looping on a real import cycle.

**Acceptance Criteria**:
- Given the two-package cycle fixture from `import_graph.rs`'s own test
  (`example.com/app/a` imports `example.com/app/b` and vice versa,
  `src/import_graph.rs:1477-1499`), when `reverse_bfs` is seeded with
  `{"example.com/app/a"}`, then it terminates and returns
  `{"example.com/app/a", "example.com/app/b"}` — both packages, no infinite loop.
  - *Given* that reverse-adjacency map (`a -> {b}`, `b -> {a}` after inversion), *When*
    `reverse_bfs(&adjacency, &BTreeSet::from(["example.com/app/a".to_string()]))` runs,
    *Then* it returns within a bounded number of steps and the result set has exactly
    2 elements.
- Given a linear chain `a` imports `b` imports `c` imports `d` (so reverse adjacency
  is `b->{a}, c->{b}, d->{c}`), when `reverse_bfs` is seeded with `{"d"}`, then the
  result is `{"a", "b", "c", "d"}` — full transitive closure, not just direct
  importers.
  - *Given* that chain's reverse adjacency, *When* `reverse_bfs` is seeded with
    `{"d"}`, *Then* the result set equals `{"a","b","c","d"}`.
- Given a seed package with no entry in the reverse-adjacency map (no importers),
  when `reverse_bfs` is seeded with just that package, then the result is exactly
  `{that package}` (the seed itself is always included — a changed package is always
  "affected," even if nothing imports it, since its own tests should still run).

**Files**: `src/affected.rs`

##### Task 2.1.2a: Implement `reverse_bfs` with explicit visited set (~5 min)
- `fn reverse_bfs(adjacency: &BTreeMap<&str, BTreeSet<&str>>, seeds: &BTreeSet<String>) -> BTreeSet<String>`:
  `VecDeque`-based BFS, `visited` seeded with `seeds.clone()`, pushes each unvisited
  importer found via `adjacency.get(node.as_str())`, returns `visited` once the queue
  drains. No depth limit (full transitive closure, not `--depth`-bounded like
  `mcp.rs`'s `traverse_call_edges`).
- Files: `src/affected.rs`

##### Task 2.1.2b: Unit tests: cycle termination, transitive chain, no-importers case (~5 min)
- The three Given/When/Then cases above, plus a disconnected-graph case (a seed with
  importers only in a component unrelated to another seed — confirms no
  cross-contamination between independent seed subgraphs, though the union is still
  computed correctly).
- Files: `src/affected.rs`

### Epic 2.2: Changed-file → package resolution
**Goal**: Map each `ChangedFile` to a seed package key, handling the delete/rename
cases via ADR-001's conservative-bail-out rule.

#### Story 2.2.1: Resolve Added/Modified/Renamed(new-side) files to package keys
**As a** the `affected` computation, **I want** every changed file that still exists
on disk mapped to its package key via the working-tree `ArchModel`, **so that** the
BFS has a correct seed set for the common case.

**Acceptance Criteria**:
- Given a working-tree `ArchModel` with `file_packages` containing
  `{"a/a.go": "example.com/app/a"}`, and a `ChangedFile { path: "a/a.go", status:
  Modified }`, when resolved, then the seed set gains `"example.com/app/a"`.
  - *Given* that model and changed file, *When* `resolve_changed_packages` runs,
    *Then* the returned seed set contains `"example.com/app/a"`.
- Given a `Renamed { old: "a/foo.go" }` entry with new path `"b/foo.go"`, and
  `file_packages` containing `{"b/foo.go": "example.com/app/b"}` (post-rename tree),
  when resolved, then the seed set gains `"example.com/app/b"` via the new-side path
  — the old-side path is handled separately by Story 2.2.2, not here.
  - *Given* that model and changed file, *When* `resolve_changed_packages` runs,
    *Then* the returned seed set contains `"example.com/app/b"`.
- Given a changed file with no entry in `file_packages` at all (unsupported
  extension — e.g. a changed `README.md`, not `Language::for_path`-recognized — or a
  path under a `SKIP_DIRS`-excluded directory such as `vendor/`, per
  `src/check.rs:1483-1501`, which `collect_repo_files`'s underlying
  `walk_and_collect_files` already excludes from the model), when resolved, then it
  contributes nothing to the seed set and is silently skipped (not an error, not a
  bail-out) — matching the "binaries/non-source files don't carry import edges"
  reasoning in `research/pitfalls.md` §1, and resolving that same doc's vendor-path
  concern as a free side effect of resolving against `ArchModel.packages` (which is
  already `SKIP_DIRS`-filtered) rather than the raw diff output.
  - *Given* `ChangedFile { path: "README.md", status: Modified }` with no
    `file_packages` entry for it, *When* resolved, *Then* the seed set is unaffected
    by this entry.
  - *Given* `ChangedFile { path: "vendor/github.com/foo/bar/bar.go", status: Modified }`
    (a vendored dependency, excluded from the working-tree `ArchModel` by
    `SKIP_DIRS`), *When* resolved, *Then* the seed set is unaffected by this entry —
    a vendor-only diff neither spuriously affects a package nor triggers a bail-out.

**Files**: `src/affected.rs`

##### Task 2.2.1a: Implement `resolve_changed_packages` for Added/Modified/Renamed-new (~5 min)
- `fn resolve_changed_packages(changed: &[ChangedFile], model: &ArchModel) -> BTreeSet<String>`:
  for `Added`/`Modified`/`Renamed{..}` entries, calls `model.package_for_file(&entry.path)`
  (Task 2.2.1b), inserts the found key if present, skips silently if absent.
  `Deleted` entries and `Renamed{old,..}`'s old-side path are **not** handled here —
  left for Story 2.2.2's `resolve_removed_path`, called separately by `compute_affected`.
- Files: `src/affected.rs`

##### Task 2.2.1b: Expose a `PathBuf -> package key` query method on `ArchModel` (~4 min)
- `ArchModel` today only exposes package grouping via `PackageNode.files` (per
  `research/stack.md` §2; `import_graph.rs`'s `file_packages` is the single source
  of truth `arch_model.rs`'s grouping is built from, `src/arch_model.rs:240-254`,
  but that raw map isn't itself a field on `ArchModel`). Rather than building a
  second, private `BTreeMap<&Path, &str>` inside `affected.rs` derived from
  `ArchModel.packages` — a small parallel index that must stay consistent with
  `ArchModel`'s own internal grouping, the same "second representation" shape this
  project's Constraints otherwise rule out, just at smaller scale (architecture
  review Concern) — add `pub fn package_for_file(&self, path: &Path) -> Option<&str>`
  to `ArchModel` (`src/arch_model.rs`, near `pub fn package`, `src/arch_model.rs:630`):
  a generically useful query on the aggregate root (scans `self.packages`' `PackageNode.files`
  once, or is backed by a lazily-built internal index if `PackageNode.files` lookup
  needs to be faster than linear) that `affected.rs` calls directly instead of
  duplicating the grouping logic. This is a small, additive query method on the
  existing shared model, not a reshaping of `ArchModel`'s data — consistent with the
  Tech Debt Disposition's "extend as-is" stance for the rest of `arch_model.rs`.
- Files: `src/arch_model.rs`, `src/affected.rs`

##### Task 2.2.1c: Unit tests for Story 2.2.1 (~5 min)
- The three Given/When/Then cases above, using the `example.com/app` fixture
  convention from `import_graph.rs`'s own tests.
- Files: `src/affected.rs`

#### Story 2.2.2: Conservative bail-out for fully-removed packages (deletes + rename old-side)
**As a** the `affected` computation, **I want** to bail out to `__ALL__` whenever a
delete or rename removes the last file of a package, **so that** it never guesses
whether that vanished package's former importers are still safe (ADR-001).

**Acceptance Criteria**:
- Given a `Deleted { path: "a/only.go" }` entry where directory `a/` has no other
  recognized-language files remaining in the working-tree `ArchModel.packages` (the
  package `example.com/app/a` no longer exists at all post-delete), when resolved,
  then `resolve_removed_path` returns a value signaling "fully removed," and
  `compute_affected` returns `AffectedResult::BailOut(BailOutReason::PackageFullyRemoved
  { path: "a/only.go" })` — no further BFS is attempted.
  - *Given* a working tree where `a/only.go` was just deleted and `a/` is now empty
    of `.go` files, *When* `compute_affected` runs, *Then* it returns
    `AffectedResult::BailOut(BailOutReason::PackageFullyRemoved { path: PathBuf::from("a/only.go") })`.
- Given a `Deleted { path: "a/one_of_two.go" }` entry where `a/` still has
  `a/other.go` present in the working-tree `ArchModel.packages` (the package
  `example.com/app/a` still exists, just with fewer files), when resolved, then
  `resolve_removed_path` returns `Some("example.com/app/a")`, which is added to the
  seed set as a changed package — no bail-out, since the package's continued
  existence is directly observable in the current tree.
  - *Given* that working tree, *When* `compute_affected` runs, *Then* the returned
    `AffectedResult::Packages(..)` (assuming no other bail-out condition fires)
    includes `"example.com/app/a"` in its seed computation (verifiable by asserting
    it or one of its importers appears in the final list).
- Given a `Renamed { old: "a/foo.go" }` entry where `a/` has no other files
  remaining, when resolved, then it bails out identically to the pure-delete case
  above (a cross-directory rename that empties the source package is treated the
  same as a delete for the old side).
  - *Given* a rename of the only file in `a/` to `b/foo.go`, *When* `compute_affected`
    runs, *Then* it returns `AffectedResult::BailOut(BailOutReason::PackageFullyRemoved
    { path: PathBuf::from("a/foo.go") })`, regardless of `b/foo.go` resolving cleanly
    via Story 2.2.1.

**Files**: `src/affected.rs`

##### Task 2.2.2a: Implement `resolve_removed_path` (~5 min)
- `fn resolve_removed_path(old_path: &Path, model: &ArchModel) -> Option<String>`:
  looks at `old_path.parent()`, checks whether any `PackageNode.files` entry in
  `model.packages` still has a file under that same parent directory; if so, returns
  `Some(that PackageNode's key)`; if the directory has zero surviving recognized
  files, returns `None` (fully removed — caller bails out).
- Files: `src/affected.rs`

##### Task 2.2.2b: Wire Deleted/Renamed-old handling into `compute_affected`'s resolution step (~4 min)
- In `compute_affected` (Task 3.1.1a), for every `Deleted` entry and every
  `Renamed{old,..}` entry's `old` path, call `resolve_removed_path`; on `None`,
  short-circuit the whole function with
  `Ok(AffectedResult::BailOut(BailOutReason::PackageFullyRemoved{path}))` before any
  BFS work; on `Some(key)`, add `key` to the seed set alongside Story 2.2.1's results.
- Files: `src/affected.rs`

##### Task 2.2.2c: Unit tests for Story 2.2.2 (~5 min)
- The three Given/When/Then cases above.
- Files: `src/affected.rs`

#### Story 2.2.3: Bail out when a `Modified` file's diff hunk changes its `package` clause
**As a** the `affected` computation, **I want** a `Modified` Go file whose diff hunk
touches its own `package` declaration line treated as untrustworthy for direct
resolution, **so that** a manual package-boundary refactor done without `git mv` (an
edited `package` clause, not a `Deleted`/`Renamed` status) can't silently resolve only
via its post-change on-disk location and drop the old package's former importers from
the seed set — the gap ADR-001 previously left as neither a resolved rename nor a
bail-out.

**Acceptance Criteria**:
- Given a `Modified` file whose diff hunk (base vs. working tree) contains both a
  removed `package foo` line and an added `package bar` line, when
  `detect_package_clause_change` runs against that file, then it returns `true`, and
  `compute_affected` short-circuits to
  `AffectedResult::BailOut(BailOutReason::PackageClauseChanged { path })` before any
  seed resolution or BFS work — the simplest, safest response, consistent with this
  plan's existing "when in doubt, bail out" pattern for `PackageFullyRemoved`/
  `ShallowCloneOrNoCommonHistory`.
  - *Given* a tempdir git repo where commit `abc1234` has `a/a.go` containing
    `package foo`, and the working tree edits that file's first line to `package bar`
    with no `git mv`, *When* `compute_affected(repo_root, "abc1234", &config)` runs,
    *Then* the result is
    `Ok(AffectedResult::BailOut(BailOutReason::PackageClauseChanged { path: PathBuf::from("a/a.go") }))`.
- Given a `Modified` Go file whose diff hunk changes only non-`package` lines (e.g. a
  function body edit), when `detect_package_clause_change` runs, then it returns
  `false`, and resolution proceeds normally via Story 2.2.1 — this check must not
  false-positive on an ordinary edit.
  - *Given* a tempdir git repo where `a/a.go`'s body (not its `package` line) changes
    between `abc1234` and the working tree, *When* `compute_affected` runs, *Then* the
    result is `Ok(AffectedResult::Packages(_))`, not a `PackageClauseChanged` bail-out.
- Given a `Modified` file that isn't a `.go` file, when checked, then
  `detect_package_clause_change` is skipped entirely for that file (matching this
  plan's Go-only v1 scope; a `package`-like line in a non-Go file carries no meaning
  here).
  - *Given* a `Modified` `README.md` whose diff hunk happens to contain a line
    starting with the literal text `package`, *When* checked, *Then* no
    `PackageClauseChanged` bail-out fires for that file.

**Files**: `src/affected.rs`

##### Task 2.2.3a: Implement `detect_package_clause_change` and wire it into `compute_affected` (~5 min)
- `fn detect_package_clause_change(repo_root: &Path, base_sha: &str, path: &Path) -> Result<bool>`:
  for a `.go`-extension path only, runs `git diff -U0 <base_sha>...HEAD -- <path>`
  (reusing the same `repo_root`/`base_sha` already resolved by Task 1.1.1a — no
  second base-resolution call), scans the hunk output for a removed line matching
  `^-package\s+(\S+)` and an added line matching `^\+package\s+(\S+)`; returns `true`
  only if both are present *and* the captured identifiers differ (a reformatted-but-
  unchanged `package` line, if one could even occur, is not a bail-out trigger). This
  is a cheap content-level check on the diff hunk already available from the same
  `git diff` machinery Story 1.1.2 establishes — not a full tree-sitter re-parse of
  the file's pre-image.
- In `compute_affected` (Task 3.1.1b), for every `Modified` entry, call
  `detect_package_clause_change` *before* Story 2.2.1's normal resolution for that
  entry; on `true`, short-circuit the whole function with
  `Ok(AffectedResult::BailOut(BailOutReason::PackageClauseChanged { path }))`, the same
  way Task 2.2.2b short-circuits on `PackageFullyRemoved`.
- Add the `PackageClauseChanged { path: PathBuf }` variant to `BailOutReason` (Task
  3.1.1a) and its `Display` impl.
- Files: `src/affected.rs`

##### Task 2.2.3b: Unit tests for Story 2.2.3 (~5 min)
- The three Given/When/Then cases above, using the same tempdir git-repo test helper
  as Task 1.1.1b/2.2.2c.
- Files: `src/affected.rs`

---

## Phase 3: CLI surface & output contract

### Epic 3.1: `AffectedResult`/`BailOutReason` types + `compute_affected` orchestration
**Goal**: Wire Phases 1–2's pieces into one pure function with the sum-type return
shape from the Domain Glossary/Pattern Decisions.

#### Story 3.1.1: Define `AffectedResult`/`BailOutReason` and implement `compute_affected`
**As a** `run_affected` (the CLI handler), **I want** one function that returns either
a package list or a bail-out reason, **so that** formatting stdout/exit-code is a
simple match, not a boolean-flag decode.

**Acceptance Criteria**:
- Given a repo where the diff touches only `internal/widget/widget.go` (no bail-out
  glob match, no delete/rename), when `compute_affected(repo_root, "main",
  &AffectedConfig::default())` runs, then it returns
  `Ok(AffectedResult::Packages(v))` where `v` contains `widget`'s package key and the
  keys of every (possibly transitive) importer of it.
  - *Given* a fixture repo matching that description, *When* `compute_affected` runs,
    *Then* the result is `Ok(AffectedResult::Packages(_))`, not `BailOut`.
- Given the diff touches `go.mod` (matched by the default `"**/go.mod"` bail-out
  glob) as well as an unrelated file, when `compute_affected` runs, then it returns
  `Ok(AffectedResult::BailOut(BailOutReason::GlobMatched{glob: "**/go.mod".into(),
  path: "go.mod".into()}))` **before** any graph-walk work is attempted (bail-out
  glob check runs first, cheaply, on the raw changed-file list).
  - *Given* a diff changing both `go.mod` and `internal/widget/widget.go`, *When*
    `compute_affected` runs, *Then* the result is `Ok(AffectedResult::BailOut(BailOutReason::GlobMatched{..}))`.
- Given nothing changed vs. `base` at all (empty diff), when `compute_affected` runs,
  then it returns `Ok(AffectedResult::Packages(vec![]))` — an empty list is a valid,
  successful answer, distinct from a `BailOut` (per `research/ux.md` §3's "ambiguous
  empty output" guidance: never let an empty list double as a bail-out signal or vice
  versa).
  - *Given* `base == HEAD` with a clean working tree, *When* `compute_affected` runs,
    *Then* the result is `Ok(AffectedResult::Packages(vec![]))`.

**Files**: `src/affected.rs`

##### Task 3.1.1a: Define `AffectedResult`/`BailOutReason` enums (~4 min)
- `#[derive(Debug, Clone, PartialEq)] pub enum AffectedResult { Packages(Vec<String>), BailOut(BailOutReason) }`.
- `#[derive(Debug, Clone, PartialEq)] pub enum BailOutReason { GlobMatched { glob: String, path: PathBuf }, PackageFullyRemoved { path: PathBuf }, PackageClauseChanged { path: PathBuf }, ShallowCloneOrNoCommonHistory }`.
  Deliberately has no "unresolvable base ref" variant — that case is a hard `Err`
  from `resolve_base_sha` (Story 1.1.1), never reaches this enum at all.
- `impl std::fmt::Display for BailOutReason` — one-line human message for each
  variant, used by `run_affected`'s stderr note.
- Files: `src/affected.rs`

##### Task 3.1.1b: Implement `compute_affected` orchestration (~5 min)
- `pub fn compute_affected(repo_root: &Path, base: &str, config: &AffectedConfig) -> Result<AffectedResult>`:
  1. `repo_toplevel` check (hard error if not a git repo).
  2. `resolve_base_sha`; propagates `Err` directly via `?` on an unresolvable ref —
     a **hard error**, not a bail-out (Story 1.1.1's reconciled exit contract).
  3. `diff_changed_files`; `None` → `Ok(BailOut(ShallowCloneOrNoCommonHistory))` (this
     one *does* stay a bail-out: `base` itself resolved fine, it's specifically the
     shared-history computation that's ambiguous — see Story 1.1.2).
  4. Bail-out glob check (Task 3.1.1c) over every `ChangedFile.path` against
     `config.effective_bail_out_globs()` (defaults unioned with any repo-supplied
     `extra_bail_out_globs`, Story 1.2.1), before touching the graph at all.
  5. Build working-tree `ArchModel` via `arch_model::collect_repo_files` +
     `arch_model::build_model` (same call shape as `arch_export.rs::run_export`,
     `include_private: true` — `affected` needs every package regardless of symbol
     visibility, unlike the export command's opt-in `--include-private`).
  6. Resolve seeds: Story 2.2.1 for Added/Modified/Renamed-new, Story 2.2.2 for
     Deleted/Renamed-old (short-circuits to `BailOut` on `PackageFullyRemoved`).
  7. `build_reverse_adjacency` + `reverse_bfs` from the seed set.
  8. Return `Ok(Packages(sorted Vec from the BFS result))`.
- Files: `src/affected.rs`

##### Task 3.1.1c: Implement the bail-out glob check (~3 min)
- `fn matches_any_bail_out_glob(changed: &[ChangedFile], globs: &[String]) -> Option<(String, PathBuf)>`:
  for each `ChangedFile`, for each glob in `globs`, if `glob::matches_scope(&path_str,
  std::slice::from_ref(glob))` is true, return `Some((glob.clone(), path.clone()))` on
  the first match (first-match-wins is fine — the specific matched glob is only for
  the debuggability message, not decision logic). Called from Task 3.1.1b with
  `&config.effective_bail_out_globs()`, not the raw `extra_bail_out_globs` field.
- Files: `src/affected.rs`

##### Task 3.1.1d: Unit tests for `compute_affected` end-to-end orchestration (~5 min)
- The three Given/When/Then cases above, built as small tempdir git-repo fixtures
  (init repo, write `go.mod` + a few packages, commit, make a second commit/working-
  tree edit, call `compute_affected`).
- Files: `src/affected.rs`

### Epic 3.2: CLI wiring
**Goal**: `kibitzer architecture affected --path <dir> --base <ref>` — the actual
user-facing entry point, with the stdout/exit-code contract from `requirements.md`'s
resolved Open Questions and `research/ux.md`.

#### Story 3.2.1: Add `ArchitectureAction::Affected` and dispatch it
**As a** a kibitzer user/CI job, **I want** `kibitzer architecture affected --base
<sha>` to exist as a real subcommand, **so that** I can invoke it exactly like
`export`/`diagram`/`change-coupling`.

**Acceptance Criteria**:
- Given kibitzer is built with this change, when `kibitzer architecture affected
  --help` runs, then it prints usage showing `--path` (default `.`) and `--base`
  (required) flags, consistent with `Export`/`Diagram`'s existing `--path` convention.
  - *Given* the built binary, *When* `kibitzer architecture affected --help` is run,
    *Then* the output contains `--base <BASE>` and `--path <PATH>`.
- Given the doc comment on `ArchitectureAction::Affected` (Task 3.2.1a) states the
  consumer-guard requirement, when `kibitzer architecture affected --help` runs,
  then the printed help text contains the guard guidance, not just the flag list —
  this is the concrete, testable form of closing the UX gap that the guard
  requirement previously lived only in this planning doc.
  - *Given* the built binary, *When* `kibitzer architecture affected --help` is run,
    *Then* the output contains the literal substring `[ -z "$PKGS" ]` (or, at
    minimum, the word `guard`) — a test asserting this string is present, not just
    a human read-through of the generated help text.
- Given `kibitzer architecture affected --base main` runs from inside a repo, when
  dispatched, then `main.rs`'s match arm calls `run_affected(&path, &base)`, which
  calls `affected::compute_affected` — end-to-end wiring, no separate code path.
  - *Given* a fixture repo, *When* the CLI is invoked with `--base main`, *Then* the
    process's stdout/exit-code matches what direct unit-testing of
    `compute_affected`'s `AffectedResult` for the same inputs would predict (verified
    by an integration test spawning the built binary, or by testing `run_affected`
    directly with an in-process `AffectedResult` fixture — Task 3.2.2c decides which).

**Files**: `src/main.rs`

##### Task 3.2.1a: Add `mod affected;` and the `ArchitectureAction::Affected` variant (~3 min)
- Add `mod affected;` alongside the other `mod` declarations (`src/main.rs:1-60`
  region, alphabetically placed).
- Add to `ArchitectureAction` enum (`src/main.rs:171-261`), grouped near
  `Export`/`Diagram` per `research/architecture.md` §3:
  ```
  /// Diffs the working tree against `--base`, walks `ArchModel.import_edges` in
  /// reverse from the changed files' packages, and prints the affected package set
  /// (or the `__ALL__` bail-out sentinel) — see `affected.rs`. Unlike every other
  /// `ArchitectureAction` variant, this subcommand's stdout is load-bearing for a CI
  /// consumer (`go test $(kibitzer architecture affected --base ...)`), not a
  /// "report, don't gate" prioritization list.
  ///
  /// Prints one package per line, or the literal `__ALL__` if the diff can't be
  /// safely narrowed. IMPORTANT: guard callers with `[ -z "$PKGS" ] && exit 0`
  /// before passing output to `go test $PKGS` — an unguarded empty result silently
  /// becomes `go test` with no args, which runs whatever package `cwd` resolves to
  /// instead of skipping.
  Affected {
      #[arg(long, default_value = ".")]
      path: PathBuf,
      #[arg(long)]
      base: String,
  },
  ```
  This doc comment is what `clap` surfaces verbatim in `kibitzer architecture
  affected --help` — it is the only place a new adopting repo is guaranteed to read
  before wiring this into a `build.yml`, so the consumer-guard requirement from the
  Risk Control section's Consumer Contract note (above) must be stated here in
  plain language, not just in this planning doc.
- Files: `src/main.rs`

##### Task 3.2.1b: Wire the dispatch match arm (~2 min)
- In the `Command::Architecture { action }` match (near `src/main.rs:510-516`), add
  `ArchitectureAction::Affected { path, base } => run_affected(&path, &base),`.
- Files: `src/main.rs`

#### Story 3.2.2: `run_affected`'s stdout/exit-code contract
**As a** stapler-squad's `build.yml` (or any CI wrapper), **I want** the exact
`test-affected.py` contract — newline-separated package list, or literal `__ALL__`,
both on stdout at exit 0; genuine errors non-zero on stderr only — **so that**
swapping the two tools is a one-line command change.

**Acceptance Criteria**:
- Given `compute_affected` returns `Ok(AffectedResult::Packages(vec!["example.com/app/a".into(), "example.com/app/b".into()]))`,
  when `run_affected` formats output, then stdout is exactly
  `"example.com/app/a\nexample.com/app/b\n"` (one per line, no header/footer/
  decoration, matching `git diff --name-only`/`go list` convention per
  `research/ux.md` §2), and the process exits `ExitCode::SUCCESS`.
  - *Given* that `AffectedResult`, *When* `run_affected` runs, *Then* captured stdout
    equals that exact string and the returned `ExitCode` is `SUCCESS`.
- Given `compute_affected` returns `Ok(AffectedResult::BailOut(BailOutReason::GlobMatched{glob:"**/go.mod".into(), path:"go.mod".into()}))`,
  when `run_affected` formats output, then stdout is exactly `"__ALL__\n"` (nothing
  else on stdout), a human-readable reason line
  (`"[kibitzer] bail-out: go.mod matched bail-out glob '**/go.mod'"` or equivalent) is
  printed to **stderr**, and the process exits `ExitCode::SUCCESS` — not a distinct
  exit code, per `requirements.md`'s resolved Open Question and `research/ux.md`'s
  explicit table.
  - *Given* that `AffectedResult`, *When* `run_affected` runs, *Then* stdout equals
    `"__ALL__\n"` exactly, and the exit code is `SUCCESS`.
- Given `compute_affected` returns `Err` (e.g. `repo_root` isn't a git repo, **or**
  `--base` doesn't resolve to a commit — Story 1.1.1's hard-error contract), when
  `run_affected` propagates it, then stdout is empty, stderr contains the error
  message (via `anyhow`'s `Context`), and the process exits non-zero (Rust's default
  `Termination` impl for `Result<ExitCode, E>`). This is the branch reconciled across
  `requirements.md`, `research/ux.md`, and this plan: an unresolvable `--base` is a
  hard error here, never an `AffectedResult::BailOut`.
  - *Given* a `repo_root` with no `.git`, *When* `run_affected` runs, *Then* stdout
    capture is empty and the exit status is non-zero.
  - *Given* a real repo and `--base does-not-exist-ref`, *When* `run_affected` runs,
    *Then* stdout capture is exactly `""`, stderr contains `"does-not-exist-ref"`,
    and the exit status is non-zero.
- Given `compute_affected` returns `Ok(AffectedResult::Packages(vec![]))` (nothing
  affected), when `run_affected` formats output, then stdout is empty (not `"\n"`,
  not any placeholder text) and a stderr note
  (`"AFFECTED: 0 packages — verify your CI wrapper guards against empty output
  (see --help)"`) is printed, exit `ExitCode::SUCCESS` — matching `research/ux.md`'s
  "empty stdout, optional stderr note" recommendation, deliberately diverging from
  the rest of the `ArchitectureAction` family's "always print something to stdout"
  convention because stdout here must stay pure for `$(...)` capture. The
  `AFFECTED:` prefix is deliberately distinct from the plain `"[kibitzer] ..."`
  prefix used by every other stderr note in this feature (see Task 3.2.2a) — this
  is the single highest-risk state (a green CI run silently running zero tests),
  so it gets its own greppable, `grep -i affected` / `grep 'AFFECTED:'`-friendly
  prefix a CI log reader or a `grep`-based CI-log linter can key on, rather than
  blending into ordinary informational output. This does not add a new exit code
  or reopen the hard-error-vs-bail-out design (both remain closed per Story
  1.1.1) — only the wording and greppability of this one stderr line changes.
  - *Given* that `AffectedResult`, *When* `run_affected` runs, *Then* stdout capture
    is exactly `""` and stderr contains the literal substring `"AFFECTED: 0 packages"`.
- Given `run_affected`'s real empty-stdout output for a zero-affected-packages case,
  and a consumer that feeds it through the exact CI shell shape `PKGS="$(...)";
  [ -z "$PKGS" ] && exit 0 || go test $PKGS`, when that shape is exercised
  end-to-end against the captured output, then the empty-`$PKGS` guard branch is
  reached (`exit 0`, `go test` never invoked) — this is the Consumer Contract note's
  required guard (Risk Control, above), made concrete and testable here rather than
  left as an assumption about how a downstream wrapper behaves.
  - *Given* kibitzer's captured stdout for the zero-affected-set case (`""`), *When*
    `sh -c 'PKGS="$1"; [ -z "$PKGS" ] && echo SKIPPED || echo "go test $PKGS"' -- "$CAPTURED"`
    runs with `$CAPTURED` set to that output, *Then* the observed output is exactly
    `SKIPPED` — demonstrating the guard branch is reachable and correct against
    kibitzer's actual contract (and, if the guard were omitted, that
    `echo "go test $PKGS"` would instead print the word-split, malformed
    `"go test"` with no package argument).

**Files**: `src/main.rs`

##### Task 3.2.2a: Implement `run_affected` (~5 min)
- ```rust
  /// `kibitzer architecture affected`: diffs `path`'s repo against `base` and prints
  /// the affected package set. Unlike `run_change_coupling`/`run_hotspots`/
  /// `run_root_cause_clusters`, this subcommand's stdout is a load-bearing CI
  /// contract (see `affected.rs`'s module doc comment) — `ExitCode::SUCCESS` still
  /// covers every *successful computation* (a narrowed list, an empty list, or the
  /// `__ALL__` bail-out are all valid answers); only a genuine failure to compute an
  /// answer at all (bad repo, git spawn failure) is non-zero.
  fn run_affected(path: &Path, base: &str) -> Result<ExitCode> {
      let (config, repo_root) = match config::find_config(path)? {
          Some((config, root)) => (config.affected, root),
          None => (config::AffectedConfig::default(), path.to_path_buf()),
      };
      match affected::compute_affected(&repo_root, base, &config)? {
          affected::AffectedResult::Packages(packages) if packages.is_empty() => {
              // Distinct `AFFECTED:` prefix (not the usual `[kibitzer] ...` shape)
              // so this line is easy to `grep` for in CI logs — the empty-set case
              // is the one state where a silent-skip-as-pass consumer bug is both
              // possible and invisible on an otherwise-green run.
              eprintln!(
                  "AFFECTED: 0 packages — verify your CI wrapper guards against \
                   empty output (see --help)"
              );
              Ok(ExitCode::SUCCESS)
          }
          affected::AffectedResult::Packages(packages) => {
              for pkg in packages {
                  println!("{pkg}");
              }
              Ok(ExitCode::SUCCESS)
          }
          affected::AffectedResult::BailOut(reason) => {
              println!("__ALL__");
              eprintln!("[kibitzer] bail-out: {reason}");
              Ok(ExitCode::SUCCESS)
          }
      }
  }
  ```
- Files: `src/main.rs`

##### Task 3.2.2b: Make `AffectedConfig` reachable from `main.rs` (~2 min)
- Confirm `config::AffectedConfig` is `pub` (Task 1.2.1a already made it so) and
  reachable via `config::find_config`'s returned `Config.affected` field; add any
  missing `pub use`/qualification needed for `main.rs` to name it.
- Files: `src/main.rs`

##### Task 3.2.2c: Integration tests for the stdout/exit-code contract (~5 min)
- Prefer testing `run_affected` (or a thin wrapper taking an injected
  `AffectedResult` — whichever keeps the test from needing a real git repo) directly
  via `std::process::Command`-capturing the built `kibitzer` binary against a small
  fixture repo, matching `arch_export.rs`'s own test style (`run_export_dry_run_prints_json_and_writes_no_file`,
  etc., which spawn real repo fixtures rather than mocking).
- Cover: non-empty list, `__ALL__` bail-out, empty-affected-set, and both hard-error
  shapes (non-git-repo, and an unresolvable `--base` ref) from the acceptance
  criteria above.
- Also spawn `kibitzer architecture affected --help` and assert its stdout contains
  the consumer-guard text from Task 3.2.1a's doc comment (e.g. the substring
  `[ -z "$PKGS" ]` or `guard`) — the test behind Story 3.2.1's new acceptance
  criterion, confirming the guard requirement actually reaches `--help`'s rendered
  output and not just the source doc comment.
- Files: `src/main.rs` (or a new `tests/affected_cli.rs` if `arch_export.rs`'s own
  precedent uses inline `#[cfg(test)]` — match whichever convention
  `arch_export.rs`'s existing tests actually use).

##### Task 3.2.2d: Integration test for the empty-affected-set CI shell-guard shape (~4 min)
- Using the empty-stdout capture from Task 3.2.2c's zero-affected-set case, spawn
  `sh -c 'PKGS="$1"; [ -z "$PKGS" ] && echo SKIPPED || echo "go test $PKGS"' -- "$CAPTURED"`
  via `std::process::Command` and assert the observed output is exactly `SKIPPED` —
  the concrete test behind Story 3.2.2's new acceptance criterion and the Risk
  Control section's Consumer Contract note. This test documents and enforces the
  guard requirement; it does not (and cannot, since kibitzer doesn't own
  `build.yml`) prevent an adopting repo from omitting the guard — the requirement is
  stated in prose above for that reason.
- Files: `src/main.rs`

---

## Phase 4: Validation

### Epic 4.1: Automated tests + real-world comparison
**Goal**: Satisfy `requirements.md`'s Success Metrics — synthetic-graph coverage of
the walk/bail-out logic, plus the real-stapler-squad-diff comparison this repo's own
CLAUDE.md requires in place of a `check backtest` run (this isn't a per-file checker,
so `kibitzer check backtest` doesn't apply, but the same "run against real signal
before shipping" bar does).

#### Story 4.1.1: Synthetic-graph coverage confirmation
**As a** a future maintainer, **I want** one place confirming Phase 1–3's unit tests
collectively cover every acceptance criterion in this plan, **so that** "done" means
something checkable, not just "code exists."

**Acceptance Criteria**:
- Given every task in Phases 1–3 that includes a "Files: `src/affected.rs`" unit-test
  task, when `cargo test affected::` runs, then all tests pass.
  - *Given* the completed Phase 1–3 implementation, *When* `cargo test affected::` is
    run, *Then* the exit code is 0 and the reported test count matches the number of
    `#[test]` functions added across Tasks 1.1.1b, 1.1.2d, 2.1.1b, 2.1.2b, 2.2.1c,
    2.2.2c, 2.2.3b, 3.1.1d.

**Files**: `src/affected.rs`, `src/config.rs`

##### Task 4.1.1a: Run the full `affected`/`config` test suite and fix any gaps (~5 min)
- `cargo test affected:: config::` (or the crate's actual test-invocation
  convention); confirm every acceptance-criterion example above has a corresponding
  passing test; add any missing one found during this pass.
- Files: `src/affected.rs`, `src/config.rs`

#### Story 4.1.2: Bail-out trigger coverage — one test per category, plus the hard-error case
**As a** the requirements' Success Metrics ("each unbounded-blast-radius file
category... fires the sentinel"), **I want** an explicit test per bail-out category
— and an explicit test confirming the one case that is *not* a bail-out — **so
that** a future refactor can't silently drop one or blur the hard-error/bail-out
line back together.

**Acceptance Criteria**:
- Given each of: (a) a default-glob match (`**/go.mod`), (b) a fully-removed package
  (Story 2.2.2), (c) a shallow-clone/no-common-history diff — when `compute_affected`
  runs against a fixture for each, then all three return `Ok(AffectedResult::BailOut(_))`
  with the category-appropriate `BailOutReason` variant.
  - *Given* fixtures for cases (a)-(c), *When* `compute_affected` runs against each,
    *Then* each result matches `Ok(AffectedResult::BailOut(reason))` where `reason`'s
    variant corresponds to the triggering category (`GlobMatched`,
    `PackageFullyRemoved`, `ShallowCloneOrNoCommonHistory` respectively).
- Given (d) an unresolvable `--base` ref, when `compute_affected` runs, then it
  returns `Err`, **not** `Ok(AffectedResult::BailOut(_))` — this is deliberately a
  separate assertion from (a)-(c) above, since conflating it with the bail-out
  categories is exactly the ambiguity Story 1.1.1's exit-contract reconciliation
  fixed.
  - *Given* a fixture repo and a nonexistent `--base`, *When* `compute_affected`
    runs, *Then* the result is `Err(_)`, and this fact is asserted explicitly (not
    just implied by the absence of an `Ok` case in a shared table).

**Files**: `src/affected.rs`

##### Task 4.1.2a: Add the bail-out-category tests plus the hard-error case as one table-driven test (~5 min)
- These likely already exist individually from Tasks 1.1.1b/1.1.2d/2.2.2c/3.1.1d;
  this task's job is to add one consolidated test that exercises the three bail-out
  categories *and* the unresolvable-`--base`-ref hard-error case end-to-end through
  `compute_affected` (not just the lower-level helper each was unit-tested through),
  confirming the full orchestration path (Task 3.1.1b) routes each condition to the
  right outcome — `Ok(BailOut(_))` with the right variant for (a)-(c), `Err` for
  (d) — catching an integration-level regression a lower-level unit test wouldn't,
  including the specific regression of (d) drifting back into `Ok(BailOut(_))`.
- Files: `src/affected.rs`

#### Story 4.1.3: Real-world comparison against `scripts/test-affected.py`
**As a** the requirements' Success Metrics and this repo's CLAUDE.md bar ("no native
checker or feature ships without being run against real-world signal"), **I want**
`kibitzer architecture affected`'s output compared against `test-affected.py`'s
output on real stapler-squad history, **so that** the cutover claim ("computing the
same affected set... for a representative sample of real historical stapler-squad
diffs") has actual evidence behind it.

**Acceptance Criteria**:
- Given at least 10 real historical stapler-squad commits (each with a resolvable
  parent as `--base`), when both `kibitzer architecture affected --path
  ~/Programming/stapler-squad --base <parent-sha>` and `python3 scripts/test-affected.py
  <parent-sha>` (run from within the stapler-squad checkout) are invoked for each,
  then their outputs are recorded side by side in
  `docs/affected-validation.md` (this repo), noting exact matches, `__ALL__`-vs-list
  divergences, and any package-set differences — with a stated root cause for each
  divergence (e.g. the `replace`-directive gap from Tech Debt Disposition), not left
  unexplained.
  - *Given* 10 selected commits, *When* both tools are run against each, *Then*
    `docs/affected-validation.md` contains one row per commit with both tools'
    output and a match/divergence verdict.
- Given any divergence is found where kibitzer's output is **narrower** than
  `test-affected.py`'s (a potential under-test regression, the one unacceptable
  failure mode), when documented, then the plan explicitly states whether it's
  reconciled (a bail-out glob or resolution fix added) or accepted with a documented
  reason — never left as an unexplained gap, per `requirements.md`'s Feasibility
  Risks section.
  - *Given* a hypothetical narrower-divergence finding, *When* recorded, *Then* the
    entry names either a follow-up fix task or an explicit accepted-risk rationale
    tied to a specific Tech Debt Disposition entry.
- Given the Non-functional Requirements' performance bar ("at most comparable to
  `test-affected.py`'s current `go list -deps` walk, not slower"), when the
  comparison run happens, then approximate wall-clock timing for both tools is
  recorded alongside the output comparison — a documented manual timing check
  (`time <cmd>` is enough; no formal benchmark harness), since this NFR was
  otherwise asserted with no task anywhere validating it.
  - *Given* the same 10+ commits, *When* both tools run, *Then*
    `docs/affected-validation.md` records at least one wall-clock timing
    observation for each tool (e.g. a representative single-commit timing, not
    necessarily all 10), with a one-line verdict on whether `affected` meets the
    "not slower" bar on this repo's current size.
- Given a synthetic fixture with 3+ nested `go.mod` modules mirroring stapler-squad's
  actual layout (a root module plus `tuitest/go.mod`, `tools/scanner/go.mod`,
  `tools/lint/go.mod`, per `requirements.md:426-427`) where the root module's
  `go.mod` contains a `replace` directive pointing at the nested `tools/scanner`
  module, when a file inside `tools/scanner/` is changed and `compute_affected` runs
  against it, then the actual computed `AffectedResult` is recorded and asserted
  against the expected result given the `replace` directive — closing the
  previously-untested gap where no test in this plan exercised more than one
  `go.mod` in the same repo. This is a deliberately constructed fixture, not an
  opportunistic sample from real history (see Task 4.1.3a).
  - *Given* that fixture and a change to a file in `tools/scanner/`, *When*
    `compute_affected` runs, *Then* the test asserts the actual computed package set
    — whether it correctly includes the root module's dependent package via the
    `replace` directive, or under-detects it per `find_go_mod_upward`'s documented
    lack of `replace`-directive support (Tech Debt Disposition, above) — rather than
    leaving the multi-module case unexercised and the risk purely theoretical.

**Files**: `docs/affected-validation.md` (new, in this repo), `src/affected.rs`
(the multi-module fixture test from Task 4.1.3a)

##### Task 4.1.3a: Construct a synthetic multi-`go.mod`/`replace`-directive fixture, then select 10+ representative historical stapler-squad commits (~10 min)
- **Multi-module fixture (constructed deliberately, not sampled from history)**:
  build a synthetic fixture (a tempdir-based test repo, or a checked-in
  `testdata/multi_module_fixture/` tree — whichever matches `src/affected.rs`'s own
  existing tempdir-git-repo test convention) with 3+ nested `go.mod` files mirroring
  stapler-squad's real layout: a root module plus `tuitest/go.mod`,
  `tools/scanner/go.mod`, `tools/lint/go.mod` (per `requirements.md:426-427`), where
  the root module's `go.mod` has a `replace` directive pointing at the nested
  `tools/scanner` module (e.g. `replace example.com/app/tools/scanner =>
  ./tools/scanner`), and the root module has a package that imports
  `tools/scanner`. Change a file inside `tools/scanner/` and run `compute_affected`
  against it directly. Record and assert what package set `affected` actually
  computes — does `reverse_bfs` find the root module's dependent package via the
  `replace` directive, or does `find_go_mod_upward`'s lack of `replace`-directive
  support cause `affected` to miss it (the Tech Debt Disposition's previously
  theoretical risk, now given a real, repeatable answer). This replaces the earlier
  "sample a `replace`-directive commit from real history *if one exists*" approach,
  which left this specific risk completely untested whenever stapler-squad's own
  history happened not to contain one.
- **Real-history commit selection** (unchanged in scope from the earlier draft):
  from `~/Programming/stapler-squad`, pick commits spanning: a normal `.go`-only
  change, a `go.mod` change, a rename, a delete, and a merge commit, using
  `git log --oneline -- '*.go' 'go.mod'` to identify candidates.
- Files: `src/affected.rs` (the multi-module fixture and its assertion); selection
  notes for the real-history commits recorded directly into Task 4.1.3b's doc.

##### Task 4.1.3b: Run both tools against each selected commit and record results, with timing (~5 min per commit, batched)
- For each commit `sha`, run both commands against `sha^` as base, wrapped in
  `time`; capture both stdouts and wall-clock times.
- Write `docs/affected-validation.md` with one table row per commit: SHA, category
  (normal/go.mod/rename/delete/merge), `test-affected.py` output summary + timing,
  kibitzer output summary + timing, verdict (match / kibitzer narrower —
  investigate / kibitzer broader — acceptable).
- Also add a short standalone section to `docs/affected-validation.md` (not a table
  row — this isn't a per-commit comparison case) titled something like "Empty
  affected-set / CI wrapper guard," restating the Risk Control section's consumer
  contract in adopter-facing terms: the exact shell guard
  (`[ -z "$PKGS" ] && exit 0`), why an unguarded `go test $PKGS` silently degrades
  to testing whatever package `cwd` resolves to, and the `AFFECTED:`-prefixed
  stderr line (Task 3.2.2a) an adopter can grep their CI logs for if they suspect
  this happened. This is the one place, alongside `--help`, where this requirement
  is documented for a human reading kibitzer's repo rather than only observed at
  runtime.
- Files: `docs/affected-validation.md`

##### Task 4.1.3c: Reconcile or document every divergence found (~5 min per divergence)
- For each row marked "kibitzer narrower," either file a follow-up fix (referencing
  the relevant Tech Debt Disposition entry — e.g. the `replace`-directive gap) or add
  an explicit accepted-risk note with reasoning, per the acceptance criteria above.
  No row may be left with an unexplained divergence. Separately, add a one-line
  verdict on the timing data gathered in Task 4.1.3b: if kibitzer is materially
  slower than `test-affected.py` on this repo's current size, state explicitly
  whether that's accepted for v1 or pulls the daemon-cache follow-up (out of scope
  here, per requirements.md's Scope section) forward.
- Files: `docs/affected-validation.md`
