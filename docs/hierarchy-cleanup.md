# Hierarchy cleanup checks

Three Fowler inheritance-chapter checks (`src/hierarchy_cleanup.rs`,
[issue #61](https://github.com/tstapler/kibitzer/issues/61)). Run with
`kibitzer check architecture <name> <dir>`:

| Check | Fowler refactoring | Flags |
|-------|--------------------|-------|
| `collapse-hierarchy` | Collapse Hierarchy | A type that is the only subtype of its supertype and barely differs from it |
| `extract-superclass` | Extract Superclass | Unrelated types in one package with near-identical method signatures |
| `remove-subclass` | Remove Subclass | A leaf subtype whose only methods are constant-returning overrides |

## Scope

Java, Kotlin, TypeScript/TSX, JavaScript, via resolved `Extends` edges in `model.type_edges`.
Not covered: Go (embedding is ignored), `Implements` edges, unresolved targets, Python and Rust
(`type_relation_supports` has no edges for them), `.d.ts` files and test files. TS `abstract
class` is a different tree-sitter node and never becomes a `Type` symbol, so it can't be a
supertype here at all.

New additive `SymbolNode` fields (`#[serde(default)]`, set in `symbol_extract.rs`):
`returns_constant` (parameterless method whose body is one `return <literal>` or Kotlin
`= <literal>`, including signed numbers like `-1`; interpolated strings don't count),
`is_abstract` (Java/Kotlin `abstract` or `sealed`), `declares_fields` (field/property
declaration, Kotlin `val`/`var` constructor parameter, TS constructor parameter property
(`private`/`public`/`protected`/`readonly`), or a constructor/`init` body that assigns
`this.<name> = ...`). A TS/JS `constructor` method is not counted as a method; Java and Kotlin
constructors are not extracted as symbols at all. Types sharing a `{package}::{name}` id (for
example two nested `Builder` classes) are dropped from analysis, because their methods
cannot be told apart.

## Thresholds

- Shared by `collapse-hierarchy` and `remove-subclass` (one helper): the subtype is not
  abstract, has no `implements` edge, its name doesn't end in `Exception`/`Error`, and neither
  type is in a test file.
- `collapse-hierarchy`: supertype has exactly one resolved subtype and is not abstract; subtype
  declares `<= 1` method (`COLLAPSE_MAX_SUBTYPE_METHODS`, constructors excluded), no fields, and
  is a leaf.
- `remove-subclass`: leaf with a resolved supertype; `>= 1` own method, every one a
  `returns_constant` override of a same-named supertype method (non-empty name); no fields.
- `extract-superclass`: types in the same package (directory) with `>= 4` signatures
  (`EXTRACT_MIN_SHARED_SIGNATURES`) and Jaccard `>= 0.7` (`EXTRACT_MIN_JACCARD`) over
  `name(param types)`. Excluded methods: `constructor`, `toString`, `equals`, `hashCode`,
  `init`. A pair is "related" (skipped) if one is an ancestor of the other, they share an
  ancestor, extend the same base text (resolved or not), or implement a common interface.
  Groups are greedy and pairwise, so every member is similar to and unrelated to every other.
  This uses a plain Jaccard check, not the Extract Class clustering helpers (those cluster
  members within one type; there was nothing to reuse across types).
- Only Go/TS carry parameter types in `SymbolNode.params`; for Java/Kotlin a signature is just
  `name()`, so overloads merge.

Thresholds are judgment calls tuned on the backtest below, not derived.

## Backtest

Release build, `kibitzer check architecture <name> ~/code/github.com/<repo>`, run after the
review fixes (nested-type id dedup, constructor-hidden fields, shared subtype guards, negative
literals). Counts are findings; the right column is the earlier run before those fixes.

| Repo | collapse-hierarchy | extract-superclass | remove-subclass | Before fixes (c / e / r) |
|------|-------------------:|-------------------:|----------------:|--------------------------|
| apache/cassandra (Java) | 20 | 19 | 5 | 20 / 20 / 5 |
| microsoft/vscode (TS) | 46 | 12 | 0 | 63 / 12 / 0 |
| denoland/deno (JS/TS) | 7 | 5 | 0 | 8 / 5 / 0 |
| android/nowinandroid (Kotlin) | 0 | 0 | 0 | 0 / 0 / 0 |

I did not diff the individual findings, so the attribution of each drop (cassandra extract -1,
vscode collapse -17, deno collapse -1) to a specific fix is inferred from which guards changed,
not checked. Wall time 11-12s per run (cassandra, deno), 52-55s (vscode), under 1s
(nowinandroid).

`nowinandroid` has no in-repo class hierarchies (every superclass is external), so Kotlin
end-to-end behavior is covered by unit tests only; no corpus evidence.

Skipped from the corpus in `backtest-repos.md`: `kubernetes/kubernetes` and `stapler-squad`
(Go: embedding is ignored, so there are no edges to check), `servo/servo` and
`BurntSushi/ripgrep` (Rust: no `Extends` edges, `type_relation_supports` excludes it). I did
not use `scripts/backtest-triage.py`; the triage below is by hand.

### Transcript replay

`kibitzer check backtest <name>` for each of the three, default transcripts directory
(4875 transcripts, 2093 snapshots checked, 1254 edits unreconstructable): zero findings for all
three. That says only that no replayed edit tripped them; collapse-hierarchy finished in 0.3s
versus 14-17s for the other two, and I did not investigate why, so treat it as weak evidence
rather than a clean bill.

### Triage (opened the code; samples, not exhaustive)

- `remove-subclass`, cassandra, 5 of 5 opened: `Fatal`/`Silent` in `ProtocolException.java`,
  `InteriorNodeHeader`, `Memoized`/`NotMemoized` all match the shape (constant override, no
  fields). Whether each refactor is worth doing is a judgment I did not make; `ReturnValue` is
  an abstract base, which Fowler's recipe handles but a reviewer may not want.
- `collapse-hierarchy`, cassandra, 4 of 20 opened: `ValidationCompactionIterator`
  (constructor-only specialization) plausible; `CMSSizeGuardrail` (sets an inherited field in its
  constructor), `ReusableLongToken` (mutable variant of an immutable token) and `GetEndpoints`
  (deprecated CLI alias carrying its own `@Command`) are intentional, so false positives.
  vscode, 3 of 63 opened: `RebaseUpstreamItem` and the empty `FileResourceMarkersRenderer`
  plausible; the third (`Queue`) was not confirmed. Rough read: well under half are actionable.
- `extract-superclass`, 8 opened across cassandra, vscode and deno (class headers and signature
  lists, method bodies not diffed): `NoSpamLogStatement`/`NoDuplicateSpamLogStatement` and vscode's
  `ManageAccountPreference...ForExtension/McpServerActionImpl` plausible. False-positive classes
  seen: siblings with different but related interfaces (`ButtonWithDropdown` vs
  `ButtonWithDescription`, `CompressedObjectTreeModel` vs `CompressibleObjectTreeModel`), JDK
  collection views (`EntrySet`/`KeySet`/`Values`), and spec-mirrored web classes
  (`CompressionStream`/`DecompressionStream`). Three earlier false-positive classes were fixed
  during the backtest: siblings sharing an unresolved base (`Message.Response`), transitive
  groups that mixed related types, and `.d.ts` declarations.

No true-positive rate was measured. Counts above are findings, not verified defects.

## Registration

All three are in `model_registry()` but **not** in `default_checks()`: the false-positive rate
is unproven (collapse and extract produced visible noise on every repo with hierarchies), and
nowinandroid gave no Kotlin evidence. Enable via `.kibitzer/inspect.json` or run directly.
`kibitzer:ignore` directives are not applied to architecture checks (see
`suppressing-checks.md`); the rule names are registered with the inline-ignore table.

