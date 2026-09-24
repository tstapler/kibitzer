# Research: Architecture — affected-tests

Scope: targeted reads of `src/arch_model.rs` (1969 lines), `src/import_graph.rs` (2723
lines), `src/cache.rs` (375 lines), `src/daemon.rs` (408 lines), `src/main.rs` (1064
lines), `src/config.rs` (1373 lines), `src/architecture_checks.rs`, `src/arch_export.rs`,
`src/change_coupling.rs` — all via `grep`/targeted `Read` (offset/limit), no whole-file
reads except where noted. Builds on prior analysis in
`project_plans/architecture-export/research/architecture.md` (the design doc for
`ArchModel`/`arch_model.rs` before it was built) and skims
`project_plans/kibitzer/research/architecture.md` (a separate, unrelated feature's
research — cited only for its `import_graph.rs`/`config.rs` precedents, which still hold).
Note: since the architecture-export research doc was written, `arch_model.rs` has been
built (1969 lines) — this doc reads the real, landed code, not the prior doc's proposal.

## 1. Integration point: `ArchModel.import_edges`, not `ImportGraph` directly

The CLI already holds a live example of exactly this choice. `arch_export.rs::run_export`
(`src/arch_export.rs:27-49`) does:

```
let (graph, files) = collect_repo_files(&repo_root)?;   // graph: ImportGraph
let model = build_model(&repo_root, &files, &graph, &prune)?;  // model: ArchModel
```

then exports `model`, not `graph`. `ArchModel { .. import_edges: Vec<ImportEdge> .. }`
(`src/arch_model.rs:185-202`) is populated verbatim from the `ImportGraph` it was built
from — `import_edges: import_graph.edges.clone()` (`src/arch_model.rs:426`) — so the two
are data-identical for edge purposes; `ArchModel` is a strict superset (adds `packages`,
`call_edges`, `field_accesses`, `file_import_aliases`, `pruning`).

**`affected`'s reverse-reachability walk should operate on `ArchModel.import_edges`,
not on `ImportGraph` directly**, for two reasons:
- The CLI's established loading path (`collect_repo_files` + `build_model`, see §2)
  already produces an `ArchModel`; there is no code path today where the CLI holds a
  bare `ImportGraph` and stops there.
- There's a direct precedent for a whole-graph algorithm consuming the raw edge `Vec`
  rather than going through `ImportGraph`'s own methods: `architecture_checks::fan_in_out`
  (`src/architecture_checks.rs:149`) takes `edges: &[ImportEdge]` and builds its own
  `HashMap<&str, (usize, usize)>` aggregation by iterating the vec directly, rather than
  calling `ImportGraph::edges_from` per node. A reverse-reachability walk is the same
  shape: build a `to -> Vec<&ImportEdge>` (or `to -> Vec<from>`) adjacency map once from
  `import_edges`, then BFS/DFS backward from the changed-file packages' keys.

**Gap: no reverse-edge helper exists today.** `ImportGraph::edges_from` (`src/import_graph.rs:38-40`)
only filters `e.from == node` — forward (a package's own imports), not reverse
(importers of a package). Nothing in `import_graph.rs` or `arch_model.rs` exposes
"importers of X" today; `affected` needs a new function (e.g.
`fn importers_of<'a>(edges: &'a [ImportEdge], node: &str) -> impl Iterator<Item = &'a ImportEdge>`,
or, more usefully for a transitive walk, a precomputed `to -> Vec<from>` map built once
per invocation) rather than an existing one it can call. This is new code, but small and
narrow, matching the existing edge-vec-consumer pattern rather than adding a new
graph-representation type — satisfies the "no second parallel graph" constraint.

Package-to-file mapping for the reverse walk's endpoints (translating "package P is
affected" back to "which files/packages does that mean for `go test`") should use
`PackageNode.files: Vec<PathBuf>` (`src/arch_model.rs:67-71`, via `ArchModel.packages:
BTreeMap<String, PackageNode>`) — already keyed identically to `ImportEdge::from`/`to`
(`src/arch_model.rs:240-245`'s `package_key_for_file` doc comment: package grouping is
guaranteed to match `ImportGraph`'s node keys, sourced from `ImportGraph.file_packages`,
`src/import_graph.rs:28-34`). No new file->package lookup needed.

## 2. Cache/daemon: synchronous, in-process access exists; it is NOT daemon-resident

There is a synchronous, non-async function that returns an already-built `ArchModel`
without any IPC round trip: `arch_model::load_cached_model(cache: &ModelCache, repo_root:
&Path, include_private: bool) -> Result<Arc<ArchModel>>` (`src/arch_model.rs:834-850`,
`pub(crate)`). It's documented as "Synchronous and blocking... Called from `mcp.rs`'s
`list_architecture_symbols`/`get_architecture_node` and `lsp.rs::build_index`"
(`src/arch_model.rs:824-828`).

But `ModelCache` (`src/arch_model.rs:721-773`) is explicitly **in-process, in-memory-only,
single-slot** — "No persistence, no daemon RPC" (`src/arch_model.rs:719`). It's a `Mutex<Option<(ModelCacheKey,
CachedModel)>>` owned by whatever process constructs it. Confirmed by grepping
`src/daemon.rs` for `ArchModel`/`ModelCache`: **zero hits**. `daemon.rs`'s cache
(`src/cache.rs::Cache`) is the separate, disk-persisted, per-file/per-check-result cache
already documented in the prior architecture-export research doc
(`project_plans/architecture-export/research/architecture.md:27-30`) — that finding still
holds verbatim; nothing in the landed code changed it.

Consequence for `affected` as a plain, one-shot CLI invocation (`kibitzer affected --base
<sha>`, run fresh per CI job): confirmed by reading `arch_export.rs::run_export`
(`src/arch_export.rs:27-49`) that **the CLI path does not use `ModelCache` at all** — it
calls `collect_repo_files` + `build_model` directly, a from-scratch walk+parse every
invocation, same as `arch_diagram.rs::run_diagram` presumably does (same helper). `ModelCache`
is reserved for long-lived processes (MCP/LSP servers) that field multiple requests in
one process lifetime.

**Answer to the "daemon cache to avoid a from-scratch rebuild every CI run" research
question: not available today, and not a small addition.** A CLI subcommand run once per
`git diff` in CI gets no benefit from `ModelCache` as it exists (a fresh process starts
with an empty slot). Getting cross-invocation reuse would require either (a) routing
`affected` through the daemon over IPC (`daemon.rs`'s `try_run_checks_via_daemon`/
`run_checks_smart`, `src/daemon.rs:237,325`, is the only cross-process code path today,
and it's shaped around per-file check results, not a whole-model query — would need a new
daemon request type) or (b) adding a `ModelCache` slot to the daemon process itself (the
type is already `Arc`-cloneable and mutex-guarded, so architecturally plausible — the
comment at `src/arch_model.rs:755-760` already reasons about concurrent daemon-side
callers) — genuinely new work, not a refactor. This matches the requirements doc's own
framing of it as "a research question, not an assumption" and explicitly Out of Scope
("daemon-cache-backed incremental recomputation (nice-to-have)"). **Recommendation: build
`affected` v1 on the same from-scratch `collect_repo_files`/`build_model` path
`arch_export`/`arch_diagram` already use, and leave daemon-cache integration as an
explicitly deferred follow-up**, not a prerequisite — consistent with the requirements'
own scope cut.

## 3. CLI dispatch pattern: pure `analyze()` fn + thin `run_*` handler in main.rs

`src/main.rs`'s `ArchitectureAction` enum (`src/main.rs:171-`, e.g. `ChangeCoupling`
at `src/main.rs:215-225`, `Hotspots` similarly) is the closest-shaped existing precedent —
also "diff a repo's history/state, walk something, emit a report," also `Batch`-only,
also not wired into `default_checks()`.

Concrete pattern, read end to end for `ChangeCoupling`:
- Library fn: `change_coupling::analyze(repo_root: &Path, limit: usize, top_n: usize) ->
  Result<Vec<CoupledPair>>` (`src/change_coupling.rs:384`) — takes a `Path`, opens git
  itself (via `git_log_commits_with_subject`, `src/change_coupling.rs:227`), returns
  structured data. No printing, no `ExitCode` — pure business logic + I/O for git only.
- CLI handler: `run_change_coupling(path: &Path, limit: usize, top: usize) ->
  Result<ExitCode>` (`src/main.rs:659-679`) calls `analyze`, then does all output
  formatting (`println!` loop) and always returns `Ok(ExitCode::SUCCESS)` when the
  analysis itself didn't error — "report, don't gate" convention, explicitly commented at
  `src/main.rs:681-683`.
- Dispatch: `main.rs`'s big match arm destructures the `ArchitectureAction` variant's
  fields and calls the handler (`src/main.rs:510-512`).

**`affected` should follow this exact shape**, with one deliberate deviation: unlike
`ChangeCoupling`/`RootCauseClusters`/`Hotspots` (which are always "report, don't gate,"
always `ExitCode::SUCCESS`), `affected` is consumed by `go test $(kibitzer affected ...)`
— its stdout contract (a package list, or a bail-out sentinel) is load-bearing, not
advisory. It still fits the same pure-fn/thin-handler split:
- `affected::compute_affected(repo_root: &Path, base: &str, ...) -> Result<AffectedResult>`
  (new module) — pure(-ish) result type distinguishing "package list" from "bail out,
  run everything," diffs `base` against `HEAD` (or working tree) via git, builds/consumes
  an `ArchModel` (per §1/§2), and does the reverse walk.
  `AffectedResult` should carry enough for the handler to format both the "consumable by
  `go test $(...)`" list output and a human-readable reason when bailing out (which glob
  matched, or which file), not just a bare `Vec<String>` — matching `PruningSummary`'s
  precedent of returning *which* files, not just a count (`src/arch_model.rs:125-127`).
- `run_affected(path, base, ...) -> Result<ExitCode>` in `main.rs`, alongside
  `run_change_coupling`/`run_hotspots`, formats the package list (newline- or
  space-separated, per the `go test $(...)` consumption pattern) or prints the bail-out
  sentinel, still `ExitCode::SUCCESS` on a successful *computation* (bail-out is a valid,
  successful answer, not an error) — errors are reserved for "couldn't diff," "couldn't
  build the model," etc.
- New `ArchitectureAction::Affected { path, base: String, scope: Option<String>, ... }`
  variant, since `affected` is fundamentally an `ArchModel` consumer exactly like
  `Export`/`Diagram`, not a `git`-only report like `ChangeCoupling`/`RootCauseClusters`
  (which don't touch `ArchModel` at all) — it belongs grouped with `Export`/`Diagram` in
  the enum, even though its handler-pattern precedent (pure fn + thin handler, always
  batch) comes from `ChangeCoupling`.

## 4. Bail-out/unbounded-blast-radius config surface: extend `Config`, mirror `Check.scope`'s glob-list shape

There is no existing "ignore/exclude glob list" in `config.rs` today (grepped for
`ignore`/`exclude` — no hits as a config field). The closest, and right, precedent is
`Check.scope: Vec<String>` (`src/config.rs:64-66`, doc comment: "Glob patterns (supporting
`**`) a file path must match for this check to apply"), consumed everywhere via
`glob::matches_scope`. Every existing "list of glob patterns a user can configure" in this
repo has that exact shape: `Vec<String>` + `#[serde(default)]` + matched through the one
shared `glob::matches_scope` function — never a bespoke pattern language per feature
(explicitly the architecture-export research doc's own conclusion at
`project_plans/architecture-export/research/architecture.md:147`: "reuse
`glob::matches_scope` rather than inventing a second glob dialect").

`Config` (`src/config.rs:371-381`) already has a precedent for a *named, nested*
feature-specific config block sitting alongside `checks`/`disabled`:
`pub architecture: ArchitectureConfig` (`src/config.rs:375`, `#[serde(default)]`). This is
the shape a bail-out list should take — **a new `pub affected: AffectedConfig` field**,
not a bolt-on to `ArchitectureConfig` (which is about layering/naming/component rules, a
different concern) and not a reuse of `Check.scope` (which scopes *checks* to files, not
*this specific subcommand's* blast-radius policy). `AffectedConfig` would hold something
like `pub bail_out_globs: Vec<String>` with sane hardcoded defaults (build config,
lockfiles, codegen inputs, kibitzer's own affected-computation source) applied when the
field is absent — following `default_checks()`'s own "hardcoded defaults, overlaid not
replaced by repo config" convention described in this repo's `CLAUDE.md` at
`Default check catalog`. Concretely: `AffectedConfig` derives `Deserialize, JsonSchema`
matching every other `config.rs` struct (e.g. `Component`, `src/config.rs:122-123`), so it
flows automatically into `kibitzer schema` (`Command::Schema`, `src/main.rs:132-136`)
without a separate schema-generation path to maintain.

Renamed/moved-file and merge-commit diff handling (the other two open questions) are a
`git diff` mechanics question, not an architecture one — out of scope for this doc, but
worth flagging for the plan phase: `change_coupling.rs`'s git plumbing
(`git_log_commits_with_subject`, `src/change_coupling.rs:227`) is the only existing
git-shelling-out precedent in this codebase and is a `git log`, not `git diff`, walk — it
doesn't answer rename detection, so that part needs fresh research/design, not reuse.

## 5. Disposition call: Extend as-is

**Extend as-is.** `arch_model.rs`/`import_graph.rs` are large (1969/2723 lines) but not
because they're tangled — the size comes from breadth (symbol extraction, call-edge
resolution, field-access resolution, pruning, `ModelCache`, all across multiple
languages), and every piece read here was a clean, single-purpose function with a
docstring explaining its one job (e.g. `package_key_for_file`, `src/arch_model.rs:253-261`;
`fan_in_out`, `src/architecture_checks.rs:149`). `import_edges: Vec<ImportEdge>` is a
plain, stable, already-multiply-consumed data shape (checkers, `fan_in_out`, the diagram
renderer, and now `affected`) — exactly the "one model, many views" pattern this repo's
own `MEMORY.md` names as a load-bearing convention. `affected` adds one more consumer
(a reverse-adjacency-map builder + BFS) and one small new library function
(`importers_of`/equivalent); it does not need to reshape either file, and reshaping them
first would violate the same "don't build a second graph" constraint the requirements
doc calls out. No SOLID/Clean/DDD boundary violation observed in the read sections —
the pure/I-O split (`build_model` pure, `arch_export.rs`/`main.rs` handle I/O and
formatting) is intact and is exactly the seam `affected` should plug into.

## 6. EventStorming Event-Command-Policy table

Skipped, per the requirements doc's own instruction. This is a single-repo CI tooling
feature (diff → walk → emit list), one actor (a CI job or a human running the CLI), no
multi-party workflow or competing-actor state machine — same reasoning the
architecture-export research doc used to skip it
(`project_plans/architecture-export/research/architecture.md:69`).
