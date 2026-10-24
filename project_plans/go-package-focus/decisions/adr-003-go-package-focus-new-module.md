# ADR-003: go-package-focus Implemented as a New Module (`src/go_package_focus.rs`)

**Status**: Accepted
**Date**: 2026-10-23

## Context

New `ArchModelChecker` implementations in this codebase can either:
1. Be added inline to `src/architecture_checks.rs` (existing pattern for small checkers).
2. Live in their own module file and be registered via `model_registry()` (existing pattern
   for larger, self-contained checkers: `src/extract_class.rs`, `src/god_class.rs`,
   `src/isp_fat_interface.rs`).

`go-package-focus` requires intra-package symbol clustering, coverage diagnostics, edge-case
handling for generics/init/generated code, and multi-scenario testdata. That is substantially
more code than any checker currently inline in `architecture_checks.rs`.

## Decision

Implement `GoPackageFocusChecker` in a new file `src/go_package_focus.rs`, registered via
`model_registry()` in `src/architecture_checks.rs`, and declared as `mod go_package_focus`
in `src/main.rs`. This matches the precedent set by `extract_class`, `god_class`, and
`isp_fat_interface`.

## Alternatives Rejected

**Inline in `src/architecture_checks.rs`**: `architecture_checks.rs` is already 730+ lines.
Adding another 300–500 lines of clustering logic, coverage diagnostics, and edge-case
handling would make the file harder to navigate. The module boundary is the natural
cohesion boundary — same reason `extract_class.rs` was separated.

**New crate**: overkill for a single checker; creates cross-crate dependency management
overhead with no compensating benefit at this scale.

## Consequences

- `src/main.rs` gains one `mod go_package_focus;` declaration.
- `src/architecture_checks.rs::model_registry()` gains one `Box::new(crate::go_package_focus::GoPackageFocusChecker)`.
- All go-package-focus logic, tests, and constants live in `src/go_package_focus.rs`.
