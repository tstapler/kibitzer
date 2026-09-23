# Research: Features — type-hierarchy-graph

Agent 2 (Features), sdd:2-research. Question: what do comparable tools' type-hierarchy
models look like, and what edge cases / unstated needs should this design handle?

## 1. Comparable tools' relationship shape

**LSP `textDocument/typeHierarchy`** (spec 3.17+, [Microsoft's LSP spec](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#typeHierarchy_supertypes)).
Three-request protocol: `textDocument/prepareTypeHierarchy` resolves the symbol at a
position to a `TypeHierarchyItem`; `typeHierarchy/supertypes` and `typeHierarchy/subtypes`
each take one `TypeHierarchyItem` and return the *direct* one-hop set of
`TypeHierarchyItem`s in that direction — a client that wants the full ancestor chain
issues repeated `supertypes` calls itself, one hop at a time, walking the tree client-side.
The protocol carries no `extends`-vs-`implements` distinction in the edge itself; both are
folded into the same "is a supertype of" relation, differentiated only implicitly by the
item's own `kind` (`SymbolKind.Class` vs `SymbolKind.Interface`) at each node. Directly
relevant: this is a **one-hop-per-call, client-drives-the-walk** design, matching
`list_callers`/`list_callees`'s BFS-with-depth shape more than a single "give me
everything" call.

**Universal-ctags `inherits` extension field** ([man page](https://docs.ctags.io/en/latest/man/ctags.1.html), `--fields=+i`).
Attaches a comma-separated list of ancestor-class names directly onto the *subtype's* tag
entry — `ClassA` inheriting from `ClassB, ClassC` shows up on `ClassA`'s tag line as
`inherits:ClassB,ClassC`. This is the opposite storage shape from a proposed
`TypeRelationEdge` list: ctags denormalizes the relationship onto the child node as a
name list (unresolved — plain strings, no id/reference), rather than a first-class edge
collection. It also doesn't distinguish `extends` from `implements` — a language with both
(Java) just lists every supertype name together in `inherits`.

**Sourcegraph SCIP `Relationship`** ([scip.proto](https://github.com/sourcegraph/scip/blob/main/scip.proto), message `Relationship`).
Attached to `SymbolInformation.relationships` (a list on the *definition* side, similar
placement to ctags' `inherits` but referencing symbol ids, not raw names). Four boolean
flags per relationship rather than a `kind` enum: `is_reference` (should "Find references"
on the target also surface this symbol), `is_implementation` (should "Find
implementations" on the target surface this symbol), `is_type_definition` ("Go to type
definition"), `is_definition` (this symbol's own definition should resolve through the
target — for mixins/inherited-field cases). SCIP's own worked example in the proto comment
is exactly the extends-vs-implements case kibitzer needs:
```
interface Animal { sound(): string }
class Dog implements Animal {
  // Dog#'s relationships = [{symbol: "Animal#", is_implementation: true}]
  public sound(): string { ... }
  // Dog#sound()'s relationships = [{symbol: "Animal#sound()", is_implementation: true, is_reference: true}]
}
```
Note SCIP emits a relationship not just on the type itself but on each *overriding
method*, propagated down from the type relationship — a granularity kibitzer's
`TypeRelationEdge` (type-to-type only, no method-level propagation) deliberately doesn't
need, since `list_supertypes`/`list_subtypes` operate on `SymbolNode` ids and a consumer
checker can already join back to `SymbolNode.parent` for method-level questions.

**IntelliJ / VS Code "Type Hierarchy" views.** Both are thin UI over the LSP-shaped
protocol (VS Code's is literally backed by `typeHierarchy/supertypes|subtypes` when the
language server supports it; IntelliJ's native Java/Kotlin indexer is older but exposes
the same three-pane Supertypes/Subtypes/Both view). Neither UI visually separates "class
extends" edges from "interface implements" edges in the tree — both render as parent
nodes, distinguished only by the node's own class-vs-interface icon. This is a UX
convention, not a data-model constraint: the underlying edges *are* typed in every
protocol above (SCIP's `is_implementation` flag, LSP's implicit `SymbolKind` per node);
the flattened tree view is a display choice. Doesn't change kibitzer's design (the
Alternatives Considered section already rejects an untyped single edge kind for the
#60/#61 reason), just confirms that decision isn't out of step with prior art — every
protocol *stores* the distinction even where the default UI collapses it.

## 2. kibitzer's existing edge/resolution conventions (what `TypeRelationEdge` should match)

Read `src/arch_model.rs` at commit
[`16008c4`](https://github.com/tstapler/kibitzer/blob/16008c46f24ef7488ec81254f451f0092b96696f/src/arch_model.rs).

- **`CallEdge`** ([L140-146](https://github.com/tstapler/kibitzer/blob/16008c46f24ef7488ec81254f451f0092b96696f/src/arch_model.rs#L140-L146)): `{from, to, resolved: bool, file, line}`. `to` is a
  `SymbolNode::id` when `resolved`, else the raw unresolved callee text — "never silently
  dropped, so a consumer can still see what the call site named."
- **`FieldAccessEdge`** ([L172-178](https://github.com/tstapler/kibitzer/blob/16008c46f24ef7488ec81254f451f0092b96696f/src/arch_model.rs#L172-L178)): `{from, to, access: AccessKind, file, line}` — no
  `resolved` flag; a site that doesn't match a declared field is **dropped**, not kept as
  unresolved-with-raw-text. The doc comment on `FieldAccessEdge` (L167-170) explicitly
  flags this as the point of difference from `CallEdge`'s convention.
- **Resolution pattern**: both edge types are built in two phases — a per-file walk
  collects raw, unresolved sites (`RawCallSite`, `RawFieldAccessSite` from
  `symbol_extract.rs`) into flat `Vec`s, then a single post-walk pass
  (`resolve_call_edges`/`resolve_field_access_edges`, L549-587) builds a whole-repo name
  index once (`build_call_target_indexes`) and resolves every raw site against it. This is
  the "same tree-sitter walk pass ... not a second full-repo pass" the requirements.md
  non-functional section already calls for — `type_edges` extraction should follow the
  identical two-phase shape: `RawTypeRelationSite` collected during the existing walk,
  resolved against a name→id index built once after.
- **id scheme**: `SymbolNode::id` is `"{package_path}::{parent}.{name}"` for methods, else
  `"{package_path}::{name}"` — deterministic, re-derivable, no lookup table needed
  ([L47-50](https://github.com/tstapler/kibitzer/blob/16008c46f24ef7488ec81254f451f0092b96696f/src/arch_model.rs#L47-L50)). A resolved `TypeRelationEdge.to` should reuse this exact scheme
  (package-qualified `Type`/`Interface` id), not invent a parallel id format the way
  `FieldAccessEdge.to` had to (fields aren't `SymbolNode`s, so it synthesizes
  `"{package}::{type}.{field}"` — `TypeRelationEdge` doesn't have this problem since both
  endpoints *are* `SymbolNode`s already).

**MCP traversal convention** — read `src/mcp.rs` at commit
[`fb1f076`](https://github.com/tstapler/kibitzer/blob/fb1f0762bd595a6322e14c6c1aafc17ecb8b5b93/src/mcp.rs).
`list_callers`/`list_callees` are **not** strictly one-hop as the requirements.md's Open
Questions section assumes ("Phase 3 should default to direct-only ... matching
`list_callers`/`list_callees`'s existing one-hop convention"). The actual implementation
(`CallTraversalRequest`, [L169-180](https://github.com/tstapler/kibitzer/blob/fb1f0762bd595a6322e14c6c1aafc17ecb8b5b93/src/mcp.rs#L169-L180)) takes a `depth` parameter defaulting to
`1` but **clamped to `[1, MAX_CALL_DEPTH=10]`**, BFS-walked via `traverse_call_edges` with
a visited-node set for cycle safety and a `truncated: bool` response flag signaling
whether more hops remain beyond the requested depth
([L228-236](https://github.com/tstapler/kibitzer/blob/fb1f0762bd595a6322e14c6c1aafc17ecb8b5b93/src/mcp.rs#L228-L236)). Default behavior is one-hop, but the tool itself already supports
transitive multi-hop traversal on request. This directly resolves requirements.md's open
question about transitive vs. direct: `list_supertypes`/`list_subtypes` should copy this
exact shape — a `depth` param defaulting to 1, same clamp constant reused or mirrored, same
BFS-with-visited-set-and-`truncated`-flag response shape — not a bare one-hop-only tool.
This is also the "concrete need for transitive walking" the requirements.md said Phase 3
should look for before deviating from a direct-only default (see §4 below for why #59/#61
specifically need it).

## 3. Edge cases

| Case | Finding |
|---|---|
| **Go multi-level struct embedding** | `go_struct_fields` in `src/symbol_extract.rs` ([L660-705](https://github.com/tstapler/kibitzer/blob/099d9d70f11a674a7d98053435e1deb953cff68a/src/symbol_extract.rs#L660-L705)) is the exact extraction point to extend: it already walks each `field_declaration` in a struct's `field_declaration_list`, and its doc comment states plainly — "An embedded field (anonymous — no `name` field at all) is skipped: embedding introduces promoted fields/methods this v1 extraction doesn't walk into." That `continue`-on-no-`name` branch ([L674, L696-701 region](https://github.com/tstapler/kibitzer/blob/099d9d70f11a674a7d98053435e1deb953cff68a/src/symbol_extract.rs#L674)) is precisely where a `field_declaration` with a `type` field but no `name` field needs to become a `RawTypeRelationSite` instead of being silently dropped. Multi-level chains (A embeds B embeds C) need no special handling at extraction time — each type only records its own *direct* embedded field(s); the chain is only "multi-level" from a graph-traversal perspective (A→B and B→C are two separate direct edges), which is exactly what a `depth`-parameterized `list_supertypes` resolves by walking, not something extraction needs to chase itself. |
| **Go embeds both a struct and an interface in one type** | Confirmed as an open problem, not yet solved by anything in the codebase: Go's embedding syntax is identical for both (an anonymous field naming a type), so distinguishing `extends` (embeds a struct) from `implements` (embeds an interface) requires the same symbol-table lookup already used elsewhere (is this name's `SymbolKind` in the model `Type` or `Interface`?) — this has to happen in the post-walk resolution phase (name is known only after every file's `SymbolNode`s are collected), matching `resolve_call_edges`'s existing two-phase shape. |
| **Kotlin ambiguous supertype list** | Same shape of problem as Go, confirmed structurally: `kotlin_is_interface` ([symbol_extract.rs L408](https://github.com/tstapler/kibitzer/blob/099d9d70f11a674a7d98053435e1deb953cff68a/src/symbol_extract.rs#L408)) already does an AST-shape check (is the node's first positional child the `interface` keyword) to classify *the declaring node itself* as Type vs Interface — but that's a different question from classifying *a name appearing in another class's supertype list*, which still needs the resolved-name→`SymbolKind` lookup. No existing code answers the second question; it's new post-walk logic, same as the Go interface-vs-struct-embed case. |
| **Multiple `implements` targets (Java/TS)** | Straightforward — TS/Java grammars expose `implements` as a list (`super_interfaces`/`implements_clause` with multiple type nodes), so `RawTypeRelationSite` extraction just emits one edge per listed interface, no dedup/special-casing needed beyond what a `Vec<TypeRelationEdge>` already provides for free. |
| **Diamond inheritance / duplicate interface listing** | `class Foo implements A, A` (typo/duplicate) or a diamond where A and B both extend C and Foo implements both A and B — neither needs special handling at the edge-collection layer; a `Vec<TypeRelationEdge>` naturally allows duplicate/parallel edges, and any dedup concern is a `list_supertypes` *presentation* question (whether the BFS visited-set already collapses a diamond's repeated node into one appearance) — `traverse_call_edges`'s existing visited-set precedent already handles this for the call graph and should be reused as-is. |
| **Self-referential / cyclic types** | Impossible in valid source for `extends` (a compile error in every target language) but worth confirming defensively: `traverse_call_edges` already proved out cycle-safety via a visited-node set on the *call* graph, including the exact "recursive function calls itself" case (`traverse_call_edges_self_loop_terminates_via_visited_set_not_depth`, mcp.rs test at [L1177-1186](https://github.com/tstapler/kibitzer/blob/fb1f0762bd595a6322e14c6c1aafc17ecb8b5b93/src/mcp.rs#L1177-L1186)) — reusing that same BFS helper (or a structurally identical one) for `type_edges` inherits this safety for free, so no new design risk here, just don't hand-roll a fresh unbounded-recursion walk. |
| **Anonymous/inline embedded structs in Go** | `type Foo struct { struct { X int } }` (an anonymous embedded *struct literal*, not a named type) — the embedded field has no name **and** no resolvable type identifier (the "type" is an inline `struct_type` node, not a `type_identifier`). This is a real gap the requirements.md doesn't mention: such a site can't become a `TypeRelationEdge` at all (there's no target `SymbolNode` — an anonymous struct is never itself declared as a symbol), so it needs a defensive skip distinct from "extends a struct" vs "extends an interface." Recommend treating it as neither (drop, don't emit an edge) — matches `FieldAccessEdge`'s "drop rather than guess" precedent, and there is no natural unresolved-text fallback since there's no name to keep. |
| **Generic type parameters on a supertype** (`class Foo<T> extends Bar<T>`) | Not yet handled anywhere in the codebase for *any* purpose — `strip_generic_params` (used pervasively in `symbol_extract.rs` for symbol *names*) is the existing precedent for "the generic parameter list is noise to strip before using a name as an id/lookup key," and the same helper (or the same stripping discipline) should apply when reading a supertype clause's type name — `Bar<T>` should resolve against the `SymbolNode` for `Bar`, not fail to match because of the trailing `<T>`. |
| **External/stdlib/vendored/cross-repo `extends`/`implements` target** | This is exactly `CallEdge`'s existing `resolved: bool` scenario, and requirements.md's Open Questions section already names it as the live design decision (resolved-flag-and-keep-raw-text like `CallEdge`, vs. drop-if-unresolved like `FieldAccessEdge`). Given `TypeRelationEdge`'s two endpoints are always full `SymbolNode`-shaped entities (unlike `FieldAccessEdge`'s synthetic field-id case), the `CallEdge` precedent is the closer analog — dropping an edge to `java.lang.Comparable` or `React.Component` would silently make every class that implements a stdlib interface look hierarchy-free to #59/#60/#61, which is a worse failure mode for a *type*-oriented graph than it is for individual field accesses (where a dropped edge is one row, not "this type appears to have no supertype at all"). Recommend `resolved: bool` + raw text, matching `CallEdge`. |

## 4. Unstated needs from #59/#60/#61

Per `docs/refactoring-catalog-analysis.md` ([lines 288-296](https://github.com/tstapler/kibitzer/blob/117685e9f1c281c85a914fd0d9e58727d52d083e/docs/refactoring-catalog-analysis.md#L288-L296) — table rows for
Pull Up Method/Field/Constructor, Push Down Method/Field, Replace Type Code with
Subclasses, Remove Subclass, Extract Superclass, Collapse Hierarchy):

- **Pull Up Method/Field/Constructor Body** (#59): described as "sibling-subclass
  identical method bodies" — this needs *siblings under a common direct parent*, which is
  answerable from direct `type_edges` alone (group subtypes by their one shared supertype
  id). No transitive walk required for this specific rule.
- **Push Down Method/Field** (#59): "superclass method used by exactly one subclass" —
  also direct-edge-only: needs the full set of a supertype's *direct* subtypes to check
  "used by exactly one," which is `list_subtypes` at depth 1.
- **Collapse Hierarchy** (#61): "near-total structural overlap between a type and its
  direct super/subtype" — explicitly *direct* per the catalog doc's own wording.
- **Extract Superclass** (#61): "structural-similarity clustering across sibling types
  with no existing hierarchy edge" — this one is different: it needs to know a *set of
  types currently has no common ancestor*, which means checking the full ancestor chain
  (transitively) for absence of overlap, not just direct parents — two types with no
  *direct* shared parent could still share a distant ancestor, which should disqualify
  them from "no existing hierarchy edge." This is the strongest concrete case in the
  catalog for needing `depth > 1` traversal, i.e., exactly the `list_supertypes` walk the
  MCP tool's `depth` parameter (§2 above) is built to answer.
  Remove Subclass (#61, "leaf subtype whose only override returns a literal") also
  implicitly needs "is this a leaf" — zero direct subtypes — answerable at depth 1 but
  *is a subtype-count question*, meaning `list_subtypes` returning an accurate direct set
  (not just existence) matters, not just whether the query non-empty.
- **Replace Type Code with Subclasses / Replace Conditional with Polymorphism** (#60):
  depends on #38's OCP-proxy metric plus #40 — the catalog doc doesn't specify
  direct-vs-transitive for this one; it's about clustering type-switch branches against
  an *existing* hierarchy's direct children, so direct edges likely suffice, but this
  wasn't confirmed by anything more specific than the table's one-line description.

**Net finding**: the item's stated scope (direct edges only, `TypeRelationEdge` +
one-hop-default MCP tools) covers #59 and most of #61 fully. **Extract Superclass (#61)
is the one concrete, catalog-documented consumer need that requires transitive ancestor
traversal**, not just direct edges — which argues for `list_supertypes`/`list_subtypes`
supporting a `depth` parameter (mirroring `list_callers`/`list_callees`'s existing
`CallTraversalRequest.depth`/`MAX_CALL_DEPTH` shape, §2 above) rather than a
direct-edges-only tool that #61 would later have to route around or duplicate BFS logic
for at the checker layer. `ArchModel.type_edges` itself only needs to store *direct*
edges either way (transitivity is a traversal-time concern, same as the call graph never
stores transitive call chains) — the "transitive or not" question is entirely about the
MCP tool surface, not the model's storage shape.
