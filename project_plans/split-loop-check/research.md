# Research: split-loop-check

## Fowler's Split Loop (canonical definition)

Fowler's *Refactoring* 2nd ed., p. 227: "If you're doing two different things in the same loop, then whenever you need to modify the loop you have to understand both things the loop is doing. By splitting the loop, you ensure you only need to understand the behavior you need to modify." The refactoring is to split one multi-accumulator loop into N separate loops, one per independent computation. The motivation is not performance (the extra O(n) pass is usually negligible); it's clarity and cohesion.

**Pattern shapes to detect:**

1. `sum1 += item.a; sum2 += item.b` — two numeric accumulators with no cross-reference.
2. `list1 = append(list1, item); list2 = append(list2, item.field)` — two slice/collection accumulators.
3. `map1[key] = val1; map2[key] = val2` — two map/dict accumulators.
4. Mixed: `sum += item.price; items = append(items, item)` — a numeric and a collection accumulator with no cross-reference.

**Non-patterns (must not flag):**

1. `sum += item; count += 1` where both are used together in a final computation — structurally they look independent (neither RHS mentions the other), so this IS a false positive for v1. Documented and accepted: the user suppresses with `kibitzer:ignore split-loop-go`.
2. `max = item if item > max else max; total += item` — `max` depends on a comparison with its own previous value, which is an auto-reference, not a cross-accumulator dependency. These are still independent. This IS flagged (correct).
3. A loop with a single accumulator — never flagged.

## Tree-sitter loop node kinds (per existing `lang_config()`)

Already catalogued in `src/checkers/rules.rs`'s `nesting_kinds` per language:

| Language    | Loop kinds                                                        |
|-------------|-------------------------------------------------------------------|
| Go          | `for_statement` (covers range, C-style, `while`-style)           |
| TypeScript/JS | `for_statement`, `for_in_statement`, `while_statement`, `do_statement` |
| Python      | `for_statement`, `while_statement`                                |
| Java        | `for_statement`, `enhanced_for_statement`, `while_statement`, `do_statement` |
| Kotlin      | `for_statement`, `while_statement`, `do_while_statement`         |
| Rust        | `for_expression`, `while_expression`, `loop_expression`          |
| Bash        | `for_statement`, `c_style_for_statement`, `while_statement`      |

## Accumulator operation shapes (per language)

The checker needs to recognise "accumulator-shaped" assignments within a loop body. The key shapes:

**Go:**
- `x += expr` — `assignment_statement` with `+=` operator
- `x = append(x, expr)` — `assignment_statement` where RHS is a `call_expression` to `append` with LHS as first arg
- `x[k] = v` — `assignment_statement` where LHS is an `index_expression`
- `x++` — `inc_statement`

**TypeScript/JavaScript:**
- `x += expr` — `augmented_assignment_expression`
- `x.push(expr)` — `expression_statement` containing `call_expression` on `x.push`
- `x = [...x, expr]` — spread assignment

**Python:**
- `x += expr` — `augmented_assignment`
- `x.append(expr)` — `expression_statement` with `call` on `x.append`

**Java:**
- `x += expr` — `assignment_expression` with `+=`
- `list.add(expr)` — `method_invocation`

**Rust:**
- `x += expr` — `compound_assignment_expr`
- `x.push(expr)` — `expression_statement` with `method_call` on `x`

## Dependency grouping algorithm (v1)

For each loop body:

1. Collect all accumulator assignments. Record `(lhs_name, rhs_text)` for each.
2. For each pair of accumulators `(A, B)`, check if A's RHS mentions B's name, or B's RHS mentions A's name.
3. Use union-find (already in `src/union_find.rs`) to group accumulators by transitive dependency.
4. If the number of independent groups ≥ 2, emit a finding at the loop's start line.

**Conservative stance:** Any case where a name is ambiguous (a field access like `item.sum` where `sum` is not a local accumulator) is treated as not-a-reference. Only plain identifier matches count.

## Existing infrastructure to reuse

- `src/union_find.rs` — already exists, used for `duplicate-code`'s connected-components grouping. Perfect fit for accumulator dependency grouping.
- `src/tree_walk.rs` — `walk_preorder` utility used by most per-function checkers.
- `src/checkers/rules.rs`'s `lang_config()` — already has loop node kinds per language.
- `inventory::submit!` pattern — standard for new checkers.
- `src/node_kind.rs` — typed node kind enums for each grammar; already has `GoKind::ForStatement`, `GoKind::RangeClause`, `GoKind::AssignmentStatement`, etc.

## False-positive risk (v1)

The main false-positive vector is accumulators that are semantically dependent (a count is used to average a sum) but share no syntactic reference in their update expressions. This is the "sum + count" pattern. Expected to be low-frequency in real code but non-zero. The docs page and `kibitzer:ignore` are the mitigation.

A second vector is functional-style loops that build up results: `result1 = transform1(result1, item); result2 = transform2(result2, item)`. These look independent and may be legitimately so, or may feed into each other later. Conservative v1 flags them; the user suppresses.

## Transcript evidence

No existing transcript evidence cited per the check-ideas.md methodology (mining came back null). This is a well-known Fowler refactoring with clear structural detectability — the item was included in the backlog based on catalog reasoning, not transcript occurrence. Must clear the transcript backtest bar before shipping.

## Comparable existing checks

`go-bulk-fetch-linear-scan` (`src/checkers/go_bulk_fetch_linear_scan.rs`) is the closest comparable: it's also a structural, Go-first, single-function-body checker that identifies a specific call-and-loop combination. Its implementation pattern (walk to function declarations, then inspect body nodes) is the template for `split-loop`.
