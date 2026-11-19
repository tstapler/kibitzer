# Rust check parity

<!-- Generated from src/rust_parity.rs. Do not edit by hand: run `KIBITZER_UPDATE_DOCS=1 cargo test rust_parity`. -->

Which of kibitzer's default checks cover Rust. Tracking issue: https://github.com/tstapler/kibitzer/issues/184.

## Shipped

Default checks whose scope includes `**/*.rs`.

| Check | Status | Ported from |
|---|---|---|
| `comment-quality-rust` | Shipped | `comment-quality-<lang>` family |
| `duplicate-code` | Shipped | language-agnostic |
| `duplicate-code-cross-file` | Shipped | language-agnostic |
| `file-complexity-rust` | Shipped | file-complexity (Go) |
| `inline-ignore` | Shipped | language-agnostic |
| `primitive-obsession-rust` | Shipped | primitive-obsession (Go) |
| `rust-error-context` | Shipped | go-error-context / java-error-context |
| `rust-file-size` | Shipped | `<lang>-file-size` family |
| `rust-ignored-error` | Shipped | go-ignored-error / java-ignored-error |
| `sentinel-return-rust` | Shipped | sentinel-return-* (Java/Kotlin/JS), reshaped for `?` |
| `split-loop-rust` | Shipped | split-loop-go |
| `syntax-rules-rust` | Shipped | `syntax-rules-<lang>` family |
| `test-no-assertion-rust` | Shipped | test-no-assertion (Go/Python/Kotlin) |

## Gaps

Checks for other languages with no Rust counterpart. N/A is intentional; Planned has a sound analogue that is not built yet and is tracked on the issue above.

| Check | Status | Rationale |
|---|---|---|
| `remove-subclass` | N/A | Rust has no implementation inheritance, so there are no `Extends` edges for this hierarchy check to analyse (docs/hierarchy-cleanup-checks.md). |
| `extract-superclass` | N/A | Rust has no implementation inheritance, so there are no `Extends` edges for this hierarchy check to analyse (docs/hierarchy-cleanup-checks.md). |
| `collapse-hierarchy` | N/A | Rust has no implementation inheritance, so there are no `Extends` edges for this hierarchy check to analyse (docs/hierarchy-cleanup-checks.md). |
| `push-down-method` | N/A | Rust has no implementation inheritance, so there are no `Extends` edges for this hierarchy check to analyse (docs/hierarchy-cleanup-checks.md). |
| `push-down-field` | N/A | Rust has no implementation inheritance, so there are no `Extends` edges for this hierarchy check to analyse (docs/hierarchy-cleanup-checks.md). |
| `pull-up-method` | N/A | Rust has no implementation inheritance, so there are no `Extends` edges for this hierarchy check to analyse (docs/hierarchy-cleanup-checks.md). |
| `pull-up-constructor-body` | N/A | Rust has no implementation inheritance, so there are no `Extends` edges for this hierarchy check to analyse (docs/hierarchy-cleanup-checks.md). |
| `pull-up-field` | N/A | Rust has no implementation inheritance, so there are no `Extends` edges for this hierarchy check to analyse (docs/hierarchy-cleanup-checks.md). |
| `go-blank-imports` | N/A | Go `_` imports run package `init` side effects; Rust has no import-time side effects (`use .. as _` only brings a trait into scope). |
| `go-type-switch-density` | N/A | Go `switch x.(type)` on an open interface; a Rust `match` over an enum is compiler-checked for exhaustiveness, which is the fix this check suggests. |
| `go-type-switch-clustering` | N/A | Same as `go-type-switch-density`: enum `match` is the idiomatic, exhaustive form. |
| `introduce-special-case` | N/A | Targets repeated Go `nil` checks; Rust encodes absence in `Option`, which is already the special-case type. |
| `go-encapsulate-collection` | N/A | Targets Go getters that hand out an internal slice or map; a Rust `&[T]`/`&Vec<T>` return is read-only and the borrow checker prevents the aliasing the check guards against. |
| `go-encapsulate-variable` | N/A | Targets exported mutable Go package variables; Rust global mutable state needs `static mut` (unsafe) or interior mutability, both visible at the declaration. |
| `java-swallowed-interrupt` | N/A | Targets swallowed `InterruptedException`; Rust threads have no interruption exception. |
| `go-bulk-fetch-linear-scan` | Planned | The pattern (fetch everything, then scan for one item) is language-independent; no Rust port yet. |
| `go-table-driven-test-candidate` | Planned | Near-identical `#[test]` functions are a `rstest`/loop candidate; no Rust port yet. |
| `java-lost-exception-cause` | Planned | Analogue is `map_err(|_| NewError)` dropping the source error; no Rust port yet. |
| `parameter-object` | Planned | Parameter data exists for Go and TypeScript only (docs/parameter-object-false-positives.md); Rust function parameters are not extracted into the architecture model yet. |
| `go-package-size` | Planned | Go package aggregate size; Rust has `rust-file-size` per file but no crate/module aggregate yet. |
