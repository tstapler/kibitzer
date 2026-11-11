# Requirements: import-cycles-xtest-false-positive

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

Mirrors the backlog item's 8 criteria.

1. Canonical `widget` / `widgettest` / `widget_test` layout: `import-cycles` reports zero findings.
2. Two regular packages that import each other still produce `[import-cycle]`.
3. Files whose `package` clause ends in `_test` cannot create cycle edges. **Implementation
   deviation:** the files are not dropped from `graph.nodes`/`file_packages` (the refactor
   planner needs `a_test.go` grouped with `a`; dropping them failed 2 existing
   `refactor::tests`). Only their *outgoing edges* are skipped (`skips_edges` in
   `QualifiedImportLangConfig`). The false edge is absent from the graph either way.
4. Internal test files (`package widget` in `widget_test.go`) keep contributing edges.
5. All existing tests pass (`cargo test`: 2575 passed in the bin target).
6. Regression test `go_external_test_package_does_not_create_false_cycle`.
7. Companion test `go_internal_test_file_still_appears_in_graph`.
8. Corpus backtest: no new findings, no lost real cycles (kubernetes, stapler-squad).

## Out of Scope

- Tracking external-test-package edges separately for a future "test coupling" checker
  (desirable but a separate feature; this fix focuses on eliminating false positives).
- Applying the same exclusion to other languages (no equivalent concept in Java/Kotlin/JS).
- Changing the `layering` or `coupling` checkers directly — the fix is upstream in
  graph construction.
