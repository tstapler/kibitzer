# Research: go-stdlib-idioms-fp

## Source files read

- `src/checkers/primitive_obsession.rs` — full file (305 lines)
- `src/checkers/rules.rs` — lines 1–1733 of 3943
- `docs/go-primitive-obsession-false-positives.md` — full file (128 lines)
- `docs/syntax-rules.md` — lines 1–80

## Checker 1: `go-primitive-obsession` — mechanism

`primitive_obsession.rs::walk()` uses `tree_walk::walk_preorder` to visit every node.
For every `ParameterList` node that is **not** a named-return list (`is_named_return_list`),
it calls `check_parameter_list`.

`check_parameter_list` fires on two shapes:
- **(a)** A single `parameter_declaration` naming ≥2 real (non-blank) identifiers of
  one primitive type — e.g. `i, j int`.
- **(b)** A run of ≥2 consecutive single-name `parameter_declaration`s sharing one
  primitive type — e.g. `a string, b string`.

**No check is made for the calling context of the `parameter_list`'s enclosing
`func_literal`.**

When `sort.Slice(s, func(i, j int) bool { ... })` is parsed, tree-sitter-go produces:
```
call_expression
  function: selector_expression ("sort.Slice")
  arguments: argument_list
    identifier (s)
    func_literal
      parameters: parameter_list        ← this fires case (a)
        parameter_declaration (i, j int)
      result: parameter_list (bool)
      body: block
```

The `parameter_list` of the `func_literal` is visited by `walk_preorder`, passes the
`is_named_return_list` guard (it is the `parameters` field, not `result`), and fires.

### Context needed to suppress

To skip this specific parameter list, we need:
1. The `parameter_list`'s parent → `func_literal` (via `parameters` field)
2. The `func_literal` is a direct child of an `argument_list`
3. The `argument_list`'s parent is a `call_expression`
4. The `call_expression`'s `function` field text matches a known stdlib allowlist

### Test file closures

`_test.go` closures are often test helpers with names like `write := func(name, body string)`.
These are not API surfaces — they are internal fixtures. The fix is to skip all
`func_literal` parameter lists when the file path ends in `_test.go`.

**Risk of false negatives**: Skipping entire test files is broader than the
callback-argument approach. It suppresses any pile-up inside a test file, including
closures that genuinely have confused argument orders. Given that test helpers are
internal and Go test files are never exported as a public API, this trade-off is
acceptable and consistent with the repo's prior `_` blank-identifier suppression.

## Checker 2: `replace-magic-literal` — mechanism

`rules.rs::walk_literals()` visits every node in the tree. For every node whose
`kind()` is in `cfg.literal_kinds` and whose `is_named()` is true, it inserts the raw
text into `collector.occurrences`.

`emit_literal_findings` applies the allow-list and excluded-const set, then fires for
any value with ≥`MAGIC_LITERAL_MIN_OCCURRENCES` (3) remaining occurrences.

**There is no check for the parent call context.** A string literal `"file"` appearing
as the 2nd, 4th, 6th argument in several `log.Warn(…)` calls accumulates 3+ occurrences
and fires.

### What "slog key position" means

The Go `log/slog` structured-logging API (and compatible wrappers) uses alternating
key-value pairs after the initial message argument:
```
slog.Info(msg, key1, val1, key2, val2, ...)
```
Position indices (0-based in the arg list):
- 0: message — a string, not a key
- 1, 3, 5, …: keys — string literals naming the attribute
- 2, 4, 6, …: values — any type

Only positions at odd indices (1, 3, 5, …) are keys.

### Detection strategy

During `walk_literals`, when a `string_literal`/`interpreted_string_literal` is found:
1. Walk up to its parent. If the parent is an `argument_list`, record the argument's
   index within that list.
2. If the `argument_list`'s parent is a `call_expression`, check the `function` field's
   text against a set of known slog-like function identifiers/selectors.
3. If the call matches **and** the argument is at an odd index ≥ 1, record the node's
   ID in a `slog_key_nodes` set.

In `emit_literal_findings`, exclude any occurrence whose `node.id()` is in `slog_key_nodes`.

### Target function patterns

Functions to match (all selectors or identifiers, checked against the text of the
`function` field of the enclosing `call_expression`):

Stdlib/slog:
- `slog.Info`, `slog.Warn`, `slog.Error`, `slog.Debug`,
  `slog.Log`, `slog.InfoContext`, `slog.WarnContext`, `slog.ErrorContext`,
  `slog.DebugContext`, `slog.With`

Standard `log` package wrappers (common in Go codebases, including stapler-squad):
- `log.Info`, `log.Warn`, `log.Error`, `log.Debug`,
  `log.InfoContext`, `log.WarnContext`, `log.ErrorContext`, `log.DebugContext`

Method-style (e.g. `logger.Warn(…)`): match any selector call where the method name is
one of `Info`, `Warn`, `Error`, `Debug`, `Log`, `InfoContext`, `WarnContext`,
`ErrorContext`, `DebugContext`, `With`.

**Note:** A broader heuristic (match any call whose arguments alternate string/non-string
starting at index 1) risks false suppressions on non-logging calls with similar shapes.
An explicit, expandable allowlist of method-name suffixes is safer and easier to audit.

### AST path

For `log.Warn("user-themes: ...", "file", name, "err", err)`:
```
call_expression
  function: selector_expression (log.Warn)
  arguments: argument_list
    interpreted_string_literal ("user-themes: ...")   ← index 0, message
    interpreted_string_literal ("file")               ← index 1, KEY
    identifier (name)                                 ← index 2, value
    interpreted_string_literal ("err")                ← index 3, KEY
    identifier (err)                                  ← index 4, value
```

The `"file"` and `"err"` nodes at indices 1 and 3 are slog keys → excluded.

### Implementation attachment point

The `walk_literals` function already collects occurrences in a single tree pass.
The cleanest attachment is:

1. Add a `slog_key_node_ids: HashSet<usize>` field to `LiteralCollector`.
2. In `walk_literals`, after inserting a literal occurrence, also check if it should be
   tagged as a slog key (parent argument_list check).
3. In `emit_literal_findings`, add the `slog_key_node_ids` filter alongside the existing
   `excluded` (named-constant) filter.

Alternatively, a separate pre-pass `walk_slog_keys` can collect the node IDs, keeping
`walk_literals` focused on collection and the slog logic isolated — matching the repo's
"two-pass collect-then-emit" pattern referenced in `rules.rs:1259`.

## Existing false-positive suppression patterns

The repo already has precedents for context-sensitive suppression:
- `is_named_return_list` — skips a `parameter_list` that is in the `result` field
- blank-identifier exclusion — skips a `parameter_list` where all names are `_`
- `is_generated` — skips `replace-magic-literal` and `extract-variable` in generated files
- `resolve_excluded_constants` — excludes literals already bound to a named constant

The proposed fixes follow the same mechanical, verifiable pattern.

## Prior false-positive docs

`docs/go-primitive-obsession-false-positives.md` tracks all historical FPs for that
check. The two new cases (stdlib callback, test closure) are **not yet listed** there.
The fixes should add entries in the **Fixed** section after landing.

No existing `docs/syntax-rules-false-positives.md` exists — if one is created, slog
keys should be the first entry.

## Risk areas

1. **AST index counting in argument_list**: tree-sitter `named_children` vs `children`.
   An `argument_list` in Go includes the comma tokens as unnamed children — index must
   be computed over `named_children` only, or the index-to-key mapping will be off by
   one per comma. Verify with `to_sexp()` or `named_child` count before implementing.
2. **Stdlib callback allowlist completeness**: `slices` package functions (`slices.SortFunc`,
   `slices.IndexFunc`, etc.) use `func(a, b T) int` or `func(T) bool` not `func(i, j int)`.
   Those may not actually trigger `primitive-obsession` since `T` is usually non-primitive.
   The list should be compile-tested against real go source snippets before landing.
3. **slog method-name suffix approach**: a function named `Debug` that is not logging
   (e.g. a graphics `Debug(msg, "color", r, "alpha", a)`) would be suppressed too. This is
   an acceptable false-negative trade-off given the low population of such functions and the
   high population of real logging calls.
