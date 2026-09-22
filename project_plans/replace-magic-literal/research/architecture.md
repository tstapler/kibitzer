# Architecture Research: replace-magic-literal

No pre-existing hotspot/architecture analysis covers literal-node handling in
`src/rules.rs`; `project_plans/kibitzer/research/architecture.md` mentions `rules.rs` only
re: grammar registration. This is fresh analysis based on reading `src/rules.rs` (2329
lines), `src/duplicate_code.rs`, and `src/extract_class.rs` directly.

## 1. Walk shape: third walk, not folded into `walk_blocks`

`SyntaxRulesChecker::check()` (`src/rules.rs:732-742`) currently runs two whole-file
traversals:

- `walk_declarations` (`src/rules.rs:745-753`) — recurses everywhere, but only dispatches
  work when `node.kind()` is in `cfg.function_kinds`; the per-node work
  (`check_declaration`) is a **single self-contained pass** that immediately computes and
  pushes findings for all three of long-function/deep-nesting/long-parameter-list/
  flag-argument from that one declaration node. Multiplexing rules onto one walk works
  here because all four rules share both the traversal scope (function declarations) *and*
  the "resolve and emit immediately" control flow.
- `walk_blocks` (`src/rules.rs:758-766`) — recurses everywhere, dispatching to
  `check_block_for_unreachable` when `node.kind() == cfg.block_kind`. It also
  emits-immediately: a block is fully decidable in isolation, no cross-block state needed.

Literal-repetition breaks that "resolve and emit immediately" shape: whether a literal is
flagged depends on every other occurrence in the file, so it can only be decided **after**
the whole tree has been seen — a collect-then-emit two-pass, not a single-pass-per-node
dispatch.

**Recommendation: add a third top-level call in `check()`, not fold into `walk_blocks`.**
Reusing `walk_blocks`'s recursion by adding a second `if` branch for `literal_kinds`
membership would save one O(n) tree traversal, but it would smuggle a fundamentally
different control-flow shape (accumulate-into-a-map, decide-after-descent) into a function
whose only current job is "recurse, and emit immediately when a `block_kind` node is
found." That mismatch is exactly the kind of hidden coupling the file's own doc comments
elsewhere are careful to call out (e.g. `check_block_for_unreachable`'s note about not
descending into `switch`/`match` bodies "as a property of the walk, not a separate
carve-out"). A dedicated `walk_literals(node, cfg, src, &mut LiteralCollector)` — identical
recursion shape to `walk_blocks`, but collecting into an accumulator instead of pushing
`Finding`s — keeps each walk function doing exactly one thing, matching the file's existing
convention (one walk shape per traversal *scope*, one dispatch predicate per walk). The
extra traversal costs nothing that matters next to tree-sitter parse time; this file already
pays that cost twice (`walk_declarations` + `walk_blocks`) for the same reason.

`check()` becomes:

```rust
walk_declarations(tree.root_node(), &cfg, src, &mut findings);
walk_blocks(tree.root_node(), &cfg, src, &mut findings);
let mut literals = LiteralCollector::default();
walk_literals(tree.root_node(), &cfg, src, &mut literals);
emit_literal_findings(literals, &mut findings);
```

## 2. New `LangRuleConfig` fields: two, following existing precedent exactly

Two additions, not more:

- **`literal_kinds: &'static [&'static str]`** — a flat kind-list field, same shape as
  `terminal_kinds`/`nesting_kinds`. Doc comment catalogs the real per-grammar node kinds
  (verified via `to_sexp()`, same standard the rest of the table already holds itself to),
  e.g. Go's `int_literal`/`float_literal`/`interpreted_string_literal`/`raw_string_literal`,
  TS/JS's `number`/`string`, Python's `integer`/`float`/`string`, Rust's
  `integer_literal`/`float_literal`/`string_literal`. `bool`/`nil`/`null`-kind literals are
  deliberately excluded here (out of scope per the requirements' "numeric/string literal"
  framing) — worth a one-line note so a future contributor doesn't "helpfully" add them.
- **`binding_finder: fn(Node, &[u8]) -> Option<(String, Node)>`** — a function pointer,
  *not* a second raw kind-list. Extracting "the bound name and initializer node from a
  const/let-style binding" needs per-grammar field-name/positional-child logic exactly like
  `body_finder`/`params_finder` already do (`field_body`/`field_params` for the uniform
  cases, bespoke functions for Kotlin's positional-only shape). A raw
  `binding_kinds: &[&str]` field would tell you *that* a node is a binding but not *which
  child is the name* vs *which is the initializer* — that asymmetry is precisely what the
  existing table already solves via function pointers wherever grammars diverge
  structurally rather than just in kind-name spelling. Reusing that established pattern
  (rather than inventing a third field to compensate) is what keeps the struct from
  bloating: 2 new fields cover both "is this a literal" (kind-list, like `terminal_kinds`)
  and "is this a name-bound literal, and what's the name" (finder function, like
  `body_finder`).

Both new fields must also be added to `node_kind_literals_are_valid_for_their_grammar`
(`src/rules.rs:1171-1216`) — that test's `kinds` vec is the file's substitute for
compile-time grammar verification, and it currently omits nothing; a new kind-list field
added to `LangRuleConfig` without a matching addition there silently loses that safety net.

## 3. Two-pass data flow: new sibling functions, not inside the existing single-pass ones

`check_declaration` and `check_block_for_unreachable` are both single-node processors —
each is handed one `Node` and immediately decides+emits. Neither is the right home for
file-scoped aggregation; the two-pass logic needs its own pair of functions, siblings to
those two, invoked from `check()`:

- **`walk_literals`** (collect pass, matches the file's naming convention for its two
  existing top-level walks) — recurses the whole tree, and for every `literal_kinds` node:
  records it (keyed by literal value, see below) in an occurrences map, and separately
  checks `binding_finder` on ancestor/sibling shape to record const-bound values.
- **`emit_literal_findings`** (emit pass) — takes the collected state once the walk is
  done, applies the allow-list and named-constant exclusion, and pushes one `Finding` per
  qualifying literal.

**Key type**: `HashMap<String, Vec<Node>>` keyed on the literal's *raw source text
including delimiters* (e.g. `"3"` vs `3` as distinct keys) — not a semantically-normalized
value — so a numeric `1` and a string `"1"` never collide, and so `1` vs `1.0` (different
raw text, arguably the "same" value in some languages) stay distinct rather than requiring
per-language numeric normalization this check doesn't need for its stated scope.

**Reporting site — first, last, or every occurrence?** Follow `duplicate-code`'s existing,
already-shipped precedent exactly (`src/duplicate_code.rs:106-123`, tests at
`only_flags_once_for_a_longer_duplicate_run` and
`reports_last_occurrence_line_number_and_lists_all_occurrences`,
`src/duplicate_code.rs:345-386`): **report once per qualifying literal, at its *last*
occurrence's line, with every occurrence's line number listed in the finding message.**
Reasons to match rather than pick independently: (a) it's the only other whole-file
aggregate rule in this codebase, so a reviewer who already knows one convention doesn't
need to learn a second; (b) reporting at every occurrence would multiply a default-on
check's noise by occurrence count, which is the opposite of what a "keep false-positive
rate low" mandate (requirements.md's Appetite section) wants for a check that's on by
default for every kibitzer install. Reporting at the *first* occurrence instead of the last
is defensible (it's arguably the more actionable line — where a reader would introduce the
constant) but isn't a strong enough reason to diverge from the established convention,
since the message body already lists every line either way.

## 4. "Referenced elsewhere by name" exclusion: generalize `collect_identifiers`, don't reinvent it

`collect_identifiers` (`src/rules.rs:898-908`) already exists and is already used for
exactly this kind of file-wide name lookup — `flag-argument` calls it (via
`collect_condition_identifiers`) to answer "is this parameter name referenced in a
condition anywhere in the body." It currently returns a `HashSet<String>` (existence only),
which is sufficient for flag-argument's "referenced at all" question but not for
magic-literal's "referenced *elsewhere*" question — a `HashSet` can't distinguish the
binding statement's own name identifier (itself an `identifier` node the walk would visit)
from a genuine second use site.

**Recommendation**: generalize `collect_identifiers` to a counting variant —
`HashMap<String, usize>` instead of `HashSet<String>` — reusable by both call sites:
flag-argument keeps checking `.contains_key(name)` (equivalent to today's `.contains`), and
magic-literal's exclusion checks `count >= 2` (1 for the declaration's own name, ≥1 more
for a real reference) for any name `binding_finder` reported as bound to that literal's raw
text. This is a small, backward-compatible generalization of existing logic, not a new
identifier-walk.

Checked and ruled out as reuse candidates:
- `src/duplicate_code.rs` operates on normalized *line text* windows, not AST identifiers —
  no reusable node-level reference-counting logic there.
- `src/extract_class.rs` counts field-access *edges in an already-built `ArchModel`*
  (whole-repo architecture graph), an entirely different representation (cross-file,
  post-architecture-extraction) from a single-file AST walk — not applicable here.

So the answer to "does this need a second file-wide identifier-reference walk": yes, but it
should be the *same* walk shape already proven for flag-argument, generalized to counts,
not a new one built from scratch.

## 5. Disposition: Extend as-is

**Extend as-is.** `rules.rs` at 2329 lines already organizes its 5 existing rules around
exactly the shape this feature needs — one shared `LangRuleConfig` table, two walk entry
points, and per-rule logic living in small dedicated functions (`check_declaration` serving
3 rules, `check_block_for_unreachable` serving 1). Adding an 8th rule costs 2 table fields,
1 new walk, and 2 new sibling functions — it doesn't strain or duplicate that existing
shape, and no hotspot/churn evidence from this pass flags `rules.rs` as a SOLID violation
or refactor candidate. Splitting into per-rule modules is a legitimate future refactor once
the file grows meaningfully past this addition, but forcing it now would be scope creep
against the feature's own "Small... matches the size of the existing three complexity
rules" appetite (requirements.md).
