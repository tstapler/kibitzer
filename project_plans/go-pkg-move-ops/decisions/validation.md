# Go Package Move Operations — Validation

## 1. AC Coverage Check

### AC-1 — Golden-file tests

**Coverage: Partial**

Step 12 creates the golden-file harness and lists nine required fixtures.
The fixture table covers: clean success, `_test` package, `internal/` visibility
blocking, cycle blocking, `rename-pkg`, `extract-decls`, `split-pkg`, and
`sync-policy`. That maps to all but one AC-1 sub-item.

**Gap**: "Negative cases matched to simulator fixture outputs" has no
corresponding fixture. The plan defers the simulator integration to a future
release (`simulation_result: Option<Value>` defaults to `None`). There is no
fixture demonstrating that a plan with a populated `simulation_result` that
reports a blocker is correctly rejected by `apply()`. A placeholder fixture
with a hand-crafted `simulation_result` blob should be added to Step 12 even
before the real simulator exists, so the gate is tested.

**Second gap**: The symbol-conflict fixture (`move-files/symbol-conflict`)
identified in Adversarial Risk 2 is named in the adversarial review but absent
from Step 12's fixture table. It needs to be added explicitly.

---

### AC-2 — Idempotence

**Coverage: No**

No implementation step addresses idempotence. The plan has no logic that
detects "the tree already matches the plan's target" and returns a no-op, nor
any logic that produces a clear actionable error for a second `--apply` on
already-applied state. The atomicity layer (Step 9) gates on `blockers` but
does not compare current file content to planned content before writing.

**Gap**: AC-2 is entirely unaddressed. Resolution path: before the snapshot
phase in `atomic_apply.rs`, compare each `new_content` to the current on-disk
content of `new_path`. If all match, return an `ApplyResult` with
`rolled_back: false` and a zero-length `applied_steps` list (the no-op case).
If some match and some do not, this is a partial-apply state: emit a clear
error with no mutation. Add a golden fixture `move-files/idempotent` that
exercises this. This work belongs in Step 9 and must be in the plan before
implementation of that step.

---

### AC-3 — Atomicity

**Coverage: Partial**

Step 9 and §1.4 describe the snapshot + temp-then-rename + restore pattern.
The approach is sound. However:

1. §1.4's restore-loop description reads:
   `for (path, content) in &snapshot { std::fs::write(path, content).ok(); }`
   It does not include `create_dir_all(path.parent())` before the write. The
   adversarial review (Risk 3) identifies this as a gap but the normative §1.4
   text was not updated to reflect the fix. An implementer reading only §1.4
   will miss it.

2. The AC requires verification "by a checksum of all modified paths before
   and after the forced failure." The plan's unit test asserts content equality
   but does not explicitly describe a checksum comparison. Content equality
   satisfies the intent; the plan should say so rather than leaving it implicit.

**Gap**: §1.4's restore loop description must be updated to include
`create_dir_all`. The unit test description in Step 9 should state that it
asserts byte-for-byte equality of the restored file against the original.

---

### AC-4 — Verification step

**Coverage: Yes**

Step 10 runs `go build ./...`, `go vet ./...`, and `go test ./...` (with
`-count=1`). All three commands run sequentially from `repo_root`. Non-zero
exit drives `_ok` booleans and rolls back. Verbatim output is stored in
`VerifyResult`. The `apply()` function refuses to declare success if any
command exits non-zero. No gaps.

---

### AC-5 — Documented limits

**Coverage: Partial**

The plan commits to documenting a "handled-or-diagnosed matrix" in
`docs/go-pkg-move-ops.md` covering build tags, cgo files, generated files,
`init()`-based registration, and dot imports.

- **Build tags**: `source.rs` already detects them (VERIFIED in research).
  The preflight layer should carry tags through unchanged or emit a diagnostic.
  No implementation step describes this explicitly.
- **Generated files** (`// Code generated`): mentioned in research §3 risk 7
  and in §4.4. No implementation step in the numbered list addresses detection.
- **Dot imports**: handled by the dot-import blocker in Step 5. Covered.
- **`init()` registration**: mentioned as a documentation target only.
  No detection logic is described.
- **cgo files** (`import "C"`): mentioned in the docs target but no
  implementation step describes detection logic or a diagnostic.

**Gap**: Build tags, generated files, `init()` patterns, and cgo are
documentation targets with no corresponding implementation steps. AC-5 requires
either a test proving correct handling, or a clear blocking diagnostic. For
v1, emitting a blocking diagnostic is acceptable, but `preflight.rs` must
include the detection. These detections should be added to Step 7 or as an
explicit new preflight sub-step. Without implementation steps, they risk being
skipped and the "handled-or-diagnosed matrix" in the docs will lack evidence.

---

### AC-6 — Toolchain choice documented

**Coverage: Partial**

The plan commits to documenting the toolchain decision in `docs/go-pkg-move-ops.md`
(Step 13 / §4.4). The research establishes that gorename and gomvpkg are
obsolete, rf is experimental with no rollback, and tree-sitter is chosen.

**Gap**: AC-6 requires documenting gopls evaluation with "the specific API
surface checked, the version tested, and the test result." The research
explicitly labels the gopls package-move API as UNVERIFIED — gopls was not
installed and the `gopls.refactor.*` workspace/executeCommand surface was not
tested. The research prescribes a prerequisite experiment: `go install
golang.org/x/tools/gopls@latest && gopls help && gopls help refactor.move`.
This experiment has not been run. The plan commits to tree-sitter before
completing this experiment.

**Risk**: If gopls does have a clean package-move command (v0.24.0 is current),
AC-6's documentation requirement cannot be met honestly — "version tested" and
"test result" cannot be filled in. The documentation would have to state the
experiment was not run, which is a weaker form of the AC's intent.

**Recommended resolution**: Run the one-shot gopls experiment as a prerequisite
to starting Step 13 (before creating `docs/go-pkg-move-ops.md`). The experiment
takes under 30 minutes and either confirms the rejection rationale or reveals a
capability that changes the architecture decision. It does not block Steps 1–12.

---

### AC-7 — Policy sync

**Coverage: Yes**

Step 11 describes `sync-policy` as a standalone subcommand that accepts
`--plan <file>`, re-reads `from`/`to` fields, and produces a `MovePlan` with
only `PolicyUpdate` steps. The steps are emitted separately and only applied
when `--apply` is passed. Policy changes are explicitly distinct from
file-move changes. No gaps.

---

### AC-8 — Backtest

**Coverage: Yes (plan-only; execution deferred to post-implementation)**

Step 13 describes the backtest script, corpus, and the per-repo procedure. The
corpus matches `docs/backtest-repos.md` (kubernetes, cassandra, servo, ripgrep,
deno, vscode). Results are recorded in `docs/go-pkg-move-ops.md`. No
structural gap; execution happens after implementation, which is the correct
sequencing. The one risk is that corpus repos may not be cloned on the build
machine — `scripts/clone-backtest-repos.sh` must be run first.

---

## 2. Adversarial Review Response

### Risk 1: byte-offset rewrites produce incorrect output when edits overlap

**Proposed mitigation**: add a re-parse comment to each rewrite function; add a
golden test requiring both import and selector rewrite in one pass.

**Sufficiency: Partial**

The mitigation is correct in principle but insufficiently specified in the plan
body. The adversarial review says "The plan must state this constraint
explicitly at the start of §Step 6" — but Step 6 in the plan text does not
include this statement. Step 6's algorithm says "Parse tree-sitter Go" as step
1, which implies re-parsing but does not make the "re-parse the *current*
source, not the original" constraint explicit. An implementer working from the
plan text alone will not see this constraint.

**Additional mitigation needed**: The step descriptions for Steps 4, 5, and 6
should each begin with an explicit note: "Input is the *current* source string
(output of the previous step). Do not cache the initial parse across steps."
The golden test for this case (`move-files/combined-import-and-selector`) should
be added to Step 12's fixture table.

---

### Risk 2: verify step does not catch symbol conflicts in `_test` packages

**Proposed mitigation**: add a symbol conflict pre-flight check in
`preflight.rs`; add golden fixture `move-files/symbol-conflict`.

**Sufficiency: Yes, but the step is not in the numbered list**

The mitigation (pre-flight symbol intersection check) correctly addresses the
gap. However, this check is described only in the adversarial review section.
It is not numbered among Steps 1–13, is not reflected in Step 7's description
(which covers only `internal/` boundaries), and the golden fixture is not in
Step 12's fixture table.

**Additional mitigation needed**: Formalize the symbol conflict check as a
numbered implementation step (Step 7a or a sub-item of Step 7). Add
`move-files/symbol-conflict` to the Step 12 fixture table. Without numbering,
this work is at risk of being treated as optional or deferred.

---

### Risk 3: restore loop is missing `create_dir_all` for moved-file rollback

**Proposed mitigation**: add `create_dir_all(old_path.parent())` to the restore
loop; add a unit test for directory-move with mid-apply failure.

**Sufficiency: Yes, but §1.4 was not updated**

The mitigation is correct and complete. The problem: §1.4 in the plan still
shows the restore loop as a simple `std::fs::write(path, content).ok()` without
the required `create_dir_all`. The fix is described only in the adversarial
review. An implementer reading §1.4 normatively will produce a defective
restore path.

**Additional mitigation needed**: §1.4's restore loop description must be
updated in the plan document before implementation of Step 9 begins. The
unit test for this case (directory move, mid-apply failure at cleanup step)
should be added to Step 9's test description, not left only in the adversarial
review.

---

## 3. Open Questions

### Q1: gopls package-move API surface (blocks AC-6)

**Status**: UNVERIFIED (research §1 explicit)

The research specifies the exact resolution: `go install
golang.org/x/tools/gopls@latest && gopls help && gopls help refactor.move`.
This must be run before `docs/go-pkg-move-ops.md` is written (Step 13). If
the command confirms a `refactor.move` surface exists and works on a test
module with `_test` packages and `internal/` targets, the architecture decision
must be revisited. If it confirms absence or failure, the tree-sitter choice
stands with evidence.

**Resolution path**: run the experiment as a zero-step prerequisite; record
the full command output in `docs/go-pkg-move-ops.md`. Does not block Steps 1–12.

---

### Q2: simulation_result gating — requirements say "must embed and refuse"; plan defers

**Status**: tension between requirements and plan

The requirements state: "An operation's plan must embed the simulation result
and refuse to apply when it reports a blocker." The plan ships `simulation_result:
Option<Value> = None` with the apply gate on `blockers` only. This means v1
ships without the gate the requirements prescribe.

**Resolution path**: the plan's deferral is pragmatic and reasonable, but it
needs explicit owner sign-off before implementation. The specific question: is
it acceptable for v1 to ship with an inert `simulation_result: None` field and
no apply-gate on the simulator, with the gate wired in a follow-up once #153
ships? If yes, update the requirements to reflect the v1/v2 boundary for this
gate.

---

### Q3: AC-2 idempotence — no implementation step exists

**Status**: gap (see AC Coverage above)

**Resolution path**: add the following to Step 9 before implementation of that
step: before the snapshot phase, compare each `(new_path, new_content)` pair
to the current on-disk content. All-match → no-op result. Partial match →
blocking error. Add golden fixture `move-files/idempotent`. This is
self-contained and requires no new research.

---

### Q4: preflight coverage for build tags, generated files, cgo, `init()` patterns

**Status**: mentioned in AC-5 and docs target, but no implementation steps

**Resolution path**: add a Step 7b (or expand Step 7) for preflight detection
of: (a) cgo files (`import "C"` in any moved file → blocking diagnostic), (b)
generated files (`// Code generated` marker → blocking diagnostic with
regeneration instruction), (c) `init()` registration patterns (detect `init()`
functions in moved files → non-blocking warning in plan output). Build tags
are already detected by `source.rs`; the preflight layer should carry them
through unchanged (not a blocker, but must be documented in the handled matrix).

---

## 4. Dependencies and Sequencing

### External dependency: go-package-focus check (#138)

`split-pkg` accepts `--focus-json <file>` — the partition input is a file, not
a live check invocation. This decouples `split-pkg` from #138's shipping state.
For testing, a hand-crafted JSON file can stand in for real `go-package-focus`
output. **Not a blocker for any implementation step.**

### External dependency: feasibility simulator (#153 umbrella)

The plan's `simulation_result: Option<Value>` field is a forward-compatible
hook. The apply gate for v1 is `blockers.is_empty()`, not `simulation_result`
presence. **Not a blocker**, but Q2 above requires owner sign-off on this
deferral being acceptable for v1.

### Minimum viable subset without these deps

Steps 1–12 (all except backtest) can be completed without #138 or #153. This
delivers a working `move-files`, `rename-pkg`, `extract-decls`, `split-pkg`
(with hand-crafted focus JSON), and `sync-policy`, with full golden-file test
coverage. Step 13 (backtest) can follow once the implementation is complete.

The one prerequisite that should be satisfied before implementation begins is
Q1 (gopls experiment) — it takes under an hour and could change the
architecture decision. All other open questions can be resolved
concurrently with Steps 1–3 (CLI scaffolding, types, import graph) without
risk, since those steps are toolchain-choice-independent.

---

## 5. Go/No-Go Verdict

**CONDITIONAL GO**

The architecture is sound, the research is thorough, and Steps 1–13 provide a
complete and orderly implementation path. However, three gaps must be resolved
in the plan document before implementation of the affected steps begins — not
blocking the start of work, but blocking those specific steps:

1. **Before Step 9**: Update §1.4's restore loop to include
   `create_dir_all(old_path.parent())`, add the idempotence check (AC-2),
   and add the directory-move unit test case.
2. **Before Step 7**: Add the symbol-conflict pre-flight check as a numbered
   step and add its golden fixture to Step 12's table.
3. **Before Step 13**: Run the gopls experiment (Q1) and add preflight
   detection for cgo, generated files, and `init()` patterns (Q4).

Implementation can start immediately on Steps 1–3 (CLI scaffolding, data
structures, import graph reverse-lookup) — these are toolchain-choice-independent
and contain no gaps. The three above gaps do not affect those steps and can be
resolved in the plan while Steps 1–3 are in flight.
