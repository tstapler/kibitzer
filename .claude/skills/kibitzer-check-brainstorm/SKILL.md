---
name: kibitzer-check-brainstorm
description: Mine Claude Code session transcripts for recurring mistake patterns (not just existing kibitzer hook firings) and propose new checks kibitzer should implement, backed by cited real occurrences
---

# Kibitzer Check Brainstorm

kibitzer (`github.com/tstapler/kibitzer`) ships a small set of built-in checks
(`src/primitive_obsession.rs`, and whatever else is registered in
`src/check.rs`). This skill looks for evidence of *new* check ideas — patterns
of agent mistakes, sloppy edits, or missed problems that show up repeatedly
across this machine's Claude Code session history — independent of whether
kibitzer currently has a check for them. Contrast with
`kibitzer-sample-review`, which only reviews firings of checks that already
exist.

The output is a set of candidate checks, each backed by cited real
transcript evidence — not a hypothetical wishlist. An idea with zero observed
occurrences is a guess, not a finding; note it separately if it's still worth
recording.

## Step 1 — Survey what mistake patterns actually recur

Don't grep for a specific keyword up front — that presupposes the category of
mistake. Instead sample broadly across sessions and look at what assistant
turns get corrected, reverted, or flagged by the user.

Useful raw signals, per transcript file in `~/.claude/projects/*/*.jsonl`:

```bash
# User turns that read as corrections — cheap first pass, high false-positive rate,
# but a good way to find candidate sessions worth reading in full.
for f in ~/.claude/projects/*/*.jsonl; do
  jq -r 'select(.type=="user") | .message.content
    | if type=="string" then . else (.[]? | .text? // empty) end' "$f" 2>/dev/null \
    | grep -iE "no,? (that|don'\''t)|revert|undo|wrong|that broke|not what I (asked|meant)" \
    | sed "s#^#$f: #"
done

# Tool-use sequences where an Edit/Write was immediately followed by another
# Edit to the same file within the same turn or the next one — a cheap proxy
# for "got it wrong on the first try."
```

Read the surrounding turns for anything that looks like a *pattern*, not a
one-off: the same class of bug (missing error check, dead import, TODO left
behind, secret-looking string committed, inconsistent naming) appearing
across multiple unrelated sessions or repos.

## Step 2 — For each candidate pattern, gather at least 2 independent occurrences

One occurrence is an anecdote. Before writing it up as a candidate check,
find a second instance in a different session/repo, or explicitly note that
only one has been found and flag it as weaker evidence. For each occurrence
record:

- transcript path and rough timestamp/turn
- repo and file(s) involved
- what the agent did wrong, in the actual edit (not a paraphrase — quote the
  diff or relevant `old_string`/`new_string`)
- whether the user caught it, and how (explicit correction, silent revert in
  a later turn, or not caught at all — only found by re-reading the transcript)

## Step 3 — Check it isn't already covered

Before proposing a check, confirm it's genuinely new:

- Read `src/check.rs` and every `src/*.rs` check module for what's already
  implemented.
- Check `docs/*-false-positives.md` — a pattern that looks new might be a
  known failure mode of an existing check, which belongs in that check's log
  instead of a new proposal.

## Step 4 — Write up each candidate

Append to `docs/check-ideas.md` (create it if it doesn't exist yet) — one
entry per candidate, not one doc per candidate:

```markdown
### <short-name> — proposed <YYYY-MM-DD>

- **Pattern**: what recurring mistake this would catch, in one or two sentences
- **Evidence**:
  - `<transcript path>` — `<repo>`, `<file>`: <one-line description of the occurrence>
  - `<transcript path>` — `<repo>`, `<file>`: <one-line description of the occurrence>
- **Why kibitzer, not something else**: why this fits kibitzer's model
  (diff-aware, per-file/per-repo check invoked from a hook/CLI/MCP call) rather
  than e.g. a linter rule, CI step, or one-off fix
- **Feasibility sketch**: rough shape of the check — what it inspects
  (AST via an existing crate, regex, file metadata), diff-aware or whole-file,
  language scope
- **Open questions**: anything that needs a design decision before implementation
  (blocking vs. advisory, false-positive risk, config surface)
```

Rank entries by evidence strength (occurrence count, severity of what slipped
through) — put the strongest cases first so a reader triaging the list sees
the best-supported ideas without reading the whole file.

## Step 5 — Don't overclaim novelty

If a pattern is well-known generic advice (e.g. "don't hardcode secrets")
with no evidence it's actually happened in these transcripts, say so
explicitly rather than presenting it with the same confidence as a
cited-occurrence finding. Speculative ideas can go in a separate `## Untested
ideas (no observed occurrences yet)` section at the bottom of
`check-ideas.md`, clearly distinguished from the evidenced list above it.

## Related

- `kibitzer-sample-review` — the companion skill for false/true-positive
  review of checks kibitzer *already* runs; use that one once a check
  proposed here actually ships.
- `docs/checking-invocations.md` — same transcript-querying caveats apply
  here: a raw text match is not evidence, and `attachment` records are the
  only trustworthy signal for what actually happened in a tool call.
