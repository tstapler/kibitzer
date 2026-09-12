# `duplicate-code` check — known false positives

Tracks confirmed false-positive firings of the `duplicate-code` kibitzer check
(`src/duplicate_code.rs`), wired up via `kibitzer check native duplicate-code
{file}`. Check new occurrences against this list before re-investigating a
firing from scratch.

## Mechanism (confirmed by reading the source)

The check slides a 6-line window (`MIN_BLOCK_LINES`) over whitespace-trimmed
lines within a single file, requires the window to avoid blank lines and have
at least 60 total characters (`MIN_BLOCK_CHARS`), and fires once a window's
exact text repeats at least 3 times (`MIN_OCCURRENCES`) in that file. It has no
generated-code detection (no check for `// Code generated ... DO NOT EDIT`) and
no path-based exclusion for `vendor/`, `zz_generated.*`, `*.pb.go`, or similar —
`file_globs()` is a bare set of language extension globs with no directory
filtering.

## Fixed

### 2026-09-12 — kubernetes/kubernetes + stapler-squad corpus backtest — no generated/vendored-file exclusion

- **Symptom**: `generated.pb.go` (kubernetes/kubernetes, protoc-gen-gogo's `MarshalToSizedBuffer` boilerplate,
  mechanically repeated per message type) and `gen/proto/go/session/v1/session.pb.go` (stapler-squad,
  protoc-gen-go's `ProtoReflect()`, 251 occurrences in one file) both flagged — nobody hand-edits generated
  protobuf/ORM code to deduplicate its mechanically repeated boilerplate. Other generated/vendored files showing
  the same pattern in the same sweep: `cmd/kubeadm/app/apis/kubeadm/v1/zz_generated.deepcopy.go`,
  `vendor/k8s.io/kube-openapi/pkg/validation/spec/gnostic.go`, `session/ent/session_query.go`,
  `session/ent/session/where.go` (ent ORM codegen).
- **Mechanism**: `duplicate_code.rs`'s window-hashing logic had no generated/vendored-file exclusion of its own —
  every match found was a genuine verbatim repeat (no coincidental/boilerplate-license-header false matches), the
  problem was scope (which files get scanned), not the matching logic.
- **Fixed by**: an early return via `file_size::is_generated` (reused, not reinvented — already `pub` and used by
  `file_size.rs`'s own checker), checking the first 20 lines for a `// Code generated ... DO NOT EDIT.` marker.
  Regression-guarded by `does_not_flag_duplication_in_a_generated_file` (`src/duplicate_code.rs`'s test module).
  Note: `find_cross_file_duplicates` (`kibitzer check duplicates`, the separate cross-file command) has the same
  theoretical exposure if invoked directly against generated files — not covered by this fix, since it wasn't
  part of the confirmed false-positive log entry above. Verified directly: both cited files now produce no
  findings, while a genuine true positive (`config/config.go:745`, stapler-squad, three functions sharing
  identical `~/`-expansion logic) still flags.

## Log

No open entries.

## Needs discussion (not false positives, logged here for context — not promoted per this doc's convention)

A large share of sampled findings in both repos (roughly 60%) are genuine, verbatim-repeated table-driven test
literals (threshold definitions, resource-claim structs, mock-command setup blocks) — real duplication by the
checker's definition, but debatable value as a lint since table-driven tests conventionally repeat near-identical
literals per case. One stapler-squad case (`pkg/classifier/classifier_test.go`) repeats a test-loop body 57–60
times, which is itself a signal a shared test helper is missing rather than a false positive. Not logged as a
`## Log` entry since these are correct according to the check's stated purpose — flagging here only so the
mechanism (generated-code scope) isn't confused with this separate, unresolved policy question (whether test
files should be excluded or literal-abstraction added).
