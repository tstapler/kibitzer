# Requirements: architecture-export

**Date**: 2026-08-23
**Type**: feature addition
**Complexity**: 3 — system design

## Problem Statement
AI agents and human developers working in a repo kibitzer inspects have no persisted, structured view of that repo's architecture. The closest existing capability — the MCP `architecture_assessment` tool — returns a one-shot report (prose findings + a Mermaid `graph TD` diagram) computed fresh on every call; there is nothing to query, filter, or navigate afterward, and it captures only package/module-level import relationships, not the types/functions within them. An agent that wants to answer "what does package X depend on," "where is type Y defined," or "show me a Component-level view of this subsystem" has to either re-run the whole-repo assessment and parse prose, or fall back to grep/Read across source files.

## Baseline
Today: `kibitzer mcp`'s `architecture_assessment` tool (`src/mcp.rs`) runs `import-cycles`/`layering`/`coupling` checks over an `ImportGraph` (`src/import_graph.rs`, Go and TS/JS import extraction only) and returns transient text plus an optional Mermaid diagram capped at 150 nodes. Nothing is written to disk, nothing is queryable after the call returns, and there is no symbol-level (type/function/interface) information at all — only package-to-package edges.

## Users / Consumers
- AI agents (Claude Code sessions via MCP tools, and future SDD/architecture-review workflows) that need to look up or reason about a repo's structure without re-deriving it from scratch each time.
- Human developers using an LSP-aware editor, or reading a CLI-exported artifact, to browse the same structure.

## Success Metrics
- An agent can query the exported tree (by package, by symbol, by C4-like level) and get a scoped, structured answer without kibitzer re-parsing the whole repo for every query.
- A human can run one CLI command to get a persisted, greppable/jq-able artifact of the repo's architecture, and/or browse it via LSP workspace symbols in an editor.
- kibitzer can render a C4-like diagram (Component/Code-level, explicitly not claiming true Container/Context conformance) from the same underlying model used by the query/export interfaces — one model, multiple views, not three separate ad hoc implementations.
- **Behavior change, not just feature existence**: in practice, a Claude Code session working on this repo (or another kibitzer-inspected repo) uses the query/export/diagram tools instead of falling back to ad hoc grep/Read for an architecture question — the first real test being this project's own future `sdd:6-verify`/architecture-review workflows on this repo. The above three metrics describe the feature working as built; this one describes it actually being reached for over the status-quo alternative, which is the metric that would actually tell us the investment paid off.

## Appetite
Large (3–6 weeks)
*(Scope must fit the appetite. If it doesn't fit, cut scope — do not move the deadline.)*

*Note (added at Phase 4 triad review, 2026-08-24)*: this appetite was pinned in the
ideation interview before Phase 3 planning had resolved the language-coverage
order/MVP cut point that Rabbit Holes below flags as "a major scope lever" — at
interview time it was a provisional estimate against an unresolved scope driver.
`implementation/plan.md` has since resolved this concretely: Go/TS/Tsx/JS symbol+import
coverage plus all three consumer interfaces (CLI export/diagram, MCP query tools, LSP
symbols) is the MVP v1 cut point (see plan.md's "MVP Cut Point" section), with
Python/Java/Kotlin as a severable Phase 5 fast-follow. The Large (3–6 week) appetite is
therefore no longer provisional — it's pinned against a concrete, resolved scope.

## Constraints
No hard deadline — solo-maintained open-source project. No compliance/regulatory constraints.

## Non-functional Requirements
- **Performance SLO**: not specified; should stay usable on kibitzer's own repo size and larger without becoming the slowest part of a session — Phase 2 research should establish a concrete target (e.g. comparable to the existing `architecture_assessment` call, which the daemon/cache infrastructure already exists to help with).
- **Scalability**: should degrade gracefully on large repos the way `architecture_assessment`'s existing 150-node Mermaid fallback does — no unbounded output.
- **Security classification**: public (open-source dev tool; exported architecture data is source-derived, not sensitive by itself — see Open Questions on monorepo scoping).
- **Data residency**: not applicable.

## Scope
### In Scope
- A structured (JSON, tree-shaped) architecture model spanning package/module level (reusing/extending `ImportGraph`) and symbol level (types, interfaces, exported functions) via tree-sitter AST walks.
- CLI export command (e.g. `kibitzer architecture export`) writing that model to a file.
- New MCP tool(s) for querying/navigating the model within a live session (distinct from the existing one-shot `architecture_assessment`, which can remain or be reimplemented on top of the new model).
- LSP workspace/document-symbol integration in `src/lsp.rs` surfacing the same model.
- C4-*like* diagram generation (Component/Code-level visual notation) from the shared model — explicitly not claiming full C4 model conformance.

### Out of Scope
- True C4 Container/Context levels (deployable services, external systems, actors) — not derivable from source alone; would require new user-authored config this project isn't taking on.
- Cross-repo / multi-repo architecture views — single repo only, matching every other kibitzer feature.
- Real-time incremental updates to the exported tree as files change (e.g. via the daemon) — v1 can be run-to-completion on demand; incremental updates are a possible follow-up, not a v1 requirement.

## Rabbit Holes
- Symbol-level extraction currently exists in no form (`import_graph.rs` is package/edge-only); this needs new per-language AST walking. Language coverage is a major scope lever — full parity across all 7 syntax-rules languages (Go/TS/TSX/JS/Python/Java/Kotlin) vs. starting with a subset (e.g. the same Go/TS/JS the import graph already covers) needs to be resolved explicitly in Phase 3 planning, not discovered mid-implementation.
- Designing one shared model that serves CLI export, MCP querying, LSP symbols, and diagram generation without each interface reimplementing its own traversal/filtering logic is the crux of this feature; getting the model's shape wrong risks four divergent implementations instead of four views on one thing.
- "Minimized" tree — the interview didn't pin down what gets pruned/collapsed (e.g. do private/unexported symbols appear? single-method interfaces? generated code?). Phase 3 needs to define concrete minimization rules per language, or this becomes unbounded scope creep of "well actually also show X."
- LSP workspace-symbol integration touches a part of `src/lsp.rs` that today only publishes diagnostics — this may be more novel/unfamiliar surface than the CLI/MCP additions, which extend patterns kibitzer already has (see `install.rs`, `architecture_checks.rs`, `mcp.rs`'s existing tools).

## Alternatives Considered
- Extending the existing one-shot `architecture_assessment` MCP tool with more detail instead of building a persisted/queryable model — rejected in the interview (the interview selected MCP query tools *and* CLI export *and* LSP), since a one-shot text response can't be searched/filtered after the fact the way a structured artifact or live query tool can.
- Adopting an existing UML/C4 generation tool (e.g. Structurizr, PlantUML) instead of building this natively — not selected; kibitzer's value is being self-contained (no external service, works offline, integrates with checks/daemon it already has) and diff-aware, which an external tool wouldn't share out of the box. Phase 2 research should still confirm there isn't a lower-effort path via one of these before committing to a from-scratch implementation.

## Feasibility Risks
- Multi-language symbol-level extraction is real new work per language (Go, TypeScript, TSX, JavaScript, Python, Java, Kotlin all have different AST shapes for "type," "interface," "exported function").
- Designing a single shared model flexible enough for CLI/MCP/LSP/diagram consumers without over-engineering it is a real design risk — see Rabbit Holes above.
- LSP workspace-symbol support may uncover protocol/tower-lsp limitations not yet exercised by kibitzer's existing diagnostics-only LSP usage.
- **Demand risk, not just execution risk**: this is a 3–6 week solo-maintainer investment (Appetite: Large) for a problem whose actual recurrence frequency hasn't been established — how often "what does this repo's architecture look like" is a genuinely blocking need, versus a nice-to-have grep/Read already answers well enough. `research/pitfalls.md` and `research/build-vs-buy.md` assessed *execution* risk (can this be built well, is a from-scratch build the right call over an existing tool) in depth; whether this is worth building at all, at this cost, right now, was not named as a risk in its own right until this Phase 4 triad review caught it.

## Observability Requirements
Standard request/command logging sufficient — this is a local dev tool, not a hosted service. No new metrics/alerts needed.

## Risk Control
Not needed — low risk. This is purely additive (new command/tools/model); it doesn't change behavior of any existing check, hook, or CLI command. No feature flag or rollback procedure beyond normal git revert.

## Open Questions
- Exact minimization rules per language (which symbols/edges get pruned from the tree) — for Phase 3 to define concretely.
- Whether the shared architecture model should be cached/persisted by the existing daemon (`src/daemon.rs`) for reuse across CLI/MCP/LSP calls in one session, or recomputed per call — a performance/complexity tradeoff for Phase 2 research.
- Whether monorepo-style repos with unrelated subprojects need any filtering/scoping beyond the existing `scope` glob pattern already used elsewhere in kibitzer (e.g. `architecture_assessment`'s `scope` param) — likely just reuse that, but worth confirming in research.
- Full symbol-level language coverage order/priority (see Rabbit Holes) — resolve in Phase 3.
