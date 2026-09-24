# Research: Similar Features, Edge Cases, Unstated Needs — `affected-tests`

Agent 2 (Features), SDD Phase 2. All repo citations are VERIFIED (file opened directly)
unless marked INFERRED/UNVERIFIED.

## 1. `scripts/test-affected.py` (VERIFIED — full file read)

Found on this machine at `/home/tstapler/Programming/stapler-squad/scripts/test-affected.py`
(39 lines of header comment + 139 lines total). It was **not** present inside this
kibitzer worktree, matching the requirements.md note that it lives in another repo.

Key mechanics, all confirmed by reading the file directly:

- **Diff scope** (`test-affected.py:73-84`): unions three sources, not just a base-ref
  diff — `git diff --name-only {base}...HEAD` (merge-base diff), `git diff --name-only HEAD`
  (uncommitted tracked edits), and `git ls-files --others --exclude-standard` (new
  untracked files). It deliberately reflects the *working tree*, not just what's
  committed. If `git diff` against `base` fails (e.g. `origin/main` not fetched locally),
  it falls back to `__ALL__` (`test-affected.py:78-83`) rather than guessing.
- **Bail-out list** (`test-affected.py:33-40`, `FULL_RESCAN_TRIGGERS`): `proto/*`,
  `go.mod`, `go.sum`, `Makefile`, `.golangci.yml`, and **its own script path**
  (`scripts/test-affected.py`) — explicitly "don't trust a change to this script's own
  logic" (comment at line 39). Matched via `fnmatch.fnmatch`, which the header comment
  (line 31-32) notes supports `*` crossing `/`, same semantics as kibitzer's own
  `src/glob.rs` `**`.
- **Why `.proto` triggers a bail-out, not a scan**: the script's own docstring
  (`test-affected.py:9-15`) states generated `.go` code from `.proto` is **gitignored**
  and not committed in stapler-squad, so a plain `.go`-file diff can never see it — this
  is a repo-specific codegen-invisibility problem, not a general "protos are scary"
  rule. A repo that commits generated code wouldn't need this trigger for that reason
  (though it might for others — see §4).
- **Graph source of truth**: `go list -json -test ./...` (`test-affected.py:107`), not a
  hand-rolled import parser — it parses the `Deps` field per package (including the
  synthetic `<pkg> [<pkg>.test]` test-binary variant packages, unified back to the real
  import path via regex at line 112-121) and does forward containment: a package is
  "affected" if it has tests **and** (it changed itself, or a changed package is in its
  dependency set) (`test-affected.py:126-131`). This is the opposite direction from what
  kibitzer needs: `go list -json` gives each package's own deps (forward edges an
  importer already declares), so checking `deps & changed_pkgs` per package **is** a
  reverse-reachability query, just computed by scanning every package's forward-dep set
  rather than inverting an edge list. It works at Go's scale but is O(packages × avg
  deps) per changed set, not indexed.
- **Output contract**: one import path per line to stdout, or the literal single line
  `__ALL__` (`test-affected.py:17-21` docstring, `test-affected.py:82,87`). Exit code is
  always the Python default (0) — the bail-out signal is carried entirely in stdout
  content (`__ALL__`), **not** a distinct exit code. Nothing else changes behavior on
  the two paths from the caller's perspective except string-matching the first line.
- **No test coverage**: no test file alongside the script was found in the stapler-squad
  checkout at `/home/tstapler/Programming/stapler-squad` (only the script itself was
  located by the `find` scan for `test-affected.py`) — consistent with requirements.md's
  "no shared test coverage" framing.
- **Deleted files**: a changed file's directory can vanish entirely (last file in a dir
  deleted); `go list ./<dir>` then fails, and the script explicitly **skips** that
  directory rather than crashing the whole run (`test-affected.py:99-103`, comment:
  "skip it rather than crashing the whole run over one path"). This silently
  under-reports packages that only *contained* deleted files with no surviving package
  — arguably fine since a deleted-only package has no tests left to run, but it does mean
  a rename that deletes the old path and adds a new one relies on the new path being
  seen via the untracked/tracked diff separately, not via any explicit rename handling.
- **No merge-commit handling**: the `{base}...HEAD` triple-dot diff already handles the
  "changes since branch point" case structurally (triple-dot = merge-base diff, not a
  direct two-ref diff), but there's no special-casing for a merge commit *itself* being
  in the HEAD range, nor for renamed files (`git diff --name-only` reports renames as a
  delete+add pair by default unless `-M` is passed, which this script does not pass) —
  see §3 for how this compares to industry tools.

## 2. kibitzer's own batch-report family (VERIFIED)

Read `src/change_coupling.rs:1-40`, `src/hotspots.rs:1-40`, `src/root_cause_clusters.rs:1-40`,
and the `ArchitectureAction` enum + dispatch in `src/main.rs:171-261,496-517,659-740`.

Shared conventions all three (`change-coupling`, `root-cause-clusters`, `hotspots`) follow,
which `affected` should either match or explicitly justify deviating from:

- **CLI shape**: `kibitzer architecture <verb>` subcommand, flags `--path <PathBuf>`
  (default `.`), `--limit <usize>` (commits scanned, default 1000), `--top <usize>`
  (results to report, default 20). `affected`'s natural flags are different in kind
  (`--base <sha>` is required-ish, not a count), so this convention transfers only
  loosely — the `--path` convention (repo root or subdirectory scope) likely still
  applies.
  **Doc-comment convention** on each variant explicitly states: "Batch-only — never wired
  into `default_checks()`/hook mode" (`src/main.rs:211-214,226-233,244-250`, restated in
  each module's own top-of-file doc comment). `affected` is inherently a batch/CI
  operation (diff-driven, not per-file), so it should carry the same explicit disclaimer
  rather than silently omitting it — a reader shouldn't have to infer why it's absent
  from `config::default_checks()` (`src/config.rs:590-599`).
- **Output format**: plain, human-readable `println!` lines (`src/main.rs:668-676,
  693-717,735-738`) — no JSON output today for any of the three. `arch_export.rs`
  (Export/Diagram, not in this family) is the only architecture subcommand with a
  `--out <file>`/`--dry-run` JSON-emission pattern (`src/main.rs:175-192`, `--out
  PathBuf` mandatory, `--dry-run` prints instead of writing). That's the closer
  precedent for a `--format json` flag on `affected` if one is added (see §5) — the
  change-coupling/hotspots family has no JSON output at all to imitate.
- **Exit code**: all three batch reports **always return `ExitCode::SUCCESS`** when the
  analysis itself completes without error — explicitly documented as "report, don't
  gate" (`src/main.rs:682-683,722-725`: "Same 'report, don't gate' convention... always
  `ExitCode::SUCCESS` when the analysis itself succeeds"). This is a hard convention
  mismatch with what a CI wrapper needs from `affected` (see §5) — `affected` is not a
  "look here" prioritization report, it's meant to gate what a CI step runs next, so
  copying "always exit 0" verbatim would defeat the purpose. This is the single most
  important deliberate deviation the design should call out explicitly, not silently.
- **"Empty result" convention**: each prints a specific one-line `[kibitzer] no ... found`
  message and returns success rather than empty stdout (`src/main.rs:664,689,731`) —
  `affected` finding zero affected packages (a real, valid outcome per
  `test-affected.py:91-92,104-105`, which just exits with no output) should decide
  whether to follow kibitzer's "always print something" convention or Python's "silent
  empty" one; these currently disagree.

## 3. Existing graph/diff infrastructure kibitzer already has (VERIFIED)

- `ImportGraph` (`src/import_graph.rs:25-35`) has `nodes: BTreeSet<String>`,
  `edges: Vec<ImportEdge>` (`from`/`to`/`file`/`line`), and
  `file_packages: BTreeMap<PathBuf, String>` — confirmed the single source of truth for
  file → package-key mapping (doc comment at `src/import_graph.rs:28-34`, reused by
  `arch_model.rs:241-254` specifically so package grouping never mismatches the import
  graph's own keys).
- `ImportGraph::edges_from` (`src/import_graph.rs:38-40`) is **forward-only** —
  `self.edges.iter().filter(move |e| e.from == node)`. There is no reverse index/method
  anywhere in `import_graph.rs` or `arch_model.rs` (confirmed by `grep -n
  "file_packages\|import_edges\|edges_from"` across both files — no `edges_to` or
  `importers_of` exists). This directly confirms rabbit hole #1 in requirements.md:
  reverse reachability is new code, not a flag flip.
- **A working precedent for building a reverse index on demand does exist**, just for
  call edges, not import edges: `src/mcp.rs`'s `traverse_call_edges`
  (`src/mcp.rs:314-341`) takes a `CallDirection` and calls
  `build_call_adjacency(edges, direction)` to build a `HashMap`-based adjacency list
  in the requested direction (forward for callees, reverse for callers — see
  `traverse_call_edges_callers_walks_the_reverse_direction`, `src/mcp.rs:1171`, and
  `list_callers`/`list_callees`, `src/mcp.rs:979,991`), then does bounded BFS with a
  `visited` `HashSet` and a `truncated` flag for depth-limited traversal. This is the
  closest in-repo template for `affected`'s reverse-reachability walk: build a
  `HashMap<&str, Vec<&ImportEdge>>` keyed by `to` (inverting `ImportGraph.edges` once),
  then BFS/DFS from each changed package's node key, unbounded depth (transitive, no
  `--depth` needed since "affected" wants full transitive closure, not N hops).
- **No existing base-ref-diff or `git diff --name-only` code path anywhere in
  `src/*.rs`** (grep across all non-test `.rs` files for `diff --name-only`,
  `merge-base`, `rev-parse` found only `git rev-parse --show-toplevel` in
  `hotspots.rs:40` and `git rev-parse HEAD`/`--abbrev-ref HEAD` in `hook_log.rs:41,45`
  — no diff-against-a-ref logic exists anywhere). `change_coupling.rs` and `hotspots.rs`
  both shell out to `git log --name-only --no-merges` for *history* scanning, not a
  *diff against a base ref* — genuinely new territory for this codebase, not an
  extension of an existing helper.
- **Glob matching already exists**: `src/glob.rs` (`glob_to_regex`, lines 1-30+) compiles
  `**`/`*`/`?` patterns to anchored regex against `/`-separated paths — the same
  semantics `test-affected.py`'s `fnmatch`-based `FULL_RESCAN_TRIGGERS` needs. The
  bail-out file list for `affected` should reuse this rather than hand-rolling pattern
  matching or pulling in a new glob crate.
- `config::default_checks()` (`src/config.rs:590-599`) confirms `affected` has no
  natural home there — it's not a per-file checker with `file_globs()`, it's a
  whole-repo, diff-driven batch command, matching the `ArchitectureAction` family's shape
  far more than `checker::registry()`'s.

## 4. Industry comparables (INFERRED/UNVERIFIED — training knowledge, not fetched live)

Flagging explicitly: nothing in this section was verified against current upstream docs
or source during this research pass — no `WebFetch`/`WebSearch` was used given the task
scope. Treat as directionally useful priors to sanity-check during planning, not settled
fact.

- **`digitalocean/gta`** ("Go Test Auto"): INFERRED — conceptually the same shape as
  `test-affected.py`: walks `go list`-derived dependency graphs from a base ref to find
  affected packages, and (from memory of its README) has a notion of "no-test" packages
  and an explicit escape hatch for files it can't reason about. I do not have high
  confidence in its exact CLI flags or current bail-out-file-list mechanism from
  training data alone — worth a live doc check before citing specifics in the plan.
  Only cite it in the design doc as "the same problem shape as prior art," not for any
  specific flag name.
- **Nx `affected`**: INFERRED — monorepo-wide (not Go-specific), builds a project graph
  from declared workspace dependencies (`project.json`/`nx.json` + inferred
  source-file imports) rather than a single language's import graph, and computes
  affected *projects* by diffing against a base commit, with a `--base`/`--head` pair
  of flags (this maps closely to kibitzer's proposed `--base <sha>`, though Nx's
  default `--head` is implicitly the working tree). Nx's known edge-case handling I
  recall with reasonable confidence: it treats changes to root-level config
  (`nx.json`, lockfiles, root `tsconfig`) as affecting the *entire* graph — the same
  "unbounded blast radius → bail to everything" shape as `FULL_RESCAN_TRIGGERS`, which
  is corroborating (not just kibitzer/stapler-squad's invention) that this is a
  load-bearing pattern any similar tool converges on independently.
- **`jest --changedSince`**: INFERRED — delegates the actual git diff/rename detection to
  `jest-changed-files`, which (from memory) uses `git diff --name-status` and does
  handle rename detection status codes (`R100` etc.) rather than treating a rename as a
  pure delete+add. If accurate, this is a concrete edge case `test-affected.py` does
  *not* handle (no `-M`/`--find-renames` flag was passed in the read source) that
  kibitzer's `affected` should decide on deliberately rather than inheriting the gap
  silently.
- **Bazel `query --changed` / `bazel query 'rdeps(...)'`**: INFERRED — Bazel's own
  affected-target computation is the closest thing to "already solved this generally":
  `rdeps(universe, changed_targets)` is a reverse-dependency query over Bazel's own
  build graph, which is exactly the reverse-reachability shape requirements.md
  describes. Bazel's practical guidance (recalled, not verified) also bails out to a
  full build/test for `BUILD`/`WORKSPACE`/toolchain changes — again corroborating the
  bail-out-list pattern as convergent, not stapler-squad-specific.
- **Common thread across all four**: every one of these systems separately arrived at
  (a) reverse reachability over some dependency graph, and (b) an explicit "some files
  have unbounded blast radius, don't try to model them, just bail to full" escape
  hatch. None of them (as far as I recall) encode that bail-out list as a hardcoded
  constant meant to be portable across projects — Nx's and Bazel's both derive from
  *that repo's own* config-file locations, which is the strongest signal that
  kibitzer's bail-out list must be a configurable/overridable set, not a single
  Go-ecosystem-wide hardcoded list, even though v1 scope is Go-only.

## 5. Bail-out file list for kibitzer's Go v1 (synthesis — not stapler-squad-specific)

Derived from generalizing `test-affected.py`'s `FULL_RESCAN_TRIGGERS`
(`test-affected.py:33-40`) to "what has unbounded blast radius in *any* Go repo," per
requirements.md's explicit warning that hardcoding it narrowly is "a hazard... wrong
here = silent under-testing":

- `go.mod`, `go.sum` — module graph itself; any package could gain/lose a transitive
  dependency edge kibitzer's import graph has no way to see without a full re-walk.
  (Inherited directly from `test-affected.py`, applies to any Go repo, not
  stapler-squad-specific.)
- Any build-tag-affecting file — `//go:build` constraint changes can silently
  change which files compile into a package on a given `GOOS`/`GOARCH`, which a
  file-level import-graph diff can't detect without re-parsing build constraints
  repo-wide. Not present in `test-affected.py` (stapler-squad may not use build tags
  heavily) but a genuine generalization gap for a portable v1.
  Concretely: any `.go` file whose diff touches a `//go:build` or legacy `// +build`
  line, and any change to files matched by `GOOS`/`GOARCH`-suffix naming
  (`_linux.go`, `_windows.go`, etc.) should probably be treated conservatively —
  though this could also be handled precisely (not via bail-out) since kibitzer
  already parses Go source with tree-sitter (`checker::GrammarCache`,
  `hotspots.rs:16`) and could recompute which files a changed build-tag affects. This
  is a genuine design decision for Phase 3, not settled here.
- **Codegen inputs** — generalizing `test-affected.py`'s `proto/*` special case:
  the trigger is specifically "generated code is gitignored and not visible to a
  `.go`-file diff" (`test-affected.py:9-15`). The portable form of this rule is not
  "protobuf is special," it's "any input to a code generator whose output isn't
  visible to the import graph" — `.proto` files, but also `go:generate` directive
  targets, `sqlc`/`stringer`/`mockgen` inputs, etc. **This should be config-driven**
  (a glob list a repo supplies, using kibitzer's existing `src/glob.rs`), not a
  hardcoded language-wide list, because which codegen tools a Go repo uses is
  project-specific — a stapler-squad-specific `proto/*` hardcode ported verbatim would
  be exactly the "hazard" requirements.md flags.
- **kibitzer's own affected-computation source** — direct analogue of
  `test-affected.py`'s self-referential trigger (line 39, "don't trust a change to this
  script's own logic"). For kibitzer this would mean: a change to the new
  `affected.rs`/whatever module implements this feature, and arguably to
  `import_graph.rs`/`arch_model.rs` themselves (the graph `affected` depends on),
  should bail out rather than trust the graph a modified graph-builder just produced.
- **Build/lint config with repo-wide effect** — `Makefile`, `.golangci.yml` in
  stapler-squad's list are stapler-squad-specific filenames, but the *category*
  ("repo-wide build/lint/CI configuration that could change what gets compiled or how
  tests run") generalizes; the portable v1 default should probably be a short,
  clearly-labeled default list (`go.mod`, `go.sum`, kibitzer's own source) plus a
  documented, easy override point for repo-specific config files — mirroring
  `docs/suppressing-checks.md`'s existing precedent (per CLAUDE.md) for how a local
  `.claude/inspect.json` overlays kibitzer's defaults rather than replacing them.

## 6. Unstated needs beyond the explicit requirements

- **Exit-code semantics are a real, currently-unaddressed gap.** Every existing
  `ArchitectureAction` batch command hardcodes `ExitCode::SUCCESS`
  (`src/main.rs:682-683,722-725`, confirmed above) — there is no precedent in this
  codebase for a batch command whose exit code carries meaning. `affected` needs one
  (e.g., 0 = "here's the list on stdout," 2 = "bail out, run everything," maybe a
  distinct code for "diff/git error, couldn't determine anything") because a CI
  wrapper branching on stdout content alone (as `test-affected.py`'s callers must,
  since it only emits `__ALL__` on stdout with no distinguishing exit code) is more
  fragile than branching on exit code plus reading stdout only in the non-bail-out
  case. This is worth flagging as a **deliberate, explicit improvement over
  `test-affected.py`'s own contract**, not an unmotivated deviation — call it out in
  the plan rather than silently changing behavior relative to the tool it's replacing.
- **`--format json` plausibly matters**, and there's already a partial precedent for it
  in `arch_export.rs`'s `--out`/`--dry-run` JSON-emission shape (`src/main.rs:175-192`)
  — but that precedent is for a full ArchModel dump, not a short package list. A JSON
  array of affected package keys (or `{"bail_out": true, "reason": "go.mod changed"}` /
  `{"bail_out": false, "packages": [...]}`) would matter specifically if this ever
  composes with a non-shell consumer (a JS/TS port, or kibitzer's own MCP server
  exposing an `affected_packages` tool alongside `list_callers`/`list_callees` —
  `src/mcp.rs:979,991`). Scope says JS/TS is out for v1, but an MCP-tool consumer is a
  plausible near-term follow-on given the existing `list_callers`/`list_callees` MCP
  tools already expose graph-shaped JSON (`src/mcp.rs:919-991`) — worth designing the
  internal data model so a JSON encoder is a thin wrapper, even if `--format json`
  itself isn't built in v1.
- **The bail-out reason should be reported, not just the boolean.** `test-affected.py`
  gives zero visibility into *which* trigger fired (`__ALL__` alone). A human debugging
  "why did CI just run everything" benefits from knowing it was `go.mod` vs. a
  self-referential kibitzer-source change vs. a git-diff failure — cheap to add,
  meaningfully more debuggable, and not in scope of the explicit requirements but a
  near-zero-cost win.
- **Renamed/moved files changing package keys** (explicitly an open question in
  requirements.md) has a concrete resolution path via existing kibitzer code:
  `file_packages: BTreeMap<PathBuf, String>` (`src/import_graph.rs:34`) is rebuilt fresh
  from the *current* tree on every `import_graph::build` call — it has no memory of
  history. A rename means the changed-file-path-to-package-key lookup must happen
  against the **post-change** tree (HEAD), and a bare `git diff --name-only` (no `-M`)
  reports a rename as delete+add, meaning the "old" path in the diff won't resolve in
  `file_packages` at all (it no longer exists) — this needs to resolve to the new path
  for the affected-computation to work, i.e. `affected` should pass `--find-renames`
  (or parse `git diff --name-status -M` and take the new-side path) rather than
  reusing `test-affected.py`'s plain `--name-only` invocation verbatim.
- **Deleted-only changes** need an explicit decision: `test-affected.py` silently drops
  a directory that `go list` can no longer resolve (`test-affected.py:99-103`).
  Whether "a package was deleted entirely" should trigger a bail-out (its former
  importers might now fail to build) or be silently ignored (nothing to test since the
  package is gone) is a real design decision requirements.md doesn't resolve — leaning
  toward "check for importers of the deleted package's old key against the
  *pre-change* graph" as the correct-but-more-expensive option, worth flagging for
  Phase 3 rather than deciding here.

## Summary of what to hand to Phase 3 (plan)

1. Reverse reachability = invert `ImportGraph.edges` into a `HashMap<&str, Vec<&ImportEdge>>`
   keyed by `to`, then BFS/DFS unbounded-depth from each changed package's
   `file_packages` key — directly modeled on `mcp.rs`'s existing `build_call_adjacency`/
   `traverse_call_edges` pattern, just for import edges instead of call edges, and full
   transitive closure instead of depth-bounded.
2. Base-ref diffing is new code for this codebase — no existing helper to extend. Needs
   `git diff --name-status -M {base}...HEAD` (triple-dot, not double-dot, to match
   `test-affected.py`'s merge-base semantics; `-M` for rename detection `test-affected.py`
   lacks) plus decisions on whether to also union in uncommitted/untracked changes like
   `test-affected.py` does.
3. Bail-out list should ship as a short hardcoded Go-generic default (`go.mod`, `go.sum`,
   kibitzer's own affected-computation source) plus a config-overlay extension point
   (reusing `src/glob.rs`) for repo-specific codegen inputs/build config, not a verbatim
   port of stapler-squad's `FULL_RESCAN_TRIGGERS`.
4. Exit-code contract and bail-out reason string are worth building from day one — cheap,
   and a genuine improvement over `test-affected.py`'s stdout-only signal.
5. Follow the `ArchitectureAction` family's `--path` convention and "batch-only, never in
   `default_checks()`" doc-comment convention; deliberately diverge from its "always exit
   0" convention and justify why in the design doc.
