# Refactor engine (read-only milestone)

Mechanical refactors for Go, planned but never applied. Three MCP tools, none of which write
to the tree:

- `list_refactor_operations {path, package}` returns `MoveFiles`/`ExtractPackage` entries, or `[]`.
- `simulate_refactor {path, operation}` returns a `FeasibilityReport`.
- `plan_refactor {path, operation}` returns a `RefactorPlan`, or an error if infeasible.

`go-package-focus` reports carry `available_operations` (omitted from JSON when empty).

## Feasibility

The post-move package graph is built in memory. Edges follow files; resolved call edges
decide which new edges a partial move creates. Blocking: import cycle, `internal/`
violation, cgo (`import "C"`), a file outside the source package, source equal to target, a
moved symbol that collides with a target symbol, a target-package file that imports the
source, and an importer that calls both moved and remaining symbols. Warning only: build-tagged and generated files
(the model ignores build constraints, so verify on every target platform).

## Plan

Edits are ordered: package-clause rewrites (`pkg_test` becomes `<target>_test`), import-path
swaps in importers (aliased with the old package name when the name changes so `pkg.Sym`
stays valid), then file moves. Every touched file carries its SHA-256 for a later stale-plan
check. A partial move whose moved and remaining symbols call each other is refused, because
it needs qualifier rewriting.

## Known limits

Type-only references and calls the model leaves unresolved (for example a name that exists in
two packages) are invisible, so a partial move can miss an importer;
the verification commands (`go build ./...`, `go vet ./...`) are the backstop once an apply
path exists. **There is no apply, rollback or WAL yet**, and no `apply_refactor` tool.

## Backtest

`scripts/refactor-backtest.sh` (not part of `cargo test`) drives `kibitzer mcp` over stdio on
the top 20 Go packages by symbol count and moves each package's first non-test file to a
sibling package, dry-run only.

| Repo | Result |
|---|---|
| `anchore/syft` (Go) | 20 listed; 17 infeasible, 3 feasible; 2 plans produced, 1 feasible move refused as entangled; zero panics |
| `BurntSushi/ripgrep` | no Go packages, nothing to plan; zero panics |
| `kubernetes/kubernetes` | not run: not cloned on the machine that ran this |
