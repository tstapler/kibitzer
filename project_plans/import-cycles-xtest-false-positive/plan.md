# Plan: import-cycles-xtest-false-positive

**Complexity**: 1 (single function change in one file, no new types or package boundaries)  
**Calibration**: Adversarial review only; architecture review and UX subagents skipped.

---

## Context

`kibitzer check architecture import-cycles` falsely reports cycles on the Go external-test-
package pattern. Root cause: `go_package_identity` in `src/import_graph.rs` derives a
file's graph node key from its directory + `go.mod` and ignores the parsed `package`
clause. `widget_test.go` (`package widget_test`) and `widget.go` (`package widget`) live in
the same directory → same graph key → `widget_test.go`'s import of the helper is
attributed to `widget`, closing a spurious cycle.

Fix: read the `package_clause` from the already-parsed tree-sitter tree inside
`go_package_identity`. If the declared name ends with `_test`, return `None` to exclude
the file from graph construction entirely.

---

## Epic 1 — Fix `go_package_identity` to skip external test packages

### Story 1.1 — Modify `go_package_identity` to parse the package clause

**File**: `src/import_graph.rs`

**Task 1.1.1** — Remove the `_` prefix from `tree`/`src` params and add the clause check.

Replace (lines 328–346):

```rust
/// Go's package identity is **not** derived from parsing the file's `package_clause`
/// node — that node names only the local package name (e.g. `a`), not the full import
/// path other files reference it by (e.g. `example.com/app/a`). It comes from the
/// nearest `go.mod`'s `module` directive (searched upward from the file's own
/// directory, per `find_go_mod_upward`) plus the file's directory relative to *that*
/// `go.mod`'s directory — not always `repo_root`, once a Go module is nested under a
/// larger repo. `tree`/`_src` are accepted-but-unused only to satisfy
/// `QualifiedImportLangConfig::package_identity`'s shared signature.
fn go_package_identity(
    repo_root: &Path,
    file: &Path,
    _tree: &tree_sitter::Tree,
    _src: &[u8],
    cache: &mut PackageIdentityCache,
) -> Option<String> {
    let dir = file.parent()?;
    let (module_path, module_dir) = go_module_for_dir(dir, repo_root, cache)?;
    go_package_import_path(&module_path, &module_dir, file)
}
```

With:

```rust
/// Go's package identity is derived from the nearest `go.mod`'s `module` directive
/// (searched upward from the file's own directory, per `find_go_mod_upward`) plus the
/// file's directory relative to *that* `go.mod`'s directory — not always `repo_root`,
/// once a Go module is nested under a larger repo.
///
/// External test package files (`package X_test`) are excluded: they are a separate
/// compilation unit from `package X` and may legally import packages that import `X`,
/// so edges from them cannot form a real import cycle. Returning `None` removes them
/// from the graph entirely, suppressing false positives in every checker that uses it.
/// Internal test files (`package widget` declared in `widget_test.go`) are *not*
/// excluded — their package name does not end with `_test`.
fn go_package_identity(
    repo_root: &Path,
    file: &Path,
    tree: &tree_sitter::Tree,
    src: &[u8],
    cache: &mut PackageIdentityCache,
) -> Option<String> {
    // Skip external test packages (package X_test): separate compilation unit.
    let root = tree.root_node();
    let mut cursor = root.walk();
    let declared_name = root
        .children(&mut cursor)
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

**Acceptance criteria for 1.1.1**:
- `cargo build` passes without warnings.
- Both `_tree`/`_src` parameter name prefixes are removed.
- Doc comment updated to document the external-test-package exclusion.

---

### Story 1.2 — Add regression test for the canonical three-package layout

**File**: `src/import_graph.rs`, in `#[cfg(test)] mod tests`

**Task 1.2.1** — Add `go_external_test_package_does_not_create_false_cycle` test after the
existing `go_import_graph_finds_a_two_package_cycle` test (line 1552).

```rust
/// Regression for the false-positive cycle pattern: an external test package
/// (`package widget_test`) that imports a helper (`widgettest`) which in turn imports
/// the package under test (`widget`).  Go's compiler and `go vet` accept this layout;
/// kibitzer must not report a cycle.
///
/// Layout:
///   example.com/m/widget/            package widget
///   example.com/m/widget/widgettest/ package widgettest  (imports widget)
///   example.com/m/widget/widget_test.go  package widget_test  (imports widgettest)
#[test]
fn go_external_test_package_does_not_create_false_cycle() {
    let dir = tmp_dir("go-external-test-pkg-no-cycle");
    write(&dir, "go.mod", "module example.com/m\n\ngo 1.21\n");

    let widget = write(
        &dir,
        "widget/widget.go",
        "package widget\n\nfunc New() {}\n",
    );
    let helper = write(
        &dir,
        "widget/widgettest/helper.go",
        "package widgettest\n\nimport \"example.com/m/widget\"\n\nfunc NewTest() { widget.New() }\n",
    );
    let ext_test = write(
        &dir,
        "widget/widget_test.go",
        "package widget_test\n\nimport \"example.com/m/widget/widgettest\"\n\nfunc TestFoo(t *testing.T) { widgettest.NewTest() }\n",
    );

    let graph = build(&dir, &[widget.clone(), helper.clone(), ext_test]).unwrap();

    // The external test file must not appear in the graph at all.
    assert!(
        !graph.nodes.contains("example.com/m/widget_test"),
        "external test package must not be a graph node"
    );

    // No edge from widget to widgettest (the false-cycle edge must be absent).
    assert!(
        !graph
            .edges
            .iter()
            .any(|e| e.from == "example.com/m/widget"
                && e.to == "example.com/m/widget/widgettest"),
        "must not have a widget -> widgettest edge from the external test file"
    );

    // Real edge: widgettest -> widget must still be present (it's a non-test import).
    assert!(
        graph
            .edges
            .iter()
            .any(|e| e.from == "example.com/m/widget/widgettest"
                && e.to == "example.com/m/widget"),
        "widgettest -> widget edge must still be present"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
```

**Acceptance criteria for 1.2.1**:
- Test compiles and passes with `cargo test go_external_test_package_does_not_create_false_cycle`.
- No existing tests are broken (`cargo test` green).

---

### Story 1.3 — Verify internal test files are unaffected

**Task 1.3.1** — Confirm (by running existing tests) that `package widget` declared in a
`widget_test.go` file still participates in the graph. The existing test at line 1655
(`file_packages_maps_go_files_to_module_qualified_keys`) covers non-test files; add a
companion test:

```rust
/// Internal test files (same package name, `*_test.go` suffix) must still appear in
/// the graph — only external test packages (`package X_test`) are excluded.
#[test]
fn go_internal_test_file_still_appears_in_graph() {
    let dir = tmp_dir("go-internal-test-file");
    write(&dir, "go.mod", "module example.com/m\n\ngo 1.21\n");
    let f = write(
        &dir,
        "widget/widget_test.go",
        "package widget\n\nfunc helperForTest() {}\n",
    );

    let graph = build(&dir, &[f.clone()]).unwrap();

    // An internal test file is part of the package; must be in the graph.
    assert!(
        graph.nodes.contains("example.com/m/widget"),
        "internal test file must still register its package"
    );
    assert_eq!(
        graph.file_packages.get(&f),
        Some(&"example.com/m/widget".to_string()),
        "internal test file must appear in file_packages"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
```

**Acceptance criteria for 1.3.1**:
- Test passes; confirms AC-4 from requirements.

---

## Epic 2 — Backtest and documentation

### Story 2.1 — Run `cargo test` suite

**Task 2.1.1** — Run `cargo test 2>&1 | tail -20` from the repo root. All tests must pass,
including the new regression test. Record the result in the PR description.

### Story 2.2 — Corpus backtest (required per CLAUDE.md)

**Task 2.2.1** — The fix only *removes* graph edges (external test files excluded from
construction). It cannot introduce new false positives; the only regression risk is a
real cycle that was only detectable through an external test file's edge. That cannot be
a real compile-time cycle (Go would reject it), so any such "lost" finding was itself a
false positive.

Run the backtest for `import-cycles` before and after the patch:

```sh
# baseline: build from master, run against a Go corpus repo
kibitzer check architecture import-cycles ~/code/github.com/kubernetes/kubernetes 2>/dev/null | wc -l
# post-patch: same
```

Document before/after counts in the PR description. A decrease is expected; an increase
is a blocker.

### Story 2.3 — Update `go_package_focus.rs` comment (optional, non-blocking)

The post-hoc workaround comment in `go_package_focus.rs` (the `test_import_edges_excluded`
message, lines 424–428, 491–494) becomes misleading once the upstream fix lands — those
edges will never be in the graph at all. Optionally update the comment to reflect this, or
remove the now-redundant filter, in a follow-up PR. Not required for this fix.

---

## Migration Plan

N/A — complexity 1. No schema changes, no stored data format changes, no migration needed.

## Observability Plan

N/A — complexity 1. The change removes false positives from a checker output; no new
metrics or logs needed.

## Risk Controls

N/A — complexity 1. Single-function edit in one file; full test suite provides coverage.
Backtest on corpus confirms no regressions.

---

## Adversarial Review

### Review pass

**Reviewer**: planning adversarial pass (self-review)

**Findings**:

1. **CONCERN — Walker cursor lifetime**: The proposed code uses `root.walk()` and passes
   `&mut cursor` to `root.children(...)`. In tree-sitter Rust, `TreeCursor` borrows the
   tree; creating it via `root.walk()` and immediately passing `&mut cursor` to the same
   chain is fine, but the reviewer should confirm the borrow does not conflict. Looking at
   how `extract_go_file_imports` (line 395) does the same thing (binds `cursor` to a local,
   then passes `&mut cursor`), the pattern is established and correct.
   **Resolution**: pattern already used at line 395; no change needed.

2. **CONCERN — `is_some_and` MSRV**: `Option::is_some_and` was stabilised in Rust 1.70.
   Confirm `Cargo.toml`'s `rust-version` allows it.
   **Resolution**: check with `grep rust-version Cargo.toml`. If the MSRV is < 1.70,
   replace with `matches!(declared_name, Some(n) if n.ends_with("_test"))`.

3. **MINOR — `declared_name` ownership**: `utf8_text` returns `&str` borrowed from `src`.
   The `.is_some_and(|n| n.ends_with("_test"))` closure captures by value (the `&str` is
   `Copy`), so this is fine. No `.to_string()` allocation needed.

4. **MINOR — No `graph.file_packages` entry for excluded files**: The research note
   says `arch_model.rs` falls back to calling `go_package_identity` directly for absent
   keys. Confirm this means excluded external test files don't appear anywhere in the arch
   model either (consistent). If any code path assumes `file_packages` is total over Go
   files, it would break. The existing pattern of `go_package_identity` returning `None`
   for files outside a module already exercises this code path — no new gap.

**Verdict**: CLEAN — no blockers. Two minors noted above (MSRV check, `to_string` skip).

---

## Flagged Design Choices

None. The fix is a straightforward application of existing parsing infrastructure
(`extract_go_file_imports`'s clause-reading pattern) to close a known gap. No ADRs needed.

---

## Open Questions / Follow-ups (non-blocking)

- Should `go_package_focus.rs`'s post-hoc `_test.go` edge filter be cleaned up in this
  PR or a follow-up? (Follow-up preferred to keep this PR minimal.)
- Should `coupling` and `layering` acceptance tests be added explicitly for external test
  files, or is the upstream graph fix sufficient evidence? (The existing tests for those
  checkers cover their logic; the graph fix's test covers the data they receive.)

---

## Implementation note (supersedes Story 1.1)

Returning `None` from `go_package_identity` broke `refactor::tests::plan_should_CoverAllImportersAndTestPackage_*`
and `apply_should_RenameTestPackages_*` (the refactor planner needs `a_test.go` grouped with `a`). Shipped
instead: `QualifiedImportLangConfig::skips_edges` (Go: `go_declares_external_test_package`) skips pass-2 edge
emission for `package X_test` files while leaving node/`file_packages` membership unchanged.

**Why AC3's literal "excluded from the node/edge set" is not met (deliberate).** Dropping `X_test` files from
the node set broke two refactor-planner tests (above): renaming package `a` must also rewrite `package a_test`
files, which requires them grouped with `a`. AC3's stated purpose ("so their imports cannot create false cycle
edges") is met by emitting no outgoing edges from those files; they remain package-group members only.

## Corpus backtest (AC8)

Baseline = `635eab4` (pre-fix) release build; after = this branch's release build. Command, per repo:

    <bin> check architecture import-cycles ~/code/github.com/<owner>/<repo>

| Repo | `[import-cycle]` findings before → after | Notes |
|---|---|---|
| kubernetes/kubernetes | 28 → 2 | 0 findings new by file. Removed: the `X_test`-shaped ones. Remaining 2 (`pkg/proxy/ipvs/testing/fake_test.go`, `kubeadm/app/util/staticpod/utils_linux.go`) are unchanged; the first is an *internal* test file (`package testing`) in the helper package — same pre-existing FP class as go-git below, out of scope (AC4 keeps internal test files). The kubeadm one was not inspected, so its status is unverified. |
| tstapler/stapler-squad | 4 → 3 | Only change: the Go `server/auth → … → testutil → … → session → server/auth` cycle removed. The 3 TypeScript cycles are byte-identical. |
| tstapler/go-git (extra, Go) | 1 → 2 | The one large SCC split into two smaller ones once `X_test` edges were gone: `http/internal/test ↔ http` and `ssh/internal/test ↔ ssh`, both from `internal/test/proxy_test.go` (`package test`, an internal test file) importing the consumer. A pre-existing false-positive class (a helper package's own `_test.go` imports the package under test, which is legal in Go); previously hidden inside the big cycle. Not introduced by this change. |

Real-cycle detection is covered at `build()` level by `go_real_mutual_import_still_reports_cycle`. The corpus
has no confirmed-real Go cycle (Go rejects them), so "no lost real cycles" rests on that test plus the
unchanged TypeScript results. Raw outputs: `~/.cache/tmp/cyc/{kubernetes,stapler-squad,go-git}.{B,H}.txt` (not committed).

Follow-up (not in scope): suppress edges from internal `_test.go` files of helper packages for cycle purposes.
Full suite after the change: `cargo test --workspace` → 2576 passed, 0 failed, 7 ignored (one unrelated
`refactor_core::txn::tests::the_lock_covers_everything_between_session_and_commit` failure on one earlier run did not recur).
