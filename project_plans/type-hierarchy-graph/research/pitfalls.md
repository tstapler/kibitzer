# Research: Pitfalls (Agent 4)

Research question: what commonly goes wrong building a type-hierarchy/inheritance graph
feature like this, specifically in kibitzer's existing codebase and conventions?

## 1. `resolve_call_edges`'s documented v1 limitations would bite `type_edges` the same way

`resolve_in` (`src/arch_model.rs:455-472`) is the shared resolution primitive
`resolve_one_call_edge` (`src/arch_model.rs:506-544`) calls: it prefers an unambiguous
same-package match, falls back to a *globally unique* cross-package match, and returns
`None` — never a guess — when a name is ambiguous at both tiers. The doc comment is explicit:
"resolution intentionally never guesses among multiple candidates."

This generalizes cleanly to `type_edges` (it's a name→id lookup over `SymbolIndex`, not
call-specific), but its accepted ceiling carries over identically:

- **Ambiguous same-named types across packages stay unresolved.** Two unrelated packages
  each declaring a `Base` type/interface — extremely plausible in a large Go/Java/Kotlin
  monorepo — means any `extends Base` outside both packages can't be resolved even though
  the source is syntactically unambiguous (an import/package-qualifier in the source
  *does* disambiguate it, but `resolve_in` doesn't consult import edges, only same-package-
  first-else-globally-unique). This is a **known, accepted ceiling for `CallEdge`**, not a
  bug — but for `type_edges` it's more consequential: a hierarchy is exactly the
  relationship #59/#60/#61 (Pull Up/Push Down, Collapse Hierarchy) need to walk correctly,
  and an unresolved (or worse, silently wrong) `extends` edge breaks those checkers' whole
  premise, not just one call-graph edge among thousands.
- **`SymbolIndex` here is built once from `packages` and is not qualifier-aware.** Note
  `file_import_aliases` (`src/arch_model.rs:191-200`) already exists precisely because
  `god_class`/`isp_fat_interface` hit this same ceiling for qualified locals and needed a
  second, alias-aware resolution path. A `pkg.Base`-qualified `extends`/`implements` clause
  (common in Java fully-qualified extends, or Go's `pkg.Type` embedding) should resolve via
  `file_import_aliases`-style alias lookup, not blind name matching, or it inherits the
  *pre*-`file_import_aliases` ceiling that motivated that fix in the first place.
- Recommendation: reuse `resolve_in`'s function signature/algorithm (build a `SymbolIndex`
  scoped to `Type`/`Interface` symbols only, mirroring `build_call_target_indexes`'s split
  of `functions_by_name`/`methods_by_name`), but route qualified names through
  `file_import_aliases` first, the same way a `pkg.Type` local already gets resolved
  elsewhere in the model, rather than treating `resolve_call_edges` as fully copy-pasteable
  as-is.

## 2. Real-world AST shapes likely to break a naive extends/implements extractor

Checked against `docs/backtest-repos.md`'s corpus (`kubernetes/kubernetes`,
`apache/cassandra`, `denoland/deno`+`microsoft/vscode` for TS, no Kotlin repo in the list —
see below):

- **Go: interface-embeds-interface is syntactically identical to struct embedding.** The
  requirements doc's own Rabbit Holes section already flags this (same anonymous-field
  AST shape for embedding a struct or an interface), but it's worth confirming from the
  existing extraction code: `go_struct_fields` (`src/symbol_extract.rs:668-699`)
  **explicitly skips anonymous/embedded fields today** — its doc comment says "An embedded
  field (anonymous — no `name` field at all) is skipped: embedding introduces promoted
  fields/methods this v1 extraction doesn't walk into." That means there is **no existing
  code path to lift from** for Go embedding extraction; `type_edges`'s Go extractor is new
  code, not a small addition to `go_struct_fields`, and needs its own walk over
  `struct_type`'s `field_declaration_list` collecting the fields that function currently
  discards. `kubernetes/kubernetes` is full of both struct-embeds-struct (`extends`) and
  interface-embeds-interface (`implements`-shaped, e.g. `io.ReadWriteCloser` composing
  `io.Reader`/`io.Writer`/`io.Closer`) — the corpus will exercise both immediately.
- **Java: interface-extends-interface, and multiple interfaces via `extends` (not
  `implements`).** A Java `interface Foo extends Bar, Baz` uses the `extends` keyword for
  interface-to-interface inheritance, with comma-separated multiple targets — different
  grammar shape from a class's single `extends` + separate `implements` list. A naive
  extractor keyed only on "class → extends (one), implements (many)" will mis-tag or drop
  this. `apache/cassandra` almost certainly has multi-interface-extends somewhere in its
  large interface set.
- **TypeScript: `implements` with a qualified + generic name** (`implements ns.Foo<T>`,
  or `extends Base<T, U>`) — the target identifier is nested inside a
  `type_arguments`/qualified-name AST shape, not a bare `identifier`. `microsoft/vscode`
  and `denoland/deno`'s TS stdlib both use generics heavily; a naive "read the extends
  clause's text" approach needs to strip type arguments (similar to
  `strip_generic_params`, already used in `symbol_extract.rs:683` for Go generic type
  names) before doing name resolution, or every generic supertype reference fails to
  resolve.
- **Kotlin: sealed class/interface hierarchies, and the same `extends`-vs-`implements`
  ambiguity requirements.md already flags.** Kotlin's single `:` supertype list can't be
  split syntactically into "superclass" vs "interfaces" — requires a symbol-table check
  (is this name a `Type` or `Interface` in the model?) per requirements.md's own Rabbit
  Holes section. Sealed classes/interfaces add another wrinkle: a `sealed class Shape`
  with subclasses declared in the *same file* (Kotlin's sealed-hierarchy restriction) is
  a common, idiomatic pattern that should resolve trivially (same-package, unambiguous)
  but is worth a fixture test given how central sealed hierarchies are to idiomatic
  Kotlin.
- **Anonymous classes implementing an interface inline** (Java `new Foo() { ... }` /
  Kotlin `object : Foo { ... }`) have no named `SymbolNode` to be the `from` side of an
  edge — likely out of scope by construction (no name to key on), but worth a one-line
  confirmation in the plan that these are silently skipped rather than crashing the
  extractor, since `apache/cassandra` uses this pattern often for one-off comparators/
  listeners.
- **Gap in the backtest corpus itself**: `docs/backtest-repos.md` states outright — "No
  Python or Kotlin exemplar is in this list yet — add one the same way if a checker needs
  backtesting against those languages." Since this feature explicitly requires Kotlin
  `extends`/`implements` extraction (requirements.md scope), and the repo's own convention
  (`docs/backtesting.md`, restated in this repo's `CLAUDE.md`) treats corpus backtesting
  as a "not done without it" gate, **a Kotlin repo needs to be added to the corpus list
  before this feature can be backtested**, not just implemented and unit-tested. This is a
  concrete, actionable gap Phase 3/5 should account for (e.g. clone a large Kotlin repo —
  `JetBrains/kotlin` or a large Android app — via `scripts/clone-backtest-repos.sh`-style
  addition) rather than shipping Kotlin support backtested only against hand-written
  fixtures.

## 3. Performance: existing precedent suggests this doesn't need new perf infrastructure

`build_model`'s per-file loop (`src/arch_model.rs:342-419` area) already does symbol
extraction, call-site collection, field-access collection, and Go import-alias collection
in one tree-sitter walk per file, with resolution (`resolve_call_edges`,
`resolve_field_access_edges`, `resolve_file_import_aliases`) as separate post-walk passes
over already-collected raw data — exactly the pattern requirements.md's Non-functional
Requirements section asks `type_edges` to follow (single walk, resolve-after-walk).

No `criterion`/dedicated `[[bench]]` target exists in `Cargo.toml` or `src/` (grepped for
"benchmark"/"criterion" — only hits are incidental, e.g. HAC merge criteria in
`extract_class.rs`, unrelated Story-acceptance-criterion comments). The one concrete perf
guard in the codebase is `run_export_completes_under_5s_on_benchmark_fixture`
(`src/arch_export.rs:306-330`): an 80-file synthetic fixture (40 Go + 40 TS files) that
must fully export (parse + build model + write JSON) in under 5s. This is a coarse
regression guard, not a profiler-backed benchmark — it would catch `type_edges` extraction
if it were accidentally quadratic or added a second full-repo pass, but wouldn't itself
tell you *why* something got slow.

Given `type_edges` extraction follows the same one-more-collection-in-the-existing-walk
shape as `field_accesses` (which shipped without its own dedicated perf validation, per
this same precedent), the expectation should be: **no new perf infrastructure needed**,
but the existing `run_export_completes_under_5s_on_benchmark_fixture` fixture should be
extended (or a sibling assertion added) to include type-hierarchy-bearing source so a
future regression in `type_edges` resolution specifically would trip the same 5s gate
rather than going unguarded. A genuinely separate concern: unlike `resolve_in`'s O(1)
hash lookups, if `type_edges` resolution needs a *transitive* walk (Open Question in
requirements.md — direct vs. transitive `list_supertypes`/`list_subtypes`), that walk's
complexity should be bounded (visited-set / cycle guard, matching `list_callees`'s
existing recursive-function termination pattern per `src/mcp.rs` test
`list_callees_on_a_recursive_function_terminates_via_the_visited_set`) — a hierarchy with
an accidental cycle (shouldn't exist in valid source, but a misresolved edge could
manufacture one) must not hang traversal.

## 4. Correctness: `looks_generated` interacts with `type_edges` exactly like `CallEdge`

`build_model`'s per-file loop (`src/arch_model.rs:342-351`) skips a file entirely — before
any symbol from it is added to `packages` — when `looks_generated` matches. A type
declared only in a skipped generated file therefore never appears in the `SymbolIndex`;
attempting to resolve an `extends`/`implements` reference to it is, at resolution time,
**indistinguishable from the target not existing in the repo at all** (there's no
"exists in source but wasn't modeled" signal separate from "doesn't exist" — both hit the
same `None` from `resolve_in`).

`CallEdge` already has the graceful answer for this: `resolved: bool` plus keeping the
raw unresolved text on `to` rather than dropping the edge (`src/arch_model.rs:136-146`,
confirmed by the `build_model_leaves_an_ambiguous_cross_package_call_unresolved_with_raw_text`
test at `src/arch_model.rs:1207-1237`, which produces the same "can't resolve" outcome for
a different root cause — ambiguity, not generation-skipping — but the same recovery shape).
`type_edges` should replicate this convention (confirmed appropriate by requirements.md's
own Non-functional Requirements: "an edge that can't be resolved... should be dropped or
kept as unresolved text, matching `CallEdge`'s existing `resolved: bool` convention"),
**not** `FieldAccessEdge`'s convention of silently dropping unmatched sites
(`src/arch_model.rs:167-170`, `resolve_field_access_edges` at `src/arch_model.rs:565-587`)
— `FieldAccessEdge` can afford to drop because an unmatched field-access site is very
likely a genuine typo/non-field-expression (the compiler would reject it in Go), whereas
an unresolved `extends`/`implements` target is routinely a *legitimate* external/vendored/
stdlib base class (e.g. Go embedding `sync.Mutex`, Java `implements Serializable`,
TS `extends React.Component`) that a refactoring-suggestion consumer (#59/#60/#61) needs
to see as "extends something real, just not modeled" rather than as "no hierarchy here at
all." Silently dropping these would make every type that only extends an external base
look like a hierarchy root with no superclass, which is a much worse silent-corruption
mode than an unresolved-but-visible edge.

## 5. False-positive risk for downstream consumers (#59/#60/#61) and the backtest-before-build convention

Because #59/#60/#61 read `ArchModel.type_edges` as a **library dependency**, not through
MCP (requirements.md, Users/Consumers section), a mis-extracted edge doesn't surface as a
visible bad MCP response somewhere a human might notice — it silently reshapes those
checkers' detection logic. Concretely:

- A Kotlin `: SomeType` ambiguity misclassified as `Extends` when it's actually
  `Implements` (or vice versa) would make #60 (type-code/switch clustering) or #61
  (hierarchy cleanup) apply Collapse-Hierarchy-style suggestions to an interface
  relationship, which requirements.md's own Alternatives Considered section says is
  exactly the distinction #60/#61 need `Extends`-vs-`Implements` to get right ("Collapse
  Hierarchy only makes sense for `extends`, not `implements`").
- A false-positive `extends` edge (e.g. a same-named-but-unrelated type across packages
  resolved wrong due to §1's ambiguity ceiling) would make #59 suggest Pull Up
  Method/Field between two types that aren't actually related, corrupting a refactoring
  suggestion a human or agent might act on directly.

This repo's stated convention (`docs/comment-quality.md`'s catalog description, this
worktree's `CLAUDE.md` "Writing a new check" section, and `docs/backtesting.md`/
`docs/backtest-repos.md` together) is that a new extraction/checker capability isn't
"done" on unit tests against hand-written fixtures alone — it needs to be run against the
real-world corpus and (for a checker specifically) transcript-backtested. `type_edges`
itself isn't a checker (no user-facing findings to backtest via
`kibitzer check backtest`), but it **is** exactly the kind of extraction feature
`docs/backtest-repos.md` describes running "against the whole tree... to see how it
behaves at a scale and style diversity no hand-built fixture set can approximate." Given
#59/#60/#61 will trust `type_edges` without re-deriving hierarchy themselves, the
concrete recommendation is: before (or as part of) landing #59/#60/#61, run
`type_edges` extraction against the corpus (`kubernetes/kubernetes` for Go embedding,
`apache/cassandra` for Java interface-extends-interface, `microsoft/vscode`/
`denoland/deno` for TS generics+qualified names, plus a to-be-added Kotlin repo per §2)
and spot-check a sample of extracted edges for correctness — not full manual review of
every edge, but enough of a sample to catch a systematic misclassification (e.g. "every
Kotlin `implements`-shaped edge came out as `Extends`") before it propagates into three
downstream checkers' detection rules. This is the same "confirmed real-world occurrence"
discipline `docs/check-ideas.md` already holds new check ideas to, applied on the
extraction-output side rather than the check-idea-input side.

## Summary of concrete recommendations for Phase 3 (plan)

1. Reuse `resolve_in`'s algorithm/shape for `type_edges`, but route qualified
   (`pkg.Type`) names through `file_import_aliases`-style alias resolution first, not
   blind same-package/globally-unique name matching alone.
2. Go embedding extraction is new code, not a small extension of `go_struct_fields`
   (`src/symbol_extract.rs:668-699`) — that function explicitly discards the anonymous
   fields `type_edges` needs to read.
3. Strip generic type arguments (`strip_generic_params`-style) before resolving a
   TS/Java/Kotlin supertype name, or generic supertypes never resolve.
4. Add a Kotlin repo to `docs/backtest-repos.md`'s corpus before/alongside landing Kotlin
   `extends`/`implements` extraction — the corpus doc currently states none exists yet.
5. Give `TypeRelationEdge` a `resolved: bool` (`CallEdge`'s convention), not
   `FieldAccessEdge`'s silent-drop convention — an unresolved external base class is a
   legitimate, common case, not a likely typo.
6. Extend `run_export_completes_under_5s_on_benchmark_fixture`
   (`src/arch_export.rs:306-330`) to include hierarchy-bearing fixtures rather than adding
   separate perf infrastructure — matches this repo's existing coarse-regression-guard
   precedent (no `criterion`/`[[bench]]` target exists for any extraction pass today).
7. Before #59/#60/#61 build on `type_edges`, spot-check extracted edges against the
   real-world corpus for systematic misclassification (esp. Kotlin
   extends-vs-implements ambiguity), matching this repo's "backtest before landing"
   discipline applied to extraction output rather than checker findings.
