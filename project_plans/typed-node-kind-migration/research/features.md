# Research: Feature landscape — typed-node-kind migration edge cases

Agent 2 (Features researcher), SDD Phase 2. All line numbers verified 2026-09-22 against
working tree at `ce29a19` (see `gitStatus` in session context).

## 1. `.kind()` usage patterns found across the 22 in-scope files

`grep -c "\.kind(" <file>` totals (raw signal, includes non-migration-relevant calls like
`.start_position()`/etc. filtered out by eye): `rules.rs` 39, `import_graph.rs` 34,
`symbol_extract.rs` 30, `declarations.rs` 23, `go_error_context.rs` 20,
`go_bulk_fetch_linear_scan.rs` 14, `god_class.rs` 11, the rest single digits.

No `.kind_id()` call sites exist anywhere in `src/` — that form is a non-issue for this
migration (nothing to migrate, nothing to watch for).

**Pattern A — direct literal equality (the common case, mechanical).**
```rust
// src/checkers/complexity_tests.rs:68
n.kind() == "function_declaration"
```
Majority of hits across the Go/Java single-language checkers are this shape and migrate
1:1 to `GoKind::of(n) == GoKind::FunctionDeclaration` (or `<Lang>Kind::of(n) == ...`).

**Pattern B — `match node.kind() { "a" => ..., "b" => ..., _ => ... }`.**
```rust
// src/import_graph.rs:995 (Python-only function, collect_python_imports)
match node.kind() {
    "import_statement" => { ... }
    "import_from_statement" => { ... }
    _ => {}
}
```
Also `src/declarations.rs:162,252,430,556` and `src/import_graph.rs:1261,1303`. All of the
`match` blocks found are inside single-language-named functions (`collect_python_imports`,
etc.), so each migrates to a single `<Lang>Kind` match — no cross-grammar branching needed
for these specific match arms (contrast with §3 below).

**Pattern C — kind checked inside a larger boolean expression, often with a config lookup.**
```rust
// src/checkers/rules.rs:922
if child.kind() == cfg.if_kind || cfg.nesting_kinds.contains(&child.kind()) {
// src/checkers/go_blank_imports.rs:123
if prev.kind() == "comment" && leading_comment_rows.contains(&prev.start_position().row) {
```
The literal half (`"comment"`) is mechanical; the `cfg.if_kind`/`cfg.nesting_kinds` half is
not — see §2/§3.

**Pattern D — kind used as a `.contains(&…)` lookup against a `&[&str]` slice (not a
HashSet, but the same "is this kind in this open set" shape).**
```rust
// src/checkers/complexity.rs:111
if function_kinds.contains(&node.kind()) {
// src/symbol_extract.rs:336 (enclosing_kind_name, shared across TS/JS/Python/Java/Kotlin)
if target_kinds.contains(&n.kind()) {
```
Occurs in `symbol_extract.rs` (2x), `go_table_driven_test.rs` (1x, `LITERAL_KINDS`),
`complexity.rs` (2x), `rules.rs` (4x). This is the dominant "dynamic, not literal-at-site"
pattern — see §2.

**Pattern E — `.kind().contains(...)` substring check (not equality).**
```rust
// src/checkers/rules.rs:784
if stmt.kind().contains("comment") {
```
Only one occurrence (`rules.rs:784`). A substring check against a kind string is
inherently not a single enum-variant equality — it's checking "is this any one of the
comment-like kinds for this grammar" without enumerating them. Migrating this one requires
either enumerating the grammar's actual comment-kind variants (`LineComment`, `BlockComment`,
etc., if the grammar splits them) or deciding this specific comparison is a NOT-cleanly-
mechanical case per the Non-Goals policy. No `starts_with`/`ends_with` variants exist
anywhere in scope — this is the only substring-style kind check in the whole set.

**Pattern F — `.kind().to_string()` (kind captured as an owned `String`, not compared
inline).**
```rust
// src/tree_walk.rs:61
kinds.push(n.kind().to_string());
```
Only one occurrence, in `tree_walk.rs` (a generic-walker debug/collection helper, not a
per-node equality check). This doesn't have an equality comparison to migrate at this call
site at all — the collected strings are presumably compared elsewhere, or used for
diagnostics/tests. No `.into()` conversions of `.kind()` exist anywhere in scope.

**No HashMap/HashSet-keyed-by-kind usage exists.** Every "set membership" check in scope
(Pattern D) uses a `&'static [&'static str]` slice with `.contains()`, never an actual
`HashMap`/`HashSet`. This matters for migration mechanics: `<Lang>Kind` derives `PartialEq`
(confirmed via `src/node_kind.rs` re-exports + generated enum, which the codebase already
relies on for `==`), but turning a `&[&str]` allowlist into a `&[<Lang>Kind]` allowlist and
keeping `.contains()` working requires `<Lang>Kind` to also be `Copy`/`PartialEq` (it already
must be, since equality comparisons are the whole point) — not a blocker, just confirms the
generated enum needs no additional trait derives beyond what direct `==` comparisons already
require.

## 2. Dynamically-derived kind strings (can't become a direct compile-time enum comparison)

Two real instances found, both structurally identical in shape: a struct field of type
`&'static [&'static str]` (or `&'static str` for a single kind) is *populated* with literals
at one per-language construction site, but *compared* generically in shared code that
receives the field as a parameter, not a literal.

- **`src/checkers/rules.rs`'s `LangRuleConfig`** (defined `rules.rs:72`,7 fields carry kind
  strings: `function_kinds`, `if_kind`, `nesting_kinds`, `else_wrapper_kinds`, `chain_kinds`,
  `terminal_kinds`, plus optional `ternary_kind`/`block_kind`). One `LangRuleConfig` instance
  per language is constructed with literal kind strings (e.g. `rules.rs:493`
  `if_kind: "if_statement"`, `rules.rs:637` Kotlin's `if_kind: "if_expression"`), but the
  actual comparisons happen generically at `rules.rs:746,759,797,887,914,922,974,977` —
  e.g. `if node.kind() == cfg.if_kind || cfg.nesting_kinds.contains(&child.kind())`. `cfg` is
  a runtime value (a reference into a `&'static LangRuleConfig`), not a literal at the
  comparison site, so `node.kind() == cfg.if_kind` cannot mechanically become
  `GoKind::of(node) == GoKind::IfStatement` without first resolving *which* `<Lang>Kind` type
  applies — see §3, since this is the same generic-function-across-grammars case.
- **`src/symbol_extract.rs`'s `LangSymbolConfig`** (`symbol_extract.rs:42`, fields
  `type_kinds`, `interface_kinds`, `function_kinds`) and its standalone
  `enclosing_kind_name(node, source, target_kinds: &[&str])` helper (`symbol_extract.rs:328`)
  — `target_kinds` is a parameter, populated by different callers with different
  per-language literal slices (e.g. `&["class_declaration"]` for TS/JS, but
  `&["class_declaration", "interface_declaration", "enum_declaration",
  "record_declaration"]` for Java, per the doc comment at `symbol_extract.rs:326-332`).
  Comparisons at `symbol_extract.rs:336,592` (`target_kinds.contains(&n.kind())`,
  `ctx.cfg.function_kinds.contains(&node.kind())`) are the same shape as `rules.rs` above.

Both are also referenced directly by `rules.rs`'s own runtime validator,
`node_kind_literals_are_valid_for_their_grammar` (`rules.rs:1173-1179`), which gathers every
`LangRuleConfig` field into one `Vec<&str>` and checks each against
`tree_sitter::Language::id_for_node_kind` at test time — the exact mechanism the migration's
Baseline section calls "narrower, complementary." If `LangRuleConfig`'s fields are converted
from `&'static [&'static str]` to `&'static [GoKind]`/`&'static [JavaKind]`/etc. (per
language, since there's no single `Kind` supertype), that runtime test becomes fully
redundant for `rules.rs` specifically (its literals no longer exist as strings to validate)
— which the requirements' Non-Goals section already anticipates ("in which case removing the
now-dead test is in scope for that file's story only").

No config-file-driven (i.e. read from disk/JSON/TOML at runtime) kind strings were found
anywhere in scope — `.claude/inspect.json`-style external config drives *check selection*,
not node-kind literals. All dynamic cases found are compile-time `&'static` Rust data,
just not literal *at the comparison site*.

## 3. Shared generic functions comparing kinds across multiple grammars

Two concrete cases, both requiring the same kind of branch-on-language dispatch to migrate
cleanly — this is the crux of Rabbit Hole #3/#4 in requirements.md.

**Case 1: `rules.rs`'s `LangRuleConfig`-driven checks** (§2 above). Every function taking
`cfg: &LangRuleConfig` (e.g. the `contains-check`/`unreachable-code`/`flag-argument` walkers
around `rules.rs:740-1000`) runs identically for all 7 languages by construction — the
generic logic ("is this child kind the if-kind, or one of the nesting kinds") is
language-agnostic; only the kind *strings* differ per language. To migrate,
`LangRuleConfig`'s kind fields can't stay `&'static [&'static str]` typed generically,
because there is no single `NodeKind` trait/enum spanning `GoKind`/`JavaKind`/etc. (each
`<Lang>Kind::of()` takes and matches against one specific grammar's `Node`, and the enums
share no common variant set or trait per `src/node_kind.rs`). Two migration shapes are
possible, both adding real complexity beyond a find-replace:
  - (a) Make `LangRuleConfig` itself generic over a `Kind` associated type/trait bound
    that all 8 `<Lang>Kind` enums implement (e.g. `fn of(node: Node) -> Self`,
    `fn as_str(&self) -> &'static str`) — this is new trait-design work, arguably crossing
    into "new codegen mechanism," which the Non-Goals section rules out ("Building any new
    codegen or enum-generation mechanism ... is complete and unchanged by this work").
  - (b) Leave `LangRuleConfig`'s cross-language-generic fields as raw `&'static str` (i.e.
    explicitly **exclude `rules.rs`'s config-driven comparisons from this migration** while
    still migrating the file's other, non-config-driven literal comparisons like
    `rules.rs:784`'s `.contains("comment")` or the ~large minority of direct-literal hits).
    This keeps the crate's actual runtime behavior identical and avoids inventing new
    generic-enum infra, at the cost of `rules.rs`'s riskiest, highest-value literals
    (the ones a typo is most likely to hide in, since they're spread across 7 far-apart
    construction sites) staying un-migrated. **Recommend (b)** given the Non-Goals framing —
    Phase 3 planning should make this an explicit scoping decision for `rules.rs`, not an
    implementation detail discovered mid-story.

**Case 2: `symbol_extract.rs`'s `walk_calls`** (`symbol_extract.rs:590-614`), gated to only
4 languages by `call_graph_supports` (`symbol_extract.rs:554-559`: `Go | TypeScript | Tsx |
JavaScript`). It hardcodes `node.kind() == "call_expression"` (`symbol_extract.rs:598`) as a
single literal shared across all 4 languages' trees, not sourced from `LangSymbolConfig` at
all. Verified (via each language's generated `<Lang>Kind` enum under
`target/debug/build/*/out/*_kind.rs`) that **all 4 gated languages** do have a
`CallExpression` variant mapping to `"call_expression"` — Go, TypeScript, Tsx, and
JavaScript all share that exact grammar node name, so today's single literal happens to be
safe for exactly the 4 languages it's gated to. (Cross-check: Python's real call-node kind
is `"call"`/`PythonKind::Call`, and Java's is `"method_invocation"`/`JavaKind::MethodInvocation`
— `JavaKind` has **no** `CallExpression` variant at all — so if `call_graph_supports` were
ever widened to Python or Java without updating this literal, the check would silently never
fire for those languages, exactly the failure class this migration exists to prevent. That's
not a live bug today only because the gate excludes them.) To migrate this one literal to a
typed comparison, the function needs to branch on `ctx.language` and compare against
whichever of `GoKind::CallExpression`/`TypeScriptKind::CallExpression`/
`TsxKind::CallExpression`/`JavaScriptKind::CallExpression` matches — a 4-way `match
ctx.language { ... }` wrapping the one comparison, not a 1:1 substitution. The sibling
function `callee_text_for` (`symbol_extract.rs:565-571`) has the identical shape one level
down: `match function.kind() { "identifier" | "selector_expression" | "member_expression" =>
... }` mixes Go's `selector_expression` and JS/TS's `member_expression` in one match arm for
one shared literal set — same 4-way-branch requirement.

## 4. Test files with raw kind-string literals

Checked every in-scope `*_tests.rs` (only `complexity_tests.rs` is in the 22-file scope
list; no other `*_tests.rs` file matched the scope grep).

`src/checkers/complexity_tests.rs` has 3 hits, all in one helper:
```rust
// complexity_tests.rs:68 (fn function_line, a Go-only test helper)
n.kind() == "function_declaration"
```
This is a legitimate migration candidate for consistency — it's a direct Go-grammar literal
equality check in a single-language helper, structurally identical to Pattern A in §1, and
`complexity_tests.rs` is explicitly named in the requirements' scope list. No reason to
exclude it: it carries the exact same typo risk as production code (a typo'd kind string in
a test helper just makes the *test* silently useless, not the checker — arguably worse,
since it removes the safety net without any signal).

`src/import_graph.rs`'s inline `#[cfg(test)]` module (not a separate `*_tests.rs` file, and
not separately listed in scope — but its parent file `import_graph.rs` *is* in scope, so its
test module's literal at `import_graph.rs:2229`, `assert_eq!(name_field.kind(),
"aliased_import")`, is inside an in-scope file). Since the file itself is in scope and this
assertion is a direct-literal equality check on a single-language (Python) node, it should
be migrated alongside the rest of `import_graph.rs`'s conversions rather than singled out —
no reason for test code within an in-scope file to be treated differently from production
code in the same file. No other in-scope file's test module was found to construct a raw
kind literal (spot-checked `rules.rs`, `symbol_extract.rs`, `declarations.rs`, `god_class.rs`
test modules — no `assert_eq!`/`.kind() ==` hits against literal kind strings besides the
one cited).

## 5. Prior similar migrations in this repo's history

`git log --oneline --all | grep -iE "migrat|node_kind|typed.*kind|enum.*kind"` returns only
two relevant commits, both from this same effort's own lead-up — **there is no prior
string-to-enum migration in this repo's history to draw staging lessons from**:

- `e830c11` *feat: generate typed node-kind enums from vendored grammar node-types.json* —
  the infra-only commit that added `build.rs`/`src/node_kind.rs` itself. Explicitly
  "Infrastructure only: no call site is migrated yet," deferring the actual migration to
  `/sdd:full` — i.e. this project.
- `659e623` *refactor: add iterative tree-sitter preorder walker, migrate 5 checkers* — not
  a kind-string migration, but a similarly-shaped "adopt new shared infra across many
  checkers" refactor (recursive→iterative tree walk). Its one transferable lesson, stated in
  its own commit message: it migrated the 5 checker files *the session was already touching*
  "per this repo's own convention: adopt new infra where already touching the code, not as a
  crate-wide rewrite," explicitly leaving other checkers with the identical pattern
  unmigrated in that commit. This matches — and independently confirms — the current
  requirements' own Risk Control section (land each file as its own story/commit, not one
  monolithic commit) rather than contradicting it; no new staging lesson beyond what's
  already planned.

`docs/typed-node-kinds.md` (read in full) adds no additional detail beyond what
`requirements.md` already cites from it — confirms the same 4 high-risk files and the same
`rules.rs`-runtime-test-redundancy note, and confirms nothing has been migrated yet ("Nothing
yet").

## Summary of migration-relevant findings not otherwise called out in requirements.md

- One `.kind().contains("comment")` substring check (`rules.rs:784`) — not equality, needs
  its own scoping decision (§1 Pattern E).
- Bare-token/punctuation literals already found in scope that must stay raw strings per the
  Non-Goals section: `import_graph.rs:709` (`c.kind() == "*"`, Kotlin star-import),
  `declarations.rs:435,756` (`.kind() == "class"`, a bare keyword token, not a named node).
  These aren't near-misses — Phase 3 should list them explicitly as intentional
  no-ops so implementers don't waste time trying to force them into `Other`.
- `rules.rs` and `symbol_extract.rs` each contain one generic-across-languages construct
  (`LangRuleConfig`'s config-driven fields; `walk_calls`/`callee_text_for`'s shared literal
  set) that cannot mechanically convert to typed enum comparisons without either (a) new
  cross-grammar trait infra (arguably out of scope per the Non-Goals section) or (b) an
  explicit per-file scoping decision to leave those specific comparisons as raw strings
  while still migrating the same file's non-generic comparisons. Recommend Phase 3 treat
  this as a required scoping decision, not something discovered mid-implementation.
