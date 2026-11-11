# Research: fix-go-external-test-pkg-cycles

## Root Cause — VERIFIED

### `go_package_identity` ignores the `package` clause

`src/import_graph.rs:336–346`:

```rust
fn go_package_identity(
    repo_root: &Path,
    file: &Path,
    _tree: &tree_sitter::Tree,   // ← explicitly ignored
    _src: &[u8],                  // ← explicitly ignored
    cache: &mut PackageIdentityCache,
) -> Option<String> {
    let dir = file.parent()?;
    let (module_path, module_dir) = go_module_for_dir(dir, repo_root, cache)?;
    go_package_import_path(&module_path, &module_dir, file)
}
```

The function derives identity from `(go.mod module path) + (file's directory relative to
go.mod)`. It never reads the file's `package` clause. `widget.go` (`package widget`) and
`widget_test.go` (`package widget_test`) live in the same directory → they both receive
`example.com/m/widget` as their graph node key.

This is intentional for the `package` *clause name* (e.g. `widget`) vs. the full *import
path* (`example.com/m/widget`) — the clause name alone can't form the qualified identity.
But it accidentally collapses external test packages into the packages they test.

### How the false cycle forms (traced through `build_qualified_name_language`)

Pass 1 (lines 216–235): both `widget.go` and `widget_test.go` are registered under
`example.com/m/widget` in `graph.nodes` and `graph.file_packages`. Both are pushed into
`file_packages`.

Pass 2 (lines 240–253): `widget_test.go`'s imports include
`example.com/m/widget/widgettest` (the helper). Because `widgettest` is in `graph.nodes`,
an edge is added: `from="example.com/m/widget" to="example.com/m/widget/widgettest"`.
`widgettest` itself imports `example.com/m/widget`. That edge also exists. Result:
`widget → widgettest → widget` — a reported cycle that Go's compiler and vet both accept.

### Parsing infrastructure already exists

`extract_go_file_imports` (lines 392–409) already parses the `package_clause` from a
tree-sitter tree and returns the declared name as `GoFileImports.package_name`. It uses:

```rust
root.children(&mut cursor)
    .find(|c| GoKind::of(*c) == GoKind::PackageClause)
    .and_then(|pc| pc.named_child(0))
    .and_then(|id| id.utf8_text(src).ok())
    .map(str::to_string)
```

`extract_go_file_imports` is called at `arch_model.rs:473` to populate the arch model's
short package names. It is not called from the import graph builder — the two code paths
are separate.

## Callers of Key Functions

- `go_package_identity`: only used as a function-pointer value in `go_lang_config()`
  (line 453), which is called only from `build()` (line 66). No other callers.
- `build_qualified_name_language`: called from `build()` for Go, Java, and Kotlin (lines
  66, 79, 84). The Go call is the only one affected; Java/Kotlin have no external-test
  package concept.
- `extract_go_file_imports`: called from `arch_model.rs:473` only (outside import_graph).

## Existing Precedent: `go_package_focus.rs`

`go_package_focus.rs` already recognises the issue. When computing coverage for a split
recommendation, it filters edges that come from `_test.go` files and reports them
separately (lines 424–428, 491–494):

```rust
let test_import_edges_excluded = model
    .import_edges
    .iter()
    .filter(|e| e.from == pkg.path && e.file.to_string_lossy().ends_with("_test.go"))
    .count();
// …
"{} import edges from _test.go files ignored; test-only cycles are not reported"
```

This post-hoc filtering was added because the graph incorrectly includes these edges. The
correct fix is to suppress them upstream in graph construction so no other checker is
affected by them either.

## Fix Options

### Option A — Parse package clause in `go_package_identity` (RECOMMENDED)

Modify `go_package_identity` to actually use the `tree`/`src` parameters already
threaded through the signature. Read `package_clause` → `PackageIdentifier`. If the
declared name ends with `_test`, return `None` to skip the file from graph construction.

```rust
fn go_package_identity(
    repo_root: &Path,
    file: &Path,
    tree: &tree_sitter::Tree,
    src: &[u8],
    cache: &mut PackageIdentityCache,
) -> Option<String> {
    // Skip external test packages (package X_test): they are a separate
    // compilation unit and may legally import packages that import X.
    let declared_name = tree.root_node()
        .children(&mut tree.root_node().walk())
        .find(|c| GoKind::of(*c) == GoKind::PackageClause)
        .and_then(|pc| pc.named_child(0))
        .and_then(|id| id.utf8_text(src).ok());
    if declared_name.is_some_and(|n| n.ends_with("_test")) {
        return None;
    }
    let dir = file.parent()?;
    let (module_path, module_dir) = go_module_for_dir(dir, repo_root, cache)?;
    go_package_import_path(&module_path, &module_dir, file)
}
```

**Pros**: single-site change; correct per-file (not a filename heuristic); removes edges
upstream so all architecture checkers benefit; aligns with the design intent already
documented in `go_package_focus.rs`'s note about test-only cycles.

**Cons**: `go_package_identity` now reads the tree for every file even when no `go.mod`
is found — minor, since the `go_module_for_dir` cache already does I/O.

Also update the doc comment on `go_package_identity` to document the exclusion.

### Option B — File-name heuristic: skip all `_test.go` files

Before calling `go_package_identity`, filter out any file whose path ends with `_test.go`.

**Pros**: even simpler.
**Cons**: internal test files (`package lib` in `lib_test.go`) are excluded too, losing
their import edges from coupling/layering checks. Those edges are arguably useful signal.
Also less precise — the right predicate is "declared package ends with `_test`", not
"file ends with `_test.go`".

### Option C — Filter at `find_cycles` time

At cycle detection, skip graph edges whose `file` path ends with `_test.go`. Leaves the
graph intact.

**Pros**: other checkers keep the edges.
**Cons**: coupling and layering checkers continue to report false violations from external
test edges; Option A's upstream removal is cleaner and consistent with the existing design
intent expressed in `go_package_focus.rs`.

**Recommendation**: Option A.

## Impact on `graph.file_packages`

`graph.file_packages` maps each file to its graph key (`PathBuf → String`). External test
files excluded by `go_package_identity` returning `None` will not appear in this map.
`arch_model.rs`'s `package_key_for_file` (line 462) calls `import_graph.file_packages`
to find a file's package; if the file is absent, it falls back to `go_package_identity`
directly. The arch model builder will also call `go_package_identity` and get `None` for
external test files, skipping them — consistent behaviour.

## Test Infrastructure

Unit tests for the import graph live in `src/import_graph.rs #[cfg(test)] mod tests`.
Tests use `tmp_dir()` + `write()` to create fixture files in a temp directory, then call
`build()`. Existing Go cycle test: `go_import_graph_finds_a_two_package_cycle` (line 1519).

The `testdata/split-sim/test-only-cycle/` fixture has an *internal* test file (`package
lib`), not an external `package lib_test` file — not a direct fixture for this bug. A new
test should use the three-package layout in the requirements.

## Backtest Requirements

Per CLAUDE.md and `docs/backtesting.md`:

1. `cargo test` — all existing import graph tests must pass.
2. The fix only *removes* findings (false positives) — it cannot introduce new false
   positives, only regressions where real cycles are missed. The concern is: could an
   external test package file's edge be the *only* edge completing a real cycle? In that
   case, the cycle is test-only and Go's compiler wouldn't reject it, so removing the
   finding is correct.
3. For corpus backtest, the Go repos are `kubernetes/kubernetes` and
   `tstapler/stapler-squad` (see `docs/backtest-repos.md`). Run:
   ```sh
   find ~/code/github.com/kubernetes/kubernetes -name '*.go' > /tmp/k8s-files.txt
   kibitzer check architecture import-cycles ~/code/github.com/kubernetes/kubernetes
   ```
   Compare finding counts before and after the patch.

## Files to Modify

| File | Change |
|------|--------|
| `src/import_graph.rs` | Modify `go_package_identity` to parse `package_clause` and return `None` for `_test`-suffixed names; update its doc comment |
| `src/import_graph.rs` | Add regression test for three-package X / Xtest / X_test layout |

No changes needed to `architecture_checks.rs` or `go_package_focus.rs` — they benefit
automatically from the upstream graph fix.
