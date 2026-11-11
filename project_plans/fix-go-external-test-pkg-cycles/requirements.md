# Requirements: fix-go-external-test-pkg-cycles

## Problem

`kibitzer check architecture import-cycles` reports false-positive import cycles on Go
repos that follow the standard external-test-package pattern:

```
example.com/m/widget/            package widget
example.com/m/widget/widgettest/ package widgettest   // imports example.com/m/widget
example.com/m/widget/widget_test.go   package widget_test   // imports example.com/m/widget/widgettest
```

Kibitzer reports:
```
widgettest/helper.go:N: [import-cycle] import cycle:
  example.com/m/widget/widgettest -> example.com/m/widget -> example.com/m/widget/widgettest
```

This is not a real cycle. Go's external test package (`package widget_test`) is a
*separate compilation unit* from `package widget`; it may legally import packages that
import `widget`. `go vet` and the Go compiler accept this without error.

## Root Cause

`go_package_identity` in `src/import_graph.rs` derives a file's graph node key from
its directory path + the nearest `go.mod`, **ignoring** the `package` clause entirely.
Both `widget.go` (`package widget`) and `widget_test.go` (`package widget_test`) live in
the same directory and therefore receive the same graph key (`example.com/m/widget`).
When `build_qualified_name_language` processes `widget_test.go`, its imports of
`example.com/m/widget/widgettest` are attributed to the `example.com/m/widget` node,
creating the spurious `widget -> widgettest` edge that closes the false cycle.

## Acceptance Criteria

1. **AC-1 — no false cycle on the canonical X / Xtest / X_test triple**  
   Given a module with `package widget`, `package widgettest` (helper that imports
   `widget`), and `package widget_test` (external test file that imports `widgettest`),
   `import-cycles` reports zero findings.

2. **AC-2 — real cycles still reported**  
   Two regular (non-test) packages that mutually import each other continue to produce
   an `[import-cycle]` finding.

3. **AC-3 — external test file edges excluded from cycle graph**  
   Files whose parsed `package` clause ends with `_test` are excluded from the import
   graph's node/edge set so their imports cannot create false cycle edges. The exclusion
   is per-file, not per-directory.

4. **AC-4 — internal test files (`package widget` in `*_test.go`) unaffected**  
   Internal test files (same package name, `*_test.go` suffix) do not declare a
   `_test`-suffixed package and must continue to be treated as part of the package they
   test. Kibitzer's existing behavior for these files is preserved.

5. **AC-5 — coupling and layering checkers unaffected**  
   Because external test edges carry little architectural signal (they are intentionally
   allowed to form these "pseudo-cycles"), it is acceptable for coupling and layering
   checkers to also exclude external test files from the graph, reducing noise there too.
   No regression in those checkers' existing tests.

6. **AC-6 — regression test added**  
   A new unit test in `src/import_graph.rs` covering the three-package layout above
   asserts that `build()` produces no edge from `example.com/m/widget` to the helper
   package.

7. **AC-7 — backtest passes on backtest-repos corpus**  
   After the fix, running `import-cycles` on the standard backtest corpus
   (`docs/backtest-repos.md`) produces no newly introduced false positives or
   regressions in real-cycle detection.

## Out of Scope

- Tracking external-test-package edges separately for a future "test coupling" checker
  (desirable but a separate feature; this fix focuses on eliminating false positives).
- Applying the same exclusion to other languages (no equivalent concept in Java/Kotlin/JS).
- Changing the `layering` or `coupling` checkers directly — the fix is upstream in
  graph construction.
