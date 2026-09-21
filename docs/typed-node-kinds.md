# Typed node kinds

`src/node_kind.rs` generates one `<Lang>Kind` enum per tree-sitter grammar this crate
uses (`GoKind`, `TypeScriptKind`, `TsxKind`, `JavaScriptKind`, `PythonKind`, `JavaKind`,
`KotlinKind`, `RustKind`) from that grammar's own vendored `node-types.json`
(`codegen/node-types/*.json`, `build.rs` does the generation). Each enum has:

- `<Lang>Kind::of(node)` — the typed kind of a parsed `tree_sitter::Node`.
- `<Lang>Kind::from_kind_str(s)` — same conversion from a raw `&str`.
- `.as_str()` — back to the raw string, for anything still expecting one (e.g. an error
  message).
- `Other` — catch-all for a kind string outside that grammar's vendored, named, concrete
  node set: a synthetic `ERROR`/`MISSING` node, or an anonymous/punctuation token (see
  `build.rs`'s `generate_enum` doc comment for the `named`/`subtypes` filter that decides
  what becomes a variant).

## Why

A checker matching `node.kind() == "if_statment"` (typo) compiles fine and just silently
never matches — the false negative is invisible until someone notices the checker doesn't
fire on real code. `GoKind::of(node) == GoKind::IfStatment` is a compile error instead: the
variant doesn't exist.

## What's migrated so far

Nothing yet — this is infrastructure only, landed ahead of the migration itself (see issue
tracking for the follow-up). `rules.rs` already has a lighter-weight, complementary
mechanism worth knowing about: `node_kind_literals_are_valid_for_their_grammar` validates
its `LangRuleConfig` kind-string literals against `tree_sitter::Language::id_for_node_kind`
at *test* time, not compile time, and only for that one file's literals. Once `rules.rs`'s
literals move to the typed enums, that runtime test becomes redundant for those fields.

21 files currently compare `node.kind()` against a raw string literal and are candidates
for this migration (`grep -rln 'node.kind() ==\|\.kind() ==\|match .*\.kind()' src/*.rs`):

```
complexity.rs             go_ignored_error.rs        java_swallowed_interrupt.rs
complexity_tests.rs       go_type_switch_density.rs  plugin.rs
declarations.rs           god_class.rs               primitive_obsession.rs
dedup.rs                  import_graph.rs            rules.rs
go_blank_imports.rs       isp_fat_interface.rs       symbol_extract.rs
go_call_resolution.rs     java_error_context.rs      tree_walk.rs
go_error_context.rs       java_ignored_error.rs
                          java_lost_exception_cause.rs
```

`rules.rs`, `symbol_extract.rs`, `import_graph.rs`, and `declarations.rs` are among the
largest files in the crate — converting those is the highest-risk, highest-payoff part of
the migration and is not a small mechanical find-replace (some `match` arms compare
against a node's *field* type, a supertype grouping several concrete kinds, or a kind that
differs subtly between two grammars sharing one checker function). Recommended to run
through `/sdd:full` (or at minimum `/sdd:3-plan` + `/sdd:4-validate`) rather than converting
ad hoc, given the size and the working, tested behavior at stake.
