# Research: UX (Agent 5) — `list_supertypes` / `list_subtypes` MCP tool design

Scope note per assignment: this feature has no GUI. "UX" here means MCP tool
discoverability/usability by an LLM agent calling it zero-shot, plus the human who reads
tool descriptions/error JSON when something goes wrong. Accessibility/keyboard-nav is not
applicable — no interactive surface exists.

## 1. What makes an MCP tool description "discoverable and correctly used" on the first call

Studied `src/mcp.rs`'s four existing architecture-query tools
(`list_architecture_symbols` L809-815, `get_architecture_node` L913-918, `list_callers`
L970-978, `list_callees` L983-990). Their `#[tool(description = "...")]` strings share a
consistent shape that a caller without conversation history can parse:

1. **Verb + direction, in the first clause.** `list_callers`: "Backward call-graph
   traversal: who calls `node`"; `list_callees`: "Forward call-graph traversal: what
   `node` calls". The directionality is stated before anything else, because
   `list_callers`/`list_callees` are otherwise easy to confuse from name alone.
   `list_supertypes`/`list_subtypes` have the exact same confusability risk ("supertype"
   vs "subtype" is a one-word flip) and should follow the identical pattern: lead with
   "who/what is above `node`" vs "who/what is below `node`" in plain English, not just
   the Extends/Implements jargon.
2. **State the exact id format expected**, inline, not by cross-reference alone.
   `CallTraversalRequest::node`'s doc comment (`src/mcp.rs:172-174`) says: "The
   `SymbolNode::id` to traverse from — as returned by `list_architecture_symbols` or
   `get_architecture_node`." This does two things: names the exact type (`SymbolNode::id`)
   and tells the agent which other tool call produces a valid value, so an agent that
   hasn't called anything else yet knows to call `list_architecture_symbols` first rather
   than guessing an id shape. `list_supertypes`/`list_subtypes`'s `node` field should
   carry the same sentence, and additionally state what happens if `node` is a
   `Function`/`Method` id instead of a `Type`/`Interface` id (see error-states section
   below) — that's the one new ambiguity this tool pair introduces that
   `list_callers`/`list_callees` doesn't have (any `SymbolNode` can be a call-graph
   participant; only `Type`/`Interface` participate in the hierarchy graph).
3. **State the return shape and that it's JSON, not prose**, in the tool description
   itself. Every one of the four tools' descriptions ends with `returns JSON
   ({field, field, ...})` naming the top-level fields — e.g. `list_callers`: "returns
   JSON ({node, depth, truncated, edges})". This lets an agent decide whether to call the
   tool at all (vs. `architecture_assessment`, which explicitly contrasts itself as the
   prose alternative in `list_architecture_symbols`'s description: "Use this for a scoped
   lookup... instead of the whole-repo architecture_assessment report"). Follow the same
   convention: `list_supertypes`/`list_subtypes` should name their exact response fields
   in the description, matching `CallTraversalResponse`'s `{node, depth, truncated,
   edges}` shape (see section 3 for whether `edges` needs new sub-fields).
4. **One-hop vs. transitive must be explicit in the description, not just the request
   schema.** `CallTraversalRequest::depth`'s doc comment (`src/mcp.rs:175-179`) states the
   default explicitly ("Defaults to 1 (immediate callers/callees only)") and names the
   clamp bound (`MAX_CALL_DEPTH`). The requirements doc's Open Questions section already
   leans toward direct-only-by-default for `list_supertypes`/`list_subtypes`, matching
   this precedent — if Phase 3 adds a `depth` param for transitive hierarchy walking (e.g.
   for Collapse Hierarchy candidates that need to see a 3-level embed chain), it should
   reuse this exact wording pattern: state the default, name the clamp constant, and
   explain in one clause why a visited-set/clamp exists (`list_callers`/`list_callees`'s
   "The walk keeps a visited-node set so a recursive... call chain can't loop forever" —
   the hierarchy-graph equivalent is a diamond/repeated-embedding cycle, which Go
   multi-level embedding can produce).
5. **Name the resolution-gap caveat up front, not just in a response field's doc
   comment.** `list_callers`/`list_callees`'s description explicitly flags: "An edge with
   `resolved: false` carries the call site's raw, unresolved callee text instead of a
   symbol id (best-effort static resolution — dynamic dispatch/DI/reflection can leave
   gaps)." This primes the agent to branch on `resolved` before it ever sees a response.
   `TypeRelationEdge` inherits the same `resolved: bool` convention per the requirements
   doc's Non-functional Requirements section ("dropped or kept as unresolved text, matching
   `CallEdge`'s existing `resolved: bool` convention"); the tool description should name
   the concrete cause for this domain — "extending/implementing an external, vendored, or
   stdlib type" — exactly the way `list_callers`/`list_callees` names its own cause
   ("dynamic dispatch/DI/reflection").

Recommended description skeleton (illustrative, not final copy — Phase 3/5 will word it):

> "Backward type-hierarchy traversal: what does `node` (a `Type`/`Interface`
> `SymbolNode::id`) extend or implement — returns JSON ({node, depth, truncated, edges}),
> not prose. An edge's `kind` is `extends` (Go struct embedding, TS/Java/Kotlin `extends`)
> or `implements` (TS/Java/Kotlin `implements`); an edge with `resolved: false` names an
> external/vendored/stdlib supertype this model can't resolve to a symbol id. Called on a
> `node` that isn't a `Type`/`Interface`, returns `{"error": "..."}` — see
> `get_architecture_node` to check a node's `kind` first. Use for hierarchy-shaped
> refactoring checks (Extract Superclass, Collapse Hierarchy, Pull Up/Push Down
> Method/Field)."

## 2. Error states

Grepped `json_error` (`src/mcp.rs:241-244`, 16 call sites) and its two established
patterns:

- **Genuinely exceptional / bad input → `json_error(...)`, a `{"error": "..."}` object.**
  Used for: repo-root/config resolution failure, a malformed cursor
  (`list_architecture_symbols`, `src/mcp.rs:840`, "invalid cursor: {c:?}"), and model-build
  failures. This is reserved for cases the agent did something the tool genuinely cannot
  process — not "no results."
- **Valid query, zero results → empty array, never an error.** Proven directly by the test
  `list_callees_returns_empty_array_not_an_error_for_an_unknown_node`
  (`src/mcp.rs:1947-1963`): calling `list_callees` on a `node` id that doesn't exist in the
  model at all returns `{"edges": [], "truncated": false, ...}`, HTTP-200-equivalent, not
  an error. This is a deliberate convention (also see
  `list_architecture_symbols_returns_empty_array_for_zero_matches_not_error`, L2041) — an
  agent doing exploratory queries against a repo it doesn't fully know yet must be able to
  distinguish "your query was malformed" from "your query was fine, there's just nothing
  there," and kibitzer draws that line at input validity, not result cardinality.
- **"Exists but excluded by a default" gets a distinguishing hint, not a bare
  not-found.** `get_architecture_node`'s `exists_but_pruned` field
  (`src/mcp.rs:951-964`) is the precedent: rather than returning an opaque `not_found`
  for a symbol that exists but was filtered by the private-symbol-pruning default, it
  returns `{"kind": "not_found", ..., "exists_but_pruned": true, "hint": "retry with
  include_private: true"}`. This is the template to reuse for `list_supertypes`/
  `list_subtypes`'s new ambiguous case (node exists, but isn't a `Type`/`Interface`).

**Recommendation for `list_supertypes`/`list_subtypes`, by matching precedent exactly:**

- `node` doesn't exist in the model at all → `list_callers`/`list_callees`'s convention
  applies directly: return `{"node": ..., "depth": ..., "truncated": false, "edges": []}`,
  not an error. An agent that hasn't yet resolved the id (e.g. is speculatively probing a
  name) shouldn't be punished with an error for a miss.
- `node` exists but is a `Function`/`Method`, not a `Type`/`Interface` → this is the one
  case with no exact precedent (call-graph traversal has no such restriction — any
  `SymbolNode` participates). Two options, and the `exists_but_pruned` pattern argues for
  the second:
  - (a) Treat identically to "doesn't exist" — empty edges, no error. Simple, but silently
    tells the agent nothing about *why* it got nothing back, and an agent that mistakenly
    passed a method id (a common mistake, since method ids and type ids share the same
    `<package>::Name` shape family) gets no signal to correct course.
  - (b) **Recommended:** mirror `exists_but_pruned` — return a normal empty-edges response
    but add a `node_kind` field (the resolved `SymbolKind` string, e.g. `"function"`) and
    a `hint` string, e.g. `"node exists but is not a Type or Interface; call
    get_architecture_node to check node.kind before calling list_supertypes"`. This keeps
    the "empty result, not an error" invariant (input wasn't malformed, there's just
    structurally nothing to return) while giving the agent enough information to
    self-correct on the next call, exactly the way `exists_but_pruned`'s `hint` field
    already does for a different kind of near-miss.
- Malformed `cursor` (if `list_supertypes`/`list_subtypes` support pagination per the
  requirements doc's constraint that they "must follow the same cursor/limit/next_cursor
  convention as `list_architecture_symbols`") → `json_error("invalid cursor: {c:?}")`,
  identical to `list_architecture_symbols`'s existing handling (`src/mcp.rs:838-841`).

## 3. Job-to-be-done and response shape (inline context vs. follow-up lookup)

The agent calling `list_supertypes`/`list_subtypes` is virtually always mid-refactor-
triage, per the requirements doc's Problem Statement: "is this type a Pull-Up-Method
candidate," "does this type already have a superclass I'd collapse into," "would
Extract Superclass introduce a diamond." In every one of these, the very next thing the
agent needs is **not just "is there an edge" but "what kind of edge, to what" — the two
things that determine which Fowler refactoring even applies** (Collapse Hierarchy only
makes sense for `extends`; Pull Up Method needs a superclass with room to receive a
member; distinguishing "is-a" from "satisfies-interface" is exactly why the requirements
doc rejected a single undifferentiated edge type in its Alternatives Considered section).

**Precedent check — what `list_callers`/`list_callees` inline vs. force a follow-up
for:** `CallEdge` (`src/arch_model.rs:140-146`) has only `from`, `to`, `resolved`, `file`,
`line` — no target `kind` or `name`. An agent that gets a caller/callee id back and needs
to know what kind of symbol it is (e.g. to filter test functions out of an impact
analysis) must issue a separate `get_architecture_node` call per id. This is a real,
observable cost in the existing tools — not hypothetical.

**Recommendation:** don't blindly replicate that gap here, but resolve it by leaning on a
field the requirements doc has *already decided* rather than inventing new inlined
lookups:

- **`kind: Extends | Implements` inlined on `TypeRelationEdge` is non-negotiable and
  already scoped** (Alternatives Considered explicitly rejects a single undifferentiated
  edge type for this reason). This single field answers the *primary* thing the agent
  needs — "is-a" vs. "satisfies-interface" — without a follow-up call, and is cheap
  because it comes for free at extraction time (each language's AST distinguishes an
  `extends` clause from an `implements` clause, or in Go/Kotlin's ambiguous-syntax case,
  the extraction pass already has to resolve target-is-Type-vs-Interface to classify the
  edge — see Rabbit Holes — so recording *that same resolved kind* on the edge is a
  restatement of work already done, not new work).
- **Do not additionally inline the target's `name`/`kind` as separate fields on the
  edge**, for consistency with `CallEdge` precedent and because `TypeRelationEdge.kind`
  already encodes 90% of what a target-kind lookup would tell you (an `Extends` edge's
  target is definitionally a `Type`; an `Implements` edge's target is definitionally an
  `Interface` — Kotlin/Go's ambiguous-syntax cases are exactly the ones the extraction
  pass already had to disambiguate to assign `kind` correctly, so by the time the edge
  exists, no consumer-side ambiguity remains). The only thing a follow-up
  `get_architecture_node` adds beyond `kind` is the target's human-readable `name` (vs.
  its raw id) and its `file`/`line` (already redundant with the edge's own `file`/`line`
  provenance fields, which the requirements doc's Open Questions section says
  `TypeRelationEdge` should carry, matching `CallEdge`). A follow-up call for `name` alone
  is cheap (one id, one call, matches existing agent workflow) and keeps
  `TypeRelationEdge`'s shape parallel to `CallEdge`'s rather than diverging into a
  heavier, MCP-tool-specific response type — consistent with "one model, many views."
- Net shape recommendation: `TypeRelationEdge { from, to, kind: Extends | Implements,
  resolved: bool, file, line }`, and `list_supertypes`/`list_subtypes` return
  `{node, depth, truncated, edges: Vec<TypeRelationEdge>}` — the exact same envelope
  `CallTraversalResponse` uses today, so an agent already competent with
  `list_callers`/`list_callees` transfers that competence with zero new response-shape
  learning, only a new `kind` field to branch on.

## 4. Accessibility / keyboard navigation

Not applicable. `list_supertypes`/`list_subtypes` are MCP JSON-RPC tool calls with no
rendered UI, no GUI, and no human interactive surface — the only "interface" is the tool
description string and JSON schema an agent reads programmatically, covered in section 1.
