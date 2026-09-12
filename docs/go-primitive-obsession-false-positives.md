# `go-primitive-obsession` check — known false positives

Tracks confirmed false-positive firings of the `go-primitive-obsession` kibitzer
check (`src/primitive_obsession.rs`, wired up per-project via `.claude/inspect.json`'s
`go-primitive-obsession` entry running `kibitzer check primitive-obsession {file}` on
every `Edit|Write` to a `**/*.go` file). Check new occurrences against this list before
re-investigating a firing from scratch.

## Root-cause mechanism (confirmed by reading the source)

The check does **not** diff old vs. new file content and does not look at which lines
an edit actually touched:

- `hook::run_hook` (`src/hook.rs`) reads the `PostToolUse` event, extracts only
  `tool_input.file_path`, and calls `run_checks_smart` — it never inspects the tool's
  diff/patch content, just the path.
- `check::run_check` (`src/check.rs`) shells out to the configured command
  (`kibitzer check primitive-obsession {file}`) with the file path substituted in.
- `main.rs`'s `CheckCommand::PrimitiveObsession` handler calls
  `primitive_obsession::check_file(&file)`, which does `std::fs::read_to_string(path)`
  and tree-sitter-parses the **entire current file on disk**, then walks every
  `parameter_list` in it (`primitive_obsession.rs::walk`/`check_parameter_list`).

So the check scans the whole file's current contents on every `Edit`/`Write` to any
`.go` file, and reports every matching signature anywhere in that file — regardless of
whether the just-applied edit added, removed, or left that signature untouched. The
project's `.claude/settings.json` matcher (`"matcher": "Edit|Write"`) and the check's
`scope: ["**/*.go"]` mean it fires on essentially any edit to any Go file in scope, not
just one that introduces a new same-typed-parameter signature.

## Fixed

### 2026-08-18 — stapler-squad-tests — ambiguous-substring fallback re-scanned whole file

- **Symptom**: an `Edit` to `server/services/session_service_test.go` moved an
  identical 3-line setup block into a new `t.Run` subtest of a table that already
  duplicates that same boilerplate across several other subtests — `tstapler/stapler-squad`.
- **Mechanism**: `compute_changed_lines` (`src/hook.rs`) located an `Edit`'s
  `new_string` by searching for it as a unique substring of the current file, and
  previously bailed to `None` (unscoped, whole-file check) whenever that text
  occurred more than once — which duplicated subtest boilerplate guarantees.
- **Fixed by**: scoping to the union of *all* occurrences of an ambiguous `new_string`
  instead of giving up and scanning the whole file. Regression-guarded by
  `unions_all_occurrences_when_new_string_is_ambiguous` and
  `duplicated_subtest_boilerplate_scopes_to_all_copies_not_whole_file` in
  `src/hook.rs`'s test module.

### 2026-08-10 — stapler-squad — deletion-only edit flagged

- **Symptom**: an `Edit` to `server/tls.go` that only *removed*
  `LoadTLSConfig(certFile, keyFile string) (*tls.Config, error)` (dead code, no
  signature added) still re-flagged an unrelated pre-existing signature elsewhere in
  the file — `tstapler/stapler-squad`.
- **Mechanism**: the generic `changed_lines` scoping added for the entry above didn't
  actually close this case. `compute_changed_lines` (`src/hook.rs`) skipped empty
  `new_string` values (pure deletions) when building ranges, and if *every* needle was
  empty it fell through to `None` (unscoped) — reproducing the original whole-file
  rescan. Separately, `scope_output_to_changed_lines` (`src/check.rs`) treated an empty
  ranges slice as "nothing to scope, return raw output" rather than "scope to nothing."
- **Fixed by**: `compute_changed_lines` now returns `Some(Vec::new())` for an
  all-deletion edit instead of `None`, and `scope_output_to_changed_lines` now treats
  empty ranges (when the raw check failed) as "everything out of scope" — suppressed
  output, passed. Regression-guarded by
  `deletion_only_edit_scopes_to_empty_ranges_instead_of_unscoped` and
  `multi_edit_of_only_deletions_scopes_to_empty_ranges` (`src/hook.rs`) and
  `scope_output_empty_ranges_suppresses_all_findings` (`src/check.rs`). Verified
  directly against a real `kibitzer hook` payload simulating a deletion-only edit on
  `examples/go/bad.go`'s pre-existing 6-string-parameter `createUser` signature: zero
  findings, exit 0 (previously would have re-flagged `createUser`).

### 2026-08-10 — stapler-squad — pre-existing unchanged signatures flagged

- **Symptom**: edits to `server/tls.go`/`main.go` that didn't alter the shape of
  `certCurrent(certFile, hashFile, want string)` or `LoadTLSConfig(certFile, keyFile
  string)` still re-flagged them — `tstapler/stapler-squad`.
- **Mechanism**: `check_file` reads and parses the entire current file, not a diff —
  but `run_native_check`/`scope_output_to_changed_lines` (`src/check.rs`) is generic,
  checker-agnostic output-line filtering that already covered `primitive-obsession`
  the same way it covers `go-blank-imports`; this entry was stale by the time it was
  re-investigated.
- **Fixed by**: nothing new — already covered by the generic scoping above. Added
  `native_primitive_obsession_check_scopes_output_to_changed_lines` and
  `native_primitive_obsession_check_suppresses_findings_for_deletion_only_edit`
  (`src/check.rs`) since no primitive-obsession-specific regression test existed yet.
  Verified directly against a real edit touching only `LoadTLSConfig`'s body while
  `certCurrent` sat untouched elsewhere in the file: only `LoadTLSConfig` fired,
  `certCurrent` did not.

### 2026-09-12 — kubernetes/kubernetes + stapler-squad corpus backtest — named return-value list misclassified as a parameter list

- **Symptom**: `func (in *FlowSchemaList) APILifecycleIntroduced() (major, minor int)` (kubernetes/kubernetes,
  703 occurrences of this shape alone across 61 `zz_generated.prerelease-lifecycle.go`-style codegen files) and
  `Acknowledged() (r bool, exists bool)` (stapler-squad `session/ent/mutation.go`) both flagged — neither is an
  input parameter list, so there's no call-site-mixup risk to catch.
- **Mechanism**: `tree-sitter-go`'s grammar gives a function's `result` field the same node kind
  (`parameter_list`) as its `parameters` field. `walk()` (`src/primitive_obsession.rs`) matched on
  `node.kind() == "parameter_list"` with no check on which field it came from.
- **Fixed by**: `is_named_return_list(node)` checks whether the `parameter_list` was reached via its parent's
  `result` field, and `walk()` now skips such nodes — verified this field is used identically by
  `function_declaration`, `method_declaration`, `func_literal`, `function_type`, and `method_elem`, so the one
  field check covers every parent kind. Regression-guarded by `ignores_named_return_list_on_function_declaration`,
  `ignores_named_return_list_on_method_declaration`, `ignores_named_return_list_on_func_literal`, and
  `still_flags_parameters_when_named_return_list_present` (a true-positive guard: params still flag even when the
  same function also has a same-shaped named return list) in `src/primitive_obsession.rs`'s test module. Verified
  directly: both corpus files now produce zero findings at the cited lines, while the doc's cited true positive
  (`cmd/kubeadm/app/cmd/util/join.go:51`, kubernetes/kubernetes, a real 3-bool boolean trap) still flags.

### 2026-09-12 — stapler-squad corpus backtest — blank identifier `_` counted as a named identifier

- **Symptom**: `CallBlocking(_, _ string, ...)` (stapler-squad `session/autonomous_driver_test.go:28`, a test fake
  with both parameters intentionally discarded) flagged, even though nothing can be passed in the wrong order to a
  value nobody reads.
- **Mechanism**: `describe_param` (`src/primitive_obsession.rs`) computed `name_count` via
  `decl.children_by_field_name("name", &mut cursor).count()`. Go's blank identifier `_` parses as a plain
  `identifier` node, so `_, _ string` yielded `name_count == 2` and fired the "multiple names in one declaration"
  branch exactly as if both names were meaningful.
- **Fixed by**: `describe_param` now also computes a blank-excluding `real_name_count`, and the "multiple names"
  branch fires on `real_name_count >= 2` instead — so both `_, _ string` (0 real names) and `x, _ string` (1 real
  name) no longer fire, since a blank name can never be confused with anything in the function body. Raw
  `name_count` is unchanged for the unrelated "run of same-typed single-name declarations" branch. Regression-guarded
  by `ignores_all_blank_names` and `ignores_mixed_blank_and_real_name` (`src/primitive_obsession.rs`'s test
  module), plus a true-positive guard `flags_real_multi_named_parameter_list`. Verified directly: the corpus
  file's line 28 finding is gone, while two unrelated genuine positives elsewhere in the same file (lines 1408,
  1417) still flag.

## Log

No open entries.
