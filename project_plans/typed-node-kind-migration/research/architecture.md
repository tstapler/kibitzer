# Architecture Research: typed-node-kind-migration

Agent 3 (Architecture), SDD Phase 2. Scope: deep-dive the four flagged high-risk files
(`src/checkers/rules.rs`, `src/symbol_extract.rs`, `src/import_graph.rs`,
`src/declarations.rs`), a lighter pass on the two newer Go checkers, a staging
recommendation, migration-specific failure modes backed by real `codegen/node-types/*.json`
evidence, and a tech-debt disposition per high-risk file.

## 1. Deep-dive: the four flagged files

### `src/checkers/rules.rs` (2329 lines, 39 `.kind()` call sites, 34 comparisons)

This file is the **hardest case in the whole migration** — not because of file-type
comparisons, but because of a genuine cross-grammar-shared architecture:

- `LangRuleConfig` (a runtime struct, one instance built per `Language` by `lang_config()`,
  lines 487–702) stores per-language `&'static str` kind names as *fields* — `if_kind`,
  `block_kind`, `ternary_kind: Option<&'static str>`, `nesting_kinds: &[&str]`,
  `chain_kinds`, `else_wrapper_kinds`, `terminal_kinds`, `function_kinds`.
- A single set of **generic engine functions** — `walk_declarations` (L745),
  `walk_blocks` (L758), `check_block_for_unreachable` (L774),
  `collect_condition_identifiers` (L881), `max_nesting_depth` (L913), `walk_if_chain`
  (L960) — take `cfg: &LangRuleConfig` and a generic `tree_sitter::Node` (whose actual
  grammar is only known at runtime, via which `Language` built the config) and compare
  `node.kind()` against those string fields, e.g.:
  ```rust
  if cfg.function_kinds.contains(&node.kind()) { ... }        // L746
  if node.kind() == cfg.block_kind { ... }                     // L759
  if (node.kind() == cfg.if_kind || cfg.ternary_kind == Some(node.kind())) ...  // L887
  ```
  These functions run for all 8 `Language::ALL` variants through one code path — they
  cannot be statically typed to a single `<Lang>Kind` enum without either (a) making the
  engine generic over a `LangKind` trait, or (b) duplicating the 6 engine functions once
  per language.

- **Category breakdown** (34 comparisons):
  - (a) mechanical, single-language: ~20 — the 8 per-language `bool_param_finder`/
    `param_counter`/`body_finder`/`panic_detector`/helper fns (`go_bool_params`,
    `ts_js_bool_params`, `py_bool_params`, `java_bool_params`, `kotlin_bool_params`,
    `rust_bool_params`, `go_panic_detector`, `rust_panic_detector`,
    `go_statement_container`, `kotlin_body`, `kotlin_params`, `rust_unwrap_statement`,
    `go_param_identifier_count`, `js_ts_param_count`, `py_param_count`,
    `kotlin_param_count`, `rust_param_count`) each operate on one grammar and can convert
    directly (e.g. `GoKind::of(decl) != GoKind::ParameterDeclaration`).
  - (b) field-type comparison: ~6 — e.g. `ty.kind() != "type_identifier" ||
    ty.utf8_text(src) != Ok("bool")` (L235, Go bool detection via
    `child_by_field_name("type")`), Java's dual-kind bool check
    `ty.kind() == "boolean_type" || (ty.kind() == "type_identifier" && ...)` (L320–321).
    Still single-language, still mechanical, just comparing a field's node rather than
    the node itself.
  - (c) cross-grammar-shared: **8 sites, the generic engine functions** listed above —
    this is the trickiest category in the file and the whole migration.
  - (d) other: `collect_identifiers`'s `node.kind() == "identifier"` (L899) — universal
    across every grammar's plain identifier kind name (`"identifier"` happens to be
    spelled the same in all 8 grammars used here, but is still 8 *different* enum types
    — `GoKind::Identifier` ≠ `RustKind::Identifier` as Rust types even though both
    stringify to `"identifier"`), same shared-generic-function problem as (c).

- **Trickiest snippet** — `walk_if_chain`'s wrapper-unwrap loop (L971–987) mixes three
  kind classes from the same `cfg` in one loop: `cfg.if_kind` (this grammar's if-node),
  `cfg.chain_kinds` (Python's `elif_clause`, the only nonempty case), and
  `cfg.else_wrapper_kinds` (JS/TS/Rust's `else_clause`) — three different `<Lang>Kind`
  variants that would need three different types depending on which language is
  currently running through this one function body.

### `src/symbol_extract.rs` (1691 lines, 28 `.kind()` comparisons)

Mixed — most is per-language mechanical, but it has the same generic-shared-function
disease as `rules.rs` in two places, plus one genuine field/anonymous-token subtlety:

- (a) mechanical, single-language: the bulk — `go_receiver_type_name`,
  `go_type_declaration_symbols`, `go_struct_fields`, `go_receiver_var_name`,
  `parameter_list_names`, `shadow_candidate_names`, `direct_identifier_names`,
  `range_clause_declared_names`, `selector_access_kind`, `selector_is_call_target` are
  all Go-only (the field-access graph, per its own module comment, is "Go-only for v1").
- (c) cross-grammar-shared:
  - `enclosing_kind_name(node, source, target_kinds: &[&str])` (L333) — one function
    body, called with **different literal kind slices per language** at each call site:
    `&["class_declaration"]` for TS/JS/Kotlin, `&["class_definition"]` for Python,
    `JAVA_TYPE_KINDS` (`&["class_declaration","interface_declaration",
    "enum_declaration","record_declaration"]`) for Java (L432, 438, 466–470). Same
    shape as `rules.rs::LangRuleConfig`, just passed positionally instead of through a
    struct.
  - `callee_text_for` (L565–573) — a single `match function.kind()` arm literally lists
    kind names from **two different grammars in the same match**:
    ```rust
    match function.kind() {
        "identifier" | "selector_expression" | "member_expression" => { ... }
        _ => None,
    }
    ```
    `selector_expression` is Go's kind for `pkg.Fn`/`recv.Method`; `member_expression`
    is JS/TS's equivalent for `obj.method`. This function is called from
    `walk_calls`, itself driven generically over `Language::Go | TypeScript | Tsx |
    JavaScript` (`call_graph_supports`, L555–559). This is the single hardest
    individual snippet found across all four files: it is not merely "shared logic
    parameterized differently," it is **one match arm whose literal set is the union of
    two grammars' distinct kind vocabularies**, evaluated against a `Node` whose actual
    grammar isn't known inside the function.
  - `walk_calls` (L590–614): `ctx.cfg.function_kinds.contains(&node.kind())` — same
    `LangSymbolConfig`-driven pattern as `rules.rs`.
- (d) explicit non-goal / anonymous token (confirmed against `codegen/node-types/*.json`,
  see §4): `find_child_by_kind(node, "modifiers"|"public"|"visibility_modifier"|
  "private"|"internal"|"interface")` (L100–154, `java_is_exported`, `kotlin_is_exported`,
  `kotlin_is_interface`). Verified: in `kotlin.json`, `"public"`, `"private"`,
  `"internal"`, `"interface"` are all `"named": false`; in `java.json`, `"public"` is
  `"named": false`. None of these exist in `KotlinKind`/`JavaKind` (build.rs's
  `concrete_named_kinds` filters to `named: true`, non-supertype only) — these
  comparisons **must stay raw strings**, exactly per the requirements' Non-Goals
  section. `visibility_modifier`/`modifiers`/`interface_kinds`-membership fields
  themselves (the *named* wrapper nodes) are mechanical and safe to migrate.

### `src/import_graph.rs` (2723 lines, ~30 `.kind()` comparisons)

Looks structurally identical to `rules.rs` at first glance (`QualifiedImportLangConfig`,
explicitly commented as "mirroring `rules.rs::LangRuleConfig`'s table-driven-generic-
function precedent") but is **actually the safest of the four files** once read closely:

- `QualifiedImportLangConfig`'s two string fields, `package_decl_kind` and
  `import_stmt_kind` (L137, L143), are both marked `#[allow(dead_code)]` and explicitly
  documented as **"informational/documentation for readers"** — they are never actually
  compared against a `Node::kind()` anywhere in the generic engine
  (`build_qualified_name_language`, L184–244). The real kind-checking work happens
  inside per-language fn-pointer fields (`collect_imports: fn(...)`, `package_identity:
  fn(...)`), each pointing at a genuinely single-language function
  (`collect_go_imports`, `collect_java_imports`, `collect_kotlin_imports`,
  `go_package_identity`, `kotlin_package_identity`, etc.) — so despite the
  config-struct shape, there is **no shared function that runs `node.kind()` against a
  cross-grammar field** the way `rules.rs`/`symbol_extract.rs` do.
- (a) mechanical, single-language: the overwhelming majority — `collect_go_imports`
  (L339), `collect_go_import_specs` (L402), `go_import_spec` (L423–429),
  `collect_js_imports` (L466), `string_fragment_text` (L479), `collect_java_imports`
  (L616), `kotlin_package_identity` (L672), `collect_kotlin_imports` (L701),
  `collect_python_imports` (L994, match block), the Rust `use_declaration` handling
  (L1341+).
- (b) field-type / anonymous-token comparison: Java's static-import detection (L616–624)
  scans raw children for the anonymous `"static"` keyword token (confirmed
  `named: false` in `java.json` — see §4); the wildcard-import detection differs
  **by grammar**: Java uses a *named* `"asterisk"` node (`named: true` — mechanical,
  convertible to `JavaKind::Asterisk`), while Kotlin uses the *anonymous* punctuation
  token `"*"` (`named: false` — must stay a raw string). This pairing is itself a
  useful concrete example that "the same concept is a named kind in one grammar and an
  anonymous token in another" (see §4).
- `package_decl_kind`/`import_stmt_kind` themselves: low-value migration targets since
  they're unused at runtime — converting them to `GoKind`/`JavaKind`/`KotlinKind` would
  require making the struct generic (one more variant of the "introduce abstraction"
  question in §3) for a field that's already `#[allow(dead_code)]`. Cheapest correct
  move is to leave them as documentation-only strings, or drop them entirely as a
  small side-cleanup (out of this migration's stated scope, but worth flagging to
  Phase 3).

### `src/declarations.rs` (1213 lines, ~12 `.kind()` comparisons)

The simplest of the four flagged files — genuinely dedicated `collect_<lang>_declarations`
functions per language (`collect_go_declarations` L161, `collect_js_ts_declarations`
L251, `collect_java_declarations` L336, `collect_kotlin_declarations` L429,
`classify_python_definition`/`collect_python_declarations` L512/553), each a plain
`match node.kind() { "…" => …, _ => … }` over exactly one grammar's kind vocabulary. No
shared-engine-over-config pattern here at all — every match arm is category (a).

- (d) one explicit non-goal / anonymous-token case: `collect_kotlin_declarations`'s
  keyword scan for `"class"` vs. `"interface"` (L430–436, plus the test assertions at
  L732–733 and L754–757) — confirmed both `"class"` and `"interface"` are
  `"named": false` in `kotlin.json`. Same situation as `symbol_extract.rs`'s
  `kotlin_is_interface`: this disambiguation **cannot** move to `KotlinKind` and must
  stay a raw string comparison, which is explicitly the requirements' documented
  Non-Goal, not a gap in this migration.
- Test-file note: `declarations.rs`'s inline `#[cfg(test)] mod tests` (not a separate
  file) also contains `.kind() ==` assertions (e.g. L732, L741–742, L750, L756) — several
  of these assert on the *same* anonymous `"class"`/`"interface"` tokens and must stay
  raw strings for the same reason; others (e.g. L741 `object_node.kind(), "object_declaration"`)
  are asserting on a genuinely named/concrete kind and could migrate for extra
  compile-time safety in the test itself, though tests aren't in the stated migration
  scope (only files listed in requirements.md's Scope section are).

## 2. Newer files: `go_bulk_fetch_linear_scan.rs`, `go_table_driven_test.rs`

Both confirmed **simple, single-language, fully mechanical** — no cross-grammar sharing,
no `LangRuleConfig`-style abstraction, no anonymous-token traps found:

- `go_bulk_fetch_linear_scan.rs`: 13 comparisons, all against `GoKind` candidates
  (`for_statement`, `if_statement`, `parameter_declaration`, `short_var_declaration`,
  `range_clause`, `return_statement`, `binary_expression`, `selector_expression`,
  `identifier`). One `match function.kind() { "identifier" => ..., "selector_expression"
  => ..., _ => None }` (L200) is a `match`, but unlike `symbol_extract.rs`'s
  `callee_text_for`, every arm is a Go-only kind — mechanical, not cross-grammar.
- `go_table_driven_test.rs`: 4 comparisons (`function_declaration`,
  `parameter_declaration`, `pointer_type`, `comment`), all Go-only, all mechanical.

Both confirmed unknown-risk files from the requirements' Rabbit Holes/Open Questions can
be **downgraded to low-risk, fully mechanical** and grouped with the other Go checker
files, not with the four flagged files.

## 3. Architectural staging recommendation

Two genuinely different classes of work exist, and Phase 3 planning should treat them as
separate stories rather than one:

**Class 1 — mechanical, per-file, fully parallelizable** (18 of 22 files: all Go/Java
checker files, `go_bulk_fetch_linear_scan.rs`, `go_table_driven_test.rs`,
`declarations.rs`, all of `import_graph.rs` except the two documentation-only dead-code
fields, and the single-language helper functions inside `rules.rs`/`symbol_extract.rs`).
Each file/function gets its own enum import and swap, independently reviewable, safely
assignable to separate parallel workers with no shared-file contention risk (aside from
`rules.rs`/`symbol_extract.rs` internally, see below).

**Class 2 — the 3 genuinely cross-grammar-shared functions**, which is a real decision
point Phase 3 must make explicitly, not default into:
1. `rules.rs`'s 6 generic engine functions (`walk_declarations`, `walk_blocks`,
   `check_block_for_unreachable`, `collect_condition_identifiers`, `max_nesting_depth`,
   `walk_if_chain`) driven by `LangRuleConfig`.
2. `symbol_extract.rs`'s `enclosing_kind_name` (parameterized by kind-name slice) and
   `walk_calls`'s `cfg.function_kinds.contains(&node.kind())`.
3. `symbol_extract.rs`'s `callee_text_for` — the single match arm mixing Go's
   `selector_expression` and JS's `member_expression`.

Three options, to present to Phase 3 as an explicit choice (per the requirements' Open
Questions and Rabbit Holes sections calling this out):

- **(A) Introduce a shared abstraction** — e.g. a `LangKind: Copy + Eq` trait each
  `<Lang>Kind` enum implements, with `LangRuleConfig`/`LangSymbolConfig` becoming generic
  over it (`LangRuleConfig<K: LangKind>`), or replacing string fields with closures
  (`is_if: fn(Node) -> bool`) each language config supplies using its own typed enum
  internally. This *is* new architecture — it expands scope beyond "convert existing
  comparisons" into "design a new generic abstraction," which the Rabbit Holes section
  already flagged as a possibility Phase 3 must decide on, not default into.
- **(B) Duplicate per-language** — split each generic engine function into N
  language-specific copies (e.g. `walk_declarations_go`, `walk_declarations_java`, …),
  each internally typed to its own `<Lang>Kind`. Straightforward and fully mechanical
  per copy, but roughly 6x's `rules.rs`'s already-large engine section and 2x's the two
  `symbol_extract.rs` functions — a real maintenance-burden tradeoff (one bug fix now
  needs applying N times) that should be weighed against option A's abstraction cost.
- **(C) Leave unmigrated** — explicitly declare these ~9 sites (6 in `rules.rs`, 3 in
  `symbol_extract.rs`) out of scope, documented as a follow-up, keeping their existing
  runtime-string comparisons. This satisfies the requirements' own stated appetite-cut
  mechanism ("flag and leave as a follow-up rather than force-fitting") and is the
  lowest-risk option for a Complexity-4/zero-behavior-change migration, at the cost of
  leaving exactly the code this migration exists to protect (shared, non-obvious
  cross-grammar logic) without the compile-time backstop.

**Recommendation for Phase 3**: default to **(C) for `callee_text_for`** specifically —
it's 3 lines, low blast radius, and the awkwardness of typing one match arm against two
grammars' enums outweighs the payoff. For the 6 `rules.rs` engine functions and the 2
`symbol_extract.rs` config-driven sites, recommend **(B) duplicate-per-language** over
(A): these functions are small (`walk_declarations`/`walk_blocks` are ~5 lines each), the
existing `LangRuleConfig`/`LangSymbolConfig` machinery already isolates per-language
*data* (kind names) from per-language *logic* cleanly — introducing a generic trait (A)
would touch every call site of `lang_config()`/`lang_symbol_config()` for a benefit that
duplication achieves more simply. But this is Phase 3's call to make explicitly, not
something Phase 2 should pre-decide.

## 4. Migration-specific failure modes (Complexity 4 requirement)

Using real evidence from `codegen/node-types/*.json`:

**Failure mode 1 — anonymous token silently maps to `Other`, changing which branch
fires.** Confirmed: `kotlin.json` lists `"class"` and `"interface"` both with
`"named": false` (verified via direct JSON inspection). `symbol_extract.rs::kotlin_is_interface`
(L137–139) and `declarations.rs::collect_kotlin_declarations` (L430–436) both scan a
`class_declaration`'s raw children for a literal `"class"`/`"interface"` keyword token —
this is exactly the mechanism Kotlin uses to distinguish a plain class from an interface,
since both parse under the *same* `class_declaration` node kind. If someone doing this
migration mechanically saw `c.kind() == "interface"` and reflexively "fixed" it to
`KotlinKind::of(c) == KotlinKind::Interface` (which doesn't exist — `Interface` isn't a
variant here, since anonymous kinds are excluded from the enum), the code would fail to
compile — the *good* outcome. The dangerous version is if someone instead widened
`Other`'s handling or added an ad hoc variant that doesn't discriminate `class` from
`interface`: every Kotlin interface in a real repo would then silently misclassify as a
class in the declaration graph — no compile error, no test failure unless the fixture
set happens to include a Kotlin interface (confirmed the file's own test suite does cover
this at L737, so a full-suite run would catch it — but a narrower re-run, e.g. `cargo
test declarations::` filtered to a different module, would not).

**Failure mode 2 — the same string names a different concept in two node-types.json
entries within one grammar.** Confirmed: `kotlin.json` contains **two separate entries**
both with `"type": "import"` — one `"named": true` (the concrete `import` statement node
— what `import_graph.rs::collect_kotlin_imports`'s `node.kind() == "import"` check at
L702 actually matches) and one `"named": false` (the anonymous `import` keyword token
that appears as that statement's own first child). `build.rs`'s `concrete_named_kinds`
correctly dedupes to the named entry only, so `KotlinKind::Import` unambiguously means
the statement node — but a **future or overlooked call site** that tried to find the
`import` keyword child by kind string (the way `find_child_by_kind(node, "class")` does
for Kotlin's class/interface keyword) would, if migrated naively, either fail to compile
(good, if using `KotlinKind::of` and getting `Other`) or — more dangerously — a
half-careful migration could alias the keyword check to the statement's own
`KotlinKind::Import` variant by pattern-matching on the *parent* node's kind instead of
the actual child, silently changing what's being tested. No such site currently exists in
these 6 files (confirmed by the grep pass above), but this is the concrete class of bug
this migration must watch for when new match sites are added later, not merely refactor
existing safe ones.

**Failure mode 3 — same real-world concept (wildcard-import marker) is a named,
concrete kind in one grammar but an anonymous punctuation token in another.** Confirmed:
`java.json`'s `"asterisk"` is `"named": true` (Java's `import a.b.*` wildcard marker is
its own concrete node); `kotlin.json`'s `"*"` is `"named": false` (Kotlin's equivalent
wildcard marker in `import a.b.*` is bare punctuation, no dedicated node). `import_graph.rs`
already handles this correctly today (L633–636 for Java uses `c.kind() == "asterisk"`;
L706–710 for Kotlin uses `c.kind() == "*"` via raw-child scan) — but a migration that
assumed "wildcard-marker detection generalizes the same way across the two languages
sharing `QualifiedImportLangConfig`" (as `rules.rs`'s pattern might tempt someone to
assume) would find Java's `"asterisk"` migrates cleanly to `JavaKind::Asterisk` while
Kotlin's `"*"` **cannot** migrate at all (no such `KotlinKind` variant exists) — the two
call sites that look structurally parallel in the source must be treated completely
differently, and treating them the same (e.g. by "fixing" Kotlin's raw check to match
Java's typed one) would either not compile or, if forced through `Other`, silently break
wildcard-import detection for every Kotlin file with a star import.

## 5. Tech Debt Disposition (four high-risk files)

- **`src/checkers/rules.rs`: Isolate via seam.** The single-language helper functions
  (~26 of 34 sites) should migrate directly — no seam needed. The 6 cross-grammar engine
  functions should be **left on the existing `LangRuleConfig` string-based seam for this
  migration**, with Phase 3 explicitly deciding (per §3) whether a future story
  duplicates them per-language or introduces a generic trait; forcing that decision
  *into* this zero-behavior-change migration would violate the requirements' own
  "no new checker functionality... this migration must not change what any checker
  flags" constraint by conflating a refactor-scope-expansion with a pure type-safety
  swap.
- **`src/symbol_extract.rs`: Isolate via seam**, same reasoning as `rules.rs`, scoped
  more narrowly: only 3 functions (`enclosing_kind_name`, `walk_calls`'s
  `function_kinds.contains` check, `callee_text_for`) need the seam; the remaining ~25
  comparisons (Go-only field-access graph, per-language `classify_node` branches once
  the language is already known via `match language { ... }`) migrate directly. Note
  `classify_node`'s `language == Language::Go { ... }` branches (L418–453) are *already*
  a de facto per-language dispatch inside one function — those branches convert cleanly
  once each arm's `.kind()` checks reference the already-known-language's enum.
- **`src/import_graph.rs`: Extend as-is (mostly mechanical, no seam needed).** Despite
  superficial resemblance to `rules.rs`'s pattern, the actual kind-comparison logic
  already lives in per-language fn pointers, not a shared generic function — this file
  needs no new abstraction. The two `#[allow(dead_code)]` documentation-only string
  fields (`package_decl_kind`, `import_stmt_kind`) can be left as plain strings (they're
  never compared to anything) or dropped in a small side-cleanup; either is out of this
  migration's stated scope.
- **`src/declarations.rs`: Extend as-is.** No shared-function pattern at all — every
  comparison is a plain per-language `match`. The one non-goal case (Kotlin's anonymous
  `"class"`/`"interface"` keyword scan) is explicitly out of scope per the requirements
  and needs no seam, just a code comment (already present) noting why it stays a raw
  string.

## Summary for Phase 3 planning

- Of the ~34+28+30+12 = ~104 total comparisons across the four flagged files, roughly
  **85-90 are mechanical or field-type (category a/b)** and can be migrated directly,
  file-by-file, fully parallel.
- **~9 sites are genuinely cross-grammar-shared** (6 in `rules.rs`'s generic engine, 3 in
  `symbol_extract.rs`) and need an explicit Phase 3 decision: duplicate-per-language
  (recommended for the 8 `LangRuleConfig`/`LangSymbolConfig`-driven ones) vs. leave
  unmigrated (recommended for `callee_text_for` specifically).
- A handful of sites (Kotlin's anonymous `class`/`interface`/`public`/`private`/
  `internal` keyword tokens, appearing in both `symbol_extract.rs` and
  `declarations.rs`) are confirmed, evidence-backed **non-goals** per the requirements —
  not gaps, not risks, just correctly out of scope.
- `go_bulk_fetch_linear_scan.rs` and `go_table_driven_test.rs` are confirmed low-risk,
  fully mechanical — safe to fold into the same batch as the other Go checker files
  rather than treated as unknown-risk.
