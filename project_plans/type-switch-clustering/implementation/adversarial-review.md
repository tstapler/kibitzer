# Adversarial Review: type-switch-clustering

**Date**: 2026-11-04

## Reviewer frame

Treat this as a reviewer who wants the checker to either not exist (too noisy) or to be
wrong in some measurable way. The plan has to survive each objection.

---

## Objection 1: The subject-string matching will produce zero findings on real code

**Claim**: Two methods on the same type will almost never have the exact same switch-subject
string in real Go code, because receiver names differ (`r`, `s`, `c`, `self`, etc.), so
`r.kind` ≠ `c.kind` even for the same field.

**Assessment**: VALID CONCERN. This is documented in the plan's risk log and research
open questions. The plan correctly scopes to conservative matching for v1 and requires a
backtest. If the backtest shows zero true positives across the entire corpus, the plan needs
to revisit normalization before landing (strip receiver variable prefix, compare field name
only: `kind` == `kind`). This is not a reason to not build the feature — it's a threshold
calibration question answerable only by running the backtest.

**Mitigation in plan**: Backtest (Story 5) is required before landing, with explicit
instruction to adjust if FP or TP rate is off. The plan should add: if the backtest shows
<2 true positives across the full corpus, strip-receiver normalization must be attempted
before the feature ships. → **Add this criterion to Story 5.2**.

---

## Objection 2: The hierarchy-present detection heuristic is unreliable

**Claim**: "If any `TypeRelationEdge`'s `from` or `to` matches one of the case type names"
is a loose string-contains check that could match unrelated types with similar names.

**Assessment**: VALID. The heuristic is a fuzzy substring match, not a resolved symbol
lookup. For example, a type named `HttpRequest` might spuriously match an edge involving
`AuthRequest`. The plan should use exact name equality, not substring containment, against
the edge's `from`/`to` fields (which are already qualified ids like `pkg::TypeName`).

**Mitigation**: Change the plan's heuristic to: "case type names from the cluster's type
switches (e.g. `*Circle`, `*Square`) are stripped of pointer prefixes and matched as
exact suffix equality against `edge.from` / `edge.to`." → **Update plan Story 3.2 step 5**.

---

## Objection 3: The ArchModelChecker re-walk adds too much latency

**Claim**: Running a second tree-sitter walk over all Go files in a large repo (e.g.
`kubernetes/kubernetes` has ~1.5M lines of Go) will make batch-mode kibitzer unacceptably
slow.

**Assessment**: PLAUSIBLE BUT MANAGEABLE. The checker runs at `batch` granularity only
(not on every edit), and `kubernetes` itself takes ~15-20s for `kibitzer run` already.
An additional Go-only AST walk that skips non-Go files and generated files should add
< 5s on a large repo. This is acceptable for a batch-only check. If the backtest shows
excessive latency, the checker can gate on a minimum-file-count or be made opt-in.
Document the expected overhead in the false-positives doc.

---

## Objection 4: This duplicates go-type-switch-density — should it just be an enhancement?

**Claim**: Both checkers walk Go type switches. Why not just add the clustering logic to
the existing per-file checker?

**Assessment**: WRONG FRAMING. `go-type-switch-density` is a per-file `Checker` — it
cannot see methods in other files. The clustering logic inherently requires cross-file
(cross-method) data. These are different checker shapes (per-file vs. whole-repo
ArchModel); merging them would require either (a) changing `go-type-switch-density` from
a `Checker` to an `ArchModelChecker` (breaking its single-file simplicity) or (b) a
two-pass accumulator pattern (complex). Two separate checkers with clear separation of
concerns is the right design. The plan is correct on this point.

---

## Objection 5: The finding message is too verbose

**Claim**: Naming all N methods in the finding message makes it long and hard to act on.

**Assessment**: VALID UX CONCERN. The message should truncate at 3 method names with
"…and N more" if the cluster is large. This matches the style of `duplicate-code`'s
findings, which truncate long location lists. → **Add truncation rule to Story 3.2's
message format**.

---

## Summary of required plan changes

1. **Story 5.2**: Add "if backtest shows <2 corpus TPs at exact-string matching, attempt
   strip-receiver normalization before shipping."
2. **Story 3.2 step 5**: Change heuristic to exact suffix-equality on case type names vs.
   `edge.from`/`edge.to`, not substring containment.
3. **Story 3.2 message format**: Truncate method list at 3 with "…and N more."

None of these change the fundamental approach — they are calibration and precision
improvements already within the plan's risk awareness.
