# Implementation Plan: Go Package Move Operations Catalog v1

## 1. Architecture Decisions

### 1.1 Module placement

All new code lives in `src/refactor/` — a new top-level module under `src/`, parallel to how
`src/go_package_focus/` is sub-modularised today. No new Cargo workspace crate. The reasons:

- The transformation layer needs access to `ImportGraph`, `ArchModel`, `GrammarCache`, and
  `checker::Language` from the same binary; a new crate would require duplicating or re-exporting
  most of the core library surface.
- kibitzer has no Cargo workspace structure yet (`Cargo.toml` is a single `[package]`). Splitting
  to a workspace crate for one feature adds churn that a future workspace migration can do
  holistically.
- Precedent: `go_package_focus/source.rs`, `go_package_focus/render.rs`, and the `inline_ignores/`
  sub-tree all follow the `src/<feature>/sub.rs` layout.

Module tree:

```
src/refactor/
  mod.rs          — re-exports, top-level orchestration (plan(), apply())
  types.rs        — MovePlan, PlanStep, ApplyResult, VerifyResult, BlockingDiagnostic
  import_rewrite.rs — import path rewriting (tree-sitter)
  pkg_clause.rs   — package clause rewriting (tree-sitter)
  selector_rewrite.rs — qualified-identifier rewriting (tree-sitter, alias-aware)
  preflight.rs    — internal/ visibility check, cycle detector, generated-file detector,
                    dot-import detector, shared-state detector
  atomic_apply.rs — snapshot + temp-then-rename + restore-on-failure
  verify.rs       — go build, go vet, go test subprocess runners, output capture
  policy_sync.rs  — .kibitzer/inspect.json path updater
  golden_tests.rs — #[cfg(test)] golden-file harness (reads testdata/refactor/)
```

`src/main.rs` gains a `mod refactor;` declaration and a new `Refactor` variant in `Command`.

### 1.2 CLI shape

New top-level variant in `Command` (mirroring `Go { action: GoAction }` and
`Architecture { action: ArchitectureAction }`):

```
Refactor {
    action: RefactorAction,
}
```

`RefactorAction` is a new `#[derive(Subcommand)] enum` with five sub-subcommands:

```
kibitzer refactor go move-files   --src <file>... --dest-pkg <import/path> [--repo <dir>] [--apply] [--dry-run] [--out <file>]
kibitzer refactor go rename-pkg   --from <import/path> --to <import/path>  [--repo <dir>] [--apply] [--dry-run] [--out <file>]
kibitzer refactor go extract-decls --from <import/path> --decls <Name,...> --to <new/import/path> [--repo <dir>] [--apply] [--dry-run] [--out <file>]
kibitzer refactor go split-pkg    --pkg <import/path> --focus-json <file>  [--repo <dir>] [--apply] [--dry-run] [--out <file>]
kibitzer refactor go sync-policy  --plan <plan.json>                       [--repo <dir>] [--apply] [--dry-run] [--out <file>]
```

The `GoRefactorAction` enum is a second level of `#[derive(Subcommand)]` nested inside `RefactorAction`:

```rust
#[derive(Subcommand)]
pub enum RefactorAction {
    Go {
        #[command(subcommand)]
        action: GoRefactorAction,
    },
}

#[derive(Subcommand)]
pub enum GoRefactorAction {
    MoveFiles  { /* ... */ },
    RenamePkg  { /* ... */ },
    ExtractDecls { /* ... */ },
    SplitPkg   { /* ... */ },
    SyncPolicy { /* ... */ },
}
```

**Shared flags across all `GoRefactorAction` variants:**

- `--repo <dir>` (default `.`) — root of the Go module to operate on.
- `--apply` — actually write files; without this flag, the command prints the plan JSON and exits 0.
- `--dry-run` — synonym for omitting `--apply`; prints plan to stdout.
- `--out <file>` — write plan JSON here instead of stdout (applies to both plan-only and post-apply result).

### 1.3 Data structures (`src/refactor/types.rs`)

```rust
/// The canonical serialization of a single file transformation — new path, new content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileTransform {
    /// Absolute path to the file being rewritten (or moved).
    pub old_path: PathBuf,
    /// Destination path (may equal old_path for in-place content rewrites).
    pub new_path: PathBuf,
    /// New file content after all rewrites.  Absent for pure moves (no content changes).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_content: Option<String>,
}

/// One logical step in a plan — describes a single atomic action and its reason.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanStep {
    pub kind: PlanStepKind,   // enum: RewriteFile | MoveFile | DeleteDir | PolicyUpdate
    pub transform: FileTransform,
    /// Human-readable explanation of why this step is needed.
    pub reason: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlanStepKind {
    RewriteFile,
    MoveFile,
    DeleteDir,
    PolicyUpdate,
}

/// A complete refactoring plan: the operation that produced it, every step, and any
/// pre-flight blockers that prevent apply.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MovePlan {
    pub operation: OperationKind,
    pub repo_root: PathBuf,
    /// Import path of the source package(s).
    pub from: Vec<String>,
    /// Import path of the destination package(s).
    pub to: Vec<String>,
    pub steps: Vec<PlanStep>,
    /// Pre-flight blockers.  Non-empty → plan must not be applied.
    pub blockers: Vec<BlockingDiagnostic>,
    /// Simulation result embedded per AC-0 dependency on feasibility simulator (#153).
    /// `None` until that simulator ships; apply is gated on `blockers` being empty, not
    /// on simulation_result being present, so v1 can ship without it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub simulation_result: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    MoveFiles,
    RenamePkg,
    ExtractDecls,
    SplitPkg,
    SyncPolicy,
}

/// A pre-flight error that prevents plan application.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockingDiagnostic {
    pub code: &'static str,      // e.g. "internal_visibility", "cycle_detected"
    pub message: String,
    pub file: Option<PathBuf>,
    pub line: Option<usize>,
}

/// Result of an apply run: which files changed, the verify output, and overall status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyResult {
    pub plan: MovePlan,
    pub applied_steps: Vec<PlanStep>,
    pub verify: VerifyResult,
    pub rolled_back: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyResult {
    pub build_ok: bool,
    pub vet_ok: bool,
    pub test_ok: bool,
    pub build_output: String,
    pub vet_output: String,
    pub test_output: String,
}
```

**JSON output format**: `MovePlan` and `ApplyResult` are serialized as pretty-printed JSON
(`serde_json::to_string_pretty`). The `operation` field acts as a type discriminant. This matches
kibitzer's existing convention (arch export, go package-focus both emit pretty-printed JSON objects).
No envelope wrapper is added; the top-level object is the plan or result directly.

### 1.4 Atomicity implementation (`src/refactor/atomic_apply.rs`)

Four-phase sequence:

1. **Collect**: `MovePlan.steps` arrive fully computed (all transforms in memory). If `blockers` is
   non-empty, return `Err` before writing anything.
2. **Snapshot**: for every `old_path` in the step list, read the current content into a
   `BTreeMap<PathBuf, Vec<u8>>`. Abort on any read error.
3. **Apply**: for each `(old_path, new_path, new_content)`:
   - Write `new_content` to `new_path.with_extension("kibitzer-tmp.<pid>")` in the same directory.
   - `std::fs::rename(tmp, new_path)` — POSIX-atomic single-file rename.
   - If `old_path != new_path`, `std::fs::remove_file(old_path)` after rename.
   - On any error: restore every already-written file from snapshot, return `Err`.
4. **Verify**: run `go build`, `go vet`, `go test` (see §1.5). On non-zero exit, restore from
   snapshot and set `rolled_back: true` in `ApplyResult`.

Restore loop: iterate snapshot, `std::fs::write(path, original_bytes)` for each. Restore errors are
logged to stderr but do not shadow the original error — the caller sees the original.

### 1.5 Verification (`src/refactor/verify.rs`)

Three `std::process::Command` invocations against the set of affected packages:

```
go build ./...              (or go build <pkg1> <pkg2>...)
go vet   ./...
go test  ./...              (with -count=1 to disable caching)
```

"Affected packages" = the union of source and destination import paths, plus any package that had an
import rewrite applied. Passed as space-separated args when the set is small (< 20 packages);
`./...` otherwise to avoid command-line length limits.

All three commands run from `repo_root`. stdout+stderr are captured with `Command::output()` and
stored verbatim in `VerifyResult`. Non-zero exit of any command → `VerifyResult.{build,vet,test}_ok = false`.

The commands run sequentially (not concurrently) because `go test` must see the build that `go
build` just validated; parallel execution can produce misleading output when one command is fixing
the same files another is compiling.

### 1.6 Post-apply formatting

After the apply phase succeeds and before the verify phase, run `goimports -w <file>` (if
`goimports` is on PATH) or `gofmt -w <file>` (fallback) on every rewritten `.go` file. Failure of
the formatter is not a rollback trigger — it logs a warning and the verify phase will catch any
syntax issue the formatter would have masked.

---

## 2. Implementation Steps

Each step is tagged with a cost class: **S** (simple, cheap — single-concern, isolated change),
**M** (medium — moderate cross-file concern), **L** (complex, expensive — significant cross-cutting
or research-heavy).

---

### Step 1 — CLI scaffolding (S)

**Files**: `src/main.rs`, `src/refactor/mod.rs` (new), `src/refactor/types.rs` (new)

Add `mod refactor;` to `src/main.rs`. Add `Refactor { action: RefactorAction }` variant to
`Command`. Add `RefactorAction` and `GoRefactorAction` enums in `src/refactor/mod.rs`. Add the
`main.rs` match arm that dispatches to `refactor::run_refactor(action, ...)`.

For now the dispatch simply calls a stub that prints the plan as `{}` and exits 0. The point of
landing this first is that every subsequent step can be compiled and tested in isolation with a
`cargo check` gate from the first commit.

No behavioral change to existing subcommands. `cargo check` and `cargo test` must pass.

---

### Step 2 — Plan data structures and JSON serialization (S)

**Files**: `src/refactor/types.rs`

Implement `MovePlan`, `PlanStep`, `PlanStepKind`, `FileTransform`, `BlockingDiagnostic`,
`ApplyResult`, `VerifyResult`, `OperationKind` with full `serde::Serialize + Deserialize` derives.

Write a unit test (in `types.rs`) that round-trips a minimal `MovePlan` through
`serde_json::to_string_pretty` → `serde_json::from_str` and asserts field equality. This catches
any `#[serde(rename_all)]` / `#[serde(skip_serializing_if)]` mistakes before the transform layer
depends on these types.

---

### Step 3 — Import graph reverse-lookup (S)

**Files**: `src/import_graph.rs` (add one method), `src/refactor/mod.rs`

Add `ImportGraph::importers_of<'a>(&'a self, pkg: &str) -> impl Iterator<Item = &'a ImportEdge>` —
filters `self.edges` for `e.to == pkg`. This is the one API addition needed; no new data is stored.

Write a unit test with a hand-built `ImportGraph` asserting that `importers_of("example.com/foo")`
returns the expected edges.

The refactoring layer calls this to enumerate every file that imports the source package and thus
needs an import-path rewrite.

---

### Step 4 — Package clause rewriter (M)

**Files**: `src/refactor/pkg_clause.rs`

Input: `(source: &str, new_pkg_name: &str)` → `Result<String>` (the rewritten source).

Algorithm:
1. Parse with `tree_sitter_go::LANGUAGE` via `GrammarCache` (or a local `Parser::new()` if the
   cache is not threadsafe in this context — check `checker.rs` first).
2. Walk the root node's named children looking for `package_clause`.
3. Within `package_clause`, find the `package_identifier` child.
4. Replace the byte range `[node.start_byte()..node.end_byte()]` in `source` with `new_pkg_name`.
5. Return the modified string.

`_test` package handling: if `source` contains `package foo_test` and the destination package name
is `bar`, the rewriter must produce `package bar_test`, not `package bar`. Detect by checking
whether the original identifier ends with `_test` — if so, suffix `_test` onto `new_pkg_name`.

Golden test: two fixtures — `package foo` → `package bar`, `package foo_test` → `package bar_test`.

---

### Step 5 — Import path rewriter (M)

**Files**: `src/refactor/import_rewrite.rs`

Input: `(source: &str, old_path: &str, new_path: &str, old_alias: Option<&str>) -> Result<String>`

Algorithm:
1. Parse tree-sitter Go.
2. Walk `import_declaration` → `import_spec_list` → `import_spec` nodes (handle both grouped and
   single-line imports).
3. For each `import_spec` whose `path` child (unquoted) equals `old_path`:
   a. If the spec has an explicit alias and `old_alias` matches it: leave the alias unchanged
      (user gave an explicit alias; rewriting the path is sufficient).
   b. If the spec has no alias (implicit alias = last path segment):
      - Old last segment = `old_path.split('/').last()`.
      - New last segment = `new_path.split('/').last()`.
      - If old ≠ new: the implicit alias changes, which means `selector_expression` nodes that
        used the old alias need rewriting (handled by Step 6). The import spec itself needs no
        alias injected — the new path's last segment becomes the implicit alias automatically.
   c. Replace the `path` node's byte range with `"new_path"` (re-quoted).
4. If after all replacements the `import_declaration` has no remaining specs, remove the entire
   `import_declaration` node.

Dot import detection: if any `import_spec` for `old_path` uses `.` as alias, append a
`BlockingDiagnostic { code: "dot_import", ... }` to the plan's blockers and do not rewrite.

Golden tests: single import, grouped import, import with explicit alias, import with dot alias
(blocking diagnostic expected), import that becomes unused after move (declaration removed).

---

### Step 6 — Qualified identifier rewriter (M)

**Files**: `src/refactor/selector_rewrite.rs`

Input: `(source: &str, old_pkg_name: &str, new_pkg_name: &str, moved_symbols: &HashSet<String>) -> Result<String>`

Algorithm:
1. Parse tree-sitter Go.
2. Walk all `selector_expression` nodes.
3. For each `selector_expression`:
   - Extract the `operand` (the `identifier` on the left of `.`).
   - Extract the `field` (the identifier on the right of `.`).
   - If `operand.text == old_pkg_name` AND `field.text ∈ moved_symbols`:
     - Replace `operand` byte range with `new_pkg_name`.
4. Return rewritten source.

This is only invoked when the package's last-path-segment name changes (i.e. `old_pkg_name !=
new_pkg_name`). When the name is unchanged, import rewriting alone is sufficient. The caller
(`refactor/mod.rs`) decides this: if `old_path.split('/').last() == new_path.split('/').last()`,
skip the selector rewrite entirely.

**Critical constraint**: `moved_symbols` is populated from `ArchModel.packages[old_path].symbols`
(the `SymbolNode` list) rather than from a textual grep. This is what prevents rewriting
`oldpkg.Error` in a file that also has `otherpkg.Error` — the symbol set is package-scoped.

Golden tests: rename with symbol set, rename with empty symbol set (no rewrites), file that imports
both old and new package simultaneously (post-move, this is a conflict — produce a
`BlockingDiagnostic`).

---

### Step 7 — `internal/` visibility checker (S)

**Files**: `src/refactor/preflight.rs`

Input: `(repo_root: &Path, old_pkg: &str, new_pkg: &str, importers: &[ImportEdge]) -> Vec<BlockingDiagnostic>`

Rule: a package path containing an `internal/` segment is importable only by packages whose root
equals the `internal/` parent. Concretely: if `old_pkg` contains `internal/`, compute the allowed
root = everything before `internal/`. For each importer: if `importer.from` does not share that
prefix, emit `BlockingDiagnostic { code: "internal_visibility", ... }`.

Similarly, if `new_pkg` contains `internal/`, verify that every existing importer of `old_pkg`
will still be within the allowed root after the move.

This check runs before any tree-sitter rewriting. A non-empty result from this check causes
`MovePlan.blockers` to be non-empty; the apply gate then refuses.

Unit test: table-driven over four cases — no internal, move into allowed internal scope, move out
of allowed internal scope, move to a different internal subtree.

---

### Step 8 — Cycle detector (S)

**Files**: `src/refactor/preflight.rs`

Input: `(graph: &ImportGraph, old_pkg: &str, new_pkg: &str) -> Option<BlockingDiagnostic>`

Algorithm: after a hypothetical move, the destination package `new_pkg` would import everything
`old_pkg` used to import (its declarations move with it). Simulate this by checking whether any
package in `new_pkg`'s existing transitive dependency set also imports (directly or transitively)
any of `old_pkg`'s importers. Concretely:

1. Collect `old_pkg`'s direct importers (from `graph.importers_of(old_pkg)`).
2. From `new_pkg`, do a forward DFS via `graph.edges_from()` to get its transitive dependency set.
3. If any importer-of-`old_pkg` appears in `new_pkg`'s transitive deps → cycle would form.
4. Return `Some(BlockingDiagnostic { code: "cycle_detected", ... })`.

Unit test: use a hand-built `ImportGraph` with a known cycle-forming configuration.

---

### Step 9 — Atomicity layer (M)

**Files**: `src/refactor/atomic_apply.rs`

Implement the four-phase sequence from §1.4. Key details:

- Snapshot is `BTreeMap<PathBuf, Vec<u8>>` (bytes, not strings — binary safety).
- Temp file naming: `{new_path}.kibitzer-tmp.{pid}` (using `std::process::id()`). Cleaned up after
  rename succeeds; also cleaned up in the restore path.
- Restore loop is `for (path, content) in &snapshot { std::fs::write(path, content).ok(); }` — note
  `.ok()` to avoid masking the original error, but log each restore failure to stderr.
- New directories needed for destination paths are created via `std::fs::create_dir_all` before
  writing the temp file.
- An empty source directory after all its files have been moved is removed with
  `std::fs::remove_dir` (non-recursive; fails silently if non-empty, which is correct behavior).

Unit test (`#[cfg(test)]`): uses `tempfile::TempDir`, creates a two-file plan, injects a failure
mid-apply by making one destination unwritable, and asserts that the first file was restored to
its original content.

---

### Step 10 — Verification step (S)

**Files**: `src/refactor/verify.rs`

Implement the `run_verify(repo_root: &Path, affected_pkgs: &[String]) -> VerifyResult` function.
Each of the three commands (`go build`, `go vet`, `go test`) is run as a `std::process::Command`
from `repo_root`. stdout and stderr are captured via `output()` and concatenated. Exit status
drives the `_ok` booleans.

The `affected_pkgs` list is passed as arguments; if it contains `./...` (the caller collapsed to
wildcard), pass `./...` as a single arg. If the list is empty (no packages affected — unusual but
not impossible for a pure policy-sync run), skip all three commands and return all `_ok = true`.

Unit test: mock a `go build` that exits 0 and one that exits 1, assert `VerifyResult` fields
match. In practice: test under `#[cfg(test)]` with a tiny real Go module in `testdata/`.

---

### Step 11 — Policy sync (`sync-policy`) (M)

**Files**: `src/refactor/policy_sync.rs`

Input: `(repo_root: &Path, plan: &MovePlan) -> Result<Vec<PlanStep>>`

Algorithm:
1. Read `.kibitzer/inspect.json` at `repo_root` (if absent: no-op, return empty).
2. Parse with `serde_json::from_str` into `config::Config` (which is already serializable —
   verified from `src/config.rs`'s `#[derive(Serialize, Deserialize)]`).
3. For every `from` import path in `plan.from`: scan `config.architecture.layers[*].paths`,
   `config.architecture.deny[*].from`/`.to`, `config.checks[*].scope`, and any glob pattern that
   matches or is a prefix of `from`.
4. For each match: compute the replacement string (substitute `from` prefix for `to` prefix from
   the plan), and emit a `PlanStep { kind: PolicyUpdate, transform: FileTransform { old_path:
   ".kibitzer/inspect.json", new_path: same, new_content: the rewritten JSON string }, reason:
   "... path changed from X to Y" }`.
5. The step is emitted as part of the `MovePlan.steps` list, but only applied when `--apply` is
   passed. The caller decides whether to include it in the main plan or as a standalone `sync-policy`
   subcommand output.

`sync-policy` as a standalone subcommand accepts `--plan <file>` (a `MovePlan` JSON previously
generated by any other subcommand) and produces a `MovePlan` whose steps are only `PolicyUpdate`
steps. It does not re-run the transform layer — it re-reads the saved plan's `from`/`to` fields
and runs policy_sync on them.

---

### Step 12 — Golden-file test harness (M)

**Files**: `src/refactor/golden_tests.rs`, `testdata/refactor/` (new directory)

Golden-file harness pattern (matching `testdata/comment-quality-corpus/`):

Each fixture is a directory under `testdata/refactor/<operation>/<case-name>/`:
- `input/` — a minimal Go module tree (go.mod + source files)
- `plan.json` — expected `MovePlan` output (blockers + steps)
- `output/` — expected file tree after apply (absent for blocking cases)
- `op.json` — the operation parameters (`{ "operation": "move_files", "from": [...], "to": "..." }`)

The test runner (`golden_tests::run_all()`) iterates all fixtures, runs `plan()` with the fixture's
`op.json`, compares the result to `plan.json` (normalize paths), and (for non-blocking cases) runs
`apply()` and compares the output tree to `output/`.

Required fixtures per AC-1:

| Case | Operation | What it tests |
|---|---|---|
| `move-files/clean` | move-files | Clean success, no naming conflicts |
| `move-files/test-pkg` | move-files | `_test` package in the moved set |
| `move-files/internal-boundary` | move-files | `internal/` visibility blocking |
| `move-files/cycle` | move-files | Cycle blocking |
| `rename-pkg/basic` | rename-pkg | Repo-wide import path rewrite |
| `extract-decls/clean` | extract-decls | Basic declaration extraction |
| `extract-decls/requires-export` | extract-decls | Exported-surface change reported |
| `split-pkg/basic` | split-pkg | Composition of move-files + extract-decls |
| `sync-policy/paths-updated` | sync-policy | inspect.json path rewrite |

---

### Step 13 — Backtest runner integration (M)

**Files**: `scripts/refactor-backtest.sh` (new), `docs/go-pkg-move-ops.md` (new per AC-5/AC-6)

The backtest script follows the model of `scripts/fan-in-out-backtest.sh`:

1. For each repo in the backtest corpus (`docs/backtest-repos.md`), identify one candidate move
   (e.g. largest package by file count, or a package already flagged by `go-package-focus`).
2. Run `kibitzer refactor go move-files --src <file> --dest-pkg <new/pkg> --repo <repo>` (plan-only,
   no `--apply`).
3. Verify the plan has no unexpected blockers (record any).
4. Run with `--apply` against a temp copy of the repo.
5. Run `go build ./...` in the temp copy and record exit code.

Results are recorded in a markdown table in `docs/go-pkg-move-ops.md` under "Backtest results".

Per AC-8: at minimum one simulated move per corpus repo, with before/after `go build ./...` exit
code recorded.

---

## 3. Ordered Execution Sequence

The steps above are numbered in dependency order. The critical path is:

```
Step 1 (CLI) → Step 2 (types) → Step 3 (graph) → Steps 4–6 (tree-sitter transforms)
                                                 → Steps 7–8 (preflight) → Step 9 (atomic)
                                                                          → Step 10 (verify)
                                                                          → Step 11 (policy)
                                                                          → Step 12 (golden tests)
                                                                          → Step 13 (backtest)
```

Steps 4, 5, 6, 7, and 8 are independent of each other and can be implemented concurrently by
parallel agents. Steps 9 and 10 depend on 4–8 being complete (the atomic layer needs the transform
outputs; verify needs a working apply). Step 11 (policy) is independent of 4–8 and can be done
concurrently with them.

### Cost summary

| Step | Cost class | Rationale |
|---|---|---|
| 1. CLI scaffolding | S | Clap boilerplate, no logic |
| 2. Data structures | S | serde derives, one round-trip test |
| 3. Reverse import lookup | S | One method on existing struct |
| 4. Package clause rewriter | M | tree-sitter walk + _test suffix edge case |
| 5. Import path rewriter | M | tree-sitter walk, dot-import detection, unused import removal |
| 6. Selector rewriter | M | tree-sitter walk, alias resolution, symbol-set gating |
| 7. internal/ checker | S | String prefix comparison, table-driven test |
| 8. Cycle detector | S | DFS over existing ImportGraph |
| 9. Atomicity layer | M | Snapshot/restore, temp-rename, error propagation |
| 10. Verification step | S | Three subprocess calls, output capture |
| 11. Policy sync | M | JSON parse/rewrite, glob matching |
| 12. Golden-file harness | M | Fixture layout, path normalization, tree comparison |
| 13. Backtest integration | M | Script + corpus runs + docs |

Total: 4 × S + 8 × M + 0 × L — no single step is L because the hardest semantic problem
(type-safe reference resolution) is pre-solved by `ArchModel` and `ImportGraph`; the tree-sitter
transforms are purely syntactic.

---

## 4. Integration with Existing Infrastructure

### 4.1 ArchModel and ImportGraph consumption

`refactor/mod.rs`'s `plan()` function takes `(repo_root: &Path)`, calls
`arch_model::collect_repo_files` + `arch_model::build_model` (same call sequence as `arch_export.rs`)
to get the `ArchModel`, and calls `import_graph::build()` to get the `ImportGraph`. Both are
already available in the same binary — no new parsing infrastructure is needed.

### 4.2 GrammarCache

The tree-sitter transforms in Steps 4–6 each call `Parser::new()` and set the Go language directly
rather than going through `GrammarCache`. `GrammarCache` is thread-local and designed for the
hook/run check path; the refactoring path is single-threaded and short-lived. If `GrammarCache` is
later extended to be passed in, the refactoring layer can adopt it without a structural change.

### 4.3 No new Cargo dependencies

All required functionality is available from crates already in `Cargo.toml`:

- `tree-sitter-go = "0.25"` — already present
- `serde` / `serde_json` — already present
- `anyhow` — already present
- `clap` — already present

The `tempfile` crate (for the atomicity unit test) is the only potential new `[dev-dependencies]`
entry. Check `Cargo.toml` first; if it is already there, no change needed.

### 4.4 `docs/go-pkg-move-ops.md` (AC-5, AC-6)

This documentation file records:
- Toolchain decision: gopls evaluated but uninstalled + API unconfirmed → rejected; gomvpkg +
  gorename obsolete → rejected; rf experimental + no rollback → rejected; tree-sitter Rust chosen.
- Handled-or-diagnosed matrix for: build tags, cgo files, generated files (`// Code generated`),
  `init()`-based registration, dot imports.
- Backtest results table.

It is created as part of Step 13 (backtest integration). It is not a plan artifact — it is a
shipped `docs/` file.

---

## Adversarial Review

### Risk 1: tree-sitter byte-offset rewrites produce incorrect output when multiple edits overlap

The import rewriter (Step 5), package clause rewriter (Step 4), and selector rewriter (Step 6)
each independently rewrite byte ranges of the same source file. If they are applied sequentially
to the same source string — each operating on the output of the previous — byte offsets from the
first parse become invalid after the first edit.

**Failure scenario**: a file with both an import path change (Step 5) and a selector rename (Step
6). The selector rewriter runs on the output of the import rewriter. But the selector rewriter
re-parses the *already-rewritten* source, so it gets fresh byte offsets — **this is safe, because
each step re-parses the current source string**. The plan calls this out explicitly: "parse the
input string at the start of each rewrite step, not once at the start of the pipeline." If an
implementer caches the initial parse across steps, the offsets will be wrong and rewrites will
corrupt the file. The plan must state this constraint explicitly at the start of §Step 6, not just
in the adversarial review.

**Mitigation**: add a comment at the top of each rewrite function: "this function re-parses `source`
at its entry point; callers must pass the current source (output of the previous step), not the
original source." Add a golden test with a file that requires both import rewrite and selector
rewrite in the same pass.

### Risk 2: the verify step does not catch semantic breakage caused by a shadowing conflict

**Failure scenario**: package A exports `Error` and `Client`. Package B (destination) already
exports `Client`. Moving A's files to B causes a duplicate `Client` declaration — but `go build`
may not be the command that catches this first. If the package clause rewrite succeeds, `go build`
will report `Client redeclared in this block`.

However, if the moved file is a test file (`_test.go`), and the naming conflict is in an external
test package, `go build ./...` does not compile external test packages — only `go test ./...` does.
A naming conflict in a `_test` package is invisible to the build step and is only caught by the
test step.

**Mitigation**: add an explicit pre-flight symbol conflict check in `preflight.rs` (not currently
listed as a step). Before generating the plan, load `ArchModel.packages[new_pkg].symbols` and
`ArchModel.packages[old_pkg].symbols`, compute the intersection of exported names, and emit a
`BlockingDiagnostic { code: "symbol_conflict", ... }` for each duplicate. Add a golden test fixture
`move-files/symbol-conflict` that exercises this. This is a pre-flight gate, not solely a
post-apply gate via `go test`.

### Risk 3: the atomicity guarantee is weaker than AC-3 claims for multi-file directory creates

**Failure scenario**: `apply()` creates a new directory and writes three files into it. The second
write fails. The restore phase attempts to write the original content back to `old_path`. But the
`old_path` may have been a different location — the file was *moved*, not copied. After `old_path`
was removed (as part of the move), the restore loop tries `std::fs::write(old_path, original_bytes)`
but the original directory still exists and the file just needs to be recreated there. This works
for content-only rewrites but has a subtle gap for moved files: if the source directory was
*removed* as an empty-directory cleanup step, restoring the file requires recreating the directory
first.

**Mitigation**: the restore loop must call `std::fs::create_dir_all(old_path.parent())` before
`std::fs::write(old_path, original_bytes)` for every file in the snapshot. Since the snapshot was
built before any writes, `old_path.parent()` is always a valid directory at snapshot time, but the
cleanup step may have deleted it. The plan must specify this in §1.4's restore loop description.
Add an atomicity unit test that exercises a full directory move (create new dir, move files, clean
old dir) with a mid-apply failure injected at the cleanup step.
