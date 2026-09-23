# Requirements: type-hierarchy-graph

**Date**: 2026-09-22
**Type**: feature addition
**Complexity**: 2-3 — new model field + per-language extraction + two MCP query tools
**Source**: backlog item `30cde5cf-884e-42bd-a89d-d1b5482b8bef` / GitHub issue #40

## Problem Statement
`SymbolNode` (`src/arch_model.rs`) records `parent` for methods-of-a-type, but `ArchModel`
has no edge type for supertype/subtype or interface-implementation relationships *between*
types. That is the missing input behind essentially the entire "Dealing with Inheritance"
chapter of Fowler's *Refactoring* catalog — Pull Up/Push Down Method/Field, Extract
Superclass, Collapse Hierarchy, Remove Subclass, Replace Type Code with Subclasses — and
Replace Conditional with Polymorphism. Every one of those refactorings is defined in terms
of a superclass/subclass or interface/implementer relationship kibitzer currently has no
way to query. It is a required prerequisite for three already-filed downstream issues —
#59 (Pull Up/Push Down family), #60 (type-code/switch clustering), and #61 (hierarchy
cleanup) — none of which can be built without it.
*(Correction, Phase 4 triad review: the backlog item's original text also called this "the
single highest-leverage capability gap in the catalog" / "unblocks more catalog entries
than any other." Checked against `docs/refactoring-catalog-analysis.md`'s own dependency
graph: that superlative doesn't hold — #33 transitively unblocks 7 catalog entries and #39
unblocks 5, versus this item's 3 (tied with #38). The narrower claim above — required
prerequisite for #59/#60/#61 — is independently verified and sufficient justification on
its own; the unsupported superlative is removed rather than carried forward.)*

## Baseline
Today: `ArchModel` (`src/arch_model.rs`) has `packages`, `import_edges`, `call_edges`, and
`field_accesses`, but no type-to-type relationship edge. `SymbolNode.parent` only relates a
`Method` to its owning `Type`/`Interface` name (string, not an id) — it says nothing about
one `Type`/`Interface` extending or implementing another. `symbol_extract.rs` already parses
per-language type/interface declarations for Go, TypeScript, Tsx, JavaScript, Python, Java,
Kotlin, and Rust (`lang_symbol_config` in `src/symbol_extract.rs`), but none of those
per-language configs capture `extends`/`implements` clauses or (for Go) struct-embedding
fields. The MCP layer (`src/mcp.rs`) exposes `list_architecture_symbols` (paginated,
filtered symbol slice) and `get_architecture_node` (single node by id) but has no
supertype/subtype query tool.

## Users / Consumers
- AI agents (Claude Code sessions via MCP tools) that want to identify hierarchy-shaped
  refactoring opportunities (Pull Up Method, Extract Superclass, etc.) — the direct
  consumer of `list_supertypes`/`list_subtypes`.
- Future kibitzer checkers #59/#60/#61, which read `ArchModel.type_edges` as a library
  dependency, not via MCP.
- CLI/diagram consumers of the shared architecture model (`kibitzer architecture
  export`/`diagram`), which should render hierarchy edges once present, consistent with the
  "one model, many views" pattern from the `architecture-export` project.

## Success Metrics
- `ArchModel` exposes `type_edges: Vec<TypeRelationEdge>` populated for every `Type`/
  `Interface` `SymbolNode` with an `extends`/`implements` relationship the source declares.
- `list_supertypes(node)` / `list_subtypes(node)` MCP tools return correct, paginated
  results for a real kibitzer-repo query (dogfood: e.g. resolve this repo's own type
  hierarchy, if any exists — otherwise a constructed fixture, since kibitzer's own Rust
  code doesn't have Go/TS/Java-style single-inheritance hierarchies).
- Go struct-embedding (`extends`) is captured correctly, including multi-level embedding
  chains.
- TS/Java/Kotlin `extends`/`implements` keyword edges are captured correctly, including
  multiple `implements` targets.
- v1 explicitly does NOT attempt Go structural interface satisfaction (method-set
  comparison) — scoped out per the backlog item, tracked as a fast-follow.
- #59/#60/#61 become buildable: each can express its detection rule in terms of
  `type_edges` without inventing its own hierarchy-parsing logic.

## Appetite
Small–Medium (3-8 days). This extends an existing, well-factored extraction pipeline
(`symbol_extract.rs`'s per-`Language` `LangSymbolConfig` pattern, `ArchModel`'s existing
edge-vec + MCP-pagination conventions) rather than building new infrastructure — closer in
shape to `field_accesses`/`FieldAccessEdge` (a recent, similarly-scoped edge addition) than
to the original `architecture-export` project that built the model from scratch.

## Constraints
No hard deadline — solo-maintained open-source project. No compliance/regulatory
constraints. Must not break existing `ArchModel` JSON serialization consumers (CLI export,
LSP, diagram) — `type_edges` is additive.

## Non-functional Requirements
- **Performance**: extraction must not meaningfully slow `build_model` — same tree-sitter
  walk pass extracting `SymbolNode`s should also extract type-relation edges where
  possible, not a second full-repo pass. `field_accesses`'s existing resolve-after-walk
  pattern (`resolve_call_edges`-style) is the precedent to follow.
- **Correctness over completeness for v1**: an edge that can't be resolved to a known
  `SymbolNode` in this model (e.g. extending an external/vendored/stdlib type) should be
  dropped or kept as unresolved text, matching `CallEdge`'s existing `resolved: bool`
  convention — never silently guessed.
- **Pagination**: `list_supertypes`/`list_subtypes` must follow the same cursor/limit/
  `next_cursor` convention as `list_architecture_symbols` (`src/mcp.rs`), not invent a new
  one.

## Scope
### In Scope
- `TypeRelationEdge` struct + `ArchModel.type_edges: Vec<TypeRelationEdge>`.
- Go: struct-embedding → `extends` edges (explicit in source, the easy/required v1 case).
- TypeScript/Tsx/JavaScript, Java, Kotlin: `extends`/`implements` keyword clauses →
  `extends`/`implements` edges (direct AST read per language).
- `list_supertypes(node)` / `list_subtypes(node)` MCP query tools, paginated per
  `list_architecture_symbols`'s existing convention.
- Unit/fixture tests per language proving edge extraction, mirroring
  `symbol_extract.rs`'s existing per-language test pattern.
- Adding a Kotlin repo (`android/nowinandroid`) to `docs/backtest-repos.md`'s corpus and a
  manual spot-check of extracted Kotlin `type_edges` against it — pulled in-scope during
  Phase 3 planning (`implementation/plan.md` Story 3.2.1), resolving the Open Questions
  entry below in favor of "in this item's scope," not a followup, given Kotlin's
  three-way `delegation_specifier` split is this feature's most novel classification logic
  and this repo's "not done without real-world corpus" convention applies to it directly.

### Out of Scope (this item)
- Go structural interface satisfaction (method-set comparison against every interface in
  the model) — explicitly deferred to a fast-follow per the backlog item's scope notes.
  This is real, separate work: it requires comparing every `Type`'s method set (from
  existing `SymbolNode`s with `parent` set) against every `Interface`'s method set,
  repo-wide, not a per-file AST read.
- Python (no static `extends` keyword semantics comparable to Java/TS — Python does have
  `class Foo(Bar):` but MRO/duck-typing make "supertype" a fuzzier concept; not requested
  by this item, and `symbol_extract.rs`'s Python config doesn't currently distinguish
  `Type`-vs-`Interface`).
- Rust (no interface/inheritance keyword in the same sense — traits + impls are a
  different relationship shape; not mentioned in this item's scope, would need its own
  design).
- The #59/#60/#61 checkers themselves — this item only produces the query surface they
  depend on.
- CLI export/diagram rendering of `type_edges` — "Unlocks" section implies future
  consumers, but this item's own success metrics are model + MCP tools only; diagram
  rendering can follow the same "one model, many views" pattern later without blocking
  this item.

## Rabbit Holes
- Go struct embedding can be multi-level and can embed both a struct (extends) and
  interfaces (implements) in the same type — the AST distinction between "embedded
  field is a named struct type" vs. "embedded field is a named interface type" needs a
  symbol-table lookup (is this name a `Type` or `Interface` in the model?), not just an
  AST-shape read, since Go's syntax for embedding either kind is identical.
- Resolving an `extends`/`implements` target name to a `SymbolNode::id` requires the same
  cross-package name resolution problem `resolve_call_edges` already solved for
  `CallEdge` — reuse that machinery/pattern rather than re-deriving it, per
  `refactoring-catalog-analysis.md`'s "#33's walk-and-resolve machinery" note.
- Kotlin: a single class can have one `extends` (superclass) and multiple `implements`
  (interfaces) in the same `:` supertype list — the AST doesn't separate them syntactically
  the way TS/Java do; needs a symbol-table check (is the supertype name a `Type` or
  `Interface`?) similar to the Go case above.

## Alternatives Considered
- Building Go structural interface satisfaction now instead of deferring it — rejected;
  the backlog item explicitly scopes it out as "meaningfully harder," and shipping
  `extends` for the explicit-syntax languages first unblocks #59/#60/#61's easier cases
  sooner.
- A single `SupertypeEdge` type without separate `extends`/`implements` semantics —
  rejected; #60 (type-switch clustering) and #61 (hierarchy cleanup) need to distinguish
  "is-a via inheritance" from "satisfies an interface" for correct refactoring suggestions
  (e.g. Collapse Hierarchy only makes sense for `extends`, not `implements`).

## Feasibility Risks
- Per-language AST shape differences for "supertype list" are real, incremental work (4
  languages: Go, TS/Tsx/JS, Java, Kotlin) but each follows the existing
  `lang_symbol_config`/`LangSymbolConfig` extension pattern already proven by `Rust`'s
  addition to that same enum — low novel-design risk, moderate mechanical-implementation
  volume.
- Cross-package name resolution reuse: if `resolve_call_edges`'s existing resolution logic
  isn't cleanly reusable/generic enough for type names, this could grow into a small
  refactor of that shared machinery rather than a pure addition — Phase 2 research should
  confirm reusability before Phase 3 commits to an approach.
- **Demand risk, not just execution risk** *(added Phase 4 triad review, matching the
  framing `project_plans/architecture-export/requirements.md` already models)*: this
  item's value is conditional on #59/#60/#61 actually getting built once unblocked — a
  solo-maintainer, open-source roadmap where "unblocked" doesn't guarantee "prioritized
  next." If none of #59/#60/#61 land within a reasonable window, `type_edges` is a
  correct-but-unconsumed capability (the MCP tools' own value is real but secondary — see
  Success Metrics' dogfood note that no self-hosted hierarchy exists in this Rust repo to
  exercise it against day-to-day). This doesn't change the recommendation (the item is
  cheap relative to the option value it creates, and Story 3.2.1/3.2.2's corpus
  spot-checks are validation work that pays off even if the checkers are delayed) but names
  the assumption explicitly rather than leaving demand implicit.

## Observability Requirements
Standard request/command logging sufficient — local dev tool, not a hosted service.

## Risk Control
Low risk — purely additive (`ArchModel.type_edges`, two new MCP tools). Doesn't change any
existing check, edge type, or CLI/MCP surface. No feature flag needed; normal git revert is
sufficient rollback.

## Open Questions
- ~~Should `TypeRelationEdge` carry a `resolved: bool`?~~ **Resolved by Phase 2 research**
  (stack.md, features.md, pitfalls.md all independently concur): yes, follow `CallEdge`'s
  keep-unresolved-with-raw-text convention, not `FieldAccessEdge`'s silent-drop. An
  unresolved external/stdlib supertype (`sync.Mutex`, `java.io.Serializable`) is a common,
  legitimate case — dropping it would make a type extending an external base look like a
  hierarchy root with no superclass at all.
- ~~Exact `TypeRelationEdge` field shape?~~ **Resolved by Phase 2 research, refined in
  Phase 3 planning**: `from`/`to` as `SymbolNode::id` strings, `resolved: bool`, `file`/`line`
  for provenance — parallel to `CallEdge`. `kind` is `Option<TypeRelationKind>`
  (`Extends | Implements`), not a required enum as Phase 2 research first assumed:
  `implementation/plan.md`'s Pattern Decisions table (type-driven-design lens) found a real
  minority state Phase 2 didn't account for — an unresolved Go/Kotlin ambiguous site (e.g.
  an external base class like `sync.Mutex`) can't be classified `Extends` vs. `Implements`
  without knowing the target's `SymbolKind`, which resolution just failed to find; forcing a
  value here would be exactly the "guess" `resolve_in`'s existing doc comment already
  disclaims. See `research/architecture.md` and `research/stack.md` for the
  extraction-pipeline placement (raw-site-then-resolve pass, not inline with `SymbolNode`
  extraction).
- **Still open, flagged by research as a genuine disagreement for Phase 3 to resolve
  explicitly**: transitive vs. direct-only traversal for `list_supertypes`/`list_subtypes`.
  `research/features.md` found a concrete need — Extract Superclass (#61) requires walking
  the *full* ancestor chain, not just direct parents — and recommends copying
  `list_callers`/`list_callees`'s `depth`/BFS/`truncated` shape. `research/architecture.md`
  counters that `traverse_call_edges`'s BFS machinery is hard-typed to `CallEdge`/
  `CallDirection` and not cleanly reusable, and recommends a simpler direct-filter pair
  instead, leaving transitive walking to be composed by the caller (or added later) rather
  than built into v1. Both agree `ArchModel.type_edges` itself stores only direct edges
  either way — this is purely about the MCP query tools' traversal depth. Phase 3 must pick
  one and state why, rather than let this surface mid-implementation.
- Kotlin repo added to the real-world backtest corpus: `research/pitfalls.md` notes
  `docs/backtest-repos.md` has no Kotlin repo today, but this feature requires Kotlin
  extends/implements extraction, which the repo's own "not done without real-world corpus"
  convention (`docs/backtesting.md`) says needs one before this checker-adjacent extraction
  logic can be considered backtested. Phase 3 should decide whether adding one is in this
  item's scope or a followup.
