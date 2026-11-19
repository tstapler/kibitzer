# Implementation Plan: type-switch-clustering

**Date**: 2026-11-04

## Story map

### Story 1 — Extract helpers (0.5 day)

**1.1** Make `type_case_count` and `node_text` in `go_type_switch_density.rs` `pub(crate)`
(or move them to a small `src/checkers/go_type_switch_helpers.rs` module) so the
clustering checker can reuse them without duplication.

**1.2** Add a new `GoTypeSwitchSite` struct (private to `go_type_switch_clustering.rs`):
```
struct GoTypeSwitchSite {
    owner_type: String,   // e.g. "MyStruct"
    method_name: String,  // e.g. "Draw"
    subject: String,      // e.g. "r.kind" (raw text)
    case_count: usize,
    file: PathBuf,
    line: usize,
}
```

### Story 2 — Per-file site extraction (1 day)

**2.1** Write `extract_type_switch_sites(file: &Path, source: &[u8], tree: &Tree) -> Vec<GoTypeSwitchSite>`:
- Walk `method_declaration` nodes (Go grammar: `method_declaration`).
- For each: extract receiver type name (strip `*`/`&` prefix), method name.
- For each `type_switch_statement` child (anywhere in the body): extract subject text,
  `type_case_count`. Record a `GoTypeSwitchSite`.
- Skip generated files (reuse `is_generated`).

**2.2** Unit tests (in-module) covering:
- Two methods on same type, same subject → 2 sites extracted.
- One method → 1 site (below cluster threshold, but correctly extracted).
- Different receiver names, same field (`r.kind` / `s.kind`) → 2 sites, subject differs.
- Generated file → 0 sites.

### Story 3 — ArchModel-level clustering checker (1.5 days)

**3.1** New file `src/checkers/go_type_switch_clustering.rs` implementing the
`ArchModelChecker` trait (as used in `src/architecture_checks.rs`).

**3.2** `check` body:
1. Walk all `.go` files in the repo (reuse `arch_model::collect_repo_files` or
   equivalent file enumeration).
2. Call `extract_type_switch_sites` per file.
3. Group sites by `(owner_type, subject)`.
4. Filter groups to `len ≥ 2`.
5. For each group: look up `model.type_edges` for the switched-on interface. Heuristic:
   take the unique case type names across the group's switches; if any `TypeRelationEdge`'s
   `from` or `to` matches one of those names, a hierarchy exists → "Replace Conditional
   with Polymorphism". Otherwise → "Replace Type Code with Subclasses".
6. Emit one `Finding` per cluster at the first method's line.

**Finding message format** (advisory):
```
[go-type-switch-clustering] `{owner_type}` switches on `{subject}` in {n} methods
({method_list}) — a type-code switch repeated across methods is a {recommendation}
candidate (Fowler: https://refactoring.com/catalog/{slug}.html)
```

**3.3** Register with `inventory::submit!` (matches `go_type_switch_density.rs` pattern).

**3.4** Register in `default_checks()` in `src/config.rs` as:
```rust
architecture_check("go-type-switch-clustering", Severity::Advisory, &["**/*.go"]),
```
(or equivalent native architecture check registration — verify the exact macro/helper
used by other `ArchModelChecker`s in `config.rs`.)

### Story 4 — Integration tests (1 day)

**4.1** Multi-file fixture test (new test in `tests/` or `src/checkers/`) with:
- Two `.go` files, same package, same type with 2 methods each containing a type switch
  on the same subject → cluster fires.
- Same but only 1 method → no cluster.
- Two methods, different subjects (`r.kind` vs. `r.shape`) → no cluster.
- Hierarchy-present fixture: `type_edges` has an edge from one of the case types → message
  says "Replace Conditional with Polymorphism".
- No hierarchy fixture → message says "Replace Type Code with Subclasses".

**4.2** The existing `default_checks_ids_are_unique` test in `config.rs` will catch a
duplicate name registration automatically (run `cargo test` after Story 3).

### Story 5 — Backtest and false-positive doc (0.5 day)

**5.1** Run `kibitzer check backtest go-type-switch-clustering` against Claude Code
transcripts.

**5.2** Run the checker against the `docs/backtest-repos.md` corpus
(`kubernetes/kubernetes`, `denoland/deno`, `BurntSushi/ripgrep` — the Go-applicable
subset). Record TP/FP counts; adjust threshold or subject-normalization if FP rate
is high.

**5.3** Create `docs/go-type-switch-clustering-false-positives.md` with:
- Corpus results table.
- Known FP patterns and suppression guidance.

## Decision log

| Decision | Rationale |
|---|---|
| Option B (re-walk files in ArchModelChecker) over Option A (pre-store in ArchModel) | No schema change; `batch`-only granularity means extra walk cost is acceptable; matches `change_coupling.rs` precedent |
| Conservative exact subject-string matching | Avoids false clusters from different-receiver-name same-field patterns; v1; document limitation |
| Hierarchy check via case-type name scan of `type_edges` | Direct `type_edges` lookup of the subject expression's type would require full type resolution; case names are already available in the AST and are what a subtype hierarchy would contain |
| `default_checks()` advisory (not blocking) | Consistent with `go-type-switch-density`; polymorphism refactorings are significant changes, not quick fixes |

## Risk log

| Risk | Mitigation |
|---|---|
| High FP rate from subject-string mismatch across receiver names | Conservative matching + backtest gate; FP suppression via `kibitzer:ignore` |
| `ArchModelChecker` API for file enumeration may differ from `Checker` API | Read `architecture_checks.rs` before writing Story 3.1 to confirm file-walk API |
| `default_checks()` count test (`default_checks_ids_are_unique`) will fail if name collides | Unique name `go-type-switch-clustering` avoids this; test will catch it immediately |
