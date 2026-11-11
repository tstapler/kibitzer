# Plan: pull-push-hierarchy-checks

## Overview

Implement five architecture checkers from Fowler's Chapter 12 as
`ArchModelChecker` impls, plus two small model extensions that unblock them.
All checkers are batch-only (`triggers: ["batch"]`) and go in
`default_checks()`.

## Phase 1 — Model extensions (prerequisite for Phases 2 and 3)

### 1a. Add `end_line: Option<usize>` to `SymbolNode`

File: `src/arch_model.rs`

Add `#[serde(default)] pub end_line: Option<usize>` to `SymbolNode`. Then
populate it in `extract_symbols_for_file` (`src/symbol_extract.rs`) via
tree-sitter `node.end_position().row + 1` (1-indexed, inclusive) for every
method/function node. All test fixture construction (e.g.
`hierarchy_checks::fixtures::sym`) needs `end_line: None`.

### 1b. Store `struct_field_names` in `ArchModel`

File: `src/arch_model.rs`

Add `#[serde(default)] pub struct_field_names: BTreeMap<String, BTreeSet<String>>`
to `ArchModel`. Key is `"{package_path}::{type_name}"`, value is a sorted set
of field names. During `build_model`, convert the existing local
`struct_fields: HashMap<(String, String), HashSet<String>>` into this field
before constructing the returned `ArchModel`.

## Phase 2 — Push Down checkers (use existing model data; simpler)

### 2a. `push_down_method.rs`

New file `src/push_down_method.rs`.

```
For each supertype P with ≥2 direct resolved subtypes [S1, S2, ...Sn]:
  For each exported Method M on P (via methods_of):
    Collect all call_edges where to == M.id and resolved == true.
    If callers is empty: skip (no evidence).
    Map each caller from-id to its owner type (prefix before the last '.').
    Discard callers whose owner type == P (a supertype method calling M is not
      evidence of single-subtype usage).
    If the remaining callers all belong to methods of exactly one Si:
      Emit finding: "[push-down-method] {M.id} is only called from {Si} ..."
```

Register in `model_registry()` and `default_checks()`.

### 2b. `push_down_field.rs`

New file `src/push_down_field.rs`.

```
For each supertype P with ≥2 direct resolved subtypes [S1, S2, ...Sn]:
  For each field F in struct_field_names[P.id]:
    Collect all field_accesses where to == "{P.id}.{F}" (the synthetic id).
    If accesses is empty: skip.
    Discard accesses from methods on P itself.
    If the remaining accesses all come from methods of exactly one Si:
      Emit finding: "[push-down-field] {P.id}.{F} accessed only from {Si} ..."
```

Register in `model_registry()` and `default_checks()`.

## Phase 3 — Pull Up checkers (require end_line and struct_field_names)

### 3a. `pull_up_field.rs`

New file `src/pull_up_field.rs`.

```
For each supertype P with ≥2 direct resolved subtypes [S1, S2, ...Sn]:
  Collect field sets: fields_i = struct_field_names[Si.id] for each Si.
  shared_fields = intersection of all fields_i (or ≥2-of-n overlap).
  For each F in shared_fields where F is NOT already in struct_field_names[P.id]:
    Emit finding: "[pull-up-field] field '{F}' is declared in {list of Si} ..."
```

Require ≥2 subtypes with the field for a finding. Don't require *all* subtypes
to have it — a subset suffices (same as `duplicate-code`'s ≥3-occurrence bar
would here be ≥2 siblings).

### 3b. `pull_up_method.rs`

New file `src/pull_up_method.rs`.

Body hashing: for a method M on subtype Si (with `end_line` populated), read
`Si.file`, extract lines `[M.line, M.end_line]`, trim and hash as a string.
If two sibling subtypes have same method name AND same body hash AND the
supertype does NOT already declare that method, emit a finding.

Skip when `end_line` is `None` (language not yet wired).

### 3c. `pull_up_constructor_body.rs`

Same as `pull_up_method.rs` but restricted to methods matching constructor
patterns:
- TypeScript/JS/Kotlin: method name == `"constructor"`
- Java: method name == enclosing type name
- Go: by convention only (no keyword), skip for Go

## Phase 4 — Backtest and noise guards

Run all five checkers against the standard corpus (`docs/backtest-repos.md`).
For each checker, record raw findings and triage ≥10 at random. If FP rate >
30%, add a noise guard (e.g. a minimum-method-count threshold, an is-abstract
guard, a same-name-different-package exclusion).

Expect low raw counts: inheritance hierarchies at real scale tend to be
deliberate, so true positives are rare and must be strong signals.

## Phase 5 — Documentation and registration

For each checker, create `docs/<checker-name>-false-positives.md` following
the existing pattern. Add an entry to `docs/hierarchy-cleanup-checks.md`
extending its table. Update `default_checks()` and `model_registry()`.

## Task breakdown

| # | Task | Category | Estimate |
|---|---|---|---|
| 1 | Add `end_line: Option<usize>` to `SymbolNode`; populate in `extract_symbols_for_file` for all OOP languages | backend | 3h |
| 2 | Add `struct_field_names: BTreeMap<String, BTreeSet<String>>` to `ArchModel`; wire in `build_model` | backend | 2h |
| 3 | Implement `push_down_method.rs` + unit tests + register | backend | 4h |
| 4 | Implement `push_down_field.rs` + unit tests + register | backend | 3h |
| 5 | Implement `pull_up_field.rs` + unit tests + register | backend | 3h |
| 6 | Implement `pull_up_method.rs` + unit tests + register | backend | 5h |
| 7 | Implement `pull_up_constructor_body.rs` + unit tests + register | backend | 3h |
| 8 | Corpus backtest all five; add noise guards; record results | test | 4h |
| 9 | Write false-positive docs for all five | docs | 2h |

## Risks

- **Body hash false negatives**: if two copies of a method differ by one
  whitespace character, identical bodies won't match. Mitigation: normalize
  whitespace before hashing (trim each line, collapse internal runs) using the
  same normalization `duplicate_code.rs` already applies.
- **Go pseudo-inheritance via embedding**: Go uses struct embedding, which
  populates `type_edges` as `Extends`. The push-down checkers may fire on
  embedded types that promote methods to an outer type — the outer type's
  method is auto-promoted, not a copy. Mitigation: skip `type_edges` where
  the Go embed has the same field name as the embedded type (structural signal
  for promotion vs inheritance).
- **end_line accuracy**: tree-sitter reports byte offsets for end position.
  Converting to 1-indexed line numbers requires `node.end_position().row + 1`.
  Off-by-one will produce wrong body slices. Must be unit-tested.
