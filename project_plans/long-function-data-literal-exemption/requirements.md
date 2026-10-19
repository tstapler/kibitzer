# Requirements: long-function data-literal exemption

Backlog item 124cfa8b-cb77-44bd-9b60-b54d59c7b6c1. Precision enhancement, not a false positive:
`long-function` fires as specified on a function whose body is one flat data literal
(e.g. a Go table-test case list), but there is nothing to extract, and extracting the
table just trips the rule at the new location.

Related planning artifacts from an earlier pass live in `../long-function-flat-literal/`
(research, plan, validation); this file is the criteria-aligned requirements for the item.

## Chosen approach

Option 1 from the item: skip `long-function` when the body is a single `return` of a
collection/composite literal whose subtree has no control-flow or call nodes.
Rejected: counting only statement lines (changes the metric for all functions) and
skipping `_test.go` (hides genuinely long test setup).

## Acceptance criteria

1. Go: a body that is one `return` of a flat composite literal (no control-flow or call
   nodes in its subtree) yields no `[long-function]` finding, for any entry count.
2. Go: single-`return` literal containing a call expression (e.g. `fmt.Sprintf(...)`) is
   still flagged when the body exceeds 40 lines.
3. Go: single-`return` literal with a nested `if`/`for` in an entry is still flagged when
   the body exceeds 40 lines.
4. Go: a body with more than one statement is unaffected.
5. Exemption driven by a new per-language `collection_literal_kinds` field; applies to at
   least Go and TypeScript. Java, Kotlin, Rust use `&[]`, behavior unchanged.
6. Unit tests: (a) Go flat literal exempt, (b) Go literal with call flagged, (c) Go
   multi-statement flagged, (d) TypeScript flat array exempt, (e) existing
   `flags_long_function` still passes.
7. `node_kind_literals_are_valid_for_their_grammar` validates `collection_literal_kinds`
   for all 8 languages.
8. `docs/syntax-rules.md` `long-function` description documents the exemption.
9. `kibitzer check backtest syntax-rules` run; result noted in the PR description.
10. `kubernetes/kubernetes` test-package sample confirms newly silenced findings are flat
    fixture helpers, not Extract Function candidates.

## Scope

Code: `src/checkers/rules.rs` (`LangRuleConfig`, the per-language constructors,
`check_declaration`). Docs: `docs/syntax-rules.md`.
