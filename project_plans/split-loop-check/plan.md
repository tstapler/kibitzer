# Implementation Plan: split-loop-check

## Overview

Add a new native per-file checker `split-loop-<lang>` that flags a loop body advancing 2+ mutually independent accumulators — Fowler's Split Loop. Go ships first (richest loop AST coverage); other languages follow in one pass once the Go checker is backtested and tuned.

## Implementation phases

### Phase 1: Go checker skeleton + basic accumulator detection

**Files to create/modify:**

- `src/checkers/split_loop.rs` — new checker module. Implement `SplitLoopChecker` struct, `Checker` trait, `inventory::submit!`.
- `src/checkers/mod.rs` — add `pub mod split_loop;`
- `src/config.rs` — add `split-loop-go` to `go_exception_checks()` (or a new `split_loop_checks()` chain in `default_checks()`).

**Core algorithm (Go, v1):**

```
fn check_loop_body(body: Node, src: &[u8], findings: &mut Vec<Finding>) {
    // 1. Walk direct children of loop body, collect accumulator assignments.
    //    - assignment_statement with += operator (or = where RHS is append(lhs, ...))
    //    - inc_statement (x++)
    // 2. For each accumulator, record (lhs_identifier_name, rhs_source_text).
    // 3. Build dependency graph: accumulator A and B are in the same group if
    //    A's rhs_text contains B's name as a word-boundary token (or vice versa).
    // 4. Use union_find to group by transitive dependency.
    // 5. If groups.len() >= 2: emit finding at loop start line.
}
```

**Loop node kinds for Go:** `for_statement` (covers `range` and C-style). Walk with `walk_preorder` looking for `GoKind::ForStatement`.

**Accumulator shapes for Go (v1):**
- `assignment_statement` with `+=` / `-=` / `*=` / `/=` operators
- `assignment_statement` where RHS calls `append(lhs_name, ...)`
- `inc_statement` (`x++`, `x--`)
- `assignment_statement` where LHS is `index_expression` with a local variable as the receiver

**Non-accumulators (v1 exclusions):**
- Multi-assignment `a, b = f()` — skip (can't split independently)
- Short-variable declarations `:=` — skip (not an update)

### Phase 2: Multi-language extension

Once Go is backtested:

- Extend `src/checkers/split_loop.rs` with per-language configs (similar to `rules.rs`'s `LangRuleConfig`).
- Each language needs: loop node kinds, accumulator assignment node kinds, augmented-assignment operator text.
- Register as `split-loop-typescript`, `split-loop-python`, etc.
- Add to `default_checks()`.

### Phase 3: Backtesting and tuning

**Transcript backtest:**
```
kibitzer check backtest split-loop-go
```
Review findings. If false-positive rate > 40%, tighten the accumulator detection rules (e.g. add a minimum of 2 accumulator *writes* per group, or exclude trivial `i++` counter-only loops).

**Real-world corpus backtest:**
```
kibitzer check native split-loop-go ~/code/github.com/kubernetes/kubernetes/.../*.go | head -50
kibitzer check native split-loop-go ~/Programming/stapler-squad/.../*.go | head -50
```
Document 10 sampled findings with true/false positive labels.

### Phase 4: Documentation

- Create `docs/split-loop-false-positives.md` documenting known false-positive patterns.
- Update `docs/syntax-rules.md` or create a sibling `docs/split-loop.md` describing the check.
- Update `default_checks()` test count assertion in `config.rs` tests.

## Key design decisions

**Union-find for grouping:** `src/union_find.rs` already exists. Use it rather than a bespoke DFS — consistent with `duplicate-code`'s usage.

**Checker name scheme:** `split-loop-go`, `split-loop-python`, etc. — follows `go-bulk-fetch-linear-scan`'s language-qualified pattern rather than `syntax-rules`'s language-parameterized singleton scheme. The split-loop check's accumulator detection logic differs enough per language (augmented assignment AST shapes vary significantly) that a struct-per-language approach is cleaner.

**Minimum accumulator count threshold:** v1 requires exactly ≥ 2 distinct named accumulator *identifiers* in independent groups. A loop with `i++` (loop variable, not a business accumulator) plus one real accumulator is NOT flagged — the loop variable is not a named accumulator unless it appears in a `+=`/`append` shape outside the range clause itself.

**Loop variable exclusion:** In a `for i, v := range slice {}` loop, `i` and `v` are range variables, not accumulators. Detect range variables from the range clause's left-hand side and exclude them from accumulator tracking.

**False positive mitigation:** A `sum + count` pair (semantically dependent but syntactically independent) is an accepted false positive. Document in `split-loop-false-positives.md`. The finding message includes the accumulator names so the user can quickly assess whether the flag applies.

## Finding message format

```
[split-loop] loop advances independent accumulators (sum, count) — Fowler's Split Loop: consider one loop per accumulator group
```

Naming the accumulator groups in the message lets the user immediately assess applicability without re-reading the loop body.

## Test plan

**Unit tests in `src/checkers/split_loop.rs`:**

1. Two unrelated `+=` accumulators → flagged
2. Two `append` accumulators with no cross-reference → flagged
3. One accumulator only → not flagged
4. Two accumulators where one's RHS mentions the other → not flagged
5. `sum += item.price; count += 1` → flagged (documented as a false positive type)
6. Range variable `i` accumulated with `+=` while another accumulator runs → depends on whether `i` appears in a business-logic `+=` outside the range clause
7. Loop with `append(list1, ...)` and `append(list2, ...)` where `list2` is derived from `list1` → not flagged (direct name reference)

**Integration test against real Go code:** One test file with 3 loops — one that should fire, one that shouldn't, one edge case.

## Risks and mitigations

| Risk | Likelihood | Mitigation |
|------|-----------|------------|
| High false-positive rate on sum+count patterns | Medium | Accept and document; add `kibitzer:ignore` escape hatch |
| Loop variable exclusion misses edge cases | Low | Backtest; if rate is high, add minimum threshold (≥3 unique accumulators to flag) |
| Multi-assignment `a, b = f()` creates phantom accumulators | Low | Explicitly skip multi-LHS assignments |
| Performance on large files (deep walk for every loop) | Low | `walk_preorder` is O(n) in tree nodes; existing checkers already do this |
| Kotlin positional-field grammar misses augmented assignments | Medium | Implement Kotlin phase after Go is stable; verify via `to_sexp()` before shipping |

## Implementation deviation (recorded at sdd:6-verify)

The shipped checker is more conservative than the plan's direct-RHS-mention + union-find
design. Added to cut false positives, per the item's "precision over recall" scope note:
early-exit bailout, unaccounted-use bailout, guard-identifier linking, if/else and switch
partition linking, same-guard linking, shared-input linking, tuple-assignment linking, and
skipping pure `x++`/`x += 1` counters. Consequence: plan test 5 (`sum += price; count += 1`)
is now *not* flagged, and "two independent appends" is flagged only when they draw on
different inputs. Corpus precision is still low (see `backtest-results.md`).
