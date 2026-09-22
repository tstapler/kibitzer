# Build vs. Buy: typed-node-kind-migration conversion work

Scope reminder: `src/node_kind.rs` / `build.rs` codegen already exists and is out of
scope. This is purely about how to convert the ~22 files' call sites fastest and safest.

## 1. Manual/agent conversion, file by file (the SDD default)

**Pros**: Each conversion gets human/agent judgment on the exact failure mode this
migration guards against — field-type comparisons, supertype groupings, and
punctuation/`Other` cases that must stay raw strings (requirements.md's non-goal).
Trivially reviewable per commit/story, matches the plan's "land each file as its own
story" rollback strategy.

**Cons**: ~159 individual `.kind() ==` / `match .kind()` comparisons across 22 files
(`grep -c` verified below) is a lot of repetitive, low-judgment work for the ~16-18
mechanical files, where the "judgment" is just "look up the PascalCase variant name."

**Verdict: Recommended** for the 4 flagged high-risk files (`rules.rs`,
`symbol_extract.rs`, `import_graph.rs`, `declarations.rs`) — confirmed non-mechanical
per requirements.md (field-type comparisons, supertypes, cross-grammar kind-name
collisions). Manual/agent conversion only.

## 2. Codemod via `ast-grep` rewrite mode — tested and verified

`sg` (ast-grep 0.44.1, already a project convention per this repo's `ast-grep` skill)
has a real rewrite mode, not just search: `sg run --pattern ... --rewrite ...` for
one-offs, or `sg scan --rule <file>.yml` for a reusable rule with a YAML `fix:`. Verified
against `src/checkers/go_blank_imports.rs`.

**First attempt (naive, doesn't work):**
```
sg run --lang rust --pattern '$NODE.kind() == "comment"' \
  --rewrite 'GoKind::of($NODE) == GoKind::Comment' src/checkers/go_blank_imports.rs
```
Works, but only for one literal at a time — not a generic codemod (you'd have to
enumerate every distinct kind string across every file by hand, at which point you've
just hand-written a per-line sed script).

**Second attempt, generic via `transform`/`convert` — works:**
```yaml
id: kind-to-enum
language: rust
rule:
  pattern: $NODE.kind() == $KIND
  has:
    field: right
    kind: string_literal
transform:
  RAW:
    replace: { source: $KIND, replace: '"', by: '' }
  ENUM:
    convert: { source: $RAW, toCase: pascalCase }
fix: GoKind::of($NODE) == GoKind::$ENUM
```
Run via `sg scan --rule rule2.yml src/checkers/go_blank_imports.rs`. This correctly
produced `GoKind::of(node) == GoKind::Comment` and
`GoKind::of(child) == GoKind::ImportSpec` from `node.kind() == "comment"` and
`child.kind() == "import_spec"` — quote-stripping (`replace`) chained into
snake_case→PascalCase (`convert: toCase: pascalCase`) both worked correctly, including
the underscore removal on a multi-word kind. This is a real, mechanical,
regex-plus-case-transform pipeline, not a manual per-literal mapping.

**What it can't express, and why it's still not "run it and done":**
- **Language is hardcoded per rule** (`GoKind`, one `fix` per language) — a shared
  checker function spanning two grammars (rules.rs's flagged risk) needs the rule to
  know *which* grammar's enum applies at that call site, which `ast-grep`'s
  pattern/transform language has no way to infer from the surrounding code (it would
  need dataflow-level knowledge of which `Node` came from which parser). This is exactly
  why rules.rs et al. are excluded from codemod scope, not a flaw in the rule above.
- **`Other`/punctuation safety is enforced by the compiler, not the rule.** Running this
  rule blindly over a whole file rewrites every matched comparison, including ones
  against anonymous/punctuation kinds that have no corresponding enum variant (e.g. a
  literal `"{"`figures nowhere in `<Lang>Kind`). Those rewrites fail `cargo build`
  (`no variant named...`) immediately — which is actually useful: **`cargo build`
  becomes the sieve** for "which literals are safe to rewrite." The workflow is
  therefore: run the rule → `cargo build` → for every new compile error, revert that one
  hunk (`git diff` / `git checkout -p`) back to the raw-string form → repeat until it
  compiles. This isn't free — it's a mechanical loop, not a single command — but it's a
  loop a script or agent can drive to convergence without per-instance kind-name lookup.
- **`match node.kind() { "a" => ..., "b" => ... }` arms** need a slightly different
  pattern (`match $NODE.kind() { $$$ARMS }` with per-arm transform), not tested here —
  plausible but would need its own verification pass before trusting it on the `match`
  call sites (`go_bulk_fetch_linear_scan.rs:200` has one: `match function.kind() { ... }`).
- No `.filter(|n| n.kind() == "...")` closures were an issue — the pattern matched fine
  through closures in the tested file, since `sg`'s structural match doesn't care about
  surrounding syntax.

**Verdict: Viable, with a build-error-driven revert loop**, for the mechanical
single-language files only (see file list below). Not "fire and forget" — needs the
compile-loop step and a `match`-arm variant of the rule verified before trusting it on
files that use `match`.

## 3. Existing published crates — do they make this migration (or its infra) redundant?

Searched crates.io/GitHub for prior art in "typed tree-sitter node kind from
node-types.json." Several exist and are directly the same pattern this repo's `build.rs`
already implements:

- **`type-sitter`** (Jakobeha) — generates a distinct Rust type per node kind (not just
  an enum) plus supertype enums, via proc-macro, build script, or CLI. More ambitious
  than this repo's enums (whole typed-node wrappers, not just kind tags).
- **`treesitter-types`** — parses `node-types.json` into structs/enums, ships 24+
  pre-generated per-language crates.
- **`tss-rust`** — same idea, hardcoded to `tree-sitter-rust`'s grammar only.

**Fit for this migration**: none of these are usable as a drop-in replacement without
violating the explicit out-of-scope constraint ("Building any new codegen or
enum-generation mechanism... is complete and unchanged by this work") — swapping in
`type-sitter` would mean regenerating every call site against a different, richer type
(distinct struct-per-kind, not a flat enum) and re-deriving `GoKind`-shaped call sites
from scratch. It would not speed up *this* migration; it would replace the very
`node_kind.rs` this migration is scoped to leave alone.
None of them offer a "convert my existing raw-string comparison" codemod either — they
solve the codegen-infra problem (already solved here), not the call-site-conversion
problem this migration is actually doing.

**Verdict: Not recommended.** Interesting prior art confirming the pattern is
well-trodden (this repo's `build.rs` isn't a one-off invention), but adopting any of them
mid-migration would expand scope, not reduce it.

## 4. Overall verdict

| Approach | Scope | Verdict |
|---|---|---|
| Manual/agent, file-by-file | 4 flagged files (`rules.rs`, `symbol_extract.rs`, `import_graph.rs`, `declarations.rs`) | **Recommended** — confirmed non-mechanical, needs judgment on field-type/supertype/cross-grammar cases |
| `ast-grep` codemod + build-error revert loop | The ~16-18 single-language, non-`match` mechanical files (all Go/Java checker files: `go_blank_imports.rs`, `go_bulk_fetch_linear_scan.rs`, `go_error_context.rs`, `go_ignored_error.rs`, `go_type_switch_density.rs`, `java_error_context.rs`, `java_ignored_error.rs`, `java_lost_exception_cause.rs`, `java_swallowed_interrupt.rs`, `primitive_obsession.rs`, `complexity.rs`/`complexity_tests.rs`, `dedup.rs`, `god_class.rs`, `isp_fat_interface.rs`, `plugin.rs`, `tree_walk.rs`, `go_call_resolution.rs`) | **Viable** — verified the core rewrite mechanic works; still needs (a) a per-language rule (one `fix` template per `<Lang>Kind`), (b) a `match`-arm variant rule verified before use, (c) a build-error-driven revert loop for any punctuation/`Other`-only literals it over-rewrites. Net effect: turns "convert N call sites by hand" into "run rule, build, revert the handful that fail, verify, backtest" — a real time save on the ~140 mechanical comparisons but not a zero-review step. Two previously-unassessed files (`go_bulk_fetch_linear_scan.rs`, `go_table_driven_test.rs`) were spot-checked here and are simple `== "literal"` comparisons in one grammar each — mechanical, same risk tier as the other Go files, not hiding the flagged-file subtlety. |
| Existing crates (`type-sitter`, `treesitter-types`, `tss-rust`) | N/A | **Not recommended** — solves a problem (codegen) already solved and explicitly out of scope; adopting one would replace `node_kind.rs`, not accelerate converting call sites to it. |

**Recommendation for Phase 3 planning**: split the epic exactly as requirements.md's Open
Questions suggests — 4 flagged files as manual/agent stories, remaining ~16-18 files as a
single "codemod + build-loop + backtest" story (or a few, grouped by language) rather
than 16-18 separate hand-conversion stories. The codemod doesn't remove the mandatory
backtest-corpus re-run per touched checker (requirements.md constraint) — it only removes
the manual lookup-and-type step, not the verification step.
