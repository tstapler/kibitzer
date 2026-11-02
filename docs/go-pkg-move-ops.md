# Go package move operations (`kibitzer refactor go`)

Atomic, verified, behavior-preserving Go package/file moves. A *plan* is computed
without touching disk, reviewed (or embedded with a feasibility-simulation result), then
*applied* all-or-nothing and verified with the Go toolchain. Design artifacts:
`project_plans/go-pkg-move-ops/`.

## Commands

```sh
kibitzer refactor go move-files    --src lib/util.go lib/util_test.go --dest-pkg example.com/m/util
kibitzer refactor go rename-pkg    --from example.com/m/lib --to example.com/m/core [--name core]
kibitzer refactor go extract-decls --from example.com/m/lib --decls Util,Widget --to example.com/m/util
kibitzer refactor go split-pkg     --pkg example.com/m/lib --partition clusters.json
kibitzer refactor go sync-policy   --plan move-plan.json
kibitzer refactor go apply         --plan move-plan.json --apply
```

Shared flags: `--repo <dir>` (module root, default `.`), `--out <file>`, `--simulation
<file>` (embed feasibility-simulation JSON), `--apply`, `--post-apply <shell cmd>`,
`--affected-only`. Without `--apply` nothing is written and the plan JSON is printed
(exit 2 if it has blockers). With `--apply`, exit 3 means verification failed and the
tree was rolled back; exit 4 means the rollback itself failed for some path (listed in
`restore_errors`) and the tree must be inspected.

`split-pkg` takes `{"clusters":[{"to":"import/path","files":["a.go","b_test.go"]}]}`
(`--focus-json` is an alias for `--partition`); the clusters are applied in order as
repeated move-files over one in-memory tree. The simulator (#153) that would produce
this partition does not exist yet, so the file format is defined here.

### Plan and apply model

* Paths in a plan are repo-relative; each step carries the new content and the SHA-256
  of the content it replaces. A plan is the exact bytes that will be written. Rewritten
  files are `gofmt`ed at plan time only when the original was already gofmt-clean, so a
  pre-existing formatting problem is not folded into the diff. Plan paths must stay inside
  the repo and `affected_packages` must be relative package paths; anything else is
  rejected before any write.
* **Idempotence.** Before writing, every step is classified against disk: all steps
  already at their target state → `already_applied` (no-op); some → error `plan is
  partially applied … nothing was modified`; any source changed since planning → error
  `stale plan … nothing was modified`.
* **Atomicity.** Every touched path (old and new) is snapshotted; each write goes to a
  temp file then `rename`. On a write/removal failure or a failed verification — a write, a removal, or verification — every
  snapshotted path is restored (the restore loop calls `create_dir_all` before each
  write), directories created by the apply are removed, and temp files are deleted.
* **Verification.** After writing: optional `--post-apply` hook (for regeneration), then
  `go build ./...`, `go vet ./...`, `go test -count=1 ./...` from the module root (or the
  plan's affected packages with `--affected-only`). Their combined stdout+stderr and exit
  codes are embedded verbatim in the result (`verify.build|vet|test`); any non-zero exit
  rolls the tree back (`status: rolled_back`).
* **Simulation gate.** `--simulation file.json` is embedded as `simulation_result`;
  `{"feasible": false}` or a non-empty `"blockers"` array becomes `simulation_blocker`
  diagnostics and the plan refuses to apply.
* **sync-policy** is a separate plan (`operation: sync_policy`) derived from another
  plan's `path_map`. It rewrites path-valued keys (`scope`, `paths`, `include`,
  `exclude`, `files`, `packages`, `dirs`, `roots`, `root`, `path`, `from`, `to`, `allow`,
  `deny`) in `.kibitzer/inspect.json`, and import paths in `.golangci.y*ml`,
  `.go-arch-lint.yml`, `.depguard.yml`. A policy plan built from a blocked move plan is
  itself blocked. It is never folded into the move plan, so a reviewer can accept one
  without the other.

Limits of the guarantee: there is no signal handling (a kill during `go test` leaves the
written, unverified tree, and the next `apply` of the plan reports it as partially
applied); file modes and symlinks are not preserved across the temp-file rename; and
verification has no baseline or timeout, so a pre-existing `go vet`/`go test` failure
rolls every move back.

## Why tree-sitter in Rust (and not `gopls`)

The item asked to check whether `gopls` offers a package-move command before choosing.
Checked with `gopls v0.24.0` (`golang.org/x/tools/gopls`, installed via Homebrew) on
2026-11-02:

* `gopls help` lists: `serve, version, help, api-json, licenses, call_hierarchy, check,
  codeaction, codelens, definition, execute, folding_ranges, format, mcp, highlight,
  implementation, imports, remote, links, prepare_rename, references, rename, semtok,
  signature, stats, symbols, workspace_symbol`. **No move/mv/extract-to-package command.**
* `gopls api-json` (code-action kinds): `refactor.extract.{constant,function,method,
  toNewFile,variable}`, `refactor.inline.{call,variable}`, `refactor.rewrite.*`,
  `source.*`. `toNewFile` stays in the same package. The options `moveType` and
  `moveDeclaration` exist but their own docs say "The implementation is unfinished so we
  use this setting to gate its use" — disabled by default, not a basis for automation.
* `gopls codeaction` on `Util` in the `_inputs/base` fixture lists "Extract declarations
  to new file" (`refactor.extract.toNewFile`) and "Split package "lib""
  (`source.splitPackage`, listed only — not executed, behavior not verified here). No
  move-to-package action.
* Package rename exists, but is not atomic or complete from the CLI. In a copy of
  `_inputs/base`: `gopls prepare_rename lib/lib.go:1:9` → `can't rename package: LSP
  client does not support file renaming`; `gopls rename -w lib/lib.go:1:9 core` →
  `open …/core/lib.go: no such file or directory`, after which the importers already
  said `example.com/m/core` while the directory was still `lib/`, and `go build ./...`
  failed with `no required module provides package example.com/m/core`. That is exactly
  the partial-edit state this feature exists to prevent. (An LSP client that implements
  file renames might do better; `rename` also cannot move files or declarations between
  packages at all.)

So the move/extract/split operations would have to be built regardless, and the one
operation `gopls` has is driven through an LSP client with no atomicity or verification
story. **Chosen approach: tree-sitter-go (already a kibitzer dependency) in Rust**, as an
in-memory overlay (`src/refactor/workspace.rs`) with byte-range edits
(`src/refactor/go_syntax.rs`) so untouched bytes are preserved exactly, then `gofmt`
and the Go toolchain for verification. Advantages: no Go helper binary or LSP session,
planning is read-only and fast (a rename plan over `kubernetes/kubernetes`, 17,846 Go
files, takes ~0.3 s), and plans compose (split = repeated move over one overlay).

**Cost of the choice.** Rewrites are syntactic, not type-resolved, which the item's
implementation notes asked to avoid. What replaces resolution:

* Qualifier rewrites touch only `pkg.Member` selectors/qualified types whose `pkg` is the
  file's actual import name for that package and whose member is one of the moved
  declarations. If the same identifier also appears anywhere else in the file (a local
  variable or parameter shadowing the package name) the file gets a
  `shadowed_package_name` blocker instead of a guess.
* Unqualified references across the new package boundary are matched by name against the
  package's top-level declarations; composite-literal key positions are
  `ambiguous_reference` blockers; unexported names crossing the boundary are
  `unexported_dependency` blockers (v1 never exports).
* A local variable that shadows a *moved top-level name* is not detected; it can produce
  a rewrite that fails to compile. That case is caught by verification and rolled back,
  never left on disk.

## Handled-or-diagnosed matrix

| Situation | rename-pkg | move-files / extract-decls / split-pkg | Where |
|---|---|---|---|
| Build constraints (`//go:build`, `// +build` — found by comment position, so a license block above is fine — and GOOS/GOARCH filename suffixes) | Handled — files move intact with their tags | **Blocked** `build_constraint` — tag variants other than the host's cannot be verified | `file_hazards` |
| cgo (`import "C"`) | **Blocked** `cgo` — preamble and sibling `.c/.h` files would be left behind | **Blocked** `cgo` | `file_hazards` |
| `.s/.c/.h/.cc/.cpp/.syso` files in the source dir | **Blocked** `native_sources` | **Blocked** `native_sources` | `dir_hazards` |
| `//go:embed` | **Blocked** `go_embed` | **Blocked** `go_embed` | `file_hazards` |
| Generated file (`// Code generated … DO NOT EDIT.`) in the moved set | Handled with warning — moves with its package; regenerate via `--post-apply` | **Blocked** `generated_file` | `file_hazards` |
| Generated *importer* that needs a path rewrite | Rewritten in place + warning | Rewritten in place + warning | importer loop |
| `init()` in moved code (registration) | Handled — `init` moves with its package, blank imports are rewritten | **Blocked** `init_registration` — importers that only side-effect-import the old package would stop registering | `file_hazards` |
| Dot import of the source package | Path rewritten | **Blocked** `dot_import` | importer loop |
| Import-path strings in reflection/plugins/config the tool cannot see | Not detected (no resolved references); `go test` is the backstop | Not detected | — |
| Nested packages under the source dir | **Blocked** `has_subpackages` | n/a | `rename_pkg` |
| `testdata/` or other non-Go files (not `*.md`/`OWNERS`/dotfiles) in the source dir | **Blocked** `non_go_assets` — they are not moved | Warning | `dir_hazards` |
| Moving into a package with the same name as the source | n/a | Handled — importers import the destination under a distinct alias (`m/sub/lib` → `sublib`) | importer loop |
| Nested Go modules, `vendor/`, `testdata/`, `_*`/`.*` dirs | Skipped, never edited | Skipped | `Workspace::walk` |
| Cross-module move (destination outside `module`) | **Blocked** `outside_module` | **Blocked** `outside_module` | |
| Method and receiver type split across packages | n/a | **Blocked** `method_split` (extract moves methods with their type) | |
| Unexported helper shared by moved and remaining code | n/a | **Blocked** `shared_helper` (helpers used only by the moved set move with it) | `close_over_helpers` |

## Pre-flight diagnostics

| Code | Meaning |
|---|---|
| `internal_visibility` | An importer (including the source package, and moved code's own imports) falls outside the parent of the last `internal/` element of the target path |
| `cycle_detected` | After the move, the source or destination package reaches itself through module-local imports (external `_test` packages are excluded, as in Go). Pre-existing cycles are reported too |
| `symbol_conflict` / `symbol_conflict_external_test` | A moved top-level name already exists in the destination package, or in its `_test.go` files (`package dest_test` included) |
| `import_collision` / `import_name_collision` | An importer already imports the new path, or the new import name collides with another import or top-level name |
| `dest_exists`, `dest_file_exists`, `same_package`, `main_package`, `mixed_packages`, `no_such_package`, `no_such_file`, `no_such_decl`, `nothing_to_move`, `overlapping_clusters`, `multiple_source_packages` | Selection/destination errors |
| `simulation_blocker` | The embedded feasibility-simulation result reports a blocker |
| `format_failed` | A rewritten file does not parse (`gofmt` rejected it) |

Exported-surface changes are listed in `surface_changes` (`old.Path.Name -> new.Path.Name`).

## Known limits of the rewrite

* Third-party imports are never pruned (their package name cannot be guessed from the
  path), so an extracted file keeps any third-party import it no longer uses and fails
  verification instead of being cleaned; standard-library and module-local imports are
  pruned. Dot and blank imports stay with the source file and are not copied to the
  destination.
* A split-out destination file starts with only a `package` clause; the source file's
  license header is not copied.
* A trailing comment on the line of the previous declaration is not treated as the next
  declaration's doc comment.

## Out of v1

Semantic refactorings, signature changes, interface extraction, cross-module moves,
exporting an unexported identifier, moving non-Go files with a package, moving nested
packages, and updating import-path strings outside Go source and the listed policy files.

## Testing

* `testdata/refactor/<op>/<case>/{op.json, plan.json, after.txt}` with shared before-trees
  in `testdata/refactor/_inputs/`. Each operation has `clean`, `test-package`,
  `internal-boundary`, `cycle-rejection`, and `simulator-negative` fixtures; move-files
  and extract-decls also have `symbol-conflict-external-test`; move-files has four
  `hazard-*` fixtures; sync-policy has `path-invalidation`. `rename-pkg/cycle-rejection`
  uses an input that already contains an import cycle (a pure rename cannot create one).
  `sync-policy` fixtures plan their `source` operation first and inherit its blockers.
* Non-blocked fixtures are applied with real `go build/vet/test` verification and then
  applied a second time (must be `already_applied`).
* `src/refactor/golden_tests.rs` also covers failure injection after every step
  (checksums of every file before/after), a real mid-apply write failure, policy-update
  failure, stale and partially applied plans, verbatim verification output and rollback,
  a failing post-apply hook, and the simulation gate.
* Regenerate goldens with `KIBITZER_BLESS=1 cargo test --bin kibitzer refactor::golden`
  and review the diff.

## Backtest (six standard repos)

Procedure: `scripts/clone-backtest-repos.sh` clones (HEAD below); for the Go repo a
throwaway `git worktree`, with `GOFLAGS=-mod=vendor GOTOOLCHAIN=go1.27.0` (the module
requires Go 1.27.0; the system Go is 1.26.4). Each repo was run with
`kibitzer refactor go rename-pkg --repo <repo> …`.

| Repo (HEAD) | Go module? | Simulated move | Before | After |
|---|---|---|---|---|
| `kubernetes/kubernetes` (`0ba07e08`) | yes | `rename-pkg` `pkg/util/slice` → `pkg/util/sliceutil` (6 importers + 2 moved files, 8 steps) | `go build` of the 6 affected packages: exit 0 | `go build`, `go vet`, `go test -count=1` of the 6 packages: exit 0, 0, 0 (`persistentvolume` tests ok in 22.4 s); diff 14 insertions / 240 deletions (the deletions are the moved files); second apply: `already_applied` |
| `kubernetes/kubernetes` | yes | `extract-decls` `CopyStrings,SortStrings` from `pkg/util/slice` → new `pkg/util/stringsutil` | `go build ./pkg/util/slice`: exit 0 | build/vet/test of `slice` + `stringsutil`: exit 0, 0, 0 |
| `apache/cassandra` (`c103032`) | no — probe | `move-files` `lib/util.go`+`util_test.go` → new `util` pkg in a synthetic probe module (see below), 8,725 files in repo | `go build ./zz_kibitzer_probe/...`: exit 0 | `applied`; build/vet/test of affected pkgs: exit 0, 0, 0; plan 0.03 s |
| `servo/servo` (`7821ec2dd`) | no — probe | same probe move, 195,563 files in repo | exit 0 | `applied`; 0, 0, 0; plan 0.20 s |
| `BurntSushi/ripgrep` (`3fce3b5`) | no — probe | same, 237 files | exit 0 | `applied`; 0, 0, 0; plan 0.02 s |
| `denoland/deno` (`60e01b1d`) | no — probe | same, 14,830 files | exit 0 | `applied`; 0, 0, 0; plan 0.05 s |
| `microsoft/vscode` (`7a332eb7`) | no — probe | same, 18,647 files | exit 0 | `applied`; 0, 0, 0; plan 0.06 s |

**Honest limit:** only `kubernetes/kubernetes` is a Go module, so it is the only row that
moves the repo's own code. `cassandra`, `servo`, `ripgrep`, `deno` and `vscode` have no
`go.mod` (run against them as-is, the tool refuses: `Error: no go.mod at <repo> (a Go
module root is required)`, exit 1). For those five the "probe" rows add a throwaway
`git worktree` of the repo at the listed HEAD, a `go.mod` (`module example.com/probe`) at
its root, and the `_inputs/base` fixture (`lib/`, `app/` with tests) copied to
`zz_kibitzer_probe/`; the move is `move-files` of `lib/util.go` + `lib/util_test.go`, applied
with `--affected-only` verification (`GOWORK=off GOFLAGS=-mod=mod`). That shows the tool
loads, plans, and applies correctly when embedded in a very large non-Go tree (the
`go build` outcomes are those of the probe packages, not of the host project's Java/Rust/TS
code), and the worktrees were removed afterwards. It is not evidence about those repos'
own code.

Scale/noise run on `kubernetes/kubernetes` (plan only, nothing applied): 60 package
directories under `pkg/` sampled with `random.seed(7)`, each planned as `rename-pkg` to
`<pkg>x`: **37 clean, 23 blocked, 0 errors (~0.4 s each)**. Blockers: `has_subpackages` ×18,
`shadowed_package_name` ×4, `non_go_assets` ×1 (counts re-run after the `non_go_assets`
check was added; before it, 38 clean / 22 blocked). The four shadow blockers are the
conservative-by-design case described above; they were not individually triaged for
being true or false positives.
