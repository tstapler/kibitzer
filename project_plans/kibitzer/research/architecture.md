# Research: Architecture — kibitzer arbitrary architecture-linting rules

Scope: read in full `src/architecture_checks.rs` (435 lines), `src/import_graph.rs` (365
lines), `src/config.rs` (370 lines), `src/checker.rs` (300 lines); skimmed `src/rules.rs`
(1283 lines, read first ~410) and `src/mcp.rs` (`architecture_assessment` tool, lines
41-271) and `src/mermaid.rs` (163 lines, in full). All line numbers below are against the
current `master` working tree at clone time (2026-08-22); no prior commit is cited because
none of these files carry meaningful history relevant to this feature (fresh area).

## 1. Existing architectural patterns (context for everything below)

kibitzer already has **three separate checker abstractions**, each with its own trait +
flat-namespace registry — this is the load-bearing precedent for every design choice below:

| Trait | File | Registry | Runs against | Dispatch key |
|---|---|---|---|---|
| `Checker` | `src/checker.rs:64-77` | `checker::registry()`/`lookup()` | one file's parsed tree (`CheckContext`) | `Check.checker: Option<String>` |
| `ArchitectureChecker` | `src/architecture_checks.rs:19-22` | `architecture_checks::registry()`/`lookup()` | whole-repo `ImportGraph` + `ArchitectureConfig` | `Check.architecture_checker: Option<String>` |
| *(new, proposed)* content/naming | — | new registry | whole-repo `DeclarationGraph` + `ArchitectureConfig` | same `architecture_checker` string field (see §6) |

`Check` (`src/config.rs:27-76`) already enforces **mutual exclusion** across
`command`/`checker`/`architecture_checker` via a count-and-bail validator
(`src/config.rs:140-165`) — exactly one must be set. `architecture_checker` checks are
further constrained to `triggers` of `["batch"]` or none (`src/config.rs:186-194`), because
rebuilding the import graph per-edit is too expensive.

`ImportGraph` (`src/import_graph.rs:22-25`) is directory/package-granularity, not
file-granularity: `nodes: BTreeSet<String>`, `edges: Vec<ImportEdge{from,to,file,line}>`.
Built once per batch invocation by `import_graph::build(repo_root, files)`, which dispatches
by file extension to `build_go`/`build_js` (`src/import_graph.rs:36-49`).

Three `ArchitectureChecker` impls exist today, all reading only `graph.nodes`/`graph.edges`
plus `ArchitectureConfig.layers`:
- `ImportCycleChecker` (`architecture_checks.rs:36-62`) — Tarjan SCC (`find_cycles`,
  `architecture_checks.rs:186-258`), also reused by `src/mermaid.rs:7,48` to highlight
  cycle edges in the diagram.
- `LayeringChecker` (`architecture_checks.rs:76-119`) — uses `layer_of()`
  (`architecture_checks.rs:69-74`): a node belongs to the **first layer name that exactly
  matches any `/`-split segment** of the node string (`segments.contains(&layer.as_str())`),
  found via `.position()` (first match wins, no ambiguity handling needed since it's a
  linear ordered list). Flags an edge when `to_layer < from_layer` (a later/lower layer
  reaching back into an earlier/higher one).
- `CouplingChecker` (`architecture_checks.rs:128-179`) — fixed fan-in/fan-out thresholds
  (`MAX_FAN_OUT`/`MAX_FAN_IN = 10`), no config.

`src/checker.rs`'s `Language` enum (`checker.rs:29-37`) **already includes** `Python`,
`Java`, `Kotlin` — all three grammars are already Cargo dependencies
(`tree-sitter-python 0.23`, `tree-sitter-java 0.23.5`, `tree-sitter-kotlin-ng 1.1.0`,
`Cargo.toml:25-27`) and are already wired into `GrammarCache`/`SyntaxRulesChecker`
(`checker.rs:89-95`, `rules.rs:231-307`) for the complexity checks. **Only
`import_graph.rs::build()`'s dispatch is missing these three** — the grammars, the
`Language` variants, and the per-language config-table pattern (`rules.rs`'s
`LangRuleConfig`, `rules.rs:59-93`) all already exist and are directly reusable.

`glob.rs` (76 lines) is the one glob-matching primitive in the codebase:
`matches_scope(rel_path: &str, scopes: &[String]) -> bool`, backed by
`glob_to_regex()` (`glob.rs:1-33`, supports `*`, `**`, `**/`, `?`). This is what
`Check.scope` (`config.rs:64`) uses today, and is the natural mechanism to reuse for
glob-mapped component definitions (see §2) — but note it's built to match **relative file
paths**, and the new use case is matching **`ImportGraph`/`DeclarationGraph` node
identifiers** (Go: `module/path/segment`; JS/TS: a directory key). Both are `/`-separated
strings, so `glob_to_regex` applies directly; no new matcher is needed.

`src/mcp.rs`'s `architecture_assessment` tool (`mcp.rs:130-271`) is the integration seam
for all of this: it walks files, filters by `scope`, loops `config.checks` calling
`run_architecture_check` for every check with `architecture_checker.is_some()`
(`mcp.rs:161-187`), then separately runs the seven `SYNTAX_RULES_CHECKERS` per-file
(`mcp.rs:66-74`, `189-230`), then unconditionally builds an `ImportGraph` again for the
Mermaid diagram (`mcp.rs:251-266`, `src/check.rs:786-847` for `run_architecture_check`
itself). `recommendation_for()` (`mcp.rs:78-92`) is a flat match on check name → canned
text; trivially extensible for new checker names.

## 2. Rule-expression schema — recommendation: arch-go-style named components + `mayDependOn`/`denyDependOn` per component

The requirement names three reference shapes: go-arch-lint's adjacency list,
depguard's file-glob deny/allow, arch-go's `mayDependOn` component rules. **Recommend
arch-go's shape** (named components + per-component allow/deny lists), for two concrete
reasons grounded in what's already in this codebase:

1. `layers` already desugars naturally into "named thing + who it may depend on" (see §3) —
   `LayeringChecker`'s existing semantics are already a `mayDependOn`-shaped relation
   (`layer_of` + "may depend on any layer at or after my index"), just expressed
   positionally instead of by name. Reusing the same shape for the general case means the
   desugar in §3 is a small, mechanical transformation, not a redesign.
2. Component membership is naturally glob-based (`Component.paths: Vec<String>`), directly
   reusing `glob_to_regex`/`matches_scope` — the same primitive `Check.scope` already uses.
   depguard's file-glob-deny/allow shape conflates "what is this component" with "what can
   it depend on" into one rule; arch-go's separates them, which composes better with
   content/naming rules in §3-4 also needing to reference the same named components.

Concrete schema (new types in `config.rs`, alongside `ArchitectureConfig`):

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct Component {
    pub name: String,
    /// Glob patterns (matches_scope semantics) matched against ImportGraph/
    /// DeclarationGraph node identifiers — NOT triggering-file paths.
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct DependencyRule {
    pub component: String,
    /// Closed-world allow-list: if Some, `component` may depend ONLY on the named
    /// components listed (plus itself, implicitly). Anything else in scope is a
    /// violation. Mirrors LayeringChecker's "None = ignored" semantics for nodes that
    /// don't map to any named component — those stay unconstrained either way.
    #[serde(default)]
    pub may_depend_on: Option<Vec<String>>,
    /// Open-world deny-list: `component` may depend on anything EXCEPT these.
    #[serde(default)]
    pub deny_depend_on: Vec<String>,
}
```

`ArchitectureConfig` grows `components: Vec<Component>` and
`dependency_rules: Vec<DependencyRule>` (both `#[serde(default)]`, additive, non-breaking).

New checker `ComponentDependencyChecker` (name e.g. `"component-deps"`), added to
`architecture_checks::registry()` alongside the existing three — implements the *existing*
`ArchitectureChecker` trait unmodified (it only needs `ImportGraph` + `ArchitectureConfig`,
no new trait needed for this one). Node→component mapping: first glob pattern match wins,
in declaration order (mirrors `layer_of()`'s `.position()` first-match rule) — a node
matching no component is ignored, same as today.

**Validation** (`config.rs::validate()`, alongside the existing unknown-checker-name bails
at `config.rs:166-185`): every `DependencyRule.component` and every name inside
`may_depend_on`/`deny_depend_on` must reference a declared `Component.name` (own it or a
desugared layer name, see §3) — bail with the same `{config_path}: check '{name}' ...`
convention on an unknown reference.

## 3. `layers` desugar — concrete, precise

`layers: Vec<String>` **keeps parsing exactly as today** — no change to that field or to
`LayeringChecker`'s code (satisfies "existing checkers continue working unmodified").
Desugaring happens via two new accessor methods on `ArchitectureConfig` that the *new*
`ComponentDependencyChecker` calls instead of reading `.components`/`.dependency_rules`
directly:

```rust
impl ArchitectureConfig {
    pub fn effective_components(&self) -> Vec<Component> {
        self.components.iter().cloned()
            .chain(desugar_layers_to_components(&self.layers))
            .collect()
    }
    pub fn effective_dependency_rules(&self) -> Vec<DependencyRule> {
        self.dependency_rules.iter().cloned()
            .chain(desugar_layers_to_rules(&self.layers))
            .collect()
    }
}
```

For `layers = ["handlers", "domain", "infra"]` (N=3, index 0 = highest):

- **Components**: one per layer name. Because `layer_of()` matches the layer name against
  **any** `/`-segment of the node (not just a prefix or suffix), each desugared component
  needs **four glob patterns** to reproduce that "segment anywhere" semantics exactly —
  a single glob can't express "contains this exact segment" directly:
  ```
  Component { name: "domain", paths: ["domain", "domain/**", "**/domain", "**/domain/**"] }
  ```
  (whole-path match, leading segment, trailing segment, middle segment.)
- **Rules**: one `DependencyRule` per layer, **not** one per pair — an allow-list rule
  naturally covers "may depend on any of these N-i", so it's **N rules total**, each an
  upper-triangular allow-list, together encoding the same coverage N² pairwise adjacency
  checks would (up to N(N+1)/2 allowed source→target relations counting self-edges):
  ```
  handlers: may_depend_on = ["handlers", "domain", "infra"]   // i=0, layers[0..]
  domain:   may_depend_on = ["domain", "infra"]                // i=1, layers[1..]
  infra:    may_depend_on = ["infra"]                          // i=2, layers[2..]
  ```
  i.e. `may_depend_on: layers[i..].to_vec()` for the layer at index `i`.

**Collision handling**: if a user declares both `layers: ["domain", ...]` *and* an explicit
`Component { name: "domain", ... }`, `effective_components()` would yield two components
named `"domain"` with different `paths`. Recommend treating this as a **validation error**
in `config.rs::validate()` (same bail convention as unknown-checker-name), not a silent
merge — the two glob sets could easily diverge and produce confusing partial coverage.

**Redundancy with the `layering` check**: nothing prevents a `.claude/inspect.json` from
registering *both* a `checks: [{architecture_checker: "layering", ...}]` entry (reading
`config.layers` directly) *and* a `{architecture_checker: "component-deps", ...}` entry
(reading the desugared form) — they'd report the same violations twice. This is a
config-authoring concern, not a code-correctness one: document that adopting
`component-deps` supersedes `layering` for a given check block, the same way a repo
wouldn't register two `command` checks running the identical linter twice. No enforcement
needed at parse time; flag for Phase 3 whether the CLI/docs should warn on this specific
overlap.

## 4. Content/naming rules — AST-inspection pass architecture

**Recommendation: new module(s), new `DeclarationGraph`, new third trait+registry** — do
**not** fold this into `import_graph.rs`'s per-file walk, and do not extend the existing
`ArchitectureChecker` trait's signature. Three reasons, all grounded in what's already
here:

1. **Established precedent for a third trait+registry, not signature-widening.** kibitzer
   already has *two* separate checker traits/registries for two separate consumption
   shapes (`Checker` for per-file, `ArchitectureChecker` for `ImportGraph`). Adding a third
   — `DeclarationChecker`/some name, consuming a `DeclarationGraph` — for content/naming's
   genuinely different graph shape is the same move made a second time, not a new pattern.
   Widening `ArchitectureChecker::check()`'s signature to `(ctx: &ArchitectureContext,
   config)` where `ArchitectureContext` bundles both graphs would in fact touch
   `ImportCycleChecker`/`LayeringChecker`/`CouplingChecker`'s call sites (mechanical, but
   real) — violating "existing checkers continue working unmodified" more literally than
   the third-trait option, which leaves `architecture_checks.rs` completely untouched.
2. **Content/naming rules are not file-local — they're component/package-local**, so they
   don't fit `Checker`'s per-file model either. "package X must only contain structs"
   aggregates over every file mapped to component X. Naming rules referencing interface
   implementation are worse: **Go interface satisfaction is structural and can span files
   in the same package** (a struct's methods can be defined across multiple `.go` files) —
   a per-file walk cannot resolve "does struct S implement interface I" correctly on its
   own. This must be a whole-repo pass with per-component aggregation, same shape as
   `ArchitectureChecker`, not `Checker`.
3. **Avoiding double-parsing already has an established mechanism — reuse it, don't
   reinvent it.** `checker.rs`'s `GrammarCache` (`checker.rs:145-171`) exists specifically
   so multiple checkers sharing a `Language` share one parse. But `import_graph.rs`'s
   `build_go`/`build_js` **don't** use `GrammarCache` today — each constructs its own
   `Parser` inside the loop and reads the file fresh (`import_graph.rs:118-131`,
   `227-233`). So "extend `ImportGraph`'s walk to also collect declarations" would still
   mean parsing once per (file, purpose) unless `import_graph.rs` is *also* refactored onto
   `GrammarCache` — at which point the cleaner change is giving the new declaration pass
   its own `GrammarCache`-backed walk (one parse per file, shared within that pass), not
   entangling it with import-graph construction, which has a different per-language
   resolution algorithm entirely (§5) and no reason to be coupled to declaration
   extraction.

Concrete shape, new module `src/declarations.rs`:

```rust
pub struct Declaration {
    pub name: String,
    pub kind: DeclKind,       // Struct, Interface, Class, Function, ...
    pub file: PathBuf,
    pub line: usize,
    pub component: Option<String>,   // resolved via the same Component.paths globs as §2
    pub methods: Vec<String>,        // best-effort, for naming rules referencing "implements X"
}

pub struct DeclarationGraph {
    pub declarations: Vec<Declaration>,
}

pub fn build(repo_root: &Path, files: &[PathBuf], components: &[Component])
    -> Result<DeclarationGraph> { ... }  // per-language dispatch, mirrors import_graph::build
```

New module `src/declaration_checks.rs` (or same file): a `DeclarationChecker` trait
(`name()`, `check(&DeclarationGraph, &ArchitectureConfig) -> Vec<ArchFinding>`, reusing the
existing `ArchFinding` type unmodified), its own `registry()`/`lookup()`, holding
`ContentChecker` (reads new `ArchitectureConfig.content_rules: Vec<ContentRule>`) and
`NamingChecker` (reads new `naming_rules: Vec<NamingRule>`):

```rust
pub struct ContentRule {
    pub component: String,
    pub allowed_kinds: Vec<String>,   // e.g. ["struct"] — "package X must only contain structs"
}
pub struct NamingRule {
    pub applies_to: NamingScope,      // e.g. Implements("Handler") or Component("domain")
    pub pattern: String,              // suffix or regex, e.g. structs implementing X must end in "Handler"
}
```

Flag for Phase 3 sizing: Go's implicit interface satisfaction means `NamingChecker`'s
`Implements(X)` scope can only be a **method-name-set approximation** (does struct S
declare every method named in interface I's declared method set — not real type-checking,
no return-type/param-type verification). State this as a documented limitation up front,
not discovered mid-implementation — it's a real gap versus arch-go, which runs against
`go/types` and gets exact interface satisfaction for free.

## 5. Per-language import-graph extraction (Python/Java/Kotlin)

The two languages already implemented use **two different resolution strategies** — this
is the real reason `import_graph.rs` isn't already uniform, and it determines how much new
code Python/Java/Kotlin actually need:

- **Go** (`import_graph.rs:103-146`): package identity = **import-path string**, anchored
  by parsing `go.mod`'s `module` directive once (`go_module_path`,
  `import_graph.rs:67-74`) and computing each file's package path as
  `{module_path}/{dir relative to repo_root}` (`go_package_import_path`,
  `import_graph.rs:76-85`). Import statements are matched **by string equality** against
  already-known package paths (`import_graph.rs:134`) — no filesystem resolution at all.
- **JS/TS** (`import_graph.rs:213-259`): package identity = **containing directory**, and
  only *relative* specifiers (`./`, `../`) are resolved, via filesystem candidate-guessing
  (extension list + `index.*` fallback) against a canonicalized known-files map
  (`resolve_relative_import`, `import_graph.rs:195-211`). Bare/absolute specifiers are
  explicitly skipped (`import_graph.rs:240-242`) — "not local, nothing to resolve."

For the three new languages:

- **Java**: package identity is **self-declared** via a `package com.foo.bar;` statement at
  the top of each file (tree-sitter `package_declaration`), and imports
  (`import com.foo.Bar;`, wildcard `import com.foo.*;`) are already fully-qualified
  strings — **structurally identical to Go's model** (declared identity + qualified-string
  matching, no filesystem resolution), just with a different anchor: no single `go.mod`-like
  manifest, the package string comes from each file itself rather than from
  directory-relative-to-module-root math. This is a strictly *simpler* case than Go's,
  not harder.
- **Kotlin**: same shape as Java — `package com.foo.bar` declaration,
  `import com.foo.Bar` / `import com.foo.*` / `import com.foo.Bar as Baz` (aliased —
  the alias is irrelevant to graph construction, only the qualified target matters).
- **Python**: genuinely different, and closer to JS's relative-resolution family than to
  Go/Java's qualified-string family — but not identical to JS either:
  - Relative imports (`from . import x`, `from ..pkg import y`) resolve by **counting
    leading dots** as levels up from the *current file's directory* (not literal `./`/`../`
    path segments like JS) — a distinct resolution function is needed, not a reuse of
    `resolve_relative_import`.
  - Absolute imports (`import foo.bar.baz`, `from foo.bar import baz`) need a **project
    source-root anchor** to resolve against, and Python has **no `go.mod`-equivalent single
    manifest** declaring one — flag this as a real, unresolved gap for Phase 3: the
    fallback is a heuristic (walk from `repo_root`, treat directories containing
    `__init__.py` as packages, dotted-path-match against that tree), which will have lower
    precision than Go's exact resolution and should be scoped/tested accordingly, possibly
    deferred to "relative imports only" for a first cut if the heuristic proves noisy.

**Recommendation on duplication**: Go/Java/Kotlin share one resolution *family*
(self/manifest-declared qualified identity + string-equality import matching) —
factor a single generic helper parameterized by a small per-language table, following the
exact precedent `rules.rs::LangRuleConfig` already establishes for per-language tree-sitter
node-kind differences (`rules.rs:59-93`, `175-308`):

```rust
struct QualifiedImportLangConfig {
    package_decl_kind: &'static str,      // e.g. "package_clause" (Go) / "package_declaration" (Java/Kotlin)
    import_stmt_kind: &'static str,
    // ... field/positional extraction fns, mirroring body_finder/params_finder in rules.rs
}
```

One shared `build_qualified_name_language()` (refactor of `build_go`'s core loop to accept
the table) called for Go, Java, Kotlin — 3 small tables, 1 shared implementation, instead
of 3 fully hand-rolled functions. Python does **not** fit this family (dot-counting
relative resolution + heuristic absolute resolution is a different algorithm entirely) and
needs its own `build_python`, bespoke like `build_js` is today. Net: `build()`'s dispatch
(`import_graph.rs:36-49`) grows three new extension-filtered branches
(`.py`→`build_python`, `.java`→shared-family call, `.kt`/`.kts`→shared-family call), same
pattern as the existing two.

## 6. Integration point: `Check.architecture_checker`

**Recommendation: keep `Check.architecture_checker: Option<String>` as a single flat
string field — no new category field.** Reasoning: `checker::registry()` already resolves
seven distinct checker names (`"syntax-rules"`, `"syntax-rules-typescript"`, ...,
`checker.rs:81-97`) through one flat namespace under the single `Check.checker` field —
category information (which language) is encoded in the *name*, not a separate schema
field. The same convention extends cleanly here: `"import-cycles"`, `"layering"`,
`"coupling"`, `"component-deps"` (§2) stay resolvable via `architecture_checks::lookup()`;
new `"content-rules"`, `"naming-rules"` (§4) resolve via the new registry. Add one thin
merge point instead of changing `Check`'s schema:

```rust
// somewhere shared, e.g. a new src/architecture.rs or as a fn in config.rs
pub fn lookup_any_architecture_checker(name: &str) -> Option<AnyArchitectureChecker> {
    architecture_checks::lookup(name).map(AnyArchitectureChecker::Import)
        .or_else(|| declaration_checks::lookup(name).map(AnyArchitectureChecker::Declaration))
}
```

This is the one place `config.rs::validate()`'s unknown-architecture-checker bail
(`config.rs:177-185`) and `check.rs::run_architecture_check` (`check.rs:786-847`, which
currently unconditionally builds an `ImportGraph` at `check.rs:813`) both need to change —
`run_architecture_check` needs to branch on which variant `lookup_any_architecture_checker`
returns and build the matching graph (`ImportGraph` vs `DeclarationGraph`) before calling
`.check()`. Given `architecture_checker` checks are already batch-only (existing
`triggers` validation, `config.rs:186-194`), building both graphs unconditionally when
*either* kind of check is configured is an acceptable simplification (the
`architecture_assessment` MCP tool already unconditionally builds an `ImportGraph` for the
diagram regardless of which checks are configured — `mcp.rs:251-266` — so this isn't a new
inefficiency class, just a second instance of an existing one).

`ArchitectureConfig` ends up with two independent-but-parallel families of fields, both
consumed by different checker registries but living in the same struct (as `layers` and a
future `components`/`dependency_rules` already would per §2-3):
```
layers, components, dependency_rules   → architecture_checks (ImportGraph)
content_rules, naming_rules            → declaration_checks (DeclarationGraph)
```

**`architecture_assessment` MCP tool + Mermaid**: the check-running loop
(`mcp.rs:161-187`) needs no structural change — it already iterates `config.checks`
generically by `architecture_checker.is_some()` and calls `run_architecture_check`, so once
that function routes to the right registry/graph internally, content/naming findings surface
through the same loop, `recommendation_for()` (`mcp.rs:78-92`) extended with a
`"component-deps"`/`"content-rules"`/`"naming-rules"` arm each. For the diagram
(`mermaid.rs`), a light, concrete extension: since `graph TD` supports `subgraph` blocks,
group nodes by resolved component name (§2's mapping) into one `subgraph {component} ...
end` per component, leaving edge/cycle-highlighting logic (`mermaid.rs:48-82`) untouched —
this visually surfaces component boundaries and dependency-rule violations (edges crossing
into a subgraph they're not allowed into) without new graph-model changes.

## 7. Not applicable

Per the requirement's skip condition: this is a linter feature (single bounded domain —
"architecture rules to check"), not a multi-actor business domain, so no
Event-Command-Policy EventStorming table is included.

## Summary of concrete recommendations for Phase 3

1. Rule schema: named `Component{name, paths}` + `DependencyRule{component,
   may_depend_on, deny_depend_on}`, reusing `glob.rs`'s existing matcher against graph node
   identifiers.
2. `layers` desugars via `ArchitectureConfig::effective_components()`/
   `effective_dependency_rules()` accessor methods (not eager mutation at parse time);
   4-glob-pattern expansion per layer to reproduce `layer_of()`'s segment-anywhere
   semantics exactly; N rules (not N² pairs) via per-component allow-lists;
   name-collision between literal and desugared components is a validation error.
3. Content/naming rules get a **third** trait+registry (`DeclarationChecker`) and a new
   `DeclarationGraph`, not a widened `ArchitectureChecker` signature and not folded into
   `import_graph.rs`'s walk — keeps existing checkers byte-for-byte unmodified and matches
   the codebase's own established two-registries-already pattern.
4. Go interface-implementation naming rules are a method-name-set approximation, not true
   type-checking — document as a scoped limitation up front.
5. Go/Java/Kotlin import extraction shares one generic qualified-name resolver +
   per-language tables (mirroring `rules.rs::LangRuleConfig`); Python needs its own
   bespoke resolver (dot-counting relative + heuristic absolute-import resolution, the
   latter flagged as lower-confidence given Python has no `go.mod`-equivalent anchor).
6. `Check.architecture_checker` stays a single flat string field; a new
   `lookup_any_architecture_checker` merge point in `check.rs`/`config.rs` dispatches
   between the two registries and the two graph types it needs to build.
