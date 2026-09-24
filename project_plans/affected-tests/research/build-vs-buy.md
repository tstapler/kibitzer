# Build vs. Buy: `kibitzer affected --base <sha>`

Research agent 6 output for SDD Phase 2, project `affected-tests`. Scope: evaluate
build-vs-buy for the two decomposed sub-components (graph traversal, git diffing), plus
reference-implementation reuse and bespoke-vs-library correctness risk.

## Evidence gathered

- `Cargo.toml` `[dependencies]` (repo root): no `petgraph`, `git2`, `gix`, or
  `libgit2-sys` present.
- `Cargo.lock`: `grep -n -i -E '^name = "(petgraph|git2|gix|libgit2-sys)"'` returns no
  matches — none of these are pulled in even transitively.
- `src/import_graph.rs:25-38`: `ImportGraph` is a plain struct —
  `nodes: BTreeSet<String>`, `edges: Vec<ImportEdge>`, `file_packages:
  BTreeMap<PathBuf, String>` — with an `edges_from` iterator helper. No graph-crate
  types anywhere in the 2723-line file.
- Git access convention: `grep -rn 'Command::new("git")' src/*.rs` returns 20+ call
  sites across `src/check.rs`, `src/hotspots.rs`, `src/change_coupling.rs`,
  `src/mcp.rs`, `src/main.rs`, `src/root_cause_clusters.rs`, `src/task_stop.rs`,
  `src/hook_log.rs`. Every one shells out via `std::process::Command`; none use a git
  library. `src/hotspots.rs:1-8`'s module doc comment explicitly cites this as a
  deliberate, named convention shared with `change_coupling.rs`/`root_cause_clusters.rs`.
- `src/hotspots.rs` and `src/change_coupling.rs` (the two closest analogs to
  `affected` — both are batch git-history reports) hand-roll their own algorithms
  (churn scoring, revision counting) with no graph or algorithms crate; `grep -n
  "petgraph\|graph"` over both files turns up nothing but the word "graph" in prose
  comments.

## 1. Reverse-reachability graph traversal

**(a) Hand-rolled BFS over existing `BTreeMap`/`Vec<ImportEdge>`**

- Pros: Zero new dependency — no binary-size, compile-time, or supply-chain surface
  added. Matches the repo's own established precedent: `hotspots.rs` and
  `change_coupling.rs`, the two most analogous existing batch reports, both hand-roll
  their algorithms rather than reaching for a crate. Works directly against
  `ImportGraph`'s existing shape (`edges_from`, `BTreeSet<String>` nodes) with no
  conversion step, so it stays inside the "one model, many views" constraint by
  construction. The algorithm itself — invert an edge list into a reverse adjacency
  map, then BFS/DFS from a seed set — is standard-library-level Rust: `HashMap<String,
  Vec<String>>` plus a `VecDeque` and a visited `HashSet`, maybe 30-50 lines.
- Cons: Kibitzer's own maintainer (not a crate maintainer) is responsible for BFS
  correctness — cycle handling, visited-set bookkeeping — though this is a well-known,
  low-complexity algorithm, not a novel one. No built-in shortest-path/weighted variants
  if a future feature needs them (not a current requirement).
- Verdict: **Recommended.** The graph is package-level (small-to-medium node count,
  per requirements.md), the algorithm is simple reverse-BFS with no weights or
  shortest-path need, and it directly reuses `ImportGraph` without any adaptation
  layer — the strongest fit for both the "one model" architectural constraint and the
  solo-maintainer low-overhead constraint.

**(b) Adopt `petgraph`, convert `ImportGraph` edges into a `petgraph::Graph`**

- Pros: Battle-tested traversal algorithms (BFS, DFS, Dijkstra, topological sort)
  available off the shelf; useful if the feature set grows to need weighted paths or
  more exotic graph queries later.
- Cons: New dependency with zero existing precedent in this codebase — not currently
  in `Cargo.toml` or `Cargo.lock`, direct or transitive, confirmed by grep. Requires a
  conversion/adapter layer between `ImportGraph`'s native `BTreeMap`/`Vec<ImportEdge>`
  representation and `petgraph::Graph`'s node-index-based API, which is itself a
  second, parallel representation of the same data — in tension with the "one model,
  many views" convention the requirements explicitly call out (a `petgraph::Graph`
  conversion is a second graph model, not a view over the first, unless kept strictly
  ephemeral and rebuilt per-call). Adds compile-time and binary size for a
  small-to-medium graph where the win is marginal. No other report in the codebase
  uses a graph crate, so this would be a one-off dependency with no shared benefit
  elsewhere in the codebase today.
- Verdict: **Viable, not recommended.** Not wrong on technical merits, but
  disproportionate to the problem size and against both the repo's dependency-adding
  precedent and its "one model" architectural rule, for a solo-maintainer personal
  tool. Revisit only if a later feature genuinely needs weighted-graph algorithms
  petgraph provides and hand-rolling would be the second such demand.

## 2. Git diffing against a base ref

**(a) Shell out to `git diff --name-only <base>...HEAD` via `std::process::Command`**

- Pros: Matches the codebase's existing, pervasive convention exactly — 20+ call
  sites across 8 files already shell out to `git` for log/diff/rev-parse/status
  operations, including the two closest analogs (`hotspots.rs`, `change_coupling.rs`).
  `hotspots.rs`'s doc comment explicitly names this as a shared, intentional pattern.
  Zero new dependency. Delegates ref resolution, merge-base handling, and diff
  semantics entirely to the user's installed `git` binary, which is guaranteed present
  in this tool's target environment (a personal dev-tooling CLI run from inside git
  repos).
- Cons: Requires a `git` binary on `PATH` (already an implicit requirement of every
  other git-touching feature in this codebase, so not a new constraint). Output
  parsing (splitting `--name-only` lines, handling rename records if `-M` is used) is
  hand-written string parsing rather than typed API access — matches existing
  precedent, e.g. `hotspots.rs`'s own diff-hunk parser (`map_ranges_to_head`).
- Verdict: **Recommended.** Directly consistent with 20+ existing call sites and an
  explicitly documented convention; introduces no new dependency; the diff operation
  needed here (`--name-only` between two refs) is simpler than the diff-hunk parsing
  `hotspots.rs` already does successfully.

**(b) Adopt `git2` (libgit2 bindings) or `gix` (pure-Rust) for in-process git access**

- Pros: Typed API, no subprocess spawn overhead, no dependency on `git` being on
  `PATH` (though that dependency already exists elsewhere in this codebase). `gix`
  avoids `git2`'s C library (libgit2) linkage entirely if a pure-Rust dependency chain
  is a goal.
- Cons: New dependency with zero existing precedent — confirmed absent from
  `Cargo.lock` even transitively. `git2` links libgit2 (a C library) via
  `libgit2-sys`, adding build complexity (a C toolchain requirement) this Rust-only
  codebase doesn't currently have anywhere. `gix` avoids the C dependency but is a
  large, actively-evolving crate whose API surface is bigger than the one operation
  needed here (`diff --name-only <base>...HEAD`). Neither is justified when the
  existing shell-out convention already solves harder git problems in this codebase
  (diff hunk parsing in `hotspots.rs`, multi-format `git log` parsing in
  `change_coupling.rs`).
- Verdict: **Not recommended.** Pure scope creep relative to the one git operation
  this feature needs, contradicts the established and explicitly-documented shell-out
  convention, and adds either a C build dependency (`git2`) or a large new crate
  surface (`gix`) with no offsetting benefit for a solo-maintainer tool.

## 3. Reference implementation reuse

- **`digitalocean/gta`** (Go, "get test affected" tool): INFERRED from training
  knowledge, not verified against the actual source in this session — no network
  fetch of the repo was performed. From recollection, `gta` builds a Go import graph
  via `go list -deps`/`golang.org/x/tools/go/packages`, diffs the working tree against
  a base ref, and walks the import graph in reverse to find affected packages — the
  same high-level algorithm shape this feature needs (changed files → package resolution →
  reverse-reachability → affected set). The algorithm shape is generic enough
  (build reverse adjacency, BFS from seeds) that it's a reasonable one-paragraph design
  reference, not literal portable code: it's Go-specific (uses Go's own package-resolution
  tooling), and this project already has its own package-key resolution logic in
  `import_graph.rs`'s per-language builders, which differs from `go list`'s semantics
  (kibitzer's tree-sitter-based Go import extraction is one of several language front
  ends, not standalone `go list` integration). Net: useful for confirming the
  algorithm's shape (reverse-BFS from a changed-file seed set) is a known, validated
  pattern, not for code reuse.
- **Rust-native "affected packages from git diff" tool to wrap instead of
  reimplementing**: none comes to mind as an existing, focused tool solving exactly
  this problem (git diff → dependency-graph reverse-reachability → affected package
  list) as a standalone Rust crate or CLI. Cargo's own workspace tooling
  (`cargo-nextest`'s `--changed-since` or similar, if present) is Cargo-workspace-specific
  and wouldn't apply to kibitzer's cross-language (Go/TS/JS/Java/Kotlin/Python) scope, and
  is UNVERIFIED to even exist as described — flagging rather than asserting it. No
  confident recommendation of a wrappable tool; treat this as an area with nothing to
  buy, reinforcing the build case above.

## 4. LLM-generated bespoke graph-walk vs. tested library — correctness-risk assessment

The requirements' explicit constraint is that this feature must never silently
under-test (a false-negative "affected set" that misses a package whose tests should
have run is the worst failure mode — worse than an over-broad "run everything"
fallback, which the design already has as an escape hatch for unbounded blast radius).
That risk profile argues for whichever approach is *easiest to verify exhaustively*,
not whichever has the most implementation heritage:

- A hand-rolled reverse-BFS over `ImportGraph` is a small, self-contained,
  easily-unit-tested piece of code (build reverse adjacency from `edges: Vec<ImportEdge>`,
  BFS from seed nodes, collect visited set) — the kind of algorithm where 100% branch
  coverage in unit tests is cheap and where a bug would show up immediately in
  targeted tests (e.g., "changing file X should mark package Y affected" fixtures
  mirroring `import_graph.rs`'s own existing test style at the bottom of that file).
  The correctness-critical part of this feature is not the graph algorithm (BFS is
  well-understood) — it's package-key resolution (changed file → correct package
  node) and edge-construction correctness, both of which already live in
  `import_graph.rs` and are exercised by its existing test suite. Adding a graph crate
  doesn't reduce risk in that part at all, since the risk is in the adapter/resolution
  layer regardless of which traversal implementation sits downstream of it.
- A dependency (`petgraph`) shifts correctness risk for the traversal itself onto a
  well-tested library, but adds a new failure surface at the conversion boundary
  (`ImportGraph` → `petgraph::Graph` and back), which is exactly the kind of adapter
  code most likely to hide the "silent under-test" bug the requirements warn about — a
  wrong node-index mapping would raise no error, just silently return a wrong package
  set. Bespoke BFS avoids this extra boundary entirely by consuming `ImportGraph`'s
  own types directly.
- Conclusion: for this specific algorithm, bespoke-and-tested is *lower* correctness
  risk than dependency-and-adapted, because the adapter layer a dependency would
  require is a worse silent-failure surface than the BFS logic it replaces. This
  reinforces the build recommendation in section 1(a) — not on maintenance-cost
  grounds alone, but because it removes a failure mode.

## Summary table

| Sub-component | Recommended | Rejected (viable/not recommended) |
|---|---|---|
| Reverse-reachability traversal | Hand-rolled BFS over `ImportGraph`'s existing `BTreeMap`/`Vec<ImportEdge>` | `petgraph` (viable but disproportionate; second graph model) |
| Git diffing vs. base ref | Shell out via `std::process::Command` (matches 20+ existing call sites) | `git2`/`gix` (not recommended; scope creep, C-toolchain or large-crate cost) |
| Reference implementation | `digitalocean/gta`'s algorithm shape as design reference only (INFERRED, unverified); no wrappable Rust-native tool found | Literal porting (not applicable — Go-specific, different package-resolution semantics) |
| Bespoke vs. library (overall) | Bespoke, tested against `import_graph.rs`-style fixtures | Library adoption shifts risk to an adapter boundary — worse for the "never silently under-test" constraint |
