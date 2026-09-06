# Architecture Review: architecture-export
**Date**: 2026-08-24
**Verdict**: CONCERNS

## Constitution Violations
<none — no constitution file exists>

## Blockers
<none — both prior blockers verified resolved, see below>

### Verified resolved
- **`SymbolNode::id` collision** (was blocker 1). The id scheme is now owner-qualified:
  `"{package_path}::{parent}.{name}"` for methods, `"{package_path}::{name}"` otherwise
  (Domain Glossary, `plan.md:60`; Pattern Decisions table, `plan.md:90`). Story 1.2.2 adds
  an explicit uniqueness AC and GWT example with two same-named methods (`Close`) on
  different types (`A`, `B`) in one package producing distinct ids (`plan.md:292-296`), and
  Task 1.2.2e's test list explicitly includes "the same-name/different-owner uniqueness
  case" (`plan.md:319`). Story 3.1.2 has a matching AC/test for `get_architecture_node`
  resolving `"<pkg>::A.Close"` to the right method, not `B`'s (`plan.md:592-595`,
  `plan.md:606`). The Java-overload accepted-limitation note (Epic 5.2, `plan.md:861-871`)
  is concrete, not hand-waved: it names the exact collision (`save(String)` vs.
  `save(String, int)`), states the resulting behavior precisely (last-extraction-wins in
  `PackageNode.symbols`), and gives a scoped revisit trigger (user report of an ambiguous
  lookup) rather than leaving it open-ended. An implementer hitting this in Phase 5 has
  enough to act on without guessing.
- **`ModelCacheKey` scope fragmentation** (was blocker 2). Checked all five locations named
  in the repair claim plus the Pattern Decisions table — all consistent, no stray `scope`
  anywhere:
  - ADR-002 (`plan.md`'s sibling ADR, lines 26-45): `ModelCacheKey { repo_root, include_private }`,
    single-slot `Mutex<Option<(ModelCacheKey, CachedModel)>>`, `scope`/`level` applied via
    `.filtered()` post-hit.
  - Pattern Decisions table (`plan.md:83`): same key shape, explicitly states *why* `scope`
    was rejected from the key (fragments one repo's cache into per-scope rebuilds).
  - Story 1.4.1 / Task 1.4.1a (`plan.md:417-432`): `ModelCacheKey { repo_root, include_private }`,
    single-slot, no `scope` field, explicit "no `scope` field" callout.
  - Story 3.1.1 (`plan.md:560`) and Task 3.1.1d (`plan.md:573`): MCP tool keyed the same way,
    single-slot field on `KibitzerServer`.
  - Story 4.3.0 / Task 4.3.0b (`plan.md:724`, `plan.md:741`): LSP background build routes
    through the identical `ModelCacheKey { repo_root, include_private: false }` via the same
    `ModelCache::get_or_build`.
  The "build once, filter many via `.filtered(scope)`" pattern is concretely specified, not
  asserted: `ArchModel::filtered()` has its own AC/tests (Story 1.3.2, `plan.md:380-393`),
  and Task 2.1.1b shows the CLI path doing exactly this (build unscoped, then
  `.filtered(scope, level)`, `plan.md:480`) — the pattern the cache now mirrors.

### Also spot-checked, holds
- **`build_model` purity**: now genuinely pure. Signature changed to
  `files: &[(PathBuf, String)]` (Domain Glossary `plan.md:67`, Story 1.3.1 AC `plan.md:336`);
  Task 1.3.1c explicitly notes "no `std::fs::read_to_string` here, `source` was already read
  by the caller" (`plan.md:364`). All three callers now do the read themselves before calling
  in: CLI (Task 2.1.1b, `plan.md:480`), MCP (Task 3.1.1b, `plan.md:567`), LSP background index
  (Task 4.3.0b, `plan.md:741`). This matches `extract_symbols_for_file`'s already-pure shape
  and closes the gap the original review found between the "pure orchestration function"
  claim and the old `&[PathBuf]`-reads-from-disk-internally signature.
- **Pattern Decisions table (Strategy dispatch, build-vs-buy)**: unchanged by the repair and
  still holds — `LangSymbolConfig` table-driven dispatch mirroring `rules.rs::lang_config`
  (`plan.md:79`), and the build-vs-buy row (native `GrammarCache`/table dispatch, bespoke
  JSON tree, Mermaid diagram, no new dependency) is intact (`plan.md:81-82`). No drift
  introduced by the repair pass in these rows.

## Concerns
Carried forward from the prior review — the repair pass didn't touch these areas, and they
remain genuinely unresolved:

- [ ] **Epic 1.4 (`ModelCache::get_or_build`)** — the signature still requires the caller to
  pass a fresh `files: &[PathBuf]` list (`plan.md:417`), meaning a full repo directory walk
  still runs on every `list_architecture_symbols`/`get_architecture_node`/`workspace/symbol`
  call regardless of cache hit/miss — only the parse+extract cost is avoided on a hit. For a
  large repo this walk itself may be a non-trivial fraction of the "recomputed every call"
  cost ADR-002 exists to eliminate. **Recommendation**: either have `ModelCache` own/cache the
  file list itself (re-walking only on an explicit invalidation signal), or explicitly
  document the walk-per-call as an accepted, benchmarked trade-off.
- [ ] **Story 1.3.2 (`ArchModel::filtered`) — `PruningSummary` staleness** — `filtered()`'s AC
  still only describes clearing `PackageNode.symbols` at `Component` level (`plan.md:390-392`);
  `PruningSummary` (populated once by `build_model`) is not updated to reflect this additional
  exclusion, so a consumer reading `pruning.private_symbols_skipped` on a `Component`-level
  filtered response still gets a count that doesn't account for the level-based clearing.
  **Recommendation**: either have `filtered()` return an updated `PruningSummary`, or add an
  explicit AC documenting that `PruningSummary` only covers `build_model`-time pruning, not
  `filtered()`-time view narrowing.
- [ ] **Story 3.1.1 (`ListArchitectureSymbolsRequest`) — stringly-typed wire boundary** —
  `level: String (default "code")` and `kind: Option<String>` (`plan.md:551`) are still raw
  strings with no specified parse step into `ModelLevel`/`SymbolKind`, and still no AC covers
  an invalid value (e.g. `"bogus"`). This directly contradicts the Pattern Decisions table's
  own stated rationale for making these sum types in the first place (`plan.md:89`).
  **Recommendation**: add a parse-at-boundary step and an AC for the invalid-value case
  (empty/`total_matched: 0` result, or an explicit MCP error — either is fine, but pick one).
- [ ] **Story 1.3.2 (`ArchModel::filtered`) — `import_edges` scoping under `scope`** — still
  unspecified whether a `scope`-narrowed `.filtered()` call retains import edges that cross
  the scope boundary (`from`/`to` referencing a package outside the filtered `packages` set)
  or drops them. The AC only pins down the `level == Component` case (`plan.md:390-392`);
  the `scope` example (`plan.md:391`) doesn't address `import_edges` at all.
  **Recommendation**: add an explicit AC stating whether `import_edges` is filtered to
  fully-in-scope edges only or retains cross-boundary edges, and that the choice is
  deliberate.

## Nitpicks
- Epic 1.4's file-placement note still defers the `arch_model.rs` vs. new `arch_cache.rs`
  module-boundary decision to line count at implementation time rather than responsibility
  (`plan.md:430`) — `arch_model.rs` already carries four distinct responsibilities (domain
  types, build orchestration, query API, cache) per the Summary-of-files table. Not fixed by
  the repair; still worth deciding by SRP up front.
- Story 3.1.1's cursor-based pagination (`plan.md:555-556`) still doesn't specify behavior if
  the underlying `ModelCache` entry is invalidated/rebuilt between two paginated calls in the
  same session (an outstanding offset-based cursor from a stale build could skip or duplicate
  results on the next page). Low-impact for a local dev tool session; still just a documented
  gap, not fixed.
- (Resolved by the repair, no longer applicable: Task 1.3.1b's leftover self-editing note
  from the prior review — `plan.md:359-361` now reads as clean, finished spec text.)

## Verified compliant (no drift found)
- Build-vs-buy alignment and per-language Strategy-dispatch pattern: unchanged by the repair,
  still match `research/build-vs-buy.md` and `rules.rs::lang_config`'s precedent respectively
  (see spot-check above).
- ADR-001 isolation, requirements.md Open Questions resolution, aggregate design
  (`ArchModel` sole tree root, no partial-mutation API), and Open/Closed compliance for Phase
  5 language additions — all outside the repair's touched areas, re-skimmed, no regressions
  found.
