# Plan: hierarchy-cleanup-checks

**Date**: 2026-11-04

## Implementation Order

Dependencies constrain sequence: `is_constant_return` extraction must land before Remove
Subclass; Extract Superclass needs no new model fields (reuses `field_accesses`). The
three checkers are otherwise independent and can be implemented in parallel after the
shared plumbing lands.

```
Phase 1 (plumbing — serial):
  A. Add SymbolNode.is_constant_return + symbol_extract extraction (Go + TS)
  B. Add helper: build subtype/supertype maps from model.type_edges (resolved Extends)
     → shared by all three checkers
     → extract as free functions in a new src/hierarchy_checks.rs

Phase 2 (checkers — parallel):
  C. Remove Subclass checker (src/remove_subclass.rs)
  D. Extract Superclass checker (src/extract_superclass.rs)
  E. Collapse Hierarchy checker (src/collapse_hierarchy.rs)

Phase 3 (wire up + backtest):
  F. Register checkers in model_registry() + default_checks()
  G. Backtest all three; write docs/hierarchy-cleanup-checks.md
  H. cargo test + fix regressions
```

## Detailed Design

### A: `SymbolNode.is_constant_return`

**File**: `src/arch_model.rs` — add field after `is_pure_delegation`:
```rust
/// True iff this Method/Function's body is a single `return <literal>` statement,
/// where literal is an integer, string, bool, nil/null/None constant, or enum variant
/// with no additional logic. Only populated for Go and TypeScript (the same languages
/// `is_pure_delegation` covers); always `false` for other languages and for
/// `Type`/`Interface` symbols. `#[serde(default)]` for cache compatibility.
#[serde(default)]
pub is_constant_return: bool,
```

**File**: `src/symbol_extract.rs` — in the same tree-sitter pass that sets
`is_pure_delegation` (look for `delegating.is_some()`), add parallel detection:
- For Go: a body whose only statement is `return_statement` containing exactly one child
  node of kind `int_literal`, `float_literal`, `interpreted_string_literal`,
  `raw_string_literal`, `true`, `false`, `nil`, or a qualified identifier resolving to
  an exported constant (conservative: only the first four + booleans + nil for v1).
- For TypeScript: same logic under `return_statement` with `number`, `string`,
  `true`, `false`, `null`, `undefined` leaf nodes.

### B: Shared hierarchy helpers (new file `src/hierarchy_checks.rs`)

```rust
// Returns: for each resolved Extends edge, a map from subtype_id -> supertype_id
pub fn subtype_to_parent(model: &ArchModel) -> HashMap<&str, &str>

// Returns: set of type ids that have no subtypes of their own (leaf nodes)
pub fn leaf_subtypes(model: &ArchModel) -> HashSet<&str>

// Returns: methods for a named type within a package (kind==Method, parent==type_name)
pub fn methods_of<'a>(model: &'a ArchModel, pkg: &str, type_name: &str)
    -> Vec<&'a SymbolNode>

// Returns: field ids accessed by methods of a type (from field_accesses where
// from matches one of that type's method ids)
pub fn field_ids_of<'a>(model: &'a ArchModel, type_method_ids: &HashSet<&str>)
    -> HashSet<&'a str>
```

### C: Remove Subclass (`src/remove_subclass.rs`)

**Algorithm**:
1. Compute `leaf_subtypes(model)` — subtypes with no children.
2. For each leaf subtype `S` with a resolved `Extends` edge to `P`:
   a. Find all methods on `S` (from `model` symbol table, `parent == S.name`).
   b. Skip if any method is NOT `is_constant_return` AND is not purely inherited
      (i.e., declared on `S`, not `P`). A subtype that overrides even one non-constant
      method is not a Remove Subclass candidate.
   c. Skip if `S` has any field access to a field not present in `P`'s field set
      (proxies "declares additional fields").
   d. Fire with message: `"{S} is a leaf subtype of {P} whose only overrides return
      constants; consider Remove Subclass (Fowler §12.4): replace with a field on {P}."`,
      pointing at `S`'s declaration line.

**Thresholds**: No numeric threshold — this is a Boolean check (all-or-nothing).

**Needs private symbols**: `true` — leaf subtypes are often unexported.

### D: Extract Superclass (`src/extract_superclass.rs`)

**Algorithm**:
1. Collect all type symbols across all packages (kind == `Type`).
2. Build `existing_relations`: set of `(id_a, id_b)` pairs where `type_edges` has an
   `Extends` or `Implements` edge (in either direction, resolved or not) between them.
3. Within each package, for each pair of types `(A, B)` with no entry in
   `existing_relations`:
   a. Compute method field sets for `A` and `B` using `method_field_sets()` from
      `extract_class.rs` (reused directly — same Go-only restriction, same minimum-method
      guard).
   b. Compute Jaccard similarity of field-id sets: `|A ∩ B| / |A ∪ B|`. Skip pairs
      where either set is empty.
   c. If Jaccard >= `EXTRACT_SUPERCLASS_THRESHOLD` (0.5 default), emit a finding:
      `"{A} and {B} share {pct}% of their field accesses with no hierarchy relationship;
      consider Extract Superclass (Fowler §12.8)."`.
4. Cap per-package output at 3 findings (noisiest signal of the three; cap prevents
   flooding on large packages with many similar DTOs).

**Thresholds**: `EXTRACT_SUPERCLASS_THRESHOLD = 0.5` (Jaccard), min methods from
`extract_class::MIN_METHODS_FOR_CLUSTERING`.

**Language restriction**: Go-only for v1 (field-access data only exists for Go).
Document explicitly in `docs/hierarchy-cleanup-checks.md`.

### E: Collapse Hierarchy (`src/collapse_hierarchy.rs`)

**Algorithm**:
1. For each resolved `Extends` edge `(S, P)`:
   a. Collect methods declared on `S`: `methods_of(model, S_pkg, S_name)`.
   b. Collect methods declared on `P`: `methods_of(model, P_pkg, P_name)`.
   c. Compute unique-to-`S` count: methods on `S` whose name does NOT appear on `P`.
   d. Compute uniqueness ratio: `unique_to_S / total_on_S`. If `total_on_S == 0`, skip.
   e. If ratio < `COLLAPSE_HIERARCHY_UNIQUE_THRESHOLD` (0.20), emit finding:
      `"{S} extends {P} but only {pct}% of its methods are unique; consider Collapse
      Hierarchy (Fowler §12.9)."`, pointing at the edge's `file`/`line`.
2. Skip edges where `resolved == false`.

**Thresholds**: `COLLAPSE_HIERARCHY_UNIQUE_THRESHOLD = 0.20`.

**Needs private symbols**: `false` — only exported types are relevant here.

### F: Registration

**`src/architecture_checks.rs`**, `model_registry()`:
```rust
Box::new(crate::remove_subclass::RemoveSubclassChecker),
Box::new(crate::extract_superclass::ExtractSuperclassChecker),
Box::new(crate::collapse_hierarchy::CollapseHierarchyChecker),
```

**`src/config.rs`**, `default_checks()`: add three entries following the
`architecture_checker` + `batch` trigger + `advisory` severity pattern already used for
`god-class`, `feature-envy`, etc.

### G: Backtest + Docs

Run `kibitzer check backtest remove-subclass`, `extract-superclass`,
`collapse-hierarchy` against Claude Code transcripts. Run against each repo in the public
corpus. Document false-positive patterns in `docs/hierarchy-cleanup-checks.md` and add
suppressible examples to the respective false-positive docs.

## Test Plan (per checker)

Each checker in `src/<checker>.rs` gets an inline `#[cfg(test)]` module with:
1. **Empty model** — no findings returned.
2. **Positive case** — a minimal `ArchModel` fixture that triggers the finding.
3. **Negative case** — a type that almost qualifies but misses the threshold.
4. **Boundary case** — exactly at threshold (fires) and just below (doesn't fire), where
   applicable (Collapse Hierarchy threshold, Extract Superclass Jaccard threshold).
5. **Unresolved edges** — confirm unresolved `type_edges` entries are ignored.

`is_constant_return` extraction tests in `src/symbol_extract.rs` follow the same pattern
as the existing `is_pure_delegation_*` tests.

## Adversarial Review (pre-write)

**Concern 1**: Remove Subclass's "no additional fields" proxy (via field_accesses) will
miss declared-but-never-accessed fields. This is a known gap, same as `is_pure_delegation`
ignoring bodies the tree-sitter grammar misclassifies. Document as "false-negative risk"
in `docs/hierarchy-cleanup-checks.md`. No fix in v1; a future `SymbolNode.fields` list
could close this.

**Concern 2**: Extract Superclass within-package-only restriction will miss cross-package
opportunities. Documented as Out of Scope. Cross-package pairs would require alias
resolution across package boundaries, which the codebase defers throughout (see
`extract_class.rs:12` module doc).

**Concern 3**: Collapse Hierarchy's "method count" comparison compares names, not
signatures. Two methods with the same name but different parameter lists would be counted
as equal. This is consistent with the codebase's existing name-equality approach in
`god_class.rs` and acceptable for v1.

**Concern 4**: All three checkers will return no findings on repos without Go/TS/Java/Kotlin
type hierarchies. This is expected behavior — the same as `feature-envy` returning nothing
on pure-function repos. Document in `docs/hierarchy-cleanup-checks.md` under "language
coverage."

**Concern 5**: Extract Superclass is Go-only due to field-access scope but Remove Subclass
and Collapse Hierarchy are multi-language. The checker docs should state language scope
per checker clearly to avoid user confusion.
