# Architecture Review: go-package-focus
**Date**: 2026-10-23
**Verdict**: CONCERNS
**Kibitzer run**: `kibitzer run src/ --trigger batch` — confirmed `architecture_checks.rs` is already at 2300 lines (rust-file-size finding); no `.kibitzer/inspect.json` found in repo root.

---

## Blockers

None.

---

## Concerns

- [ ] **Story 2.1.2a — call_edges scope ambiguous**: The plan says "restricted to the target package" but `ArchModel::call_edges` contains edges where `from` and `to` span different packages. For clustering, both `from` AND `to` must be within the same target package (intra-package only). Outgoing edges to external packages would import spurious connections: two unrelated symbols that both call `"log.Printf"` would be merged via their shared external callee. The task description must state this constraint explicitly; `edge.from.starts_with(pkg_prefix) && edge.to.starts_with(pkg_prefix)` is the guard. Failure mode: nearly every package in a real repo has external calls, so an unconstrained edge set merges all symbols with any shared external callee into one cluster → checker produces no split findings on real packages.

- [ ] **Story 2.1.2a — unresolved CallEdge not filtered**: `CallEdge.resolved == false` means `to` is a raw, unresolved string (e.g. `"database.Query"`, `"fmt.Println"`). If two methods in a package both have unresolved edges whose raw `to` strings happen to match (e.g. both call a stdlib function by the same raw name), Union-Find merges them spuriously. The plan must require `edge.resolved == true` for any call edge used in clustering. Field accesses have no `resolved` field (drops-on-mismatch per `FieldAccessEdge` doc) so they are safe as-is. Failure mode: silent false-negative clusters in packages that import popular stdlib/vendor functions.

- [ ] **Story 2.1.2a — "share field access patterns" needs a two-step matrix, not stated**: The plan says "symbols that share callers or field access patterns" — but the Union-Find operates over symbol IDs, not field IDs. The implementer must build an adjacency from the field-access side as: for each field id, collect all methods that access it; for each pair of those methods, add a Union-Find edge. "Symbols that share field access patterns" is silent on this co-access matrix step. Without it an implementer may interpret "field access patterns" as "each method's own field-access set" and apply Jaccard distance (the `extract_class.rs` approach) rather than Union-Find connected components, yielding a different algorithm. The task should spell out the co-access adjacency construction.

- [ ] **Story 3.1.1a — temp file leaks on subprocess failure or panic**: The plan's `run_build_cost` steps are: create temp file → run `go build` → read temp file → delete temp file. If `go build` exits non-zero (e.g. compilation error in the target module), the plan gives no exit path that deletes the temp file. Similarly, a Rust panic between creation and deletion leaks the file permanently on Windows (open file handles block deletion). Use `tempfile::NamedTempFile` (auto-deletes on `Drop`) or a custom RAII wrapper. The plan must add explicit error handling: if `go build` returns non-zero, surface the stderr, delete the temp file, and return an error — not silently parse an empty or partial file. The `tempfile` crate is already in Rust's ecosystem and avoids platform-specific `Drop` logic.

- [ ] **Story 3.1.1 — no test strategy for go_build_cost.rs**: The plan describes no unit tests for the new module. Two paths are testable without a real Go installation: (a) the version-gate path — mock `go version` output `"go version go1.18.3 linux/amd64"` and assert the version-too-old diagnostic is returned; (b) the JSON parse-failure path — pass a malformed or empty action graph file and assert a graceful error. The actiongraph schema is undocumented and may change (`plan.md` says so), which makes parse-failure handling the most likely real-world failure mode. Without tests, this path is untested at CI time. At minimum add a `#[cfg(test)]` block with mock-output tests for `go_version()`.

- [ ] **Story 2.1.1, 2.1.3 — Go-only language guard not specified**: `GoPackageFocusChecker` is Go-only. `ArchModel::field_accesses` is Go-only for v1 (per its doc: "Go-only, same scope as call_graph_supports"), so for a non-Go repo, `field_accesses` will be empty and no clusters will form — producing zero findings silently. This is acceptable behavior, but the plan must say so explicitly rather than relying on the accidental consequence of an empty slice. Additionally, if a mixed-language repo has Go alongside other languages, non-Go packages will appear in `ArchModel::packages`; the checker should skip them (e.g. check that at least one file in the package has a `.go` extension) rather than emitting unhelpful size-only informational findings for non-Go packages.

---

## Nitpicks

- `union_find.rs` operates on a dense `0..n` integer index space (not string keys). Story 2.1.2a needs a `symbol_id → usize` index map (`HashMap<&str, usize>` + `Vec<&str>` for reverse lookup) before the Union-Find call. Trivial to implement but absent from the task description; an implementer could miss it.

- `go_version()` parsing: Go pre-release strings (`go1.19rc1`, `go1.19beta2`) and future formats won't parse cleanly with a `go1.X.Y` regex. A lenient parse — extract the major version after `go1.` and stop at the first non-digit — is safer. The `1.19rc1 >= 1.19` comparison would fail with strict version parsing.

- Hypothesis naming via "common-prefix heuristic" (Task 2.1.2b) is underspecified. What happens when symbols share no common prefix (e.g. `Encode`/`Store`, `Parse`/`Render`)? The plan says `"hypothesis — verify manually" if ambiguous` — but no threshold for "ambiguous" is given. Define a minimum common-prefix length (e.g. ≥ 3 chars) below which the note fires.

- `architecture_checks.rs` is already at 2300 lines with a kibitzer `rust-file-size` finding. Story 2.1.1b adds one line to `model_registry()` — that's fine and correctly scoped. But the debt comment in the plan ("`architecture_checks.rs` is 730+ lines") is stale; kibitzer now reports 2300. Update the plan's size citation.

- Story 1.1.1 (Tech Debt Disposition): after the `fan_in_out` fix, the comment added to `DipConcreteCouplingChecker`'s dedup (lines 681-683) should note two distinct facts: (a) the `BTreeSet` dedup is still needed for pair iteration (not redundant after the fix), but (b) the `counts` it gets from `fan_in_out` now reflect true package-level coupling rather than inflated raw-edge counts. Without (b), a future reader might assume the checker was dedup-ing to compensate for `fan_in_out`'s bug and remove the comment's rationale after the bug is fixed.

---

## Rationale notes

**Why no BLOCKER**: The design is structurally sound — `ArchModelChecker` trait, `needs_private_symbols()`, separate module placement, `whole_repo_check` wiring, and corpus-backtest gate before `default_checks()` are all correct. The fan_in_out fix is correctly sequenced as a prerequisite gate. The Union-Find clustering approach is a defensible first-pass heuristic consistent with the checker's advisory-only, high-FP-tolerance charter.

**Lens 3 (Pattern selection) — Union-Find on directed call edges**: The plan uses "symbols that share callers" (co-called-by-same-caller) as the call-edge signal, not "A calls B" directly. Co-caller clustering is a reasonable package-level signal because co-called symbols tend to serve the same client. Combined with the intra-package constraint in Concern 1, this avoids the "shared external utility = same group" false-negative problem at the cost of requiring both edges to point into the package from the same intra-package caller. The design is acceptable once Concerns 1 and 2 are addressed.

**`extract_class.rs` overlap guard** (Task 2.1.2a): Merging all methods of a struct into a super-node before clustering is the correct approach to prevent `go-package-focus` from re-detecting splits that `extract_class` already proposes at the type level. The plan's "treat all methods on a single struct as a super-node" guard is sound; it leverages the `"{package}::{type}.{method}"` symbol ID convention.

**`fan_in_out` DipConcreteCouplingChecker interaction**: Confirmed: `DipConcreteCouplingChecker` calls `fan_in_out` for `counts` (used inside `dip_finding` for instability calculation) and separately deduplicates `(from, to)` pairs for the iteration loop. After the fix, `counts` will reflect true package coupling, making `from_instability` in `dip_finding` correct. The `BTreeSet` dedup at lines 681-683 remains necessary for the loop and is not made redundant by the fix. Plan disposition is correct.
