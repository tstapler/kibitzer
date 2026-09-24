# Pitfalls: affected-tests

Research agent 4 (Pitfalls), SDD Phase 2. All file:line citations are against this
worktree's checkout of `tstapler/kibitzer` (branch
`tstapler/triage-185caafd-ccaf-4f3a-8296-4e55ed34a9fe`) unless marked "industry
pattern (not repo-verified)".

## 1. Tree-sitter Go import extraction — what it does NOT handle

Read `src/import_graph.rs`'s Go path: `go_lang_config()` (`src/import_graph.rs:437`),
`find_go_mod_upward` (`src/import_graph.rs:270`), `go_package_identity`
(`src/import_graph.rs:327`), `collect_go_imports`/`go_import_spec`
(`src/import_graph.rs:339`, `:423`).

- **No `replace` directive support.** `find_go_mod_upward` parses only the `module `
  line of `go.mod` (`src/import_graph.rs:274-278`); it never reads a `replace`
  directive. In a multi-module repo where module A's `go.mod` has
  `replace github.com/org/b => ../b`, a file in A that imports
  `github.com/org/b` gets that literal string as its edge target
  (`go_import_spec`, `:423-435`), but B's own package identity — computed from
  B's *own* `go.mod`'s `module` line — will be whatever B's go.mod declares, which
  need not be `github.com/org/b`. The graph-membership guard (edges are kept only
  when the target matches a node some other file in the run declared — see the
  module doc comment at `src/import_graph.rs:109-120`) means this edge silently
  fails to attach to B's node. Reverse-reachability from B outward would then miss
  A entirely — a real under-detection risk for exactly the "must never silently
  under-test" case the requirements call out, and specific to monorepos using
  local `replace` directives (a common Go monorepo pattern).
- **No build-tag (`//go:build`) awareness.** Nothing in `import_graph.rs` inspects
  build constraints or `_linux.go`/`_darwin.go` filename suffixes. Every `.go` file
  handed to `build()` is parsed and its imports extracted regardless of which
  platform/build-tag combination would actually select it at compile time. Net
  effect for `affected`: the graph can report edges (and therefore affected
  packages) for a platform variant that wouldn't even be compiled in the CI
  environment being gated — over-inclusion, not under-inclusion, so it's the safer
  direction, but it can still produce a confusingly large affected set, and the
  reverse case (an edge that's real for the *other* platform) is exactly as
  invisible as any other build-tag blind spot.
- **Vendored dependencies are handled, but at a different layer.** `import_graph.rs`
  itself has no vendor-awareness, but `vendor` directories are already excluded at
  the whole-repo walk (`SKIP_DIRS` in `src/check.rs:1483-1501`, consumed by
  `walk_and_collect_files` at `src/check.rs:1470`, which is what feeds file lists
  into `import_graph::build` everywhere it's called — `src/arch_model.rs:788`,
  `src/check.rs:1295`, etc.). **This only protects `affected` if its diff-driven
  file list is filtered the same way.** `git diff --name-only` returns raw
  repo-relative paths with no `SKIP_DIRS` filtering; a naive `affected`
  implementation that feeds those paths straight into graph lookups (rather than
  routing them through the same skip-list / walk semantics) could treat a
  `vendor/`-only diff as either spuriously affecting something or spuriously
  triggering a bail-out.
- **Generated code**: see the dedicated answer below — `looks_generated` does
  *not* cover the generated-code case `affected` needs.

### Does `looks_generated` already cover the generated-code case for `affected`?

**No — it solves a different, narrower problem.** `looks_generated`
(`src/arch_model.rs:224-229`) is a content-banner heuristic ("do not edit" /
"code generated", case-insensitive, scanned line-by-line) invoked only inside
`build_model` (`src/arch_model.rs:348`) to skip generated *output* files from
symbol/architecture extraction (reduces noise in the architecture model). Two
gaps for `affected`:

1. **It never runs during `import_graph::build`.** `import_graph::build` takes
   only `&[PathBuf]` (`src/import_graph.rs:49`) and reads/parses every file's
   imports itself — there is no `looks_generated` call anywhere in
   `import_graph.rs` (confirmed by grep: the only hits are in `arch_model.rs`).
   So generated Go files (e.g. `*.pb.go`) are fully present in the import graph
   with real edges — which is actually *correct* behavior for reverse
   reachability (their imports are real), just worth knowing it's not the same
   gate `build_model` applies.
2. **It says nothing about codegen *inputs*.** The actual requirements-flagged
   risk is a change to a `.proto`/`.sql`(sqlc)/schema file that regenerates
   `.pb.go` (or equivalent) — but `.proto` files aren't Go/TS/JS/Java/Kotlin/Python,
   so they never enter `import_graph.rs` at all, and a diff touching only the
   `.proto` produces zero graph edges to walk. `looks_generated` cannot help here
   because it operates on files already selected for parsing, not on the
   diff's changed-file list before the graph is even consulted. This is exactly
   the "codegen inputs" unbounded-blast-radius bail-out case requirements.md
   already flags — the design needs its own explicit check (e.g. a
   glob/extension list of known codegen-input file types) independent of
   `looks_generated`.

## 2. Cycle safety for reverse-reachability

**The codebase does not assume the import graph is acyclic — quite the opposite,
it has a dedicated cycle-detection checker.** `find_cycles`
(`src/architecture_checks.rs:423-488`) runs full Tarjan SCC on `ImportGraph` and
is wired up as the `import-cycles` checker (`src/architecture_checks.rs:54-58`).
Its own test suite includes `go_import_graph_finds_a_two_package_cycle`
(`src/import_graph.rs:1477`) — i.e., kibitzer's Go import-graph construction can
and does produce genuine two-node cycles in test fixtures (a repo mid-refactor,
or with a real bug, will not compile with Go's own toolchain, but kibitzer's
static tree-sitter extraction has no compile step and will happily represent it).

Conclusion for the design: **a reverse BFS/DFS walk for `affected` must carry an
explicit visited-set**, regardless of language. There is no DAG invariant
anywhere in `import_graph.rs`/`arch_model.rs` to lean on (no assertion, no
comment claiming acyclicity) — relying on "Go doesn't have import cycles" would
be relying on an invariant of *compiled* Go, not of this graph representation.
The requirement doc's own caveat that JS/TS *does* allow real cycles just makes
this doubly true once that language is added — the same visited-set logic must
already be correct for Go today, not deferred as "add cycle protection later
when JS/TS lands."

## 3. Git diffing pitfalls

**No existing kibitzer code diffs against an arbitrary base ref — confirmed.**
The only git-diff-for-scoping code in the repo is `map_ranges_to_head`
(`src/check.rs:1363-1378`), which hardcodes `git diff --no-color -U0 HEAD --
<rel_path>` for a single file's hunks. No `-M`/`--find-renames`, no
`merge-base`, no configurable base ref, no batching across a changed-file list.
Every other `Command::new("git")` call in the repo (`git show`, `git archive`,
`git log`, `git rev-parse` for status checks) is similarly single-purpose;
grep confirms none of them pass `-M`/`--find-renames` or call `merge-base`.

- **Shallow clones are a real, already-hit problem in this repo's own CI**, not
  just an industry pattern: `.github/workflows/ci.yml:15-19` sets
  `fetch-depth: 0` with the comment "covgate diffs against origin/master to
  scope its coverage gate to this PR's changed lines; a shallow checkout
  wouldn't have that ref." That's a different tool (`covgate`) hitting the exact
  failure mode `affected --base <sha>` would hit under GitHub Actions' default
  `fetch-depth: 1`: `git merge-base`/`git diff <base>...HEAD` against a base
  commit not present in the shallow history fails (or, depending on git
  version/flags, silently resolves to the wrong commit). The design must either
  document a required `fetch-depth: 0` (or `fetch-depth: N` with a fallback
  `git fetch --deepen`) the same way `covgate`'s consumer does, or detect
  shallow state (`git rev-parse --is-shallow-repository`) and bail out loudly
  rather than compute a wrong, narrower-than-real diff.
- **A base ref that's a branch name that's since moved**: `git archive <base>`/
  `git diff <base>` resolve a branch name at call time, not at whatever moment
  the caller intended (e.g. CI trigger time). If a long-running job holds a
  branch name rather than a resolved SHA, and someone pushes to that branch
  meanwhile, the "base" silently becomes newer than intended, shrinking (or
  distorting) the diff. General pattern (not repo-verified) but the fix is
  cheap and should be load-bearing in the design: resolve `--base` to a SHA via
  `git rev-parse <base>` once, up front, and use that resolved SHA for every
  subsequent git call.
- **Binary files in the diff**: `git diff --name-only`-style output includes
  binary files without a way to distinguish them from text without `-a`/`--numstat`
  inspection (industry pattern, not repo-verified in this codebase — nothing in
  `check.rs`'s existing diff code handles binaries since it only ever diffs
  known source files already scoped to a supported language). `affected` will
  see arbitrary changed paths from a real diff, so it needs to explicitly
  classify/skip or bail-out on binaries rather than assume every changed path is
  parseable text.
- **Renames**: no code in this repo passes `-M`/`--find-renames` to any `git
  diff`/`log` invocation (confirmed by grep across `check.rs`, `change_coupling.rs`,
  `hotspots.rs`, `root_cause_clusters.rs`, `mcp.rs`, `main.rs`). Note
  `src/change_coupling.rs:14-16` explicitly *excludes* mass-rename commits as
  noise for its own unrelated purpose (temporal-coupling), which is a different
  problem (noise filtering) from `affected`'s correctness problem: without `-M`,
  git reports a rename as a delete + an add, and `affected`'s reverse-reachability
  would need to recognize that the "new" path's package identity is
  the same package as the "old" path's importers depended on — otherwise a pure
  rename with no logic change looks like "delete an entire package's edges, add
  a brand-new disconnected one," which is both a false-negative (importers of
  the old path no longer show as affected once it's "deleted") and potential
  false-positive noise.

## 4. ArchModel staleness vs. a diff's base ref

This is the sharpest structural gap found. `ModelCache`
(`src/arch_model.rs:698-773`) is the existing cache `affected` would be tempted
to reuse:

- `ModelCacheKey` (`src/arch_model.rs:698-701`) is keyed on `{repo_root,
  include_private}` only — **no git ref/SHA field at all.**
- Freshness is decided purely by per-file on-disk `mtime`/length stamps
  (`crate::cache::stamp`, referenced at `src/arch_model.rs:740-750`) compared
  against the working tree at call time. The cache has no concept of "which
  commit this model reflects" — it reflects "whatever is on disk right now."

That means `ModelCache::get_or_build` is answering "is the working tree
unchanged since I last built?", not "does this model reflect the diff's base or
head?" Two concrete failure shapes for `affected --base <sha>` if it naively
calls into this cache:

1. **Working-tree drift within a single CI job is a non-issue in practice**
   (the checkout doesn't change under a running job), but reusing this same
   `ModelCache` slot across multiple `affected` invocations in a long-lived
   process (the MCP/LSP/daemon modes this cache exists for —
   `src/arch_model.rs:716-719` says "no daemon RPC" but the cache type itself is
   generic) risks serving a model built for a previous checkout/branch if the
   caller ever swaps `repo_root`'s contents between calls without the stamps
   catching every changed file (e.g. a file whose content changed but whose
   mtime/length happen to collide — noted as a real risk the cache's own docs
   flag only for *symmetric* content, at `src/arch_model.rs:1928`, but the
   general mtime-based-staleness class is a known limitation, not something the
   cache defends against with content hashing).
2. **The real structural gap: one snapshot cannot correctly answer "affected by
   a diff" at all**, cache or no cache. `affected` needs, for a *deleted* file,
   to find who imported it *before* the deletion — but a model built from the
   current working tree (post-diff / HEAD) has no node for a deleted file, so a
   reverse-reachability query against a HEAD-only graph would return nothing for
   it, silently missing every importer of now-deleted code. The existing prior
   art for building a model at a *specific* ref — `check_native_against_git_head_repo`
   (`src/check.rs:1241-1316`) — does exactly the right shape of thing (`git
   archive HEAD | tar -x` into a temp dir, then `walk_and_collect_files` +
   `import_graph::build`/`build_model` against that snapshot, `src/check.rs:1256-1304`)
   but is hardcoded to `HEAD` and bypasses `ModelCache` entirely (no caching, a
   fresh archive+walk+parse every call). `affected` needs the two-sided version
   of this: build (or otherwise obtain) graphs for **both** `<base>` and
   `HEAD`/working tree, and union reachability computed from each — importers of
   a file as it existed at `<base>` (to catch deletions/moves) *and* importers
   of a file as it exists at `HEAD` (to catch new imports of changed files) —
   rather than trusting a single cached-or-fresh snapshot to answer both
   directions.

## 5. CI-integration pitfalls

- **Exit-code ambiguity is already a live pattern in this codebase, not a
  hypothetical.** `main()` returns `Result<ExitCode>` (`src/main.rs:327`); a
  native check with findings explicitly returns `ExitCode::from(1)`
  (`src/main.rs:401`, and again at `src/main.rs:449` for backtest, `:612`/`:652`
  for architecture checks). Any `?`-propagated `Err` from deeper in the call
  chain (git command failed, file not readable, parse failure) surfaces through
  the same `Result<ExitCode>` return and — via Rust's blanket `Termination` impl
  for `Result<T: Termination, E: Debug>` — *also* exits with a non-zero code
  (conventionally 1), printing the Debug-formatted error to stderr rather than
  stdout. **A CI wrapper watching only the exit code cannot distinguish
  "findings, task worked as intended" from "the tool itself errored."** For a
  correctness-critical feature whose whole premise is "must never silently
  under-test," `affected` reusing this same undifferentiated 0/1 contract would
  mean a git failure (e.g. shallow-clone `merge-base` miss, from pitfall 3)
  looks identical, at the exit-code level, to a real narrowed-but-valid package
  list — a wrapper that treats "non-zero, must be findings" the naive way could
  go either direction wrong. The design should reserve a distinct exit code (or
  a machine-readable status field in its own stdout contract) for "error, could
  not compute affected set — you must run everything," separate from both "here
  is a valid narrowed set" and "here is the run-everything bail-out sentinel."
- **The "ambiguous empty output" trap** is the sharper version of the same
  problem: if `affected` prints nothing to stdout, a wrapper cannot tell
  "computed a valid answer: zero packages affected, skip tests" from "crashed
  before printing anything, ran nothing, and CI happily reports green." The
  bail-out sentinel called for in requirements.md (a "run everything" signal)
  needs to be a distinct, explicit token from both "empty list" and "no output
  at all" — never let an empty stdout body do double duty as a semantic answer.
- **Partial/truncated stdout on large output**: general CI pattern, not
  repo-verified — a large monorepo diff cascading to thousands of affected
  packages could produce megabytes of stdout; some CI log capture layers
  truncate very long single lines or very large total output. If the design
  emits one huge JSON blob or one huge newline-delimited list, a truncated read
  on the consumer side degrades silently into a partial (looks-valid but is
  actually incomplete) package list — arguably worse for a correctness-critical
  feature than an outright empty/error signal, since "some real answer, but
  truncated" is much harder for a wrapper to detect than "no answer." Consider
  bounding output size explicitly (write to a file path and print a summary +
  path, the same "avoid unbounded stdout" shape kibitzer's own `run_checks`-style
  commands already lean toward for large findings sets) rather than one
  unbounded stdout stream.

## Summary of what's repo-verified vs. general pattern

Repo-verified (file:line cited above): no `replace`-directive handling, no
build-tag awareness, vendor exclusion lives in `walk_and_collect_files`/`SKIP_DIRS`
not `import_graph.rs`, `looks_generated` only gates `build_model`'s symbol
extraction and never runs inside `import_graph::build`, `find_cycles`/Tarjan
proves the graph has no acyclicity invariant, no existing git-diff code uses
`-M`/`merge-base`/an arbitrary base ref, this repo's own CI already had to set
`fetch-depth: 0` for a different diff-scoped tool, `ModelCache` has no git-ref
key and is keyed purely on working-tree mtime/length stamps, and the CLI's
existing exit-code contract already conflates "findings" and "error" at the
`ExitCode` level.

General industry patterns flagged but not repo-verified: binary files in a diff,
a moved branch-name base ref, partial/truncated stdout on very large output.
