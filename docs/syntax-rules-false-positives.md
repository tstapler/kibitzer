# `syntax-rules` check — known false positives

Tracks suspected false-positive firings of the native `syntax-rules` check
(`src/rules.rs`; the Go flavor runs as `syntax-rules`, other languages as
`syntax-rules-<lang>`). Check new occurrences against this list before
re-investigating a firing from scratch. Reporting convention:
`docs/reporting-false-positives.md`.

## Log

### 2026-10-05 — private Go service repo — `long-function` flags a function that only returns a flat table-test case list
- **Repo**: private Go service (name withheld), file `pkg/<name>_test.go`.
- **What changed**: a table-driven test was split so the case table lives in its own
  helper, `func generateCases(t *testing.T) []testCase { return []testCase{ {...}, ... } }`.
  The helper's body is one `return` of a composite literal with about a dozen entries,
  75 lines in total. It has no branches, loops, or calls beyond building each entry.
- **Why it's a false positive**: `long-function` exists to prompt *Extract Function* on
  code that does too much. A function that is a single flat data literal has nothing to
  extract, and splitting it only scatters the cases. The edit that created it was already
  the extraction; the finding then re-fired on the Stop hook's unscoped re-check of the
  file. Arguably this is a precision limit more than a misfire, so treat it as a request
  to consider a data-literal exemption, not a bug in the line count.
- **Mechanism**: `src/rules.rs::check_declaration` computes
  `body.end_position().row - body.start_position().row + 1` and flags it when it exceeds
  `LONG_FUNCTION_LINES` (40). It looks only at line count. The sibling `deep-nesting`
  rule in the same function walks control flow, but `long-function` never inspects what
  the body contains, and I found no test-file (`_test.go`) handling in `src/rules.rs` (by
  `grep`, not by running every path).
- **Reproduction** (run with the installed binary, 2026-10-05):

  ```go
  package demo

  type tc struct {
  	name  string
  	input int
  	want  int
  }

  // cases only returns a flat table: no branches, loops or calls.
  func cases() []tc {
  	return []tc{
  		{name: "case 1", input: 1, want: 2},
  		// ... 45 rows in total, one line each ...
  	}
  }
  ```

  `kibitzer check native syntax-rules table_test.go` prints
  `table_test.go:10: [long-function] body spans 49 lines (over 40) ...` and exits 1.
- **Related, not traced**: in the same file `replace-magic-literal` and `long-function`
  fired on the table-driven test function itself. `docs/syntax-rules.md` already notes
  that `replace-magic-literal` is prone to table-test hits, so I did not file that one.
