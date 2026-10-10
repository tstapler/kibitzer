# Architecture Research: go-stdlib-idioms-fp

Two independent fixes, each in its own file. Analysis below is per-fix.

## Fix 1: `primitive_obsession.rs` — suppress func_literal callback and `_test.go` findings

### Current traversal shape

`walk()` (`src/checkers/primitive_obsession.rs:66-73`) drives `tree_walk::walk_preorder` and
fires on every `ParameterList` node that passes `!is_named_return_list(n)`.
`is_named_return_list` (`lines 83-87`) already shows the upward-traversal pattern this fix
reuses: `node.parent()` gives the signature node, and `parent.child_by_field_name("result")`
distinguishes a named-return `parameter_list` from a call-site one.

`tree_walk::walk_preorder` (`src/tree_walk.rs:40-56`) visits in preorder using `TreeCursor`
internals — no Rust stack growth per depth level. The `visit` closure returns a `bool` to
prune subtrees; `false` from the closure skips all descendants. Neither `walk()` nor
`check_parameter_list` uses the `Path` argument today (`_file: &Path` in `check()`).

### What needs to change

**Suppression 1 — stdlib callback `func_literal` arguments.** For a `parameter_list` inside
`sort.Slice(data, func(i, j int) bool {...})`, the AST parent chain is:

```
parameter_list → (function signature of the func_literal's type) → func_literal
               → argument_list → call_expression
```

The existing `is_run_subtest_closure` in `src/checkers/complexity.rs:246-264` is the
direct template: start from the inner node, walk `.parent()` twice to reach `func_literal`,
then check `parent == ArgumentList`, grandparent `== CallExpression`, then inspect the
`function` field for the callee name. A new `is_stdlib_callback_param_list(node: Node,
src: &[u8]) -> bool` function follows the same shape:

1. `node.parent()` → the `func_type` (function signature node of the `func_literal`).
2. `.parent()` → the `func_literal` (`GoKind::FuncLiteral`).
3. `.parent()` → the `argument_list` (`GoKind::ArgumentList`).
4. `.parent()` → the `call_expression` (`GoKind::CallExpression`).
5. `call.child_by_field_name("function")` filtered to `GoKind::SelectorExpression`, then
   `.child_by_field_name("field")` compared to the stdlib callback set: `{"Slice", "SliceStable",
   "SliceIsSorted", "Search", "SearchInts", "SearchFloat64s", "Sort"}` (covering `sort.*`)
   plus `{"Walk"}` (filepath.Walk) and similar. The operand (package name) could optionally
   be checked too (`sort`, `filepath`, `http`) but is not required for a first pass — the
   method name alone disambiguates sufficiently, and false negatives from an ambiguous name
   are accepted over false positives from checking every closure.

All four `GoKind` variants needed — `FuncLiteral`, `ArgumentList`, `CallExpression`,
`SelectorExpression` — are already used in `complexity.rs:211,250,256,260` and compile
today.

**Suppression 2 — `_test.go` files.** The `check()` method receives `file: &Path` (currently
`_file`). Add an early return before `walk(...)`:

```rust
if file.to_str().is_some_and(|s| s.ends_with("_test.go")) {
    return Ok(Vec::new());
}
```

No new dependencies; the path is already in scope.

### Disposition: Extend as-is

Two small, additive changes. No new walks, no new structs. The parent-traversal shape is
already proven in `complexity.rs`; re-expressing it in `primitive_obsession.rs` as a
dedicated predicate keeps each function doing one thing. The callback name set should be a
named `const` slice (same convention as `PRIMITIVE_TYPES`) rather than inlined in the
predicate.

---

## Fix 2: `rules.rs` — suppress `replace-magic-literal` for slog/log key positions

### Current two-pass architecture

`check()` (`src/checkers/rules.rs:1204-1227`) runs:

1. `walk_literals(root, cfg, src, &mut collector)` — collects every `literal_kinds` node
   into `collector.occurrences` (keyed by raw text) and every const-binding into
   `collector.bound`. **Already uses upward traversal via `.parent()` in `binding_finder`
   implementations** — so parent traversal inside `walk_literals` itself is an established
   pattern, not a new idiom.
2. `resolve_excluded_constants(root, src, &collector.bound)` → `HashSet<usize>` of
   excluded node IDs (const-bound literal initializers referenced ≥2 times).
3. `emit_literal_findings(&collector, cfg, &excluded, findings)` → filters by
   `MAGIC_LITERAL_MIN_OCCURRENCES` and emits one finding per qualifying literal, excluding
   nodes in `excluded`.

The `LiteralCollector` struct (`lines 188-191`) has two fields: `occurrences` and `bound`.
`LiteralOccurrence` (`lines 178-181`) holds the `Node` and its `kind`.

The `LangRuleConfig` struct (`lines 198+`) uses function-pointer fields for per-language
behavior: `binding_finder`, `body_finder`, `params_finder`, `bool_param_finder`,
`panic_detector`. A new optional function-pointer field follows this established pattern
exactly — it's not an ad-hoc addition.

### What needs to change

**Step 1 — `LangRuleConfig` field.** Add:

```rust
/// For Go: returns `true` if `node` is a string literal at an odd argument position
/// (0-indexed: 1, 3, 5, …) of a slog/log family call — i.e. a key in a
/// `slog.Info("msg", "key1", val1, …)` alternating key-value sequence.
/// `None` for all non-Go grammars; `None` also means "never exclude by this rule."
slog_key_literal: Option<fn(Node, &[u8]) -> bool>,
```

Set to `Some(go_is_slog_key_literal)` in `go_lang_config()`, `None` elsewhere.
No effect on the 7 other languages; no changes to their config constructors beyond adding
`slog_key_literal: None`.

**Step 2 — `go_is_slog_key_literal(node, src) -> bool`.** Parent-traversal predicate:

1. `node.parent()` must be `GoKind::ArgumentList`.
2. `ArgumentList.parent()` must be `GoKind::CallExpression`.
3. The call's `function` field must be a `GoKind::SelectorExpression` whose `field` text is
   in the slog/log method set: `{"Info", "Debug", "Warn", "Error", "Log", "InfoCtx",
   "DebugCtx", "WarnCtx", "ErrorCtx", "With", "WithGroup"}` — covers both `slog.*` and
   `*slog.Logger.Method` forms since tree-sitter sees the same AST shape for both
   package-qualified and method calls; the package/receiver name (`slog`, `logger`, `log`)
   is deliberately not checked to avoid false negatives from locally aliased loggers.
4. The node's 0-indexed position among the `ArgumentList`'s named children is odd (1, 3,
   5, …) — i.e. a key position in the alternating `(key string, val any, …)` schema. The
   first argument (index 0) is either the message string or a `context.Context`; either way
   it is not a slog key.

Position is counted via `argument_list.named_children(&mut cursor).position(|c| c.id() ==
node.id())`.

**Step 3 — `LiteralCollector` field.** Add:

```rust
pub(crate) slog_key_ids: HashSet<usize>,
```

Populated in `walk_literals` when `cfg.slog_key_literal` is `Some(f)` and `f(node, src)`
returns `true`: `collector.slog_key_ids.insert(node.id())`.

**Step 4 — `emit_literal_findings` / `build_literal_finding`.** Pass `slog_key_ids`
alongside `excluded` to the filter:

```rust
.filter(|occ| !excluded.contains(&occ.node.id()) && !slog_key_ids.contains(&occ.node.id()))
```

No separate walk needed — the check is O(1) per literal node and fits inside the existing
`walk_literals` dispatch, consistent with `binding_finder`'s inline collection pattern.

### Why not a third walk (`walk_slog_keys`)

A separate `walk_slog_keys` pass would maintain the "one walk per traversal scope" doc
comment discipline from the `replace-magic-literal` architecture.md. However:

- The inline check inside `walk_literals` is cheaper (one pass instead of two) and the
  per-node work (`is_slog_key_literal`) is O(1) via `.parent()` — not a child scan that
  would make the walk O(n²).
- `binding_finder` already demonstrates that per-node context checks (upward traversal) live
  inside `walk_literals`, not in a separate walk. Slog key detection is the same shape.
- A third walk would be warranted if the slog exclusion needed cross-node aggregation (like
  `walk_literals`'s own occurrence counting does). It doesn't — each node is independently
  decidable.

### Disposition: Extend as-is

`rules.rs` is already organized around exactly this pattern: per-language behavior lives in
`LangRuleConfig` function-pointer fields, and the walk calls them inline. Adding one new
optional field + one new Go-specific predicate + one new collector field fits without
straining the file's structure. The `node_kind_literals_are_valid_for_their_grammar` test
(`src/checkers/rules.rs:1171-1216`) does not need a new entry because this change adds no
new literal kind lists — only a predicate on existing nodes.

---

## Cross-fix summary

| Fix | Files changed | New constructs | Pattern borrowed from |
|-----|--------------|----------------|----------------------|
| primitive-obsession | `src/checkers/primitive_obsession.rs` (1 file) | `is_stdlib_callback_param_list`, test-file early return | `is_named_return_list`, `complexity.rs:is_run_subtest_closure` |
| replace-magic-literal slog | `src/checkers/rules.rs` (1 file) | `go_is_slog_key_literal`, `slog_key_literal` field, `slog_key_ids` field | `binding_finder` inline collection, existing `excluded` filter |

Both fixes are surgical, self-contained within their respective files, and reuse existing
parent-traversal and function-pointer-field patterns already proven in this codebase.
