# Research: Build vs. Buy — Architecture Linting for kibitzer

**Agent**: Research Agent 6 (Build vs. Buy)
**Scope**: requirements.md scope items 1-4 (dependency rules, content rules, naming rules,
Python/Java/Kotlin import extraction)

## Summary

Kibitzer is a Rust binary; all four comparison tools (`depguard`, `go-arch-lint`, `arch-go`,
`cht-go-lint`) are Go and cannot be linked in. There is no viable "buy" option at the
dependency level for any of the four in-scope epics. The Rust ecosystem has no mature,
generic "named components + graph policy rules" crate, and the one crate close to
kibitzer's exact niche (`architect-linter-pro`) is unproven. The right call is to **build**,
using kibitzer's own existing tree-sitter-direct pattern (already proven in `src/rules.rs`
and `src/import_graph.rs`) and using `arch-go`'s rule *schema* — not its Go implementation —
as a design reference for the dependency/content/naming rule shapes.

---

## 1. Existing Rust crate for graph-based dependency-rule evaluation

Searched crates.io and GitHub for "architecture linter Rust", "dependency rule engine Rust",
"graph policy Rust".

| Candidate | Verified via | Findings |
|---|---|---|
| [`architect-linter-pro`](https://crates.io/crates/architect-linter-pro) ([repo](https://github.com/sergiogswv/architect-linter-pro)) | `crates.io` API + `gh api repos/...` | Multi-language Rust arch linter with a "dynamic rule engine" — the closest match to kibitzer's exact niche. But: 1 GitHub star, 64 total downloads since it appeared on crates.io (2026-02-26), only 2 published versions ever (5.0.0 → 6.0.0 in 9 days — an odd major-version jump for a 2-release-old crate), 45 KB repo. Reads as a solo/vibe-coded project, not a maintained library. |
| [`alint`](https://crates.io/crates/alint) ([repo](https://github.com/asamarts/alint)) | `WebFetch` on repo README, 63 stars, 1112 downloads, actively pushed | Real and reasonably maintained, but explicitly out of scope by its own docs: "not a code / AST linter (use ESLint, Clippy, ruff)"; its 13 rule families (existence, content, naming, JSONPath query, text hygiene, git hygiene, etc.) are all filesystem/text-level, not import-graph or code-semantic. Wrong problem domain. |
| `cargo-deny`, `dependency-graph` (crates.io) | WebSearch | Operate on Cargo's own crate/version dependency graph, not source-level named-architectural-component rules. Wrong domain. |

**Verdict: Not recommended (buy).** No mature generic crate exists for "named components +
allow/deny dependency rules." **Build**, extending `ArchitectureConfig`/`ArchitectureChecker`
(`src/config.rs`, `src/architecture_checks.rs`), which are already structured for exactly
this: a checker trait operating on `ImportGraph` + typed config.

---

## 2. Existing Rust crate for cross-language declaration extraction

Kibitzer's own `src/rules.rs` already hand-rolls this today for syntax rules: a
`LangRuleConfig` table per language (function/class node kinds, field-name vs. positional
child lookup — e.g. Kotlin's `function_declaration` exposes no field names at all, unlike
Go/JS/Python/Java) walked recursively with `node.kind()` dispatch. This is proven,
already in production.

| Candidate | Verified via | Findings |
|---|---|---|
| [`tree-parser`](https://crates.io/crates/tree-parser) | crates.io API | 2140 downloads, but **no `repository` field on crates.io** — the source cannot be audited. Async-first design doesn't match kibitzer's synchronous per-file batch model. Unproven, unauditable. |
| [`ast-grep-core`](https://crates.io/crates/ast-grep-core) / `ast-grep-language` | crates.io API (2.29M downloads, created 2022, actively maintained), [languages doc](https://ast-grep.github.io/reference/languages.html) confirms Python/Java/Kotlin/Go/JS/TS all built-in | The one genuinely mature candidate. But adopting it as a dependency means: (a) a second, redundant set of tree-sitter grammar bindings alongside the ones kibitzer already depends on directly (`tree-sitter-go`, `tree-sitter-python`, etc. in `Cargo.toml`); (b) its core value is pattern-match-and-rewrite ("find this shape"), not "enumerate every top-level declaration and classify it" — the latter is what content/naming rules need, and kibitzer's `rules.rs` pattern already solves it directly against `tree_sitter::Node`. |

**Verdict: Build is already the established, working answer.** Extending
`rules.rs`'s `node.kind()`-dispatch-plus-per-language-table pattern to declaration
enumeration (rather than introducing `ast-grep-core` as a library dependency) is
**Recommended** — consistent with the codebase's existing architecture, and confirmed
below (§3) to be structurally the same approach `arch-go` itself uses in Go.
`ast-grep-core` is **Viable** only as a future, separate consideration if kibitzer ever
wants shape-based pattern matching as a distinct rule category — not a fit for this scope.

---

## 3. `arch-go`'s algorithm as reference design (not dependency)

Read the actual source via `gh api` (not just docs) — `internal/validators/*.go` (schema
validation) and `internal/verifications/{dependencies,contents,naming}/*.go` (actual rule
evaluation):

- **Dependency rules** ([`check_allowed.go`](https://github.com/arch-go/arch-go/blob/main/internal/verifications/dependencies/check_allowed.go)): import path strings are bucketed into `Internal`/`External`/`Standard` by a simple prefix check against the module path, then matched against glob-derived regexes (`text.PreparePackageRegexp`). No graph algorithm — pure string/regex matching per edge.
- **Content rules** ([`retrieve_contents.go`](https://github.com/arch-go/arch-go/blob/main/internal/verifications/contents/retrieve_contents.go)): uses Go's own `go/ast` + `go/parser` stdlib, walked with `ast.Inspect` and a **type-switch** on node type (`*ast.FuncDecl` → Functions or Methods depending on `Recv`; `*ast.InterfaceType`; `*ast.StructType`), incrementing counters. This is the *same shape* as kibitzer's `node.kind()`-switch walk in `rules.rs` — just against Go's typed AST instead of tree-sitter's generic `Node`. Confirms the "build" pattern kibitzer already uses is not idiosyncratic; it's the same algorithm `arch-go` itself uses, adapted per-toolchain.
- **Naming rules** ([`implements_interface.go`](https://github.com/arch-go/arch-go/blob/main/internal/verifications/naming/implements_interface.go)): pure method-signature-set comparison (name + parameter types + return types) between a struct's extracted methods and an interface's — no AST cleverness, just data-structure comparison after extraction.

**Verdict: Recommended to port the *rule schema*, not the code** (which can't be reused —
`go/ast`/`go/parser` have no Rust equivalent, and arch-go is Go-only regardless). Concretely
worth adopting into kibitzer's `ArchitectureConfig`:
1. Named components as glob patterns (not just an ordered `layers: Vec<String>` tier list).
2. Dependency rules as `should_only_depend_on` / `should_not_depend_on`, each bucketable if useful (kibitzer doesn't need Go's std/external/internal split verbatim, but the *mutually-exclusive-should-only-vs-should-not* shape is a clean, already-battle-tested API).
3. Content rules as `should_only_contain` / `should_not_contain` over a small enum of declaration kinds (struct/interface/function/method — kibitzer will need a per-language mapping since not every language has all four).
4. Naming rules as "declarations of kind K matching condition C (e.g. implements interface X) must match name pattern Y" — condition + pattern, not a full DDD/architecture-role system (explicitly out of scope per requirements.md).

This is a cheap, low-risk shortcut: the schema decisions are already field-tested by
arch-go's own `arch-go.yml` self-check config, so kibitzer doesn't have to invent them from
scratch — but the answer to the open question in requirements.md is **no, arch-go's core
logic is not a reusable library** (CLI-only, and stdlib-coupled even if it were importable
cross-language).

---

## 4. LLM-authored tree-sitter queries vs. verified grammar sources

Pulled each target grammar's own `src/node-types.json` directly via `gh api` (the grammar's
own generated source of truth, not memory/guessing) for the three new languages in scope:

- **Python** (`tree-sitter/tree-sitter-python`, already a kibitzer dependency at 0.23):
  import syntax is spread across **six distinct node kinds** —
  `import_statement` (field `name`), `import_from_statement` (fields `module_name`, `name`,
  optional `wildcard_import` child), `relative_import`, `aliased_import`,
  `wildcard_import`, `future_import_statement`. Meaningfully more complex than Go's single
  `import_spec` or JS's single `import_statement`/`export_statement` that `import_graph.rs`
  currently handles — a naive one-node-kind query would silently miss `from x import y`,
  relative imports, and aliasing. The grammar ships its own [`queries/tags.scm`](https://github.com/tree-sitter/tree-sitter-python/blob/master/queries/tags.scm) and `queries/highlights.scm` as a maintainer-verified reference.
- **Java** (`tree-sitter/tree-sitter-java`, kibitzer dep at 0.23.5): single node kind
  `import_declaration`, **positional children** (no field names) of type `asterisk` /
  `identifier` / `scoped_identifier` — closer to Go's simplicity. Grammar also ships its
  own `queries/tags.scm`/`highlights.scm`.
- **Kotlin** (kibitzer depends on `tree-sitter-kotlin-ng` 1.1.0, which crates.io resolves to
  [`tree-sitter-grammars/tree-sitter-kotlin`](https://github.com/tree-sitter-grammars/tree-sitter-kotlin) — the actively-used org-maintained grammar, 2M+ crates.io downloads, distinct from and more heavily used than the older `fwcd/tree-sitter-kotlin` personal repo (191 stars, `tags.scm` for it is an [open, unresolved issue](https://github.com/fwcd/tree-sitter-kotlin/issues/158) as of this research — a real signal that Kotlin tree-sitter tooling is less mature than Go/JS/Python/Java generally): import syntax is a single positional (no field names) `import` node with `identifier`/`qualified_identifier` children. **No bundled `queries/` directory exists in this grammar's repo at all** — kibitzer would be the first to write an import-extraction query against it; there is no existing reference to crib from for this specific grammar.

**Recommended verification approach** (not just "be careful"): for each new language, before
writing extraction code, (1) pull `src/node-types.json` from the grammar's own pinned source
via `gh api repos/<owner>/<repo>/contents/src/node-types.json` and grep for the relevant node
kind(s); (2) if the grammar ships `queries/tags.scm` or `highlights.scm` (Python and Java do;
Kotlin's grammar does not), read those first — they encode a maintainer-verified mapping,
reducing the risk below the "bespoke LLM-authored query" pitfall flagged in the broader
project research; (3) write a unit test (matching kibitzer's existing test style in
`import_graph.rs`) that parses a small fixture file per import variant and asserts on
`tree.root_node().to_sexp()` output before wiring the extraction logic — this is a
mechanical, non-judgment-based check that node kind/field name assumptions are correct
against the actual pinned grammar version, not LLM memory of it. Python's and Kotlin's
grammars specifically warrant the most fixture coverage: Python for its six import node
kinds, Kotlin because there's no existing query to fall back on if the hand-written one is
subtly wrong.

---

## 5. Fork-or-adapt an existing OSS Rust project

Searched for existing Rust projects doing tree-sitter-based import-graph extraction for
Python/Java/Kotlin.

| Candidate | Verified via | Findings |
|---|---|---|
| [`iohub/codegraph-core`](https://github.com/iohub/codegraph-core) | `gh api` | Rust, tree-sitter-based, claims multi-language dependency graphs (Rust/Python/JS/TS/Go/C++/Java). **No license on the repo** (`license: null` — all-rights-reserved by default, blocks legal reuse/forking), 4 stars, created 2025-08, thin community signal. |
| [`MaibornWolff/DependaCharta`](https://github.com/MaibornWolff/DependaCharta) | `gh api` | Real, BSD-3-Clause, 17 stars, actively pushed (2026-08-21). Tree-sitter-based, no-compile, multi-language dependency viz — same *spirit* as this epic and a second independent validation (alongside arch-go) that the glob-matched-component approach is sound. But implemented in **Kotlin/JVM**, not Rust — nothing to fork at the code level. |
| `cargo-machete` | prior knowledge, not re-verified in depth | Rust-only; parses Rust via `syn`, not tree-sitter, not multi-language. Wrong tool for this problem. |

**Verdict: Not recommended.** Nothing found is simultaneously Rust, licensed for reuse,
mature, and scoped to Python/Java/Kotlin import extraction. `import_graph.rs::build()`
already has the per-language dispatch point (`build_go`/`build_js`) ready for extension —
adding `build_python`/`build_java`/`build_kotlin` following the same shape is the
straightforward path, informed by §4's verification approach per language.

---

## Overall Recommendation

**Build**, end to end, across all four in-scope epics. No dependency shortcut exists for any
of them in the Rust ecosystem at adequate maturity. Concretely:

1. Extend `ArchitectureConfig` with arch-go-inspired schema shapes (§3) — named/glob
   components, `should_only_depend_on`/`should_not_depend_on`, `should_only_contain`/
   `should_not_contain`, naming-condition-plus-pattern — rather than designing the schema
   from scratch.
2. Extend `import_graph.rs`'s existing per-language dispatch with `build_python`,
   `build_java`, `build_kotlin`, each verified against that grammar's own `node-types.json`
   (and `queries/tags.scm` where it exists) per §4, with fixture-based unit tests before
   trusting the extraction logic — Python needs the most coverage (six import node kinds),
   Kotlin the most caution (no existing reference query at all).
3. Extend `rules.rs`'s existing `node.kind()`-dispatch pattern for declaration
   enumeration (content/naming rules), rather than adding `ast-grep-core` as a dependency —
   consistent with the codebase's proven approach and structurally identical to how
   `arch-go` itself solves the same problem in Go.
