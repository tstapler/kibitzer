# Technology Stack — go-stdlib-idioms-fp

## 1. tree-sitter version and Node API

- **`tree-sitter = "0.26"`**, **`tree-sitter-go = "0.25"`** (`Cargo.toml` lines 25–26).
- `tree_sitter::Node` methods used in this codebase:
  - `.parent()` → `Option<Node<'_>>` — used in `god_class.rs:475` and `primitive_obsession.rs:84` for AST upward traversal.
  - `.child_by_field_name("name")` → `Option<Node>` — field-based child access (used everywhere).
  - `.children(&mut cursor)` / `.named_children(&mut cursor)` / `.children_by_field_name("name", &mut cursor)` — iteration.
  - `.named_child(index)` → `Option<Node>` — positional named-child access.
  - `.kind()` → `&str` — raw grammar kind string.
  - `.utf8_text(src)` → `Result<&str>` — source text slice.
  - `.start_position().row` — 0-based line number (add 1 for 1-based findings).
  - `.is_named()` — distinguishes named nodes from anonymous punctuation/keyword tokens.
  - `.id()` → `usize` — stable identity within a parse (used by `replace-magic-literal`).

## 2. Relevant Go AST node kinds (tree-sitter-go)

For **Issue 1** (primitive-obsession fires on stdlib callback signatures):

| Node kind | `GoKind` variant | Role |
|---|---|---|
| `parameter_list` | `GoKind::ParameterList` | the node the checker visits |
| `parameter_declaration` | `GoKind::ParameterDeclaration` | individual parameter within the list |
| `func_literal` | `GoKind::FuncLiteral` | anonymous function; its `parameter` field holds the `parameter_list`; its parent is `argument_list` when it's a callback |
| `argument_list` | `GoKind::ArgumentList` | parent of a `func_literal` passed as an argument |
| `call_expression` | `GoKind::CallExpression` | grandparent; its `function` field is a `selector_expression` or `identifier` naming the stdlib function |
| `function_declaration` / `method_declaration` | `GoKind::FunctionDeclaration` / `GoKind::MethodDeclaration` | top-level function parents (already excluded from named-return detection via `result` field check at `primitive_obsession.rs:84`) |

The existing `is_named_return_list` guard (`primitive_obsession.rs:83–87`) already uses `node.parent().and_then(|parent| parent.child_by_field_name("result"))` — the same `.parent()` + `.child_by_field_name` idiom extends naturally to the new exclusion.

For **Issue 2** (replace-magic-literal fires on slog attribute-key strings):

| Node kind | `GoKind` variant | Role |
|---|---|---|
| `call_expression` | `GoKind::CallExpression` | the slog/log call |
| `selector_expression` | `GoKind::SelectorExpression` | `log.Warn` / `slog.Info` in the `function` field |
| `argument_list` | `GoKind::ArgumentList` | holds the message string then alternating key/value pairs |
| `interpreted_string_literal` | (literal kind string) | the key strings that currently fire |

Go string literal kinds are `"interpreted_string_literal"` and `"raw_string_literal"` — both are listed in `GO_LITERAL_KINDS` (`rules.rs:56–57`).

## 3. `GoKind` enum

`src/node_kind.rs` re-exports `GoKind` (and per-language siblings) generated at build time by `build.rs` from `codegen/node-types/*.json`. The enum is never hand-edited.

Key API:
- `GoKind::of(node)` — converts `node.kind()` to a typed variant; unknown kinds map to `GoKind::Other`.
- `GoKind::from_kind_str("func_literal")` — same lookup by string (for tests).
- `GoKind::CallExpression`, `GoKind::FuncLiteral`, `GoKind::ParameterList`, `GoKind::ParameterDeclaration`, `GoKind::Identifier`, `GoKind::ConstSpec`, `GoKind::FunctionDeclaration`, `GoKind::MethodDeclaration` are all confirmed variants (`node_kind.rs` tests + usages in `primitive_obsession.rs`, `rules.rs`, `god_class.rs`).
- The variant for `argument_list` is `GoKind::ArgumentList` (follows the same PascalCase-from-snake_case codegen convention; direct use is in `symbol_extract.rs:925` context — confirmed via the `GoKind::of(p) == GoKind::CallExpression` pattern which is its parent).
- The variant for `selector_expression` is `GoKind::SelectorExpression` (used in `symbol_extract.rs:696` as a string, and `god_class.rs` uses `is_call_target` which checks whether a selector's parent is a `CallExpression` — consistent with the selector being a direct child of `CallExpression`).

## 4. Parent-node traversal patterns

Two established patterns, both in `src/checkers/primitive_obsession.rs` and `src/god_class.rs`:

**Pattern A — predicate on parent kind** (`god_class.rs:474–481`):
```rust
fn is_call_target(selector: Node) -> bool {
    selector.parent().is_some_and(|p| {
        GoKind::of(p) == GoKind::CallExpression
            && p.child_by_field_name("function").is_some_and(|f| {
                f.start_byte() == selector.start_byte() && f.end_byte() == selector.end_byte()
            })
    })
}
```

**Pattern B — field-name match against self** (`primitive_obsession.rs:83–87`):
```rust
fn is_named_return_list(node: Node) -> bool {
    node.parent()
        .and_then(|parent| parent.child_by_field_name("result"))
        .is_some_and(|result| result == node)
}
```

Both patterns are lifetime-safe: `Node<'_>` from `.parent()` carries the same `'tree` lifetime as the child, so multi-hop traversal (`node.parent()?.parent()?`) is valid without unsafe code. The `tree_sitter::Node` type implements `PartialEq` by node identity (`id()`), so the `result == node` equality check in Pattern B works as pointer equality.

For Issue 1 the relevant chain is:
```
parameter_list.parent()         → func_literal (kind == GoKind::FuncLiteral)
func_literal.parent()           → argument_list (kind == GoKind::ArgumentList)
argument_list.parent()          → call_expression (kind == GoKind::CallExpression)
```

For test-file detection (`_test.go` closures), `go_call_resolution.rs:143–146` already shows the idiomatic check: `path.file_name().is_some_and(|n| n.to_string_lossy().ends_with("_test.go"))`. The `CheckContext` carries the file path so this check is available to any checker's `check()` method without extra plumbing.

## 5. Existing exclusion infrastructure worth reusing

- `is_named_return_list` in `primitive_obsession.rs` — the `.parent()/.and_then/.is_some_and` chain to exempt a `parameter_list` from firing; the callback-argument exemption follows the identical shape.
- `go_panic_detector` in `rules.rs` — walks `call_expression → function → identifier → text` to identify a named callee; the same walk identifies `slog.Warn`/`log.Warn` in Issue 2 (`call.child_by_field_name("function")` → check kind == `SelectorExpression`, then check the `field` child's text is one of `{"Debug","Info","Warn","Error","Log","WarnContext","InfoContext","DebugContext","ErrorContext"}`).
- `MAGIC_LITERAL_ALLOWLIST` and `resolve_excluded_constants` in `rules.rs` — existing two-pass collect/emit pipeline for `replace-magic-literal`; the slog-key exclusion slots into the `walk_literals` collect phase as an additional per-literal context check, not a post-processing step.
