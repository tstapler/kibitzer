# Implementation Plan: Encapsulate Variable Check

## Overview

Add a whole-repo `ArchModelChecker` that flags exported Go struct fields written from
outside the package that declares them — Fowler's **Encapsulate Variable**. The
`FieldAccessEdge`/`AccessKind::Write` infrastructure from issue #39 is already live;
this plan consumes it.

---

## Task Breakdown

### Task 1 — `src/encapsulate_variable.rs`: core checker

**File**: `src/encapsulate_variable.rs` (new, mirroring `god_class.rs` / `middle_man.rs` layout)

**Logic**:

```
for edge in &model.field_accesses:
    if edge.access != AccessKind::Write: continue
    caller_pkg = edge.from.split_once("::").left
    field_id   = edge.to   // "{field_pkg}::{TypeName}.{FieldName}"
    field_pkg  = field_id.split_once("::").left
    if caller_pkg == field_pkg: continue          // same-package — fine
    type_name  = field_id between "::" and "."
    field_name = field_id after last "."
    if field_name.starts_with(lowercase): continue  // unexported field
    if type_name ends_with any DTO_SUFFIXES: continue
    emit ArchFinding { file: edge.file, line: edge.line, message: ... }
```

**DTO suffix list** (compile-time constant):
```rust
const DTO_SUFFIXES: &[&str] = &[
    "Config", "Options", "Opts", "Params", "Args",
    "Request", "Response", "Settings", "Result",
];
```

**Finding message**:
```
[encapsulate-variable] {caller_package}/{caller_symbol} writes {field_pkg}.{TypeName}.{FieldName} from outside its package — consider adding a setter
```

**`ArchModelChecker` impl**:
- `name()` → `"encapsulate-variable"`
- `needs_private_symbols()` → `false` (only exported fields matter)
- `check(model, _config)` → iterate `model.field_accesses`, apply logic above

**Deduplication**: a field written `N` times from the same caller method/function emits one finding per write site (each has a distinct `file`+`line`). If the same field is written from multiple different callers, each produces its own finding — consistent with how `import-cycles` reports one finding per cycle edge.

**Registration**: add `Box::new(EncapsulateVariableChecker)` to `model_registry()` in `architecture_checks.rs`.

**Estimate**: ~3h

---

### Task 2 — `src/main.rs`: declare module

Add `mod encapsulate_variable;` to `src/main.rs` (adjacent to `mod god_class;` at line 29).

**Estimate**: 15m

---

### Task 3 — `src/config.rs`: wire into `default_checks()`

Add to `go_exception_checks()`:
```rust
whole_repo_check("go-encapsulate-variable", "encapsulate-variable"),
```

Place after `go-encapsulate-collection` for thematic grouping.

**Estimate**: 15m

---

### Task 4 — Unit tests in `src/encapsulate_variable.rs`

Write a `#[cfg(test)]` block using the same test-model builder pattern as
`architecture_checks.rs`'s `model_with_edges` / `lcom_field_access` helpers.

Test cases (all ACs covered):
1. Cross-package write on exported field → finding emitted
2. Same-package write → no finding
3. Cross-package write on unexported field → no finding
4. Cross-package write on a type ending in `Config` → no finding
5. Cross-package write on a type ending in `Options` → no finding
6. Cross-package `AccessKind::Read` → no finding
7. Finding message format matches expected string exactly

**Estimate**: 2h

---

### Task 5 — `docs/go-encapsulate-variable-false-positives.md`

Create the false-positive documentation file (same pattern as
`docs/go-encapsulate-collection-false-positives.md`). Content:
- Description of the check
- Known false-positive patterns (DTO structs, builder patterns, test setup)
- How to suppress with `kibitzer:ignore encapsulate-variable -- reason`
- How to suppress an entire type via `.kibitzer/inspect.json` `disabled` or `scope`

**Estimate**: 45m

---

### Task 6 — Backtest against corpus

Run checker against the standard backtest repos (`kubernetes/kubernetes` at minimum,
ideally also `BurntSushi/ripgrep`):

```bash
kibitzer check backtest encapsulate-variable   # transcript-based
# and
kibitzer run <corpus-repo>                     # real-world repo corpus
```

Triage false positives with `scripts/backtest-triage.py`. Document before/after
numbers with the DTO suffix allowlist applied.

If noise is too high: expand DTO suffix list, add `_test.go` file skipping (same as
`go-encapsulate-collection`'s test-file carve-out), or move the check from
`default_checks()` to opt-in. Record the decision in the false-positive doc.

**Estimate**: 2h (depends on corpus clone freshness)

---

### Task 7 — `cargo test` green

Run `cargo test` after all code changes. Fix any compilation or test failures.

**Estimate**: 30m

---

## Implementation Order

1. Task 1 (checker core) — the central unit of work; everything else gates on it
2. Task 2 (mod declaration) — needed to compile
3. Task 4 (unit tests) — drives correctness
4. Task 3 (default_checks wiring) — wires the check into batch runs
5. Task 7 (cargo test green) — verify
6. Task 6 (backtest) — tune noise before shipping
7. Task 5 (false-positive docs) — written after backtest numbers are known

---

## Open Questions

- **Should `_test.go` files be excluded?** `go-encapsulate-collection` excludes them
  because a fake's getter exists specifically to hand back test-seeded state. The same
  reasoning applies here (test setup routinely writes fields from outside the package
  under test). Recommend adding this exclusion in Task 1 — confirm in backtest.
- **Per-write-site vs. per-field deduplication?** Current plan: one finding per write
  site (edge). An alternative is one finding per distinct `(caller_pkg, field)` pair
  (deduplicated across all call sites). The per-site approach is consistent with every
  other checker and makes the finding clickable from the editor; prefer it unless
  backtest shows unacceptable volume.
- **Severity: Advisory or Blocking?** Advisory, same as `go-encapsulate-collection`.
  Blocking would be too disruptive for existing codebases with large DTO surfaces.

---

## Adversarial Review

*Potential weaknesses and counter-arguments:*

**W1 — DTO suffix list is fragile**: Any type not ending in a known suffix but still
intentionally public-mutable will generate noise. Mitigation: the list covers the most
common Go conventions; callers can add `kibitzer:ignore` for one-offs; the standard
`disabled`/`scope` machinery silences entire packages.

**W2 — `field_access_supports` is Go-only**: The check silently returns nothing on
non-Go repos. This is consistent with `go-encapsulate-collection` and other Go-only
checks; the `whole_repo_check` name prefix `go-` communicates this.

**W3 — Write detection is syntactic only**: `&recv.Field` passed to a mutating callee
is missed. This is a documented ceiling on the underlying infrastructure, not a new
limitation of this checker. Advisory severity appropriately reflects this uncertainty.

**W4 — Cross-package writes in generated code**: protobuf-generated structs, mock
builders, etc. The `looks_generated()` check on the write-site file would suppress
these. Consider filtering write sites where the caller's file looks generated.

**W5 — `model.field_accesses` is populated only for methods, not functions**: Check
`symbol_extract.rs:1357–1358` — `extract_field_access_sites_for_file` only walks
method bodies (not bare functions). A bare function in package B writing to a package
A field would be missed. This is a known gap in the extraction layer, not something
this checker can fix alone.
