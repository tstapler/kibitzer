# Validation: split-loop-check

## Adversarial review

### Challenge 1: "sum + count" false positive

**Claim:** A `sum += item.price; count += 1` loop will be flagged even though `sum` and `count` are often used together (e.g., `avg = sum / count`).

**Assessment: Real gap, accepted.** The two accumulators share no syntactic reference in their update expressions. v1 will flag this. This is a known Fowler-cited false-positive class — the textbook itself notes that separating a sum/count loop into two passes is usually fine, since modern CPUs handle the second pass cheaply. The mitigation is `kibitzer:ignore split-loop-go`. Document in `split-loop-false-positives.md`.

**Decision: Accept false positive; document and add to false-positive page.**

---

### Challenge 2: Loop variable `i` being treated as an accumulator

**Claim:** In `for i := 0; i < n; i++ { sum += items[i]; list = append(list, items[i]) }`, `i++` looks like an accumulator increment but `i` is a loop variable.

**Assessment: Mitigated by range-variable exclusion.** For C-style loops, the init and increment clauses are part of `for_statement`'s structure — the check should only look at the body. For `range` loops, the range variables are declared in the range clause, not the body. The implementation must walk body-only, not the loop header. If `i++` appears in the body AND `i` also appears in a `+=` context, it would be tracked. But a bare loop counter `i++` with no `+=` elsewhere for `i` does not meet the "two independent accumulator groups" condition.

**Decision: Address in implementation by walking body-only statements; add a test case.**

---

### Challenge 3: Nested loops — inner loop accumulators leak to outer analysis

**Claim:** An outer loop contains an inner loop, each with its own accumulators. The outer-loop body walk might pick up the inner loop's accumulators and generate spurious independence findings.

**Assessment: Real gap.** The walk must only collect *direct* children's accumulator assignments, not recurse into nested loops. The implementation must stop at nested loop boundaries.

**Decision: Explicitly exclude accumulator collection from nested loop bodies. Use a non-recursive child walk for accumulator collection (look only at the loop body's direct statement children, then recurse into if/block children but not into nested loops).**

---

### Challenge 4: Precision vs. recall tradeoff for multi-language support

**Claim:** The accumulator detection AST shapes differ significantly per language (Python's `+=` is `augmented_assignment`, Java's is `assignment_expression` with `+=`, Rust's is `compound_assignment_expr`). Shipping all languages at once increases implementation risk.

**Assessment: Valid.** Go first, single-language v1, is the right scope. The plan already says this. Confirm: multi-language is Phase 2, with per-language verification against `to_sexp()` output before each language ships. The checker name scheme (`split-loop-go`, `split-loop-python`, etc.) naturally supports this staged rollout — each language is independently registered.

**Decision: Confirm staged rollout. Go only for initial ship. Multi-language in a follow-up PR.**

---

### Challenge 5: `default_checks()` test count assertion breaks on new check

**Claim:** `config.rs` has `assert_eq!(config.checks.len(), default_checks().len())` assertions and an explicit count test. Adding a new check without updating these will cause test failures.

**Assessment: True, well-documented in existing checks.** The plan calls for updating these. Explicitly flag as a required task. The test `default_checks_ids_are_unique` also exists — the new check name must be unique.

**Decision: Explicitly include "update default_checks test count" as a required implementation task.**

---

### Challenge 6: `backtesting.md` bar — no transcript occurrence evidence

**Claim:** Unlike `docs/check-ideas.md`'s "confirmed transcript occurrence" bar, this check is proposed from catalog reasoning. This creates a risk that it fires on no real code or fires too noisily.

**Assessment: Acknowledged risk.** The mitigation is the two-step backtest gate: (1) transcript backtest to see if it fires on real edits at all, and (2) real-world corpus backtest. If the transcript backtest shows 0 firings across 1000+ file snapshots, the check may have zero real-world impact. If the false-positive rate is > 40%, the threshold or detection criteria must be tightened.

**Decision: Transcript and corpus backtests are hard gates before ship. Document results in `project_plans/split-loop-check/backtest-results.md`. Do not ship without them.**

---

## Validation summary

| Risk | Decision |
|------|---------|
| sum+count false positive | Accept; document; `kibitzer:ignore` escape |
| Loop variable treated as accumulator | Mitigate: body-only walk; test case |
| Nested loop leakage | Mitigate: stop at nested loop boundaries |
| Multi-language complexity | Staged rollout: Go first, others follow |
| `default_checks()` test count | Required task to update assertions |
| No transcript evidence | Hard gate: backtest before ship |

## Open questions

1. Should the finding be on the loop's start line or on the first independent accumulator's line? (Start line is more consistent with `long-function`'s behavior.)
2. Should the minimum number of independent groups be configurable via `options`? (No for v1; add if backtest shows noise from 2-group threshold.)
3. Is `x--` / `x -= 1` an accumulator? (Yes — the sign doesn't matter; it's still a numeric accumulator update.)
4. Should method-call accumulators like `list.add(item)` (Java) and `list.push(item)` (TypeScript/Rust) be detected in v1 for those languages, or only `+=`? (Defer per-language method-call detection to Phase 2 to keep v1 scope tight.)
