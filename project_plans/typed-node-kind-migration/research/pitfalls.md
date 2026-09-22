# Pitfalls research: typed-node-kind-migration

## 1. `clippy -D warnings` interaction

CI runs `cargo clippy --workspace --all-targets -- -D warnings` (`.github/workflows/ci.yml:30`)
with no `clippy.toml`/lint-level overrides in the repo — i.e. default `clippy::all`
(correctness/style/complexity/perf), not pedantic/nursery. Relevant risk for this migration:

- `src/node_kind.rs`'s generated enums are `#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]`
  and `#[non_exhaustive]` (`build.rs:100-101`). `Copy` means no clone/borrow-related lints are
  possible from switching `&str == &str` to `EnumVariant == EnumVariant`.
  `#[non_exhaustive]` only affects *downstream* crates matching on the enum — within this crate
  (the only consumer) a match still must be exhaustive over the real variant set plus `Other`,
  same as today's string match needing a `_` arm. No new exhaustiveness burden.
- The real risk is **`clippy::match_like_matches_macro`** (style, warn-by-default → hard error
  under `-D warnings`). Today's code mixes two idioms: simple `if node.kind() == "x"` chains
  (`go_blank_imports.rs:65,75`, `go_ignored_error.rs:119`) and full `match` statements with
  boolean-returning bodies (`rules.rs` has several `.find(|c| c.kind() == "...")` closures, which
  are fine, but any full `match kind_str { "a" | "b" => true, _ => false }` block that gets
  hand-converted to an equivalent `match GoKind::of(node) { GoKind::A | GoKind::B => true, _ =>
  false }` — instead of `matches!(GoKind::of(node), GoKind::A | GoKind::B)` — trips this lint
  immediately, whereas the string-based equivalent (`kind == "a" || kind == "b"`) never had a
  matches!-shaped match to trigger it in the first place. This is a new failure mode
  *introduced by the migration itself*, not present in the current code.
  Mitigation: prefer `matches!(...)` for any multi-arm boolean check when converting an `||`
  chain, not a `match` with `true`/`false` arms.
- `clippy::single_match` (style, warn-by-default) is a pre-existing risk carried over unchanged:
  a `match kind { X => foo(), _ => {} }` string-based match already trips it today if present;
  converting to the enum doesn't add new exposure here, just needs the same `if let`-vs-`match`
  discipline already in force.
- No existing `<Lang>Kind` call site outside `src/node_kind.rs`'s own `#[cfg(test)]` module
  exists yet to use as a style precedent (`grep -rn "Kind::of\|Kind::from_kind_str" src/*.rs
  src/checkers/*.rs` outside `node_kind.rs` returns nothing) — `src/node_kind.rs:11-13` even
  carries `#![allow(dead_code, unused_imports)]` explicitly because "not every generated
  enum/variant has a caller outside this module's own tests yet." The migration is the first
  real-world consumer, so there's no in-repo idiom to copy; the `matches!` guidance above is a
  net-new pattern this migration needs to establish, not one it can borrow.

## 2. Performance regression risk

Read `build.rs:113-135` (`write_enum_impl`) directly rather than guessing: `from_kind_str`
compiles to a single `match s { "kind_a" => Self::A, "kind_b" => Self::B, ..., _ =>
Self::Other }` over `&str` patterns, and `of(node)` is just `Self::from_kind_str(node.kind())`
(`build.rs:123-125`). This is a **plain Rust string-literal match** — rustc lowers this to the
same length-then-byte-compare / jump-table strategy the compiler already uses for
`node.kind() == "if_statement"` directly; there is no `HashMap`, no allocation, and no interning.
`<Lang>Kind` itself is `Copy` (a fieldless enum, effectively a `u8`-sized tag), so passing it
around or comparing it (`GoKind::of(node) == GoKind::IfStatement`) is at most as cheap as the
original string compare and arguably cheaper for a *chain* of `||`-ed comparisons (`kind() == "a"
|| kind() == "b" || kind() == "c"`, which does the `match` dispatch to build `node.kind()`'s
`&str` three times structurally under the hood in source but the compiler already commons that
up) — converting such a chain to `matches!(Kind::of(node), A | B | C)` calls `node.kind()` once
and does one small integer-tag comparison per arm instead of three string comparisons. **Verdict:
no perf regression risk; if anything a very minor, unmeasurable improvement is more likely.** No
benchmark is warranted — the requirements doc's own NFR section already correctly marks
performance N/A.

## 3. Partial-migration / cross-file inconsistency risk

Grepped every function signature across the 22 in-scope files for a `pub fn` that takes or
returns a kind-typed value (`&str` named `kind`, or a `Kind` return type) crossing a *file*
boundary — found none. Every kind-comparison helper identified (`find_child_by_kind`,
`enclosing_kind_name`, `selector_access_kind` in `symbol_extract.rs`; `terminal_label` in
`rules.rs`) is a private (non-`pub`) function scoped to its own file, so no other file's code can
observe a kind-string/kind-enum signature change made during one file's migration story. This
substantially de-risks landing files independently, as the requirements' "Risk Control" section
already plans.

However, there **is** a real, higher-risk-than-file-boundary form of partial-migration
inconsistency inside `rules.rs` itself: `LangRuleConfig` (`src/checkers/rules.rs:72`) is a struct
of `&'static str`/`&'static [&'static str]` kind fields (`if_kind`, `block_kind`, `nesting_kinds`,
`chain_kinds`, `ternary_kind`, etc.) instantiated once per language (Go at `rules.rs:493` with
`if_kind: "if_statement"`, `block_kind: "block"`; a TS-family config at `rules.rs:540` with
`ternary_kind: Some("ternary_expression")`, `block_kind: "statement_block"`; Python at
`rules.rs:635-637` with `if_kind: "if_expression"`, `chain_kinds: &["elif_clause"]`) — and these
per-language string fields are consumed by **shared, generic functions** that take one
`LangRuleConfig` value regardless of which grammar produced it (e.g. `rules.rs:759`
`node.kind() == cfg.block_kind`, `rules.rs:887-888` `node.kind() == cfg.if_kind ||
cfg.ternary_kind == Some(node.kind())`, `rules.rs:914-922`). A `LangRuleConfig`'s fields cannot
be migrated to a single `<Lang>Kind` type because the struct is generic *across* languages by
design — `if_kind` holds `GoKind`'s `"if_statement"` for one instance and `TypeScriptKind`'s
`"if_statement"`-that-might-differ (or genuinely differs, e.g. Python's `"if_expression"`) for
another. This is exactly the "kind name that differs subtly between two grammars sharing one
checker function" rabbit hole requirements.md already names (line 68) — the concrete evidence is
that it's not hypothetical, it's the entire structure of `LangRuleConfig` and its consuming
functions. A migration that converts each `LangRuleConfig` instance's literal fields to
per-language enum values without also making the *shared* comparison functions generic over
`<Lang>Kind` (e.g. via an enum-returning closure/trait per config, or keeping the comparison at
the `&str` boundary via `.as_str()`) risks either (a) not compiling because the field types
differ per instantiation, or (b) a well-intentioned partial fix that calls `.as_str()` on the
enum at the comparison site, which technically "migrates" the literal but defeats the
compile-time-typo-safety goal for exactly this file — the one requirements.md flags as
highest-value. This is a design decision for Phase 3 planning, not a mechanical find-replace,
and should be called out explicitly as its own sub-story with its own review, separate from the
mechanical Go/Java files.

## 4. Backtest drift risk / cost per file

Per `docs/backtesting.md` and `docs/backtest-repos.md`, "re-run each touched checker's backtest
corpus" is actually **two separate, differently-scoped commands**, both required by kibitzer's
own `CLAUDE.md` "Writing a new check" discipline (not just one):

1. **Transcript backtest**: `kibitzer check backtest <name> --only-new` (optionally
   `--transcripts-dir`) replays real historical Claude Code session edits from
   `~/.claude/projects/*/*.jsonl` through the one named checker. Cost is bounded and mostly
   I/O-cheap: results are persistently cached at `~/.cache/kibitzer/backtest-cache.json`, keyed
   by transcript mtime+size and the *checker-name set* run — so a pure code change to an
   already-migrated checker forces a full re-reconstruct-and-recheck of every transcript (cache
   key is checker selection, not content hash of the checker itself, so it doesn't know the
   checker's internals changed unless invoked with the same name — it always re-runs, this is
   working as intended per `docs/backtesting.md:67-70`, not a caching bug). No documented runtime
   number for a full transcript corpus; likely fast (JSONL parsing + one file-check per
   snapshot) but scales with corpus size, which isn't controlled by this migration.
2. **Real-world repo corpus**: per `docs/backtest-repos.md`, run the checker via
   `kibitzer check native <name> <file>` piped from `find` (or `kibitzer run <repo>` for the full
   default set) against one or more of the ~14 large cloned repos (`kubernetes/kubernetes`,
   `apache/cassandra`, `servo/servo`, etc. — whichever repo(s) match the checker's target
   language). This is the expensive one: e.g. `find
   ~/code/github.com/kubernetes/kubernetes -name '*.go' -print0 | xargs -0 -n1 kibitzer check
   native <name>` walks an entire large real-world codebase file-by-file. No runtime is
   documented in `docs/backtest-repos.md` itself, but "kubernetes/kubernetes" alone is on the
   order of tens of thousands of Go files — this is not a 5-minute check, it is a
   whole-codebase-scan-scale operation per touched checker, and 22 files map to roughly 8-10
   distinct checkers (several files are shared infra like `tree_walk.rs`/`symbol_extract.rs`/
   `declarations.rs` that many checkers depend on transitively, meaning a change there could
   require re-backtesting *every* checker that calls them, not just one).
3. `docs/backtesting.md`'s own workflow (step 5) explicitly calls for re-running "periodically or
   after any change" — for shared-infra files this means the backtest cost is not additive
   per-file but potentially multiplicative: migrating `tree_walk.rs` (used by many checkers per
   its role as a generic AST-walk helper) plausibly requires re-backtesting most/all of the
   default-checks catalog's checkers, not just one, if any consumer's kind logic could be
   affected. **Concretely**: at ~22 files but only ~8-10 distinct checkers, and with 2-4 of those
   files being shared infra rather than single-checker-owned, the realistic per-story backtest
   cost ranges from "a few minutes" (a single, self-contained Go checker file like
   `go_blank_imports.rs`) to "a multi-repo, multi-checker re-run" (any of the four flagged
   high-risk files, especially `tree_walk.rs`/`symbol_extract.rs`/`declarations.rs`/`rules.rs`
   which other checkers depend on). Phase 3 planning should size backtest time per-story
   accordingly rather than assuming a flat cost across all 22 files.

## 5. Single most likely false-confidence pitfall

**`cargo test --workspace` passing 100% is not proof the enum mapping is correct**, because the
existing unit-test fixtures were written to exercise *checker behavior* (does this pattern get
flagged), not *every distinct kind string a checker's match arms compare against*. A wrong enum
mapping is invisible to the test suite whenever:

- Two kind strings normalize to enum variants that a migrator swaps by mistake, and the swap
  still produces *some* match for at least one test fixture that happens to only ever exercise
  one of the two swapped kinds. `build.rs:89-95`'s own comment concedes this exact class of bug
  is structurally possible: "a rare grammar-internal collision (two distinct kind strings
  normalizing to the same identifier) keeps whichever sorts first" — i.e. the codegen itself
  already has one known potential silent-collision path, separate from a migrator's own
  copy/paste mistake.
- **Concrete example for one of the four high-risk files**: `rules.rs`'s Python `LangRuleConfig`
  (`rules.rs:635-637`) sets `if_kind: "if_expression"` and `chain_kinds: &["elif_clause"]` — note
  that Python's *statement*-level if is `if_statement` in tree-sitter-python's actual grammar,
  and `if_expression` is the *ternary* form (`x if cond else y`); this file's own naming already
  overloads `if_kind` across languages to mean "the node this language's nesting-depth logic
  treats as an if" rather than a single canonical `<Lang>Kind` variant name, which is a strong
  signal that a migrator skimming quickly could map `if_kind` to `PythonKind::IfExpression` (a
  distinct real ternary-expression variant, not the Python statement `if`) and have every
  *existing* fixture — which likely only ever exercises `if`/`elif` complexity in ordinary
  statement form — either (a) never touch the `if_expression` code path in its fixtures at all
  (silently making the swap untestable either way) or (b) coincidentally still pass because the
  ternary and statement forms rarely co-occur in the same fixture, so a wrong mapping produces
  zero test failures while quietly changing which real-world Python code the checker's
  nesting-depth logic actually recognizes as a nested `if`.
- **Why `cargo test` alone can't catch this class of bug**: unit fixtures are hand-written to be
  *minimal* reproductions of the pattern under test, which is exactly the property that makes
  them unlikely to accidentally also exercise an adjacent, easily-confused kind. This is the
  concrete "one level deeper" answer requirements.md's Feasibility Risks section gestures at but
  doesn't spell out: it's not merely "the migration could introduce a wrong mapping invisibly" in
  the abstract, it's that **the fixture suite's own minimalism is what makes the invisibility
  possible** — a broader/messier fixture, or the mandatory backtest-corpus re-run against real
  code (large real-world files necessarily co-mix statement-`if` and ternary-`if` in the same
  file), is the only thing structurally capable of surfacing this, which is exactly why
  requirements.md is right to make the backtest re-run mandatory rather than optional per file.
