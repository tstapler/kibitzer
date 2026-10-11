# Adversarial Review: long-function-flat-literal

## Lens 1 — Correctness

**Challenge 1.1**: The `has_logic_descendants` walk stops at `cfg.function_kinds`
boundaries. But Go's `func_literal` is NOT in `cfg.function_kinds` — it's in
`cfg.nesting_kinds`. Would a composite literal that contains a func_literal be
incorrectly exempted?

**Answer**: No. A `func_literal` node is in `cfg.nesting_kinds` for Go (`"func_literal"`
is listed at line 894). The `has_logic_descendants` check returns `true` on the first
`nesting_kinds` hit, which includes `func_literal`. So a composite literal containing an
inline function is correctly NOT exempted. The walk stops at `function_kinds`
(`function_declaration`, `method_declaration`) to avoid false positives from nested
named functions inside a literal value (which would be unusual but not impossible in
some languages). In Go, `func_literal` is caught by `nesting_kinds` before the
`function_kinds` early-exit is even tested. The logic is correct.

**Challenge 1.2**: The predicate counts `container.named_children()`. In Go, a
`statement_list` node's named children include `expression_statement` wrappers around
plain expressions. Would `return []tc{...}` parse as an `expression_statement` wrapping
a `return_statement`, causing the one-named-child check to see `expression_statement`
instead of `return_statement`?

**Answer**: No. In Go's grammar, `return_statement` is a direct statement kind, not
wrapped in `expression_statement`. `expression_statement` wraps expression-as-statement
constructs (`f()`, `x++`, etc.), not `return`. Cross-check: `go_panic_detector`
(line 611) checks for `GoKind::ExpressionStatement` on the outer node and then
`GoKind::CallExpression` inside it — it never wraps a `return_statement`. Safe.

**Challenge 1.3**: What if a Go composite literal has a trailing comma followed by a
comment, and tree-sitter parses the comment as a named child of the `literal_value`?

**Answer**: Tree-sitter's named children only include semantically meaningful nodes;
comments are `comment` nodes which are typically unnamed (invisible to `named_children`).
The walk uses `named_children`, so comments are invisible. No issue.

## Lens 2 — Precision trade-offs

**Challenge 2.1**: A function that constructs a complex data structure where each entry
calls a helper — `{name: newName("x"), input: transform(1)}` — is NOT a flat literal and
would correctly fire. But a function where entries use struct field access —
`{name: base.Name, input: base.Input}` — is borderline. `base.Name` in Go parses as a
`selector_expression` (a `call_kinds`? No — `call_expression` for Go is a function call,
not a field access).

**Answer**: `cfg.call_kinds` for Go is `&["call_expression"]`. A `selector_expression`
(field access) is not in `call_kinds`. So `{name: base.Name}` passes the logic check and
is correctly exempted — field access is pure data retrieval, not logic. This is the right
behavior: a fixture that references a shared base struct's fields is still a flat data
table.

**Challenge 2.2**: A TypeScript function returning an `object` (not an array) — e.g.
a large record/map — also qualifies for exemption. Is an object-returning function
a valid "flat data" case?

**Answer**: Yes. A function `function registry(): Record<string, Handler> { return {...} }`
where the object is a flat key-value map is exactly the same pattern — large, not
extractable. Exempting `"object"` as a `collection_literal_kinds` entry for TypeScript
is correct.

## Lens 3 — Completeness

**Challenge 3.1**: Python's `return [(name, input, want), ...]` uses a `list` containing
`tuple` nodes. Would `"list"` in `collection_literal_kinds` handle this, given that
`tuple` is the actual child kind?

**Answer**: Yes. The predicate checks whether the *return value's* kind is in
`collection_literal_kinds` — i.e., whether the `list` node is what's returned. The
`tuple` entries inside the list are not checked against `collection_literal_kinds`; they
are checked by `has_logic_descendants` for control-flow/call nodes. A list of tuples
passes all four steps. Correct.

**Challenge 3.2**: Should `"tuple"` also be in Python's `collection_literal_kinds`?
`def cases() -> tuple[TC, ...]: return TC(...), TC(...), ...` uses a bare tuple as the
return value in Python.

**Answer**: A bare tuple `return a, b, c` in Python parses as a `return_statement`
whose named child is an `expression_list` (or sometimes `tuple`), not a `list`. If the
grammar uses `tuple` for `return x, y`, then yes, `"tuple"` should be added. This
requires grammar verification (Task 1.1.1f mentions `to_sexp()` verification). If the
grammar uses `expression_list`, that should be added instead. The task correctly defers
this to a `to_sexp()` check.

## Lens 4 — Risks not covered in `pitfalls.md`

**Challenge 4.1**: A Rust function returning `&[Case]` where the return is a slice
literal `&[Case { ... }, ...]` — is this a case the exemption should cover?

**Answer**: In Rust's tree-sitter grammar, `&[...]` is a `reference_expression` wrapping
an `array_expression`. `array_expression` is not a commonly-idiomatic fixture pattern in
Rust the way `composite_literal` is in Go, and it may contain logic. The plan correctly
sets `collection_literal_kinds = &[]` for Rust and adds a comment explaining why. A Rust
user who wants this exemption can file a follow-up once the Go/TS/Python case is proven.
The plan correctly bounds scope.

**Challenge 4.2**: The backtest (Epic 3) uses transcript history to check for suppressed
real findings. But transcript history may contain no Go functions matching the flat-literal
shape over 40 lines — making the backtest uninformative.

**Answer**: This is a valid limitation of `kibitzer check backtest`. The plan explicitly
requires also sampling the real-world corpus (kubernetes/kubernetes test packages, Task
3.1.1b), which is large enough that flat-literal fixtures over 40 lines almost certainly
exist. If transcript backtest finds zero matches, that's documented in the PR description
with the corpus result, and the implementation proceeds.

## Verdict

No blocking correctness issues found. The four-step predicate is sound for Go, TypeScript,
and Python. Java/Kotlin/Rust correctly use `&[]`. The `function_kinds` vs `nesting_kinds`
distinction for closures is handled correctly by the nesting-kinds check firing first.
The plan should proceed as written.
