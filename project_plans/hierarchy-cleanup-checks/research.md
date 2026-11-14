# Research: hierarchy-cleanup-checks

**Date**: 2026-11-04

## Codebase Findings

### Dependency status: #40 is shipped

`src/arch_model.rs:241-248` confirms `TypeRelationEdge` is live:
```rust
pub struct TypeRelationEdge {
    pub from: String,   // subtype SymbolNode::id
    pub to: String,     // supertype id or raw text if unresolved
    pub kind: Option<TypeRelationKind>,  // Extends | Implements
    pub resolved: bool,
    pub file: PathBuf,
    pub line: usize,
}
```
`ArchModel.type_edges: Vec<TypeRelationEdge>` is at `src/arch_model.rs:266`, behind
`#[serde(default)]` for backward compatibility.

### Dependency status: #38 is shipped

`src/extract_class.rs` implements the full HAC clustering pipeline:
- `MIN_METHODS_FOR_CLUSTERING = 4`, `MAX_METHODS_FOR_CLUSTERING = 80`
- `MIN_ENTITY_PLACEMENT_SCORE = 0.75`
- `ExtractClassCandidate { package, type_name, groups, entity_placement_score }`
- `method_field_sets()` → Jaccard distance → `hac_partitions()` → `candidate_splits()`

The clustering currently operates within a single type's method set. For Extract
Superclass, the same Jaccard machinery can compare field-access sets between two
separately declared types.

### ArchModelChecker trait

`src/architecture_checks.rs:604-618`:
```rust
pub trait ArchModelChecker {
    fn name(&self) -> &str;
    fn check(&self, model: &ArchModel, config: &ArchitectureConfig) -> Vec<ArchFinding>;
    fn needs_private_symbols(&self) -> bool { false }
}
```
Registered in `model_registry()` at `src/architecture_checks.rs:620-634`.

### SymbolNode — existing body fields

`src/arch_model.rs:77-98`:
- `is_pure_delegation: bool` — set when body is a single call or `return <call>`
- Extracted in `src/symbol_extract.rs:544-563` for Go, TS, JS

No `is_constant_return` field exists yet. The extraction logic in `symbol_extract.rs`
would need a parallel path: detect `return <literal>` bodies (integer, string, bool, nil,
null constants) using the same tree-sitter traversal that detects pure delegation.

### SymbolNode — field information gap

`SymbolNode` does not carry field declarations — only methods are `SymbolNode`s (kind
`Type`/`Interface`/`Function`/`Method`). Fields on a struct/class are only indirectly
visible via `FieldAccessEdge` entries that reference them as `"{pkg}::{Type}.{field}"`.
For Remove Subclass's "no additional fields" check, we can proxy: a subtype declares no
additional fields if its `FieldAccessEdge`s only reference fields also accessed in the
supertype's method set. This is an approximation — a declared field never accessed in a
method would be missed — but matches the codebase's existing "no evidence, not zero"
stance (see `SymbolNode::call_edges`).

### language coverage for type_edges

From `src/symbol_extract.rs`, type/interface extraction is live for:
- **Go**: struct embedding → `Extends` (via `dotted_text_is_go_package_qualifier`)
- **TypeScript/TSX**: `extends`/`implements` keywords
- **Java**: `extends`/`implements`
- **Kotlin**: `:` supertypes list

Go structural interface satisfaction is explicitly excluded (arch_model.rs:235-239).

### Collapse Hierarchy — existing precedent

`src/extract_class.rs:38` uses `MIN_ENTITY_PLACEMENT_SCORE = 0.75` for intra-type splits.
The description item says "#59 diffs siblings" — that is the Pull Up/Push Down pair,
which diffs a type against its sibling types to find shared methods. Collapse Hierarchy
is a simpler variant: diff a type against its direct supertype, flag when the subtype's
unique-method ratio is very low (< ~20%).

The method enumeration for a type comes from filtering `ArchModel` package symbols where
`symbol.parent == type_name && symbol.kind == Method`.

### No existing hierarchy checkers

None of the current `model_registry()` entries (`InstabilityChecker`,
`DipConcreteCouplingChecker`, `LcomChecker`, `GodClassChecker`,
`IspFatInterfaceChecker`, `UnreferencedPrivateSymbolChecker`,
`SingleCallSiteDelegationChecker`, `MiddleManChecker`, `EncapsulateVariableChecker`,
`FeatureEnvyChecker`, `GoPackageFocusChecker`) touches `type_edges`.

### Backtest infrastructure

`kibitzer check backtest <name>` replays Claude Code session transcripts
(`~/.claude/projects/*/*.jsonl`). Public corpus is cloned via
`scripts/clone-backtest-repos.sh` (kubernetes, cassandra, servo, ripgrep, deno, vscode,
stapler-squad). `scripts/backtest-triage.py` tracks triage across runs.

## Literature / Prior Art

- Fowler *Refactoring* 2nd ed., §12.4 (Remove Subclass), §12.8 (Extract Superclass), §12.9 (Collapse Hierarchy)
- Fokaefs et al., "Identification and application of Extract Class refactorings in object-oriented systems," JSS 2012 — the JDeodorant algorithm reused in `extract_class.rs`
- For cross-type similarity: Demeyer et al., "Finding Refactorings via Change Metrics," OOPSLA 2000; structural-similarity detection is equivalent to their "type neighborhood" analysis

## Risk Analysis

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|-----------|
| `is_constant_return` extraction wrong for edge cases (multi-value returns, named returns) | Medium | Medium | Follow `is_pure_delegation`'s tests exactly; test named-return and multi-value false cases |
| Remove Subclass false-positives on template methods where constant return is intentional design | High | Medium | Require ALL methods on subtype to be constant-return; partial patterns excluded |
| Extract Superclass noise on two small types with identical trivial field sets | Medium | High | Minimum method count guard (`MIN_METHODS_FOR_CLUSTERING`) |
| Collapse Hierarchy fires on deliberate thin adapter subclasses | Medium | Medium | Threshold at 20% uniqueness; suppressible via `kibitzer:ignore` |
| Go embed chains produce multiple type_edges hops — Collapse Hierarchy fires on intermediate nodes | Low | Medium | Only check direct parent (depth-1 Extends edge), not transitive |
| type_edges for unresolved to-targets makes subtype/leaf detection unreliable | Medium | High | Only process edges where `resolved == true` for all three checkers |
