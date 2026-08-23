# Research: Pitfalls — kibitzer architecture-linting expansion

Research Agent 4 (Pitfalls), SDD Phase 2. Scope: what commonly breaks in this
type of feature, stack-specific risk, and what Phase 3 (plan) / Phase 5
(implement) should explicitly design against.

## 1. Tree-sitter grammar pitfalls for IMPORT node kinds (Python/Java/Kotlin)

`docs/syntax-rules.md` documents function/control-flow node kinds, verified
against real `to_sexp()` output per its own stated discipline ("verified
against each grammar's real `to_sexp()` output, not guessed by analogy" —
`src/rules.rs:55`). Import-statement node kinds are a **different node
family** and have not been touched by that verification pass. Below is what
was recovered by fetching the actual `node-types.json` for each pinned
grammar's upstream repo — this is stronger than analogy but **still weaker
than the repo's own bar**: it wasn't run through `to_sexp()` against the
exact pinned crate version (`Cargo.toml`: `tree-sitter-python = "0.23"`,
`tree-sitter-java = "0.23.5"`, `tree-sitter-kotlin-ng = "1.1.0"`), only
fetched from each repo's default branch. **Phase 5 must re-verify with a real
`to_sexp()` dump before writing extraction code**, matching the existing
`rules.rs` citation discipline — treat everything below as a strong lead, not
a substitute for that step.

### Python (fetched `tree-sitter/tree-sitter-python` `src/node-types.json`)

- `import_statement` — field `name` (multiple, required), accepting
  `dotted_name` or `aliased_import`.
- `import_from_statement` — field `module_name` (single, required, accepts
  `dotted_name` or `relative_import`), field `name` (multiple, optional,
  accepts `dotted_name`/`aliased_import`), plus an unfielded optional
  `wildcard_import` child (`from x import *`).
- `relative_import` wraps `import_prefix` (the leading dots) + `dotted_name`.
- `future_import_statement` (`from __future__ import x`) is a **separate**
  node kind from `import_from_statement`, same field shape — a naive walk
  that only matches `import_from_statement` will silently miss `__future__`
  imports. Likely irrelevant to local-package graph edges but worth a
  one-line exclusion note rather than a silent gap.
- Both `import_statement` and `import_from_statement` use **named fields**
  (`name`, `module_name`) unlike the Go/JS extractors' pure `child_by_field_name`
  pattern already in `import_graph.rs` — so this is the easy case, structurally
  closest to the existing Go/JS code.

### Java (fetched `tree-sitter/tree-sitter-java` `src/node-types.json`)

- `import_declaration` is the single node kind for all imports (plain,
  qualified, and `import static`).
- **It declares no named fields at all** — children are positional:
  `identifier`, `scoped_identifier`, or `asterisk` (for `import java.util.*`).
- This is the *same gotcha class* `syntax-rules.md` documents for Kotlin
  functions (`function_value_parameters`/`function_body` being positional,
  not field-based) — except here it hits imports specifically, in the one
  language (Java) whose function/control-flow nodes `syntax-rules.md`
  describes as "proper fields (Go-like, no wrapper)" (`docs/syntax-rules.md:77-80`).
  **Do not assume Java import extraction can reuse the same
  `child_by_field_name` pattern the rest of that grammar's fields use** —
  it needs a `kind()`-filtered walk like Kotlin's `kotlin_params`/`kotlin_body`
  helpers, not a copy of the Go extractor's field-lookup pattern.
- `import static` is not visibly distinguished by node kind in this data —
  worth confirming via a live `to_sexp()` whether a `static` keyword appears
  as a sibling token or is otherwise detectable, since `import static` names
  a member, not a package, and including it as a package-graph edge would be
  wrong.

### Kotlin (fetched `tree-sitter-grammars/tree-sitter-kotlin`, the repo
`tree-sitter-kotlin-ng` on crates.io points to)

- The node kind is **`import`**, not `import_header`/`import_list` as in the
  older `fwcd/tree-sitter-kotlin` grammar — confirms `syntax-rules.md`'s
  existing warning not to assume node names by analogy from a different
  fork of the "same" grammar; this crate is `-ng`, and its import node name
  differs from the community grammar most search results and tutorials
  describe.
- `import` has **no named fields**; accepts `identifier` or
  `qualified_identifier` as positional children — same positional-children
  pattern as Kotlin's function nodes already documented in `syntax-rules.md`,
  so the existing `kotlin_params`-style `kind()`-filtered walk is the right
  template to reuse here too.
- `import`, `package_header`, and top-level `statement` are siblings directly
  under `source_file` — no wrapping `import_list` container node exists in
  this grammar (unlike Java, which also has no wrapping list but for
  different reasons). A walk that assumes an intermediate import-list node
  (as some other grammars have) will find nothing.
- Not yet confirmed from static schema data: how `import x.y.Z as W`
  (aliased import) is represented — the fetched `node-types.json` view didn't
  surface an alias field/child. Flag explicitly for the `to_sexp()`
  verification pass; if aliasing changes the local package this resolves to,
  getting it wrong produces a wrong edge, not just a missed one.

### Net verification task for Phase 5

For each of the three languages, produce one small fixture file with a
plain import, a from/qualified import, and (Python/Java) a wildcard import,
run it through `to_sexp()`, and paste the real output into the extraction
module's doc comment — same discipline as `rules.rs:55` and
`rules.rs:158`. Do this **before** writing the walker, not as cleanup after.

## 2. General architecture-linter design mistakes to design against

- **Glob ambiguity / multi-component match.** If a file matches more than
  one named component's glob (e.g. `handlers/admin/**` and a broader
  `handlers/**`), the schema needs an explicit precedence rule (most-specific
  glob wins, first-declared wins, or reject as a config error at load time).
  Silently picking "first match in declaration order" is the easy
  implementation but is exactly the kind of implicit behavior that produces
  confusing, hard-to-explain findings later — `config.rs`'s existing
  `validate()` already treats ambiguous/invalid config as a hard load-time
  error (mutually-exclusive check fields, unknown checker names) rather than
  silently picking a default; the new component-mapping code should follow
  that same precedent, not introduce a new silent-fallback style.
- **Silent misconfiguration — typo'd component name in a dependency rule.**
  A rule referencing a component name that doesn't exist in the named-component
  map (typo, stale rename) should be a **load-time validation error**, not a
  rule that silently never fires. `config.rs::validate()` already does this
  for `checker`/`architecture_checker` names (`rejects_unknown_checker_name`,
  `rejects_unknown_architecture_checker_name` tests at `src/config.rs:286`,
  `src/config.rs:344`) — the new dependency-rule schema must extend the same
  validation pass to component names referenced in rules, or a whole rule
  category goes dark with zero signal that it did.
- **Performance cliff on large repos.** The current model already restricts
  architecture checks to `batch` trigger only, enforced at config-validation
  time (`config.rs:186-194`, `rejects_architecture_checker_on_non_batch_trigger`
  test) — this guardrail is structural, not just documented, and the new
  checker categories (package-content, naming-convention) must register
  through the same `ArchitectureChecker`/`architecture_checker` path so they
  inherit it automatically rather than being wired as a new `checker`
  (per-file, `PostToolUse`-eligible) by mistake, which would reintroduce the
  exact "rebuild graph on every edit" cost the NFR calls out.
  Content/naming rules that need whole-repo AST re-parse (not just the
  already-built import graph) are a **new** cost the existing layering/cycle/
  coupling checkers don't pay — they consume the graph, not the ASTs. Confirm
  whether the batch run can reuse the ASTs already parsed for import-graph
  extraction rather than re-parsing every file a second time for
  content/naming rules.
- **Rule-precedence ambiguity when multiple rules apply to the same edge.**
  With a single global `layers: Vec<String>` list, "higher may depend on
  lower" is the only possible rule per edge — no precedence question exists
  today. Arbitrary named-component allow/deny rules reopen it: if component
  A has both an explicit "A may depend on B" allow rule and a broader "A may
  not depend on anything in group G" deny rule and B ∈ G, which wins? Pick
  and document a resolution order (e.g. most-specific rule wins, or deny
  always wins over allow) at the schema-design stage (Phase 3), not
  discovered ad hoc during implementation — this is a design decision, not a
  bug to fix later.

## 3. Backward-compatibility regression risk: proving `layers` desugars identically

"No crash" is not evidence of identical behavior — the right bar is
**identical findings**, since `LayeringChecker::check` (`src/architecture_checks.rs:88-118`)
is a pure function of `(graph, config)` and already has a precise,
quotable semantics to hold constant:

> `layer_of` (`src/architecture_checks.rs:69-74`): a package belongs to the
> **first** layer whose name matches **any `/`-separated path segment
> exactly** — not a glob, not a prefix/suffix match, not case-insensitive.
> Packages matching no layer are silently ignored by the checker
> (`None` short-circuits via `?` in `check`, `src/architecture_checks.rs:96-97`).

The recommended test design, using fixtures the module already has:

- **Golden/characterization test, not a fresh handwritten one.** Reuse the
  existing `import_graph.rs` test fixtures (`go_import_graph_finds_a_two_package_cycle`,
  `ts_import_graph_finds_a_two_module_cycle`, `src/import_graph.rs:294-352`)
  and the existing `layering_*` tests (`src/architecture_checks.rs:337-379` —
  `layering_flags_a_reverse_dependency`, `layering_allows_a_forward_dependency`,
  `layering_ignores_packages_outside_the_declared_layers`,
  `layering_with_no_declared_layers_has_no_findings`). For each, run the
  *old* `layers: Vec<String>` path and the *new* desugared-to-named-components
  path over the identical graph and assert the two `Vec<ArchFinding>` results
  are equal (`ArchFinding` already derives `PartialEq, Eq` —
  `src/architecture_checks.rs:12`), not just "both empty" or "both non-empty."
- **Specifically test the exact-segment-match semantics**, since this is the
  one place the new glob-based component model could silently diverge: a
  package path segment `domain2` must NOT match a layer/component named
  `domain` under the old semantics; confirm the new glob-mapped desugaring
  reproduces that (a naive `**/domain/**` translation is not obviously
  equivalent to segment-exact matching for edge cases like a segment that is
  a substring of another, or a layer name containing `/`-illegal or
  glob-special characters). Write this as an explicit regression case, since
  it's exactly the kind of thing "old tests pass by coincidence" would miss —
  the existing `layering_ignores_packages_outside_the_declared_layers` test
  doesn't probe this because its fixture packages don't have adversarial
  segment names.
- **Property-based option**: if a property-testing crate is already a dev
  dependency (check `Cargo.toml`), a generator over small synthetic graphs +
  layer lists, asserting `old_check(graph, layers) == new_check(graph,
  desugar(layers))` for every generated case, is stronger than a handful of
  golden cases and cheap to add given `ArchFinding`'s existing `Eq`. If no
  property-testing crate is already a dependency, don't add one just for
  this — the golden-file approach above is sufficient and matches the
  existing test style in both files.

## 4. Scope-creep / sequencing risk across the 4 gaps

Gap 1 (arbitrary named-component dependency rules) is being designed before
gaps 2–3 (package-content and naming-convention rule categories) have
concrete schema needs on the table. Concrete risk: gap 1's schema will
naturally center on **components as sets of paths/packages** (for
dependency-direction rules between them), but gap 2 (package-content:
"package X must only contain structs") and gap 3 (naming-convention:
"structs implementing X must end in Y") need a different indexing shape —
gap 2 needs per-*file-content* classification within a component (what kinds
of top-level declarations does this package contain), and gap 3 needs
per-*symbol* matching (type name × implemented-interface), not per-package.
If gap 1 ships a component-definition schema shaped only for
dependency-direction rules (e.g. "component = glob → resolves to a set of
package/directory nodes in the import graph"), gaps 2–3 may need to either:

- reuse the same named-component definitions (good — the intended design
  per the requirements doc's phrasing "arbitrary named-component
  definitions... + arbitrary per-component allow/deny dependency rules,"
  which reads as components being reused as the shared indexing primitive
  across all rule categories), or
- discover the component definition needs additional metadata (e.g. does a
  component's glob match whole directories only, or also match individual
  files for content rules that need file-granularity, not just
  directory/package-granularity like the current `ImportGraph` nodes) —
  which would mean revising gap 1's schema after gap 2/3 design work,
  potentially re-touching already-implemented dependency-rule code and its
  config parsing/validation.

**Sequencing recommendation for Phase 3**: design the named-component
schema (gap 1) *together with* a concrete worked example of a package-content
rule (gap 2) and a naming-convention rule (gap 3) referencing the same
component definitions, before locking the schema — don't design gap 1 in
isolation and treat 2–3 as "just new rule types plugged into an existing
component model" until that's actually been proven on paper. If Phase 3
can't produce a satisfying worked example for gap 2/3 against the gap-1
schema, that's the signal to revise the schema then, not during Phase 5.
Also worth flagging: `ImportGraph` nodes are directory/package-granularity by
design ("too dense at file granularity to reason about cycles or layering" —
`src/import_graph.rs:20`) — if package-content rules need file-granularity
("package X must only contain structs" implies looking inside each file, not
just at the package node), that's evidence gaps 2–3 need a second index
(a file-content classification separate from `ImportGraph`) rather than
extending `ImportGraph` itself, which would blur its single existing
responsibility.

## 5. False-positive history in this codebase (highest-signal prior art)

`docs/check-ideas.md`'s correction-mining methodology found **zero** evidenced
recurring-mistake patterns for kibitzer checks generally, including **zero**
architecture/layering/circular-import/module-boundary signal in a
2,957-row structural edit-churn scan (`docs/check-ideas.md:83-86`) — so there
is no prior transcript-evidenced false-positive pattern specific to
architecture rules to build against yet. That absence is itself informative:
it means the risks below are anticipated from mechanism, not backed by a
confirmed prior incident, and should be treated with that caveat.

The highest-signal *actual* false-positive history in this repo is
`docs/go-primitive-obsession-false-positives.md`, which documents a
confirmed, root-caused mechanism directly relevant to the new package-content
and naming-convention rule categories:

- **Root cause**: `primitive-obsession`'s checker re-parses and re-scans the
  **entire current file on disk** on every `Edit`/`Write`, with no diff
  awareness — it reports every matching pattern found anywhere in the file,
  regardless of whether the triggering edit touched it
  (`docs/go-primitive-obsession-false-positives.md:9-29`). Confirmed
  incidents: a pure-deletion edit still got flagged for *other*,
  untouched signatures in the same file (2026-08-10 entry); pre-existing,
  unchanged signatures re-fired on unrelated edits to the same file
  (second 2026-08-10 entry).
- **Direct relevance to gaps 2–3**: package-content rules ("package X must
  only contain structs") and naming-convention rules ("structs implementing X
  must end in Y") are, by nature, **whole-package or whole-symbol-set**
  checks, not diff-scoped ones — there's no obvious "which lines changed"
  scoping for "does this package's *entire* content set satisfy a shape
  constraint." But per `config.rs:54-58` and the NFR, these run only on
  `batch` (whole-repo), not `PostToolUse` — so the specific whole-file-rescan
  *false-positive* mechanism (re-flagging untouched code as if newly
  introduced) mostly doesn't apply here as it's architected. The
  **analogous** risk that does carry over: a batch run reporting the same
  finding on unrelated future batch runs even though nothing about that
  finding changed, with no way for an agent/user to distinguish "still
  broken, unaddressed" from "this specific finding was already seen and
  triaged" — the existing checkers have no such de-duplication/acknowledgment
  concept either, so this isn't a new gap introduced by this feature, but the
  new rule categories will surface it more, since content/naming violations
  on a legacy codebase could produce a large, static finding set that
  batch-fires unchanged on every run. Worth flagging to Phase 3 as a UX
  question (not a hard requirement): should a first-run baseline/suppression
  concept exist, or is "re-report identically every batch run, let the human
  or agent triage" acceptable for v1 (matching the "no suppression
  mechanism" philosophy already stated in `docs/reporting-false-positives.md:51-56`
  for other checkers)?
- No confirmed false-positive history exists yet for
  `ImportCycleChecker`/`LayeringChecker`/`CouplingChecker` specifically (they
  postdate the correction-mining pass, or simply haven't misfired yet) — so
  gap 1's arbitrary-dependency-rule extension has no prior incident to build
  a regression test from beyond the golden-file approach in section 3.

## Summary of concrete Phase 3/5 action items

1. Before writing Python/Java/Kotlin import extraction: dump real `to_sexp()`
   output for each language's import forms against the exact pinned grammar
   version and cite it in the extraction module, per `rules.rs`'s existing
   discipline. Java and Kotlin imports are positional-children, not
   field-based — do not copy the Go/JS field-lookup pattern for those two.
   Also confirm Python's `future_import_statement` and Java's `import static`
   handling explicitly rather than by omission.
2. Extend `config.rs::validate()` to reject (at load time) any dependency/
   content/naming rule referencing an undeclared component name, and define
   an explicit glob-overlap precedence rule for components — both following
   the existing "hard error over silent no-op" precedent already in that
   function.
3. Confirm the new rule categories register as `architecture_checker`s (not
   per-file `checker`s) so the existing batch-only trigger validation applies
   automatically; determine whether they can reuse ASTs already parsed for
   import-graph extraction instead of re-parsing per rule category.
4. Define and document rule-precedence (allow vs. deny, specific vs. general)
   for arbitrary dependency rules at the schema design stage.
5. Write a golden-file/characterization regression test asserting
   `layers`-desugared findings are `Eq`-identical to the old code path's
   findings, including an adversarial fixture probing segment-exact-match
   edge cases the current tests don't cover.
6. Design gap 1's named-component schema against a worked gap-2 and gap-3
   example before locking it, rather than sequencing gaps 2–3 as an
   afterthought; watch specifically for a directory/package-granularity vs.
   file-granularity mismatch between `ImportGraph`'s existing node
   granularity and what content rules need.
