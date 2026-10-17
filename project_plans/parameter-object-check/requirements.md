# Requirements: parameter-object-check

**Backlog item:** Native check — Introduce Parameter Object (repeated parameter-group clustering)  
**Issue reference:** [#51](https://github.com/tstapler/kibitzer/issues/51)

## Problem statement

When the same group of 2+ parameters (by name and type) recurs across multiple function signatures in a codebase, that repeated cluster is a structural signal that those parameters belong together as a named type — Fowler's **Introduce Parameter Object** refactoring (*Refactoring*, 2nd ed., Chapter 6). This smell is related to but distinct from `long-parameter-list` (Change Function Declaration): the issue isn't total parameter count but repeated *co-occurrence* of the same subset.

## Proposed solution

A new whole-repo native check (`architecture_checker`) that:

1. Extracts typed parameter lists from every `Function`/`Method` symbol in the repo.
2. Clusters functions by overlapping parameter-name+type subsets.
3. Flags clusters of 3+ functions sharing a common subset of 2+ `(name, type)` pairs.

## Critical pre-condition gap (found during triage)

The proposal states "depends on: None — uses ArchModel's existing symbol data." This is **incorrect**. `SymbolNode` (defined in `src/arch_model.rs`) currently has no `params` field — it stores `id`, `name`, `kind`, `file`, `line`, `exported`, `parent`, and `is_pure_delegation`. Parameter name+type information is parsed at the AST level in individual checkers like `primitive_obsession.rs` but is **not** lifted into the shared `ArchModel`. Before the clustering checker can be built, parameter extraction must be added to `SymbolNode` or a parallel data structure.

## Acceptance criteria

1. **AC-1 (SymbolNode extension):** `SymbolNode` gains a `params: Vec<(String, String)>` field (name, type) with `#[serde(default)]` so existing cached `cache.json` entries still deserialize; `extract_symbols_for_file` populates it for Go, TypeScript/TSX, JavaScript, Java, Kotlin, and Rust functions/methods (other languages leave it empty).
2. **AC-2 (Clustering logic):** A new `ParameterObjectChecker` struct (or equivalent) implements `ArchitectureChecker` and identifies exact-subset matches: all functions whose parameter set contains a specific subset of 2+ `(name, type)` pairs; clusters of ≥3 such functions are flagged.
3. **AC-3 (Finding format):** Each finding points at the first function in the cluster (`file` + `line`) with a message listing the repeated parameter subset and the count of affected functions, following the existing `{file}:{line}: [parameter-object] ...` convention.
4. **AC-4 (Severity):** Registered at `Advisory` severity; the check does not block CI.
5. **AC-5 (Default check):** Wired into `default_checks()` in `src/config.rs` with `triggers: ["batch"]` (same as other `architecture_checker` entries).
6. **AC-6 (Backtesting):** Run against the six public-corpus repos (`kubernetes/kubernetes`, `apache/cassandra`, `servo/servo`, `BurntSushi/ripgrep`, `denoland/deno`, `microsoft/vscode`) and `stapler-squad` itself; confirm false-positive rate is acceptable before marking ready.
7. **AC-7 (Unit tests):** At minimum, tests for (a) exact-subset matching fires correctly, (b) clusters smaller than 3 functions do not fire, (c) subsets smaller than 2 params do not fire, (d) serde round-trip of the new `params` field.

## Out of scope

- Fuzzy/Jaccard-similarity matching (mentioned in the proposal as a follow-on after exact-subset matching is proven).
- Auto-applying the refactoring (suggestion only).
- Integration with the `primitive-obsession` checker (separate check, separate file).

## Related

- `docs/refactoring-catalog-analysis.md` § "Introduce Parameter Object" row
- `src/arch_model.rs` — `SymbolNode` definition
- `src/symbol_extract.rs` — `extract_symbols_for_file`, `parameter_list_names`
- `src/checkers/primitive_obsession.rs` — pattern for per-function parameter traversal via tree-sitter
- `src/jaccard.rs` — existing Jaccard utility (available for fuzzy follow-on)
- `src/architecture_checks.rs` — `ArchitectureChecker` trait and `registry()`
