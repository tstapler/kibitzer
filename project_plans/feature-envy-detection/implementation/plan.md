# Implementation Plan: feature-envy-detection

**Feature**: New `feature-envy` ArchModelChecker detecting Move Function / Move Field candidates in Go code
**Date**: 2026-11-03
**Status**: Ready for implementation
**ADRs**: None (decisions embedded in Pattern Decisions below)

---

## Domain Glossary

| Term | Definition |
|------|-----------|
| **Feature Envy** | A method that accesses another type's fields or methods far more than its own type's, signaling the method belongs with that other type. |
| **Foreign field access** | A field read/write on a syntactically-typed local variable whose inferred type differs from the receiver's own type; counted by the ATFD walker in `god_class.rs`. |
| **Local field access** | A field access recorded in `ArchModel.field_accesses` where `access.from == method_id`; represents own-receiver state access. |
| **Foreign method call** | A call in `ArchModel.call_edges` where `edge.from == method_id` and the callee's type prefix differs from the receiver's own type. |
| **Foreign ratio** | `foreign_count / (foreign_count + local_count)` — the fraction of a method's total data accesses that land on foreign types. |
| **Dominant foreign type** | The foreign type with the highest access count for a given method; names the suggested Move Function / Move Field target. |
| **Move Function candidate** | A method whose foreign ratio and foreign count exceed thresholds, counting both foreign field accesses and foreign method calls on the foreign side. |
| **Move Field candidate** | Same as Move Function, but restricted to field accesses only on both sides (no method calls). |
| **ATFD walker** | The `collect_foreign_field_accesses` / `walk_selectors_on_typed_locals` pair in `god_class.rs` that walks a method AST and collects `(foreign_type, field)` pairs. |
| **min-total guard** | Suppression that skips methods with fewer than `FEATURE_ENVY_MIN_TOTAL` combined accesses; prevents noise on tiny or trivial methods. |
| **own-type** | The receiver's declared type name, used to distinguish own-receiver accesses from foreign-type accesses in both walkers. |
| **adapter suppression** | Skipping types named `*Adapter`, `*Wrapper`, `*Proxy`, or `*Decorator`, same as `middle_man.rs`; these intentionally delegate and always look envious. |
| **FileCache** | Per-package cache of `(source_string, Tree)` pairs from `god_class.rs`; must be constructed fresh per package to avoid stale-tree panics. |
| **per-type access map** | `HashMap<String, usize>` mapping foreign type name to access count for one method body; needed for dominant-type identification. |

---

## Pattern Decisions

### Approach Comparison

| Approach | Strength | Weakness | Decision |
|----------|---------|---------|---------|
| **A: Extend god_class.rs directly** — add `FeatureEnvyChecker` as a second struct in the same file | Zero new files; shared helpers already in scope | Violates the established one-checker-per-file pattern; inflates an already complex file | Rejected |
| **B: New `src/feature_envy.rs` with promoted helpers** — promote `collect_foreign_field_accesses` to `pub(crate)`, write new file | Matches the pattern of every other arch-model checker (middle_man.rs, lcom.rs); clean blast radius; mechanical promotion only | Requires one `pub(crate)` visibility change in `god_class.rs` | **Chosen** |
| **C: Extract shared `go_access_walker.rs` module** — move the ATFD walker out of `god_class.rs` into a new shared module | Cleanest long-term design; eliminates duplication for future callers | Larger blast radius; requires refactoring a stable file (11 commits); premature for one new caller | Rejected |

**Chosen**: Approach B. The one `pub(crate)` promotion in `god_class.rs` is purely additive and does not change behavior; the precedent for this exact mechanical change is `FileCache` and `find_method_node`, which are already `pub(crate)` for exactly this reason. `walk_selectors_on_typed_locals` is called only by `collect_foreign_field_accesses` and does not need to be exported — widening its visibility unnecessarily would expand the pub(crate) surface area.

### Foreign Call Counting for Move Function

The existing ATFD walker (`collect_foreign_field_accesses`) explicitly excludes call targets via `is_call_target`. Move Function requires counting foreign method calls too. Rather than duplicating or parameterizing the walker, foreign method calls are read from `ArchModel.call_edges` with `edge.from == method_id` where the callee type prefix differs from own-type. This avoids touching the walker and re-uses already-built model data.

### Per-Type Count vs. Flat Set

`distinct_foreign_field_accesses` (the existing helper) returns a flat `usize` count — unusable for dominant-type identification. The new checker needs `foreign_field_accesses_per_type` returning `HashMap<String, usize>` (foreign type → count of distinct accessed fields). The existing helper is not reused; the per-type variant runs the same walk but accumulates into a map.

---

## Tech Debt Disposition

**`god_class.rs` — Extend as-is.** The file is stable (11 commits, deliberate layering, well-tested). One purely additive promotion is required:

1. `fn collect_foreign_field_accesses(…)` → `pub(crate) fn collect_foreign_field_accesses(…)`

`walk_selectors_on_typed_locals` is called only by `collect_foreign_field_accesses`; it stays private — there is no need to widen its visibility. No other changes to `god_class.rs`. `distinct_foreign_field_accesses` remains private and is not shared — its flat-count shape is wrong for feature envy's dominant-type tracking.

---

## Migration Plan

No data migrations required. `ArchModel` is not modified. No schema changes.

---

## Observability Plan

Findings report the dominant foreign type and both foreign and local counts inline in the message, e.g.:

```
[feature-envy:move-function] pkg::Type.Method (type.go:N): 7 foreign accesses (5 to ForeignType, 2 to OtherType) vs 1 own (ratio 0.88, threshold 0.33); dominant target: ForeignType — consider Move Function
```

No metrics, dashboards, or structured logging beyond the standard `ArchFinding` output.

---

## Risk Control

| Risk | Likelihood | Mitigation |
|------|-----------|-----------|
| Zero-own-access ratio inflation (`own=0` → `ratio=1.0`) | High — common for tiny forwarding methods | **(F1)** Two-layer guard: (1) `FEATURE_ENVY_MIN_TOTAL` skips methods with `foreign + local < FEATURE_ENVY_MIN_TOTAL`; (2) own-signal pre-filter in Task 2.2.1c skips any method where `local == 0 && foreign_total < FEATURE_ENVY_MIN_FOREIGN`, closing the gap where a fully-foreign method at exactly `FEATURE_ENVY_MIN_TOTAL` slips through layer 1. Mirrors `methods_with_own_signal` at `architecture_checks.rs:867`. |
| Unnamed pure-delegation types bypassing adapter suppression | Medium — adapters without naming-convention suffixes (Adapter/Wrapper/Proxy/Decorator) look envious but are intentional | **(F2)** `if sym.is_pure_delegation { continue; }` per method in Task 2.2.1c (pre-computed by `symbol_extract.rs`, zero cost). Catches delegation types regardless of naming convention. |
| FileCache stale-tree panic | High if misused | Construct `FileCache::new()` once per package, never shared across packages — same fix god_class.rs already applies |
| Adapter/constructor false positives (named) | Medium | Adapter suppression list (same as `middle_man.rs`); `New*` functions caught by `FEATURE_ENVY_MIN_FOREIGN` ≥ 3 (constructors rarely have 3+ distinct foreign field accesses) |
| Same-type parameter distortion (`func (t T) Merge(other T)`) | Low | `other.X` correctly dropped by `foreign_type_of` guard (same-name type is own type, not foreign) |
| FP rate exceeds acceptance bar on public corpus backtest | Medium | If the estimated FP rate on the public corpus exceeds 30%, raise `FEATURE_ENVY_MIN_FOREIGN` to 5 (PMD's calibrated value) and re-run before shipping; document the threshold-adjustment decision in `decisions/validation.md`. |

---

## Unresolved Questions

None. All design choices confirmed by existing code and research. Backtest results to be recorded in `project_plans/feature-envy-detection/decisions/validation.md` during Phase 5.

---

## Dependency Visualization

```
Phase 1 (promote helpers)
  └─ Phase 2 (new feature_envy.rs)
       ├─ Phase 3 (registration in main.rs + model_registry)
       └─ Phase 4 (unit tests)
            └─ Phase 5 (backtest + validation artifact)
```

Phases 3 and 4 are independent of each other once Phase 2 is complete.

---

## Phase 1: Prerequisite — Promote ATFD Walker Helpers

### Epic 1.1: Expose god_class.rs internals for cross-module reuse

#### Story 1.1.1: Promote `collect_foreign_field_accesses`

**Given** `collect_foreign_field_accesses` is `fn` (private) in `src/god_class.rs`  
**When** `feature_envy.rs` imports and calls it  
**Then** the crate compiles without visibility errors

**Acceptance criteria**: `cargo build` passes with no changes other than adding `pub(crate)` to the `collect_foreign_field_accesses` function signature.

##### Task 1.1.1a — Promote helper visibility in `src/god_class.rs` (XS)

Files: `src/god_class.rs`

Add `pub(crate)` to `collect_foreign_field_accesses` (line ~429) only. `walk_selectors_on_typed_locals` is called only by `collect_foreign_field_accesses` and does not need to be exported — promoting it would unnecessarily widen the pub(crate) surface area. No other changes.

```rust
// Before
fn collect_foreign_field_accesses(…) { … }

// After
pub(crate) fn collect_foreign_field_accesses(…) { … }
```

Verify: `cargo build` exits 0.

---

## Phase 2: Core Checker Implementation

### Epic 2.1: Per-method access counting helpers

#### Story 2.1.1: `foreign_field_accesses_per_type` helper

**Given** a method AST node, source string, own-type name, and optional file import aliases  
**When** `foreign_field_accesses_per_type` runs  
**Then** it returns a `HashMap<String, usize>` mapping each distinct foreign type to the count of distinct `(type, field)` pairs accessed; call targets are excluded (field accesses only)

**GWT example**:

Given a Go method body:
```go
func (o *Order) CalculateDiscount(c Customer) float64 {
    _ = c.Tier
    _ = c.Tier  // duplicate — same (Customer, Tier) pair
    _ = c.Balance
    return 0
}
```
When `foreign_field_accesses_per_type` runs with `own_type = "Order"`  
Then it returns `{"Customer": 2}` (Tier + Balance = 2 distinct pairs)

##### Task 2.1.1a — Implement `foreign_field_accesses_per_type` in new `src/feature_envy.rs` (S)

Files: `src/feature_envy.rs` (new)

```rust
/// Counts distinct `(type, field)` pairs accessed on each foreign type within `method`,
/// using the same ATFD walker as `GodClassChecker`. Call targets are excluded (field
/// accesses only). Returns a map of `foreign_type_name -> distinct_pair_count`.
pub(crate) fn foreign_field_accesses_per_type(
    method: Node,
    source: &str,
    own_type: &str,
    file_aliases: Option<&HashMap<String, String>>,
) -> HashMap<String, usize> {
    let mut all_pairs: HashSet<(String, String)> = HashSet::new();
    crate::god_class::collect_foreign_field_accesses(
        method, source, own_type, file_aliases, &mut all_pairs,
    );
    let mut counts: HashMap<String, usize> = HashMap::new();
    for (ty, _field) in all_pairs {
        *counts.entry(ty).or_insert(0) += 1;
    }
    counts
}
```

#### Story 2.1.2: `foreign_call_counts_per_type` helper

**Given** a method ID and the arch model  
**When** `foreign_call_counts_per_type` runs  
**Then** it returns a `HashMap<String, usize>` counting calls to each foreign type's methods, using `ArchModel.call_edges`

**GWT example**:

Given `model.call_edges` containing:
- `{from: "pkg::Order.Calculate", to: "pkg::Pricer.GetRate", resolved: true}`
- `{from: "pkg::Order.Calculate", to: "pkg::Pricer.GetBase", resolved: true}`
- `{from: "pkg::Order.Calculate", to: "pkg::Logger.Log", resolved: true}`

When `foreign_call_counts_per_type("pkg::Order.Calculate", model, "Order")` runs  
Then it returns `{"pkg::Pricer": 2, "pkg::Logger": 1}`

##### Task 2.1.2a — Implement `foreign_call_counts_per_type` in `src/feature_envy.rs` (S)

Files: `src/feature_envy.rs`

```rust
/// Counts method calls to each foreign type from `model.call_edges`, for the Move
/// Function variant (which counts both field accesses and method calls on the foreign
/// side). `own_type_prefix` is the method's own type (bare name, same-package). A call
/// is "foreign" when its callee type prefix differs from own-type prefix.
fn foreign_call_counts_per_type(
    method_id: &str,
    model: &ArchModel,
    own_type: &str,
) -> HashMap<String, usize> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for edge in &model.call_edges {
        if !edge.resolved || edge.from != method_id {
            continue;
        }
        // `to` is `{pkg}::{Type}.{method}` — extract type portion.
        let callee_type = edge.to.rsplit_once('.').map(|(prefix, _)| prefix);
        if let Some(ty) = callee_type {
            if !ty.ends_with(&format!("::{own_type}")) {
                *counts.entry(ty.to_string()).or_insert(0) += 1;
            }
        }
    }
    counts
}
```

#### Story 2.1.3: `local_field_access_count` and `dominant_type` helpers

**Given** a method ID and the arch model  
**When** `local_field_access_count` runs  
**Then** it returns the count of entries in `model.field_accesses` where `access.from == method_id`

**GWT example** for `dominant_type`:

Given `counts = {"ForeignType": 5, "OtherType": 2}`  
When `dominant_type(&counts)` runs  
Then it returns `Some("ForeignType")`

##### Task 2.1.3a — Implement `local_field_access_count` and `dominant_type` in `src/feature_envy.rs` (XS)

Files: `src/feature_envy.rs`

```rust
fn local_field_access_count(method_id: &str, model: &ArchModel) -> usize {
    model.field_accesses.iter().filter(|a| a.from == method_id).count()
}

fn dominant_type(counts: &HashMap<String, usize>) -> Option<&str> {
    counts.iter().max_by_key(|(_, &v)| v).map(|(k, _)| k.as_str())
}
```

---

### Epic 2.2: `FeatureEnvyChecker` struct and trait implementation

#### Story 2.2.1: Checker struct, constants, and suppression logic

**Given** a method belonging to a type whose name ends in "Adapter"  
**When** `FeatureEnvyChecker.check(model, config)` runs  
**Then** no finding is emitted for that method (adapter suppression)

**GWT example for test-file suppression**:

Given a method defined in `service_test.go` (`.file` ends with `_test.go`)  
When `FeatureEnvyChecker.check(model, config)` runs  
Then no finding is emitted for that method

##### Task 2.2.1a — Implement constants, struct, and trait boilerplate in `src/feature_envy.rs` (XS)

Files: `src/feature_envy.rs`

```rust
/// Foreign-access fraction above which a method is flagged. Consistent with
/// PMD/Lanza-Marinescu 2006: "more accesses to other classes than to its own."
/// 0.33 means foreign accesses make up at least one-third of total — set intentionally
/// permissive for a syntactic detector that cannot see through generics or return-type
/// inference.
const FEATURE_ENVY_RATIO_THRESHOLD: f64 = 0.33;

/// Minimum number of distinct foreign accesses (field + call for move-function; field
/// only for move-field) before flagging — noise guard for tiny methods.
const FEATURE_ENVY_MIN_FOREIGN: usize = 3;

/// Minimum combined (foreign + local) accesses before a method is evaluated at all.
/// Zero-own-access methods have ratio=1.0 by definition and generate false positives
/// for any threshold; this guard also suppresses those.
const FEATURE_ENVY_MIN_TOTAL: usize = 4;

const INTENTIONAL_DELEGATION_SUFFIXES: [&str; 4] =
    ["Adapter", "Wrapper", "Proxy", "Decorator"];

pub struct FeatureEnvyChecker;
```

##### Task 2.2.1b — Implement `ArchModelChecker` trait for `FeatureEnvyChecker` (M)

Files: `src/feature_envy.rs`

Implement:
- `fn name(&self) -> &str { "feature-envy" }`
- `fn needs_private_symbols(&self) -> bool { true }`
- `fn check(&self, model: &ArchModel, _config: &ArchitectureConfig) -> Vec<ArchFinding>`

The check method iterates `model.packages.values()`, calls `feature_envy_findings_for_package` per package, flattens and returns.

##### Task 2.2.1c — Implement `feature_envy_findings_for_package` with suppression and ratio logic (M)

Files: `src/feature_envy.rs`

Algorithm per package:
1. Build `symbols_by_id: HashMap<&str, &SymbolNode>` from package symbols.
2. Create `FileCache::new()`.
3. Iterate `methods_by_type(pkg)`:
   a. Skip if type name ends with any `INTENTIONAL_DELEGATION_SUFFIXES`.
   b. For each method id in the type's methods:
      - Look up `SymbolNode`; skip if file is a test file (`checkers::is_test_file`).
      - **(F2)** Skip if `sym.is_pure_delegation` — `if sym.is_pure_delegation { continue; }`. These are adapters/wrappers pre-computed by `symbol_extract.rs` that intentionally delegate everything to a wrapped type; they always look envious but are never candidates. This catches delegation types that don't carry naming-convention suffixes (Adapter/Wrapper/Proxy/Decorator), so it complements the type-name suffix check in step 3a rather than replacing it.
      - Load `(source, tree)` from `FileCache`; skip on cache miss.
      - Locate `method_node` via `find_method_node`; skip on miss.
      - Compute `field_counts: HashMap<String, usize>` via `foreign_field_accesses_per_type`.
      - **Normalize `field_counts` keys** (required): `foreign_field_accesses_per_type` returns bare type names for same-package foreign types (e.g., `"Customer"`) because `god_class::foreign_type_of` only qualifies cross-package types. `foreign_call_counts_per_type` always produces `"pkg::Type"` keys from `edge.to`. Before merging, qualify any bare key (one with no `::`) to `"{pkg}::{key}"` using the method's own package name. Cross-package keys already contain `::` and pass through unchanged. This prevents the same foreign type from being split across two keys (`"Customer"` vs. `"pkg::Customer"`) in the merged map, which would break dominant-type identification and threshold evaluation.
      - Compute `call_counts: HashMap<String, usize>` via `foreign_call_counts_per_type`.
      - Compute `local = local_field_access_count(method_id, model)`.
      - **(F1) own-signal pre-filter**: compute `foreign_total = sum of all values in the merged (field_counts + call_counts) map`; if `local == 0 && foreign_total < FEATURE_ENVY_MIN_FOREIGN`, skip — zero-own-access helpers yield ratio=1.0 by definition and are almost never true candidates. The `FEATURE_ENVY_MIN_TOTAL` guard already catches many of these, but a method with exactly `FEATURE_ENVY_MIN_TOTAL` total accesses that are all foreign (e.g., 4 foreign + 0 local) still passes the min-total guard; this pre-filter closes that gap by requiring at least `FEATURE_ENVY_MIN_FOREIGN` foreign accesses before a zero-own method can proceed, preventing the noisiest false positives. See `architecture_checks.rs:867` (`methods_with_own_signal`) for the existing precedent.
      - **Move Function variant**:
        - `foreign_fn = sum of (field_counts + call_counts)` merged per type.
        - `total_fn = foreign_fn + local`.
        - If `total_fn >= FEATURE_ENVY_MIN_TOTAL && foreign_fn >= FEATURE_ENVY_MIN_FOREIGN && foreign_fn as f64 / total_fn as f64 >= FEATURE_ENVY_RATIO_THRESHOLD`:
          - Find `dominant_fn` from merged map.
          - Emit `[feature-envy:move-function]` finding with method location, counts, dominant type.
      - **Move Field variant**:
        - `foreign_fld = sum of field_counts`.
        - `total_fld = foreign_fld + local`.
        - If `total_fld >= FEATURE_ENVY_MIN_TOTAL && foreign_fld >= FEATURE_ENVY_MIN_FOREIGN && foreign_fld as f64 / total_fld as f64 >= FEATURE_ENVY_RATIO_THRESHOLD`:
          - Find `dominant_fld` from `field_counts`.
          - Emit `[feature-envy:move-field]` finding.
4. Sort findings by `(file, line)`.

**GWT example** for move-function:

Given a Go method `Order.Calculate` in `order.go` line 42 with 5 foreign field accesses + 3 foreign calls to `Pricer`, and 1 local field access.  
When `FeatureEnvyChecker.check` runs against a model containing this method.  
Then a finding is emitted matching:
```
[feature-envy:move-function] pkg::Order.Calculate (order.go:42): 8 foreign accesses (5 to pkg::Pricer, 3 to pkg::Logger) vs 1 own (ratio 0.89, threshold 0.33); dominant target: pkg::Pricer — consider Move Function
```
at file `order.go` line 42.

**GWT example** for move-field (no method calls counted):

Given the same method but only the 5 field accesses count (method calls excluded from move-field).  
When `FeatureEnvyChecker.check` runs.  
Then a finding is emitted matching:
```
[feature-envy:move-field] pkg::Order.Calculate (order.go:42): 5 foreign field accesses (to pkg::Pricer) vs 1 own (ratio 0.83, threshold 0.33); dominant target: pkg::Pricer — consider Move Field (field accesses only, method calls excluded)
```
at file `order.go` line 42.

---

## Phase 3: Registration and Wiring

### Epic 3.1: Wire checker into the runtime

#### Story 3.1.1: Register `FeatureEnvyChecker` in `model_registry()` and `main.rs`

**Given** the checker is registered in `model_registry()`  
**When** `kibitzer check feature-envy <dir>` runs  
**Then** it dispatches to `FeatureEnvyChecker.check` without error

##### Task 3.1.1a — Add `mod feature_envy` to `src/main.rs` (XS)

Files: `src/main.rs`

Insert `mod feature_envy;` in alphabetical order among the existing `mod` declarations (between `mod false_positive;` and `mod git_cmd;`).

##### Task 3.1.1b — Add to `model_registry()` in `src/architecture_checks.rs` (XS)

Files: `src/architecture_checks.rs`

Add to `model_registry()`:
```rust
Box::new(crate::feature_envy::FeatureEnvyChecker),
```
Position: after `Box::new(crate::middle_man::MiddleManChecker)`, before `Box::new(crate::go_package_focus::GoPackageFocusChecker::default())`.

Verify: `cargo build` and `kibitzer check list` shows `feature-envy` in output.

---

## Phase 4: Unit Tests

### Epic 4.1: Fixture-driven unit tests

#### Story 4.1.1: Six required test cases

**Given** inline Go source fixtures written as test helper strings  
**When** each unit test invokes `FeatureEnvyChecker.check` on a minimal `ArchModel` built from the fixture  
**Then** the right findings (or absence of findings) are produced

##### Task 4.1.1a — Write test fixtures and test functions in `src/feature_envy.rs` (M)

Files: `src/feature_envy.rs` (append `#[cfg(test)] mod tests { … }`)

Required test cases:
1. **Flagged Move Function candidate** — method with 4 foreign field accesses + 2 foreign calls vs 1 local; expect `[feature-envy:move-function]` finding naming the dominant foreign type.
2. **Flagged Move Field candidate** — method with 4 foreign field accesses + 0 foreign calls vs 1 local; expect `[feature-envy:move-field]` finding; no move-function finding (call-count is 0 so the same ratio holds for both, but verify both labels emit).
3. **Suppressed test-file method** — same access counts but method is in a file ending in `_test.go`; expect zero findings.
4. **Below-threshold true negative** — method with 2 foreign field accesses vs 3 local (ratio = 0.4 but `foreign_count < FEATURE_ENVY_MIN_FOREIGN`); expect zero findings. Second sub-case: 3 foreign vs 7 local (ratio = 0.3 < 0.33 threshold); expect zero findings.
5. **Same-package foreign type key normalization** — method `pkg::Order.Calculate` with 3 foreign field accesses to a same-package type `Customer` (returned as bare `"Customer"` by `foreign_field_accesses_per_type`) and 2 foreign calls to `pkg::Customer` (returned as `"pkg::Customer"` by `foreign_call_counts_per_type`), plus 1 local access. After key normalization, both counts must merge under `"pkg::Customer"` for a combined foreign count of 5 (not split 3+2 under separate keys). Expect a `[feature-envy:move-function]` finding with dominant type `pkg::Customer` and foreign count 5; if keys are not normalized the combined count never reaches `FEATURE_ENVY_MIN_FOREIGN` from either key alone and the finding is absent — confirming the normalization step is load-bearing.
6. **FileCache stale-tree regression** — two Go source files in the same package, second file shorter than first; verify `feature_envy_findings_for_package` completes without panicking. Modeled on `god_class.rs:file_cache_parses_a_second_shorter_file_correctly_after_a_longer_first_one`.

Test infrastructure: use `tempdir`, write Go source to disk, build `ArchModel` by calling `arch_model::build_arch_model` or construct it manually in the same pattern as `god_class.rs`'s tests (use `method_symbol`, `FieldAccessEdge` helpers).

---

## Phase 5: Backtest and Validation

### Epic 5.1: Session-transcript corpus backtest

#### Story 5.1.1: Run `kibitzer check backtest feature-envy`

**Given** the checker is registered and `~/.claude/projects/*/*.jsonl` session transcripts exist  
**When** `kibitzer check backtest feature-envy` runs  
**Then** it replays real historical edits and reports TP/FP counts; results are recorded in the validation artifact

##### Task 5.1.1a — Run backtest and record results (M)

Files: `project_plans/feature-envy-detection/decisions/validation.md` (new, plain prose)

Steps:
1. `cargo build --release`
2. `kibitzer check backtest feature-envy`
3. Record command and output (finding count, TP rate if the backtest tool reports it).

---

### Epic 5.2: Public repo corpus backtest

#### Story 5.2.1: Run checker against public corpus

**Given** the repos from `docs/backtest-repos.md` are cloned via `scripts/clone-backtest-repos.sh`  
**When** `kibitzer run <repo>` or equivalent runs for each corpus repo  
**Then** findings are collected; TP/FP counts are recorded with any notable false positives noted by type name

##### Task 5.2.1a — Run public corpus backtest (M)

Files: `project_plans/feature-envy-detection/decisions/validation.md`

Steps:
1. `scripts/clone-backtest-repos.sh` if repos are not present.
2. For each repo: `kibitzer run <path> --check feature-envy 2>&1 | tee /tmp/fe-<repo>.txt`
3. Count total findings; manually triage a sample (10–20) for false positives.
4. Record: total findings per repo, estimated FP rate, any systemic false-positive pattern.

##### Task 5.2.1b — Write validation artifact (S)

Files: `project_plans/feature-envy-detection/decisions/validation.md`

Write concise validation summary covering:
- Backtest corpus results (TP rate from session-transcript run)
- Per-repo public corpus finding counts
- Estimated FP rate; if the estimated FP rate on the public corpus exceeds 30%, raise `FEATURE_ENVY_MIN_FOREIGN` to 5 (PMD's calibrated value) and re-run before shipping; document the threshold-adjustment decision in `decisions/validation.md`
- Any threshold adjustment made as a result (if any)
- Final verdict: checker is ready / requires threshold adjustment before shipping
