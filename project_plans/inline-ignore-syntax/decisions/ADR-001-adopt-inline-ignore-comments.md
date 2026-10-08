# ADR-001: Adopt inline ignore comments (reverses "no inline suppression")

**Status**: Accepted (pending implementation)  **Date**: 2026-10-07

## Context
`docs/suppressing-checks.md:11` and `docs/accepting-findings.md:80` state that kibitzer deliberately has no inline suppression comment; the per-finding lever is `.kibitzer/accepted/*.json`, keyed on `(rule, file, line, content)` (`src/accepted_findings.rs`). That entry goes stale on any edit to the line and is costly for an agent to author, so already-judged findings are re-fed by the hook and re-triaged. The original stance rested on two properties: suppressions are reviewable, and each carries a written reason.

## Decision
Add `kibitzer:ignore <rule>[,<rule>] -- <reason>` (accepted tradeoff) and `kibitzer:false-positive <rule>[,<rule>] -- <reason>` (checker misfire), as real comments (tree-sitter comment node, Markdown HTML comment, or leading-comment regex fallback), on the finding's line or the line directly above it. Rule and a non-empty reason are mandatory; there is no blanket, block, or file scope. Both markers suppress identically; the second feeds a maintainer worklist (`kibitzer check false-positives list --inline`).

Both original properties survive: the comment is in the diff, and the reason is enforced by the parser (a malformed ignore suppresses nothing and emits a finding). `.kibitzer/accepted/` stays, as the fallback for locations that cannot hold a comment, and is unchanged.

Round 3 (2026-10-07): the second marker is conditional. Plan Task 0.1.2 tests marker choice before any implementation; if agreement with the hand label is under 70% or checker misfires are under 10% of addressable cases, collapse to one marker (`kibitzer:ignore`) and drop the listing command. The whole decision is also gated by plan Phase 0 (baseline replay go/no-go).

## Alternatives rejected
- Config-only plus an agent-writable `kibitzer accept` tool: out of scope per requirements; still one file per finding.
- Reuse `# noqa` / `// nolint` / `eslint-disable`: other tools own those semantics and most make the reason optional.
- Content-hash-anchored `accepted/` entries: still a separate file per finding.

## Consequences
Both docs are updated in the same change. Suppressions can now be scattered through source; mitigated by `[ignore-volume]`, `[unused-ignore]`, the run-footer suppressed count, and greppable markers (`rg 'kibitzer:'`).
