# Stack Research: affected-tests

## 1. Git operations: shell out to `git`, no `git2`/`gix` dependency exists

`Cargo.toml`'s `[dependencies]` block
([Cargo.toml:18-38](https://github.com/tstapler/kibitzer/blob/master/Cargo.toml#L18-L38))
has no `git2` or `gix` entry, and neither name appears anywhere in `Cargo.lock` either
(confirmed: `grep -n '^name = "git2"\|^name = "gix"' Cargo.lock` — no match), so there's
no transitive copy pulled in by another dependency either.

Every existing git interaction shells out via `std::process::Command::new("git")` and
parses stdout as text:

- `src/change_coupling.rs:67-75` (`git_log_commits`) — runs
  `git log --no-merges --name-only --pretty=format:\u{1} -n<limit>` and hand-parses the
  `\u{1}`-delimited output.
- `src/change_coupling.rs:227-...` (`git_log_commits_with_subject`) — same pattern, adds
  `%B` subject/body parsing.
- `src/hotspots.rs:39` — reuses the same `git log` shell-out for churn counts.
- `src/check.rs:1363-1374` (`map_ranges_to_head`) — runs
  `git diff --no-color -U0 HEAD -- <rel_path>` and parses unified-diff hunks by hand to
  remap changed-line ranges onto HEAD content. This is the closest existing precedent to
  what `affected` needs, but it's single-file and always diffs against `HEAD`, not an
  arbitrary base SHA across the whole tree — grepping for `diff --name-only` or
  `merge-base` anywhere in `src/*.rs` turns up nothing, confirming the requirements
  doc's claim that cross-file, arbitrary-base diffing is genuinely new plumbing.

**Recommendation: shell out, don't add a git library crate.** Reasons:

- It matches the repo's only established convention for git access (three call sites,
  zero library usage) — introducing `git2` (a libgit2 FFI binding, needs a C toolchain
  and OpenSSL/zlib at build time) or `gix` (pure Rust, but a large dependency surface
  and a different API idiom than anything else here) for one subcommand breaks that
  consistency for no functional gain: `affected` only needs `git diff --name-only
  <base>...HEAD` (changed file list) and possibly `git merge-base`, both single `git`
  invocations with easy-to-parse plain-text output — nowhere near the point where a
  library's object-graph API would pay for itself.
  - Note: `--name-only <base>...HEAD` (three dots, "merge-base" form) vs `<base>..HEAD`
    (two dots, direct diff) is a real decision `affected` will need to make explicitly —
    stapler-squad's `scripts/test-affected.py` is the reference for which stapler-squad
    actually uses today; not verified in this pass since it's outside this repo.
- `git2`/`gix` would add real build-time cost (git2's libgit2 C build; gix's dependency
  tree) for a subcommand that CI likely already has a real `git` binary available to
  (it's PR-time CI gating per the requirements' non-functional section) — shelling out
  costs nothing extra in that environment.
- cargo-dist (this repo's release pipeline, per the project's root `CLAUDE.md`) cross-
  compiles to multiple targets; `git2`'s native C dependency is a known source of cross-
  compilation friction that a pure shell-out avoids entirely.

## 2. `ArchModel`/`import_graph.rs`: what's reusable, what's missing

`ImportGraph` (`src/import_graph.rs:25-41`) is:

```rust
pub struct ImportGraph {
    pub nodes: BTreeSet<String>,
    pub edges: Vec<ImportEdge>,
    pub file_packages: BTreeMap<PathBuf, String>,
}
impl ImportGraph {
    pub fn edges_from<'a>(&'a self, node: &'a str) -> impl Iterator<Item = &'a ImportEdge>
}
```

`ImportEdge` (`src/import_graph.rs:15-20`) is `{ from: String, to: String, file: PathBuf,
line: usize }` — directed, forward edges only (`from` imports `to`).

`ArchModel` (`src/arch_model.rs:185-202`) wraps this: `import_edges: Vec<ImportEdge>` is
a clone of the built `ImportGraph`'s edges (`src/arch_model.rs:426`), plus
`file_packages`-equivalent grouping baked into `packages: BTreeMap<String, PackageNode>`.

**Reusable directly:**
- `file_packages: BTreeMap<PathBuf, String>` (`src/import_graph.rs:34`) is exactly the
  "which package does this changed file belong to" lookup `affected` needs to turn a
  `git diff --name-only` file list into seed graph nodes — already keyed by repo-relative
  `PathBuf`, already the single source of truth per its own doc comment.
- `import_edges: Vec<ImportEdge>` (`src/arch_model.rs:188`) is the forward adjacency list
  a BFS/DFS needs; each edge already carries the file/line that produced it, useful for
  explaining *why* a package was pulled in (a `--verbose`/`--explain` flag later).
- `edges_from` (`src/import_graph.rs:38-40`) gives forward traversal (what does node X
  import) for free, and its filter-by-`from` pattern is the template for the missing
  reverse version.

**Genuinely missing (confirmed by `grep -n "fn.*reverse\|fn.*importers\|fn.*edges_to" src/*.rs` — no match):**
- No reverse-adjacency index or `edges_to`/importers-of query. `edges_from` filters
  `self.edges` linearly by `from`; there's no equivalent by `to`, and no precomputed
  reverse map. `affected`'s core operation (given changed packages, find everything that
  transitively imports them) is exactly this missing direction — it would need either a
  linear scan per BFS layer (`O(edges)` per node, fine at kibitzer's target repo sizes but
  worth building a `BTreeMap<String, Vec<&ImportEdge>>` reverse index once instead) or a
  new `ImportGraph::edges_to`/reverse-index method added alongside `edges_from`.
- No BFS/DFS/reachability traversal exists anywhere in `import_graph.rs` or
  `arch_model.rs` today — `list_callers`/`list_callees` (surfaced via MCP per the
  project's `CLAUDE.md`) operate on `call_edges`/one-hop lookups for the LSP/MCP "jump to
  caller" use case, not multi-hop transitive closure. Confirmed no `fn bfs`/`fn dfs`/
  `fn reachable` anywhere in `src/*.rs`.
- No test-file association: `ImportGraph`/`ArchModel` model *packages*, not "which test
  binary/file covers this package" — for Go that mapping is trivial (same package
  directory, `_test.go` suffix) but is not something either type currently exposes; the
  new code will need to derive it from the file list directly rather than the graph.

## 3. `cache.rs`/`daemon.rs`: no ArchModel-fetching API — build fresh, use `ModelCache` in-process

Two independent caches exist, both confirmed by reading their full public surfaces
(`grep -n "^pub struct\|^pub fn\|^    pub fn" src/daemon.rs src/cache.rs`):

- **`cache::Cache`** (`src/cache.rs:60-66`) is a persisted, file-fingerprint-keyed cache
  of `CheckResult`s (findings), not of `ArchModel`s — "Persistent... cache of check
  results" per its own doc comment (`src/cache.rs:56-58`). Not usable for this.
- **`daemon.rs`**'s wire protocol (`src/daemon.rs:15-27`, `enum Request`) has exactly
  three ops: `RunChecks`, `Ping`, `Shutdown`. There is no RPC to fetch a built
  `ArchModel` or import graph — the daemon exists to keep a warm process alive across
  `run`/`hook` invocations so per-file check commands and tree-sitter grammar loads
  aren't repeated, not to serve a shared architecture model.
- **`arch_model::ModelCache`** (`src/arch_model.rs:716-723`) is the actual model cache,
  but explicitly *"single-slot, in-process, in-memory-only... No persistence, no daemon
  RPC"* (doc comment, `src/arch_model.rs:716-719`). It's instantiated fresh per process:
  `mcp.rs:455` (`model_cache: Arc::new(ModelCache::new())`) for the MCP server and
  `lsp.rs:314` for the LSP server — each long-lived server process gets one, and it never
  crosses a process boundary. `load_cached_model` (`src/arch_model.rs:834-850`, `pub(crate)`)
  is the helper that walks files, builds a `ModelCacheKey{repo_root, include_private}`,
  and calls `ModelCache::get_or_build`.

**Implication for `affected`:** a one-shot `kibitzer affected --base <sha>` CLI
invocation is a fresh process each run (unlike the long-lived MCP/LSP servers), so
`ModelCache`'s in-memory single-slot cache buys nothing across invocations — every CI
run rebuilds `ArchModel` from scratch via `build_model`/`load_cached_model`'s pattern.
The requirements doc flags this as "a research question not an assumption"
(Non-functional section) — the answer is: **no existing plumbing reuses the daemon or
persists `ArchModel` across process invocations today.** Making `affected` daemon-aware
(have it try `try_run_checks_via_daemon`-style IPC to a warm daemon that holds a built
`ArchModel`) is possible in principle but would require adding a new `Request` variant
and a server-side `ArchModel` cache to `daemon.rs` — real new work, not a reuse of
anything that exists. Given `build_model`'s cost is a repo walk + tree-sitter parse
(bounded by repo size, not history depth), benchmarking a from-scratch build against
`go list -deps`'s cost on stapler-squad's actual size before deciding this is worth
building is the right next step, not assumed up front.

## 4. Reverse-reachability traversal: hand-rolled BFS over `BTreeMap`, not `petgraph`

`petgraph` is confirmed absent: `grep -n "petgraph" Cargo.toml Cargo.lock` — no match in
either file, so it's not even a transitive dependency today.

**Recommendation: hand-rolled BFS, matching the repo's existing style.** Reasons:
- Every existing graph-shaped structure in this codebase (`ImportGraph`, `PackageNode`
  relationships, `CallEdge`/`FieldAccessEdge` lists in `ArchModel`) is a flat
  `Vec`/`BTreeMap`/`BTreeSet` of edges/nodes with linear-scan or hand-written index
  helpers (`edges_from`), not a `petgraph::Graph`. Introducing `petgraph` for one new
  subcommand would mean either (a) converting `ImportGraph` to a `petgraph` graph
  internally — a real refactor of shared code touching `Architecture Export`,
  `Architecture Diagram`, and the MCP tools that already consume `import_edges`, well
  outside this project's stated scope of reusing the existing model — or (b) building a
  parallel `petgraph::Graph` just for `affected`'s BFS, which is exactly the "second
  parallel dependency graph" the requirements' Constraints section explicitly forbids.
- The actual traversal need — reverse BFS from a seed set of changed packages, over an
  adjacency list with on the order of hundreds to low thousands of package nodes (repo
  package count, not file count) — is a textbook `VecDeque`-based BFS over a
  `BTreeMap<String, Vec<String>>` reverse-adjacency index built once from
  `import_edges`. This is well within hand-rolled territory; `petgraph` earns its keep
  for algorithms this codebase doesn't need (shortest path, strongly-connected
  components, topological sort with cycle detection as a first-class algorithm) rather
  than plain reachability.
- Concretely: build `reverse: BTreeMap<&str, Vec<&str>>` once from `ArchModel::import_edges`
  (grouping by `to` instead of `from`, the mirror of `edges_from`'s `by-from` filter),
  seed a `BTreeSet<String>` visited-set with the changed packages, and BFS outward
  following `reverse` edges — the same shape as any textbook graph BFS, no new crate
  required.

## 5. `clap` version and subcommand pattern

`clap = { version = "4", features = ["derive"] }` — `Cargo.toml:20`. No new dependency
or feature flag needed for a new subcommand.

The nested-subcommand shape to copy is already established in `src/main.rs`'s top-level
`Command` enum (`src/main.rs:69-137`): variants like `Daemon { #[command(subcommand)]
action: DaemonAction }` and `Architecture { #[command(subcommand)] action:
ArchitectureAction }` each delegate to their own nested enum. `affected` most resembles
`Schema { #[arg(long)] out: Option<PathBuf> }` (`src/main.rs:132-136`) or `Run` — a flat
struct variant with `#[arg(long)]` flags — rather than a nested-subcommand family, since
the requirements describe one subcommand (`kibitzer affected --base <sha>`) with flags,
not multiple actions:

```rust
Affected {
    #[arg(long)]
    base: String,
    // additional flags (e.g. --format json, --dir) as the plan phase decides
},
```

## Dependency additions summary

No new crate dependencies are needed for this feature: git access reuses
`std::process::Command`, graph traversal reuses hand-rolled BFS over existing
`BTreeMap`/`Vec` structures, and CLI parsing reuses the already-present `clap` "derive"
feature. The only new *code* is: a changed-file-list-via-git-diff helper (no existing
equivalent), a reverse-adjacency index + BFS over `ArchModel::import_edges` (no existing
equivalent), and the bail-out file-pattern check (build config/lockfiles/codegen inputs)
described in the requirements' Scope section (not researched in this pass — a policy
list, not a stack question).
