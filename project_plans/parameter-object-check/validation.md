# Validation: parameter-object-check

## Adversarial review results

### Claim: "no hard capability dependency"

**Verdict: REJECTED.**  
The proposal and the refactoring catalog analysis both state this check has no hard capability dependency and uses the existing symbol table. Direct inspection of `src/arch_model.rs` lines 67–95 shows `SymbolNode` has no `params` field. Parameter data is parsed per-file by individual checkers (e.g. `primitive_obsession.rs`) but is not lifted into the shared `ArchModel`. Two new phases of work are required before the clustering checker can be built: (1) extend `SymbolNode` to carry `params`, and (2) extend the `ArchitectureChecker` trait dispatch to pass `&ArchModel`. Together these represent ~50% of the total implementation cost.

### Claim: "language-agnostic once the symbol table exposes typed parameter lists"

**Verdict: PARTIALLY ACCEPTED.**  
The clustering logic in `ParameterObjectChecker` is language-agnostic — it operates on `Vec<(String, String)>` regardless of source language. However, the *extraction* step in `extract_symbols_for_file` must be implemented per-language. Each language's tree-sitter grammar names parameter nodes differently (`parameter_declaration` in Go, `required_parameter`/`optional_parameter` in TypeScript, `formal_parameter` in Java). The plan correctly phases this: Go first, TS/JS second, Java/Kotlin and Rust as follow-on.

### Risk: false positives on idiomatic boilerplate

**Verdict: REAL, MITIGATED.**  
In Go codebases, `(ctx context.Context, ...)` appears in nearly every service method. Any function pair sharing `ctx context.Context` plus one business parameter would fire spuriously. The plan's suppression list seeded with `("ctx", "context.Context")` addresses the worst case. Backtesting against `kubernetes/kubernetes` (heavily Go) is essential to calibrate. Mark check advisory-only until FP rate is confirmed.

### Risk: subset explosion on long-parameter-list functions

**Verdict: REAL, CAPPED.**  
A 15-param function generates 32,751 size-≥2 subsets. The plan's N=10 cap produces at most 1,013 subsets per function — acceptable. Functions with >10 params are already flagged by `long-parameter-list`; they're the most important refactoring candidates anyway and should not be silently excluded from parameter-object detection. Implementation: when `params.len() > 10`, enumerate only size-2 subsets (C(10,2)=45 at most) and skip larger subsets. This gives a weaker but correct signal for pathological cases.

### Risk: `ArchitectureChecker::check` signature change breaks all existing checkers

**Verdict: MANAGEABLE.**  
The trait extension adds `model: Option<&ArchModel>` to all five existing checkers (`_model: Option<&ArchModel>` with ignore). This is a one-line mechanical change per checker. No behavior changes. The call site in the run path needs to be found and updated to pass the model — the plan must identify this call site explicitly before implementation starts.

### Risk: cache invalidation after SymbolNode changes

**Verdict: VERIFY BEFORE SHIPPING.**  
If existing `cache.json` entries have `SymbolNode` objects without `params`, the `#[serde(default)]` attribute handles deserialization gracefully.

Cached nodes will carry `params: vec![]` and emit no findings until the cache is rebuilt. A `--force-rebuild` flag or cache-version bump may be needed to ensure users see findings on first run after upgrading. Precedent: existing additive fields (`is_pure_delegation`) used the same pattern and users tolerated stale `false` values — empty `params` is acceptable on the same basis.

## Pre-implementation checklist

- [ ] Identify the exact call site where `architecture_checker.check()` is invoked (search `src/run.rs` and `src/run_context.rs`)
- [ ] Confirm whether `ArchModel` is already built in that code path for batch mode
- [ ] Read `src/symbol_extract.rs` lines 640–700 (`extract_symbols_for_file`) to find all Go function extraction paths that must emit `params`
- [ ] Confirm tree-sitter Go grammar field name for the parameter list on `function_declaration` and `method_declaration` nodes (should be `"parameters"`)
- [ ] Verify that `primitive_obsession.rs`'s `walk()` function extracts the same nodes we need (it does — confirmed by reading it)
- [ ] Check whether any existing tests assert `SymbolNode`'s exact field set (would fail if `params` is added without `..Default::default()` or explicit `vec![]`)

## Go / No-go

**Go.** Implementation is well-understood, risks are identified and mitigated, and the prerequisite gap (SymbolNode params) is concrete and solvable. Repeated parameter clusters are a genuine, detectably structural smell — this check delivers real value. Work is correctly phased with advisory severity and a backtest gate before any FP-sensitive deployment.

The only pre-condition for starting implementation: confirm the architecture checker dispatch call site, then proceed with Phase 1.
