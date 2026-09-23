# Research: Stack (tree-sitter grammars, node kinds, existing patterns)

Agent 1 — sdd:2-research for `type-hierarchy-graph` (backlog `30cde5cf` / issue #40).

## Dependency versions (`Cargo.toml`, lines 24-35)

```
tree-sitter            = "0.26"
tree-sitter-go         = "0.25"   (vendored: 0.25.0)
tree-sitter-typescript = "0.23"   (vendored: 0.23.2, covers both `typescript` and `tsx` grammars)
tree-sitter-javascript = "0.23"   (vendored: 0.23.1)
tree-sitter-python     = "0.23"
tree-sitter-java       = "0.23.5"
tree-sitter-kotlin-ng  = "1.1.0"
tree-sitter-rust       = "0.24"
```

`Language::grammar()` (`src/checker.rs:55-62`) is the single place each enum variant maps to
its grammar crate (`tree_sitter_go::LANGUAGE`, `tree_sitter_typescript::LANGUAGE_TYPESCRIPT`/
`LANGUAGE_TSX`, etc.) — no separate grammar-loading path to worry about.

All five grammars needed for this feature (Go, TS, TSX shares TS's `node-types.json`, JS,
Java, Kotlin) are present in `~/.cargo/registry/src/.../*/src/node-types.json`, so every
node-kind claim below is read from the actual vendored grammar, not guessed:

- `tree-sitter-go-0.25.0/src/node-types.json`
- `tree-sitter-typescript-0.23.2/typescript/src/node-types.json`
- `tree-sitter-javascript-0.23.1/src/node-types.json`
- `tree-sitter-java-0.23.5/src/node-types.json`
- `tree-sitter-kotlin-ng-1.1.0/src/node-types.json`

## Existing pattern: `LangSymbolConfig` / `lang_symbol_config` (`src/symbol_extract.rs:42-273`)

Table-driven, one `match lang { ... }` producing a `LangSymbolConfig { type_kinds,
interface_kinds, function_kinds, name_finder, is_exported }` per `Language`, consumed by a
single generic recursive walker (`walk` → `classify_node`, `symbol_extract.rs:389-507`).
Two real deviations from a pure table, both instructive for the new `extends`/`implements`
extraction:

- **Go's `type_declaration` is special-cased in `walk`** (`symbol_extract.rs:497-498`)
  because it can wrap *multiple* `type_spec` children (a grouped `type (...)` block) that
  must be classified independently — handled by `go_type_declaration_symbols`
  (`symbol_extract.rs:355-387`), not through the generic `classify_node` path. Any Go
  type-relation extraction needs the same per-`type_spec` walk, not just a
  `type_declaration`-level check.
- **Kotlin's `class_declaration` also covers interfaces** — disambiguated by
  `kotlin_is_interface` (`symbol_extract.rs:137-139`), which checks whether the node's
  first positional child (not a field) has `kind() == "interface"`. The new Kotlin
  extraction has to reuse this same disambiguation before deciding whether a
  `class_declaration`'s supertype list produces `extends` or `implements`/`extends`
  edges for an interface-on-interface case.

`find_child_by_kind` (`symbol_extract.rs:100-103`) — "first direct positional child of a
given kind" — is the existing helper for exactly the kind of walk this feature needs
(Java `modifiers`→`public`, Kotlin `modifiers`→`visibility_modifier`); reuse it rather than
writing a new positional-child scanner.

**Important existing gap this feature must *not* inherit:** `go_struct_fields`
(`symbol_extract.rs:668-705`) explicitly **skips** anonymous (embedded) struct fields —
"An embedded field (anonymous — no `name` field at all) is skipped: embedding introduces
promoted fields/methods this v1 extraction doesn't walk into." That skip is exactly the
node shape the new Go `extends`-edge extraction must *not* skip — see below.

## Per-language node kinds for supertype/interface extraction (verified via `node-types.json`)

### Go — struct embedding (no `extends` keyword; structural)

`type_spec.type` → `struct_type` → (positional child) `field_declaration_list` →
`field_declaration` children. A `field_declaration`'s `name` field is `multiple: false,
required: false` (plural in principle, `commaSep1`, but zero names for an embedded field) —
when a `field_declaration` has **no** `name` field-child at all, its `type` field IS the
embedded type (the field is anonymous/embedded by definition). The `type` field's node kind
is one of:

- `type_identifier` — plain same-package embed (`Base`)
- `qualified_type` (fields: `package: package_identifier`, `name: type_identifier`) —
  cross-package embed (`pkg.Base`); resolve `package` through the *same*
  `file_import_aliases`/qualified-name machinery `resolve_call_edges` already uses for
  qualified calls (see below), not a new alias table.
- `pointer_type` (single positional child, no field name, wrapping one of the above) —
  `*Base` / `*pkg.Base` embed; must be unwrapped one level, same pattern
  `go_receiver_type_name` (`symbol_extract.rs:310-323`) already uses for pointer receivers.
- `generic_type` (field `type`: `type_identifier` or `qualified_type`, plus
  `type_arguments`) — `Base[T]` embed; unwrap `.type` the same way
  `rust_impl_type_name`/`strip_generic_params` already treat generic Self types elsewhere
  in this file.

This is exactly the shape `go_struct_fields` walks and currently discards — reuse its
`type_spec` → `struct_type` → `field_declaration_list` → `field_declaration` traversal, but
branch on "`name` field absent" to emit a `TypeRelationEdge` instead of skipping.

Go interfaces can also embed other interfaces (`interface_type` containing a bare
`type_elem`/qualified name rather than a `method_elem`) — out of scope per
`requirements.md` (only struct embedding is v1), but the same `type_spec.type ==
interface_type` check `go_type_declaration_symbols` already does to classify
`SymbolKind::Interface` is the natural place to add it as a fast-follow.

### TypeScript / Tsx / JavaScript — `extends`/`implements` clauses

`class_declaration` has an optional **positional** (not field) child `class_heritage`
(`typescript`/`javascript` node-types.json). The two grammars diverge here — this is a
concrete pitfall, not a copy-paste-safe assumption:

- **TS/Tsx**: `class_heritage` wraps one or more of `extends_clause` (field `value`,
  `multiple: true`, type `expression`) and `implements_clause` (positional children of
  type `type`, `multiple: true`). So `class X extends Y implements A, B` — walk
  `class_heritage`'s children, route `extends_clause` children to `extends` edges and
  `implements_clause` children to `implements` edges. The extracted supertype name is
  whatever `expression`/`type` node's text is (`identifier` for a plain name,
  `nested_type_identifier`/member-expression-ish node for a namespaced one — take
  `node_text` of the whole node and let edge resolution worry about matching, same as
  `CallEdge`'s `callee_text` handling).
- **JS**: `class_heritage` is *not* a wrapper — it directly contains a single positional
  `expression` child (no `extends_clause`/`implements_clause` substructure, since plain JS
  has no `implements`). `class X extends Y` → `class_heritage`'s one child *is* the
  superclass expression. Do not reuse the TS branch's `extends_clause`/`implements_clause`
  lookup for JS; it will find nothing.

### Java — `extends`/`implements` clauses

`class_declaration` has two genuine **fields** (unlike TS/JS, which use a positional
wrapper): `superclass` (optional field, node kind `superclass`, wrapping one `_type`) and
`interfaces` (optional field, node kind `super_interfaces`, wrapping one `type_list` whose
children are `_type`). So: `child_by_field_name("superclass")` →
`.child_by_field_name` doesn't apply further (superclass has no fields) — take its single
child; `child_by_field_name("interfaces")` → `super_interfaces`'s single `type_list` child
→ iterate `type_list`'s `_type` children.

`interface_declaration` (Java interfaces extending other interfaces, `interface A extends
B, C`) has no `interfaces`/`superclass` field at all — instead a **positional** child of
kind `extends_interfaces` (wrapping one `type_list`, same shape as `super_interfaces`).
This is the Java analogue of TS's field-vs-positional divergence: reading
`class_declaration.interfaces`/`superclass` fields for a class, but
`find_child_by_kind(node, "extends_interfaces")` for an interface — a class's `implements`
and an interface's `extends` are *not* the same field name in this grammar.

### Kotlin — supertype list (`class X : Y(), A, B`)

`class_declaration` has a positional (not field) child `delegation_specifiers`
(`kind() == "delegation_specifiers"`, only present when a `:` supertype list exists), whose
children are `delegation_specifier` nodes. Each `delegation_specifier`'s single child
disambiguates superclass vs. interface **structurally**, not just "ambiguous" as
`requirements.md` phrases it:

- `constructor_invocation` (children: `type`, `value_arguments`) — the `Y()` call-syntax
  entry. Kotlin allows at most one of these per class (only one class can be a direct
  superclass) → this is the `extends` edge; the `type` child's text is the supertype name.
- a bare `type` (usually wrapping `user_type`, which itself has an `identifier` child, e.g.
  `A`) with **no** `constructor_invocation`/`explicit_delegation` wrapper → an interface
  reference → `implements` edge.
- `explicit_delegation` (children: `primary_expression`, `type`; the `by fooImpl` delegate
  syntax) — also an interface reference (delegated implementation) → `implements` edge, but
  note the supertype name is in its `type` child, not the whole node's text.

So the "ambiguous" case `requirements.md` flags is resolved by branching on which node kind
a `delegation_specifier` wraps (`constructor_invocation` vs. `type`/`explicit_delegation`),
not by any heuristic on the type name itself.

## Existing cross-package name-resolution pattern to reuse (`src/arch_model.rs`)

`resolve_call_edges`/`resolve_one_call_edge` (`arch_model.rs:502-558`) is the direct
precedent for a new `type_edges` resolver, and the shape is a strong fit:

1. **Raw, package-scoped extraction** produces an unresolved struct during the per-file
   walk (`RawCallSite { caller_id, callee_text, file, line }`,
   `symbol_extract.rs:544-552`) — no cross-file/cross-package lookup happens yet. A new
   `RawTypeRelationSite { type_id: String, supertype_text: String, edge_kind: extends|implements,
   file, line }` mirrors this exactly (`type_id` = the declaring type's already-built
   `SymbolNode::id`, same as `caller_id`).
2. **A single whole-repo index is built once** after every package is known:
   `build_call_target_indexes` (`arch_model.rs:479-500`) builds `SymbolIndex<'a> =
   HashMap<&'a str, Vec<(&'a str, &'a str)>>` (`name -> [(package, id)]`), split by kind
   (`Function`/`Method`). The type-relation equivalent is a single `Type`/`Interface`
   index built from `packages` the same way — `SymbolKind::Type | SymbolKind::Interface`
   symbols only, keyed by name.
3. **Resolution prefers an unambiguous same-package match, else a globally unique one,
   else gives up** — `resolve_in` (`arch_model.rs:455-472`) is generic over the index type
   already (`&HashMap<&str, Vec<(&str, &str)>>`, `caller_pkg`, `name`) and can likely be
   called *as-is* for supertype-name resolution, no rewrite needed. `caller_package`
   (`arch_model.rs:446-448`) — split `id` on `"::"` — also applies unchanged to a
   `type_id`, since `SymbolNode::id` has the identical `"{package}::{name}"` shape for
   `Type`/`Interface` symbols (`symbol_extract.rs::build_id`).
4. **Policy choice for an unresolved edge is a real fork in the existing code, not an
   accident** — `CallEdge` keeps an unresolved edge with `resolved: false` and the raw
   callee text in `to` (`resolve_one_call_edge`, `arch_model.rs:528-543`); `FieldAccessEdge`
   instead silently drops a site that doesn't resolve (`resolve_field_access_edges`,
   `arch_model.rs:565-587`, no `resolved` field on `FieldAccessEdge` at all). Per
   `requirements.md`'s scope (`extends`/`implements` edges as first-class query targets for
   `list_supertypes`/`list_subtypes`), `TypeRelationEdge` should almost certainly follow
   `CallEdge`'s convention (keep + `resolved: bool`) rather than `FieldAccessEdge`'s
   drop-on-miss: an unresolved supertype (external library base class, e.g. extending a
   third-party `Exception`) is exactly the kind of node a Pull-Up/Push-Down refactoring
   check (issue #59, the downstream consumer named in `requirements.md`) still needs to see
   named, even if it can't be resolved to a local `SymbolNode::id`.
5. **Go qualified-name resolution already has a purpose-built alias table** —
   `ArchModel.file_import_aliases: BTreeMap<PathBuf, HashMap<String, String>>`
   (`arch_model.rs:191-200`, built by `resolve_file_import_aliases`,
   `arch_model.rs:599-621`) maps a per-file import alias to a real `packages` key
   specifically so a `pkg.Type`-qualified reference can be resolved past the qualifier.
   This is the correct existing mechanism for Go's `qualified_type` embedded-field case
   (`pkg.Base`) — look the file's alias map up before falling back to name-only
   `resolve_in`, rather than inventing a second alias table.

## MCP query-tool pattern to follow for `list_supertypes`/`list_subtypes`

`list_architecture_symbols` (`src/mcp.rs:809-911`) is the pagination/response-shape
precedent `requirements.md` explicitly calls out ("paginated, per `list_architecture_symbols`'s
existing convention"): `Parameters<...Request>` struct in, `limit.clamp(1, 1000)`,
`cursor: Option<String>` parsed as a plain `usize` offset (non-numeric cursor → `json_error`,
not a silent reset), `next_cursor` computed as `Some(next_offset)` iff more remain else
`None`, and a `{total_matched, returned, next_cursor, ..., <items>}` JSON envelope
(`ListArchitectureSymbolsResponse`) — no prose fallback. Both `resolve_repo_root`
(`mcp.rs:785-791`) and `load_model_off_stack` (`mcp.rs:797-807`, `spawn_blocking` because
`ArchModel` building is synchronous disk/tree-sitter work) are already factored out
specifically so new tools can call them directly instead of re-inlining `find_config`/cache
dispatch — `get_architecture_node` (`mcp.rs:913+`) is the other existing caller of both,
useful as the second precedent for "resolve one `node` id, then look it up," which
`list_supertypes(node)`/`list_subtypes(node)` will need for their required input parameter
before the pagination logic even starts.

## Summary of concrete gotchas for the implementation plan

- Go: don't reuse `go_struct_fields` as-is — reuse its traversal shape but invert its
  "anonymous field → skip" branch into the extraction target.
- TS/Tsx vs. JS: `class_heritage`'s internal structure is different (wrapped
  `extends_clause`/`implements_clause` vs. a single bare `expression` child) — two branches,
  not one shared one, despite JS and TS sharing everything else in `LangSymbolConfig`.
- Java: `class_declaration`'s `superclass`/`interfaces` are named fields; `interface_declaration`'s
  `extends_interfaces` is a positional child with no field name — same
  field-vs-positional-child split the codebase already handles elsewhere via
  `find_child_by_kind`.
- Kotlin: the superclass/interface ambiguity is resolved by node kind
  (`constructor_invocation` vs. `type`/`explicit_delegation`) inside each
  `delegation_specifier`, not by any name heuristic.
- Resolution: reuse `resolve_in`/`caller_package`/`SymbolIndex` verbatim if possible; follow
  `CallEdge`'s keep-unresolved convention, not `FieldAccessEdge`'s drop-on-miss; route Go's
  qualified embeds through the existing `file_import_aliases` map rather than a new one.
