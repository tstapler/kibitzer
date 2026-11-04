# Architecture Review: conditional-logic-smells
**Date**: 2026-11-03
**Verdict**: BLOCKED (2 blockers, 5 concerns)

---

## Blockers

- [ ] **Story 3.1.2a** — `decompose-conditional` will fire on `return` expressions, not just if-conditions.
  `walk_dense_expressions` builds `dense` as either `("condition", condition_node)` for
  branching nodes or `("return", return_node)` for return statements. The plan's Task 3.1.2a
  pseudocode maps dense to just the node (`dense.map(|(_, n)| n)`), dropping the position tag
  before calling `condition_decision_count`. A `return a && b && c && d && e` with 4 `&&`
  operators would then score 4 decision points and fire the check even though it is not a
  conditional expression. The check's requirements, Fowler's definition, and every test case
  in the plan scope the rule to *if/ternary conditions only*.
  **Remediation**: Guard the `condition_decision_count` call on `position == "condition"`:
  ```rust
  if let Some(("condition", cond_node)) = dense {
      let decisions = condition_decision_count(cond_node, cfg, src);
      if decisions >= DECOMPOSE_CONDITIONAL_THRESHOLD { ... }
  }
  ```
  The existing `extract-variable` check can keep its current form (it fires on returns too,
  per design); only the new `decompose-conditional` check needs the guard.

- [ ] **Story 2.1.1a** — assignment bodies excluded from `consolidate-conditional` without a
  recorded scope decision. Requirements AC4 and the rule definition both say "each statement
  is either a `return` of the same value or an assignment to the same local variable of the
  same value." Task 2.1.1a step 3 restricts the consequence body to "`return_kind` only."
  The CATALOG description in Task 2.1.2a confirms the narrowing ("returning the same value")
  but no row in the Plan Decisions table records this as an intentional Go-first scope cut.
  Without a recorded decision, the implementation will silently miss the assignment case,
  failing acceptance criterion AC4.
  **Remediation**: Add a row to the Pattern Decisions table:
  > `consolidate-conditional` body type | Go-first return-only | requirements.md AC4 | Cover
  > assignment bodies in a follow-up | Assignment-body detection requires matching the LHS
  > identifier across run members, adding complexity; return-only covers the dominant pattern
  > and ships the check sooner.
  Then update the CATALOG description to say "returning the same value" (drop the assignment
  language) so it matches the implementation.

---

## Concerns

- [ ] **Story 3.1.6a, should-fire test 1** — test spec is internally contradictory. The case
  specifies `if a && b && c && d && e { ... }` and annotates it "(exactly 4 `&&` = 4 decision
  points — fires at threshold 5)." `condition_decision_count` returns 4; `4 >= 5` is false;
  the finding is not emitted. This would be a failing unit test at threshold 5. Either the
  condition needs a fifth `&&` (`a && b && c && d && e && f`) or the description needs
  correction. The annotation "fires at threshold 5 when count ≥ threshold" is what
  `>= DECOMPOSE_CONDITIONAL_THRESHOLD` means, but 4 does not satisfy it.
  **Recommendation**: use `a && b && c && d && e && f` (5 `&&` = 5 decision points, fires
  exactly at threshold 5) for the boundary test, and add a separate should-not-fire case for
  `a && b && c && d && e` (4 decision points) to verify the off-by-one.

- [ ] **Task 1.1.1a** — `bool_local_finder` HRTB lifetime is unresolved. The plan commits to
  `for<'a> fn(Node<'a>, &'a [u8]) -> Option<String>` but `Option<String>` is a heap-allocated
  owned type — it does not borrow from either `Node` or `&[u8]`. The higher-ranked trait bound
  is therefore unnecessary. The research/architecture.md catches this ("If the return type stays
  a heap-allocated `String` and never borrows from `src`, a bare `fn(Node, &[u8]) -> Option<String>`
  suffices — verify at implementation time") but the plan does not resolve the open question. A
  bare signature matches `bool_param_finder: fn(Node, &[u8]) -> Vec<String>` and is simpler.
  **Recommendation**: use `fn(Node, &[u8]) -> Option<String>` and remove the note; if the
  implementation needs the HRTB (e.g., because it returns `&'a str` from `src`), change the
  return type to `Option<&'a str>` and add the HRTB then.

- [ ] **Task 1.1.1b** — `collect_bool_flags` return type omits lifetime annotation. The
  signature in the plan is:
  ```rust
  fn collect_bool_flags(body: Node, cfg: &LangRuleConfig, src: &[u8]) -> HashMap<String, Node>
  ```
  `Node` in tree-sitter is `Node<'_>` — it carries a lifetime tied to the parsed tree. Without
  an explicit lifetime the function signature will not compile as written. The correct form is:
  ```rust
  fn collect_bool_flags<'a>(body: Node<'a>, cfg: &LangRuleConfig, src: &[u8]) -> HashMap<String, Node<'a>>
  ```
  Caught at compile time, but fixing it requires explicit thought about whether `emit_flag_findings`
  needs the declaration node at all (it is listed in the return value but only the name is used
  in pass 2 for identifier classification — the node could be replaced with the declaration line
  number to avoid the lifetime dependency entirely).

- [ ] **Story 2.1.2b** — no generated-file guard for `walk_consecutive_ifs`. The architecture
  doc notes: "Generated files skip `walk_dense_expressions` and `walk_literals`/
  `emit_literal_findings`." The plan adds `walk_consecutive_ifs` to `check()` after `walk_blocks`
  with no corresponding guard, no call to `crate::checkers::file_size::is_generated`, and no
  mention of whether the outer `check()` already guards all walks. If the guard is inside `check()`
  at the top (before any walk dispatch), the concern is void — but neither the plan nor the
  architecture doc confirms this.
  **Recommendation**: confirm the generated-file guard placement in `check()` before implementing
  Task 2.1.2b, and add a note to the task. If the guard is walk-local (inside each walk function),
  add one for `walk_consecutive_ifs`.

- [ ] **Tech debt table** — `check_declaration` self-violation not fully dispositioned. The table
  says "Accept: new rule logic goes in separate helpers." True — but `check_declaration` is
  already 54 lines (above `LONG_FUNCTION_LINES = 40`), and Task 1.1.2b adds four more lines
  inside it (the `collect_bool_flags`/`emit_flag_findings` call block), bringing it to ~58.
  The plan is silent on whether a `kibitzer:ignore long-function` annotation is in place, or
  whether CI enforces that kibitzer's finding count on its own codebase does not increase. If
  there is a no-new-self-findings gate, this task will fail it.
  **Recommendation**: add one line to the tech debt table: "A `kibitzer:ignore long-function`
  annotation is already present / will be added before Task 1.1.2b lands — confirm before merge."

---

## Nitpicks

- `docs/adr/ADR-000-architecture-constitution.md` does not exist. No impact on implementation.
- `DECOMPOSE_CONDITIONAL_THRESHOLD = 5` in plan.md vs. `= 3` in research/architecture.md —
  plan.md is authoritative and the 5 is well supported by pitfalls.md's corpus evidence. The
  architecture doc's earlier estimate of 3 should be struck through or annotated "superseded by
  plan.md" to avoid confusing implementors who read both.
- Glossary entry for "condition decision count" describes it as "computed with a new
  cross-grammar helper `condition_decision_count`" and separately says `cyclomatic_complexity`
  "returns 1 + decision points." `condition_decision_count` returns a 0-based count (0 for a
  single-comparison expression), not 1+. The wording is correct in context but the juxtaposition
  with the cyclomatic convention is misleading — a reader could add 1 by habit.
- Build-vs-buy is well justified. No existing Go linter implements any of the three checks at
  Fowler's exact specification level. Native tree-sitter implementation is the only viable path.
  No concerns here.
- SOLID, layer coupling, and OCP lenses: the "Extend as-is" disposition for `rules.rs` is
  honest and proportionate. The file will reach ~5,850 lines, but the production section stays
  ~2,370. The plan correctly anticipates that a `src/checkers/rules/` subdirectory split becomes
  warranted at a higher rule count — that is the right OCP trigger and the right time to revisit.
  No structural violation today.
