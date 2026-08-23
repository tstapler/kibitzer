# Research: Features, Edge Cases, and Unstated Needs — Architecture Rule Expansion

Research Agent 2 (Features) — SDD Phase 2, `tstapler/kibitzer`.
Repo: `/home/tstapler/code/github.com/tstapler/kibitzer`

## 1. Existing "similar feature" — exact patterns the new rule categories must match

Read in full: `src/architecture_checks.rs` (435 lines), `src/import_graph.rs` (365
lines), `src/config.rs` (370 lines). Also read `src/mcp.rs:1-180`, `src/mermaid.rs`
(162 lines, full), `src/glob.rs` (76 lines, full), and skimmed `src/rules.rs` for the
multi-language tree-sitter dispatch precedent.

### Checker trait shape (`src/architecture_checks.rs:19-34`)

```rust
pub trait ArchitectureChecker {
    fn name(&self) -> &str;
    fn check(&self, graph: &ImportGraph, config: &ArchitectureConfig) -> Vec<ArchFinding>;
}
pub fn registry() -> Vec<Box<dyn ArchitectureChecker>> {
    vec![Box::new(ImportCycleChecker), Box::new(LayeringChecker), Box::new(CouplingChecker)]
}
pub fn lookup(name: &str) -> Option<Box<dyn ArchitectureChecker>> { ... }
```

New rule categories (dependency rules, content rules, naming rules) should each be
a new zero-sized `struct` implementing `ArchitectureChecker`, added to `registry()`.
Note two places a new checker must be wired beyond the trait impl, both currently
hand-maintained lists rather than derived from `registry()`:

- `src/config.rs:177-195` `validate()` — looks up `architecture_checker` names via
  `crate::architecture_checks::lookup`, already generic, no change needed there.
- `src/mcp.rs:66-74` `SYNTAX_RULES_CHECKERS` — a **separate**, deliberately
  hand-maintained fixed list (comment: "so a future non-complexity native checker
  ... isn't silently swept into 'architecture'"). New `architecture_checker`s do NOT
  go in this list (it's for per-file complexity checkers only) — but
  `src/mcp.rs:78-92` `recommendation_for()` is a `match` on check name that DOES need
  a new arm per new checker if canned recommendation text is wanted, matching the
  existing `"import-cycles"` / `"layering"` precedent (`CouplingChecker` has none —
  precedent that a canned recommendation is optional, not mandatory).

### Finding shape (`ArchFinding`, lines 12-17)

```rust
pub struct ArchFinding {
    pub file: Option<PathBuf>,
    pub line: Option<usize>,
    pub message: String,
}
```

`file`/`line` are `Option` specifically because some findings are graph-wide
(`CouplingChecker` always emits `None`/`None`) while others tie to one edge
(`ImportCycleChecker`/`LayeringChecker` always emit `Some`). New checkers should
follow the same convention: package-content and naming-convention findings will
almost certainly have a concrete file+line (the offending declaration), so should
populate both, same as `LayeringChecker`.

### `ArchitectureConfig` shape (`src/config.rs:118-130`)

Currently exactly one field: `layers: Vec<String>`, `#[serde(default)]`,
plain `Deserialize` (no `Serialize` — config is read-only, never re-emitted). The
struct's own doc comment is the spec for `layer_of()`'s matching semantics. This is
the struct the requirements doc says must gain "arbitrary named-component
definitions (glob-mapped)" and "arbitrary per-component allow/deny dependency
rules," with `layers: Vec<String>` continuing to parse and desugar internally — so
the extension is additive fields on this struct, not a replacement.

### `layer_of()` — the precedent for glob/name matching ambiguity (lines 64-76)

```rust
fn layer_of(node: &str, layers: &[String]) -> Option<usize> {
    let segments: Vec<&str> = node.split('/').collect();
    layers.iter().position(|layer| segments.contains(&layer.as_str()))
}
```

`.position()` returns the **first** match — this is the existing, tested precedent
(`layering_ignores_packages_outside_the_declared_layers`, `layering_with_no_declared_layers_has_no_findings`)
for two of the edge cases the research question raises directly: a node matching
**zero** layers is silently ignored (not an error, not a finding); a node matching
**multiple** layer names (e.g. path contains both `domain` and `infra` as
segments) silently resolves to whichever layer is declared first in the config
list. **First-match-wins by declaration order is kibitzer's existing convention**
for this exact ambiguity class — the new component-glob matcher should either
follow this precedent explicitly (first-declared-component-wins) or explicitly
break from it with a stated reason, not leave it implicit.

### Glob matching is already a reusable utility

`src/glob.rs::matches_scope(rel_path: &str, scopes: &[String]) -> bool` (full file
read, 76 lines) converts `**`/`*`/`?` globs to anchored regex, already used for
`Check.scope`. This is very likely the correct primitive for "arbitrary named
components (glob-mapped)" — reuse rather than reimplement, though `matches_scope`
today only returns true/false against a list (OR semantics), not "which pattern in
a named-component map matched, and were there multiple matches" — the new
component resolver needs a variant that returns the matching component name(s), not
just a bool.

### `ImportGraph` shape (`src/import_graph.rs:11-31`)

Directory/package granularity, not file granularity (`nodes: BTreeSet<String>`,
`edges: Vec<ImportEdge>` with `from`/`to`/`file`/`line`). All three existing
checkers operate purely on this graph + `ArchitectureConfig`; they never re-read
source files. A content-rules checker ("package X must only contain structs") and a
naming-rules checker fundamentally cannot work from `ImportGraph` alone — they need
per-declaration AST information the graph doesn't carry (this matches the
requirements doc's "Rabbit Holes" note that content rules need "a new AST-inspection
pass distinct from both import_graph.rs and rules.rs"). Concretely this likely means
either (a) widening `ArchitectureChecker::check`'s signature to take something like
`&[PathBuf]` / parsed ASTs alongside the graph, or (b) a structurally distinct
checker trait/registry for content and naming rules that runs a separate AST pass.
Decide this explicitly in Phase 3 — it's an architecture decision, not just a config
schema one, since it changes the trait signature every existing checker implements.

### Per-language import extraction (`src/import_graph.rs`)

`build()` (lines 36-50) dispatches by extension: `.go` → `build_go`, JS-like
(`ts/tsx/js/jsx/mjs/cjs`) → `build_js`. No Python/Java/Kotlin branch exists yet —
confirmed by reading the full dispatch and the module's own doc comment ("Only Go
and TypeScript/JavaScript are extracted for now — Python/Kotlin/Java import
extraction can follow the same per-language dispatch pattern later," line 34-35).
Two structurally different strategies are already in the file, both real precedent
to choose between per new language:

- **Go** (`build_go`, lines 103-146): absolute import paths resolved via
  `go.mod`'s `module` directive + directory-relative concatenation
  (`go_package_import_path`). No cross-file resolution table needed — an import path
  string is checked directly against the set of known local package paths
  (`graph.nodes.contains(&import_path)`).
- **JS/TS** (`build_js`, lines 213-259): relative-only (`./`/`../` prefixes; bare
  specifiers like `import { z } from 'zod'` explicitly skipped, line 240-241,
  tested at line 355-364) resolved via filesystem canonicalization against a
  `known_files: HashMap<PathBuf, PathBuf>` built from the files-in-scope, trying
  multiple extension/`index.*` candidates.

Python's import model (`from ..pkg import x`, absolute-from-package-root imports,
no per-file manifest analogous to `go.mod` unless parsing `pyproject.toml`/`setup.py`)
doesn't cleanly fit either existing strategy — it needs a hybrid: absolute imports
resolved against a discovered package root (Go-like) plus relative-dot imports
resolved by dots-as-parent-hops (JS-like but with `.`/`..`/`...` counting instead of
path-relative resolution). Java (package-declaration + fully-qualified import,
closer to Go's model since `package` statements are absolute) and Kotlin (similar to
Java plus JS-style relative-free imports) are architecturally closer to `build_go`'s
approach than `build_js`'s.

### Test conventions (both files' `#[cfg(test)] mod tests`)

- `architecture_checks.rs` tests build `ImportGraph`/`ImportEdge` fixtures directly
  in-memory (no filesystem), one behavior per test, named
  `<subject>_<condition>_<expected-outcome>` (e.g.
  `layering_ignores_packages_outside_the_declared_layers`).
- `import_graph.rs` tests write real temp-directory fixtures to disk (`tmp_dir`/
  `write` helpers, PID+atomic-counter-namespaced dirs, explicit
  `std::fs::remove_dir_all` cleanup) and call `build()` end-to-end — necessary since
  extraction is fundamentally file-system + tree-sitter-parse driven, unlike the
  checkers which operate on an already-built in-memory graph. New per-language
  extraction tests should follow the `import_graph.rs` disk-fixture pattern; new
  checker-logic tests should follow the `architecture_checks.rs` in-memory pattern.
  This split is a real convention, not incidental — content/naming checkers, which
  need real source (not just a graph), will need to decide which convention theyre
  closer to per the trait-signature decision above.

## 2. Edge cases per new rule category

### Dependency rules (arbitrary named-component allow/deny)

- **Zero components match a package.** Direct analogue of `layer_of()` returning
  `None` today — existing precedent is "silently ignored, not an error, not a
  finding" (tested: `layering_ignores_packages_outside_the_declared_layers`). The
  new dependency-rule checker should very likely keep this behavior for
  consistency, but it's a real decision point: depguard/go-arch-lint users
  sometimes *want* an unmatched-package warning (a `default: deny` style catch-all,
  which `go-arch-lint` supports) — note as a design choice, not assume silence is
  obviously right for the new checker just because `layer_of` does it.
- **Multiple overlapping component globs match one package.** No existing
  multi-glob-per-node precedent to reuse (`layer_of` matches by *exact segment
  name* against a flat list, not by glob, so there's no "two globs both match"
  case in the current code — this is genuinely new ambiguity the layering checker
  never had to resolve). Options: first-declared-wins (consistent with
  `layer_of`'s `.position()` semantics), most-specific-glob-wins (longest match —
  what most path-based override systems, e.g. `.gitignore`/`CODEOWNERS`, do), or
  reject at config-validation time as an error (`validate()` in `config.rs` is the
  established place for schema-level rejections — e.g. the `set_count > 1` mutual
  exclusivity check at lines 150-157). Given `go-arch-lint`/`depguard` both allow
  overlapping component definitions with defined precedence rules, and kibitzer's
  own `validate()` already rejects config ambiguity elsewhere (mutually-exclusive
  `command`/`checker`/`architecture_checker`), a config-time validation error for
  overlapping globs (rather than silent runtime tie-breaking) is worth strong
  consideration as the safer default — it fails loud at config-load instead of
  producing silently different findings depending on declaration order.
- **A rule references a component name that isn't declared.** Not analogous to
  anything in the current single-list `layers` model (there's no separate
  name-to-rule indirection today). Should be a `validate()`-time error, matching
  the existing pattern of `config.rs:166-176` rejecting an unknown `checker` name
  at load time rather than failing at check-run time.
- **A rule allows A→B while another rule denies A→B (same pair, conflicting).**
  Needs an explicit precedence policy (e.g. "deny wins," which is `depguard`'s
  actual behavior — an explicit deny always overrides a broader allow). Undecided
  in the requirements doc; flag for Phase 3.

### Package-content rules (`arch-go`'s `contentsRules` shape)

- **A file with zero top-level declarations** (e.g. a doc-comment-only file, a
  file that's 100% `_test.go`-style, or a build-tag-gated file that's effectively
  empty for the current build). No existing content-inspection precedent in this
  codebase to compare against — `rules.rs`'s complexity checks (`syntax-rules`)
  operate per-*declaration* (function/method), not per-*file*, so they simply
  produce zero findings for a declaration-less file; a content rule ("package X
  must only contain structs") needs to decide whether an empty file trivially
  satisfies "only contains structs" (vacuous truth) or is itself worth flagging as
  suspicious/dead weight. `rules.rs`'s existing behavior (silently zero findings,
  no special-case) is the natural default to match unless there's a specific
  reason to diverge.
- **A package spanning multiple files with conflicting content** (e.g. one file
  in `internal/model/**` has only structs, a second file in the same package adds
  a function). This is squarely the scenario `contentsRules` exists to catch — the
  edge case is *reporting*: should the finding attribute to the offending
  file+line of the violating declaration (consistent with every existing
  checker's `Some(file)`/`Some(line)`), or bundle all violations for the package
  into one finding (like `ImportCycleChecker`'s one-finding-per-cycle, not
  one-per-edge)? Per-declaration findings match kibitzer's dominant convention
  (`LayeringChecker`, `rules.rs`'s complexity checks) more closely than the
  cycle-checker's bundling, which is the exception justified by a cycle being
  inherently a multi-edge structure.
- **Language-specific ambiguity in "what counts as a declaration."** `rules.rs`'s
  per-language dispatch (lines 226-305, `LangRuleConfig`) already has extensive
  precedent for how differently each grammar exposes declarations — e.g. Kotlin's
  `if_expression` exposing only `condition` as a named field vs. every other
  grammar exposing `consequence`/`alternative` too (comment at line 418-420). A
  content-rule checker inspecting "does this package contain only structs" will
  hit the same per-grammar irregularity for "what is a struct/type declaration" and
  should budget for it, not assume one AST-walk generalizes across languages.

### Naming-convention rules (`arch-go`'s `namingRules`, `cht-go-lint`'s naming category)

- **A type implementing multiple interfaces with different naming requirements**
  (the question posed directly). No precedent in this codebase at all — nothing
  today inspects interface implementation relationships (`ImportGraph` doesn't
  model types/interfaces, only package-level import edges). This needs new
  infrastructure to know "which interfaces does type T implement" before naming
  rules can even be conditioned on it — likely out of reach for a first cut of
  naming rules unless the rule model is scoped to path/package-glob-conditioned
  naming (e.g. "everything in `handlers/**` matching `type *Handler struct` must
  end in `Handler`") rather than interface-conditioned naming. Flag this as a
  potential scope-narrowing decision for Phase 3: interface-conditioned naming
  rules are a materially harder feature than glob-conditioned ones, and the
  conflicting-multi-interface case may be legitimately out of scope for v1.
- **Case-convention ambiguity across languages.** Go idiomatically uses
  `CamelCase`/`camelCase` with no underscores; Python idiomatically uses
  `snake_case`; Java/Kotlin use `camelCase` methods + `PascalCase` types. A single
  cross-language naming-rule config needs either per-language default conventions
  or fully explicit per-rule regexes — bare "must match regex X" config (no
  language awareness) risks false positives the moment the same rule is reused
  across a polyglot repo, which is explicitly this feature's stated goal.

## 3. Multi-language import-graph extraction — edge cases

Confirmed via `Cargo.toml` (`grep -n "tree-sitter" Cargo.toml`): the three grammars
already present as dependencies are `tree-sitter-python = "0.23"`,
`tree-sitter-java = "0.23.5"`, and **`tree-sitter-kotlin-ng = "1.1.0"`** (not
`tree-sitter-kotlin` — the crate name in `src/rules.rs`'s test code is literally
`tree_sitter_kotlin_ng::LANGUAGE`, confirmed at `src/rules.rs:1173`). Any Phase 3
plan referencing the Kotlin grammar dependency must use this exact crate name.

- **Relative vs. absolute imports.** Go and Java/Kotlin lean absolute
  (package-declaration-rooted); JS/TS is relative-or-bare (kibitzer's `build_js`
  already special-cases this by skipping bare specifiers entirely, line 240-241);
  Python is genuinely mixed within one file (`import pkg.mod` absolute alongside
  `from . import x` / `from .. import y` relative-by-dot-count). None of the two
  existing extraction strategies (`build_go`'s absolute-path-string match,
  `build_js`'s filesystem-canonicalization-of-relative-specifier) handles Python's
  dot-count relative form directly — needs new resolution logic, not a reuse of
  either existing helper as-is.
- **Wildcard imports** — Python `from x import *`, Kotlin `import x.*`. Both
  grammars will expose these as their own import-statement shape (Python:
  `import_from_statement` with a `wildcard_import` node instead of named imports;
  Kotlin: an `import_header` whose path ends in a wildcard rather than an
  identifier). These still name the *source module* being imported from/into —
  the graph-edge extraction (which only needs the *module path*, not the specific
  imported symbols) is likely unaffected as long as the extractor doesn't assume
  every import statement has a named-symbol list to walk (a plausible bug: an
  extractor written against Python's `from x import y, z` shape that expects a
  child symbol list could error or silently skip on `from x import *`, whose AST
  shape is a sibling `wildcard_import` node instead of an `dotted_name`/`aliased_import`
  list — needs a specific test case, not just "if node has children, keep going").
- **Aliased imports** — `import foo as bar` (Python), `import com.foo.Bar as Baz`
  (Kotlin has import aliasing too via `as`). The alias name is irrelevant to graph
  construction (the edge should point at the *real* module path, `foo`/`com.foo.Bar`,
  not the local alias `bar`/`Baz`) — this is actually the *simple* case as long as
  extraction reads the "source" path field rather than the bound name, matching
  how `build_js`'s `collect_js_imports` already reads only `child_by_field_name("source")`
  and never looks at the imported-binding names at all (lines 167-178). Java doesn't
  have import aliasing (no `as` clause) — not applicable there.
- **Conditional/dynamic imports** — Python's `importlib.import_module("...")` /
  `try: import x except ImportError: import y`, JS's `import()` dynamic-import
  expression or `require()` inside a conditional. Neither existing extractor
  (`build_go`/`build_js`) handles anything beyond static `import`/`import_statement`
  syntax nodes — `build_js`'s `collect_js_imports` only matches `import_statement`/
  `export_statement` node kinds (line 168), so a `require()` CommonJS call or a
  dynamic `import()` expression is already silently invisible to kibitzer's graph
  today, for JS. This is existing, accepted scope-narrowing, not a regression to
  fix as part of this feature — but the requirements doc doesn't mention it, so
  worth flagging explicitly: Python/Java/Kotlin extraction should match this same
  "static imports only" scope for consistency, and any dynamic-import gap should be
  named as a known limitation rather than silently inherited without comment.
- **Test-only imports that shouldn't count toward architecture violations.**
  No existing precedent for excluding test files from the import graph — `build()`
  takes whatever `files` list it's handed (already filtered upstream by whatever
  caller assembled it) and extracts unconditionally; there's no `_test.go`/`*.test.ts`/
  `test_*.py`-aware filtering inside `import_graph.rs` itself. This means the
  exclusion point, if wanted, is the caller (`walk_and_collect_files` /
  `architecture_assessment`'s `scope` glob in `src/mcp.rs`), not new logic inside
  the per-language extractors — worth confirming whether current Go/JS layering
  checks already produce false positives from test-only imports (e.g. a test file
  in a `domain` package importing test-fixture helpers from `infra` — layering
  would flag this today, since nothing distinguishes test files). This is a
  pre-existing gap the new work would inherit and cross the same way for the three
  new languages unless explicitly addressed — flag as a candidate improvement
  regardless of which languages are in scope, since it isn't new to Python/Java/Kotlin.

## 4. Unstated needs (forward-looking observations, not scope creep)

- **Per-rule severity for the new rule categories.** `config.rs`'s `Severity` enum
  (`Blocking`/`Advisory`, lines 9-14) is currently a property of the whole `Check`
  wrapping an `architecture_checker` — i.e. severity is set once per *checker
  invocation* in `.claude/inspect.json`, not per individual rule inside
  `ArchitectureConfig`. A user adopting arbitrary named-component dependency rules
  will very likely want some rules blocking ("services must never import
  `net/http` directly") and others merely advisory ("prefer domain not importing
  infra, but grandfather it for now") *within the same checker run* — go-arch-lint
  supports exactly this per-rule severity distinction. Today's model can't express
  it without splitting into multiple `architecture_checker` check entries with
  disjoint scopes, which doesn't compose cleanly for pairwise dependency rules
  (a rule is about a *pair* of components, not a file glob `scope` the way
  `Check.scope` works). Flag for Phase 3 as a likely-wanted schema extension:
  per-rule `severity` inside `ArchitectureConfig`'s new rule structs, separate
  from the outer `Check.severity`.
- **Incremental/diff-aware architecture checking.** Explicitly out of scope per
  the requirements doc, and `config.rs:186-194`'s `validate()` actively *enforces*
  batch-only today (rejects any `architecture_checker` check with a
  non-`"batch"` trigger, with the stated rationale "rebuilding the import graph on
  every `PostToolUse` edit is too expensive," comment at `config.rs:56-58`). This
  is a real, deliberate constraint, not an oversight — but it's worth naming as a
  forward-looking tension: kibitzer's core value proposition elsewhere (per
  `docs/checking-invocations.md` and the diff-aware `{changed_lines}` /
  git-HEAD-baseline-downgrade mechanism referenced in `check.rs`, per
  `docs/check-ideas.md:107-108`) is being the "diff-aware" checker, and architecture
  checks are the one category that's whole-repo batch-only. As the new rule
  categories multiply the number of findings a whole-repo run can produce (four
  new rule categories vs. today's three checkers), the batch-only UX gap between
  "architecture checks" and "everything else kibitzer does" gets more visible, not
  less. Not a reason to implement incremental checking now — the per-edit
  graph-rebuild cost argument still holds — but worth a forward-looking note in
  the plan doc so it isn't rediscovered as a surprise later.
- **No evidence in `docs/check-ideas.md` of user-reported architecture pain.**
  Read in full (138 lines). The doc's own methodology section states directly, after
  two separate corpus-mining passes (free-text correction-phrase mining across
  3-day and unrestricted windows, plus structural edit-churn mining across 2,957
  matches): *"Zero matches ... touched dependency direction, layering, circular
  imports, or module-boundary violations — no architecture-relevant signal at all,
  not even at single-occurrence strength."* This is a genuine negative finding
  worth carrying into Phase 3/4: the case for this feature is Tyler's stated
  strategic goal (persona-linter-for-everything, replacing per-language external
  tools) and direct feature-parity comparison against the four external tools, not
  an evidenced recurring-mistake pattern the way `duplicate-code` was justified
  (`check-ideas.md:124-136`, implemented "ahead of a confirmed transcript
  occurrence" but at least backed by a full-corpus backtest that "did turn up real
  duplication"). No equivalent backtest exists yet for architecture rules. Not a
  reason to not build it — but Phase 4 (validate) should not claim evidenced-need
  backing for this feature that the transcript-mining data doesn't support.
- **`docs/checking-invocations.md`** (94 lines, read in full) has no
  architecture/content/naming-specific content — it's entirely about how to verify
  kibitzer's hook actually fired via Claude Code transcript `attachment` records,
  unrelated to this feature's scope. No signal to report from this file beyond
  confirming it doesn't contain hidden requirements.

## Key files (with line references)

- `/home/tstapler/code/github.com/tstapler/kibitzer/src/architecture_checks.rs` —
  `ArchitectureChecker` trait (19-22), `registry()`/`lookup()` (24-34),
  `layer_of()` (64-76, the first-match-wins precedent), `LayeringChecker` (76-119).
- `/home/tstapler/code/github.com/tstapler/kibitzer/src/import_graph.rs` —
  `build()` dispatch (36-50), `build_go` (103-146), `build_js` (213-259),
  doc comment noting Python/Kotlin/Java are unextracted (34-35).
- `/home/tstapler/code/github.com/tstapler/kibitzer/src/config.rs` —
  `ArchitectureConfig` (118-130), `Severity` (9-14), `validate()` (140-206).
- `/home/tstapler/code/github.com/tstapler/kibitzer/src/glob.rs` — `matches_scope`
  (38-45), reusable glob-matching primitive for component definitions.
- `/home/tstapler/code/github.com/tstapler/kibitzer/src/mcp.rs` —
  `SYNTAX_RULES_CHECKERS` (66-74), `recommendation_for` (78-92),
  `architecture_assessment` tool (130-180+).
- `/home/tstapler/code/github.com/tstapler/kibitzer/src/mermaid.rs` — full
  dependency-diagram renderer, would need a legend/highlight extension for new
  finding types (e.g. distinguishing a dependency-rule violation edge from an
  import-cycle edge).
- `/home/tstapler/code/github.com/tstapler/kibitzer/src/rules.rs` — per-language
  `LangRuleConfig` dispatch pattern (226-305), the precedent for how much
  per-grammar irregularity to expect when extending to Python/Java/Kotlin.
- `/home/tstapler/code/github.com/tstapler/kibitzer/Cargo.toml:21-27` — exact
  tree-sitter grammar crate names/versions, notably `tree-sitter-kotlin-ng = "1.1.0"`.
- `/home/tstapler/code/github.com/tstapler/kibitzer/docs/check-ideas.md` — negative
  finding: no evidenced architecture-relevant user-correction signal across two
  corpus-mining passes.
- `/home/tstapler/code/github.com/tstapler/kibitzer/docs/checking-invocations.md`
  — no architecture-specific content; hook-verification methodology only.
