# Requirements: hollow-qualifier-check

## Item

Backlog candidate: a new kibitzer check for **hollow qualifier phrasing** in markdown
headings and lead sentences — the pattern where a noun or noun phrase is followed by a
distancing adverb ("fundamentally", "generically", "essentially", "broadly", "ultimately",
"basically") that describes the *category* of what follows instead of its content.
Example: "The primitive, generically" or "the approach, fundamentally." Grammatically
valid; reads stiff and robotic.

## Context

- Observed occurrence: heading "The primitive, generically" in `titus-state-ownership-nop/README.md`
  (design-docs repo), flagged by the user during a 2026-09-23 review session as
  "terrible, robotic." Logged in `docs/check-ideas.md`'s "Untested ideas" section and
  on the `writing:tone` skill side.
- **Below threshold for implementation.** This repo's methodology bar requires 2+
  independent occurrences before implementing a new check. One confirmed occurrence is on
  file. This triage is filed to track the candidate and plan a future implementation;
  implementation should not start until a second independent occurrence is confirmed.
- Closest analogues in the codebase: `repetitive-sentence-structure`,
  `ai-vocabulary-density`, `filler-phrase-density` — all single-file regex/markdown-parse
  scans over `**/*.md`, advisory severity. This check would follow the same shape.

## Proposed check behavior

| Attribute | Value |
|---|---|
| **Name** | `hollow-qualifier-phrasing` |
| **Severity** | Advisory (style judgment, not a broken fact) |
| **Scope** | `**/*.md` |
| **Trigger** | Single-file scan (`PostToolUse`) |
| **Default** | Start as opt-in (like `ai-vocabulary-density`); graduate to default if backtest FP rate is acceptably low |

**Pattern to flag:** In a heading or the first sentence of a paragraph, a noun phrase
followed by a comma and one of a hardcoded set of distancing adverbs:

```
<noun phrase>, <adverb>
```

Where the adverb list includes words like: "fundamentally", "generically",
"essentially", "broadly", "ultimately", "basically", "generally", "conceptually",
"abstractly", "theoretically".

**Exact heuristic to implement:**
1. For each heading (ATX `#`/`##`/… and setext) in the markdown, check if the heading
   text matches the pattern `^.+,\s*(fundamentally|generically|essentially|...)$` (case-
   insensitive, trimmed).
2. For each prose paragraph's first sentence, apply the same regex.
3. Report a finding at the heading/paragraph's start line with a message like: `heading
   reads as category label, not content — "The primitive, generically" names the kind of
   thing, not the thing itself`.

**Non-goals:**
- Do NOT scan all sentences (just headings + first sentences of paragraphs) — limits
  false positive surface.
- Do NOT flag if the adverb appears mid-sentence (only comma-then-adverb at end of
  heading/sentence).
- Do NOT require tree-sitter grammar — pure regex over pulldown-cmark's heading events
  is sufficient, matching the existing `markdown_link_integrity` approach.

## Acceptance criteria

1. A heading ending with `, generically` (case-insensitive) produces a finding.
2. A heading ending with `, fundamentally` produces a finding.
3. A paragraph lead sentence ending with `, essentially` produces a finding.
4. A heading that does NOT end with a distancing adverb does NOT produce a finding.
5. A mid-paragraph sentence (not the first) ending with `, generically` does NOT produce
   a finding.
6. A list item ending with `, generically` does NOT produce a finding (list items are
   excluded, consistent with other prose checkers).
7. The check is registered via `inventory::submit!` and is callable with
   `kibitzer check native hollow-qualifier-phrasing <file>`.
8. Absent a `.kibitzer/inspect.json` opt-in, the check does NOT run in default mode
   (opt-in only, like `ai-vocabulary-density`).
9. The adverb list is configurable via `Check.options` in `.kibitzer/inspect.json`
   (same pattern as `ai-vocabulary-density`'s `words` option).
10. Unit tests pass for all the above scenarios.
11. The check has been backtested against the standard public-code corpus
    (`docs/backtest-repos.md`) and the FP rate documented before being considered for
    graduation to `default_checks()`.
12. `docs/prose-checks.md` updated to list the new check in the opt-in table.

## Out of scope

- Graduating the check from opt-in to default (deferred until backtest data available).
- Full NLP/POS-tagging — regex is sufficient for the mechanical structural pattern.
- Setext headings (underline-style `===`/`---`) — ATX headings (`#`) cover the observed
  failure mode; setext can be a follow-up if occurrence evidence surfaces.
