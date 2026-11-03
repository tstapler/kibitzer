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
  pre-existing formatting problem is not folded into the diff. A plan may only touch `.go` files
  (move operations) or the known policy files (`sync_policy`), must not leave the repo or
  cross a symlink, may not mix step kinds, and its `affected_packages` must be relative
  package paths; anything else is rejected before any write. Temp files are created
  exclusively and file permissions are carried over on moves and restored on rollback.
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
applied); verification has no baseline, so a pre-existing `go vet`/`go test` failure
rolls every move back.

## Trust model

Planning (no `--apply`) only reads files and runs `gofmt`. **`--apply` runs `go build`,
`go vet` and `go test` in the target repo, which executes that repo's own test and `init`
code** (and `go` may fetch a toolchain named by `go.mod`), as does a `--post-apply`
command; the child inherits your environment, so pin `GOTOOLCHAIN`/`GOFLAGS` yourself if you
need to. Only apply to repositories you trust. Each command has a timeout (default 30 min,
`--verify-timeout-secs`; the whole process group is killed) and each output stream is capped
at 1 MiB with a note when truncated.

A plan carries a `plan_id`, the SHA-256 of its contents (everything but `repo_root`).
`apply` refuses a plan whose id does not match, whose steps lack `old_sha256` for an existing
file, whose paths leave the repo or cross a symlink, or (for `sync_policy`) whose content is
not exactly what the tool derives from the tree and the plan's path map. The id is a
**checksum, not a signature**: it catches edits and truncation, not a plan forged with a
recomputed id, so review plans you did not generate. A plan can still add or rewrite `.go`
files, and `go test` then runs them.

Writes go through the shared write-ahead-logged transaction (`src/refactor_core/txn.rs`): a
failure, a failed verification, or a panic restores every touched file; a killed process
leaves a log that the next `apply` rolls back before doing anything else. A rollback that
itself fails exits 4 (a clean rollback exits 3). Files written by a `--post-apply` hook or
by `go` itself are not rolled back. Verification output is embedded verbatim (capped) in the
result JSON, so treat saved results as you would test logs.

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
in-memory overlay (`src/go_refactor/workspace.rs`) with byte-range edits
(`src/go_syntax.rs`) so untouched bytes are preserved exactly, then `gofmt`
and the Go toolchain for verification. Advantages: no Go helper binary or LSP session,
planning is read-only and fast (a rename plan over `kubernetes/kubernetes`, 17,846 Go
files, takes ~0.3 s), and plans compose (split = repeated move over one overlay).

**Cost of the choice.** Rewrites are syntactic, not type-resolved, which the item's
implementation notes asked to avoid. Dynamic references are therefore handled by
*detect-and-block* for the shapes listed in the matrix (path/name string literals and the
path-keyed registries), and by `go build/vet/test` + rollback for anything else; a
resolved-reference engine (`go/packages` helper) is the follow-up that would close the
remaining gap. What replaces resolution:

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
| String literal spelling the moved package's import path (`plugin.Open`, registries, config keys) | **Blocked** `import_path_string` | n/a (path unchanged) | `string_references` |
| String literal `"pkg.Name"` / `"<import path>.Name"` naming a moved symbol | n/a | **Blocked** `import_path_string` | `string_references` |
| Registries/reflection keyed by package path or name in moved code (`gob.Register[Name]`, `proto.RegisterType/RegisterFile`, `plugin.Open/Lookup`, `runtime.Caller[s]/FuncForPC`, any `.PkgPath`) | **Blocked** `path_keyed_registration` | **Blocked** `path_keyed_registration` | `file_hazards` |
| Any other run-time name construction (`fmt.Sprintf("%s.%s", …)`, struct tags, build-time codegen reading paths) | Not detectable syntactically; `go test` is the backstop | Not detectable | — |
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

## One shared core, two planners (and a third engine outside it)

`kibitzer refactor go` (`src/go_refactor/`, five operations, Workspace-based planning) and the
MCP `plan_refactor`/`apply_refactor` engine (`src/refactor/`, #159, ArchModel-based
planning) plan differently but apply, verify and roll back through the same code:

| Concern | Single implementation |
|---|---|
| Go syntax (package clause, imports, build constraints, cgo, declarations) | `src/go_syntax.rs` (the former 198-line `refactor/go_syntax.rs` is deleted) |
| `go.mod` module line | `go_call_resolution::parse_module_path` (used by import graph, both engines) |
| SHA-256 | `refactor_core::sha256_hex` |
| Bounded command runner (no shell, timeout, process-group kill, output cap; `Locked` env for MCP, `Inherit` for the CLI) | `refactor_core/exec.rs` |
| Path/name guards (`is_inside`, `is_protected`, `is_go_ident`, `is_safe_import_path`) | `refactor_core/guard.rs` |
| Backup + WAL + temp-and-rename + crash `recover` + rollback-on-drop | `refactor_core/txn.rs` |

What remains separate on purpose: the plan schemas (`MovePlan` is content-based: each step
carries the final bytes; `RefactorPlan` is edit-based), the planners, and the MCP engine's
restriction to `go build ./...`/`go vet ./...` (the CLI also runs `go test`). Behavior change
for the MCP engine: it now flags files constrained only by their GOOS/GOARCH filename suffix
(`*_linux.go`) as build-tagged. Backtest on `kubernetes/kubernetes` (17,846 `.go` files,
`build_constraint_vs_legacy_on_corpus` in `src/go_syntax.rs`): 2,496 flagged by both, 221
flagged only by the new implementation (all suffix- or comment-aware cases), 0 only by the
old one.

The MCP split engine (`src/refactor_split.rs`, #158: `plan_split_refactor`, `apply_split_refactor`,
`verify_refactor`, `revert_refactor`) is **not** on the shared core: it has its own file writer,
restore and shell-based verification, and no crash recovery. Moving it onto `refactor_core::{txn,
exec}` is the natural next step. To avoid a name clash with the move engine's tools (both
originally used `plan_refactor`/`apply_refactor`), the split tools were renamed
`plan_split_refactor`/`apply_split_refactor`; `plan_refactor`/`apply_refactor` now mean the move
engine, so a client written against the old split-tool schema gets a schema error.

Transaction behavior worth knowing (all in `refactor_core/txn.rs`, covered by its tests):
the process-wide lock is taken first and held through staleness checks, body computation,
writes and verification, and any orphaned log is rolled back before anything is read. Backups
and the log are fsynced, and renames are followed by a directory sync. Crash recovery trusts
nothing on disk: every path in a log must resolve (symlinks followed) inside the repo and avoid
`.git`/`.kibitzer`, the temp name must be the one the module would pick, backups must be
regular files, and `.kibitzer` may not be a symlink; a log from the first MCP release
(`{original, backup, dest, temp}`) is still understood. A crash while changing a `.kibitzer`
file therefore needs a manual restore from `.kibitzer/plans/.wal-backup`; the error says so.
The lock is per process: two kibitzer processes on one repo are not serialized. The verification
runner no longer sets `GOWORK=off` (a `go.work` workspace is a legitimate layout), does not hang
on a descendant that keeps the output pipe open, and the child dies with its parent on Linux.

## Known limits

* `ops.rs::partial_move` is ~500 lines and covered by golden tests only.
* `verify` cannot stop a `--post-apply` hook or `go` from writing files outside the plan, and
  those are not rolled back. A `SIGINT` mid-verification leaves the log behind; the next
  `apply` in that repo restores the tree first.
* Not mitigated: the window between path validation and the write (an attacker with local
  write access to the repo during apply can swap a directory for a symlink).
* The workspace walk reads every `.go` file fully (no size cap, no depth limit).
* `--out` follows symlinks and uses the default file mode.
* A planned file that is a symlink (or any non-regular file) is refused by the transaction, where the
  first MCP release copied through it; the planner does not yet skip symlinks up front, so such a
  plan fails cleanly at apply, before any write.
* `add_import` returns the source unchanged when it cannot splice safely (a malformed import
  group); the CLI's always-on `go build` catches that, but a caller without verification would not
  notice.

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
* `src/go_refactor/golden_tests.rs` also covers failure injection after every step
  (checksums of every file before/after), a real mid-apply write failure, policy-update
  failure, stale and partially applied plans, verbatim verification output and rollback,
  a failing post-apply hook, and the simulation gate.
* Regenerate goldens with `KIBITZER_BLESS=1 cargo test --bin kibitzer go_refactor::golden`
  and review the diff.

## Backtest (six standard repos)

**Reproduce:** `scripts/clone-backtest-repos.sh && cargo build --release &&
scripts/go-move-backtest.sh all` (or `ONLY=k8s scripts/go-move-backtest.sh all`) runs every row
below in a throwaway `git worktree` per repo (the corpus checkouts are not modified) and prints
`go build`/`go vet` before and after, the tool's status and verify exit codes, and the diff size.
Kubernetes is a `go.work` workspace needing Go 1.27.0 (`GOTOOLCHAIN=go1.27.0`, set by the
script; the system Go here is 1.26.x) and `-mod=vendor`, so its before/after builds cover the
affected packages rather than `./...`. All rows were last re-run on the binary built from the
shared `refactor_core` (see "One shared core, two planners").

| Repo (HEAD) | Go module? | Simulated move | Before | After |
|---|---|---|---|---|
| `kubernetes/kubernetes` (`0ba07e08`) | yes | `rename-pkg` `pkg/util/slice` → `pkg/util/sliceutil` (6 importers + 2 moved files, 8 steps) | `go build` of the 6 affected packages: exit 0 | `go build`, `go vet`, `go test -count=1` of the 6 packages: exit 0, 0, 0 (`persistentvolume` tests ok in 22.4 s); diff 8 files, +16/−16 (rename-detected); second apply: `already_applied` |
| `kubernetes/kubernetes` | yes | `extract-decls` `CopyStrings,SortStrings` from `pkg/util/slice` → new `pkg/util/stringsutil` | `go build ./pkg/util/slice`: exit 0 | build/vet/test of `slice` + `stringsutil`: exit 0, 0, 0 |
| `apache/cassandra` (`c103032`) + `go-chi/chi/v5` | yes (real module at repo root) | `move-files` `middleware/sunset.go`+`sunset_test.go` → new package `zzmovedsunset` | `go build ./...`, `go vet ./...`: exit 0, 0 | `applied`; verify build/vet/test exit 0, 0, 0; after `go build`/`go vet ./...`: 0, 0; diff 3 files, +5/−4 |
| `servo/servo` (`7821ec2dd`) + `sirupsen/logrus` | yes | `rename-pkg` `hooks/test` → `hooks/testx` (7 steps) | 0, 0 | `applied`; 0, 0, 0; after: 0, 0; diff 7 files, +37/−37 |
| `BurntSushi/ripgrep` (`3fce3b5`) + `spf13/cobra` | yes | `rename-pkg` `doc` → `docx` (11 steps) | 0, 0 | `applied`; 0, 0, 0; after: 0, 0; diff 11 files, +16/−16 |
| `denoland/deno` (`60e01b1d`) + `urfave/cli/v3` | yes | `extract-decls` `FlagNames` → new `internal/flagnames` | 0, 0 | `applied`; 0, 0, 0; after: 0, 0; diff 3 files, +25/−19 |
| `microsoft/vscode` (`7a332eb7`) + `spf13/viper` | yes | `rename-pkg` `internal/encoding/dotenv` → `dotenvx` | `./...` **fails before the move** (see below); module packages: build 0, vet 0 | first attempt with `./...` verification: `rolled_back` (tree restored; correct per AC4). With `--affected-only`: `applied`; 0, 0, 0; after on module packages: build 0, vet 0, `go test` ok; diff 4 files, +5/−5 |

**How the five non-Go repos became Go modules:** none of them has a `go.mod`, so for each the
backtest creates a throwaway `git worktree` of the repo at the listed HEAD, copies a real,
small public Go module (the tracked files of the local clone, `cp -n`, never overwriting a host
file, `.github/` skipped) into the worktree root with its own `go.mod`/`go.sum`, commits that as
a baseline, and runs the move with `--apply` (`GOWORK=off GOFLAGS=-mod=mod`). The moves are real
moves of that module's own code, with importers rewritten; the `go build`/`go vet` outcomes are
those of the module, not of the host project's Java/Rust/TS code. Moves were chosen by a
plan-only scan of every `rename-pkg` and single-file `move-files` in each candidate module;
blocked candidates were not forced. Examples of correct refusals met on the way: chi
`middleware` rename (`path_keyed_registration`, a test uses `runtime.Caller`), chi
`wrap_writer` move (`unexported_dependency` on shared test helpers), and every unexported
`extract-decls` in gorilla/mux and urfave/cli (`shared_helper`; v1 does not export).
vscode caveat: the repo itself contains invalid `.go` fixtures
(`extensions/vscode-colorize-tests/test/colorize-fixtures/test-13777.go`,
`extensions/copilot/test/scenarios/test-explain/functions.go`) and a C++-bearing Go directory,
so `go build ./...` fails at the repo root with or without the move; the tool rolled the first
attempt back instead of declaring success. Worktrees were removed afterwards.

Scale/noise run on `kubernetes/kubernetes` (plan only, nothing applied): 60 package
directories under `pkg/` sampled with `random.seed(7)`, each planned as `rename-pkg` to
`<pkg>x`: **37 clean, 23 blocked, 0 errors (~0.4 s each)**. Blockers: `has_subpackages` ×18,
`shadowed_package_name` ×4, `non_go_assets` ×1 (counts re-run after the `non_go_assets`
check was added; before it, 38 clean / 22 blocked). The four shadow blockers are the
conservative-by-design case described above; they were not individually triaged for
being true or false positives.
