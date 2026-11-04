# Research: Encapsulate Variable Check

## 1. Infrastructure readiness

### 1.1 `FieldAccessEdge` and `AccessKind` (issue #39)

The write-site tracking this feature depends on is already live:

- **`src/arch_model.rs:184–214`** — `AccessKind { Read, Write }` enum and `FieldAccessEdge { from, to, access, file, line }` struct.
- **`src/symbol_extract.rs:1160`** — `field_access_supports(language)` returns `true` only for `Language::Go` (v1 ceiling documented inline).
- **`src/arch_model.rs:850–876`** — `resolve_field_access_edges` populates `ArchModel::field_accesses`.
- **Write-detection scope**: syntactic only — `=`/`:=` left-hand position, `++`/`--` operand. `&recv.Field` passed to a mutating callee is counted as `Read` (documented ceiling).

The `ArchModel::field_accesses` vec is populated and available; no new extraction work is needed.

### 1.2 Field id format

`FieldAccessEdge.to` is `"{package_path}::{Type}.{field}"` (arch_model.rs:868–870).  
`FieldAccessEdge.from` is a `SymbolNode::id`: `"{package_path}::{parent}.{name}"` (methods) or `"{package_path}::{name}"` (functions).  

Package extraction: split on `"::"`, take the left side — same as `caller_package()` (arch_model.rs:567–568, private fn).

### 1.3 Exported field detection

Go exported fields have an uppercase first letter. The field name is the last segment after the final `.` in `FieldAccessEdge.to`. `str::chars().next().map(|c| c.is_uppercase())` is the existing pattern used by `go_is_exported` in `symbol_extract.rs:75–80`.

### 1.4 `ArchModelChecker` trait

Defined at `architecture_checks.rs:604–618`:
```rust
pub trait ArchModelChecker {
    fn name(&self) -> &str;
    fn check(&self, model: &ArchModel, config: &ArchitectureConfig) -> Vec<ArchFinding>;
    fn needs_private_symbols(&self) -> bool { false }
}
```
Registered in `model_registry()` (architecture_checks.rs:620–632), looked up by `lookup_model(name)` (line 634).

### 1.5 Wiring into `default_checks()`

`whole_repo_check(name, architecture_checker)` in `config.rs:736–741` creates a batch-only `Check` with `architecture_checker` set. The existing pattern:

```rust
whole_repo_check("go-package-size", "package-size"),
whole_repo_check("parameter-object", "parameter-object"),
```

New entry will be: `whole_repo_check("go-encapsulate-variable", "encapsulate-variable")`.

The `name` field (first arg) is the check's display/lookup name in reports; the second arg is the `ArchModelChecker::name()` returned by the checker struct.

## 2. Analogous implementations

### 2.1 `god_class.rs` — closest analogue

A Go-only `ArchModelChecker` that iterates packages/symbols, reads source files directly for WMC, and uses `ArchModel::field_accesses` (via ATFD). Same "re-read file for data not in `ArchModel`" pattern. Its registration in `model_registry()` and `default_checks()` is the template to follow.

### 2.2 `go_encapsulate_collection.rs` — per-file sibling

A per-file `Checker` (not `ArchModelChecker`) for a related Fowler refactoring. Demonstrates the naming convention `go-encapsulate-*` and the existing false-positive doc pattern (`docs/go-encapsulate-collection-false-positives.md`).

### 2.3 `unreferenced_symbols.rs` — cross-package reasoning

Iterates `model.packages` and `model.call_edges` to reason about cross-package call patterns. Provides a template for "is this id in package X, accessed from package Y?" logic.

## 3. DTO/config struct allowlist design

The requirement is to avoid flagging types that are "conventionally public-and-mutable by design." Two options:

**Option A — suffix allowlist in checker code (default)**:
Hard-code a default set of well-known DTO suffixes: `Config`, `Options`, `Opts`, `Params`, `Args`, `Request`, `Response`, `Settings`, `Result`. Extract the type name from `FieldAccessEdge.to` (segment between `::` and `.`), check against the suffix list. This aligns with `go-encapsulate-collection`'s "skip exported fields" carve-out (its checker skips when the field itself is exported, since the caller can already mutate directly — a structurally different carve-out but the same "this is intentionally public" judgement).

**Option B — `.kibitzer/inspect.json` scope config**:
Let repos disable the check entirely or restrict its scope via the standard `scope` field on the check entry. This is already supported by the `Check.scope` / `matches_scope` machinery.

**Chosen approach**: Option A for the default (compile-time suffix list in the checker), plus Option B naturally available through the existing `scope`/`disabled` machinery. No new config schema changes needed.

## 4. Finding message format

Existing `ArchFinding` message format for comparable checkers:
- `[import-cycles] cycle: a → b → a`
- `[instability] pkg is in zone of pain (I=0.1, A=0.1): ...`
- `[parameter-object] ...`

Proposed format:
```
[encapsulate-variable] {caller_package}.{CallerType}.{method} writes {field_pkg}.{FieldType}.{FieldName} from outside its package — consider adding a setter
```

The `ArchFinding.file` and `.line` point to the write site (from `FieldAccessEdge.file` and `.line`).

## 5. Noise risk assessment

**Main noise vectors**:
1. **DTO/config structs** — standard Go pattern where callers fill a `Config{}` literal or set fields before passing it. Mitigated by the suffix allowlist.
2. **Test setup** — test files assign fields to seed state. Some callers are `_test.go` files; excluding test files from the check would reduce noise. Worth investigating in backtest.
3. **Builder patterns** — a package intentionally exposes fields for callers to set (e.g. `http.Server`). Mitigated by: (a) suffix allowlist covers many; (b) callers can suppress with `kibitzer:ignore`.
4. **Generated code** — `looks_generated()` already exists in `arch_model.rs:300–305`; the checker can skip files matching it.

**Expected backtest signal**: The kubernetes/kubernetes codebase has many cross-package field writes in test and config code. Expect moderate noise before the allowlist is tuned. The backtest will establish the baseline.

## 6. Where to place the new file

By analogy with `god_class.rs` and `middle_man.rs` (top-level `src/` files, not under `src/checkers/`), the new file should be `src/encapsulate_variable.rs`, added to `src/main.rs`'s module list via `mod encapsulate_variable;`.

## 7. `kibitzer:ignore` integration

The inline-ignore post-pass (`inline_post_pass.rs`) already applies `kibitzer:ignore <name>` directives to `ArchFinding` results. The check name `encapsulate-variable` is what a user would write in a `// kibitzer:ignore encapsulate-variable -- reason` comment. No extra work needed.
