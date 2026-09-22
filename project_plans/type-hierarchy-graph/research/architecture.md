# Research: Architecture — type-hierarchy-graph

Builds on `project_plans/architecture-export/research/architecture.md` (the
`arch_model.rs` module-boundary and "one model, many views" research for the project that
created this file). That doc's `ArchModel` shape proposal (§5) already matured into the
real `packages: BTreeMap<String, PackageNode>` / edge-vec design this feature extends —
cited by file:line below rather than re-derived.

## 1. Insertion point in `build_model`: raw-site-then-resolve, not inline

`build_model` (`src/arch_model.rs:317-441`) runs two extraction shapes side by side in its
per-file loop, and this feature should follow the second, not the first:

- **Inline, single-pass** (`SymbolNode` extraction, `src/arch_model.rs:382-393`): calls
  `extract_symbols_for_file` and pushes directly into `package.symbols` — safe only because a
  `SymbolNode` needs no cross-file knowledge to construct (its `id` is derived purely from its
  own package path + name).
- **Raw-site-then-resolve** (`CallEdge` via `resolve_call_edges`, `FieldAccessEdge` via
  `resolve_field_access_edges`): during the same per-file loop, `collect_call_sites`
  (`src/arch_model.rs:271-285`) and `collect_field_access_sites` (`src/arch_model.rs:290-304`)
  each append a *raw*, unresolved record (`RawCallSite`/`RawFieldAccessSite`,
  `src/symbol_extract.rs:544-549` and `:747-755`) to a `Vec` — no lookup happens yet. Only
  after the loop finishes and `packages` is fully populated does `resolve_call_edges`
  (`src/arch_model.rs:549-558`) build a whole-repo `SymbolNode.name -> [(package, id)]` index
  (`build_call_target_indexes`, `:479-500`) and resolve every raw site against it in one pass
  (`build_model`'s call site: `src/arch_model.rs:418-419`).

**`type_edges` must follow the raw-site-then-resolve pattern**, for the same reason
`CallEdge` does: resolving `extends X` to a `SymbolNode::id` needs to know whether `X` is a
`Type` or `Interface`, and that fact may live in a file the per-file walk hasn't reached yet
(see §2). Concretely, this means:

- A new `RawTypeRelationSite` in `src/symbol_extract.rs`, shaped like `RawCallSite`: an
  owner id (the `Type`/`Interface`'s own `SymbolNode::id`, since the edge's `from` is a type,
  not a call site inside a function body — closer to `RawFieldAccessSite`'s
  `receiver_type`/`package_path` pair than to `RawCallSite`'s `caller_id`), a raw target-name
  string, and a `kind` hint where the source syntax already disambiguates it (TS/Java: the
  `extends`/`implements` keyword *is* the kind; Go/Kotlin: no keyword-level signal, kind is
  decided at resolve time — see §2).
- A new `extract_type_relation_sites_for_file` in `src/symbol_extract.rs`, following
  `extract_call_sites_for_file`'s shape (`src/symbol_extract.rs:621-640`): per-language walk
  producing raw sites, called from a new `collect_type_relation_sites` wrapper in
  `src/arch_model.rs` (mirroring `collect_call_sites`) inside `build_model`'s existing loop.
- A new `resolve_type_edges` in `src/arch_model.rs`, called after the loop alongside
  `resolve_call_edges`/`resolve_field_access_edges` (`src/arch_model.rs:418-419`), using a
  `SymbolNode.name -> [(package, id, kind)]` index (a `SymbolKind`-carrying variant of
  `build_call_target_indexes`'s index, since — unlike the call graph — the resolver itself
  needs `kind` to decide `Extends` vs `Implements` for Go/Kotlin, not just to pick a match).

This also satisfies the non-functional requirement (requirements.md:72-75: "same tree-sitter
walk pass... not a second full-repo pass") — the raw-site collection happens inline during
the existing walk, exactly like the call/field-access sites; only *resolution* is deferred,
which is a cheap in-memory index build, not a second parse pass.

## 2. Data flow: per-file walk cannot classify Type-vs-Interface targets alone

No, a per-file walk extracting `extends X` cannot know whether `X` is a `Type` or
`Interface` at walk time in the general case — this requires the second, after-the-walk pass,
for two independent reasons:

1. **Forward reference across files.** `files` is walked in whatever order the caller's file
   list provides (`src/arch_model.rs:342`, no sorting or dependency ordering) — `X`'s own
   declaration, which carries its `SymbolKind`, may not have been visited yet when the file
   declaring `extends X` is processed. This is the exact problem `resolve_call_edges` already
   solves for callee names (a call to a function declared later in the file list) — reuse the
   same "collect now, look up in the completed index later" shape, not a per-file forward
   lookahead.
2. **Syntactically identical embed for `Type` and `Interface` targets (Go, Kotlin).** This is
   requirements.md's flagged rabbit hole (requirements.md:116-120, :125-128), and it's real,
   confirmed by reading the actual grammars, not just the requirements doc's assertion:
   - **Go** (VERIFIED via `tree-sitter-go` 0.25.0's `src/node-types.json`, the same
     verification discipline `symbol_extract.rs`'s own header comment (`src/symbol_extract.rs:8-9`)
     requires — "verified against real `to_sexp()`/grammar output, not guessed"):
     `field_declaration`'s `type` field is required, but its `name` field is optional
     (`"required": false`, `"multiple": true`). An embedded field — struct or interface — has
     no `name` child at all; its `type` field's text (a `type_identifier` for a same-package
     embed, `qualified_type` for `pkg.Base`) *is* the embedded name. `go_struct_fields`
     (`src/symbol_extract.rs:668-705`) already walks exactly this shape and already skips the
     no-`name` case on purpose (`src/symbol_extract.rs:666`, doc comment: "An embedded field
     (anonymous — no `name` field at all) is skipped") — that skip is precisely where
     type-edge extraction needs to hook in, reading `type` instead of ignoring the node. But
     the AST alone cannot say whether that embedded name refers to a struct (`extends`) or an
     interface (`implements`) — Go's grammar has one shape for both.
   - **Kotlin**: confirmed by `lang_symbol_config`'s own Kotlin comments (`src/symbol_extract.rs:246-257`,
     `kotlin_is_interface`, `:137-139`) — Kotlin's supertype list (`class Foo : Base(), Iface1,
     Iface2`) is one syntactic list with no per-entry keyword distinguishing "the one
     superclass" from "the interfaces," matching requirements.md:125-128.
   - **TS/JS/Java are the easy case**: `extends`/`implements` are distinct keywords/fields in
     these grammars (per requirements.md:27, :88-89), so `kind` is fully known at walk time
     for those three languages — no resolve-time ambiguity, `RawTypeRelationSite.kind` can be
     set directly from the keyword.

**Conclusion**: `resolve_type_edges` needs the completed `SymbolKind` index for Go and
Kotlin's ambiguous cases (look up the target name, read its `SymbolKind`, and classify
`Extends` if `Type`/`Implements` if `Interface`), while TS/Java/JS sites already carry a
known `kind` from extraction and only need name-to-id resolution, not kind inference. Both
cases still route through the same after-the-walk `resolve_type_edges` pass — for
TS/Java/JS, this is simple `to.id` lookup (matching `CallEdge`'s own resolved/unresolved
split, `src/arch_model.rs:136-146`); for Go/Kotlin, it's lookup + kind classification. An
edge whose target name isn't found in the whole-repo index at all (e.g. extending a stdlib
or vendored type) should follow `CallEdge`'s convention — kept `resolved: false` with the
raw text — per requirements.md's still-open question (requirements.md:160-163), which this
research recommends resolving in favor of `CallEdge`'s convention over `FieldAccessEdge`'s
drop-if-unresolved one: a supertype edge to an unresolvable external base is exactly the
"vendored/stdlib type" case requirements.md itself names as expected and non-erroneous
(requirements.md:77-79), and #59/#60/#61 (the checkers this unblocks) plausibly still want
to see "this type extends *something*, just not one we can resolve" rather than silently
losing the edge — the same reasoning that already justified `CallEdge`'s `resolved: bool`
over silently dropping unresolved calls.

## 3. MCP integration points

**`list_callers`/`list_callees`** (`src/mcp.rs:970-988`) are the nearest precedent for a
one-hop graph-traversal tool, but their machinery is concretely tied to `CallEdge`/
`CallDirection`, not generic:

- `CallTraversalRequest` (`src/mcp.rs:168-180`): `path`, `node` (a `SymbolNode::id`), `depth`
  (default 1, clamped to `MAX_CALL_DEPTH = 10`, `src/mcp.rs:188`).
- `CallTraversalResponse` (`src/mcp.rs:227-236`): `node`, `depth`, `truncated: bool`,
  `edges: Vec<CallEdge>`.
- Both tools share one body, `call_traversal` (`src/mcp.rs:1060-1086`): resolve repo root,
  clamp depth, load the model (always `include_private: false`, sharing `get_architecture_node`'s
  cache slot — `src/mcp.rs:1068-1070`), call `traverse_call_edges` (`src/mcp.rs:314-341`, a
  BFS keyed by `CallDirection` — `Callees` follows `from -> to`, `Callers` follows `to -> from`,
  `src/mcp.rs:256-264`), and serialize.
- `traverse_call_edges` and its helpers (`build_call_adjacency`, `expand_call_frontier`,
  `src/mcp.rs:272-307`) are hard-typed to `&[CallEdge]` — not a reusable generic. A
  `list_supertypes`/`list_subtypes` pair cannot just call into this machinery; it needs its
  own, `TypeRelationEdge`-typed sibling.

Per requirements.md's own resolution of its open question (requirements.md:166-170:
default to direct-only, one-hop, matching `list_callers`/`list_callees`'s convention, unless
research finds a concrete need for transitive walking — **this research found none**: neither
#59/#60/#61's stated needs nor the MCP tool naming ("list_supertypes"/"list_subtypes", not
"walk_hierarchy") imply a multi-hop walk), `list_supertypes`/`list_subtypes` don't need the
full BFS-with-`depth`-and-`truncated` machinery at all — a direct filter over
`model.type_edges` (`edge.to == node` for supertypes, `edge.from == node` for subtypes) is
sufficient and simpler than porting `traverse_call_edges`. Recommend a small,
`TypeRelationEdge`-specific pair of helpers *not* generalized from `traverse_call_edges` (no
depth parameter, no visited-set, no BFS) — over-fitting to the `CallEdge` machinery's shape
would add unused `depth`/`truncated` fields to the response for a tool that will only ever
return one hop.

**`list_architecture_symbols`** (`src/mcp.rs:816-911`) is the pagination convention to
match, per requirements.md:80-82 and :166-167 ("must follow the same cursor/limit/
`next_cursor` convention"):
- Request: `limit` (default 200, clamped `[1, 1000]`, `src/mcp.rs:99,111-113`), `cursor`
  (`Option<String>`, `None` = page 1, a non-numeric cursor is a hard error, not a silent
  reset — `src/mcp.rs:101-104,836-842`).
- Response: `total_matched`, `returned`, `next_cursor: Option<String>`, plus a
  `possibly_pruned: bool` flag distinguishing "nothing here" from "hidden by the
  exported-only default" (`src/mcp.rs:122-131,883-895`) — `list_supertypes`/`list_subtypes`
  should carry the same `possibly_pruned` field, since an unexported supertype dropped by
  `include_private: false` is exactly the same "hidden, not absent" case
  `ListArchitectureSymbolsResponse` already handles.
- Cursor mechanics: an in-memory offset encoded as a string (`src/mcp.rs:836-842,872-881`),
  not an opaque token requiring server-side state — `list_supertypes`/`list_subtypes` should
  reuse this exact offset-as-string-cursor shape (trivial here, since direct-only edges from
  one `node` are typically few, but consistency with the rest of the tool family matters more
  than the pagination actually mattering at this edge count).

Recommended shapes, combining both precedents:
```rust
struct TypeHierarchyRequest {
    path: String,
    node: String,          // SymbolNode::id, like CallTraversalRequest::node
    include_private: bool, // like ListArchitectureSymbolsRequest, NOT in CallTraversalRequest
    limit: usize,           // default 200, clamp [1, 1000] — list_architecture_symbols convention
    cursor: Option<String>,
}
struct TypeHierarchyResponse {
    node: String,
    total_matched: usize,
    returned: usize,
    next_cursor: Option<String>,
    possibly_pruned: bool,
    edges: Vec<TypeRelationEdge>,
}
```
`list_supertypes`/`list_subtypes` should be two thin wrappers sharing one body (a
`type_hierarchy` fn taking a direction enum), exactly mirroring `list_callers`/`list_callees`
→ `call_traversal`'s split (`src/mcp.rs:979-981,991-993,1060`).

## 4. Hotspot/complexity status of the touched files

No `*hotspot*` or `*architecture-review*` doc exists for this specific gap — confirmed by
`find . -iname '*hotspot*' -o -iname '*architecture-review*'` (excluding `.git`/`target`),
which returns only `src/hotspots.rs` (an unrelated checker) and three *other* projects'
`implementation/architecture-review.md` files (`kibitzer`, `architecture-export`,
`checker-plugin-system`), none of which target this feature.

However, one of those — `architecture-export`'s own review, which *built* `arch_model.rs` —
already flagged the exact file this feature extends, as a nitpick rather than a blocker:

> "Epic 1.4's file-placement note still defers the `arch_model.rs` vs. new `arch_cache.rs`
> module-boundary decision to line count at implementation time rather than responsibility
> (`plan.md:430`) — `arch_model.rs` already carries four distinct responsibilities (domain
> types, build orchestration, query API, cache) per the Summary-of-files table."
> — `project_plans/architecture-export/implementation/architecture-review.md:97-101`

That deferred decision was never revisited: `arch_model.rs` is now 1969 lines. **Running
kibitzer's own default checks against the three touched files confirms this concretely
(VERIFIED — `mcp__kibitzer__run_checks`, run this session):**

| File | Lines | `rust-file-size` (>500 flags) | Notable `long-function` hits |
|---|---|---|---|
| `src/arch_model.rs` | 1969 | flagged | `build_model` itself: 120 lines (>40) |
| `src/symbol_extract.rs` | 1691 | flagged | `walk`-family fns: 93 lines (>40) |
| `src/mcp.rs` | 2620 | flagged | 12 functions over 40 lines, one 182-line fn at 5-deep nesting |

All three files this feature touches are already past kibitzer's own default file-size
threshold and already carry multiple long-function findings unrelated to this feature (note:
`mcp.rs`'s and `symbol_extract.rs`'s line counts include co-located `#[cfg(test)]` modules,
which inflate the raw count but not the `long-function` findings, which only fire on
non-test code).

**One-line assessment: Extend as-is, but as new functions following the existing raw-site/
resolve pattern — not by growing `build_model`'s body itself.** `build_model` is already
flagged as a 120-line long-function; adding `type_edges`'s collection/resolution *inline*
into that function's body (rather than as new sibling functions called from it, the same way
`resolve_call_edges`/`resolve_field_access_edges` are separate functions invoked from a
two-line call site at `src/arch_model.rs:418-419`) would make an already-over-threshold
function worse. The *file's* 1969-line total is a pre-existing, already-flagged condition
this feature's ~150-250 line addition (new struct + extraction fn + resolve fn, per the
`FieldAccessEdge` precedent's actual size) will not meaningfully worsen if kept to the same
new-function-per-concern shape the file's two existing edge types already establish — this is
a clean addition to an already-large file, not a file that needs splitting *before* this
feature can land. The `arch_cache.rs` split `architecture-export`'s review flagged remains a
separate, pre-existing disposition call this feature doesn't need to resolve to proceed.

## 5. Cross-package name resolution reuse (feasibility risk from requirements.md:146-149)

Confirmed reusable, with caveats: `resolve_call_edges`'s core shape — build a whole-repo
`name -> [(package, id)]` index once (`build_call_target_indexes`, `src/arch_model.rs:479-500`),
then resolve each raw site by preferring an unambiguous same-package match, falling back to a
globally-unique match (`resolve_in`, `src/arch_model.rs:455-472`) — is directly reusable *as a
pattern*, not as a literal shared function, because:
- The existing index is split into `functions_by_name`/`methods_by_name`
  (`SymbolKind`-partitioned) because call resolution needs to know *which* bucket to search
  first based on whether the call syntax was qualified (`src/arch_model.rs:517-521`).
  Type-edge resolution instead needs a single `Type`+`Interface`-only index that *returns*
  the `SymbolKind` alongside the id (so Go/Kotlin's ambiguous case, §2, can classify by it) —
  a different index shape, same `resolve_in`-style same-package-preferred lookup logic.
- Go's `file_import_aliases` map (`src/arch_model.rs:191-200`, resolved by
  `resolve_file_import_aliases`) already exists specifically to let a `pkg.Type`-qualified
  local resolve past a package-qualified name to a real `packages` key — built for
  `god_class`/`isp_fat_interface`'s consumer detection, but directly applicable to a Go
  embedded field written as `pkg.Base` (a `qualified_type` per §2's grammar confirmation).
  Type-edge resolution for Go should reuse this existing alias map rather than re-deriving
  cross-package alias resolution, closing requirements.md's risk about this machinery not
  being "cleanly reusable" — it is, via `file_import_aliases`, which already solves exactly
  this sub-problem for a different edge type.

No refactor of the shared machinery is needed; `resolve_type_edges` is a new function
alongside `resolve_call_edges`/`resolve_field_access_edges`, reusing `file_import_aliases`
and following `resolve_in`'s lookup-preference pattern with a new `Type`/`Interface`-scoped,
`SymbolKind`-carrying index.
