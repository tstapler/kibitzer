# Operation catalog v1 (Go) — requirements

Backlog item `853746a0-b6ad-445f-85cd-d372f87b9471`; issue https://github.com/tstapler/kibitzer/issues/155 (umbrella #153).

The detailed requirements, research, plan and validation for this item already exist under
[`project_plans/go-pkg-move-ops/`](../go-pkg-move-ops/requirements.md) (`research/research.md`,
`implementation/plan.md`, `decisions/validation.md`). This file restates the item's scope and
criteria under the name the pipeline expects; the `go-pkg-move-ops` directory is authoritative.

## Goal

Atomic, verified, behavior-preserving Go package/file moves and import rewrites: each operation
produces a plan, applies it atomically, and verifies the result.

## Operations (v1)

1. `move-files` — move files to a new/existing package; fix `package` clauses, importers, `_test` packages, `internal/` visibility.
2. `rename-pkg` — repository-wide rename / import-path change.
3. `extract-decls` — move declarations plus transitive private helpers (acyclic only); report exported-surface changes.
4. `split-pkg` — split a package by file clusters from a simulator partition.
5. `sync-policy` — update dependency guards / `.kibitzer/inspect.json` scopes as a separate, reviewable plan step.

Out of scope: semantic refactorings, signature changes, interface extraction, cross-module moves.

## Acceptance criteria

1. Golden-file tests per operation: clean success, `_test` package, `internal/` crossing, cycle rejection, simulator-negative.
2. Applying the same plan twice is a no-op or a clear error; no partial mutation.
3. Induced failure at each apply stage leaves the tree byte-identical (checksums); restore calls `create_dir_all` before every write.
4. Plan result embeds verbatim `go build ./...`, `go vet ./...`, `go test -count=1 ./...` output; non-zero exit means no success.
5. Build tags, cgo, generated code, `init()` registration: handled or blocking diagnostic; matrix in `docs/go-pkg-move-ops.md`.
6. `docs/go-pkg-move-ops.md` records the `gopls` inspection result and the chosen approach (tree-sitter in Rust) with rationale.
7. `sync-policy` is a separate plan step with a path-invalidation golden test.
8. Backtest against kubernetes, cassandra, servo, ripgrep, deno, vscode: one simulated move each, before/after `go build ./...` recorded in the doc.
9. Pre-flight symbol-conflict check against the destination's external `_test` packages, with a golden fixture.
