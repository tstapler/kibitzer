# `affected` real-world validation

Task 4.1.3 (`project_plans/affected-tests/implementation/plan.md`): comparing
`kibitzer architecture affected` against stapler-squad's own
`scripts/test-affected.py` on real historical stapler-squad diffs — the "run against
real signal before shipping" bar this repo's CLAUDE.md applies in place of `kibitzer
check backtest` for a feature that isn't a per-file checker.

## Method

Ten commits were selected from `~/Programming/stapler-squad`'s real history
(2026-09, the `fix/session-notification-cleanup-hidden` branch tip and its
ancestors), spanning every category `test-affected.py`'s design distinguishes:

| # | SHA | Category | Subject |
|---|-----|----------|---------|
| 1 | `b08607e77` | normal Go-only | fix(session): event-driven cleanup for archived/hidden session notifications |
| 2 | `eaae1d9f8` | normal Go-only (found the bug below) | fix(terminal): capture doneCh/stdout at control-mode spawn time |
| 3 | `0b664a9e0` | normal Go-only | fix(session): eliminate Instance.Status data race in hibernate/resume |
| 4 | `ca02f0121` | normal Go-only | fix(insights): attribute cost to sessions/backlog items |
| 5 | `59e5ef38a` | normal Go-only | fix(scroll): allowlist idle-equivalent statuses |
| 6 | `8e5bfdb56` | `go.mod` change | feat(agy): full Antigravity CLI & Gemini rule-enforcement hook support |
| 7 | `57e85c635` | `go.mod` change | feat(hostname-detect): periodic + network-change LAN hostname re-detection |
| 8 | `c7229dc5f` | rename (`.go` test files) + non-Go (TS/React) changes | fix(async-session-creation): close /sdd:6-verify review findings |
| 9 | `65edb5c73` | delete-heavy, `.proto` change | feat(git)!: remove native_git_worktree/native_git_merge feature flags |
| 10 | `78db27678` | merge commit | Merge remote-tracking branch 'origin/main' |

For each commit `sha`, both tools were run with `sha^1` (or `sha^` for a non-merge)
as base, from a **separate `git worktree add --detach` checkout** of stapler-squad
per commit (never the user's actual checkout/branch — that stays untouched):

```
kibitzer architecture affected --path <worktree> --base <parent-sha>
python3 <worktree>/scripts/test-affected.py <parent-sha>   # run with cwd=<worktree>
```

**Environment caveat**: `test-affected.py` shells out to `go list -json -test ./...`,
which needs stapler-squad's gitignored generated code
(`server/web/dist/` frontend build, `gen/` protobuf output, `session/ent/*.go` ent
codegen) to even parse. Each worktree had **today's** generated output for these
copied in from the user's real checkout, rather than regenerating it per historical
commit — a deliberate approximation (regenerating per-commit would need each
commit's own toolchain state) that only matters for a commit whose diff itself
touches codegen inputs, which none of the ten selected commits' Go-package-set
outcome depended on. `scripts/test-affected.py` itself didn't exist before commit
`6c3f3b69d` (#704); for `c7229dc5f` (which predates it), the script was invoked from
**outside** the worktree (`python3 <scratch>/test-affected.py <parent>`, `cwd` still
the worktree) so it never showed up as an untracked file inside the worktree's own
diff — an earlier attempt that copied the script *into* the worktree pollutes
`test-affected.py`'s own `all_changed` set with itself, spuriously matching its
`FULL_RESCAN_TRIGGERS` entry for `scripts/test-affected.py` and forcing a false
`__ALL__` that has nothing to do with the real commit.

## Bug found and fixed during this exercise

**Commit `eaae1d9f8`** (a normal Go-only fix, deleting a resolved bug's tracking doc
`docs/bugs/open/BUG-086-tmux-control-mode-refcounting-race-under-concurrent-start-stop.md`
alongside its Go fix) initially produced a **divergence**: kibitzer returned
`__ALL__` (`BailOutReason::PackageFullyRemoved`) while `test-affected.py` returned a
concrete 29-package list. Root cause: `compute_affected`'s deleted/renamed-old
handling ran `resolve_removed_path` for **every** `Deleted`/`Renamed{old}` entry
regardless of file extension. A deleted Markdown file in a Go-free docs directory
was never part of a Go package to begin with, so `resolve_removed_path` correctly
found no surviving `.go` file at that path — and *incorrectly* concluded the package
was "fully removed," when there never was one. In a repo with any Go-free directory
(docs, config, fixtures — completely ordinary), this bug would have made `affected`
spuriously bail out to `__ALL__` on routine non-Go deletes, defeating much of the
feature's point.

**Fixed** in `src/affected.rs::compute_affected`: a `Deleted`/`Renamed{old}` entry
now only reaches `resolve_removed_path` when its path has a `.go` extension,
matching this project's Go-only v1 scope. Regression test:
`compute_affected_does_not_bail_out_when_a_non_go_file_is_deleted_from_a_go_free_directory`
(`src/affected.rs`). Re-run against the same commit after the fix: **no divergence**
(see table below).

A second, related fix landed alongside it, also surfaced by this exercise:
`compute_affected` originally built the `ArchModel` over **every** language
`walk_and_collect_files` found (Go, TS/JS, Python, Java, Kotlin, Rust), not just Go —
in stapler-squad's polyglot tree this produced JS/TS package "keys" in `affected`'s
stdout that are **absolute filesystem paths** (`import_graph.rs`'s `dir_key` for
JS/TS, unlike Go's module-qualified import-path keys) — tokens a CI wrapper piping
straight into `go test $PKGS` could choke on, and a direct violation of this
project's explicit "v1 ships Go support only" scope (`requirements.md`, AC #10).
Fixed by scoping the model build to `.go` files only before ever calling
`arch_model::build_model_from_files` — this also incidentally addresses most of the
per-invocation wall-clock cost (see Performance below), since a large fraction of
what was being tree-sitter-parsed on every call was never Go code the graph walk
could even use.

## Results

Every one of the ten commits, after both fixes, is **never narrower** than
`test-affected.py` — the invariant this whole feature exists to guarantee. Every
divergence is kibitzer being *broader* (safe direction), for two well-understood,
expected reasons: (1) `test-affected.py` filters its output to packages that declare
at least one `_test.go` file (`has_tests`); `affected` deliberately doesn't apply
that filter — a changed package with no tests yet still belongs in the answer if a
future test is added there. (2) `affected`'s Go-package resolution reaches a few
kibitzer-repo-irrelevant paths `go list ./...` also enumerates differently
(`session/ent/schema` as its own package vs. folded into `session/ent`,
`tools/lint/silenttransition/testdata/src/a` — literal lint-checker test fixture
source, not a real target).

| # | SHA | kibitzer result | test-affected.py result | Verdict |
|---|-----|-----------------|--------------------------|---------|
| 1 | `b08607e77` | 30 packages | 20 packages | Match — kb ⊇ py (has_tests filter + ent/testdata extras) |
| 2 | `eaae1d9f8` | 41 packages (after fix; was `__ALL__` before) | 29 packages | Match after fix — kb ⊇ py |
| 3 | `0b664a9e0` | 30 packages | 20 packages | Match — kb ⊇ py |
| 4 | `ca02f0121` | 32 packages | 23 packages | Match — kb ⊇ py |
| 5 | `59e5ef38a` | 30 packages | 20 packages | Match — kb ⊇ py |
| 6 | `8e5bfdb56` | `__ALL__` (`**/go.mod`) | `__ALL__` | Exact match, same trigger category |
| 7 | `57e85c635` | `__ALL__` (`**/go.mod`) | `__ALL__` | Exact match, same trigger category |
| 8 | `c7229dc5f` | 30 packages | 6 packages | Match — kb ⊇ py (kb also resolves the `.go` test-file renames correctly, no spurious bail-out) |
| 9 | `65edb5c73` | `__ALL__` (`**/*.proto`) | `__ALL__` | Exact match, same trigger category |
| 10 | `78db27678` | 0 packages (empty, merge introduced no net diff) | 0 packages | Match — both empty |

Zero commits show a `test-affected.py`-only package absent from kibitzer's output
(the dangerous direction) across all ten, both before and after the one real
divergence above was fixed.

## Performance

| # | SHA | kibitzer (release build) | test-affected.py |
|---|-----|---------------------------|-------------------|
| 1 | `b08607e77` | 7.53s | 2.05s |
| 2 | `eaae1d9f8` | 8.03s | 1.80s |
| 3 | `0b664a9e0` | 7.46s | 1.66s |
| 4 | `ca02f0121` | 7.71s | 1.86s |
| 5 | `59e5ef38a` | 7.95s | 1.71s |
| 6 | `8e5bfdb56` (bail-out, glob-matched before any graph work) | 1.04s | 0.71s |
| 7 | `57e85c635` (bail-out) | 0.84s | 0.72s |
| 8 | `c7229dc5f` | 6.63s | 1.72s |
| 9 | `65edb5c73` (bail-out) | 1.08s | 0.69s |
| 10 | `78db27678` (empty result) | 8.44s | 0.89s |

**Verdict: does not meet the "not slower than `test-affected.py`" NFR** for the
non-bail-out case — kibitzer is **roughly 4–5x slower** (7–8.5s vs. 1.7–2s) on
stapler-squad's current size (~2,085 real `.go` files, verified via `find`). The
bail-out fast path (glob match short-circuits before any `ArchModel` build) is
comparable to `test-affected.py`'s own fast path (both ~0.7–1.1s, dominated by
process/git overhead).

Root cause, confirmed rather than assumed: `compute_affected` rebuilds a fresh
`ArchModel` (full tree-sitter parse of every `.go` file) on **every invocation**,
with no incremental/daemon-cache reuse — exactly pre-mortem Failure Mode #5's
prediction, now measured. `test-affected.py`'s `go list -json -test ./...` benefits
from Go's own incremental build cache, which kibitzer's design has no equivalent
of.

This is an **accepted risk for v1**, per `plan.md`'s Risk Control section and the
pre-mortem's P2 disposition — not a blocker, since (a) the bail-out fast path (the
overwhelming majority of `go.mod`/lockfile/proto changes, which are common enough in
practice) is not slowed down at all, and (b) the daemon-cache-backed incremental
recomputation that would close this gap is explicitly out of scope for v1
(`requirements.md`, AC #10).

**Revisit trigger** (per pre-mortem Failure Mode #5's follow-up): re-run this
comparison if stapler-squad's tracked `.go` file count grows past ~3,000 (a ~50%
increase from today's ~2,085), or if a single non-bail-out `affected` invocation in
CI is observed to exceed 15 seconds. If either fires, pull the daemon-cache-reuse
follow-up (out of scope here) back into consideration rather than treating this
measurement as permanent proof the NFR is met.

## Multi-`go.mod`/`replace`-directive fixture (Task 4.1.3a)

A deliberately constructed synthetic fixture (not sampled from real history — see
`src/affected.rs::compute_affected_multi_module_fixture_with_replace_directive_finds_the_dependent_package`)
mirrors stapler-squad's real multi-module layout (root + `tuitest/go.mod` +
`tools/scanner/go.mod` + `tools/lint/go.mod`, with the root module's `go.mod`
`replace`-ing `tools/scanner` to a local path). Finding: `affected` resolves this
correctly — `import_graph.rs`'s `find_go_mod_upward` never reads `replace`
directives at all, but doesn't need to here, since edge resolution matches on the
*literal import path string*, and the `replace`d module's own `go.mod` declares that
same path. The `replace`-directive gap named in the plan's Tech Debt Disposition
would only bite if a `replace` pointed an import path at a module whose own declared
`module` name differs from the string written at the import site — not exercised
here, still an accepted, documented risk.

## Empty affected-set / CI wrapper guard

`run_affected` prints pure-empty stdout (no sentinel, no trailing newline) for
`AffectedResult::Packages(vec![])`, with a distinct, greppable
`AFFECTED: 0 packages — verify your CI wrapper guards against empty output` note on
stderr. Any CI wrapper composing this into `go test $(...)` **must** guard with
`[ -z "$PKGS" ] && exit 0` before invoking `go test $PKGS` — an unquoted, unguarded
`$PKGS` word-splits to nothing and `go test` silently falls back to testing whatever
package `cwd` resolves to, masking an intentional "nothing affected" skip as a
narrow, misleading pass. This exact shell shape is exercised by
`tests/affected_cli.rs::empty_affected_output_reaches_the_ci_wrapper_guard_branch`,
and the guard requirement is also stated directly in
`kibitzer architecture affected --help`'s doc comment (`src/main.rs`), not only
here.
