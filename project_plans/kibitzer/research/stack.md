# Research: Stack (crates, versions, patterns)

**Research question**: What specific libraries, crates, versions, and patterns apply to
the architecture-linting feature (arbitrary component rules, package-content rules,
naming rules, Python/Java/Kotlin import-graph extraction)?

## 1. tree-sitter grammar versions — current pins vs. latest on crates.io

Checked `crates.io/api/v1/crates/<name>` (Aug 22 2026) against `Cargo.toml` /
`Cargo.lock` in `/home/tstapler/code/github.com/tstapler/kibitzer`:

| Crate | Pinned (`Cargo.toml`) | Locked (`Cargo.lock`) | Latest on crates.io | Verdict |
|---|---|---|---|---|
| `tree-sitter` | `0.26` | 0.26.12 | 0.26.12 | current |
| `tree-sitter-go` | `0.25` | 0.25.0 | 0.25.0 | current |
| `tree-sitter-typescript` | `0.23` | 0.23.2 | 0.23.2 | current |
| `tree-sitter-javascript` | `0.23` | 0.23.1 | **0.25.0** | behind |
| `tree-sitter-python` | `0.23` | 0.23.6 | **0.25.0** | behind |
| `tree-sitter-java` | `0.23.5` | 0.23.5 | 0.23.5 | current (pin matches latest) |
| `tree-sitter-kotlin-ng` | `1.1.0` | 1.1.0 | 1.1.0 | current |

**Upgrade risk assessment for `tree-sitter-python`/`tree-sitter-javascript` to 0.25.0**:
pulled each grammar crate's own `dependencies` from crates.io
(`/crates/tree-sitter-python/0.25.0/dependencies`,
`/crates/tree-sitter-javascript/0.25.0/dependencies`) — both declare `tree-sitter`
only as a **dev-dependency** (`^0.25.8`, used for the grammar's own tests) and depend
on `tree-sitter-language ^0.1` as their sole normal (runtime) dependency. That's the
same ABI-stability shim already in the lockfile (`tree-sitter-language 0.1.7`) that
lets `tree-sitter-go 0.25` and `tree-sitter-java 0.23.5` coexist with `tree-sitter
0.26` today (this repo already mixes grammar-crate minor versions against one
`tree-sitter` core version without issue). **Upgrading `tree-sitter-python` and
`tree-sitter-javascript` to 0.25.0 is low-risk** — it does not touch the `tree-sitter`
core version this project is built against. Since the JS/TS import-graph path
(`import_graph.rs::build_js`) is existing, working code, treat this as an optional
opportunistic bump, not a requirement for this feature — the feature only strictly
needs `tree-sitter-python`, `tree-sitter-java`, `tree-sitter-kotlin-ng` to work at
their **current** pins, which they already do (proven by `syntax-rules`' use of the
same three grammars — see `docs/syntax-rules.md`).

**Grammar node kinds relevant to import-graph + package-content/naming extraction**
(verified by extracting the actual crate sources):

- **Java** (`~/.cargo/registry/src/.../tree-sitter-java-0.23.5/src/node-types.json`,
  read directly): `import_declaration` (fields: `asterisk`, `identifier`, `scoped_identifier`),
  siblings at the same declaration level: `class_declaration`, `interface_declaration`,
  `package_declaration`, `record_declaration`. These same node kinds cover both the
  import-graph epic (`import_declaration`) and the package-content/naming epics
  (`class_declaration`/`interface_declaration`/`record_declaration` give you "what
  top-level declarations does this file contain, and what are their names").
- **Python** (extracted `tree-sitter-python-0.25.0.crate`'s `src/node-types.json`
  directly — same node kinds present in the currently-locked 0.23.6, this part of the
  grammar is stable across that range): `import_statement` (field `name`:
  `aliased_import` | `dotted_name`) and `import_from_statement` (field `module_name`)
  — Python needs **both** node kinds handled (`import x` vs `from x import y`), unlike
  Go/JS which only needed one import node kind each. `class_definition` and
  `function_definition` are the declaration-kind nodes for naming/content rules.
- **Kotlin** (`tree-sitter-kotlin-ng`): **not locally cached** (no extracted source or
  `.crate` file found under `~/.cargo/registry`, unlike Java/Python/JS which were
  present from a prior build) — could not verify node kinds by direct inspection.
  Web search corroborates the upstream `fwcd/tree-sitter-kotlin` grammar (which
  `tree-sitter-kotlin-ng` is based on) exposes `import_header`, `package_header`, and
  `class_declaration` node kinds, but this is **UNVERIFIED against the actual pinned
  crate** — flag this explicitly as the Phase 4 pre-mortem item that maps to the
  requirements doc's stated Feasibility Risk #1 ("grammar versions... touch different
  node kinds not yet verified"). Plan should budget a spike task: `cargo add
  tree-sitter-kotlin-ng` in a scratch binary, dump `node-types.json` from the actual
  built crate, and confirm the node kind names before writing `build_kotlin()`.

## 2. Rust rule-engine / graph-DSL crates — build vs. buy

Searched for a Rust equivalent of `depguard`/`go-arch-lint`'s allow/deny rule
expression. No good match exists:

- `cargo-deny` and the `restrict` crate are for auditing **Cargo's own dependency
  graph** (licenses, banned crates, security advisories) — a different domain (Rust
  package registry deps, not target-language import graphs) and not embeddable as a
  library for evaluating rules over an arbitrary `ImportGraph`.
- No maintained Rust crate found for "named-component + pairwise allow/deny
  dependency rule" evaluation specifically (the `depguard`/`go-arch-lint` niche has no
  direct Rust equivalent crate).
- Generic rule-engine options exist (`rhai`, `cel-interpreter`, datalog-style crates
  like `crepe`/`ascent`) but pulling in an embedded scripting/query language is a poor
  fit here: `Cargo.toml`'s dependency list is deliberately small (serde, clap, anyhow,
  regex, pulldown-cmark, tree-sitter\*, rmcp, schemars, tokio, tower-lsp) — no
  graph/rule-engine/scripting crate anywhere in it — and every existing checker
  (`ImportCycleChecker`, `LayeringChecker`, `CouplingChecker` in
  `src/architecture_checks.rs`) is a plain Rust `struct` + `impl ArchitectureChecker`
  with hand-rolled logic. Even cycle detection uses a **hand-rolled Tarjan's SCC**
  over `HashMap<String, _>` (`find_cycles()`, `src/architecture_checks.rs:181-243`)
  rather than pulling in `petgraph` (confirmed via crates.io: `petgraph 0.8.3` is
  current and would be a perfectly serviceable graph crate, but it isn't used —
  `ImportGraph`'s `nodes: BTreeSet<String>` / `edges: Vec<ImportEdge>` shape is
  intentionally simple, not petgraph's `Graph<N,E>`).

**Recommendation**: hand-roll the new `ComponentRule`/allow-deny model as a plain data
structure + a new `ArchitectureChecker` impl, matching the existing pattern exactly.
This is both the lower-risk and the more consistent choice — it doesn't introduce a
new dependency category, and it lets `layers: Vec<String>` desugar into the same
component model in-process (no serialization/interpretation boundary to cross). Do
**not** reach for `petgraph`, `rhai`, or `cel-interpreter`; they'd be the first
graph/scripting dependency in the crate and aren't needed for pairwise allow/deny
matching over a `BTreeSet`/`Vec<Edge>`.

## 3. Config/serde glob patterns already established — reuse, don't reinvent

`src/config.rs::Check.scope: Vec<String>` (line 64) is the existing precedent for
"glob patterns matched against a repo-relative path," documented as: `/// Glob
patterns (supporting `**`) a file path must match for this check to apply.` It's
backed by `src/glob.rs`, which is a **hand-rolled glob-to-regex compiler**
(`glob_to_regex()`) wrapped by `pub fn matches_scope(rel_path: &str, scopes:
&[String]) -> bool` — not the `globset`/`glob` crates (neither appears in
`Cargo.toml`). `matches_scope` already handles `**`, `**/`, `*`, `?`, with an
empty-list-matches-everything convention, and has its own unit tests
(`src/glob.rs:47-76`).

**The new component-glob-mapping (`ArchitectureConfig` component definitions) should
call `glob::matches_scope` directly**, not introduce a second glob implementation or
a new crate dependency. Concretely: a `Component { name: String, paths: Vec<String>
}` shape (paths = glob patterns in the same `**`-supporting syntax as `Check.scope`)
matched via `matches_scope(&node_path, &component.paths)` is consistent with both the
existing `Check.scope` field and the `#[serde(default)]` / doc-comment-heavy style
used throughout `config.rs` (every field has a `///` doc comment explaining exactly
how it's interpreted — `ArchitectureConfig` follows this too, e.g. lines 122-129).

`ArchitectureConfig` itself (`src/config.rs:120-130`) is `#[derive(Debug, Clone,
Default, Deserialize)]` with `#[serde(default)]` on its one field today (`layers`) —
the new `components`/`rules`/`content_rules`/`naming_rules` fields should follow the
same `#[serde(default)]` pattern so old configs with only `layers` set continue to
deserialize unchanged (this is also what the requirements doc's "layers continues to
parse... no breaking change" scope item needs — `serde`'s `#[serde(default)]` on an
added field is sufficient by itself for backward compatibility, no custom
`Deserialize` impl or migration code required).

## 4. Tree-sitter declaration-extraction pattern — hand-rolled walk, no query crate

Checked for use of `tree_sitter::Query`/`QueryCursor` (tree-sitter's built-in
S-expression query API, which would be the "don't hand-roll" option for "find all
nodes of kind X"): **not used anywhere in `src/`** (`grep -rn "tree_sitter::Query"
src/` → no matches). Every existing AST-walking checker uses **plain recursive
`Node` traversal**:

- `src/rules.rs::walk_declarations()` (line 348): recurses over `node.children()`,
  checking `cfg.function_kinds.contains(&node.kind())` — a per-language
  `LangRuleConfig` (line 175, `lang_config()`) holds the set of node-kind strings
  that count as a "declaration" for that language (functions/methods), so the
  language-specific node-kind names are centralized in one config struct rather than
  scattered through match arms.
- `src/primitive_obsession.rs::walk()` (line 72): same shape — recurse, check
  `node.kind() == "parameter_list"`, dispatch to a language-specific handler.
- `src/import_graph.rs::collect_go_imports()` (line 87): identical recursive-walk
  shape, checking `node.kind() == "import_spec"`.

**This confirms hand-rolled recursive walking, keyed by node-kind string constants
per language, is the established and only pattern in this codebase** — there's no
existing use of tree-sitter's `Query` API to deviate from. For the new
package-content and naming-rule categories (which need "extract all top-level
declarations of kind X with their names," per requirements item 2/3), the natural
extension is: add a `declaration_kinds` (or reuse/extend `LangRuleConfig` from
`rules.rs`, which is closely related to import_graph.rs's — same per-language
node-kind config split) and a `walk_declarations`-shaped function returning
`(kind, name, file, line)` tuples per node, mirroring `collect_go_imports`'s
`(String, usize)` output shape. Whether to add `tree_sitter::Query` for the *new*
rule categories (since the node-kind matching there is closer to "find all nodes
matching pattern P," which `Query`'s S-expression syntax is designed for) is a
legitimate implementation-phase judgment call — it would be a genuinely new
capability (no existing code uses it) with real benefit (query strings are more
concise ways to say "class_declaration with a name field," and would sidestep
re-deriving four more per-language node-kind constant tables) — but note that
adopting it is a departure from the codebase's 100%-hand-rolled precedent and should
be an explicit, deliberate choice in Phase 3 planning, not a silent one.

## Summary of concrete recommendations

1. Keep `tree-sitter 0.26`, `tree-sitter-java 0.23.5`, `tree-sitter-kotlin-ng 1.1.0`
   pinned as-is — already current. Optionally bump `tree-sitter-python` and
   `tree-sitter-javascript` to `0.25` (verified low-risk: both only touch
   `tree-sitter-language ^0.1` at runtime) — not required for this feature.
2. No external rule-engine/graph crate. Hand-roll the component/rule model as a new
   `ArchitectureChecker` impl, matching `ImportCycleChecker`/`LayeringChecker`/
   `CouplingChecker`'s existing plain-struct pattern (including hand-rolled graph
   algorithms — no `petgraph`, despite it being current at `0.8.3`).
3. Reuse `glob::matches_scope()` (`src/glob.rs`) for component path-to-glob matching
   — do not add `globset`/`glob` crates or write a second glob matcher. New
   `ArchitectureConfig` fields should follow the `#[serde(default)]` convention
   already on `layers` for backward compatibility.
4. No `tree_sitter::Query` usage anywhere in the codebase today — every AST walk
   (`rules.rs`, `primitive_obsession.rs`, `import_graph.rs`) is hand-rolled recursion
   over `node.kind()` string matches, centralized per-language in config structs like
   `LangRuleConfig`. Follow that pattern for package-content/naming extraction unless
   Phase 3 planning deliberately decides to introduce `Query` as a first use.
5. Kotlin (`tree-sitter-kotlin-ng`) grammar node kinds for imports/declarations are
   **unverified** — no local crate source was available to inspect directly (unlike
   Java and Python, which were verified against actual `node-types.json`). Budget a
   verification spike before implementing `build_kotlin()`.
