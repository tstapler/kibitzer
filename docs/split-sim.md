# split-sim: what-if for Go package splits

`kibitzer architecture split-sim <package> --partition NAME=f1.go,f2.go --partition NAME2=... [--path DIR] [--format json|text]`
and the MCP tool `split_sim` answer "is this split feasible, and what would it change?" without editing
anything. Go only. Advisory: exits 0 even for an infeasible split, and abstains with a reason when the
input can't be analysed (unknown package, unassigned files, fewer than two parts).

Reports: import cycles (production vs test-only warning), the exported-surface delta, callers to
rewrite, blockers (`init()`, build tags/platform filename suffixes, cgo, shared unexported fields or
package-level locks, methods separated from their receiver), warnings for generated files, distinct-package
before/after reverse-dependency counts per part, and coverage diagnostics. Sharing through parameters,
interface fields or unexported types is not detected; identifier matches for package-level vars/consts are
syntactic, so strings, comments and shadowing locals can add spurious references. See also
`go-package-focus` (`docs/go-package-focus.md`), which answers *should* a package be split.

`path` is the Go module root (the MCP tool does not walk up to the enclosing repo).

## Verification

Ground truth: the real split is performed in a sibling fixture (`testdata/split-sim/*-after`), and
`kibitzer architecture affected`'s reverse-BFS is run on it; the tests
`reverse_dep_counts_match_affected_after_the_real_split_*` in `src/split_sim/tests.rs` assert equality.

| Fixture | Before (direct/transitive) | After, part 1 | After, part 2 |
|---|---|---|---|
| `legal-split` | 2 / 3 | P1: 1 / 2 | P2: 2 / 4 |
| `fanout-split` | 3 / 4 | P1: 2 / 3 | P2: 1 / 2 |

Note: sibling parts count as reverse dependencies of each other when one uses the other, which is why
`legal-split` P2 shows 4 transitive against 3 before.

## Backtest

Real package: `github.com/tstapler/stapler-squad/session` (166 `.go` files), split `backlog*.go` vs the rest,
release build, `--path` at the module root: 7.2 s wall clock including the `include_private: true` model build.
Result: not feasible; 1 production cycle, 22 blockers (platform-tagged files, `init()` in `state_machine.go:68`,
methods separated from receivers, `Storage.repo` accessed across parts, a package-level lock), 1 generated-file
warning. The findings match what a real split of that package would hit. Only this one package was run;
the larger corpus repos (`docs/backtest-repos.md`) have not been run.
