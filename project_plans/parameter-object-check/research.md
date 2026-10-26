# Research: parameter-object-check

## Codebase findings

### SymbolNode has no params field (blocking prerequisite)

`src/arch_model.rs` lines 67–95 define `SymbolNode`:

```rust
pub struct SymbolNode {
    pub id: String,
    pub name: String,
    pub kind: SymbolKind,
    pub file: PathBuf,
    pub line: usize,
    pub exported: bool,
    pub parent: Option<String>,
    #[serde(default)]
    pub is_pure_delegation: bool,
}
```

No `params` field. The proposal's "depends on: None" claim is **false**. Parameter extraction must be added here or in a parallel structure before the clustering checker can operate.

### Where parameter data does exist today

`src/symbol_extract.rs` parses parameter lists at the tree-sitter AST level in two places:

- `parameter_list_names()` (line ~993): extracts parameter *names* from a Go `parameter_list` node — used for shadowing/receiver detection, not exported into `SymbolNode`.
- `primitive_obsession.rs` checker: walks Go function signatures and groups parameters by type — per-file, not shared.

These confirm that tree-sitter can reach parameter data. The work is lifting it into a shared structure.

### Existing architecture checker pattern

`src/architecture_checks.rs` defines `ArchitectureChecker` trait:

```rust
pub trait ArchitectureChecker {
    fn name(&self) -> &str;
    fn check(&self, graph: &ImportGraph, config: &ArchitectureConfig) -> Vec<ArchFinding>;
}
```

**Problem**: the trait takes `ImportGraph` + `ArchitectureConfig`, not `ArchModel`. Existing architecture checkers (`ImportCycleChecker`, `LayeringChecker`, `CouplingChecker`, etc.) inspect the import graph but do **not** have access to per-symbol parameter data. To cluster by parameters, the new checker needs `ArchModel` (which has `PackageNode.symbols`), not just `ImportGraph`.

Two options:
1. **Add `params` to `SymbolNode`** and thread `ArchModel` into the architecture checker dispatch path so parameter-group clustering can operate there.
2. **Separate per-file checker** that reads its own file's functions via tree-sitter and emits per-file findings — loses cross-file clustering but avoids the architecture-checker plumbing change.

Option 1 is the correct design (the proposal's intent); option 2 is a weaker fallback.

### How `architecture_checker` entries reach `ArchModel`

Looking at `src/run.rs` and `src/run_context.rs` (not fully read), the architecture checker dispatch must be found. However, the `check()` signature on `ArchitectureChecker` only has `ImportGraph` — so either the signature needs to be extended to pass `ArchModel`, or the clustering checker needs to build its own model.

The `load_cached_model` function in `arch_model.rs` is the standard entry point for building an `ArchModel`. The new checker could call it internally, but that's expensive and duplicates work the run context already does. Better: extend the `ArchitectureChecker::check` signature to receive `&ArchModel` (or an `Option<&ArchModel>` for backward compat).

### Jaccard similarity (for future use)

`src/jaccard.rs` implements `jaccard(a: &HashSet<&str>, b: &HashSet<&str>) -> f64` over string sets. The immediate implementation uses exact-subset matching (cheaper, fewer false positives) as specified; Jaccard is available for the follow-on fuzzy extension.

### Primitive obsession checker as pattern reference

`src/checkers/primitive_obsession.rs` is the closest existing pattern: it parses Go parameter nodes using tree-sitter `GoKind::ParameterDeclaration`, groups them by type, and fires on ≥2 same-type parameters. The parameter-object check will do something analogous but cross-file and cross-language, clustering `(name, type)` pairs rather than `type` alone.

### Backtesting infrastructure

`docs/backtesting.md` and `docs/backtest-repos.md` define the corpus and tooling. The CLAUDE.md requires backtesting against the six public-corpus repos plus `stapler-squad` before shipping. The public corpus (`kubernetes/kubernetes` etc.) is large-scale Go/Java/Rust/TS — good coverage of the languages the param extraction must support.

### Config wiring for architecture checkers

`src/config.rs` `default_checks()` (line 759+) registers architecture checkers with `triggers: ["batch"]`. The new `parameter-object` check follows this pattern, registered at `Severity::Advisory`.

## Literature

- Fowler, M., *Refactoring*, 2nd ed., Chapter 6 "Introduce Parameter Object": the canonical motivation. The smell is "a group of data items that regularly travel together" across call sites.
- `docs/refactoring-catalog-analysis.md` row for "Introduce Parameter Object" (🧩 issue 51): verified as filed, status "no hard capability dependency" — **triage finding: this is incorrect**, the dependency is real.

## Risk assessment

| Risk | Likelihood | Mitigation |
|---|---|---|
| `ArchitectureChecker` trait signature must change | HIGH | Extend signature to carry `Option<&ArchModel>` with default `None`; existing checkers ignore it |
| Cross-language param extraction is noisy | MEDIUM | Start Go-only (tree-sitter grammar is most mature), add TS next, others after backtesting |
| Cross-file clustering is expensive on large repos | MEDIUM | Group by exact `(name,type)` subset using a `BTreeMap<Vec<(String,String)>, Vec<SymbolRef>>`; O(N·K) where K is max subset size |
| False positives: common param pairs like `(ctx context.Context, err error)` | HIGH | Advisory-only severity; suppress standard patterns (e.g. `ctx`/`context.Context` is already idiomatic Go, not a parameter object candidate) |
