# long-function: exempt flat data-literal bodies

Backlog item `124cfa8b-cb77-44bd-9b60-b54d59c7b6c1`. Research, plan, adversarial review and
validation for this change already exist in `../long-function-flat-literal/`; this file is the
requirements record for the item as run under the sdd pipeline.

## Problem

`long-function` (`src/checkers/rules.rs`, `check_declaration`) counts body lines only. A function
whose body is one `return` of a flat table literal (a table-test case list) has nothing to extract,
so the finding is noise and the usual remedy (extract a helper) re-trips the rule.

## Requirements

1. Go: a body that is a single `return` of a composite literal with no control-flow or call nodes in
   its subtree is not flagged, however many entries it has.
2. A literal containing a call expression, or a nested `if`/`for`/func literal with control flow, is
   still flagged above 40 lines.
3. A body with more than one statement is unaffected.
4. The exemption is driven by a per-language `collection_literal_kinds` field: Go and TypeScript
   (plus Python) populate it; Java, Kotlin and Rust use `&[]`, so their behavior is unchanged.
5. Tests: Go flat exempted, Go with call flagged, Go multi-statement flagged, TS flat array exempted,
   existing `flags_long_function` still passes; `node_kind_literals_are_valid_for_their_grammar`
   validates the new field for all languages.
6. `docs/syntax-rules.md` documents the exemption.
7. Backtest (`kibitzer check backtest syntax-rules`) and a `kubernetes/kubernetes` corpus sample
   confirm no genuine Extract Function candidate is silenced; results go in the PR description.

## Non-goals

Per-rule scoping, `_test.go` special-casing, or counting only statement lines (options 2 and 3 of
the item).
