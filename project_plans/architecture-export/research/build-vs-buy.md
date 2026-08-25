# Build vs. Buy: architecture-export

## Codebase context (verified in-repo)

- `src/checker.rs` already loads tree-sitter grammars for all 7 in-scope languages
  (`Language::{Go,TypeScript,Tsx,JavaScript,Python,Java,Kotlin}`) through a
  `GrammarCache` that parses each language at most once per cache instance and hands
  callers a shared `Tree` (`src/checker.rs:146-179`). `src/rules.rs`'s
  `SyntaxRulesChecker` already does per-language AST dispatch over that same cache via a
  `lang_config(Language) -> LangRuleConfig` match with per-language node-kind tables and
  walk functions (`src/rules.rs:175-320` and the `walk_declarations`/`check_declaration`
  walkers at `src/rules.rs:353-404`), and its 40+ tests (`src/rules.rs:487-900+`) already
  exercise node-kind handling across all 7 grammars.
- `src/mermaid.rs` hand-rolls a `graph TD` renderer against `ImportGraph`
  (`src/import_graph.rs`) with a 150-node cap (`MAX_NODES`, `src/mermaid.rs:12`) and
  cycle highlighting — no existing dependency on Mermaid's C4 syntax or any diagramming
  library.
- `src/mcp.rs`'s `architecture_assessment` tool (`src/mcp.rs:130`) is transient: it
  re-runs `import-cycles`/`layering`/`coupling` over a freshly built `ImportGraph` on
  every call and returns text + optional Mermaid diagram — no persisted model.
- `src/lsp.rs` currently implements only diagnostics (`diagnostics_for_file`,
  `src/lsp.rs:69-80`) via `tower-lsp`; no `document_symbol`/`workspace_symbol` handlers
  exist yet.
- `Cargo.toml` pins `tree-sitter = "0.26"` plus one crate per grammar
  (`tree-sitter-go`, `-typescript`, `-javascript`, `-python`, `-java`,
  `tree-sitter-kotlin-ng`) — no ctags, LSIF, SCIP, or graph-DSL dependency exists today.

## 1. Existing OSS library/tool for symbol extraction + architecture modeling

### universal-ctags (shell out for symbol extraction)

**Pros**: Battle-tested, wide language coverage (~140 languages via `--list-languages`),
zero new Rust dependency — just a subprocess call and tag-file parsing.

**Cons**: kibitzer would be re-parsing every file a second time in a second process
(ctags' own C parser) after already tree-sitter-parsing it in-process for
`SyntaxRulesChecker`/other checkers via `GrammarCache`. That's strictly worse than reuse
on three axes: (a) performance — double parse per file; (b) consistency — ctags' notion
of "what counts as a symbol" per language is independently tuned from tree-sitter's
grammar and can disagree with what kibitzer's own AST-walk-based checks already treat as
a declaration; (c) new runtime dependency — kibitzer is a single static-ish Rust binary
today (`Brewfile`/`cargo-dist` release model), and shelling out to a system `ctags`
binary reintroduces an external-tool dependency the rest of the codebase avoids. Search
confirms even Sourcegraph — heavy ctags users historically — now prefers building
tree-sitter-based indexers (`scip-ctags`) over extending universal-ctags itself for
languages with good tree-sitter grammars, which is exactly kibitzer's situation for all
7 in-scope languages.

**Verdict**: **Not recommended.** kibitzer already has the parse; shelling out to a
second, less-integrated parser to get roughly the same information is strictly worse
than reusing `GrammarCache`.

### LSIF (Language Server Index Format)

**Pros**: Real open standard for "structured, queryable code index," originally
designed for exactly this precomputed-navigation use case.

**Cons**: LSIF is legacy even at its own steward. Sourcegraph fully deprecated LSIF
ingestion in favor of SCIP as of server version 4.6 (support for reading LSIF-encoded
data was removed; the migration path is one-way/destructive) — see Sourcegraph's own
"SCIP - a better code indexing format than LSIF" announcement and migration docs. LSIF's
opaque-integer-ID graph model (edges/vertices keyed by numeric IDs) is also harder to
grep/jq by hand than a plain nested JSON tree, which cuts against the requirement's
"greppable/jq-able artifact" success metric.

**Verdict**: **Not recommended.** Its own creator has moved past it; adopting it now
means targeting a format the ecosystem is leaving.

### SCIP (Sourcegraph Code Intelligence Protocol)

**Pros**: LSIF's actively-maintained successor — human-readable string-based symbol IDs
(vs. LSIF's opaque integers), a documented Protobuf schema (`scip.proto`), and an
existing, maintained `scip` crate on crates.io with Rust bindings generated from that
schema, which is a real "adopt this as an output format" option without hand-rolling a
schema. SCIP is optimized for code-navigation queries (definitions/references), which
overlaps with, but is narrower than, this feature's C4-like architecture-level modeling
goal.

**Cons**: SCIP is purpose-built for symbol-reference indexing (go-to-def/find-refs), not
for package/module → component-level architecture modeling with C4-like layering. Using
it as kibitzer's *primary* internal model would mean bending a reference-graph format to
represent import-graph/layering/coupling concepts it wasn't designed for. It's a
plausible secondary *export* format, not a foundation to build the whole model on.

**Verdict**: **Viable, but only as an optional secondary output format**, not the
primary data model — the requirements' own JSON-tree/C4-like ask doesn't map onto SCIP's
symbol-index shape well enough to be the source of truth.

### Structurizr (DSL / Lite)

**Pros**: The actual reference implementation of the C4 model, created by C4's author —
best available option for genuine C4 semantics and compatibility if literal C4
compliance mattered. Free to use as a CLI/DSL; only the multi-user *server* offering is
licensed. Structurizr Lite ships as a self-contained Docker image with an embedded JVM +
Tomcat, so no separate Java install is needed to *view* a workspace.

**Cons**: Two dealbreakers for kibitzer specifically. First, emitting Structurizr DSL
requires a fundamentally different data shape than this feature's — Structurizr models
Person/SoftwareSystem/Container/Component *relationships* at the C4 Context/Container
level, i.e., precisely the levels this feature's Scope explicitly excludes ("True C4
Container/Context levels" is Out of Scope). What kibitzer needs is Component/Code-level
detail (types, functions) that Structurizr's DSL doesn't natively grow into a symbol-
level tree — it's not "just emit DSL and get code-level detail for free." Second,
*viewing* the rendered result requires standing up the Structurizr Lite Docker
container (JVM inside), which directly contradicts CLAUDE.md's stated value prop of
kibitzer being self-contained/offline/no external service — a `kibitzer export` that
requires `docker run structurizr/lite` to be useful is a meaningfully heavier
dependency than kibitzer's current single-binary distribution via Homebrew/cargo-dist.
Also worth noting: Structurizr Lite itself is being sunset in favor of a new "Structurizr
local" product per the vendor's own recent announcement, i.e., even adopting it now
means targeting a tool mid-transition.

**Verdict**: **Not recommended** as a target format or embedded renderer. Could be
mentioned in docs as "you can hand-translate the exported JSON into Structurizr DSL
yourself if you want C4-Context-level views," but kibitzer should not build or ship an
integration.

### Mermaid C4 diagram types (`C4Context`/`C4Container`/`C4Component`)

**Pros**: Mermaid confirms 5 C4 diagram types exist (`C4Context`, `C4Container`,
`C4Component`, `C4Dynamic`, `C4Deployment`), syntactically PlantUML-C4-compatible.
kibitzer already renders Mermaid text in `src/mermaid.rs` and in `architecture_assessment`
(`src/mcp.rs`) — Mermaid is already the house diagram format, zero new rendering
dependency (it's just string templating, same as today's `graph TD` output), and it
renders natively in GitHub/most Markdown viewers and Claude artifacts, matching how
kibitzer's other Mermaid output is already consumed.

**Cons**: Mermaid's own docs still mark C4 support as experimental (a first-party
*plugin* extending core syntax, not a fully stabilized core diagram type) — worth a
compatibility check against whatever Mermaid version the consuming renderers pin, but
not a blocker since kibitzer only emits text.

**Verdict**: **Recommended** as the diagram *renderer* for the Component/Code-level
requirement — extend `src/mermaid.rs` with a `C4Component`/`C4Dynamic` emitter that
reads from the new structured model, the same way `render_dependency_graph` already
reads from `ImportGraph`. This directly satisfies "C4-*like* diagram generation" without
adopting Structurizr or hand-rolling a bespoke notation.

## 2. SaaS/managed API (Sourcegraph, CodeSee, Swimm, etc.)

**Verdict: Not recommended**, and explicitly so rather than skipped:

- kibitzer's whole value proposition (per `CLAUDE.md` and its existing architecture) is
  a local, offline, single-binary advisory tool that runs in CI/local dev/agent hooks
  without phoning home. `architecture_assessment` and every existing checker in
  `src/checker.rs`'s registry run entirely in-process against files on disk.
- A hosted architecture-visualization SaaS (Sourcegraph Cloud, CodeSee, Swimm) requires
  uploading source or index data to a third party, network access at check time (or at
  minimum at export time), and typically a paid subscription — all in direct tension
  with "self-contained, offline, no external service."
- These tools also solve a different problem: they're built for human-browsable web UIs
  over a *hosted* index, not for a CLI-first, greppable/jq-able local artifact an agent
  can read via MCP mid-session. Even setting aside the offline requirement, the
  interaction model doesn't fit.

## 3. LLM-generated implementation vs. reusing kibitzer's existing `GrammarCache` +
   per-language dispatch pattern

**The existing pattern (`src/rules.rs`) is directly reusable, not just analogous.**
`SyntaxRulesChecker::check` (`src/rules.rs:342`) already: (a) receives a `CheckContext`
with a parsed `Tree` for its declared `Language` from `GrammarCache`; (b) looks up a
per-language `LangRuleConfig` via `lang_config(Language)` that supplies language-specific
node-kind strings (function/method/class declaration kinds, parameter-list kinds, body
kinds — e.g. compare the Go table at `src/rules.rs:177` against the Kotlin table at
`src/rules.rs:280`); (c) walks the tree generically over that config
(`walk_declarations`, `check_declaration`). Extracting symbols (types, interfaces,
exported functions, their names and spans) is structurally the same problem — "walk
declarations of interest per language, using a per-language table of node kinds" — just
extracting a name+kind+span instead of counting lines/params/nesting-depth. The 40+
existing tests in `src/rules.rs` are proof this walk pattern already handles real
divergence across all 7 grammars correctly (e.g. Kotlin's different param/body node
shapes at `src/rules.rs:149-175` are already isolated behind the `LangRuleConfig`
abstraction).

**Pros of extending the existing pattern**: no new correctness risk class introduced;
reuses parsed trees kibitzer already builds (via the same `GrammarCache` instance per
file, so symbol extraction and rule-checking can share one parse); reuses an
already-tested per-language node-kind table instead of hand-authoring 7 new ones from
scratch; consistent with kibitzer's existing architecture (`Checker` trait, `Language`
enum) so a future contributor finds symbol extraction where they'd expect it.

**Cons**: `LangRuleConfig` as it exists today is scoped to function/method-level
constructs (bodies, params, nesting) — it will need new fields for type/interface/
exported-symbol node kinds (struct/class/interface/type-alias declarations, export
modifiers) per language, which is real per-language work, not free. But that work is
additive to a proven table-driven shape, not a new architecture.

**Verdict**: **Extending the existing `GrammarCache` + per-language node-kind table
pattern is the right call.** Writing new, independent AST-walking logic from scratch
(whether hand-written fresh or LLM-generated fresh) would re-litigate the exact
per-language node-kind divergence problem `src/rules.rs` already solved and tested,
for no benefit — it's the textbook case the Pitfalls research should flag: bespoke
"7 similar-but-different per-language cases" logic is where subtle bugs (missing a
node kind, wrong field name for one grammar) hide, and kibitzer already has a tested
answer for that shape of problem in this exact codebase.

## 4. Fork or adapt an existing tool

- **tree-sitter-graph**: A real Rust crate/DSL for "construct graphs from parsed
  source code" via tree-sitter — closest conceptual fit of anything found. But it's a
  generic graph-construction *DSL* (you write `.tsg` query-like rules to build arbitrary
  graphs), which is a new authoring language and mental model layered on top of
  tree-sitter queries — not clearly less work than kibitzer's existing native
  `Node`-walking approach in `src/rules.rs`, and it would be a second way to consume
  tree-sitter trees alongside the existing walkers, fragmenting the codebase's
  tree-sitter usage pattern rather than reusing it.
- **stack-graphs** (GitHub's related project, uses tree-sitter-graph under the hood for
  name resolution): confirmed **archived by GitHub on 2025-09-09, now read-only** — a
  disqualifying signal for a new dependency; adopting an archived project means
  inheriting unmaintained code with no path to fixes.
- **scip-ctags / SCIP indexers** (Sourcegraph): purpose-built for reference indexing,
  not architecture/C4 modeling — same mismatch as SCIP itself in section 1.
- No tree-sitter-based "codebase architecture mapper" close enough to kibitzer's
  specific ask (JSON tree spanning package→symbol levels, C4-like diagram output, MCP
  query tools, LSP symbols) turned up in search; the nearest hits are narrower
  (reference-indexing) or broader/heavier (Structurizr, full SaaS platforms) than what's
  needed, or unmaintained (stack-graphs).

**Verdict**: **Not recommended.** Nothing found is both closely enough scoped to fork
and actively maintained. `tree-sitter-graph` is live but a poor architectural fit
(new DSL vs. reusing existing native walkers); `stack-graphs` is archived.

## Overall recommendation

**Build natively on kibitzer's existing tree-sitter infrastructure — this is the
conclusion the research supports, not just the framing it started from.** Concretely:

1. **Symbol/architecture model**: extend `src/checker.rs`'s `GrammarCache` +
   `src/rules.rs`'s per-language `LangRuleConfig` pattern (add type/interface/exported-
   function node-kind fields) rather than shelling out to ctags, adopting LSIF, or
   authoring net-new AST-walking logic. This is the one part of the research that came
   back unambiguous: kibitzer already solved "walk 7 divergent grammars correctly," and
   nothing external does that walk *for kibitzer's own already-parsed trees* — every
   external option either re-parses (ctags) or solves an adjacent, narrower problem
   (SCIP/LSIF's reference-indexing, tree-sitter-graph's generic-graph DSL).
2. **Primary artifact format**: bespoke JSON tree (per the requirements), not LSIF (dead)
   or SCIP (wrong shape for architecture/C4 levels) — but expose the `scip` crate as a
   *possible* secondary export target later if symbol-reference-style queries
   (go-to-def-like) become a wanted use case; that's a separable, additive decision, not
   a blocker for the core model.
3. **Diagram output**: extend `src/mermaid.rs` with Mermaid's `C4Component`/`C4Dynamic`
   syntax rather than embedding/depending on Structurizr — keeps kibitzer single-binary
   and offline, reuses the existing Mermaid-emission pattern and consumer expectations
   (GitHub/agent renderers already handle Mermaid from `architecture_assessment` today).
4. **SaaS**: excluded outright — contradicts kibitzer's offline/self-contained design
   center, confirmed by re-reading `CLAUDE.md` and the existing all-in-process
   `Checker`/MCP architecture.
5. **Fork candidates**: none adopted — `tree-sitter-graph` is maintained but a worse fit
   than kibitzer's own pattern; `stack-graphs` is archived (2025-09-09) and disqualified
   on maintenance grounds alone.

## Sources

- [universal-ctags/ctags — RFC: Which language do you want us to support?](https://github.com/universal-ctags/ctags/discussions/2931)
- [universal-ctags/ctags GitHub](https://github.com/universal-ctags/ctags)
- [Sourcegraph — Migrating code intelligence data from LSIF to SCIP](https://sourcegraph.com/docs/admin/how-to/lsif-scip-migration)
- [Sourcegraph blog — SCIP: a better code indexing format than LSIF](https://sourcegraph.com/blog/announcing-scip)
- [scip crate — crates.io](https://crates.io/crates/scip)
- [sourcegraph/scip — scip.proto](https://github.com/sourcegraph/scip/blob/main/scip.proto)
- [Structurizr — Why "as code"?](https://docs.structurizr.com/as-code)
- [Structurizr DSL docs](https://docs.structurizr.com/dsl)
- [Structurizr Lite docs](https://docs.structurizr.com/lite)
- [Structurizr Lite Docker image](https://hub.docker.com/r/structurizr/lite)
- [Introducing Structurizr vNext (Patreon)](https://www.patreon.com/Structurizr/posts/introducing-146923136)
- [Mermaid — C4 Diagrams docs](https://mermaid.js.org/syntax/c4.html)
- [mermaid-js/mermaid — c4.md source](https://github.com/mermaid-js/mermaid/blob/develop/packages/mermaid/src/docs/syntax/c4.md)
- [tree-sitter/tree-sitter-graph](https://github.com/tree-sitter/tree-sitter-graph)
- [github/stack-graphs releases](https://github.com/github/stack-graphs/releases)
- [GitHub blog — Introducing stack graphs](https://github.blog/open-source/introducing-stack-graphs/)
