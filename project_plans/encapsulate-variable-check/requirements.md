# Requirements: Encapsulate Variable Check

**Item ID**: 49714d15-be83-470b-b0e7-10ce306d12b2  
**Project name**: `encapsulate-variable-check`  
**Source**: Backlog item — "Native check: Encapsulate Variable (field written from outside its own package)"

---

## Problem Statement

A public/exported mutable field written directly from outside the module that owns it
bypasses any invariant that module might want to enforce later. This is Fowler's
**Encapsulate Variable** (née Self-Encapsulate Field) refactoring. Detecting "written
from outside" (not just "declared as exported") requires field-write-site tracking,
which issue #39's `FieldAccessEdge`/`AccessKind::Write` infrastructure already provides
in the live codebase.

The gap: no checker currently consumes those write-edges to surface cross-package field
mutations.

---

## Scope

**In scope (v1)**:
- Go only — `FieldAccessEdge` / `AccessKind` extraction is Go-only (`field_access_supports` in `symbol_extract.rs`).
- Whole-repo `ArchModelChecker` (not a per-file checker) — cross-package attribution requires the full `ArchModel`.
- Exported fields only (uppercase first letter in Go) written from a different package than the declaring package.
- Advisory severity — same as other structural-smell checkers (`go-encapsulate-collection`, LCOM, etc.).
- Allowlist for conventionally-public DTO/config types: suppress findings on types whose name ends in a configurable suffix list (default: `Config`, `Options`, `Opts`, `Params`, `Args`, `Request`, `Response`, `Settings`, `Result`).

**Out of scope (v1)**:
- Languages other than Go.
- Detecting writes through pointer aliasing (`&recv.Field` passed to a mutating callee).
- Generating a setter stub as a refactor operation (follow-up: could use `AvailableOperation` once a "introduce setter" refactor tool exists).
- Detecting read-only access from outside (not a violation of encapsulation).

---

## Acceptance Criteria

1. **AC-1 — Cross-package write flagged**: A `FieldAccessEdge` with `access == AccessKind::Write` whose caller package (prefix before `::` in `from`) differs from the field's declaring package (prefix before `::` in `to`) and whose field name is uppercase-first produces an `ArchFinding` pointing to the write site (`file`, `line`).

2. **AC-2 — Same-package write not flagged**: A write to an exported field from within the same package as the declaring type produces no finding.

3. **AC-3 — Unexported field write not flagged**: A write to a field whose name starts with a lowercase letter produces no finding, even if it crosses packages.

4. **AC-4 — DTO allowlist suppresses finding**: Types whose name ends in a suffix from the default allowlist (`Config`, `Options`, `Opts`, `Params`, `Args`, `Request`, `Response`, `Settings`, `Result`) produce no finding, regardless of how their fields are written.

5. **AC-5 — Read-only access not flagged**: A `FieldAccessEdge` with `access == AccessKind::Read` from an outside package produces no finding.

6. **AC-6 — Registered in `model_registry()` and `default_checks()`**: `model_registry()` in `architecture_checks.rs` returns a `EncapsulateVariableChecker` instance; `default_checks()` in `config.rs` includes a `whole_repo_check("go-encapsulate-variable", "encapsulate-variable")` entry.

7. **AC-7 — Finding message format**: Each finding message is `[encapsulate-variable] {caller_pkg}.{type}.{method} writes {field_pkg}.{type}.{field} from outside its package; consider a setter`. The `file` and `line` fields on `ArchFinding` point to the exact write site.

8. **AC-8 — Backtest run and documented**: The checker has been run against the standard backtest corpus (`kubernetes/kubernetes`, `BurntSushi/ripgrep`, at minimum) with true-positive and false-positive counts recorded in `docs/go-encapsulate-variable-false-positives.md` or the PR body.

9. **AC-9 — `kibitzer:ignore encapsulate-variable` suppresses finding**: Inline `// kibitzer:ignore encapsulate-variable -- <reason>` at the write site silences the finding for that line.

10. **AC-10 — Unit tests cover all AC branches**: Tests for cross-package write, same-package write, unexported field, DTO-suffix type, read-only access, and the finding message format all pass with `cargo test`.

---

## Depends On

- Issue #39 (`FieldAccessEdge` / `AccessKind::Write` infrastructure) — already shipped in the live codebase (`src/arch_model.rs`, `src/symbol_extract.rs`). No blocking dependency.

---

## References

- `src/arch_model.rs`: `FieldAccessEdge`, `AccessKind`, `ArchModel::field_accesses`
- `src/architecture_checks.rs`: `ArchModelChecker` trait, `model_registry()`, `ArchFinding`
- `src/config.rs`: `whole_repo_check()`, `default_checks()`, `go_exception_checks()`
- `src/god_class.rs`: reference implementation of a Go-only `ArchModelChecker`
- Fowler, *Refactoring* 2nd ed., "Encapsulate Variable" (p. 132)
