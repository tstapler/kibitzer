# ADR-002: Filter slog key literals via HashSet in LiteralCollector, not a per-call predicate

**Status**: Accepted
**Date**: 2026-10-10

## Context

`replace-magic-literal` fires on string literals like `"file"` and `"err"` that appear
repeatedly as slog attribute keys in `log.Warn("msg", "file", name, "err", err)`. These are
in the alternating key-value position mandated by the slog convention; extracting them as
named constants makes the call site less readable, not more.

Two data-model approaches were considered for exempting these literals.

## Decision

Add a `slog_key_predicate: Option<fn(Node, &[u8]) -> bool>` field to `LangRuleConfig`
(set to `Some(go_is_slog_key_literal)` for Go, `None` for all other seven languages), and a
`slog_key_ids: HashSet<usize>` field to `LiteralCollector`. During `walk_literals`, when the
predicate is `Some` and returns `true` for a literal node, record that node's `id()` in
`slog_key_ids`. In `build_literal_finding`, filter occurrences whose `node.id()` is in
`slog_key_ids` before applying the repetition threshold.

## Alternatives considered

**A. Inline predicate call in `walk_literals` that skips adding to `occurrences` entirely**
— rejected. If a literal appears 2 times as a slog key and once as a genuine repeated
string, removing it from `occurrences` entirely would prevent the single non-key occurrence
from ever being counted. The `HashSet` approach correctly attributes each occurrence
individually.

**B. New `LangRuleConfig` field `exempt_literal_predicate` called only from `build_literal_finding`**
— also viable. The `HashSet` approach was preferred because: (1) it keeps the node-tree
traversal (walking, predicate eval, ID recording) in the single `walk_literals` pass, rather
than re-traversing the parent chain per literal per `build_literal_finding` call; (2) it
does not require changing `build_literal_finding`'s interface beyond adding one `HashSet`
argument; (3) the `LiteralCollector` already accumulates per-node data (`bound`), so a
second accumulation field is idiomatic.

**C. Per-language override of `walk_literals`** — rejected. Would require duplicating or
abstracting the entire walk, which is significantly more invasive than either field-based
approach.

## Consequences

- `LangRuleConfig` gains one new field; all eight language configs must set it (`None` for
  seven of them). This is a mechanical, low-risk change.
- `LiteralCollector` gains one new field; `#[derive(Default)]` handles zero-initialization.
- `emit_literal_findings` and `build_literal_finding` gain one additional `&HashSet<usize>`
  argument.
- `slog.With()` is excluded from the initial `SLOG_METHOD_NAMES` set because its first
  argument is already a key (no leading message), making its position convention the
  opposite of `Info`/`Warn`/`Error`/`Debug`. It can be added in a follow-up after
  verifying the AST shape.
