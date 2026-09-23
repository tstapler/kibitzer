# Build vs. Buy: type-hierarchy-graph

Scope note (per task framing): "buy" here means *adopt a crate/tool that extracts
supertype/interface edges from a tree-sitter tree kibitzer already has parsed* — not
replace kibitzer's tree-sitter architecture. See
`project_plans/architecture-export/research/build-vs-buy.md` for the prior, closely
related decision this one follows the same shape as.

## Codebase context (verified in-repo)

- `src/symbol_extract.rs`'s `LangSymbolConfig`/`lang_symbol_config` (`src/symbol_extract.rs:42-66,
  181+`) is the exact table-driven, per-`Language` node-kind pattern the
  `architecture-export` build-vs-buy doc already validated for symbol extraction (itself
  modeled on `src/rules.rs`'s `LangRuleConfig`). It already has per-language `is_exported`,
  `name_finder`, interface-vs-type classification (e.g. Go's `type_spec` differentiated by
  its `type` field being `interface_type`, Kotlin's `interface` keyword found via
  `find_child_by_kind`) for Go/TypeScript/Tsx/JavaScript/Python/Java/Kotlin/Rust. Adding
  `extends`/`implements` extraction is additive to this same table, not a new mechanism.
- `Cargo.toml` confirms the dependency set is unchanged since the `architecture-export`
  decision: `tree-sitter = "0.26"` plus one grammar crate per language, no ctags/LSIF/SCIP/
  graph-DSL dependency. Nothing has changed that would revisit that prior verdict.
- `src/arch_model.rs`'s `resolve_call_edges`/`resolve_one_call_edge`/`resolve_in`
  (`src/arch_model.rs:455-558`) is a directly reusable two-pass pattern: (1) walk every file
  once, emit unresolved `RawCallSite`s with a caller id and callee text; (2) build a
  whole-repo `name -> [(package, id)]` index from already-extracted `SymbolNode`s, then
  resolve each raw site against it, preferring an unambiguous same-package match and
  falling back to a unique global match, else leaving `resolved: false`. This is precisely
  the "resolve an `extends`/`implements` target name to a `SymbolNode::id`" problem the
  requirements' Rabbit Holes section flags — confirms `resolve_call_edges`'s machinery is
  generic enough to reuse (a `name -> id` index keyed by package + a same-package-preferred
  lookup), addressing the Feasibility Risk the requirements raised about whether this reuse
  would actually work. `resolve_field_access_edges` (`src/arch_model.rs:565-587`) is the
  second, simpler precedent: drop-if-unresolved rather than flag-unresolved, the two
  competing conventions the requirements' Open Questions section is deciding between.
- `build.rs` + `src/node_kind.rs` generate typed `<Lang>Kind` enums (`GoKind`,
  `TypeScriptKind`, ...) from vendored `node-types.json` per grammar, so a `node.kind()`
  string-literal typo becomes a compile error instead of a silent non-match
  (`src/node_kind.rs:1-6`, `build.rs:1-13`). This materially changes the correctness-risk
  picture for hand-written per-language AST matching versus what the `architecture-export`
  build-vs-buy doc could assume when it was written — see Section 3. Note:
  `symbol_extract.rs` itself does not yet consume these typed enums (still raw `&'static
  str` kind tables) — the typed-enum migration is real infrastructure but not yet applied
  to this specific file (confirmed: no `node_kind`/`GoKind`/`TypeScriptKind` reference in
  `src/symbol_extract.rs`).

## 1. Existing OSS library/crate for extends/implements extraction over a tree-sitter tree

### Rust crates on crates.io

A crates.io/GitHub search for a crate that walks a tree-sitter tree and emits type-hierarchy
(`extends`/`implements`/struct-embedding) edges turned up nothing purpose-built. What
exists is either far narrower (single-purpose grammar/binding crates like
`tree-sitter-typescript`, `tree-sitter-java`) or far more generic (`tree-sitter-graph`, a
DSL for building arbitrary graphs from a tree-sitter tree, already evaluated and rejected
in the `architecture-export` doc as "a new authoring language and mental model layered on
top of tree-sitter queries," not a supertype/interface-extraction library). No crate
offers "give me the supertype list for this class_declaration node" for any of Go/TS/Java/
Kotlin. **Verdict: Not recommended** — nothing to adopt; the gap this feature fills isn't
commodified as a library.

### rust-analyzer / `ra_ap_rust-analyzer`

Published on crates.io as `ra_ap_rust-analyzer` (rust-analyzer's own crates re-published
under that prefix specifically so external tools can depend on them), with an `ide` crate
exposing supertype/trait-impl-aware APIs. **Cons, disqualifying**: it is a Rust-only
semantic-analysis engine (built on Rust's HIR/type inference) — it has no notion of Go
struct embedding, TS/Java `extends`/`implements`, or Kotlin supertype lists, and this
feature's in-scope languages explicitly exclude Rust (per requirements' Out of Scope:
"Rust — traits + impls are a different relationship shape... not mentioned in this item's
scope"). Embedding a multi-hundred-thousand-line Rust semantic-analysis engine to solve a
problem it doesn't address for any in-scope language is a non-starter. **Verdict: Not
recommended** — wrong problem domain (Rust-specific semantic analysis vs. multi-language
syntactic `extends`/`implements` extraction).

### universal-ctags (subprocess)

ctags has a documented `inherits` extension field (comma-separated base-class list) for
C++ and Java, confirmed via its own field docs. Its actual behavior for Go (which has no
`extends`/`implements` keyword — struct embedding is structural) and Kotlin is unconfirmed
by this search and plausibly thin, since ctags' per-language parsers are independently
hand-maintained C code with uneven feature depth across ~140 languages, unlike
tree-sitter's grammar-completeness guarantee. More fundamentally, the
`architecture-export` build-vs-buy doc's rejection of ctags stands unchanged here: kibitzer
already tree-sitter-parses every file once for `SymbolNode` extraction in the very same
walk this feature extends; shelling out to ctags a second time would double-parse, risk
disagreeing with kibitzer's own tree-sitter-based notion of "what's a type" (ctags' `struct`/
`interface`/`class` classification is independently tuned from tree-sitter's grammar), and
reintroduce an external-binary runtime dependency the single-binary/`cargo-dist` release
model avoids. Nothing about this specific feature (vs. the general symbol-extraction
problem the prior doc addressed) weakens any of those three objections — if anything the
Go case is worse, since ctags' `inherits` field is documented only for C++/Java, not Go's
struct-embedding shape this feature needs correctly. **Verdict: Not recommended** — same
reasoning as the prior decision, applies unchanged.

### stack-graphs (GitHub)

Confirmed via GitHub search: **archived by GitHub on 2025-09-09, now read-only** — same
finding the `architecture-export` doc already made, now a year further into being
unmaintained. Disqualified on maintenance grounds alone, independent of fit.
**Verdict: Not recommended.**

### tree-sitter's own `Query`/`QueryCursor` API

Distinct from an external crate: `tree-sitter` itself (already a dependency) ships a
declarative S-expression query sublanguage (`tree-sitter::Query`/`QueryCursor`) as an
alternative to manual `Node` walking. Confirmed via `grep` that kibitzer's checkers
(`rules.rs`, `symbol_extract.rs`, etc.) do not currently use it anywhere — every existing
checker hand-walks `Node` children via `child_by_field_name`/`children()`. Using
`.scm`-style queries for `extends`/`implements` clauses (e.g. a query pattern matching
`(class_declaration (class_heritage (extends_clause value: (identifier) @super)))` for TS)
is a real, zero-new-dependency option since the API ships in the `tree-sitter` crate
already in `Cargo.toml`. **Pros**: declarative capture-based matching is arguably easier to
verify correct for a "list all direct children of this shape" query than a hand-walked
`child_by_field_name` chain, and tree-sitter's own docs recommend queries for exactly this
kind of "does this subtree match a pattern" extraction. **Cons**: introducing the Query API
here would be the first use of it in the codebase, fragmenting kibitzer's tree-sitter usage
into two competing idioms (manual walk vs. compiled query) for no clear correctness gain
over the existing, already-proven, already-tested manual-walk pattern every other
`LangSymbolConfig` field uses — the same "don't introduce a second way to consume
tree-sitter trees" objection the `architecture-export` doc raised against `tree-sitter-graph`
applies here at a smaller scale. **Verdict: Viable but not recommended** — technically
available at zero new dependency cost, but extending the existing manual-walk convention
(matching how `type_kinds`/`interface_kinds`/`function_kinds` already work) keeps one
idiom in the file rather than two, and this feature's per-language shapes (Go embedded
fields, TS/Java heritage clauses, Kotlin supertype lists) are simple enough that a query
sublanguage's main advantage — matching complex, deeply-nested shapes — isn't needed.

## 2. SaaS/managed API

Same conclusion as the `architecture-export` doc, re-affirmed rather than re-derived:
kibitzer's offline/self-contained/single-binary value proposition (CLAUDE.md, and every
existing `Checker`/MCP tool running in-process against local files) rules out a hosted
code-intelligence API (Sourcegraph Cloud, CodeSee, Swimm, etc.) for the same reasons —
network dependency at check/index time, third-party source upload, subscription cost, and
a human-browsable-UI interaction model that doesn't fit kibitzer's CLI/MCP-first,
agent-queryable design. Nothing about type-hierarchy extraction specifically changes that
calculus — if anything it's an even smaller, more purely-syntactic problem (reading a
supertype list off a parse tree) than the general symbol-indexing problem SaaS platforms
solve, making a hosted dependency even less justified for it. **Verdict: Not recommended.**

## 3. LLM-generated / hand-written per-language matching vs. a tested pattern library

The requirements frame this correctly: "extends/implements extraction" is tree-sitter AST
pattern matching, not a general-purpose data structure (hash map, sort) where reinventing
one is textbook wasted/risky effort. The relevant question is whether kibitzer's existing
in-house pattern for this exact shape of problem is trustworthy enough to extend, or
whether the per-language divergence (4 languages: Go, TS/Tsx/JS, Java, Kotlin) is risky
enough to warrant an external, pre-tested library instead.

**Evidence this is the right level of "custom code," not excessive:**
- `symbol_extract.rs`'s existing `LangSymbolConfig` table already solves a structurally
  identical problem — per-language node-kind/field lookups for type/interface
  classification — for the same 4 (plus more) languages, and its test suite
  (`lang_symbol_config_go_type_kinds_and_export_detection`,
  `go_struct_fields_skips_embedded_anonymous_fields`, etc., `src/symbol_extract.rs:1059,
  1513`) is direct proof the pattern catches real per-language AST divergence correctly
  today. Adding `extends_kinds`/`implements_kinds`-equivalent fields (or a
  `supertype_finder: fn(Node) -> Vec<(String, RelationKind)>` per language) to that same
  table is incremental, not a new architecture — matching the `architecture-export` doc's
  own conclusion for the symbol-extraction problem this feature builds on.
- `build.rs`/`node_kind.rs`'s generated typed `<Lang>Kind` enums are a mitigation the prior
  `architecture-export` decision didn't have available to weigh: a hand-written per-language
  match against `GoKind::TypeSpec`/`TypeScriptKind::ExtendsClause` fails to compile on a
  typo'd variant, versus a raw string literal that just silently never matches. If the new
  `extends`/`implements` code in `symbol_extract.rs` adopts these typed enums (it doesn't
  yet — see Codebase context above), the main correctness risk this section is assessing —
  "did I get the node-kind string exactly right for all 4 grammars" — moves from a runtime/
  test-only guarantee to a compile-time one. This is a concrete, low-cost risk reducer
  Phase 3 should adopt regardless of the build-vs-buy conclusion.
- The specific new AST shapes needed are individually simple, not exotic: TS/JS
  `class_heritage` → `extends_clause`/`implements_clause` children with `value`/`type`
  fields; Java `superclass`/`super_interfaces` fields on `class_declaration`; Kotlin a
  `:`-delimited supertype list on `class_declaration` (per requirements' Rabbit Holes note,
  needing a symbol-table check to split extends-vs-implements since the grammar doesn't);
  Go embedded fields inside `field_declaration_list` with no explicit `name` field. None of
  these require anything beyond `child_by_field_name`/positional-child lookups the codebase
  already does throughout `symbol_extract.rs` and `rules.rs`.

**Where the real risk actually lives (confirmed against the requirements, not invented
here):** not the AST-matching mechanics, but the *semantic* disambiguation the
requirements' own Rabbit Holes section already flags — Go's identical syntax for embedding
a struct (`extends`) vs. an interface (`implements`), and Kotlin's single supertype list
not syntactically distinguishing the two — both need a symbol-table lookup against
already-extracted `Type`/`Interface` `SymbolNode`s, not a fancier AST-matching library. No
external crate/tool found in this research addresses that disambiguation either — it's
inherent to how these two languages' grammars work, so no "buy" option removes it.

**Verdict: Hand-writing per-language `extends`/`implements` matching, extending
`LangSymbolConfig`, is the right level of custom code — Recommended.** It's the same
conclusion the `architecture-export` doc reached for symbol extraction generally, it's
additive to a table already proven correct by 40+ existing tests across these grammars,
and the one real correctness hazard (Go/Kotlin embed-vs-implements disambiguation) is a
resolution-pass problem solvable with the existing `resolve_call_edges`-style machinery,
not an AST-parsing-correctness problem any external library would help with.

## 4. Fork or adapt an existing project

Re-checked against the `architecture-export` doc's own fork-candidate survey (`tree-sitter-graph`,
`stack-graphs`, `scip-ctags`) plus a fresh look for anything narrower and closer to this
feature's specific ask (a tree-sitter-based tool that already extracts and exposes
supertype/interface edges as queryable data, not just renders a diagram):

- No new candidate surfaced. The nearest-adjacent category — tree-sitter-based
  "architecture"/dependency-graph tools (e.g. various `tree-sitter`-powered call-graph or
  import-graph generators found in general search) — either don't model type hierarchy at
  all (they're import/call-graph-only, same gap kibitzer itself had before
  `field_accesses`/this feature) or are early-stage/single-language hobby projects not
  meaningfully more mature or better-licensed than writing the ~4-language extension
  directly against kibitzer's own proven pattern.
- `stack-graphs` remains archived (confirmed above) and was the one project in this space
  close enough in architecture (tree-sitter-based, cross-file name resolution — exactly
  the "resolve an extends target to a SymbolNode id" problem) to have been worth adapting
  from; its archival forecloses that option same as it did for `architecture-export`.

**Verdict: Not recommended.** Nothing found is both closely enough scoped and actively
maintained; the one closest-fit candidate (`stack-graphs`) is unmaintained.

## Overall recommendation

**Build natively, extending `symbol_extract.rs`'s existing `LangSymbolConfig` pattern and
`arch_model.rs`'s existing `resolve_call_edges`-style two-pass resolution machinery.** No
option in any of the four categories changes the `architecture-export` doc's precedent:

1. **No crate/library to adopt** — nothing on crates.io does tree-sitter-tree-to-type-hierarchy
   extraction for these languages; `rust-analyzer` is real but wrong-language;
   `tree-sitter-graph` is available but a worse-fit idiom than the codebase's proven manual
   walk; `stack-graphs` is archived.
2. **ctags subprocess**: rejected on the same double-parse/consistency/external-dependency
   grounds as before, made slightly worse here since ctags' documented `inherits` field
   coverage (C++/Java) doesn't clearly extend to Go's embedding shape this feature needs.
3. **SaaS**: excluded outright, unchanged reasoning.
4. **Hand-written per-language extraction is the right scope of "custom code,"** not a
   symptom of avoidable reinvention — it's additive to an already-tested table-driven
   pattern, and the one genuine risk (Go/Kotlin extends-vs-implements disambiguation) is a
   resolution problem the codebase already has a proven, reusable answer for
   (`resolve_call_edges`'s name-index-and-resolve two-pass shape), not an AST-parsing
   problem any external tool would derisk further.
5. **New, concrete mitigation this research surfaces beyond the prior decision**: adopt the
   already-existing-but-unused `node_kind.rs` typed `<Lang>Kind` enums for the new
   `extends`/`implements` node-kind matches, converting the main correctness risk (typo'd
   node-kind string) from a test-only guarantee into a compile-time one, at zero new
   dependency cost.

## Sources

- [ra_ap_rust-analyzer — crates.io](https://crates.io/crates/ra_ap_rust-analyzer)
- [rust-analyzer `ide` crate docs](https://rust-lang.github.io/rust-analyzer/ide/)
- [github/stack-graphs — GitHub (archived 2025-09-09, confirmed read-only)](https://github.com/github/stack-graphs)
- [github/stack-graphs releases](https://github.com/github/stack-graphs/releases)
- [Universal Ctags — extension fields documentation (`inherits` field)](https://docs.ctags.io/en/latest/man/ctags.1.html)
- [tree-sitter-graph — GitHub](https://github.com/tree-sitter/tree-sitter-graph)
- `project_plans/architecture-export/research/build-vs-buy.md` — prior, closely related
  decision this research follows and re-validates rather than re-derives from scratch.
- In-repo: `src/symbol_extract.rs`, `src/arch_model.rs`, `src/node_kind.rs`, `build.rs`,
  `Cargo.toml` (all read directly for this research, not inferred).
