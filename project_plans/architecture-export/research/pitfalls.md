# Research: Pitfalls — architecture-export

Scope: what commonly goes wrong building a multi-language, static-analysis-derived
architecture model exported to CLI/MCP/LSP/diagram consumers, grounded in what kibitzer's
existing checkers (`src/rules.rs`, `src/checker.rs`), caching (`src/daemon.rs`,
`src/cache.rs`), import graph (`src/import_graph.rs`), MCP tool (`src/mcp.rs`), and LSP
server (`src/lsp.rs`) already do or don't do.

## 1. Tree-sitter grammar drift — what did and didn't generalize in `src/rules.rs`

`src/rules.rs`'s `LangRuleConfig` (lines 54–313) is the existing precedent for solving
"one concept, seven grammars." It generalized well for three rules (long-function,
deep-nesting, long-parameter-list) but only by admitting a lot of per-language escape
hatches — every one of these is a preview of a pitfall symbol-extraction will hit harder,
since symbols have far more shape variety than "a function body and its params":

- **Node kind names never unify.** `if_kind` is `"if_statement"` everywhere except
  Kotlin's `"if_expression"` (line 297). There is no canonical "if-node" kind across
  grammars — the table hardcodes one string per language and a comment explaining why.
  Expect the same for "type declaration": Go's `type_declaration` wraps a `type_spec`
  that can itself be a struct *or* an alias *or* an interface; TS's `interface_declaration`
  is a top-level kind; Python's `class_definition` and Kotlin's `class_declaration` are
  each their own kind. There is no way around a per-language table for "what counts as a
  type" — confirmed by the fact this table exists at all instead of a shared traversal.
- **Field-based access breaks silently for one language.** `field_body`/`field_params`
  (lines 95–101) use `child_by_field_name`, which works for Go/TS/JS/Python/Java — but
  Kotlin's `function_declaration`/`anonymous_function` "expose no field names at all"
  (line 157), forcing `kotlin_body`/`kotlin_params` to fall back to positional
  `named_children().find(|c| c.kind() == "...")`. This was caught only because someone
  did "an explicit `field_name_for_child` dump" (line 158) — not something `to_sexp()`
  alone reveals. **Design implication**: assume at least one of the 7 target languages
  will silently lack field names for some node the model needs, and budget verification
  time (dump `field_name_for_child`, don't guess from grammar docs) per language, not
  just per feature.
- **"Same" node kind, different children shape.** `js_ts_param_count` vs. `py_param_count`
  vs. `kotlin_param_count` (lines 122–155) all count "named children of the params node"
  but each needs its own filter: JS/TS is "just count them," Python must exclude
  `positional_separator`/`keyword_separator` marker nodes (bare `/` and `*`), Kotlin must
  exclude a sibling `parameter_modifiers` node. Go doesn't even have one-parameter-per-
  child — `parameter_declaration` can group multiple names (`func f(a, b int)`), so
  `go_param_identifier_count` needs a nested field-count fallback (`.max(1)`, line 117).
  **This is the single strongest signal that a shared symbol model cannot walk a generic
  tree-sitter query across languages** — every language needs its own hand-verified
  extraction function, and reusing a Query-based (`tree-sitter query language`) approach
  will hit the same divergence, just moved into `.scm` query files instead of Rust match
  arms.
- **Chaining/nesting semantics diverge structurally, not just lexically.** Go's `else if`
  nests a bare `if_statement` directly under `alternative`; JS/TS wrap it in an
  `else_clause` first (comment, lines 56–58); Python's `elif` is a wholly distinct
  `elif_clause` node with its own condition/consequence/alternative fields (chain_kinds,
  line 253); Kotlin chains via a nested `if_expression` with only `condition` as a named
  field, others positional (lines 291–296). For architecture-export this maps to: a TS
  `export { X } from './y'` re-export is structurally nothing like a Go `import`, and a
  Java package/file-path binding is nothing like Python's relative-import dot-counting —
  **there is no "generic import" abstraction that survives contact with all 7 grammars**;
  expect `import_graph.rs`'s current per-language `build_go`/`build_js` split (it already
  admits Python/Kotlin/Java aren't done, line 35) to become 5 more fully bespoke modules,
  not 5 small deltas on a shared walker.
- **What stayed simple**: the registry-level dispatch (`checker::lookup`, `name()`
  distinguishing entries) and the `RuleMeta`/file_globs layer generalized fine — the
  *policy* layer (which checker runs on which files, severity, IDs) is language-agnostic.
  It's only the *AST-walk* layer that fragments. Design the shared model with that same
  split: a language-agnostic `Symbol`/`Module` output schema, fed by fully per-language
  extraction functions — don't try to make the extraction itself generic.

## 2. Performance — caching granularity in `daemon.rs`/`cache.rs` doesn't cover this

- **`daemon.rs`'s cache is per-file, per-trigger, keyed by mtime fingerprint**
  (`src/cache.rs`, `key(file_path)` at line 143, `FileFingerprint{mtime_secs,
  mtime_nanos}` at lines 16–17 — "without hashing file contents"). `Cache::get`/`put`
  (lines 71, 90) store one `CheckResult` list per `(file_path, trigger)`. This
  granularity **does not generalize to a whole-repo model**: today's checks are already
  file-scoped (one file in, findings for that file out), so per-file caching is a natural
  fit. An architecture model is inherently repo-scoped (symbols reference each other
  across files; the import graph needs global node/edge state) — caching it per-file
  would require either (a) one cache entry per file plus a separate invalidation/merge
  step to reassemble the whole-repo view, or (b) a single repo-level cache entry that
  gets invalidated by *any* file change, defeating incrementality. Neither is what
  `Cache` does today; this is new cache-design work, not a reuse of `daemon.rs`.
- **`src/lsp.rs`'s `Backend` does not use the daemon cache at all.** `diagnostics_for_file`
  (line 71) calls `run_checks_for_trigger` directly — it never goes through
  `daemon::run_checks_smart`/`try_run_checks_via_daemon`. Every `did_open`/`did_change`/
  `did_save` (lines 136–146) triggers a full re-run of every in-scope check, re-reading
  and re-parsing the file from disk each time (comment at lines 90–95 notes this already:
  diagnostics reflect "last-saved content, not unsaved keystrokes," and wiring in the live
  buffer is "real future work, called out in issue #11, not done here"). **Confirmed
  pitfall**: if `workspace/symbol` or `textDocument/documentSymbol` is bolted onto this
  same `Backend` the naive way, a workspace-symbol query would either (a) re-walk every
  file in the repo synchronously per query — unacceptable on a large repo, especially
  since some LSP clients fire `workspace/symbol` on every keystroke of the picker — or (b)
  require introducing the model's own persistent, incrementally-updated symbol index that
  `did_save` amends rather than rebuilds. This is real new infrastructure, not something
  `daemon.rs`'s existing per-file cache absorbs for free.
- **No streaming/background indexing exists anywhere in the codebase today** — `run_lsp_server`
  (line 155) is a straight `tower_lsp::Server::new(...).serve(...)` with no background
  task, no debounce, no async indexer. Any workspace-symbol support needs that machinery
  built from scratch.

## 3. Output-size/usability — precedent for thresholds and exclusions

- **`duplicate_code.rs` precedent**: `MIN_BLOCK_LINES = 6`, `MIN_BLOCK_CHARS = 60`,
  `MIN_OCCURRENCES = 3` (lines 10, 13, 18) — three independent numeric floors, tuned
  together, that exist specifically to suppress noise (a prior commit,
  `c3e6719 Require 3+ occurrences before flagging duplicate-code blocks`, shows
  `MIN_OCCURRENCES` was raised after the 2-occurrence version was too noisy in practice).
  **Precedent for architecture-export**: expect the first cut of symbol-tree pruning to
  be too noisy and need at least one comparable knob tuned against real-repo output, not
  just unit tests — e.g. a minimum "exported-ness" bar (skip unexported/private symbols
  by default), possibly a minimum symbol count per file/package before it's worth a node.
- **`check.rs`'s `SKIP_DIRS`** (lines 1078–1086: `.git`, `node_modules`, `vendor`,
  `target`, `dist`, `build`, `.next`) is the existing, simple mechanism for keeping
  generated/vendored code out of batch scans — a flat hardcoded directory-name list, not
  a configurable glob. It's reused implicitly by anything that calls `walk()`/
  `walk_and_collect_files`. **Gap for architecture-export**: this list is Node/Go/JS-
  biased (no `__pycache__`, `.venv`, `target` is shared Rust/Java-ish but not
  Maven/Gradle's actual `build/` distinctions, no Kotlin/Java equivalents like `.gradle`,
  no Python `*.egg-info`). If the export command reuses `SKIP_DIRS` as-is, expect
  Python/Java/Kotlin repos to leak build artifacts and generated code into the model
  unless the list is extended per the new languages in scope.
- **`mcp.rs`'s existing 150-node Mermaid cap** (comment at line 51: "Repos over 150 nodes
  fall back to a text note instead — pass a narrower `scope` to render a subgraph") is
  the direct precedent for the diagram half of this feature's scope; see §5 below for
  what the *queryable-tree* half needs that this cap doesn't address (it only gates
  diagram rendering, not the underlying `ImportGraph`, which has no size limit at all —
  `graph.nodes`/`graph.edges` are unbounded `BTreeSet`/`Vec`).
- **No existing "minimized tree" concept anywhere in the codebase** — nothing in
  `rules.rs`/`duplicate_code.rs`/`import_graph.rs` produces a pruned *tree*; they all
  produce flat finding lists or a flat graph. "Minimized UML-like tree" pruning rules
  (what requirements.md flags as a rabbit hole) are genuinely undesigned work with zero
  reusable precedent in this repo — budget it as net-new design, not adaptation.

## 4. Correctness pitfalls in import/symbol extraction

Confirmed from `import_graph.rs`:

- **String-literal false positives are a real, already-mitigated risk for JS/TS.**
  `collect_js_imports` (lines 167–178) only matches `import_statement`/
  `export_statement` nodes with a `source` field — i.e., it walks the AST, not text/regex,
  so a string literal that merely *looks like* a path (e.g. `const x = "./foo"` outside an
  import) is correctly not picked up. This confirms AST-walking (not text scanning) is the
  right approach for the new model too — but re-export/barrel handling is only partially
  solved: `collect_js_imports` treats `export_statement` with a `source` field (i.e.
  `export { x } from './y'`) as an edge, which is correct for direct re-exports, but there
  is **no special handling of barrel files** (`index.ts` aggregating and re-exporting many
  submodules) — a symbol "exported through" a barrel will resolve to the barrel's
  directory as an edge target, not to the original defining file, unless the new model
  explicitly follows re-export chains to their origin. This is unbuilt, not solved.
- **Bare/package specifiers are already explicitly filtered out**, not silently mis-
  resolved: `build_js` (line 242) — `if !(spec.starts_with("./") || spec.starts_with("../"))
  { continue; }` — deliberately treats non-relative specifiers as external and skips them.
  This is a real design decision to carry forward: local-only edges, external packages
  excluded, confirmed correct by the `ts_import_of_bare_package_specifier_is_ignored` test
  (line 369).
- **Go blank imports (`_ "pkg"`) are not addressed in `import_graph.rs`** —
  `collect_go_imports` (lines 87–101) reads any `import_spec`'s `path` field unconditionally;
  it does not check for the `_` name alias, so a blank import currently *does* produce a
  graph edge, which is arguably correct for a dependency graph (the package's init side
  effects are a real dependency) but would be wrong for a *symbol* export (there's nothing
  to name — no exported symbol from a blank-imported package should appear as a node in a
  symbol-level tree). Note kibitzer already has a dedicated `go_blank_imports.rs` checker
  (found via grep) with its own detection logic — that's the pattern to reuse for
  distinguishing "dependency edge" from "symbol reference" in the new model, not a fresh
  implementation.
- **Not yet built, called out as such in the file's own doc comment** (line 34): "Only Go
  and TypeScript/JavaScript are extracted for now — Python/Kotlin/Java import extraction
  can follow the same per-language dispatch pattern later." So Python relative-vs-absolute
  imports, Java package-vs-file-path mismatches, and Kotlin's import resolution are **all
  unimplemented today** — confirm the plan phase treats these as full new extraction
  modules (per §1's finding that each language needs bespoke, hand-verified extraction),
  not small deltas.
- **Generic/templated type identity** has no precedent in this codebase at all — nothing
  in `rules.rs`/`import_graph.rs` currently needs to identify a *type* (only functions,
  params, if-chains). This is genuinely new ground; the symbol model needs an explicit
  decision on how a Go generic function (`func F[T any](...)`), a TS generic interface, or
  a Java generic class collapses to one "symbol identity" vs. being ignored/flattened.

## 5. MCP/LSP protocol-level pitfalls

- **The existing 150-node Mermaid cap (`mcp.rs` line 51) only bounds the diagram, not the
  underlying data.** `ArchitectureAssessmentRequest`'s `include_diagram` (line 53) gates
  Mermaid rendering; the text/findings portion of `architecture_assessment`'s output
  (lines 232–250) has no size cap at all — it's `finding_count` lines joined with no
  truncation. For a queryable tree exposed via new MCP tool(s), the same failure mode
  (unbounded text response) is worse: a whole-repo symbol tree serialized as JSON for a
  single MCP call could be enormous on a large repo (every function/type/interface across
  7 languages). **Design implication, following the plan question posed in the prompt**:
  the equivalent cap for a queryable tree should not be "150 nodes total" (that's diagram-
  specific, tuned for Mermaid legibility) but a *per-query* result cap plus pagination —
  e.g. a `list_symbols` MCP tool should take a scope/package filter and a page
  token/cursor rather than returning the whole model, mirroring how
  `ArchitectureAssessmentRequest.scope` (line 45) already narrows by glob today. Returning
  the *whole* model on every call reintroduces the exact "transient, recomputed every
  call" problem this feature exists to fix (per requirements.md's Problem Statement) — a
  large-but-unpaginated single response is barely better than the status quo for an agent
  that must parse it every time.
- **`tower_lsp`'s `ServerCapabilities` currently declares only `text_document_sync`**
  (`lsp.rs` lines 123–128) — no `document_symbol_provider`, `workspace_symbol_provider`,
  or any other capability flag is set (`..Default::default()` for the rest). Adding
  workspace/document symbol support means: (a) declaring the capability in
  `initialize()`, and (b) implementing `LanguageServer::symbol` (workspace symbols) and/or
  `LanguageServer::document_symbol` — **neither trait method is touched anywhere in this
  codebase today**, so there's no tower-lsp usage precedent here to lean on; this is the
  first real exercise of tower-lsp beyond diagnostics publishing, matching the
  requirements.md-flagged risk ("LSP workspace-symbol support may uncover protocol/
  tower-lsp limitations not yet exercised").
- **Editor-side timeout risk for `textDocument/documentSymbol` is real and unaddressed by
  current architecture.** Since `diagnostics_for_file` today re-parses from disk
  synchronously per request with no cache in the LSP path (§2), a naive `document_symbol`
  implementation that walks the file's AST on every request is probably fine per-file
  (small, single-file cost) — but `workspace/symbol` (searches across the *whole* repo)
  synchronously walking every file on every keystroke-triggered query is the actual risk;
  many editors apply a client-side timeout (commonly a few hundred ms to a couple of
  seconds) on `workspace/symbol`, so it needs to be served from a pre-built in-memory
  index (built at `initialize`/`initialized` time, updated incrementally on `did_save`),
  never recomputed from scratch inline with the request — this is new async/background
  infrastructure the current single-threaded, on-demand `check_and_publish` model (lines
  96–112) doesn't provide.
- **`did_change` triggering full re-check today (line 140–142) is itself a latency
  precedent to not repeat**: it already re-runs every check on every keystroke-adjacent
  edit (mitigated only by using last-saved-on-disk content, not live buffer content, per
  the comment at lines 90–95). Reusing this same "recompute everything synchronously on
  every notification" pattern for symbol-tree maintenance would compound: symbol
  extraction across 7 languages is strictly more expensive than the existing per-file
  complexity checks, so an unthrottled `did_save`-triggered full-model rebuild is a
  concrete latency risk even if `workspace/symbol` itself is served from a cache.

## Summary of design implications

1. Treat every language's extraction (imports *and* symbols) as a fully bespoke,
   hand-verified module — the existing `LangRuleConfig` table and `import_graph.rs`'s
   Go/JS split both show "shared generic walker" does not survive contact with real
   grammar divergence; budget per-language verification time accordingly.
2. The shared model needs its own incremental, whole-repo-scoped cache — `daemon.rs`'s
   per-file mtime-keyed `Cache` does not generalize, and `lsp.rs` doesn't even use that
   cache today, so LSP symbol support requires new caching/indexing infrastructure from
   scratch, not reuse.
3. Cap MCP tool responses with pagination/scoping, not a single fixed node count copied
   from the Mermaid 150-node cap — that cap is diagram-legibility-specific, not
   data-size-specific, and doesn't bound `ImportGraph` itself.
4. Extend `SKIP_DIRS` (`check.rs`) with the new languages' build/generated-artifact
   directories, and design an explicit "exported/public only by default" pruning knob
   analogous to `duplicate_code.rs`'s `MIN_OCCURRENCES`, expecting to tune it against real
   repos rather than get it right in the first pass.
5. `workspace/symbol`/`document_symbol` are unexercised tower-lsp surface in this codebase
   — plan real protocol-limitation discovery time, and design the index to be served from
   a background-maintained structure, never recomputed inline per request.
