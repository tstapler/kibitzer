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

(Authoritative copy of the backlog item's 12 criteria, same numbering.)

1. `push-down-method` fires when a supertype method's only resolved callers all belong to methods of exactly one direct subtype; silent when no resolved callers exist.
2. `push-down-field` fires when a supertype field's field-access edges all originate from methods of exactly one direct subtype (excluding the supertype's own methods); skips when no access evidence exists.
3. `pull-up-method` fires when >=2 direct sibling subtypes share a same-named method with identical body hashes (from `end_line`-delimited source slices); skips when `end_line` is None.
4. `pull-up-constructor-body` fires under the same conditions as pull-up-method, constructors only (TS/Kotlin: `constructor`; Java: method name equals owning type name).
5. `pull-up-field` fires when >=2 direct sibling subtypes declare a field with the same name AND same type text, and the supertype does not already declare it.
6. All five are `ArchModelChecker` impls registered in `architecture_checks::model_registry()` and `config::default_checks()`.
7. All five skip test files and fixtures/testdata via `hierarchy_checks::is_ignorable_file`, and skip unresolved Extends edges.
8. push-down-method and push-down-field also gate on no external callers/accessors outside the type hierarchy.
9. `SymbolNode` gains `end_line: Option<usize>` (serde default), populated by `extract_symbols_for_file` for Go/TS/Java/Kotlin method bodies.
10. `ArchModel` gains `struct_field_names: BTreeMap<String, BTreeSet<String>>` (serde default) persisting field-declaration data already computed in `build_model`.
11. All five pass corpus backtest with <=20% FP on the standard repos, or threshold and known FP categories are documented before merging.
12. Each checker ships `docs/<name>-false-positives.md` following the existing convention.

## Known constraints discovered in validation

- `extract_struct_fields_for_file` (src/symbol_extract.rs:1178) is Go-only and yields `(type_name, field_name)` with no type text. AC5 needs type text, so Phase 1b also persists a companion `struct_field_types` map (`pkg::Type` -> field -> type text) populated by extending the Go field walker; pull-up-field and push-down-field are Go-only until other languages gain field extraction. Document this limit in the FP docs.
- Field-access edges exist for Go only (`field_access_supports`), so push-down-field is Go-only too.
