# Implementation Plan: type-hierarchy-graph

**Feature**: `TypeRelationEdge`/`ArchModel.type_edges` (Go struct-embedding, TS/Tsx/JS/Java/
Kotlin `extends`/`implements` extraction) plus `list_supertypes`/`list_subtypes` MCP query
tools, unblocking checkers #59/#60/#61.
**Date**: 2026-09-22
**Status**: Ready for implementation
**ADRs**: [ADR-001](../decisions/ADR-001-list-supertypes-subtypes-depth-parameter.md) — `list_supertypes`/`list_subtypes` take a `depth` parameter (BFS), not a direct-only filter.

---

## System type

Extends an existing, well-factored two-phase extraction pipeline (`symbol_extract.rs`'s
per-`Language` table-driven walker → `arch_model.rs`'s raw-site-then-resolve pass) with one
new edge type and its resolver, plus two new MCP query tools following an existing
request/response convention. No new subsystem, no new persistence, no new external
dependency — a same-shape sibling to `CallEdge`/`FieldAccessEdge` and
`list_callers`/`list_callees`/`list_architecture_symbols`.

---

## Domain Glossary

| Term | Definition | Notes |
|------|-----------|-------|
| `TypeRelationEdge` | One resolved (or unresolved-but-visible) type-hierarchy edge: a declaring `Type`/`Interface` (`from`) and the supertype/interface it extends or implements (`to`). | New struct, `src/arch_model.rs`. Parallel to `CallEdge`. |
| `TypeRelationKind` | Whether an edge is `Extends` (is-a, inheritance) or `Implements` (satisfies-an-interface). `Option<TypeRelationKind>` on the edge — `None` only for the rare case where source syntax doesn't disambiguate *and* the target couldn't be resolved to learn its `SymbolKind` either (see Story 1.5.1). | New enum, `src/arch_model.rs`. `#[serde(rename_all = "lowercase")]`, matching `AccessKind`. |
| `RawTypeRelationSite` | An unresolved type-relation site collected during the per-file walk: the declaring type's own `SymbolNode::id`, the raw (already generic-stripped) supertype name text, an optional `kind_hint` when the source syntax already disambiguates it, and `file`/`line`. | New struct, `src/symbol_extract.rs`. Parallel to `RawCallSite`. |
| `kind_hint` | `RawTypeRelationSite`'s `Option<TypeRelationKind>` set at extraction time. `Some` for TS/Tsx/JS/Java (keyword-level `extends`/`implements` is unambiguous) and for Kotlin's `constructor_invocation`/`explicit_delegation` shapes. `None` for Go embedded fields and Kotlin's bare-`type` supertype-list entries, where the AST alone can't tell a struct/superclass from an interface. | See Story 1.5.1 for why Kotlin is split across both cases, not uniformly `None` as `research/architecture.md`/`research/features.md` assumed. |
| `type_relation_supports(language)` | Per-language gate (mirrors `call_graph_supports`) — `true` for Go, TypeScript, Tsx, JavaScript, Java, Kotlin; `false` for Python, Rust (out of scope). | New fn, `src/symbol_extract.rs`. |
| `extract_type_relation_sites_for_file` | Public entry point: walks an already-parsed tree, returns every `RawTypeRelationSite` for `source`. No file I/O, no cross-file resolution — same convention as `extract_call_sites_for_file`. | New fn, `src/symbol_extract.rs`. |
| `collect_type_relation_sites` | `build_model`'s per-file wrapper that fills in `file` on each raw site, mirroring `collect_call_sites`. | New fn, `src/arch_model.rs`. |
| `TypeRelationIndex` | Whole-repo `SymbolNode.name -> [(package, id, SymbolKind)]` index over `Type`/`Interface` symbols only, built once after every package is known. Carries `SymbolKind` (unlike `SymbolIndex`, the call graph's name index) because Go/Kotlin's ambiguous sites need the target's kind to classify `Extends` vs. `Implements`. | New type + builder fn `build_type_relation_index`, `src/arch_model.rs`. |
| `TypeRelationPackageIndex` | Secondary `(package_path, name) -> (id, SymbolKind)` index used for a Go-qualified embed (`pkg.Base`) once `file_import_aliases` has resolved `pkg` to a real package path — avoids re-running the same-package/globally-unique ambiguity logic on an already-disambiguated reference. | New type, `src/arch_model.rs`. |
| `resolve_type_edges` | Post-walk resolution pass: builds `TypeRelationIndex`/`TypeRelationPackageIndex` from `packages`, resolves every `RawTypeRelationSite`, returns `Vec<TypeRelationEdge>`. Called from `build_model` alongside `resolve_call_edges`/`resolve_field_access_edges`. | New fn, `src/arch_model.rs`. |
| `ArchModel.type_edges` | New field: `Vec<TypeRelationEdge>`, additive to `ArchModel`'s existing JSON shape. | `src/arch_model.rs`. |
| `TypeHierarchyDirection` | MCP-layer enum: `Supertypes` (walk `from -> to`, "what does `node` extend/implement") or `Subtypes` (walk `to -> from`, "what extends/implements `node`"). | New enum, `src/mcp.rs`. Named after the tool pair directly, not mirroring `CallDirection`'s `Callers`/`Callees` naming. |
| `traverse_type_edges` / `build_type_adjacency` / `expand_type_frontier` | `TypeRelationEdge`-typed BFS trio, structurally parallel to `traverse_call_edges`/`build_call_adjacency`/`expand_call_frontier` — not a shared generic (see ADR-001). | New fns, `src/mcp.rs`. |
| `MAX_TYPE_HIERARCHY_DEPTH` | Clamp bound for `TypeHierarchyRequest::depth`, value `10` — a separate named constant from `MAX_CALL_DEPTH`, even though the value matches, so the two tool families' clamp bounds can be tuned independently later. | New const, `src/mcp.rs`. |
| `TypeHierarchyRequest` / `TypeHierarchyResponse` | Request/response pair for `list_supertypes`/`list_subtypes`: `{path, node, depth, include_private, limit, cursor}` in, `{node, depth, truncated, total_matched, returned, next_cursor, possibly_pruned, node_kind, hint, edges}` out — combines `CallTraversalRequest/Response`'s depth/BFS shape with `ListArchitectureSymbolsRequest/Response`'s pagination shape (see ADR-001). | New structs, `src/mcp.rs`. |
| `list_supertypes` / `list_subtypes` | The two MCP tools, thin wrappers sharing one `type_hierarchy` body — mirrors `list_callers`/`list_callees` → `call_traversal`'s split. | New tool methods, `src/mcp.rs`. |
| `node_kind` / `hint` (response fields) | Set only when `node` resolves to a `SymbolNode` that isn't a `Type`/`Interface` (e.g. a `Function`/`Method` id was passed) — `edges` stays `[]` (not an error), `node_kind` names the actual kind (typed `Option<SymbolKind>`, not a hand-written string — reuses the existing enum's `#[serde(rename_all = "lowercase")]` so the JSON shape is unchanged), `hint` tells the caller to check `get_architecture_node` first. Mirrors `get_architecture_node`'s `exists_but_pruned`/`hint` precedent. | New response fields, `src/mcp.rs`. |

---

## Pattern Decisions

| Component | Pattern Chosen | Source | Alternative Rejected | Reason |
|-----------|---------------|--------|---------------------|--------|
| Overall extraction architecture | Raw-site-then-resolve, table-driven per-language config (extends `LangSymbolConfig`/`symbol_extract.rs`) | Existing `CallEdge`/`FieldAccessEdge` precedent (`arch_model.rs`) | (a) Inline single-pass resolution during the per-file walk; (b) a declarative tree-sitter `Query`/`QueryCursor` sublanguage instead of manual `Node` walking | (a) can't resolve Go/Kotlin's ambiguous embeds — the target's `SymbolKind` may live in a file not yet visited (`research/architecture.md` §2). (b) is zero-new-dependency but introduces a second tree-sitter-consumption idiom into a codebase that has never used it, for shapes simple enough not to need it (`research/build-vs-buy.md` §1). |
| `list_supertypes`/`list_subtypes` traversal | `depth`-parameterized BFS with visited-set + `truncated`, mirroring `list_callers`/`list_callees` | `research/features.md` (see ADR-001) | Direct-edges-only filter, no `depth` | `research/architecture.md`'s premise (existing tools are one-hop-only) is factually wrong; a catalog-documented consumer (#61 Extract Superclass) needs transitive ancestor walking. See ADR-001 for full reasoning. |
| Node-kind matching in new extraction code | Raw `&'static str` kind literals (`node.kind() == "class_heritage"`), matching every existing `LangSymbolConfig`/`classify_node` match arm in this file | Existing `symbol_extract.rs` convention | Typed `<Lang>Kind` enums (`GoKind`, `TypeScriptKind`, ...) from `src/node_kind.rs`, per `research/build-vs-buy.md` §3's "concrete mitigation" | `node_kind.rs`'s generated enums exist but are adopted by zero call sites in `symbol_extract.rs` today (confirmed: `grep -rn "GoKind\|TypeScriptKind\|node_kind::" src/symbol_extract.rs` → no hits). Introducing them for only the ~20 new match arms in this feature, while the file's other 40+ existing match arms stay raw strings, creates two idioms in one file for a typo-risk this file's own test suite (`lang_symbol_config_go_type_kinds_and_export_detection` etc.) already catches today. A full-file migration to typed enums is real, valuable, but out of scope for this feature (not named in `requirements.md`'s Scope section) — worth a separate follow-up item, not bundled here. |
| Go embedded-field kind resolution | Resolve-time `SymbolKind` lookup (like Go) — deferred to `resolve_type_edges` | `research/architecture.md` §2, `research/pitfalls.md` §2 | Assume embed is always `extends` (struct) | Go's grammar produces byte-identical shape for embedding a struct or an interface (`research/stack.md`) — guessing would silently misclassify `implements`-shaped embeds (e.g. embedding `io.Reader`), which `resolve_in`'s own doc comment already establishes as against this codebase's resolution philosophy ("intentionally never guesses"). |
| Kotlin supertype-list kind resolution | **Split**: `constructor_invocation` → `Some(Extends)` at extraction time (unambiguous — only a class can appear with call syntax in a supertype position); `explicit_delegation` (`by`) → `Some(Implements)` at extraction time (unambiguous — delegation is interface-only); bare `type` (no wrapper) → `None`, resolved like Go's ambiguous case | Verified directly against `tree-sitter-kotlin-ng` 1.1.0's `node-types.json` this session (see Story 1.5.1) | Treat the entire `delegation_specifier` list as uniformly ambiguous (`research/architecture.md`/`research/features.md`), or treat bare `type` as always `implements` (`research/stack.md`'s original claim) | `research/stack.md`'s "bare type = interface" claim is wrong for a real, idiomatic, documented Kotlin case: a derived class with **no primary constructor** lists its superclass as a bare `type` too (each secondary constructor delegates via `: super(...)`) — e.g. `class MyView : View { constructor(ctx: Context) : super(ctx) }` where `View` is a class, not an interface, written bare. This is common in Android code (the exact corpus Story 3.2.1 adds). `research/architecture.md`/`research/features.md` were right that ambiguity exists, but treating *all three* `delegation_specifier` shapes as ambiguous throws away two shapes (`constructor_invocation`, `explicit_delegation`) the grammar genuinely does disambiguate unambiguously. |
| `TypeRelationEdge.kind`'s type | `Option<TypeRelationKind>` | Type-driven design: don't force a binary choice the extractor sometimes can't make | A required (non-`Option`) `TypeRelationKind` with a default/guessed value | An unresolved Go/Kotlin ambiguous site (external base, e.g. `sync.Mutex`) can't be classified `Extends` vs. `Implements` without knowing the target's `SymbolKind`, which resolution just failed to find. Forcing a value would be exactly the "guess" `resolve_in`'s doc comment already disclaims. `None` here is a real, minority state (only Go/Kotlin ambiguous *and* unresolved), not a scope-creep abstraction. |
| `TypeRelationEdge` resolved/unresolved convention | `CallEdge`'s convention: keep with `resolved: bool`, raw text in `to` when unresolved | `research/stack.md`, `research/architecture.md`, `research/pitfalls.md` (all three independently concur — also already settled in `requirements.md`'s Open Questions) | `FieldAccessEdge`'s convention: silently drop | An unresolved supertype (external/vendored/stdlib base) is a common, legitimate case, not a likely typo — dropping it would make a type extending an external base look hierarchy-root-free to #59/#60/#61. |
| Go qualified-embed (`pkg.Base`) resolution | Route through `ArchModel.file_import_aliases` first, falling back to name-only `resolve_in` only if no alias entry exists | `research/architecture.md` §5, `research/pitfalls.md` §1 | Treat `pkg.Base` the same as an unqualified name (last-segment-only `resolve_in`, `CallEdge`'s existing ceiling) | `file_import_aliases` already exists specifically to resolve a package-qualified local past its alias (built for `god_class`/`isp_fat_interface`) — reusing it for a qualified embed is strictly more precise than the call graph's existing, accepted ceiling, and the machinery is already there. |
| `list_supertypes`/`list_subtypes` response shape | Combine `CallTraversalResponse`'s `{node, depth, truncated, edges}` with `ListArchitectureSymbolsResponse`'s `{total_matched, returned, next_cursor, possibly_pruned}`, plus new `node_kind`/`hint` | ADR-001, `research/ux.md` §2-3 | Either precedent alone | `requirements.md`'s Non-functional Requirements section hard-requires the pagination convention; ADR-001 separately requires `depth`. Neither research doc proposed combining both — this plan does, since both constraints are independently binding. |
| `TypeHierarchyRequest.include_private` (no `CallTraversalRequest` equivalent) | Add the field, threaded into `load_model_off_stack(repo_root, req.include_private)` | `ListRefactorCandidatesRequest`'s doc comment (same rationale) | Match `call_traversal`'s precedent exactly (always `load_model_off_stack(repo_root, false)`, no field) | An unexported `Type`/`Interface` extending/implementing another is a normal, common case for a hierarchy query (unlike a call graph query, where `call_traversal`'s existing default already suits its callers) — excluding unexported types by default would silently under-report supertypes/subtypes for exactly the kind of internal-refactor question (#59/#60/#61) this tool exists to answer. |
| `TypeRelationEdge` field placement (`kind` inlined, target `name` not inlined) | Inline `kind` only; a target's human name/file/line require a follow-up `get_architecture_node` call | `research/ux.md` §3 | Inline target `name`+`kind`+`file`+`line` on the edge | `kind` answers the one thing a consumer can't already get elsewhere (is-a vs. satisfies-interface) and is free (already computed during resolution). `file`/`line` on the edge are the *declaration site's* provenance, already redundant with a target lookup; inlining more diverges `TypeRelationEdge` from `CallEdge`'s shape for marginal benefit. |
| Extraction dispatch structure | New sibling functions per language/declaration-kind divergence (`go_embedded_type_relations`, `ts_class_heritage_relations`, `js_class_heritage_relation`, `java_class_type_relations`, `java_interface_type_relations`, `kotlin_delegation_type_relations`), called from a shared `walk_type_relations` dispatcher | Existing `walk`/`walk_calls`/`walk_struct_fields` shape | One monolithic per-node-kind `match` spanning all 6 languages; a single combined function per language (e.g. one `ts_js_class_heritage_relations`, one `java_type_relations`) | Matches this file's own established shape (one specialized fn per language-specific AST divergence, dispatched by a thin recursive walker) — see Tech Debt Disposition below for why this matters given the file's existing size. TS and JS need separate functions (`class_heritage`'s internal shape diverges — wrapped `extends_clause`/`implements_clause` vs. a single bare `expression` child, per `research/stack.md`); Java needs separate class/interface functions (`superclass`/`interfaces` are named fields on `class_declaration` vs. a positional `extends_interfaces` child with no field name on `interface_declaration`) — six functions, not four, reflects the six actual AST-shape divergences this feature handles (Epics 1.2-1.5), corrected from an earlier draft's undercount. |

---

## Tech Debt Disposition

| Area | Existing Issue | Disposition | Justification |
|------|----------------|--------------|----------------|
| `src/arch_model.rs` (1969 lines, flagged by `rust-file-size`; `build_model` itself 120 lines, flagged by `long-function`) | Pre-existing, confirmed via `mcp__kibitzer__run_checks` per `research/architecture.md` §4; never resolved, `architecture-export`'s own review deferred the `arch_cache.rs` split decision to "later." | **Extend as-is**, as new sibling functions (`collect_type_relation_sites`, `resolve_type_edges`, index builders) called from a two/three-line addition to `build_model`'s existing loop and post-loop resolution block — not by growing `build_model`'s body itself. | This feature's ~150-250 line addition (per the `FieldAccessEdge` precedent's actual size) follows the exact shape the file's two existing edge types already establish. Adding it *inline* into `build_model` would make an already-over-threshold long-function worse; adding it as new functions does not meaningfully worsen the file's pre-existing, already-flagged size. The `arch_cache.rs` split remains a separate disposition call this feature doesn't need to resolve to land — re-litigating it here would be scope creep beyond `requirements.md`'s Scope section. |
| `src/symbol_extract.rs` (1691 lines, flagged; `walk`-family fns 93 lines, flagged) | Same category as above — pre-existing, confirmed. | **Extend as-is**, same reasoning: new per-language extraction fns dispatched from a new thin `walk_type_relations`, not folded into the existing `walk`/`classify_node`. | Identical shape to the call-site/field-access extraction additions that already landed in this file without triggering a split. |
| `src/mcp.rs` (2620 lines, flagged; already has 12 functions over the 40-line `long-function` threshold, confirmed via `kibitzer run src --trigger batch`) | Pre-existing, confirmed. A first draft of this plan proposed one `type_hierarchy` function combining `call_traversal`'s full responsibility set (resolve root, clamp depth, load model, BFS traverse — ~27 lines) with `list_architecture_symbols`'s full pagination/pruning responsibility set (cursor parsing, skip/take/next_cursor, `possibly_pruned` — ~93 lines); the architecture review caught that this combination would run 70-100+ lines, making `type_hierarchy` the file's 13th over-threshold function — not the "not a growth of an already-flagged function" the original estimate claimed. | **Extend as-is, but split** (Story 2.1.2, Tasks 2.1.2a-c): a shared `paginate<T>` helper (~15-25 lines) extracted from `list_architecture_symbols` and reused by both it and `type_hierarchy`; a `node_kind`/`hint` early-response helper (~15-25 lines); `type_hierarchy`'s own body built from both (~15-20 lines, once the two helpers are factored out); plus two ~10-line thin wrapper tool methods (`list_supertypes`/`list_subtypes`). `list_architecture_symbols` itself is edited to call the shared `paginate<T>` helper instead of its inline logic — a small, behavior-preserving refactor that shrinks it slightly, the opposite of growing an already-flagged function. No single new or edited function in this story exceeds the size of the two existing precedents (`call_traversal`, `list_architecture_symbols`) it's built from. |
| Node-kind string-literal matching (typo-silently-never-matches risk) | Not a filed issue — `research/build-vs-buy.md` §3/§4's own observation that `node_kind.rs`'s typed enums exist but are unused. | **Isolate via seam, deferred**: not adopted for this feature's new match arms (see Pattern Decisions row above), but every new node-kind string literal this feature introduces must have a same-fixture test asserting it actually matches (per Story ACs below) — the test suite is this feature's seam against the typo risk, since the typed-enum migration itself is out of scope. | A full-file typed-enum migration is a legitimate, separate follow-up (raise as a backlog item after this lands), not a prerequisite — `requirements.md`'s Scope section doesn't name it, and bundling it would roughly double this feature's footprint for a risk this plan's fixture-test discipline already covers. |

---

## Observability Plan
- **Logs**: None beyond kibitzer's existing standard command/request logging — local dev tool, no new log surface needed (per `requirements.md`'s Observability Requirements).
- **Metrics**: None. No hosted service, no metrics pipeline exists for any existing `ArchModel` extraction pass.
- **Alerts**: None applicable.

## Risk Control
- **Feature flag**: None. Purely additive (`ArchModel.type_edges`, two new MCP tools) — doesn't change any existing check, edge type, or CLI/MCP surface, per `requirements.md`'s Risk Control section.
- **Rollback procedure**: Standard `git revert` — no data migration, no schema to roll back.
- **Staged rollout**: None needed (local dev tool, not a hosted service).

## Unresolved Questions
None — both flagged disagreements (transitive-vs-direct traversal; Kotlin supertype-list
ambiguity) are resolved above (ADR-001 and the Pattern Decisions table, respectively). The
Kotlin backtest-corpus gap (`requirements.md`'s last Open Question) is resolved as an
in-scope task, not deferred — see Story 3.2.1.

## Dependency Visualization

```
Epic 1.1 (TypeRelationEdge type + type_edges field)
   |
   +--> Epic 1.2 (Go struct-embedding extraction) ----+
   +--> Epic 1.3 (TS/Tsx/JS extends/implements) -------+
   +--> Epic 1.4 (Java extends/implements) ------------+--> Story 1.6.1/1.6.2
   +--> Epic 1.5 (Kotlin supertype list) --------------+   (resolve_type_edges,
                                                             wired into build_model)
                                                                |
                                                                v
                                                    Epic 2.1 (list_supertypes/
                                                    list_subtypes MCP tools,
                                                    per ADR-001)
                                                                |
                                                                v
                                                    Epic 3.1 (perf fixture)
                                                    Epic 3.2 (Kotlin corpus +
                                                    spot-check)
```
Epics 1.2-1.5 (the four language extractors) are mutually independent — each only touches
its own new functions in `symbol_extract.rs` plus its own fixture tests, so they can be
implemented/reviewed in parallel once Epic 1.1's `RawTypeRelationSite`/`TypeRelationEdge`
types exist *and* Epic 1.1's Task 1.1.1c (updating every other `ArchModel { ... }`
struct-literal site in the crate so `cargo build --tests` succeeds repo-wide) has landed —
without it, every Epic 1.2-1.5 worker starts from a non-compiling crate. Epic 1.6
(resolution) depends on all four being present so its tests can exercise
cross-language-irrelevant but shared resolution logic. Epic 2.1 depends on
`ArchModel.type_edges` existing (Epic 1.6) but not on any specific language extractor
being "done" first (it can be built/tested against Go fixtures alone, then re-verified
once all languages land). Within Epic 2.1, Story 2.1.2's Task 2.1.2a (extracting the
shared `paginate<T>` helper out of `list_architecture_symbols`) is sequenced before Task
2.1.2c (`type_hierarchy`'s body) specifically so `type_hierarchy` is written against the
shared helper from the start rather than duplicating and later refactoring it. Epic
3.1/3.2 depend on the full pipeline being in place.

---

## Phase 1: Model, extraction, and resolution

### Epic 1.1: `TypeRelationEdge` domain type and `ArchModel.type_edges`
**Goal**: Establish the new edge/raw-site types and wire an (initially always-empty)
`type_edges` field through `ArchModel`'s existing serialization, so every subsequent story
has a stable type to extend.

#### Story 1.1.1: `TypeRelationEdge`/`TypeRelationKind` structs + `ArchModel.type_edges` field
**As a** checker author (#59/#60/#61), **I want** a `TypeRelationEdge` type on `ArchModel`
with the same resolved/unresolved convention as `CallEdge`, **so that** I can query
type-hierarchy relationships without inventing my own hierarchy-parsing logic.
**Acceptance Criteria**:
- `TypeRelationEdge { from: String, to: String, kind: Option<TypeRelationKind>, resolved: bool, file: PathBuf, line: usize }` is defined and derives `Debug, Clone, PartialEq, Serialize, Deserialize`.
  - *Given* the struct definition, *When* `serde_json::to_string` serializes a `TypeRelationEdge { from: "pkg::Dog".into(), to: "pkg::Animal".into(), kind: Some(TypeRelationKind::Extends), resolved: true, file: "pkg/dog.go".into(), line: 3 }`, *Then* the output JSON contains `"kind":"extends"` (lowercase, matching `AccessKind`'s `rename_all` convention).
- `ArchModel.type_edges: Vec<TypeRelationEdge>` is added as a new field, and `build_model` populates it (empty `Vec` for now — resolver lands in Story 1.6.2).
  - *Given* an existing `arch.json` fixture from before this change (no `type_edges` key), *When* it's deserialized into the new `ArchModel`, *Then* deserialization still succeeds (additive field, `requirements.md`'s Constraints section: "must not break existing `ArchModel` JSON serialization consumers").
- `cargo build --tests` succeeds repo-wide after this story lands (not just `cargo build`) — `ArchModel` does not derive `Default` (`src/arch_model.rs:184`), so every existing `ArchModel { ... }` struct-literal construction in the crate, including ones in other files' test modules, must be updated to set the new field, not just `build_model`'s.
  - *Given* the full set of `ArchModel { ... }` struct-literal sites in the crate (verified via `grep -rn "ArchModel {" src/` — 18 literal-construction sites total: `src/arch_model.rs` alone has 8, `build_model` plus `filtered` plus 6 test-helper literals in its own `#[cfg(test)]` module; the other 10 are spread across `src/arch_diagram.rs` (2), `src/architecture_checks.rs` (2), `src/isp_fat_interface.rs` (1), `src/god_class.rs` (3), `src/extract_class.rs` (1), `src/lsp.rs` (1)), *When* Tasks 1.1.1b and 1.1.1c land, *Then* every one of those 18 sites has a `type_edges: vec![],` line added (matching each site's existing `field_accesses: vec![],`-style initialization) and `cargo build --tests` succeeds with no missing-field errors.
- **Files**: `src/arch_model.rs`, `src/arch_diagram.rs`, `src/architecture_checks.rs`, `src/isp_fat_interface.rs`, `src/god_class.rs`, `src/extract_class.rs`, `src/lsp.rs`

##### Task 1.1.1a: Define `TypeRelationKind` and `TypeRelationEdge` (~4 min)
- In `src/arch_model.rs`, immediately after the existing `FieldAccessEdge` struct (after line 178), add:
  ```rust
  /// Whether a `TypeRelationEdge` is an inheritance ("is-a") or interface-satisfaction
  /// relationship. `None` on the edge itself (not this enum) covers the rare case where
  /// source syntax doesn't disambiguate and resolution couldn't either — see
  /// `TypeRelationEdge`'s doc comment.
  #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
  #[serde(rename_all = "lowercase")]
  pub enum TypeRelationKind {
      Extends,
      Implements,
  }

  /// One type-hierarchy edge: `from` is the declaring `Type`/`Interface`'s `SymbolNode::id`;
  /// `to` is a `SymbolNode::id` when `resolved`, else the raw supertype/interface text —
  /// never silently dropped, matching `CallEdge`'s convention (an unresolved target is
  /// routinely a legitimate external/vendored/stdlib base, not a typo).
  ///
  /// `kind` is `None` only when the source syntax alone can't distinguish `extends` from
  /// `implements` (Go embedded fields; Kotlin's bare-`type` supertype-list entries) *and*
  /// resolution couldn't determine it either (the target's `SymbolKind` would have settled
  /// it, but the target didn't resolve) — this is never guessed.
  ///
  /// `None` is common, not a rare corner case — every unresolved external-base Go embed
  /// (`sync.Mutex`, an un-indexed `io.Reader`) and every ambiguous Kotlin bare-`type` entry
  /// produces it. A consumer that cares about the extends/implements distinction MUST treat
  /// `None` as unclassified — never default it to `Extends`/`Implements` (e.g. never
  /// `kind.unwrap_or(Extends)` or a catch-all `_ => Extends` match arm). Doing so would
  /// silently reintroduce exactly the guessing `resolve_in`'s doc comment already forbids.
  ///
  /// Note: `(resolved: true, kind: None)` is type-representable but never produced by
  /// `resolve_one_type_edge`'s spec'd logic (Task 1.6.1c) — any successfully-resolved
  /// target always yields a `SymbolKind` to infer `kind` from, unless `kind_hint` already
  /// supplied one. A test asserting this combination doesn't occur for any Story
  /// 1.6.1/1.6.2 fixture is cheap insurance, not required for correctness.
  #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
  pub struct TypeRelationEdge {
      pub from: String,
      pub to: String,
      pub kind: Option<TypeRelationKind>,
      pub resolved: bool,
      pub file: PathBuf,
      pub line: usize,
  }
  ```
- Files: `src/arch_model.rs`

##### Task 1.1.1b: Add `type_edges` field to `ArchModel` and thread an empty `Vec` through `build_model` (~3 min)
- Add `pub type_edges: Vec<TypeRelationEdge>,` to the `ArchModel` struct (after `field_accesses` at line 190), with a doc-comment sentence noting it's populated by `resolve_type_edges` (added in Story 1.6.2).
- In `build_model`'s `Ok(ArchModel { ... })` construction (around line 423-440), add `type_edges: Vec::new(),` after `field_accesses`.
- Files: `src/arch_model.rs`

##### Task 1.1.1c: Add `type_edges: vec![],` to every other `ArchModel { ... }` struct-literal site in the crate (~10 min)
- `ArchModel` does not derive `Default` (confirmed: `src/arch_model.rs:184`), so Task 1.1.1b's new field breaks every other full struct-literal construction of `ArchModel`, not just `build_model`'s — this is a required companion task, not optional cleanup. Verified via `grep -rn "ArchModel {" src/` this session: beyond `build_model` (Task 1.1.1b), there are 17 more literal-construction sites (excluding struct/`impl`/function-signature matches, which don't need changes):
  - `src/arch_model.rs` (same file, 7 more sites in its own `#[cfg(test)] mod tests`): `filtered` (~line 676), `empty_model` test helper (~line 967), and four more inline `let model = ArchModel { ... }` test literals (~lines 984, 1711, 1752, 1786, 1820).
  - `src/arch_diagram.rs` (2 sites, ~lines 246, 332).
  - `src/architecture_checks.rs` (2 sites, ~lines 1642, 1846).
  - `src/isp_fat_interface.rs` (1 site, ~line 355).
  - `src/god_class.rs` (3 sites, ~lines 559, 578, 614).
  - `src/extract_class.rs` (1 site, ~line 375).
  - `src/lsp.rs` (1 site, ~line 714).
- In each, add `type_edges: vec![],` immediately after that site's existing `field_accesses: ...,` line — every site already follows this exact `vec![]`-per-empty-field style (confirmed by reading each; none use `..Default::default()` or a spread, since there's no `Default` impl to spread from).
- Run `cargo build --tests` after this task and confirm it succeeds with zero missing-field errors (see Story 1.1.1's new AC) — this is the task that closes that AC out, not a follow-up.
- Files: `src/arch_model.rs`, `src/arch_diagram.rs`, `src/architecture_checks.rs`, `src/isp_fat_interface.rs`, `src/god_class.rs`, `src/extract_class.rs`, `src/lsp.rs`

##### Task 1.1.1d: Fixture test — additive-field JSON round-trip (~3 min)
- In `arch_model.rs`'s existing `#[cfg(test)] mod tests` (starts line 853), add
  `type_edges_field_is_additive_and_empty_by_default`: builds a minimal `ArchModel` via
  the existing test-fixture helper used by nearby `CallEdge`/`FieldAccessEdge` tests,
  asserts `model.type_edges.is_empty()`, and round-trips it through
  `serde_json::to_string`/`from_str` to confirm `TypeRelationEdge`/`TypeRelationKind`
  serialize/deserialize without error.
- Files: `src/arch_model.rs`

---

### Epic 1.2: Go struct-embedding extraction (`extends`)
**Goal**: Extract `RawTypeRelationSite`s for Go struct embedding — the one v1 language
where the AST needs a real new walk (`go_struct_fields` explicitly discards this shape
today), per `research/pitfalls.md` §2.

#### Story 1.2.1: `RawTypeRelationSite` + Go embedded-field extraction
**As a** #59/#60/#61 checker, **I want** Go struct embedding recorded as a `RawTypeRelationSite`
for every embed shape (plain, qualified, pointer, generic), **so that** Go's `extends`
relationships are captured without guessing struct-vs-interface at extraction time.
**Acceptance Criteria**:
- `RawTypeRelationSite { type_id: String, package_path: String, target_text: String, kind_hint: Option<TypeRelationKind>, file: PathBuf, line: usize }` is defined.
  - *Given* the struct definition, *When* a Go extractor builds one for `type Dog struct { Animal }` in package `example.com/app/pkg`, *Then* it can construct `RawTypeRelationSite { type_id: "example.com/app/pkg::Dog", package_path: "example.com/app/pkg", target_text: "Animal", kind_hint: None, file: ..., line: ... }`.
- `type_relation_supports(Language) -> bool` returns `true` for Go/TypeScript/Tsx/JavaScript/Java/Kotlin, `false` for Python/Rust.
  - *Given* `Language::Rust`, *When* `type_relation_supports` is called, *Then* it returns `false`.
- A Go struct embedding a same-package type produces one `RawTypeRelationSite` with `kind_hint: None` and `target_text` equal to the embedded type's bare name.
  - *Given* Go source `package pkg\n\ntype Animal struct{}\n\ntype Dog struct {\n\tAnimal\n\tName string\n}\n`, *When* `extract_type_relation_sites_for_file(Language::Go, source, &tree, "pkg")` runs, *Then* it returns exactly one site: `type_id: "pkg::Dog"`, `target_text: "Animal"`, `kind_hint: None` (the `Name string` field produces no site — it has a `name` field, so it's not embedded).
- A pointer-embedded type (`*Animal`) resolves to the same `target_text` as a value embed.
  - *Given* `type Dog struct {\n\t*Animal\n}`, *When* extracted, *Then* the site's `target_text` is `"Animal"` (the `pointer_type` wrapper is unwrapped, same pattern `go_receiver_type_name` already uses).
- A cross-package qualified embed (`pkg.Base`) keeps the qualifier in `target_text`.
  - *Given* `type Dog struct {\n\tother.Animal\n}` (with an `other` import), *When* extracted, *Then* `target_text` is `"other.Animal"` (resolution splits the qualifier later, Story 1.6.1).
- A generic embed (`Base[T]`) strips the type-argument list from `target_text`.
  - *Given* `type Dog struct {\n\tBase[int]\n}`, *When* extracted, *Then* `target_text` is `"Base"` (via `strip_generic_params`, matching `go_struct_fields`'s sibling handling of generic type names).
- An anonymous embedded struct literal (no named type) produces no site.
  - *Given* `type Foo struct {\n\tstruct{ X int }\n}`, *When* extracted, *Then* no `RawTypeRelationSite` is emitted for that field (there's no target name to resolve — see `research/features.md`'s edge-case table).
- **Files**: `src/symbol_extract.rs`

##### Task 1.2.1a: Define `RawTypeRelationSite` and `type_relation_supports` (~3 min)
- In `src/symbol_extract.rs`, after `RawFieldAccessSite`'s definition (around line 747-755), add the `RawTypeRelationSite` struct (doc comment: mirrors `RawCallSite`/`RawFieldAccessSite`'s "file left empty, caller fills in" convention) and a `type_relation_supports(language: Language) -> bool` fn (mirroring `call_graph_supports` at line 554, returning `true` for `Go | TypeScript | Tsx | JavaScript | Java | Kotlin`).
- Files: `src/symbol_extract.rs`

##### Task 1.2.1b: `go_embedded_type_relations` — walk `type_spec` → `struct_type` → `field_declaration_list`, invert `go_struct_fields`'s skip (~5 min)
- Add `fn go_embedded_type_relations(type_decl: Node, source: &str, package_path: &str, out: &mut Vec<RawTypeRelationSite>)`, reusing `go_struct_fields`'s `type_spec`/`struct_type`/`field_declaration_list` traversal shape (lines 668-705) but branching the opposite way: for a `field_declaration` with **no** `name` field-child, read its `type` field instead of skipping. Handle the four shapes per `research/stack.md`'s verified grammar findings: `type_identifier` (direct), `qualified_type` (`package`+`name` fields, join as `"pkg.Name"`), `pointer_type` (unwrap one level, same pattern as `go_receiver_type_name`), `generic_type` (take `.type` field, ignore `type_arguments` — equivalent to stripping generics). An inline `struct_type` embed (no `type_identifier`/`qualified_type`/`pointer_type`/`generic_type` at the top) is skipped (no target name to resolve). `type_id` is built via the existing `build_id(package_path, None, &type_name)` using the enclosing `type_spec`'s own name.
- Files: `src/symbol_extract.rs`

##### Task 1.2.1c: Wire into `walk_type_relations` dispatcher + `extract_type_relation_sites_for_file` (~3 min)
- Add `fn walk_type_relations(node: Node, language: Language, source: &str, package_path: &str, out: &mut Vec<RawTypeRelationSite>)`, mirroring `walk_struct_fields`'s shape (lines 707-720): for Go, dispatch `type_declaration` nodes to `go_embedded_type_relations`; recurse into children.
- Add `pub fn extract_type_relation_sites_for_file(language: Language, source: &str, tree: &Tree, package_path: &str) -> Vec<RawTypeRelationSite>`: returns `Vec::new()` early if `!type_relation_supports(language)`, else walks `tree.root_node()` via `walk_type_relations`.
- Files: `src/symbol_extract.rs`

##### Task 1.2.1d: Fixture tests for all five Go embed shapes + the anonymous-struct skip, plus the per-language gate (~6 min)
- In `symbol_extract.rs`'s test module, add a `type_relation_sites(language, source, package_path) -> Vec<RawTypeRelationSite>` helper (mirroring `call_sites` at line 1378) and one test per AC above (`go_embedded_type_relations_captures_same_package_embed`, `..._unwraps_pointer_embed`, `..._keeps_qualifier_on_cross_package_embed`, `..._strips_generic_args_on_generic_embed`, `..._skips_anonymous_inline_struct_embed`, `..._skips_named_field_not_embedded`).
- Also add `type_relation_supports_returns_true_for_supported_languages_false_for_python_and_rust` (asserts `true` for Go/TypeScript/Tsx/JavaScript/Java/Kotlin, `false` for Python/Rust — this is the gate every other language epic's extraction depends on and had no direct test until `validation.md`'s Phase 4 review flagged the gap) and `extract_type_relation_sites_for_file_returns_empty_for_unsupported_language` (calls `extract_type_relation_sites_for_file` with `Language::Python`/`Language::Rust` on a source string that *would* produce sites if walked, asserts `Vec::new()` — proves the early-return actually short-circuits the walk, not just that the gate function itself returns the right bool).
- Files: `src/symbol_extract.rs`

---

### Epic 1.3: TypeScript/Tsx/JavaScript `extends`/`implements` extraction
**Goal**: Extract `class_heritage` clauses — with the TS/Tsx-vs-JS grammar divergence
`research/stack.md` flags handled as two branches, not one.

#### Story 1.3.1: TS/Tsx `class_heritage` (`extends_clause`/`implements_clause`) extraction
**As a** #60/#61 checker, **I want** TS/Tsx `extends`/`implements` clauses captured with
their keyword-determined kind, **so that** these sites never need resolve-time kind
classification.
**Acceptance Criteria**:
- A TS class with both `extends` and `implements` produces one `Extends`-hinted site and one `Implements`-hinted site per listed interface.
  - *Given* TS source `class Dog extends Animal implements Runnable, Named {}`, *When* `extract_type_relation_sites_for_file(Language::TypeScript, source, &tree, "pkg")` runs, *Then* it returns three sites for `type_id: "pkg::Dog"`: `{target_text: "Animal", kind_hint: Some(Extends)}`, `{target_text: "Runnable", kind_hint: Some(Implements)}`, `{target_text: "Named", kind_hint: Some(Implements)}`.
- A generic supertype (`extends Base<T>`) strips the type-argument list.
  - *Given* `class Dog<T> extends Base<T> {}`, *When* extracted, *Then* the site's `target_text` is `"Base"`.
- Tsx shares the TypeScript config and produces identical results for the same source shape.
  - *Given* the same `class Dog extends Animal {}` source parsed as `Language::Tsx`, *When* extracted, *Then* it produces the same single `Extends`-hinted site as the TS case (matching `lang_symbol_config(Language::Tsx) => lang_symbol_config(Language::TypeScript)`'s existing delegation at line 203).
- **Files**: `src/symbol_extract.rs`

##### Task 1.3.1a: `ts_class_heritage_relations` — walk `class_declaration` → `class_heritage` → `extends_clause`/`implements_clause` (~5 min)
- Add `fn ts_class_heritage_relations(class_decl: Node, source: &str, package_path: &str, out: &mut Vec<RawTypeRelationSite>)`: find `class_heritage` via `find_child_by_kind` (positional, not a field, per `research/stack.md`); within it, `extends_clause`'s `value` field (`expression`) → one `Extends`-hinted site; `implements_clause`'s positional `type` children (`multiple: true`) → one `Implements`-hinted site each. `target_text` is the whole matched node's text run through `strip_generic_params`. `type_id` is the class's own id — read via `class_decl.child_by_field_name("name")` + `build_id`, mirroring `classify_node`'s existing id-building.
- Files: `src/symbol_extract.rs`

##### Task 1.3.1b: Wire into `walk_type_relations` for TS/Tsx, plus fixture tests (~4 min)
- Extend `walk_type_relations` (Task 1.2.1c): for `Language::TypeScript | Language::Tsx`, dispatch `class_declaration` nodes to `ts_class_heritage_relations`.
- Add tests: `ts_class_heritage_relations_captures_extends_and_multiple_implements`, `ts_class_heritage_relations_strips_generic_args`, `tsx_class_heritage_relations_matches_typescript`.
- Files: `src/symbol_extract.rs`

#### Story 1.3.2: JavaScript `class_heritage` (bare expression) extraction
**As a** #60/#61 checker, **I want** plain-JS `class X extends Y` captured despite JS's
`class_heritage` having no `extends_clause`/`implements_clause` substructure, **so that**
JS classes aren't silently skipped by reusing the TS branch.
**Acceptance Criteria**:
- A JS class with `extends` produces one `Extends`-hinted site; JS has no `implements` keyword, so no `Implements` sites are ever produced for JS.
  - *Given* JS source `class Dog extends Animal {}`, *When* `extract_type_relation_sites_for_file(Language::JavaScript, source, &tree, "pkg")` runs, *Then* it returns exactly one site: `{target_text: "Animal", kind_hint: Some(Extends)}`.
- A JS class with no `extends` produces zero sites (no `class_heritage` node at all).
  - *Given* `class Dog {}`, *When* extracted, *Then* it returns an empty `Vec`.
- **Files**: `src/symbol_extract.rs`

##### Task 1.3.2a: `js_class_heritage_relation` — single positional `expression` child, no wrapper lookup (~3 min)
- Add `fn js_class_heritage_relation(class_decl: Node, source: &str, package_path: &str, out: &mut Vec<RawTypeRelationSite>)`: find `class_heritage` via `find_child_by_kind`; if present, its **one** positional child (no `extends_clause` substructure, per `research/stack.md`'s explicit divergence note) is the superclass expression — emit one `Extends`-hinted site with `strip_generic_params`-normalized text. Do not call `ts_class_heritage_relations` for JS — this is a deliberately separate function per the Pattern Decisions table.
- Wire into `walk_type_relations` for `Language::JavaScript`.
- Files: `src/symbol_extract.rs`

##### Task 1.3.2b: Fixture tests (~2 min)
- Add `js_class_heritage_relation_captures_extends`, `js_class_heritage_relation_empty_for_no_extends`.
- Files: `src/symbol_extract.rs`

---

### Epic 1.4: Java `extends`/`implements` extraction
**Goal**: Handle Java's field-vs-positional-child split (`class_declaration`'s `superclass`/
`interfaces` fields vs. `interface_declaration`'s positional `extends_interfaces`) per
`research/stack.md`'s grammar verification.

#### Story 1.4.1: Java `class_declaration` — `superclass`/`interfaces` fields
**As a** #60/#61 checker, **I want** a Java class's `extends`/`implements` captured via
their named grammar fields, **so that** the single-superclass-plus-multiple-interfaces
shape is read correctly.
**Acceptance Criteria**:
- A class with both a superclass and multiple interfaces produces one `Extends` site and one `Implements` site per interface.
  - *Given* Java source `class Dog extends Animal implements Runnable, Named {}`, *When* `extract_type_relation_sites_for_file(Language::Java, source, &tree, "pkg")` runs, *Then* it returns `{target_text: "Animal", kind_hint: Some(Extends)}` plus `{target_text: "Runnable", kind_hint: Some(Implements)}` and `{target_text: "Named", kind_hint: Some(Implements)}`.
- A class with no `extends`/`implements` produces zero sites.
  - *Given* `class Dog {}`, *When* extracted, *Then* it returns an empty `Vec`.
- **Files**: `src/symbol_extract.rs`

##### Task 1.4.1a: `java_class_type_relations` — `child_by_field_name("superclass"/"interfaces")` (~4 min)
- Add `fn java_class_type_relations(class_decl: Node, source: &str, package_path: &str, out: &mut Vec<RawTypeRelationSite>)`: `class_decl.child_by_field_name("superclass")` → take its single child, one `Extends` site; `class_decl.child_by_field_name("interfaces")` → the `super_interfaces` node's single `type_list` child → iterate `_type` children, one `Implements` site each. `strip_generic_params` on every extracted name.
- Wire into `walk_type_relations` for `Language::Java`'s `class_declaration` nodes.
- Files: `src/symbol_extract.rs`

##### Task 1.4.1b: Fixture tests (~3 min)
- Add `java_class_type_relations_captures_superclass_and_multiple_interfaces`, `java_class_type_relations_empty_for_plain_class`.
- Files: `src/symbol_extract.rs`

#### Story 1.4.2: Java `interface_declaration` — positional `extends_interfaces`
**As a** #60/#61 checker, **I want** `interface Foo extends Bar, Baz` captured via its
positional child (not the class's field names), **so that** interface-to-interface
inheritance isn't mis-tagged or dropped by a naive extractor keyed on `class_declaration`'s
field shape.
**Acceptance Criteria**:
- An interface extending multiple interfaces produces one `Extends` site per listed interface (interface-to-interface inheritance uses the `extends` keyword, not `implements` — per `research/pitfalls.md` §2).
  - *Given* Java source `interface Foo extends Bar, Baz {}`, *When* extracted, *Then* it returns `{target_text: "Bar", kind_hint: Some(Extends)}` and `{target_text: "Baz", kind_hint: Some(Extends)}` for `type_id` built from `Foo`.
- **Files**: `src/symbol_extract.rs`

##### Task 1.4.2a: `java_interface_type_relations` — `find_child_by_kind(node, "extends_interfaces")` (~3 min)
- Add `fn java_interface_type_relations(interface_decl: Node, source: &str, package_path: &str, out: &mut Vec<RawTypeRelationSite>)`: `find_child_by_kind(interface_decl, "extends_interfaces")` → its single `type_list` child → iterate `_type` children, each an `Extends` site (interface-extends-interface, per the AC above — not `Implements`).
- Wire into `walk_type_relations` for `Language::Java`'s `interface_declaration` nodes.
- Files: `src/symbol_extract.rs`

##### Task 1.4.2b: Fixture test (~2 min)
- Add `java_interface_type_relations_captures_multiple_extends_targets_as_extends_kind`.
- Files: `src/symbol_extract.rs`

---

### Epic 1.5: Kotlin supertype-list extraction
**Goal**: Extract `class Foo : Base(), Iface1, Iface2` per `delegation_specifier`'s
structural shape — correctly splitting the two unambiguous cases
(`constructor_invocation`, `explicit_delegation`) from the one genuinely ambiguous case
(bare `type`), per this plan's Pattern Decisions correction of the research disagreement.

#### Story 1.5.1: Kotlin `delegation_specifiers` extraction, three-way split
**As a** #60/#61 checker, **I want** Kotlin's supertype list correctly split into
`Extends`/`Implements`/ambiguous by `delegation_specifier` shape, **so that** the common
"class with no primary constructor" idiom (bare-`type` superclass) doesn't get
misclassified as `implements` by an overeager structural-only rule.
**Acceptance Criteria**:
- A `constructor_invocation`-shaped entry (`Base()`) is `Extends`, hinted at extraction time (unambiguous — only a class can appear with call syntax).
  - *Given* Kotlin source `class Dog : Animal(), Runnable {}`, *When* `extract_type_relation_sites_for_file(Language::Kotlin, source, &tree, "pkg")` runs, *Then* the `Animal` site has `kind_hint: Some(Extends)`.
- An `explicit_delegation`-shaped entry (`by`) is `Implements`, hinted at extraction time (unambiguous — delegation is interface-only).
  - *Given* `class Dog(impl: Runnable) : Runnable by impl {}`, *When* extracted, *Then* the `Runnable` site has `kind_hint: Some(Implements)`.
- A bare-`type` entry (no `constructor_invocation`/`explicit_delegation` wrapper) is emitted with `kind_hint: None` — genuinely ambiguous at extraction time.
  - *Given* `class Dog : Runnable {}` (bare interface reference, no parens), *When* extracted, *Then* the `Runnable` site has `kind_hint: None`.
- The "no primary constructor" idiom (superclass written bare, disambiguated only by resolution against a known `Type`) is captured with `kind_hint: None`, not misclassified as `Implements` — proving the Pattern Decisions table's correction.
  - *Given* `class MyView : View {\n    constructor(ctx: Int) : super(ctx)\n}` (mirroring the real Android idiom where `View` is a class, `research/stack.md`'s original "bare type = interface" claim would wrongly call this `Implements`), *When* extracted, *Then* the `View` site has `kind_hint: None` (left for `resolve_type_edges`, Story 1.6.1, to classify via `View`'s actual `SymbolKind` once both files are in the index).
- A class with multiple bare-`type` interface entries alongside one `constructor_invocation` entry splits correctly.
  - *Given* `class Dog : Animal(), Runnable, Named {}`, *When* extracted, *Then* it returns three sites: `Animal` (`Some(Extends)`), `Runnable` (`None`), `Named` (`None`).
- **Files**: `src/symbol_extract.rs`

##### Task 1.5.1a: `kotlin_delegation_type_relations` — walk `delegation_specifiers` → `delegation_specifier`, three-way dispatch (~5 min)
- Add `fn kotlin_delegation_type_relations(class_decl: Node, source: &str, package_path: &str, out: &mut Vec<RawTypeRelationSite>)`: `find_child_by_kind(class_decl, "delegation_specifiers")` → for each `delegation_specifier` child, inspect its one non-`annotation` child's kind: `"constructor_invocation"` → read its `type` child's text, `Some(TypeRelationKind::Extends)`; `"explicit_delegation"` → read its `type` field's text, `Some(TypeRelationKind::Implements)`; `"type"` (bare) → read its own text, `None`. `strip_generic_params` on every extracted name. `type_id` built from `class_decl`'s own `name` field, same as `ts_class_heritage_relations`.
- Wire into `walk_type_relations` for `Language::Kotlin`'s `class_declaration` nodes (both plain classes and, per `kotlin_is_interface`, interfaces with their own supertype lists — Kotlin interfaces can also list other interfaces after `:`, which is always the bare-`type`/ambiguous shape since interfaces have no constructors).
- Files: `src/symbol_extract.rs`

##### Task 1.5.1b: Fixture tests for all five ACs, including the no-primary-constructor idiom (~5 min)
- Add `kotlin_delegation_type_relations_constructor_invocation_is_extends`, `..._explicit_delegation_is_implements`, `..._bare_type_is_ambiguous`, `..._no_primary_constructor_superclass_is_ambiguous_not_implements` (the AC that directly tests this plan's correction of `research/stack.md`'s original claim), `..._splits_mixed_supertype_list`.
- Files: `src/symbol_extract.rs`

---

### Epic 1.6: Resolution — the whole-repo `Type`/`Interface` index and `resolve_type_edges`
**Goal**: Resolve every language's raw sites into `TypeRelationEdge`s, wired into
`build_model` alongside the existing `resolve_call_edges`/`resolve_field_access_edges`
calls.

#### Story 1.6.1: `TypeRelationIndex`/`TypeRelationPackageIndex` + `resolve_one_type_edge`
**As a** #59/#60/#61 checker, **I want** raw sites resolved the same way `CallEdge`'s
targets are (same-package-preferred, then globally-unique, else unresolved-but-kept), with
Go-qualified names routed through `file_import_aliases`, **so that** resolution accuracy
matches the existing call-graph's accepted ceiling rather than a fresh, untested one.
**Acceptance Criteria**:
- An unqualified name unambiguous in its own package resolves to that package's `Type`/`Interface` id, with `kind` taken from `kind_hint` if `Some`, else from the resolved target's own `SymbolKind`.
  - *Given* Go package `pkg` containing `type Animal struct{}` and `type Dog struct { Animal }`, *When* `resolve_type_edges` runs, *Then* it produces `TypeRelationEdge { from: "pkg::Dog", to: "pkg::Animal", kind: Some(Extends), resolved: true, ... }` (kind inferred: `Animal`'s `SymbolKind` is `Type`).
- The same site against an interface target infers `Implements`.
  - *Given* Go package `pkg` containing `type Reader interface{ Read() }` and `type File struct { Reader }`, *When* resolved, *Then* it produces `kind: Some(Implements)` (inferred: `Reader`'s `SymbolKind` is `Interface`).
- An unresolvable target (not in the whole-repo index — e.g. stdlib) is kept with `resolved: false` and the raw text in `to`; if `kind_hint` was `None`, the final `kind` is also `None` (can't classify without knowing the target).
  - *Given* Go source `type Locker struct { sync.Mutex }` with no `sync` package in this model, *When* resolved, *Then* it produces `TypeRelationEdge { from: "pkg::Locker", to: "sync.Mutex", kind: None, resolved: false, ... }`.
- A Go qualified embed (`other.Animal`) resolves via `file_import_aliases`, not blind same-package/globally-unique matching.
  - *Given* file `f.go` with an alias entry `file_import_aliases[f.go]["other"] == "example.com/app/other"`, and package `example.com/app/other` declaring `type Animal struct{}`, and `type Dog struct { other.Animal }` declared in `f.go`, *When* resolved, *Then* `to` is `"example.com/app/other::Animal"`, `resolved: true` — not resolved by name-only fallback (which the AC in the next line proves would find a *different*, wrong `Animal` if one existed elsewhere).
- A keyword-hinted site (TS/Java/Kotlin `constructor_invocation`/`explicit_delegation`) keeps its `kind_hint` regardless of the resolved target's actual `SymbolKind` (source syntax wins over inference when both are available — though in valid source they should always agree).
  - *Given* a TS `implements` site with `kind_hint: Some(Implements)` whose target happens to resolve to a `SymbolNode` (correctly) of kind `Interface`, *When* resolved, *Then* `kind` is `Some(Implements)` (taken from `kind_hint`, not re-derived).
- A keyword-hinted site whose target does *not* resolve (a common case for TS/Java/Kotlin, not a rare corner case — classes routinely extend/implement framework or stdlib types this model never indexes, e.g. `extends React.Component`) still keeps its `kind_hint`, rather than silently dropping it or falling back to `None` just because resolution failed.
  - *Given* a TS `implements Foo` site with `kind_hint: Some(Implements)` where `Foo` is not present in this model's `TypeRelationIndex` (an external/framework interface), *When* resolved, *Then* it produces `TypeRelationEdge { kind: Some(Implements), resolved: false, to: "Foo", .. }` — `kind_hint` alone is sufficient to set `kind`; resolution success is only needed to *infer* a kind when no `kind_hint` was available (see the `sync.Mutex` AC above, which is Go's `kind_hint: None` case — this AC covers the `kind_hint: Some` + unresolved combination that AC doesn't exercise).
- **Files**: `src/arch_model.rs`

##### Task 1.6.1a: `TypeRelationIndex`/`TypeRelationPackageIndex` types + `build_type_relation_indexes` (~4 min)
- Add `type TypeRelationIndex<'a> = HashMap<&'a str, Vec<(&'a str, &'a str, SymbolKind)>>;` (name -> `[(package, id, kind)]`) and `type TypeRelationPackageIndex<'a> = HashMap<(&'a str, &'a str), (&'a str, SymbolKind)>;` (`(package, name) -> (id, kind)`).
- Add `fn build_type_relation_indexes(packages: &BTreeMap<String, PackageNode>) -> (TypeRelationIndex<'_>, TypeRelationPackageIndex<'_>)`, mirroring `build_call_target_indexes` (lines 479-500) but filtering to `SymbolKind::Type | SymbolKind::Interface` only and carrying `kind` in both index value shapes.
- Files: `src/arch_model.rs`

##### Task 1.6.1b: `resolve_in_typed` (kind-carrying `resolve_in`) (~3 min)
- Add `fn resolve_in_typed<'a>(index: &TypeRelationIndex<'a>, caller_pkg: &str, name: &str) -> Option<(&'a str, SymbolKind)>`, same same-package-preferred/globally-unique/`None`-on-ambiguity algorithm as `resolve_in` (lines 455-472), adapted to return `(id, kind)` instead of just `id`.
- Files: `src/arch_model.rs`

##### Task 1.6.1c: `resolve_one_type_edge` — qualifier split, alias routing, kind_hint-vs-inferred logic (~5 min)
- Add `fn resolve_one_type_edge(type_index: &TypeRelationIndex, package_index: &TypeRelationPackageIndex, file_import_aliases: &BTreeMap<PathBuf, HashMap<String, String>>, site: RawTypeRelationSite) -> TypeRelationEdge`:
  1. If `site.target_text` contains `'.'`, split into `(qualifier, name)`. Look up `file_import_aliases.get(&site.file).and_then(|m| m.get(qualifier))` → if `Some(target_pkg)`, look up `package_index.get(&(target_pkg, name))`. If that alias lookup finds nothing (not Go, or no alias entry), fall back to `resolve_in_typed(type_index, caller_package(&site.type_id), name)` (last-segment-only, matching `CallEdge`'s existing ceiling per `research/pitfalls.md` §1).
  2. Else (unqualified), `resolve_in_typed(type_index, caller_package(&site.type_id), &site.target_text)`.
  3. Determine `kind`: `site.kind_hint` if `Some`; else the resolved target's `SymbolKind` mapped `Type -> Extends, Interface -> Implements`, or `None` if unresolved.
  4. Build the `TypeRelationEdge`: `to`/`resolved` per whether step 1/2 found a match, matching `resolve_one_call_edge`'s `Some`/`None` branching (lines 528-543).
- Files: `src/arch_model.rs`

##### Task 1.6.1d: Unit tests for every AC in Story 1.6.1 (~5 min)
- In `arch_model.rs`'s test module, add one test per AC: `resolve_type_edges_infers_extends_kind_from_resolved_type_target`, `..._infers_implements_kind_from_resolved_interface_target`, `..._keeps_unresolved_edge_with_raw_text_and_none_kind`, `..._routes_qualified_go_embed_through_file_import_aliases`, `..._prefers_kind_hint_over_inferred_kind_when_both_available`, `..._keeps_kind_hint_when_target_is_unresolved`.
- Files: `src/arch_model.rs`

#### Story 1.6.2: Wire `resolve_type_edges` and `collect_type_relation_sites` into `build_model`
**As a** kibitzer maintainer, **I want** `type_edges` populated the same way `call_edges`/
`field_accesses` are — one raw-site collection during the existing per-file walk, one
resolution pass after — **so that** the non-functional requirement ("same tree-sitter walk
pass ... not a second full-repo pass") holds.
**Acceptance Criteria**:
- `build_model` on a two-file Go fixture (one package, `Animal`/`Dog` as in Story 1.6.1's first AC) produces a non-empty `ArchModel.type_edges`.
  - *Given* the fixture files, *When* `build_model` runs, *Then* `model.type_edges` contains the resolved `Dog -> Animal` `Extends` edge.
- A mixed-language repo (one Go package, one TS package, each with an unrelated hierarchy) produces edges for both, each correctly kinded.
  - *Given* a Go fixture (`Dog extends Animal` via embedding) and a TS fixture (`class Cat extends Feline {}`) in the same `build_model` call, *When* it runs, *Then* `model.type_edges` contains one edge per language, each `resolved: true`.
- **Files**: `src/arch_model.rs`

##### Task 1.6.2a: `collect_type_relation_sites` wrapper (~2 min)
- Add `fn collect_type_relation_sites(language: Language, source: &str, tree: &tree_sitter::Tree, package_path: &str, file: &Path) -> Vec<RawTypeRelationSite>`, mirroring `collect_call_sites` (lines 271-285): calls `extract_type_relation_sites_for_file`, fills in `file` on each site.
- Files: `src/arch_model.rs`

##### Task 1.6.2b: `resolve_type_edges` (top-level resolver) (~2 min)
- Add `fn resolve_type_edges(packages: &BTreeMap<String, PackageNode>, file_import_aliases: &BTreeMap<PathBuf, HashMap<String, String>>, raw_sites: Vec<RawTypeRelationSite>) -> Vec<TypeRelationEdge>`, mirroring `resolve_call_edges` (lines 549-558): builds indexes via `build_type_relation_indexes`, maps each raw site through `resolve_one_type_edge`.
- Files: `src/arch_model.rs`

##### Task 1.6.2c: Wire both into `build_model`'s loop and post-loop resolution (~4 min)
- In `build_model`'s per-file loop (around line 395-415), add `let mut raw_type_relation_sites: Vec<RawTypeRelationSite> = Vec::new();` (declared alongside `raw_call_sites`/`raw_field_access_sites` near line 330) and, inside the loop, `raw_type_relation_sites.extend(collect_type_relation_sites(language, source, &tree, &package_path, path));`.
- After the loop, replace the `type_edges: Vec::new(),` placeholder from Task 1.1.1b with a call to `resolve_type_edges(&packages, &file_import_aliases, raw_type_relation_sites)` — note this must come *after* `file_import_aliases` is computed (line 420-421), since Go qualified-embed resolution depends on it; reorder `build_model`'s post-loop block accordingly if needed (compute `file_import_aliases` before `type_edges`).
- Files: `src/arch_model.rs`

##### Task 1.6.2d: Integration tests for both ACs, plus a corpus-wide corruption-guard invariant (~7 min)
- Add `build_model_populates_type_edges_for_go_embedding`, `build_model_populates_type_edges_across_go_and_typescript_in_one_call` to `arch_model.rs`'s test module, using the existing multi-file fixture-building test helpers already used by nearby `call_edges`/`field_accesses` integration tests.
- Add `build_model_resolves_qualified_go_embed_across_two_packages_through_the_real_pipeline`: a two-package Go fixture with an aliased cross-package embed (`import other "pkg/other"`, `type Dog struct { other.Animal }`), run through the actual `build_model` entry point (not `resolve_type_edges` called directly with a hand-built `file_import_aliases` map, which is all Story 1.6.1's AC exercises) — asserts the resolved edge's `to` matches `other.Animal`'s real `SymbolNode::id`. This closes the ordering risk Task 1.6.2c's own note flags (`file_import_aliases` must be computed before `type_edges`): a future reorder of `build_model`'s post-loop block would silently degrade this case to the wrong (name-only) fallback rather than crash, and this is the only test that runs the real ordering instead of a hand-built map.
- Add `build_model_type_edges_invariant_every_resolved_edge_targets_a_real_type_or_interface_node`: a corpus-style invariant check, not a single-fixture example. **Sequencing note**: Epic 3.1's benchmark fixture (Task 3.1.1a) is Phase 3, sequenced after this Phase 1 task, and lives under a different file (`src/arch_export.rs`) than this task's own `src/arch_model.rs` — it can't be reused *yet* without a forward dependency. Build a small, local, inline multi-language fixture directory for this test instead (one Go file with a same-package embed, one TS file with `extends`/`implements`, one Kotlin file with a `constructor_invocation` supertype — enough to exercise all three `kind` outcomes: `Some(Extends)`, `Some(Implements)`, and `None` for an unresolved Go embed of an external type), inline in `arch_model.rs`'s test module, not shared with Epic 3.1. Then assert for every `edge` in `model.type_edges` where `edge.resolved`: (1) `edge.to` exists as a `SymbolNode::id` in `model.packages` with `kind` of `Type` or `Interface` (never a dangling id, never a `Function`/`Method`); (2) if `edge.kind.is_some()`, it agrees with the target's actual `SymbolKind` (`Extends` ↔ `Type`, `Implements` ↔ `Interface`) — never a silent mismatch between the edge's claimed kind and the resolved target's real kind. (Epic 3.1's later benchmark fixture may optionally converge on reusing this same small fixture once both exist — not required, just a possible follow-up simplification, and Task 3.1.1a's own description is unaffected either way.) **Rationale (pre-mortem P1, `implementation/pre-mortem.md` #1)**: `type_edges` is consumed as a raw library field by not-yet-built checkers #59/#60/#61 with no re-verification of their own — a resolution bug here has no feedback path until those checkers exist and someone traces a bad refactoring suggestion back to this "done" feature. This test converts that into a merge-time, CI-enforced failure instead of a someday-maybe discovery.
- Files: `src/arch_model.rs`

---

## Phase 2: MCP query tools

### Epic 2.1: `list_supertypes`/`list_subtypes`
**Goal**: Ship the two MCP tools per ADR-001's `depth`+pagination-combined shape.

#### Story 2.1.1: `TypeHierarchyRequest`/`TypeHierarchyResponse` + `traverse_type_edges` BFS
**As an** AI agent triaging a hierarchy-shaped refactor, **I want** a `depth`-bounded,
paginated walk over `type_edges`, **so that** I can answer both "direct parent" (#59) and
"full ancestor chain" (#61 Extract Superclass) questions without re-implementing BFS
myself.
**Acceptance Criteria**:
- At `depth: 1` (the default), only direct edges are returned, with `truncated` signaling whether a further hop would surface more.
  - *Given* `type_edges` containing `Dog -> Animal` (`Extends`) and `Animal -> LivingThing` (`Extends`), *When* `traverse_type_edges(&model.type_edges, "pkg::Dog", 1, TypeHierarchyDirection::Supertypes)` runs, *Then* it returns `([Dog->Animal], true)` — `truncated: true` because `Animal -> LivingThing` exists one hop further out.
- At `depth: 2`, the full two-hop ancestor chain is returned, `truncated: false`.
  - *Given* the same edges, *When* traversed at `depth: 2`, *Then* it returns `([Dog->Animal, Animal->LivingThing], false)`.
- `depth` beyond `MAX_TYPE_HIERARCHY_DEPTH` is clamped, not rejected.
  - *Given* a request with `depth: 999`, *When* the tool handles it, *Then* the effective depth used is `10` (`MAX_TYPE_HIERARCHY_DEPTH`), not an error.
- A diamond/repeated-ancestor shape doesn't loop or duplicate infinitely (visited-set safety, mirroring `traverse_call_edges`'s proven cycle safety).
  - *Given* `type_edges` containing `B -> A`, `C -> A`, `D -> B`, `D -> C` (D has two paths to A), *When* traversed from `D` at `depth: 5`, *Then* the walk terminates and `A` appears via both `B` and `C`'s edges but the walk doesn't hang or infinitely recurse.
- **Files**: `src/mcp.rs`

##### Task 2.1.1a: `TypeHierarchyDirection` + `build_type_adjacency`/`expand_type_frontier`/`traverse_type_edges` (~5 min)
- Add `#[derive(Debug, Clone, Copy)] enum TypeHierarchyDirection { Supertypes, Subtypes }` with an `endpoints` method (mirroring `CallDirection::endpoints`, lines 256-264): `Supertypes -> (&edge.from, &edge.to)`, `Subtypes -> (&edge.to, &edge.from)`.
- Add `type TypeAdjacency<'a> = HashMap<&'a str, Vec<&'a TypeRelationEdge>>;`, `fn build_type_adjacency`, `fn expand_type_frontier`, `fn traverse_type_edges` — direct structural copies of `build_call_adjacency`/`expand_call_frontier`/`traverse_call_edges` (lines 272-341) retyped to `TypeRelationEdge`/`TypeHierarchyDirection`.
- Add `const MAX_TYPE_HIERARCHY_DEPTH: usize = 10;` near `MAX_CALL_DEPTH` (line 188).
- Files: `src/mcp.rs`

##### Task 2.1.1b: `TypeHierarchyRequest`/`TypeHierarchyResponse` structs (~4 min)
- Add, near `CallTraversalRequest`/`CallTraversalResponse` (around line 168-236):
  ```rust
  #[derive(Serialize, Deserialize, JsonSchema)]
  struct TypeHierarchyRequest {
      path: String,
      node: String,
      #[serde(default = "default_depth")]
      depth: usize,
      #[serde(default)]
      include_private: bool,
      #[serde(default = "default_limit")]
      limit: usize,
      #[serde(default)]
      cursor: Option<String>,
  }

  #[derive(Serialize)]
  struct TypeHierarchyResponse {
      node: String,
      depth: usize,
      truncated: bool,
      total_matched: usize,
      returned: usize,
      next_cursor: Option<String>,
      possibly_pruned: bool,
      /// Set only when `node` resolves to a SymbolNode that isn't a Type/Interface.
      /// Typed `Option<SymbolKind>`, not a hand-written string — SymbolKind's existing
      /// `#[serde(rename_all = "lowercase")]` already serializes to the same JSON shape
      /// (e.g. `"function"`) a raw String field would have, with compile-time safety
      /// against a new SymbolKind variant being silently missed.
      node_kind: Option<SymbolKind>,
      hint: Option<String>,
      edges: Vec<TypeRelationEdge>,
  }
  ```
  (Reuses existing `default_depth`/`default_limit` fns from `CallTraversalRequest`/`ListArchitectureSymbolsRequest`.)
- Files: `src/mcp.rs`

##### Task 2.1.1c: Unit tests for `traverse_type_edges` (~4 min)
- Add `traverse_type_edges_direct_only_at_depth_one_reports_truncated`, `traverse_type_edges_full_chain_at_depth_two_not_truncated`, `traverse_type_edges_diamond_terminates_via_visited_set` to `mcp.rs`'s test module (near the existing `traverse_call_edges_*` tests at lines 1155-1186).
- Files: `src/mcp.rs`

#### Story 2.1.2: `list_supertypes`/`list_subtypes` tool registration
**As an** AI agent, **I want** `list_supertypes`/`list_subtypes` discoverable and
self-describing on first call, with the same error/empty-result/pruning conventions as
every other architecture-query tool, **so that** I don't have to guess response shape or
misinterpret an empty result as an error.
**Acceptance Criteria**:
- `list_supertypes(node)` returns the direct-and-transitive-per-depth set of what `node` extends/implements; `list_subtypes(node)` returns the reverse.
  - *Given* `type_edges` containing `Dog -> Animal` (`Extends`), *When* `list_supertypes` is called with `node: "pkg::Dog"`, *Then* the response's `edges` contains the `Dog -> Animal` edge; *When* `list_subtypes` is called with `node: "pkg::Animal"`, *Then* the response's `edges` also contains the same edge (same edge, opposite query direction).
- Both tool descriptions state the `depth` default/clamp and the `resolved: false` meaning, per `research/ux.md` §1.
  - *Given* the registered tool router, *When* its description string for `list_supertypes` is inspected, *Then* it mentions `depth`, `1`, `MAX_TYPE_HIERARCHY_DEPTH`'s value (`10`), and names an external/vendored/stdlib cause for `resolved: false` (mirroring `call_traversal_tools_are_registered_with_json_descriptions`'s existing assertion style, `mcp.rs:2343`).
- Querying a `node` id that doesn't exist in the model returns an empty-edges response, not an error.
  - *Given* `node: "pkg::DoesNotExist"`, *When* `list_supertypes` is called, *Then* the response is `{"edges": [], "truncated": false, ...}` with no `"error"` key (mirroring `list_callees_returns_empty_array_not_an_error_for_an_unknown_node`, `mcp.rs:1948-1963`).
- Querying a `node` that exists but is a `Function`/`Method` (not `Type`/`Interface`) returns empty edges plus a `node_kind`/`hint` pair, not an error.
  - *Given* `node: "pkg::DoSomething"` where `DoSomething` is a `Function` `SymbolNode`, *When* `list_supertypes` is called, *Then* the response has `edges: []`, `node_kind: Some(SymbolKind::Function)` (serializing to JSON as `"function"`, via `SymbolKind`'s existing `#[serde(rename_all = "lowercase")]` — see Task 2.1.1b), `hint: Some("node exists but is not a Type or Interface; call get_architecture_node to check node.kind before calling list_supertypes")`.
- Pagination follows `list_architecture_symbols`'s cursor convention exactly.
  - *Given* a `node` whose supertype-direction BFS at the requested depth collects 5 edges and `limit: 2`, *When* `list_supertypes` is called with no `cursor`, *Then* the response has `returned: 2`, `total_matched: 5`, `next_cursor: Some("2")`; *When* called again with `cursor: "2"`, *Then* it returns edges 3-4 and `next_cursor: Some("4")`.
- A malformed `cursor` is a hard error, matching `list_architecture_symbols`'s convention.
  - *Given* `cursor: "not-a-number"`, *When* `list_supertypes` is called, *Then* the response is `{"error": "invalid cursor: ..."}`.
- **Files**: `src/mcp.rs`

Per the architecture review's Blocker finding: a single `type_hierarchy` function combining
`call_traversal`'s full responsibility set (resolve root, clamp depth, load model, BFS
traverse — ~27 lines, `src/mcp.rs:1060-1086`) with `list_architecture_symbols`'s full
pagination/pruning responsibility set (cursor parsing, skip/take/next_cursor,
`possibly_pruned` computation — ~93 lines, `src/mcp.rs:816-911`) would land as 70-100+
lines — `mcp.rs`'s 13th over-40-line function (it already has 12, confirmed via
`kibitzer run src --trigger batch`), not the ~40-line estimate this plan originally gave.
Tasks 2.1.2a-c below split that one function into three small pieces instead — see the
corrected Tech Debt Disposition table entry below for the resulting size estimates.

##### Task 2.1.2a: Extract a shared `paginate<T>` helper out of `list_architecture_symbols`, ahead of adding `type_hierarchy` (~4 min)
- Add `fn paginate<T>(items: Vec<T>, offset: usize, limit: usize) -> (Vec<T>, usize, Option<String>)` (returns `(page, total_matched, next_cursor)`) near `list_architecture_symbols` (`src/mcp.rs:816`), moving its existing skip/take/next_cursor logic (~lines 872-881) into the new fn's body verbatim.
- Update `list_architecture_symbols` to call `paginate(...)` instead of the inline logic it's replacing. This is a small, behavior-preserving refactor of already-shipped code, not new functionality — every existing `list_architecture_symbols` pagination test must still pass unchanged after this task, with no test text edits required.
- This task is sequenced first in Story 2.1.2 (before `type_hierarchy` exists) specifically so `type_hierarchy` (Task 2.1.2c) is written against the shared helper from the start, rather than duplicating the skip/take/next_cursor logic and extracting it in a follow-up.
- Files: `src/mcp.rs`

##### Task 2.1.2b: `node_kind`/`hint` early-response helper (~3 min)
- Add a small helper (e.g. `fn non_type_node_hint(model: &ArchModel, node: &str) -> Option<(SymbolKind, &'static str)>`) that scans `model.packages` for a `SymbolNode` matching `node`'s id (same lookup pattern `get_architecture_node` uses, `src/mcp.rs:943-949`) and, when found and not `Type`/`Interface`, returns `Some((kind, hint_str))`; returns `None` when `node` doesn't resolve at all, or resolves to a `Type`/`Interface` (both cases fall through to normal BFS handling in `type_hierarchy`).
- This isolates the "does `node` resolve to a `Type`/`Interface`, else return `node_kind`/`hint` early" check called out by the architecture review as its own testable unit, separate from `type_hierarchy`'s BFS/pagination flow.
- Files: `src/mcp.rs`

##### Task 2.1.2c: Slim `type_hierarchy` shared body, built from the two extracted helpers (~4 min)
- Add `async fn type_hierarchy(&self, req: TypeHierarchyRequest, direction: TypeHierarchyDirection) -> String` to `impl KibitzerServer` (near `call_traversal`, line 1060): resolve repo root (`Self::resolve_repo_root`), clamp `depth` to `[1, MAX_TYPE_HIERARCHY_DEPTH]`, parse `cursor` into an `offset` (same non-numeric-cursor `json_error` handling `list_architecture_symbols` already has), load the model via `load_model_off_stack(repo_root, req.include_private)`.
- Call `non_type_node_hint(&model, &req.node)` (Task 2.1.2b) — if `Some((kind, hint))`, return the standard empty-edges response envelope with `node_kind`/`hint` set, skipping BFS entirely.
- Otherwise, call `traverse_type_edges(&model.type_edges, &req.node, depth, direction)`, then `paginate(edges, offset, req.limit)` (Task 2.1.2a) instead of hand-copied skip/take/next_cursor logic, compute `possibly_pruned` analogously to `list_architecture_symbols`'s (zero results, `include_private` false, pruning summary non-empty), and serialize `TypeHierarchyResponse`.
- With both helpers extracted, this function's own body should land around 15-20 lines — comparable to `call_traversal`'s existing size, not a combination of both precedents' full responsibility sets.
- Files: `src/mcp.rs`

##### Task 2.1.2d: `list_supertypes`/`list_subtypes` tool methods with full descriptions (~4 min)
- Add two `#[tool(description = "...")]`-annotated methods near `list_callers`/`list_callees` (line 970-993): `list_supertypes` (description: "Backward type-hierarchy traversal: what does `node` (a Type/Interface SymbolNode id) extend or implement, up to `depth` hops (default 1, clamped to 10) — returns JSON ({node, depth, truncated, total_matched, returned, next_cursor, possibly_pruned, node_kind, hint, edges}), not prose. An edge's kind is extends or implements (None when source syntax and resolution both leave it ambiguous); resolved: false names an external/vendored/stdlib supertype this model can't resolve to a symbol id. Called on a node that isn't a Type/Interface, returns empty edges plus a node_kind/hint pair, not an error — see get_architecture_node to check node.kind first. Use for hierarchy-shaped refactoring checks (Extract Superclass, Collapse Hierarchy, Pull Up/Push Down Method/Field). For Go, this only reports interface satisfaction expressed via struct embedding — Go's structural (method-set-only) interface satisfaction is not detected; an empty result for a Go interface does not mean nothing implements it."), `list_subtypes` (mirrored, forward direction, same Go caveat).
- Both delegate to `self.type_hierarchy(req.0, TypeHierarchyDirection::Supertypes | Subtypes).await`.
- Files: `src/mcp.rs`

##### Task 2.1.2e: Integration tests for every AC in Story 2.1.2 (~5 min)
- Add to `mcp.rs`'s test module (near the existing `list_callers`/`list_callees` integration tests, lines 1800-1963): `list_supertypes_returns_direct_edge_by_default`, `list_subtypes_returns_reverse_direction_of_same_edge`, `list_supertypes_tool_description_mentions_depth_and_resolved_semantics`, `list_supertypes_returns_empty_array_not_an_error_for_an_unknown_node`, `list_supertypes_reports_node_kind_and_hint_for_a_function_id`, `list_supertypes_paginates_via_next_cursor`, `list_supertypes_rejects_malformed_cursor`.
- Also add direct unit tests for the two new helpers: `paginate_splits_items_by_offset_and_limit_and_reports_next_cursor` (Task 2.1.2a) and `non_type_node_hint_returns_kind_and_hint_for_a_function_id_and_none_for_a_type_id` (Task 2.1.2b).
- Files: `src/mcp.rs`

---

## Phase 3: Validation

### Epic 3.1: Performance regression guard
**Goal**: Per `research/pitfalls.md` §3's recommendation, extend the existing coarse perf
guard rather than build new perf infrastructure.

#### Story 3.1.1: Extend the 5s benchmark fixture with hierarchy-bearing source
**As a** kibitzer maintainer, **I want** `type_edges` extraction covered by the existing
whole-repo perf regression guard, **so that** a future accidental quadratic resolution
pass is caught the same way a slow `call_edges`/`field_accesses` regression would be.
**Acceptance Criteria**:
- The benchmark fixture's Go/TS source includes at least one embedding/`extends` relationship per file, and the whole export still completes under 5s.
  - *Given* `run_export_completes_under_5s_on_benchmark_fixture`'s 80-file fixture (40 Go + 40 TS, `src/arch_export.rs:306-330`) modified so each `Widget{i}` Go struct embeds a shared `Base` type and each `Shape{i}` TS interface extends a shared `BaseShape` interface, *When* the test runs, *Then* `run_export` still completes in under 5 seconds and `elapsed` is asserted the same way the existing test already does.
- **Files**: `src/arch_export.rs`

##### Task 3.1.1a: Add a shared `Base`/`BaseShape` type to the fixture and embed/extend it from every generated type (~4 min)
- In `run_export_completes_under_5s_on_benchmark_fixture` (`src/arch_export.rs:306-330`), add one `write_fixture(&dir, "shared/base.go", "package shared\n\ntype Base struct{}\n")` and one `write_fixture(&dir, "web/shared/base.ts", "export interface BaseShape { area(): number; }\n")` before the loop; modify the per-iteration `go_src`/`ts_src` templates so `Widget{i}` embeds `shared.Base` (with a matching import) and `Shape{i}` extends `BaseShape` (with a matching import), per the language-specific import syntax already exercised elsewhere in this file's other fixtures.
- Files: `src/arch_export.rs`

##### Task 3.1.1b: Confirm the timing assertion still passes with the added extraction work (~2 min)
- Run `cargo test run_export_completes_under_5s_on_benchmark_fixture -- --nocapture` and confirm the printed/asserted elapsed time; no code change expected here, this task is the verification step the AC requires.
- Files: none (verification only)

### Epic 3.2: Kotlin backtest-corpus gap
**Goal**: Resolve `requirements.md`'s last Open Question as an in-scope task (not deferred)
— `research/pitfalls.md` §2/§5 and this repo's own `docs/backtesting.md` convention treat
corpus backtesting as a "not done without it" gate for a new extraction capability, and this
feature is the first to require Kotlin `extends`/`implements` extraction.

#### Story 3.2.1: Add a Kotlin repo to the real-world backtest corpus and spot-check `type_edges` against it
**As a** kibitzer maintainer, **I want** at least one real-world Kotlin codebase in the
backtest corpus, with `type_edges` extraction spot-checked against it, **so that** Kotlin's
three-way `delegation_specifier` split (Story 1.5.1) is validated against real code, not
just hand-written fixtures — closing the exact gap `research/pitfalls.md` §2 names.
**Acceptance Criteria**:
- `docs/backtest-repos.md` lists a Kotlin repo with a "why chosen" justification, and its "No Python or Kotlin exemplar" caveat sentence is removed or updated to reflect Kotlin's addition.
  - *Given* `docs/backtest-repos.md`'s current table (14 rows, no Kotlin), *When* this task lands, *Then* the table has a new `android/nowinandroid` row (`Kotlin | Google's official, actively-maintained modern-Kotlin/Jetpack-Compose sample app — idiomatic sealed-class/interface hierarchies and the no-primary-constructor Android View idiom Story 1.5.1's extraction depends on getting right.`), and the "No Python or Kotlin exemplar is in this list yet" sentence is updated to "No Python exemplar is in this list yet" (Kotlin is now covered, Python remains a real gap for a future Python-scoped feature).
- `scripts/clone-backtest-repos.sh`'s `REPOS` array includes the new repo.
  - *Given* the script's current 13-entry array, *When* this task lands, *Then* it has 14 entries including `"android/nowinandroid"`.
- A manual spot-check of `type_edges` extracted from the cloned repo finds no systematic misclassification (e.g. "every bare-`type` Kotlin supertype resolved as `Extends` when it should be `Implements`, or vice versa").
  - *Given* the repo cloned via `scripts/clone-backtest-repos.sh` and `kibitzer architecture export` run against it (or an equivalent `ArchModel`-building CLI invocation), *When* a sample of `type_edges` entries (at least 20, biased toward Kotlin files with visible `sealed class`/`: SomeInterface` supertype lists) is manually compared against the source, *Then* no systematic kind-classification error is found — an individual unresolved external-library edge (e.g. extending an AndroidX base class not in this model) is expected and fine; a *pattern* of wrong `Extends`/`Implements` classification is not.
- **Files**: `docs/backtest-repos.md`, `scripts/clone-backtest-repos.sh`

##### Task 3.2.1a: Add `android/nowinandroid` to `docs/backtest-repos.md` and `scripts/clone-backtest-repos.sh` (~3 min)
- Add the table row and update the caveat sentence in `docs/backtest-repos.md` per the AC above.
- Add `"android/nowinandroid"` to the `REPOS` array in `scripts/clone-backtest-repos.sh`.
- Files: `docs/backtest-repos.md`, `scripts/clone-backtest-repos.sh`

##### Task 3.2.1b: Clone the repo and run the extraction spot-check (~5 min, manual/verification task — not a code change)
- Run `scripts/clone-backtest-repos.sh` (or a direct `git clone --depth 1` of `android/nowinandroid` into `~/code/github.com/android/nowinandroid`, per the repo-placement convention).
- Run kibitzer's architecture export (or an equivalent `ArchModel`-building path) against the clone, inspect a sample of `type_edges` for Kotlin-sourced entries, and record (in the PR description per this repo's Proportionality convention — not a new doc file) whether any systematic misclassification was found.
- Files: none (verification only; findings go in the PR description, not a new file)

#### Story 3.2.2: Spot-check Go/Java/TS `type_edges` against the existing corpus
**As a** kibitzer maintainer, **I want** Go/Java/TypeScript `type_edges` extraction
spot-checked against already-cloned real-world repos, **so that** validation rigor isn't
asymmetric across languages — Kotlin gets a dedicated new corpus repo (Story 3.2.1) because
its three-way classification split is the most novel logic in this feature, but Go
interface-embeds-interface and Java's `interface Foo extends Bar, Baz` (multi-target,
`extends`-not-`implements`) are exactly the real-world shapes `research/pitfalls.md` §2
flags as fixture-blind-spots, and the corpus already contains both at zero marginal clone
cost (flagged as an open Concern by `adversarial-review.md`, `pre-mortem.md` Failure #2,
and independently by this plan's engineering triad review — addressed here rather than
deferred a third time).
**Acceptance Criteria**:
- A sample of `type_edges` extracted from `kubernetes/kubernetes` (Go) and
  `apache/cassandra` (Java) — both already in `docs/backtest-repos.md`'s corpus, no new
  clone needed — is manually reviewed and finds no systematic misclassification.
  - *Given* `kubernetes/kubernetes` and `apache/cassandra` already cloned per
    `scripts/clone-backtest-repos.sh`, *When* kibitzer's architecture export (or an
    equivalent `ArchModel`-building path) runs against each and a sample of `type_edges`
    (at least 15 per repo, biased toward Go struct embedding and Java
    `interface`-extends-`interface` declarations) is manually compared against source,
    *Then* no systematic kind-classification error is found — individual unresolved
    external/stdlib edges are expected and fine, a *pattern* of wrong classification is not.
- **Files**: none (verification only; findings recorded in the PR description, same
  convention as Task 3.2.1b)

##### Task 3.2.2a: Run the Go and Java extraction spot-check (~5 min, manual/verification task — not a code change)
- Run kibitzer's architecture export against the already-cloned `kubernetes/kubernetes` and `apache/cassandra` corpus repos, inspect a sample of `type_edges` per the AC above, record findings in the PR description.
- Files: none (verification only)

---

## Scope Note: Mergeable Core vs. Deferrable Hardening
Per `pre-mortem.md` Failure #3 and this plan's engineering triad review: Phase 1 (`ArchModel.type_edges`, all six language extractors, resolution) + Phase 2 (`list_supertypes`/`list_subtypes` MCP tools) together are the mergeable v1 that satisfies `requirements.md`'s Success Metrics and unblocks #59/#60/#61 — the actual point of this item. Phase 3 (Epic 3.1's perf-fixture extension, Epic 3.2's Kotlin/Go/Java corpus spot-checks) is validation hardening, not core capability; if a solo maintainer's calendar runs short mid-implementation, Phase 3 can ship as a fast-follow PR without blocking #59/#60/#61 from starting against Phase 1+2's `type_edges`. This is an explicit, pre-agreed checkpoint, not permission to skip Phase 3 silently — the corpus spot-checks (Stories 3.2.1/3.2.2) are still required before any of #59/#60/#61 should be treated as validated against real-world code, per this repo's "not done without it" corpus convention; "fast-follow" means a follow-up PR, not "never."

---

## Summary of new files touched (no new files created)
- `src/arch_model.rs` — `TypeRelationEdge`/`TypeRelationKind`, `ArchModel.type_edges`, `collect_type_relation_sites`, `TypeRelationIndex`/`TypeRelationPackageIndex`, `build_type_relation_indexes`, `resolve_in_typed`, `resolve_one_type_edge`, `resolve_type_edges`, wiring into `build_model`; plus `type_edges: vec![],` added to its own 7 other pre-existing `ArchModel { ... }` test-literal sites (Task 1.1.1c).
- `src/arch_diagram.rs`, `src/architecture_checks.rs`, `src/isp_fat_interface.rs`, `src/god_class.rs`, `src/extract_class.rs`, `src/lsp.rs` — one-line `type_edges: vec![],` addition to each file's existing `ArchModel { ... }` test-literal site(s) (Task 1.1.1c) — required because `ArchModel` doesn't derive `Default`, so the new field is a breaking change to every full struct literal, not just `build_model`'s.
- `src/symbol_extract.rs` — `RawTypeRelationSite`, `type_relation_supports`, per-language extraction fns (`go_embedded_type_relations`, `ts_class_heritage_relations`, `js_class_heritage_relation`, `java_class_type_relations`, `java_interface_type_relations`, `kotlin_delegation_type_relations`), `walk_type_relations`, `extract_type_relation_sites_for_file`.
- `src/mcp.rs` — `TypeHierarchyDirection`, `build_type_adjacency`/`expand_type_frontier`/`traverse_type_edges`, `MAX_TYPE_HIERARCHY_DEPTH`, `TypeHierarchyRequest`/`TypeHierarchyResponse`, a shared `paginate<T>` helper (extracted from `list_architecture_symbols`, Task 2.1.2a), a `node_kind`/`hint` helper (Task 2.1.2b), `type_hierarchy` (Task 2.1.2c), `list_supertypes`/`list_subtypes`.
- `src/arch_export.rs` — benchmark fixture extended with hierarchy-bearing source.
- `docs/backtest-repos.md`, `scripts/clone-backtest-repos.sh` — Kotlin corpus entry.
