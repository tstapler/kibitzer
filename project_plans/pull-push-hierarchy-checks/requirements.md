# Requirements: pull-push-hierarchy-checks

## Summary

Five advisory architecture checkers from Fowler's inheritance chapter
(Refactoring, 2nd ed., Chapter 12), all operating over `ArchModel.type_edges`
(resolved `Extends` edges). The checkers fire when a codebase has members
that belong in the wrong level of a class hierarchy.

| Checker | Smell | Fires when |
|---|---|---|
| `pull-up-method` | Duplicate method in siblings | ≥2 direct subtypes of the same supertype share a method with structurally identical bodies |
| `pull-up-field` | Duplicate field in siblings | ≥2 direct subtypes declare a field with the same name and type |
| `pull-up-constructor-body` | Duplicate constructor setup in siblings | ≥2 direct subtypes have constructors with identical body hashes |
| `push-down-method` | Superclass method used by one subtype only | A supertype method has call-site evidence only from methods of a single direct subtype |
| `push-down-field` | Superclass field used by one subtype only | A supertype field has field-access evidence only from methods of a single direct subtype |

## Background and dependency status

The issue description lists #40 (type-hierarchy graph), #39 (field read/write
tracking), and #33 (call-graph edges) as blockers. All three are already
shipped: `ArchModel` already carries `type_edges`, `field_accesses`, and
`call_edges`. The existing `remove-subclass`, `extract-superclass`, and
`collapse-hierarchy` checkers rely on exactly these fields.

The remaining gaps are:

1. **`SymbolNode` lacks `end_line`** — required to slice the source text of a
   method body for body-hashing (Pull Up Method / Constructor Body).
2. **`ArchModel` does not persist field declarations** — `build_model` builds
   a `struct_fields: HashMap<(pkg, type_name) → HashSet<field_name>>` locally
   during resolution but discards it. Pull Up Field and Push Down Field need
   this per-type field-name set at checker time.

## Acceptance criteria

1. `push-down-method` fires when a supertype has a method whose only resolved
   call-site callers are methods belonging to exactly one direct subtype, and
   silently skips the case when no resolved callers exist (no-evidence stance).
2. `push-down-field` fires when a supertype field (`pkg::Type.field` id) has
   field-access edges whose `from` ids all resolve to methods of exactly one
   direct subtype, and skips when no access evidence exists.
3. `pull-up-method` fires when ≥2 direct sibling subtypes of the same
   supertype declare a same-named method whose line-range body hashes (using
   `duplicate_code.rs`'s windowed approach) are identical, and skips
   cross-file body comparison when `end_line` is not yet populated.
4. `pull-up-constructor-body` fires under the same conditions as
   `pull-up-method` but restricted to constructors (methods named
   `constructor`, `__init__`, `new`, or matching a language-specific
   constructor pattern).
5. `pull-up-field` fires when ≥2 direct sibling subtypes declare a field with
   the same name (regardless of type text), and treats identical name + type
   text as a stronger signal.
6. All five checkers are `ArchModelChecker` implementations registered in
   `architecture_checks::model_registry()` and in `config::default_checks()`.
7. All five skip test files and `fixtures/`/`testdata/` directories using the
   existing `hierarchy_checks::is_ignorable_file` guard.
8. All five skip unresolved `Extends` edges (same guard as existing hierarchy
   checkers).
9. Each checker ships with a false-positive documentation file following the
   `docs/*-false-positives.md` convention.
10. All five pass backtest (`kibitzer check backtest <name>`) with ≤5% false-
    positive rate on the standard corpus (`docs/backtest-repos.md`), or the
    threshold and known FP categories are documented before merging.
11. `SymbolNode` gains an optional `end_line: Option<usize>` field populated
    by `extract_symbols_for_file` for Go/TS/Java/Kotlin method bodies; `None`
    on languages not yet wired up. `#[serde(default)]` for cache compatibility.
12. `ArchModel` gains a `struct_field_names: BTreeMap<String, BTreeSet<String>>`
    field (key = `pkg::TypeName`, value = field-name set) populated during
    `build_model`. `#[serde(default)]` for cache compatibility.
