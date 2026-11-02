# Go Package Move Operations — Research

## 1. gopls Feasibility

### API surface

gopls is a separate Go module (`golang.org/x/tools/gopls`), distinct from
`golang.org/x/tools`. The most recent version in this machine's module cache is
**v0.24.0** (released 2026-10-14), confirmed at
`~/go/pkg/mod/cache/download/golang.org/x/tools/gopls/@v/v0.24.0.info`
(VERIFIED).

gopls is not installed on PATH on this machine (`which gopls` returns empty,
VERIFIED). Testing the actual command surface was not possible.

### Identifier rename

The `golang.org/x/tools/refactor/rename` package (the old `gorename` library)
explicitly says in its own package doc comment at
`~/go/pkg/mod/golang.org/x/tools@v0.33.0/refactor/rename/rename.go` (VERIFIED,
read):

> "Package rename contains the obsolete implementation of the deleted
> golang.org/x/tools/cmd/gorename. This logic has not worked properly since the
> advent of Go modules, and should be deleted too. Use gopls instead, either via
> the Rename LSP method or the 'gopls rename' subcommand."

gopls exposes identifier rename via the LSP `textDocument/rename` method and a
CLI subcommand `gopls rename -w <file>:#<byte-offset> <new-name>`. This is
confirmed by gorename's redirect above (VERIFIED via the package comment).

### Package-level move

Whether gopls has a dedicated `workspace/executeCommand` command for package-level
moves (distinct from identifier-level rename) is UNVERIFIED — gopls was not
installed and its full command set was not examined. The `gopls.refactor.*`
command namespace is known to exist from public LSP docs, but which specific
commands are present in v0.24.0 and whether any directly executes a package
directory move is not confirmed.

### Driving gopls from Rust

Driving gopls from a Rust binary requires one of:
- Spawning `gopls rename ...` as a subprocess (CLI mode, identifier rename only).
- Sending JSON-RPC LSP messages over a spawned gopls process's stdio (`{"method":"workspace/executeCommand",...}`).

Both approaches require `gopls` to be present in the user's PATH at kibitzer run
time. This is a new runtime dependency kibitzer has not taken on before — it
currently has no mandatory external-binary dependencies for its analysis path.

### Version compatibility risk

gopls renamed or removed commands between minor versions (e.g. the gopls.mod.*
command set changed between 0.9 and 0.11). Any integration that calls
`workspace/executeCommand` must version-gate the call, adding maintenance surface.

### gopls verdict

INFERRED: gopls covers identifier-level rename well. For package-level moves it
is a plausible approach but the exact command API is unconfirmed on this machine
and it would introduce a mandatory runtime dependency. If the plan phase chooses
gopls, a one-shot verification experiment (`go install gopls@latest && gopls
help`) must be run before implementation to confirm the package-move command
surface and its exact behavior at module boundaries.

---

## 2. go/packages and go/ast Rewriting

### gomvpkg (`golang.org/x/tools/cmd/gomvpkg`)

VERIFIED (source read at
`~/go/pkg/mod/golang.org/x/tools@v0.33.0/cmd/gomvpkg/main.go` and
`refactor/rename/mvpkg.go`):

`gomvpkg` moves a package directory and rewrites all import paths that reference
it. Its algorithm:

1. Build the import graph via `importgraph.Build(ctxt)`.
2. Find all packages that import the moved path or any of its subpackages.
3. Rewrite their `import` declarations using `go/ast`.
4. Move the directory (optionally via a `--vcs_mv_cmd` template like `"git mv
   {{.Src}} {{.Dst}}"`).

`mvpkg.go` imports `golang.org/x/tools/go/loader` (VERIFIED). `go/loader` uses
`go/build` context (GOPATH-based). The module (`golang.org/x/tools@v0.33.0`) does
not contain a module-aware replacement for this path — the package still imports
`go/loader`. This means `gomvpkg` may not resolve import paths correctly in
Go-modules projects that use multi-module layouts or vendoring.

`gomvpkg` is not a library with a clean public API — it is a CLI tool whose
library entry point (`rename.Move`) is in a package marked obsolete. It is not a
safe building block for kibitzer.

### refactor/importgraph (`golang.org/x/tools/refactor/importgraph`)

VERIFIED (source at
`~/go/pkg/mod/golang.org/x/tools@v0.33.0/refactor/importgraph/graph.go`):

Provides `Build(ctxt *build.Context)` returning `(forward Graph, reverse Graph,
errors map[string]error)` where `Graph` is `map[string]map[string]bool`. Uses
`go/build` context. Same GOPATH limitation as `gomvpkg`.

### gorename (`golang.org/x/tools/refactor/rename`)

VERIFIED (package doc in rename.go): Marked obsolete, "has not worked properly
since the advent of Go modules." Do not use.

### rf (rsc.io/rf by Russ Cox)

VERIFIED (binary present at `/home/tstapler/go/bin/rf`, source at
`~/go/pkg/mod/rsc.io/rf@v0.0.0-20260710193434-85dbf8d53aee/`):

`rf` is a refactoring tool written by Russ Cox. It uses `exec.Command("go",
"env", "GOMOD")`, `exec.Command("go", "mod", "edit", "-json")`, and
`exec.Command("go", "list", ...)` to load packages (VERIFIED from `refactor/snap.go`
lines 176, 189, 212). It is **module-aware**.

`rf`'s `mv` command (VERIFIED from `mv.go`) supports:
- Moving a named declaration (type, function, const, var) from one package to another.
- Moving a method to a different type.
- Moving a whole package directory (`dst.Kind == refactor.ItemDir`) to a new import path.

It uses `go/types` for type-checking — rewrites are driven by resolved types, not
text matching.

The `rf` README states (VERIFIED): "rf is incredibly rough and likely to be buggy
and change incompatibly." It is explicitly experimental.

`rf`'s apply step writes files with `ioutil.WriteFile` directly (VERIFIED from
`refactor/edit.go` line 348). No temp-then-rename, no rollback — a mid-apply
crash can leave a partially-rewritten tree.

### goimports

VERIFIED (binary at `/home/tstapler/.local/bin/goimports`). Adds/removes import
declarations to match what the source actually uses. Useful as a post-apply
cleanup step, not a rewrite engine itself.

### Conclusion

The only module-aware, type-safe package move tool available locally is `rf`.
Using it from kibitzer would mean spawning it as a subprocess (`rf 'mv
old/pkg new/pkg'`) and parsing its output or checking the exit code. Its
experimental status and lack of rollback are risks, but it is the most capable
existing tool for the job if a subprocess approach is chosen.

---

## 3. Tree-Sitter Go Approach

### What kibitzer already has

VERIFIED (Cargo.toml lines 26, 25; import_graph.rs; arch_model.rs; symbol_extract.rs):

- `tree-sitter-go = "0.25"` is already a dependency.
- `import_graph.rs` already parses `import_spec` nodes to extract `path` and
  `name` (alias) fields from every Go file. It tracks explicit import aliases in
  `GoImportSpec { path: String, alias: Option<String> }`.
- `go_package_identity()` in `import_graph.rs` already searches upward for
  `go.mod` to compute the full qualified import path for any Go file, without
  needing to parse the `package_clause` node.
- `arch_model.rs` builds `PackageNode` (with `path`, `files`, `symbols`),
  `CallEdge` (from/to symbol ids, file, line), and `FieldAccessEdge` — a full
  call and field-access graph, already resolved.
- `symbol_extract.rs` extracts `type_declaration` (Go types and interfaces),
  `function_declaration`, and `method_declaration` with file and line positions.
- `declarations.rs` builds a flat `DeclarationGraph` with `DeclKind` and component
  assignment.
- The tree-sitter `selector_expression` node is already walked in `arch_model.rs`
  for field-access tracking (`walk_field_accesses`).

### What a pure tree-sitter rewrite would look like

Given an operation spec (move declarations D from package A to package B):

1. **For each file in package A that contains a declaration in D:**
   - Rewrite the `package_clause` to reflect the new package name (the last segment of B's path, unless an explicit name is given).
   - Move the file to the new directory.

2. **For each file in package A or B (or any file that imports A) that references a moved declaration:**
   - Walk `import_spec` nodes: find the spec whose `path` matches A's import path, rewrite it to B's import path, updating the alias if it was implicit (was matching the last-segment name).
   - Walk `selector_expression` nodes: when the operand identifier resolves (via import alias table) to the A package, and the selected name is in the moved declaration set D, the reference is already valid if the import rewrite happened; no further AST change needed.
   - If A is now empty after the move, remove its import spec.

3. **For importers of B that already existed before the move:**
   - No import changes needed — they already import B. But if any newly moved declarations shadow existing B names, report a conflict.

4. **Run goimports / gofmt post-apply** to normalize the import block.

### Risks and limits

1. **Alias resolution**: Required to correctly scope selector rewrites.
   Kibitzer already tracks aliases — this risk is managed.

2. **Dot imports** (`import . "pkg"`): Merge a package's exported names into the
   file's unqualified scope. Kibitzer already skips dot imports in `GoImportSpec`
   (VERIFIED from `go_import_spec()` which returns `None` for dot/blank imports).
   A move tool must either handle dot imports explicitly or produce a blocking
   diagnostic and refuse to proceed when a mover package has a dot-importing caller.

3. **`_test` packages**: A file ending in `_test.go` with `package foo_test` is an
   external test package whose import path is the same as `foo`'s but whose
   `package` clause differs. The rewrite must handle both `package foo` (internal
   tests) and `package foo_test` (external tests) when moving a package.

4. **Declaration set vs. identifier scope**: tree-sitter cannot determine whether
   `a.Error` refers to the moved `Error` type or some other `Error` in a different
   package also aliased to `a`. The arch_model already disambiguates this: import
   path → package node → symbol list. The rewrite step must use this resolved data,
   not guess from identifier text alone.

5. **Cycle detection**: tree-sitter cannot detect import cycles. Kibitzer's
   existing `ImportGraph` can: after computing the proposed move, check that no new
   cycle would be introduced using the existing `edges_from()` traversal.

6. **Build tags and cgo files**: Files with `//go:build` constraints may only
   exist on certain platforms. `source.rs` (`go_package_focus/source.rs`) already
   detects build tags (VERIFIED). A move tool must carry them along unchanged or
   produce a blocking diagnostic.

7. **Generated files**: The `// Code generated` marker means a file should not
   be hand-edited. A move tool should detect this marker and either skip the file
   with a diagnostic, or include a note that the generator must be re-run.

### Tree-sitter verdict

A pure Rust/tree-sitter approach is viable for the syntactic transformation
layer. It avoids all external runtime dependencies and aligns with kibitzer's
existing architecture. The semantic decisions (which declarations to move, which
callers need rewrites, cycle detection) are supplied by the existing arch_model
and import_graph — tree-sitter is responsible only for the text transformations
that follow. Post-apply `go build ./...` and `go vet ./...` (already required by
AC-4) catch any transformation the syntax layer missed.

---

## 4. Atomicity Mechanisms

### What existing tools do

**rf**: `ioutil.WriteFile` in sequence, no temp-then-rename, no rollback
(VERIFIED from `refactor/edit.go` line 348). Mid-apply crash leaves partial
state.

**gomvpkg**: Moves the directory first (via the vcs_mv_cmd template or
`os.Rename`), then rewrites importers. Directory move and file rewrites are not
atomic together. If a file rewrite fails, the directory is already moved.

Neither tool satisfies kibitzer's AC-3 requirement of byte-identical pre-apply
state on failure.

### Pattern that satisfies AC-3

The collect-then-apply pattern:

1. **Collect phase**: compute all (file_path → new_content_bytes) pairs in
   memory. If any computation fails (cycle detected, internal/ boundary violated,
   generated file in the way), abort here — no files are written.

2. **Snapshot phase**: read and cache the current content of every file that will
   be written. If any read fails, abort.

3. **Apply phase**: for each (path, new_content) pair, write via a temp file in
   the same directory then `rename()` over the original (POSIX-atomic per file).
   If any write fails, restore every already-written file from the snapshot.

4. **Verify phase**: run `go build ./...` and `go vet ./...` over affected
   packages. On non-zero exit, restore from snapshot and report the failure output.

The snapshot-and-restore strategy (step 2–3) provides the cross-file
pseudo-atomicity the requirements demand. True all-or-nothing atomicity across
multiple files is not achievable without a transaction journal, but a failed write
that prompts a full restore is indistinguishable from a pre-apply state for any
reader that cannot observe the in-progress window.

The write-to-temp-then-rename pattern (step 3) prevents a partial write from
corrupting a file that `go build` would then attempt to compile: `rename()` is
atomic on POSIX, so the file is either the old content or the new content, never
a partial write.

---

## 5. Existing Art Summary

| Tool | Primary capability | Module-aware | Installed locally | Status |
|---|---|---|---|---|
| `gorename` (`golang.org/x/tools/refactor/rename`) | Identifier rename | No (go/loader) | No | Obsolete (self-described in source) |
| `gomvpkg` (`golang.org/x/tools/cmd/gomvpkg`) | Package directory move + import rewrite | No (go/loader) | No | Pre-modules, not maintained |
| `rf` (rsc.io/rf) | Declaration and package mv, type-checked | Yes (go list) | Yes (`~/go/bin/rf`) | Experimental, "likely buggy and change incompatibly" |
| gopls rename | Identifier rename via LSP | Yes | No | Active, production-quality |
| gofmt | Formatting | N/A | Yes (`/home/linuxbrew/.linuxbrew/bin/gofmt`) | Active |
| goimports | Import cleanup | N/A | Yes (`~/.local/bin/goimports`) | Active |

**rf** is the most capable existing tool locally available. It is the closest
analogue to what kibitzer needs (type-safe, module-aware, `mv` at declaration and
package granularity). Its experimental label and lack of rollback are the primary
risks.

**gomvpkg** predates modules and is effectively unmaintained. Its use of
`go/loader` means it may misidentify import paths in module-mode projects.

**gorename** is self-declared obsolete — do not use.

---

## 6. Kibitzer Codebase Hooks

All findings in this section are VERIFIED (files read directly).

### `src/import_graph.rs`

Provides `ImportGraph { nodes: BTreeSet<String>, edges: Vec<ImportEdge>,
file_packages: BTreeMap<PathBuf, String> }`.

- `ImportEdge`: `from`, `to` (both qualified import paths), `file`, `line`.
- `file_packages`: maps every file path to its package's qualified import path.
- `edges_from(node)`: forward iterator over outgoing edges from a package.
- `GoImportSpec { path, alias: Option<String> }` and `GoFileImports` track
  per-file import aliases — required for correct selector-expression rewriting.
- `go_package_identity()` resolves the full import path for any Go file via
  upward `go.mod` search.

**Reuse for move-files**: the reverse of `edges_from()` (all packages that import
package A) can be derived by scanning `edges` for `to == A`. This enumerates
every file that needs an import-path rewrite when A is moved. No new graph build
is needed — the existing `ImportGraph` already contains this data.

### `src/arch_model.rs`

Provides `PackageNode { path, files, symbols }` and the full call/field-access
graph:

- `SymbolNode`: `id` (deterministic, owner-qualified), `name`, `kind`
  (Type/Interface/Function/Method), `file`, `line`, `exported`, `params`.
- `CallEdge`: `from` (symbol id), `to` (symbol id or raw text), `resolved`, `file`, `line`.
- `FieldAccessEdge`: `from` (method id), `to` (synthetic field id), `access`, `file`, `line`.

**Reuse for extract-decls**: `SymbolNode` already identifies which declarations
exist in a package and their export status. `CallEdge` (filtered to
`from ∈ moved_set`) reveals transitive private helpers that must move with their
caller. The `exported` flag drives the "what new exports are required" computation.

### `src/go_package_focus.rs`

`analyze(model: &ArchModel, mode: EvidenceMode) -> Vec<PackageFocusFinding>` — the
advisory check that produces the partition `split-pkg` consumes. Each finding
names the two disconnected components and which types belong to each.

The `GoPackageFocusChecker` is already registered in the architecture check
registry (`architecture_checks::registry()`). Its output JSON is the natural
input spec for `kibitzer refactor go split-pkg`.

The module at `src/go_package_focus/source.rs` already detects package-level
shared state (`sync`/`atomic`/`chan` variables), build tags, and generics — these
are the "contrary evidence" signals a move tool must also respect as potential
blockers.

### `src/declarations.rs`

`DeclarationGraph { declarations: Vec<Declaration> }` where `Declaration { name,
kind: DeclKind, file, line, component }`.

Less relevant to the refactoring layer (the `SymbolNode` in `arch_model.rs` is
richer), but the `component` field here provides the `.kibitzer/inspect.json`
component assignment — useful for `sync-policy`.

### `src/symbol_extract.rs`

`extract_symbols_for_file(tree, source, language)` turns a parsed tree-sitter
tree into `Vec<SymbolNode>`. For Go: handles `type_declaration` (wrapping
`type_spec`, classified Type vs Interface via `interface_type` child), plus
`function_declaration` and `method_declaration`.

**Reuse**: the same tree-sitter parsing machinery can be extended in-place to
also walk and rewrite import paths and package clauses. The `checker::GrammarCache`
already caches parsed trees to avoid redundant parsing.

### `src/main.rs` — missing `Refactor` subcommand

VERIFIED: `Command` enum has Run, Hook, Mcp, Lsp, Daemon, Check, Status, Install,
Tests, Architecture, Go, Plugin, Schema. There is no `Refactor` variant.

The new feature adds a top-level `Refactor` variant with a `Go` sub-action
containing `MoveFiles`, `RenamePkg`, `ExtractDecls`, `SplitPkg`, and `SyncPolicy`
sub-subcommands — a peer to the existing `Go { action: GoAction }` and
`Architecture { action: ArchitectureAction }` variants.

---

## Appendix: Toolchain Decision Prerequisites

The requirements (AC-6) require the chosen approach to be documented with:
- The specific API surface checked
- The version tested
- The test result

Based on this research, two questions remain open and must be answered before the
plan phase commits to an approach:

1. **gopls package-move API**: run `go install golang.org/x/tools/gopls@latest &&
   gopls help` and `gopls help refactor.move` to confirm whether a package-level
   move command exists and what its exact invocation is. If confirmed, measure
   whether it works on a test Go module with a multi-file package move, including
   `_test` packages and `internal/` targets.

2. **rf package-level mv reliability**: run `rf 'mv example.com/foo example.com/bar'`
   on a test module with a multi-file, multi-importer package. Record whether it
   correctly rewrites all import paths, handles `_test` packages, and produces a
   clean `go build ./...`. If the answer is yes, assess whether the "likely buggy
   and change incompatibly" warning represents a real deployment risk for kibitzer
   users who may not have `rf` installed.

If both produce clean results, the plan should compare them on the criteria that
matter: no runtime dependency (favors tree-sitter), type-safe rewrites (favors
rf/gopls), confirmed `internal/` boundary enforcement (must test all three),
and cycle detection (kibitzer's import_graph covers this regardless of choice).
