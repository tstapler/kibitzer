# Stack research: typed-node-kind-migration

## 1. Generated `<Lang>Kind` API (`src/node_kind.rs`, `build.rs`)

`build.rs` (`GRAMMARS` const, lines 25-42) reads each grammar's vendored
`codegen/node-types/<lang>.json` and, per grammar, walks every entry that is
`"named": true` and *not* a `subtypes` grouping (an abstract supertype like Go's
`_expression` — `Node::kind()` never returns a supertype's own name, only a
concrete subtype, so supertypes are excluded from variants: `build.rs:82-83`).
Each surviving `(PascalCase variant, raw kind string)` pair becomes an enum
variant; output is written to `$OUT_DIR/<module>_kind.rs` and `include!`'d by
`src/node_kind.rs`.

Generated API per enum (identical shape for all 8):
- `<Lang>Kind::of(node: tree_sitter::Node) -> Self` — typed kind of a live node.
- `<Lang>Kind::from_kind_str(s: &str) -> Self` — same conversion from a raw `&str`
  (falls back to `Other` for any unrecognized string).
- `.as_str(self) -> &'static str` — round-trips back to the raw string (for error
  messages, etc.); `Other.as_str()` returns `""`.
- `Other` — catch-all variant: covers synthetic `ERROR`/`MISSING` nodes and any
  anonymous/punctuation token, i.e. anything `Node::kind()` can return that isn't a
  named concrete kind in that grammar's `node-types.json`.
- Enum is `#[non_exhaustive]`, `#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]`.
- One Rust-keyword collision handled explicitly: Rust grammar's `self` kind ->
  `SelfValue` variant, not `Self` (raw-identifier escaping doesn't work here since
  `Self` is a keyword even in type position) — `build.rs:169-174`.

8 enums exist, one per grammar in `checker::Language`: `GoKind`, `TypeScriptKind`,
`TsxKind`, `JavaScriptKind`, `PythonKind`, `JavaKind`, `KotlinKind`, `RustKind`.
TypeScript and Tsx are separate grammars/enums (matches how `checker::Language`
already treats them as distinct variants).

Wiring: `src/node_kind.rs` currently has `#![allow(dead_code, unused_imports)]` at
the top (line 13) with a comment noting "not every generated enum/variant has a
caller outside this module's own tests yet" — **this lint allow needs to be
narrowed or removed as part of the migration**, once call sites exist, so a truly
unused variant/enum post-migration is caught rather than permanently silenced.
`Cargo.toml` needs no new entries for this — `serde_json` (already a dependency,
used by `build.rs` to parse the vendored JSON) is the only extra `[build-dependencies]`
requirement, and it's already present (build.rs runs fine today, confirmed by
`node_kind.rs`'s own passing unit tests referenced below).

## 2. Prior design notes (`docs/typed-node-kinds.md`)

- Confirms this is landed infrastructure only; **zero files migrated yet** ("Nothing
  yet — this is infrastructure only").
- Names `rules.rs`'s existing complementary mechanism:
  `node_kind_literals_are_valid_for_their_grammar`, a **test-time** (not compile-time)
  check validating `LangRuleConfig`'s kind-string literals against
  `tree_sitter::Language::id_for_node_kind`. Doc explicitly says this runtime check
  becomes redundant for whichever `rules.rs` literals move to the typed enums —
  worth flagging in the plan as removable dead weight once `rules.rs` is done, not
  something to keep in parallel forever.
- Doc's own file list is 21 files (one fewer than requirements.md's ~22 —
  requirements.md adds `go_bulk_fetch_linear_scan.rs` and `go_table_driven_test.rs`
  as newer/never-risk-assessed additions not in the doc's original grep-derived
  list; `plugin.rs` and `dedup.rs` are in both).
- Explicitly recommends running this migration through `/sdd:full` or at minimum
  `/sdd:3-plan` + `/sdd:4-validate` given size/risk — consistent with the SDD
  workflow already in progress for this project.

## 3. tree-sitter Rust API surface: what has a typed equivalent and what doesn't

| API | Typed equivalent? |
|---|---|
| `Node::kind() -> &str` | **Yes** — `<Lang>Kind::of(node)` / `from_kind_str()`. This migration's whole subject. |
| `Node::kind_id() -> u16` | No generated equivalent, and **no call site in this repo uses `kind_id()`** — grepped `src/*.rs` + `src/checkers/*.rs` for `kind_id`, zero matches. Not a concern for this migration. |
| `Node::child_by_field_name(&str) -> Option<Node>` | **No typed equivalent exists.** `node_kind.rs`/`build.rs` only generate kind enums from `node-types.json`'s named-kind list; there is no generated `FieldId`/field-name enum, so field-name strings (`"left"`, `"body"`, `"function"`, `"operand"`, etc.) stay raw `&str` literals post-migration, same typo risk as today. This matches the requirements doc's rabbit-hole note that some `rules.rs`/`symbol_extract.rs` comparisons are against **a node's field type**, not a kind string — those are a different problem this migration's tooling does not solve. Confirmed via grep: no `FieldId`/`field_id_for_name`/`field_name_for` generation exists anywhere in `build.rs` or `node_kind.rs`; the two hits for `field_name_for_child` in `symbol_extract.rs:97` and `rules.rs:394` are doc-comments citing manual verification via that tree-sitter API, not generated code. |
| Supertype/grouped comparisons (e.g. matching several concrete kinds as one category) | Partially addressable: each concrete kind still gets its own enum variant, so a supertype match becomes an explicit `matches!(GoKind::of(n), GoKind::A | GoKind::B | ...)` — more verbose than a supertype string check but still compile-checked per variant. No generated helper groups variants back into supertypes. |

## 4. Before/after shape, from real call sites

`src/checkers/go_ignored_error.rs` (small, representative):
```rust
if function.kind() != "selector_expression" { return false; }
...
if operand.kind() != "identifier" { return false; }
...
.filter(|n| n.kind() == "identifier")
```
Post-migration shape: `if GoKind::of(function) != GoKind::SelectorExpression`, etc.
— direct 1:1 replacement, no control-flow change, since every comparison here is a
plain `Node::kind()` against a named concrete kind string with no field-type or
supertype wrinkle. This file is a good "trivial mechanical" exemplar.

`src/checkers/java_ignored_error.rs`:
```rust
fn is_comment(node: Node) -> bool {
    matches!(node.kind(), "line_comment" | "block_comment")
}
...
if node.kind() != "catch_clause" { return; }
...
.filter(|n| n.kind() == "catch_formal_parameter")
```
Post-migration: `matches!(JavaKind::of(node), JavaKind::LineComment | JavaKind::BlockComment)`,
`if JavaKind::of(node) != JavaKind::CatchClause`, etc. Also mechanical — this file's
one `field_name_for_child`-mentioning line (docs/comment reference above is actually
`rules.rs`/`symbol_extract.rs`, not this file) doesn't apply here; every kind check in
`java_ignored_error.rs` is a plain concrete-kind string, no field-type ambiguity.

Both files show the general "after" idiom: `<Lang>Kind::of(node) == <Lang>Kind::Variant`
replacing `node.kind() == "literal"`, and `matches!(<Lang>Kind::of(node), V1 | V2)`
replacing `matches!(node.kind(), "s1" | "s2")`. `child_by_field_name("name")` calls
stay unchanged (no typed equivalent, see §3).

## 5. tree-sitter version pinning vs. vendored `node-types.json` — skew risk

`Cargo.toml` pins:
```
tree-sitter = "0.26"
tree-sitter-go = "0.25"
tree-sitter-typescript = "0.23"
tree-sitter-javascript = "0.23"
tree-sitter-python = "0.23"
tree-sitter-java = "0.23.5"
tree-sitter-kotlin-ng = "1.1.0"
tree-sitter-rust = "0.24"
```
`codegen/node-types/*.json` are vendored copies (not extracted from the actual
crate's registry checkout at build time — `build.rs`'s own doc comment, lines 8-14,
explains why: no `tree-sitter-*` grammar crate exposes `links`/`DEP_*` build-script
metadata, so there's no portable way for `build.rs` to locate another crate's source
dir). This means **the vendored JSON's grammar version and the `Cargo.toml`-pinned
crate version can silently drift** — e.g. if `tree-sitter-go` bumps from 0.25 to a
later 0.25.x/0.26 that adds/renames a node kind, `codegen/node-types/go.json` won't
reflect that until someone manually re-vendors it (build.rs's own comment says as
much: "Re-vendor the file ... if a grammar dependency's version bumps enough to add
new node kinds a checker needs").

- All 8 JSON files are the same size in line count (644 lines is `ls -la` file-size
  column, not line count — actual byte sizes range 50.7K–110.7K, unremarkable) and
  were vendored together in a single commit, `e830c11` ("feat: generate typed
  node-kind enums from vendored grammar node-types.json") — `git log` shows this is
  the only commit touching `codegen/node-types/`, so there is currently **no drift
  yet** between the vendored JSON and whatever `Cargo.lock` resolved at that commit.
- No version-provenance record exists next to the vendored files (no comment header
  in the JSON, no README in `codegen/node-types/`) tying a given JSON snapshot to a
  specific grammar crate version — a re-vendor later would have nothing to diff
  against to confirm it matches the currently pinned Cargo version. This is a real
  gap but **out of scope for this migration** per requirements.md ("Out of Scope:
  New codegen mechanism ... is done") — worth a one-line follow-up note in the plan,
  not a blocker.
- For this migration specifically: as long as the migration lands against the
  *current* `Cargo.lock` state (no incidental grammar crate bumps in the same PR),
  there's no skew risk introduced. The risk only materializes on a *future* grammar
  version bump that isn't paired with a `codegen/node-types/*.json` re-vendor — an
  existing structural risk of the generation approach, not something this migration
  makes better or worse.

## Key takeaways for the planner

1. The generated enums only cover `Node::kind()` strings — field names
   (`child_by_field_name`) have zero typed equivalent, so the "some comparisons are
   against a node's field type" rabbit-hole (rules.rs, symbol_extract.rs) is real and
   unsolved by any existing generated code; those call sites will look different
   (still raw string field names) even after migration.
2. `kind_id()` is unused in this repo — no migration surface there.
3. `node_kind.rs`'s current `#![allow(dead_code, unused_imports)]` should shrink/go
   away as call sites land — track that as an explicit migration-completion signal,
   not just "22 files touched."
4. Small files like `go_ignored_error.rs`/`java_ignored_error.rs` are a clean 1:1
   mechanical `.kind() == "str"` -> `<Lang>Kind::of(n) == <Lang>Kind::Variant` swap —
   a good template/reference PR to land first before the four large/risky files.
