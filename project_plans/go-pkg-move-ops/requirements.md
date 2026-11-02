# Go Package Move Operations Catalog v1

## Item ID

853746a0-b6ad-445f-85cd-d372f87b9471

## Summary

Once a package split or file move has been decided (by a human, an agent, or an
advisory check such as `go-package-focus` / `extract-class`), carrying it out is
mechanical but error-prone: move files, fix the `package` clause, rewrite import
paths in every importer (including external `_test` packages), update any
dependency-guard configuration, and regenerate build metadata. Kibitzer should
provide a small, well-tested set of **behavior-preserving operations** that
produce a plan, apply it atomically, and verify the result. This is the execution
layer of the mechanical-refactor umbrella (umbrella: #153).

## Target Users

- **Primary**: an AI coding agent (Claude Code or similar) told to apply a
  package-split or file-move decision produced by `go-package-focus` or a human
  reviewer — the agent issues a single `kibitzer refactor go move-files ...`
  command instead of driving `sed` / manual imports rewrites.
- **Secondary**: a Go developer running the same command interactively to
  carry out a decomposition decision and get an atomic, reviewable diff.

## Operation Catalog (v1 scope)

1. **Move files to a package** (`kibitzer refactor go move-files`): updates
   `package` clauses, rewrites importers' import paths, fixes qualified
   references, handles `_test` packages and `internal/` visibility.
2. **Rename package / change import path** (`kibitzer refactor go rename-pkg`):
   repository-wide rewrite including test packages and generated code
   references.
3. **Extract declarations to a new package** (`kibitzer refactor go extract-decls`,
   acyclic cases only): moves a set of declarations plus their transitive private
   helpers, exporting what is genuinely required and reporting each
   exported-surface change in the plan.
4. **Split a package by file clusters** (`kibitzer refactor go split-pkg`): a
   composition of operations 1 and 3 driven by a partition (e.g. from
   `go-package-focus` findings).
5. **Update repository-declared policy** (`kibitzer refactor go sync-policy`):
   updates `.kibitzer/inspect.json` scopes and layering/dependency-guard paths
   when a move invalidates them — as an explicit, reviewable plan step, not a
   silent edit.

## Out of v1 Scope

- Semantic refactorings (signature changes, interface extraction, method moves)
- Cross-module moves (separate `go.mod`)
- Non-Go languages

## Problem Statement

Agents carry out package moves by hand with `sed` + regex over import paths.
This is where silent breakage comes from: identically-named identifiers in other
packages get rewritten, `_test` packages are missed, `internal/` rules are
violated silently, and `.kibitzer/inspect.json` policy is left pointing at
stale paths. Kibitzer should close this gap with a principled, verified
execution layer: plan → apply atomically → verify.

## Constraints

- Rewrites must be driven by **resolved references**, not text matching; the same
  identifier name in a different package must never be touched.
- Formatting (`gofmt` / `goimports`) is part of the apply step so diffs are
  clean and stable.
- An induced failure mid-apply must leave the working tree byte-identical to
  its pre-apply state (all-or-nothing).
- A post-apply hook point lets a repository run its own regeneration step
  (build-file generator, proto codegen, etc.) as part of verification.
- Whether `gopls` offers suitable programmatic commands for package-level moves
  **must be verified** before choosing between it, `go/packages`-based rewriting,
  or a pure in-Rust implementation over the existing tree-sitter data.
- This operation depends on the feasibility/what-if issue (#138 / #52 simulation
  layer); an operation's plan must embed the simulation result and refuse to
  apply when it reports a blocker.

## Acceptance Criteria

### AC-1 — Golden-file tests
Each operation has golden-file tests (before tree, plan, after tree) covering:
- A clean success case with no naming conflicts
- A `_test` package in the moved set
- An `internal/` visibility boundary crossing
- A case with a cycle that must be rejected
- Negative cases matched to simulator fixture outputs

### AC-2 — Idempotence
Applying the same plan twice is either a no-op (if the tree already matches
the plan's target) or a clear, actionable error message with no partial mutation.

### AC-3 — Atomicity
Inducing a failure at each stage of `apply` (file write, import rewrite, policy
update) leaves the working tree byte-identical to its pre-apply state, verified
by a checksum of all modified paths before and after the forced failure.

### AC-4 — Verification step
The plan's result embeds the verbatim output of:
- `go build ./...` across all affected packages
- `go vet ./...` across all affected packages
- `go test ./...` for every touched package

The apply step refuses to declare success if any of these commands exit non-zero,
and the failure output is preserved in the plan result for diagnosis.

### AC-5 — Documented limits
Build tags, cgo files, generated code (identified by the `// Code generated`
marker), and `init()`-based dynamic registration patterns either:
- Are handled correctly (with a test proving it), or
- Produce a clear blocking diagnostic that prevents a partial edit.
A "handled or diagnosed" matrix is documented in `docs/go-pkg-move-ops.md`.

### AC-6 — Toolchain choice documented
The implementation notes in `docs/go-pkg-move-ops.md` record:
- Whether `gopls` workspace-level rename/move was evaluated and why it was
  accepted or rejected (cite the specific API surface checked, the version
  tested, and the test result)
- The chosen approach (gopls, `go/packages`-based rewriting, or tree-sitter
  in Rust) with rationale

### AC-7 — Policy sync
`sync-policy` updates `inspect.json` scopes and guard paths in a separate,
reviewable plan step distinct from the file-move plan, so a reviewer can
see exactly which policy paths change and why.

### AC-8 — Backtest
The new subcommand is backtested against the standard real-world repo corpus
(`docs/backtest-repos.md`: kubernetes, cassandra, servo, ripgrep, deno, vscode)
using at least one simulated move per repo, with before/after `go build ./...`
outcomes recorded.

## Dependencies

- `go-package-focus` check (#138) — produces the partition that `split-pkg` consumes
- Architecture model / import graph (`src/import_graph.rs`, `src/arch_model.rs`) —
  drives the reference resolution that replaces text-matching
- Feasibility simulator (#153 umbrella) — plan must embed simulation result before
  apply is allowed
- Tree-sitter Go grammar (`tree-sitter-go`) — already vendored in `Cargo.toml`

## Related Issues

- #138 (`go-package-focus`) — advisory detector feeding this execution layer
- #52 (`Feature Envy: Move Function/Field`) — a semantic-level sibling
- #153 (mechanical-refactor umbrella) — this is one of its operations
- Architecture export PR (#29) — "one model, many views" pattern reused here
