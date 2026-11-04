# Research: Architecture — feature-envy-detection

**Date**: 2026-11-03

## 1. ArchModelChecker dispatch

`ArchModelChecker` is a trait defined in `src/architecture_checks.rs` (lines 604–618):

```rust
pub trait ArchModelChecker {
    fn name(&self) -> &str;
    fn check(&self, model: &ArchModel, config: &ArchitectureConfig) -> Vec<ArchFinding>;
    fn needs_private_symbols(&self) -> bool { false }
}
```

Registration is in `model_registry()` (`src/architecture_checks.rs:620–632`), which returns a `Vec<Box<dyn ArchModelChecker>>`. Current members: `InstabilityChecker`, `DipConcreteCouplingChecker`, `LcomChecker`, `GodClassChecker`, `IspFatInterfaceChecker`, `UnreferencedPrivateSymbolChecker`, `SingleCallSiteDelegationChecker`, `MiddleManChecker`, `GoPackageFocusChecker`.

`lookup_model(name)` (`src/architecture_checks.rs:634–636`) searches `model_registry()` by name. Called from `check.rs:1195` via `lookup_any_architecture_checker`, which is the resolution point for both CLI `kibitzer check architecture <name>` and MCP `run_checks`.

Dispatch in `run_architecture_check` (`src/check.rs:1237–1310`): when `lookup_any_architecture_checker` returns `AnyArchitectureChecker::Model(checker)`, the caller:
1. Calls `build_arch_model_for_check(repo_root, files, checker.needs_private_symbols())` — builds `ArchModel` from walked files, with or without private symbols depending on the checker's flag.
2. Calls `checker.check(&model, arch_config)`.
3. Formats each `ArchFinding` as `{file}:{line}: {message}` or bare `{message}` when no file/line is attached.
4. Returns a `CheckResult`.

**FeatureEnvyChecker's `needs_private_symbols` must return `true`**: the checker examines methods by ratio of field accesses, and unexported methods contribute to that ratio just as exported ones do.

**Findings flow back as strings** in `CheckResult.output`, formatted by `run_architecture_check`. No separate finding struct reaches the CLI/MCP — just the formatted message lines.

## 2. Comparable checker patterns

### GodClassChecker (`src/god_class.rs`)

Structure: `impl ArchModelChecker for GodClassChecker`, `check()` iterates `model.packages.values()`, calls `god_class_findings_for_package(pkg, model)`, collects results.

`god_class_findings_for_package`:
1. Builds a `FileCache` (per-package, not per-checker — see below).
2. Calls `methods_by_type(pkg)` to get `BTreeMap<type_name, Vec<method_id>>`.
3. For each type with `>= GOD_CLASS_MIN_METHODS` methods, computes WMC (re-parses source via `FileCache`), ATFD (re-parses source via `FileCache`), and TCC (from `model.field_accesses`).
4. Flags when **all three** exceed their thresholds.
5. `ArchFinding` has `file: None, line: None` — finding is type-scoped, not line-scoped.

**FileCache** (`src/god_class.rs:116–137`): a `HashMap<PathBuf, Option<(String, Tree)>>` owned per call to `god_class_findings_for_package`. Avoids re-parsing the same file for multiple methods on different types. `FeatureEnvyChecker` should use the same pattern when re-reading source for any AST-based detection.

### MiddleManChecker (`src/middle_man.rs`)

Structure: `impl ArchModelChecker for MiddleManChecker`, `check()` iterates `model.packages.values()`, calls `findings_for_package(pkg, &model.repo_root, &implementers)`, then sorts findings by `(file, line)`.

Pattern differences from GodClassChecker:
- Does NOT re-parse source — relies entirely on `SymbolNode.delegation_field` (pre-computed by `symbol_extract.rs` during model build).
- Needs `type_edges` from `model` (to skip types that implement interfaces).
- Sets `needs_private_symbols() = true`.
- Applies suppression: `INTENTIONAL_DELEGATION_SUFFIXES`, test files, min-method threshold, and interface implementers.
- Returns findings with `file: Some(...)`, `line: Some(...)` from the first delegating method.

**FeatureEnvyChecker should follow the MiddleManChecker pattern** for the parts that use `model.field_accesses` (no re-parse needed); it should follow the GodClassChecker pattern for the parts that need ATFD (re-parsing to walk AST for foreign field accesses).

## 3. Data flow: FieldAccessEdge layout

Defined in `src/arch_model.rs:207–214`:

```rust
pub struct FieldAccessEdge {
    pub from: String,   // SymbolNode::id of the method: "{pkg}::{Type}.{method}"
    pub to: String,     // synthetic field id: "{pkg}::{Type}.{field}"
    pub access: AccessKind,
    pub file: PathBuf,
    pub line: usize,
}
```

For a method `pkg::Type.Method`:
- `from = "pkg::Type.Method"` — identical to `SymbolNode::id` of the method.
- `to = "pkg::Type.FieldName"` — the receiver type's own field. The `"{pkg}::{Type}"` prefix identifies the owning type.

**Own-field vs. foreign-field discrimination**: `model.field_accesses` contains **only** own-receiver accesses (a method reading/writing a field on its own receiver). It does NOT contain foreign-type field accesses — those are detected by `god_class.rs`'s AST walker (`collect_foreign_field_accesses`, which re-reads source). Feature Envy needs both:
- **Own field accesses**: already in `model.field_accesses` — filter by `access.from` prefix matching the method id.
- **Foreign field accesses**: must come from the same AST walker in `god_class.rs`.

To count own-type accesses for a method `method_id`, the predicate is:
```rust
model.field_accesses.iter().filter(|a| a.from == method_id).count()
```

To determine if a `to` field belongs to the same type, check whether `to` starts with `"{pkg}::{type}."` (same type prefix as the method's owning type).

## 4. Ecosystem landscape: thresholds and comparable detectors

### JDeodorant (Tsantalis & Chatzigeorgiou 2009)
The canonical academic implementation. Uses "entity placement" distance: measures how "close" a method is to each class (by counting accesses to that class's fields/methods). Flags Move Method when the method is closer to a foreign class than its own.

Thresholds: ratio-based (method more attracted to foreign class than own). No fixed numeric cutoffs — uses relative distances. This is the most accurate but most complex approach. Not directly portable to a simple ratio detector.

### PMD (FeatureEnvy rule, based on Lanza/Marinescu 2006)
Uses: `ATFD > FEW` and `LAA < ONE_THIRD` and `FDP <= FEW`.
- `ATFD` (Access to Foreign Data) > 5: accesses to external fields.
- `LAA` (Locality of Attribute Accesses) < 0.33: fraction of accesses to own attributes out of all attribute accesses.
- `FDP` (Foreign Data Providers) ≤ 5: distinct foreign types accessed.

These values come directly from Lanza/Marinescu *Object-Oriented Metrics in Practice* (2006). PMD's thresholds are `FEW = 5`, `ONE_THIRD = 1/3`.

**Kibitzer's closest approximation**: ratio = foreign/(foreign+own); flag when ratio >= threshold and foreign_count >= minimum. This is a simplified LAA + ATFD combination. Reasonable starting thresholds: `FEATURE_ENVY_RATIO_THRESHOLD = 0.33` (matching PMD's LAA ceiling, inverted: LAA < 1/3 means foreign > 2/3 of local, but kibitzer uses foreign/(foreign+own) which starts flagging at 0.33) and `FEATURE_ENVY_MIN_FOREIGN = 3` (conservative relative to PMD's `ATFD > 5`).

### SonarQube
Uses similar ratios to PMD. No single published threshold — values are configurable. Community defaults tend to match PMD's FEW=5 / ONE_THIRD.

### IntelliJ IDEA (MoveMethod suggestion)
Heuristic: flags a method when it references members of another class more than it references its own class's members. Internally uses AST-level type resolution to count accesses; thresholds are not published.

**Summary for Kibitzer v1**: adopt `FEATURE_ENVY_RATIO_THRESHOLD = 0.33` and `FEATURE_ENVY_MIN_FOREIGN = 3`. Both are consistent with PMD/Lanza-Marinescu while being slightly less aggressive on the foreign-count side, appropriate for a first corpus run before tuning.

## 5. Tech debt disposition for `src/god_class.rs`

**git log**: 11 commits to `src/god_class.rs` (VERIFIED: `git log --oneline src/god_class.rs | wc -l` = 11). File is 746 lines. Recent commits include cross-package ATFD resolution (#86), type-hierarchy integration (#99), and false-positive fixes (#136).

**Hotspot assessment**: moderate churn (11 commits), moderate size (746 lines). Not a high-complexity × high-churn hotspot by the standard metric — the file has grown deliberately with each new capability (ATFD, cross-package resolution, ISP shared helper) and each commit has been purposeful. The public-crate surface area is already well-factored: the functions `FeatureEnvyChecker` needs are already `pub(crate)`:

| Function | Visibility | Purpose |
|---|---|---|
| `FileCache` | `pub(crate)` | Per-package file parse cache |
| `find_method_node` | `pub(crate)` | Re-locate a method's AST node after fresh parse |
| `collect_typed_locals` | `pub(crate)` | Enumerate syntactically-typed locals in a method |
| `simple_type_name` | `pub(crate)` | Extract a bare/qualified type name from an AST node |
| `resolve_qualified_type` | `pub(crate)` | Resolve `alias.Type` to `pkg::Type` via `file_import_aliases` |
| `is_call_target` | `pub(crate)` | Test whether a selector is used in call position |
| `node_text` | `pub(crate)` | Extract source slice for an AST node |

The `collect_foreign_field_accesses` / `walk_selectors_on_typed_locals` pair is `fn` (private to the module), not yet `pub(crate)`. `FeatureEnvyChecker` needs to call `distinct_foreign_field_accesses` (also private). Two options:

**Disposition recommendation: Extend as-is**

Rationale: `god_class.rs` is not a hotspot. Its per-method ATFD function (`distinct_foreign_field_accesses`) and its AST walker (`collect_foreign_field_accesses` / `walk_selectors_on_typed_locals`) are exactly the right-shaped building blocks for Feature Envy's foreign-access counting. The existing `pub(crate)` helpers (`FileCache`, `find_method_node`, etc.) are already the correct seam. The only change required is promoting `distinct_foreign_field_accesses` (and its callees `collect_foreign_field_accesses` / `walk_selectors_on_typed_locals`) from `fn` to `pub(crate) fn` — a purely mechanical visibility change, not a structural refactor. This is simpler and safer than extracting a shared module that neither GodClassChecker nor FeatureEnvyChecker needs to collaborate on beyond this one function.

The "refactor-first" case would be justified if `god_class.rs` were actively in flux (a true hotspot) or if Feature Envy needed to diverge from ATFD's existing semantics. Neither is true: 11 commits over the file's lifetime is deliberate layering, not churn, and Feature Envy's foreign-count semantics are identical to ATFD's — it wants the same walking code, not a different one.

## From `project_plans/interface-impl-carveout-delegation/research/architecture.md`

That document covers `ArchModel.type_edges` and `SymbolNode::id` construction. Findings directly applicable to `FeatureEnvyChecker`:

- **Method id format** (confirmed): `"{pkg}::{TypeName}.{methodName}"` for methods, `"{pkg}::{TypeName}"` for the owning type.
- **Owning type id reconstruction**: given a method `SymbolNode`, extract `package = sym.id.split_once("::").map_or("", |(p, _)| p)`, then `type_id = format!("{package}::{}", sym.parent.as_deref().unwrap())`.
- **`type_edges` suppression pattern** (reused by MiddleManChecker and applicable to FeatureEnvyChecker's adapter suppression): build a `HashSet<String>` of "types that implement any interface" once per `check()` call; O(1) per candidate method.

The carveout research also confirms that Go structural interface satisfaction is not in `type_edges` — the adapter/wrapper suffix heuristic from MiddleManChecker is the right fallback for Go in v1.

## Summary

- `FeatureEnvyChecker` registers in `model_registry()`, is dispatched through `lookup_any_architecture_checker` → `build_arch_model_for_check(needs_private_symbols=true)` → `checker.check(&model, config)`.
- Own field accesses are available directly from `model.field_accesses` (filter `access.from == method_id`); foreign field accesses require re-parsing source via the existing ATFD walker in `god_class.rs`.
- `methods_by_type()` (already `pub(crate)` in `architecture_checks.rs`) handles the type-grouping iteration; `FileCache` (already `pub(crate)` in `god_class.rs`) handles per-method file caching.
- Thresholds: `FEATURE_ENVY_RATIO_THRESHOLD = 0.33`, `FEATURE_ENVY_MIN_FOREIGN = 3` (consistent with PMD/Lanza-Marinescu 2006, conservative for a first run).
- Disposition: **Extend as-is** — promote `distinct_foreign_field_accesses` and its two private callees to `pub(crate)`, no structural refactor warranted.
