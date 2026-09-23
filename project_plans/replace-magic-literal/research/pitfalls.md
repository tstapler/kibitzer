# Pitfalls: replace-magic-literal

Research for SDD Phase 2, Agent 4. All claims below are VERIFIED against this repo's
source/docs at the paths cited, unless marked INFERRED.

## 1. The `duplicate-code` "2+ is too aggressive" precedent — directly parallel, and worse here

`docs/refactoring-catalog-analysis.md:130-136` (VERIFIED, read directly):

> Implemented as `duplicate-code`... on 2026-08-19, ahead of a confirmed transcript
> occurrence. A full-corpus backtest did turn up real duplication, but with volume
> inflated by flagging every extra copy of a block past the first repeat — since a lot
> of two-copy repetition is benign, defensible fixture/boilerplate, the checker now only
> flags once a block occurs 3+ times.

This is an exact precedent for the requirement's proposed `>=2` threshold
(requirements.md's "Proposed Rule" section, and AC2). `duplicate-code` shipped at
"any repeat" and had to be walked back to 3+ after a real backtest showed two-copy
repetition is routinely benign. `docs/duplicate-code-false-positives.md:47-55`
(VERIFIED) adds the specific mechanism that made two-copy noisy: table-driven test
literals — "roughly 60%" of sampled findings across the kubernetes/cassandra backtest
corpus were "genuine, verbatim-repeated table-driven test literals... real duplication
by the checker's definition, but debatable value as a lint since table-driven tests
conventionally repeat near-identical literals per case."

**The risk is structurally worse for `replace-magic-literal` than it was for
`duplicate-code`**, for two reasons:
- `duplicate-code`'s unit is a 6-line, 60-character window (`MIN_BLOCK_CHARS`/
  `MIN_BLOCK_LINES`, `docs/duplicate-code-false-positives.md:10-13`) — a fairly large,
  specific match. A single literal (e.g. the string `"pending"` or the number `404`) is
  a far smaller, far more common unit, so the base rate of incidental 2-occurrence
  matches is inherently higher.
- The requirement's own scope note (requirements.md:105-107) explicitly anticipates
  table-driven test/fixture files as the main false-positive vector for *this* check
  too, and defers mitigation entirely to `scope` excludes or `.kibitzer/accepted/` —
  i.e. per-repo config, not a built-in threshold or heuristic. Given `duplicate-code`
  needed a *catalog-wide* threshold change (not a per-repo opt-out) to control the
  exact same table-driven-test noise, shipping `replace-magic-literal` at `>=2` with no
  higher default threshold and no test-file carve-out risks reproducing the same
  overcorrection cycle, but as a *default-on* check from day one (`duplicate-code` did
  it retroactively; this one would ship pre-corrected knowledge and still choose >=2).

**Recommendation surfaced by this precedent** (for Phase 3 planning, not decided here):
consider whether backtesting should test both `>=2` and a `>=3` variant before locking
the threshold, exactly the axis `duplicate-code` already proved matters — rather than
treating AC2's ">=2" as fixed and only discovering the same problem after ship, per
"a checker that hasn't been run against this corpus at least once isn't done."

## 2. Real-world noise proxies from other checks' false-positive logs

Read `docs/duplicate-code-false-positives.md`, `docs/comment-quality-false-positives.md`,
`docs/file-complexity-false-positives.md`, and `docs/backtest-repos.md` in full
(VERIFIED). One clear, repeated pattern across *every* checker that has a
false-positive log: **generated code was the first real-world hit in every single
case**, and none of the checks anticipated it before backtesting:

- `duplicate-code`: `docs/duplicate-code-false-positives.md:21-40` — flagged
  `generated.pb.go` (kubernetes), `session.pb.go` (stapler-squad, 251 occurrences in one
  file), `zz_generated.deepcopy.go`, `gnostic.go` (vendored), and ent ORM codegen. Fixed
  by adding a `file_size::is_generated` early-return.
- `file-complexity`: `docs/file-complexity-false-positives.md:19-27` — same story,
  `zz_generated.validations.go` and a protoc-gen-gogo `generated.pb.go`. Same fix.
- `go-table-driven-test-candidate`: its own source comment
  (`src/go_table_driven_test.rs:53-60`, read directly) explains it added the
  `is_generated` guard *proactively*, citing the `duplicate_code.rs` precedent by name —
  "generated test scaffolding (protobuf conformance suites, codegen'd table tests)
  mechanically repeats near-identical TestXxx functions."

Literal repetition is arguably the single pattern *most* exposed to this, since
generated code (protobuf field-number constants, gRPC status-code switches, codegen'd
enum-to-string tables, ORM column-index constants) is dense with mechanically repeated
numeric/string literals by construction — more so than repeated 6-line blocks or high
cyclomatic complexity, both of which need actual branching/control-flow to trigger.

Beyond generated code, the specific noise categories the task named are all corroborated
as live risks by what other checks found in the same backtest corpus:
HTTP status codes, array indices, port numbers, retry counts, and buffer sizes are all
literals that legitimately repeat 2+ times in ordinary, non-code-smell code (e.g. a
handler file testing `200` in three different assertions, or a config file with `8080`
in both a listen address and a doc comment example). None of these currently have any
dedicated false-positive doc — `replace-magic-literal` would be the first checker to
inspect literal *values* at all, so there is no existing false-positive precedent
narrower than the `duplicate-code`/`file-complexity` generated-code pattern above to
lean on. This is a gap the backtest step is specifically there to fill (AC 7-8), not
something resolvable from existing docs alone.

## 3. Named-constant exclusion — false-negative/false-positive risk in the logic itself

No existing kibitzer checker does name-binding/reference tracking of this shape
today — VERIFIED by `grep -n "const_declaration\|lexical_declaration"
src/rules.rs` returning zero matches, and by reading `LangRuleConfig`'s full field
list (`src/rules.rs:72-144`): every field is either a node-kind constant or a pure
per-node-shape function (body/param finders, unwrap/panic detectors). None hold any
cross-node state, and none do identifier-reference resolution. The requirement
document's own Baseline section already flags this precisely: "No literal-related node
kinds exist anywhere in `LangRuleConfig` today" and the whole-file walk needed is a new
shape versus the existing per-declaration walks (requirements.md:36-39). This is new
capability, not composition of existing capability — the highest-risk part of the
implementation, and where most of the below can go wrong:

- **False negatives via shadowing**: if the exclusion just checks "is there a
  const/let with this literal as initializer, referenced elsewhere in the file by the
  same name," a shadowed binding in a different scope (`{ let x = 42; ... } { let x = 43;
  use(x); }`) could cause a raw, unrelated `42` elsewhere in the file to be wrongly
  treated as "already factored" if the matching is by textual identifier name only,
  without verifying the reference actually resolves to *that* declaration's scope.
- **Cross-scope/cross-function const**: a `const` declared inside function A and "used"
  only within function A's own body would trivially satisfy a naive "referenced
  elsewhere" check if that check doesn't distinguish "elsewhere in the same scope" from
  "elsewhere in the file" — but the smell (Fowler's Replace Magic Literal) is about a
  raw literal repeated at *multiple unrelated sites*, not one already-named local. A
  const used only within its own declaring block doesn't demonstrate the literal has
  been "correctly factored" file-wide; it just means one occurrence has a name.
- **"Declared but used once" ambiguity**: the spec text says the exclusion applies when
  the const is "referenced elsewhere" (requirements.md:74-75, "referenced elsewhere by
  name") — implying a const declared and never read again (write-only) should *not*
  exempt it. This needs an explicit reference-count check (>=1 use *besides* the
  declaration site), which is a positive design decision worth locking down in Phase 3,
  since "referenced" is ambiguous between "has a reference" and "has any use including
  the initializer itself."
- **Multi-value destructuring**: `const [a, b] = [42, 42]`-style or Go's
  `a, b := 42, 42` multi-assignment patterns don't have a single clean "the direct
  initializer of this binding" node shape — the literal's parent chain to the binding
  name is one level removed via a tuple/list pattern. A naive `child_by_field_name`
  walk (the pattern every existing `LangRuleConfig` finder function uses, e.g.
  `body_finder`, `params_finder`) would likely miss these entirely, silently falling
  back to "not a const initializer" and flagging correctly-factored code as a
  false positive instead — the opposite failure mode from shadowing.
- **Same literal both const-bound and raw elsewhere**: the requirement's success
  criteria (AC4) exempts *the literal value overall* if a const exists for it and is
  referenced — but doesn't specify whether raw, non-const uses of that same value
  elsewhere in the file should still be flagged (arguably they should — a constant
  existing doesn't retroactively fix a *different*, unrelated raw occurrence of the same
  number). This is a real design ambiguity in the requirements as written, not just an
  implementation-bug risk — worth resolving explicitly in Phase 3's plan rather than
  left implicit, since "the smell is already fixed" (requirements.md:88-89) reads as
  "exempt the whole file" but a stricter, arguably more correct reading is "exempt only
  the const's own declaration site, still flag every raw sibling occurrence."

## 4. Cross-language: "any let/val is a named constant" doesn't hold

- **Python has no `const` keyword at all** — the language convention is a
  module-level `SCREAMING_SNAKE_CASE = <literal>` assignment, which is
  indistinguishable at the AST level from any other assignment
  (`assignment` node in tree-sitter-python). VERIFIED: `grep -rn
  "SCREAMING\|UPPER_SNAKE"` across `src/*.rs` returns nothing — kibitzer has no
  existing convention-based constant detector to reuse for Python, so this
  language needs a genuinely different heuristic (name-casing-based, not
  keyword-based) than every other language in the table, which is an asymmetry
  the other 5 existing `LangRuleConfig` rules (long-function, deep-nesting,
  long-parameter-list, flag-argument, unreachable-code) never had to handle —
  they're all structural/keyword-driven, not naming-convention-driven.
- **Kotlin's `val`** and JS/TS's `const` are "immutable reference," not "intentionally
  promoted to a named symbolic constant" — Fowler's smell is specifically about giving a
  magic literal a *meaningful name*, not merely binding it once. A local
  `val timeout = 42` used once, three lines below its declaration, technically satisfies
  "initializer of a val referenced elsewhere" (if "elsewhere" is read loosely as "any
  site other than the declaration"), but doesn't represent the refactoring the issue is
  chasing — the literal is still effectively magic, just wrapped in one extra
  indirection. Treating "any `let`/`val`, however local or trivial" as exempt risks
  under-flagging real magic literals that happen to sit one hop behind a variable.
  This mirrors the general risk in section 3 above (over-broad "referenced elsewhere"
  matching) but is specifically acute in Kotlin/JS/TS/Rust (`let`) since those
  languages use plain mutability-scoped keywords for both "true named constant" and
  "ordinary local variable," unlike Go's `const` (compile-time, more clearly
  intentional) or Java's `static final`.

## 5. Performance: whole-file walk, no existing size guard on `SyntaxRulesChecker`

The requirement itself already flags the *shape* problem: this needs a new
`walk_blocks`-style whole-tree walk rather than reuse of the existing
`walk_declarations` per-function walk (requirements.md:33-39, confirmed directly by
reading `src/rules.rs:745-766`: `walk_declarations` recurses the whole tree looking for
`function_kinds`, and `walk_blocks` recurses the whole tree looking for `block_kind` —
both are already O(nodes-in-file), so a literal-collecting walk is the same asymptotic
class as the existing `unreachable-code` rule, not a new order of cost by itself).

The real gap: **no rule inside `SyntaxRulesChecker` calls
`file_size::is_generated` or any size cutoff today** — VERIFIED, `grep -n
"is_generated" src/rules.rs` returns zero matches, versus `duplicate_code.rs:60`,
`complexity.rs:50`, `go_error_context.rs:43`, `hotspots.rs:111`,
`go_type_switch_density.rs:52`, and `go_table_driven_test.rs:60`, which all call it.
`file_size.rs:12` defines `MAX_FILE_LINES = 500` for the *separate* `file-complexity`
check, which flags oversized files as its own advisory — it isn't a guard other
checkers consult to skip expensive work, so there's no existing "skip if file is huge"
mechanism for `SyntaxRulesChecker` to inherit for free.

Concretely: literal-repetition tracking needs a `HashMap<LiteralValue, Vec<Line>>` (or
similar) built by one pass over the whole tree, then a second pass (or inline check) to
resolve the const-exclusion per section 3. That's still linear in file size, so absent
pathologically large single files this is unlikely to be a *correctness*-blocking
performance problem — but combined with section 2's generated-code finding (dense,
often very large machine-generated files are exactly where literal repetition is
highest), a large generated file with thousands of repeated literals is the specific
case where both the noise problem (section 2) and a real cost multiplier (building and
holding a large per-file literal index) coincide. The same `is_generated` early-return
every sibling checker already uses would resolve both at once, and is the obvious reuse
target rather than a bespoke size guard.

## 6. Self-referential firing on kibitzer's own repo/testdata

This repo dogfoods its own `syntax-rules-*` checks with no test-file or testdata
exclusion — VERIFIED by reading `.claude/inspect.json` (this repo's own config,
root of the worktree): `syntax-rules-go`'s scope is `["**/*.go"]` with no `!`
exclusion pattern, unlike the three `dogfood-*` *architecture* checks, which are
explicitly scoped down to `testdata/dogfood-architecture/**` only. Rust files
aren't in `.claude/inspect.json`'s scope list at all (no `syntax-rules-rust` entry —
Rust isn't in this repo's own `.claude/inspect.json`, only Go/TS/Tsx/JS/Python/
Java/Kotlin are configured, even though `lang_config()` in `src/rules.rs` supports
Rust), so kibitzer's own `src/*.rs` — including `rules.rs` itself, 2,329 lines, with
its unit-test module in the same file — would *not* currently be scanned by any
`syntax-rules-rust` check via this repo's committed config; whether `default_checks()`
(`config::default_checks()`, `src/config.rs`) still runs a Rust `syntax-rules` variant
regardless of the repo-local `.claude/inspect.json` overlay is the detail that decides
whether this is moot or live — `docs/suppressing-checks.md:9-13` states the local
config "overlays" (adds to/replaces individual entries) rather than replacing the
built-in catalog wholesale, which implies the Rust default likely still runs.

Two concrete self-firing vectors either way:
- `src/rules.rs`'s own test module has repeated literals by construction — table-driven
  Rust unit tests with repeated line/column numbers, repeated string fixtures
  (`"package main\n"`-style boilerplate lines appear 15+ times per
  `grep -c` in `src/rules.rs`, `src/file_size.rs:154-208` shows the identical pattern
  used to build fixture source via `.repeat(N)`), and repeated named-constant references
  like `MAX_FILE_LINES + 1` used at multiple call sites — a plausible true-but-unhelpful
  or borderline-noisy firing on kibitzer's own source, the same self-referential problem
  `docs/duplicate-code-false-positives.md`'s "needs discussion" section already names for
  `duplicate-code` (60% of its findings were "genuine... debatable value" table-driven
  test literals).
- No `testdata/` directory currently exists for `syntax-rules`-family checks
  specifically (only `testdata/dogfood-architecture/` for the *architecture* checkers,
  and `testdata/comment-quality-corpus/` for comment-quality per this repo's top-level
  CLAUDE.md) — so there's no precedent testdata tree to check for false positives yet,
  but the requirement's own AC10 anticipates needing one ("table-driven test/fixture
  files... have a documented mitigation path"), and per this repo's own convention that
  mitigation path is `scope` excludes or `.kibitzer/accepted/`, not a built-in
  self-exemption — meaning kibitzer's own future test fixtures for this checker
  (in `src/rules.rs`'s test module) would need one of those two levers applied
  explicitly, or accept the noise, exactly as `duplicate-code`'s unresolved
  "should `_test.go` be excluded" question remains open today
  (`docs/duplicate-code-false-positives.md:57-60`).

## Summary of what Phase 3 should decide explicitly (not resolved here)

1. Whether the default threshold is really `>=2` or should start at `>=3` given the
   `duplicate-code` precedent (section 1).
2. Whether `replace-magic-literal` calls `file_size::is_generated` from day one rather
   than waiting for a backtest to discover the same gap every other checker hit
   (section 2, 5).
3. The exact semantics of "referenced elsewhere" for the const exclusion — same-scope
   vs. file-wide, reference-count >=1 beyond the declaration, and whether a const's
   existence exempts *other* raw occurrences of the same literal value (section 3).
4. A distinct, casing-convention-based heuristic for Python's constant exclusion rather
   than reusing the keyword-based approach the other 7 languages can share (section 4).
5. Whether `src/rules.rs`'s own test fixtures need a `scope` exclusion or
   `.kibitzer/accepted/` entries once this ships, given this repo dogfoods its own
   default checks with no test-file carve-out today (section 6).
