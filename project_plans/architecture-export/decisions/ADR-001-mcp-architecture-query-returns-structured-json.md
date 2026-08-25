# ADR-001: `list_architecture_symbols`/`get_architecture_node` return structured JSON, not the flat-string convention every other kibitzer MCP tool uses

**Status**: Accepted
**Date**: 2026-08-23

## Context

Every existing kibitzer MCP tool (`list_checks`, `run_checks`, `architecture_assessment` in
`src/mcp.rs`) returns a flat, human/agent-readable `String` — prose lines in the
`[level] message` convention, with `##`-prefixed section headers for structure
(`architecture_assessment`'s `## Recommendations` / `## Dependency graph`). This is a
deliberate, consistent house style (confirmed by reading `src/mcp.rs` directly), not an
oversight — it matches the fact that those three tools each report a bounded list of
findings meant to be read, not machine-navigated field-by-field.

The new architecture-export feature introduces `list_architecture_symbols` and
`get_architecture_node` (`src/mcp.rs`), which query a tree-shaped `ArchModel`
(`src/arch_model.rs`) — packages containing symbols, each with `kind`/`exported`/`file`/
`line`/`parent` fields. The requirements doc's own success metric is that an agent can
"query the exported tree... and get a scoped, structured answer" without re-parsing prose.

## Decision

`list_architecture_symbols` and `get_architecture_node` return `serde_json::to_string(&response)`
— a real JSON object with typed fields (`packages`/`symbols` arrays, `total_matched`,
`next_cursor`, etc.) — not a formatted `String` in the existing tools' prose convention.

This is an intentional, explicit deviation from house style, not an oversight future
maintainers should "fix" back to consistency. It is scoped narrowly: only these two new
*query* tools deviate. `architecture_assessment` (and any future one-shot advisory-report
tool) keeps the flat-string convention, since a report meant to be read by a human or
summarized by an agent is a different shape of output than a scoped, field-level query
result meant to be parsed.

Each tool's `#[tool(description = "...")]` states "returns JSON" explicitly (per the UX
research finding that an agent conditioned on the other two tools' text output will
otherwise try to string-match a JSON blob), and `KibitzerServer::get_info()`'s
`instructions` field gets a clause distinguishing the two query tools ("return JSON") from
the three prose tools, so an agent choosing between them has session-level guidance.

## Alternatives Rejected

- **Match the existing flat-`String` convention** (e.g. one line per symbol, `##`
  sections per package) — rejected because it forces the agent to re-parse prose to
  extract structured fields it already has natively in `ArchModel`, defeating the
  "queryable, scoped answer" success metric the whole feature exists to satisfy.
- **Wrap the JSON in a `String` with a text preamble** (e.g. `"3 symbols found:\n<json>"`)
  — rejected as neither fish nor fowl: it doesn't help a human skim it, and it forces an
  agent to strip a preamble before it can `JSON.parse`/`serde_json::from_str` the payload.
  Returning `serde_json::to_string` directly, described as JSON in the tool description,
  is unambiguous.

## Consequences

- An agent has to learn two different response shapes across kibitzer's five MCP tools
  (three prose, two JSON) rather than one uniform shape. Mitigated by each tool's own
  description stating its shape and by `get_info()`'s disambiguating instructions.
- If a future MCP tool is added that queries `ArchModel` (or any other structured model),
  the precedent this ADR sets is: query tools over structured data return JSON; one-shot
  advisory/report tools return prose. Follow that split rather than re-litigating it per
  tool.
