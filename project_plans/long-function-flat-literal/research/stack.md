# Stack Research: long-function-flat-literal

No new dependencies. All implementation uses:

| Component | Already in use | Notes |
|-----------|---------------|-------|
| `tree_sitter::Node` traversal (`named_children`, `child_by_field_name`, `kind()`) | Yes | Same API used by `max_nesting_depth`, `check_block_for_unreachable`, etc. |
| `LangRuleConfig` struct fields (`statement_container`, `return_kind`, `nesting_kinds`, `call_kinds`, `if_kind`) | Yes | Read-only access |
| `LangRuleConfig.collection_literal_kinds` | New field (8 lines across constructors) | Or derived from `density_boundary_kinds` per `research/architecture.md` Option A — either way, no new dependency |
| Rust test infrastructure (`#[test]`, inline `check_source`) | Yes | No new test framework |

The only new code is:
1. One new function `is_flat_data_literal_body(body: Node, cfg: &LangRuleConfig) -> bool`
2. One new field `collection_literal_kinds` on `LangRuleConfig` (optional per Option A)
3. One guard expression in `check_declaration`'s `long-function` branch
4. 4–6 unit tests in `src/checkers/rules.rs`'s `mod tests`
5. One paragraph update in `docs/syntax-rules.md`
