# Plan: parameter-object-check

## Overview

Implement the `parameter-object` native architecture check — Fowler's *Introduce Parameter Object* smell detector. The check identifies repeated `(name, type)` parameter subsets across function signatures and flags clusters of 3+ functions sharing 2+ common parameters.

## Prerequisite: SymbolNode parameter extraction

The single largest prerequisite: `SymbolNode` must gain a `params` field and `extract_symbols_for_file` must populate it. This is separate from and prior to the clustering checker itself.

## Phase 1 — Extend SymbolNode with parameter data

### Task 1.1 — Add `params` field to `SymbolNode`

**File:** `src/arch_model.rs`

Add:
```rust
/// Typed parameter list for `Function`/`Method` symbols; `[]` for `Type`/`Interface` and
/// for languages where parameter extraction is not yet implemented. Each tuple is
/// `(name, type_text)` in declaration order. `#[serde(default)]` for cache compatibility.
#[serde(default)]
pub params: Vec<(String, String)>,
```

All existing `SymbolNode { ... }` construction sites must add `params: vec![]` (mechanical search-and-update via `symbol_extract.rs`).

### Task 1.2 — Populate params in `extract_symbols_for_file`

**File:** `src/symbol_extract.rs`

For each language's function/method extraction node, extract the `parameters` child (tree-sitter field name) and collect `(name, type_text)` pairs. Start with Go (grammar is most mature); TypeScript/TSX next; Java/Kotlin and Rust as follow-on.

For Go: use the existing `parameter_list_names` helper as a model. A `parameter_declaration` has named children for identifier(s) and a `type` field — pair each identifier with the type text.

Edge cases:
- Variadic params (`...T`): include with type text as-is.
- Multiple names on one type (`a, b string`): emit two pairs, both with the same type.
- Receiver param: exclude (it is not a business-parameter; it is already captured in `parent`).
- Anonymous params (type only, no name, common in interfaces): emit `("", type_text)`.

### Task 1.3 — Cache compatibility

The `#[serde(default)]` attribute on `params` handles old cache entries gracefully (same precedent as `is_pure_delegation`). Add a regression test asserting that a `SymbolNode` JSON without `params` still deserializes to `params: vec![]`.

## Phase 2 — Extend `ArchitectureChecker` trait for model access

**File:** `src/architecture_checks.rs`

The current `check()` signature is:
```rust
fn check(&self, graph: &ImportGraph, config: &ArchitectureConfig) -> Vec<ArchFinding>;
```

Extend to:
```rust
fn check(&self, graph: &ImportGraph, config: &ArchitectureConfig, model: Option<&ArchModel>) -> Vec<ArchFinding>;
```

All five existing checkers (`ImportCycleChecker`, `LayeringChecker`, `CouplingChecker`, `ComponentDependencyChecker`, `PackageSizeChecker`) add `_model: Option<&ArchModel>` and ignore it.

Update the call site in `src/run.rs` (or wherever architecture checkers are dispatched) to pass the model. The run context already builds an `ArchModel` for MCP/diagram commands; for the batch-mode path, the model may need to be built lazily here.

## Phase 3 — Implement `ParameterObjectChecker`

**File:** `src/architecture_checks.rs` (or new `src/parameter_object.rs` if the file grows large)

```rust
pub struct ParameterObjectChecker;

impl ArchitectureChecker for ParameterObjectChecker {
    fn name(&self) -> &str { "parameter-object" }

    fn check(&self, _graph: &ImportGraph, _config: &ArchitectureConfig, model: Option<&ArchModel>) -> Vec<ArchFinding> {
        let Some(model) = model else { return vec![]; };
        // Collect all function/method symbols with ≥2 params
        // Build an inverted index: subset -> Vec<(file, line, fn_id)>
        // For each subset of size ≥2, if ≥3 functions share it, emit a finding
        // ...
    }
}
```

### Clustering algorithm

```
all_fns = model.packages
    .flat_map(|p| p.symbols)
    .filter(|s| s.kind in {Function, Method} && s.params.len() >= 2)

// Inverted index: BTreeMap<Vec<(String,String)>, Vec<SymbolRef>>
// where each key is a sorted subset of size ≥2
// For each fn, enumerate all subsets of size ≥2 of its params
// Insert fn into each subset's list

for (subset, fns) in index {
    if fns.len() >= 3 {
        emit ArchFinding {
            file: fns[0].file,
            line: fns[0].line,
            message: format!("[parameter-object] {} functions share parameter group ({}) — consider Introduce Parameter Object",
                             fns.len(), format_subset(&subset)),
            severity_override: Some(Severity::Advisory),
        }
    }
}
```

**Subset explosion guard:** For a function with N params, the number of size-≥2 subsets is `2^N - N - 1`. Cap N at 10 before enumerating subsets to avoid combinatorial blowup on pathological long-parameter-list signatures (those are already flagged by a separate check).

**Common-pair suppression:** A configurable deny-list of `(name, type)` pairs that are idiomatic boilerplate and should be excluded from subset computation. Initial list: `[("ctx", "context.Context")]` (Go's ubiquitous first parameter — pairing it with any business param would produce universal false positives).

## Phase 4 — Wire into default_checks and config

**File:** `src/config.rs`

Add to `default_checks()`:
```rust
Check {
    name: "parameter-object".to_string(),
    architecture_checker: Some("parameter-object".to_string()),
    severity: Severity::Advisory,
    triggers: vec!["batch".to_string()],
    ..Check::default()
}
```

Add to `architecture_checks::registry()`:
```rust
Box::new(ParameterObjectChecker),
```

## Phase 5 — Unit tests

**File:** `src/architecture_checks.rs` (tests mod) or `src/parameter_object.rs`

Required tests (per AC-7):
- `fires_on_exact_subset_match_in_3_or_more_functions` — three Go functions each taking `(user_id string, tenant_id string)` → 1 finding.
- `does_not_fire_on_cluster_of_two` — only two functions share the pair → 0 findings.
- `does_not_fire_on_single_param_subsets` — three functions share only `(user_id string)` → 0 findings.
- `serde_round_trip_of_params_field` — `SymbolNode` without `params` in JSON → `params: vec![]`.
- `ctx_context_pair_suppressed` — three functions share `(ctx, context.Context)` + `(user_id, string)` → only the `user_id`-less subset suppressed.

## Phase 6 — Backtest

Per CLAUDE.md / docs/backtesting.md:
1. `kibitzer check backtest parameter-object` against `~/.claude/projects/*/*.jsonl`.
2. Run against each public-corpus repo (`kubernetes/kubernetes`, `BurntSushi/ripgrep`, etc.) via `find <repo> -name '*.go' | xargs kibitzer check native parameter-object`.
3. Manually triage false positives; document in `docs/parameter-object-false-positives.md` (following `docs/go-primitive-obsession-false-positives.md` pattern).
4. Adjust suppression list or thresholds if FP rate > ~20%.

## Adversarial review

**Attack: "no hard capability dependency"**  
The refactoring catalog analysis and proposal both say this check needs no new capability. That is false: `SymbolNode` has no `params` field. Phase 1 + Phase 2 together are the real dependency — approximately the same size as the checker itself. Estimate should account for this.

**Attack: subset explosion**  
Enumerating all size-≥2 subsets of a 15-parameter function produces 2^15 - 16 = 32,751 index entries for one function. The cap at N=10 (1,013 subsets max) and filtering to functions with ≥2 params keeps this manageable. Document the cap in a code comment.

**Attack: cross-language type-text normalization**  
`string` in Go vs `String` in Java vs `str` in Python are the same conceptual type. The exact-subset matching design deliberately does NOT normalize across languages — cross-language clustering is out of scope and would produce meaningless findings. Within-language, types appear as written in source, which is correct.

**Attack: advisory-only severity may mean no one sees it**  
Correct concern, but accepted for this check: it's a refactoring suggestion, not a defect. The `kibitzer run` batch output always shows advisories. If uptake is low, a follow-on can elevate severity after backtesting confirms low FP rate.

## Estimate

| Phase | Work | Cost class |
|---|---|---|
| Phase 1 — SymbolNode params field + extraction (Go) | Medium structural Rust edit across arch_model + symbol_extract | M (output-heavy, multi-file) |
| Phase 1b — TS/JS extraction | Additive, same structure | S |
| Phase 2 — ArchitectureChecker trait extension | Mechanical signature change + 5 call sites | S |
| Phase 3 — ParameterObjectChecker | Core logic, ~100 lines | M |
| Phase 4 — Wire into default_checks + registry | Trivial | XS |
| Phase 5 — Unit tests | 5 test cases | S |
| Phase 6 — Backtest + triage | Read-heavy, judgment-heavy | M (wall-clock: depends on corpus clone time) |
