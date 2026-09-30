# `duplicate-code-fuzzy` — opt-in near-duplicate check

`duplicate-code` (on by default) only flags blocks that repeat *verbatim*.
`duplicate-code-fuzzy` (issue #47) extends the same window-hashing approach
with a normalization pass: every string/char/numeric literal in a window is
replaced with a placeholder before hashing, so a block that's copy-pasted
except for one literal — e.g. `getUsersByStatus("active")` and
`getUsersByStatus("pending")` with otherwise-identical bodies — still groups
together. Once a group has 3+ occurrences whose literals actually differ,
it's flagged as a **Parameterize Function** candidate, and the finding names
the differing literal value(s) so the suggested parameter is obvious.

It's opt-in rather than a default, the same way the backtested prose checks
in `docs/prose-checks.md` are: unlike exact duplication, "these blocks differ
only by a literal" is a judgment call about whether that literal is worth
promoting to a parameter (a table-driven-test-style literal difference isn't
the same signal as a genuinely copy-pasted function).

## Backtest (kubernetes/kubernetes)

Ran the built release binary against every `.go`/`.rs`/`.ts`/`.java` file in
`kubernetes/kubernetes` (`docs/backtest-repos.md`). Before excluding test
files, ~97% of the ~1960 raw findings (by filename: `validation_test.go`,
`helpers_test.go`, `describe_test.go`, ...) landed in `_test.go` table-driven
test cases — those are *already* the shared-body-plus-varying-literal shape
Parameterize Function recommends moving toward, so "extract a shared
function" is backwards advice there, not a genuine catch. Fixed by adding
`is_test_file` (checks `_test.go`/`_test.py`/`test_*.py`/`.spec.*`/`.test.*`/
`*Test(s).java`/`*Test(s).kt` filename conventions), mirroring the same
`_test.go` carve-out `complexity.rs` already applies for a different reason.

The remaining non-test-file findings were genuine near-duplicates, though a
few (Go blank-import registration blocks differing only by import path, e.g.
`cmd/kube-controller-manager/app/import_known_versions.go`) have a
technically-correct match but a nonsensical remediation — an `import`
declaration can't be "extracted into a shared function." Not fixed here
(would need to recognize import-only windows specifically); worth keeping in
mind when triaging this checker's findings in a real repo.

Re-running after adding `is_test_file` dropped the finding count sharply, but
two smaller residual noise sources remain, both pre-existing/shared rather
than introduced here: (1) `is_generated`'s first-20-lines window misses files
whose Apache license header pushes the actual `// Code generated ... DO NOT
EDIT.` marker further down (e.g. `api.pb.go`) — the same gap `duplicate-code`
itself has; (2) Go test-support files that don't follow the strict
`_test.go` suffix (`allocator_testing.go`, `store_tests.go`, `testcase.go`)
still carry table-driven-style literal variation. Neither is fixed here.

## Backtest (real transcripts, `kibitzer check backtest`)

Also ran the mandatory transcript-based leg (`docs/backtesting.md`) against
this machine's own `~/.claude/projects` history, per this repo's "both
required, not either/or" policy for landing a new checker.
`kibitzer check backtest duplicate-code-fuzzy --only-new` started at 155
findings; almost all of the worst offenders were this checker's own blind
spot for *Rust*, whose test-file conventions don't match the filename-suffix
heuristic `is_test_file` was built around:

- **Inline `#[cfg(test)]` modules.** Unlike every other supported language,
  Rust keeps test code in the same file as production code
  (`#[cfg(test)] mod tests { ... }`), so there's no separate filename to
  exclude. Fixed by truncating the scanned line range at the first
  `#[cfg(test)]` line (`trimmed_lines_before_rust_test_module`) — this
  checker's own table-driven fixture arrays (`src/java_swallowed_interrupt.rs`
  and siblings in this very repo) were the single largest false-positive
  source before the fix.
- **`tests/` integration-test directories.** Rust integration tests
  (`crates/*/tests/*.rs`) and some JS/TS suites (`__tests__/`) live in a
  directory by convention, not a filename suffix. Confirmed via a real hit
  (`crates/cli/tests/browser_session.rs`, from another of Tyler's repos) and
  fixed by adding a path-component check to `is_test_file`.

After both fixes and clearing the stale `~/.cache/kibitzer/backtest-cache.json`
(the cache keys on checker *name*, not code version, so a local rebuild
doesn't invalidate it — re-run with a fresh cache after any checker-logic
change), the same command dropped to **84 findings**. The remainder is a mix
of genuine near-duplicates worth flagging (a CLI subcommand-registration
table in `landing.rs`, a Go workflow-state struct literal in
`create_workflow.go`) and lower-value hits on dense literal arrays/word
lists (`comment_quality.rs`'s word list, and even this checker's own
`is_test_file` extension-suffix array) — the same "arrays of short literals
window-match each other" limitation the exact-match `duplicate-code` checker
already has, not something newly introduced here.

Enable it via `.kibitzer/inspect.json` (see `docs/suppressing-checks.md`):

```json
{
  "checks": [
    {
      "name": "duplicate-code-fuzzy",
      "checker": "duplicate-code-fuzzy",
      "severity": "advisory",
      "scope": ["**/*.go", "**/*.ts", "**/*.py", "**/*.java", "**/*.rs"]
    }
  ]
}
```

## Scope

This is the "Parameterize Function" half of #47's proposal (literal
normalization producing a fuzzy match). The other half — matching an inline
block against a *named function's* body specifically, so the finding can say
"this matches `computeFoo()` — call it instead" (Replace Inline Code with
Function Call) — is not implemented here; it needs the checker to resolve a
duplicate window against a function's actual boundaries (via the AST), not
line-window text, and is left as a follow-up.
