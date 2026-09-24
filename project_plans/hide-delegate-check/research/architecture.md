# Research: Architecture — `hide-delegate` chain-depth check

Builds on `project_plans/kibitzer/research/architecture.md` (prior SDD research pass, same
repo, 2026-08-22 clone) — that file already documents `rules.rs::LangRuleConfig`'s
per-language table pattern (its lines ~50-53, ~323-330, ~418) as the established precedent
for adding per-language node-kind config without touching shared walk logic. This file does
**not** re-derive that pattern; it extends it to a construct that prior pass never covered
(chain-walking vs. nesting-depth/line-count), and resolves the type-inference question that
pass explicitly deferred.

Scope: read `src/rules.rs` in full (2329 lines: `LangRuleConfig` struct, all 7
`lang_config()` instantiations, `check_declaration`/`walk_declarations`/`walk_blocks`/
`max_nesting_depth`), `src/config.rs`'s `syntax_rules_checks()` wiring, `src/symbol_extract.rs`
(verified Go/TS call-site node kinds), and the vendored `codegen/node-types/*.json` for all
7 grammars (Go, TypeScript, JavaScript/TSX share TS's table, Python, Java, Kotlin, Rust) to
get real, build-verified node-kind names — not guessed by analogy, matching this repo's own
stated convention at `src/rules.rs:67-71`.

## 1. Integration point: extend `SyntaxRulesChecker`, not a new `Check`/registry

`config.rs::syntax_rules_checks()` (`src/config.rs:718-738`) already wires **one `Check`
per language**, each pointing `checker: Some("syntax-rules"/"syntax-rules-typescript"/...)`
at the single `SyntaxRulesChecker` (`src/rules.rs:705-743`), which runs every `rules.rs`
rule (`long-function`, `deep-nesting`, `long-parameter-list`, `flag-argument`,
`unreachable-code`) in one AST pass per file via `SyntaxRulesChecker::check()`
(`src/rules.rs:732-742`: `walk_declarations` then `walk_blocks`).

This means `hide-delegate` needs **zero new `config.rs` wiring** — no new `Check`, no new
`checker` name, no new registry. It's a third top-level walk function alongside
`walk_declarations`/`walk_blocks`, called from the same `check()` body, plus a `CATALOG`
entry (`src/rules.rs:34-65`) and a `RuleMeta` block. This is the cheapest possible
integration shape and directly satisfies requirements.md AC #1 ("active with no
`.claude/inspect.json` required — consistent with every other `rules.rs` check").

## 2. Walk shape: sibling to `walk_blocks`, not a `LangRuleConfig` function-field addition

The task brief's framing — "does this fit as another function field on `LangRuleConfig`,
or does it need a materially different walk" — resolves cleanly once `walk_declarations` vs.
`walk_blocks` are read side by side:

- `walk_declarations` (`src/rules.rs:745-753`) finds `function_kinds` nodes, then
  `check_declaration` computes nesting depth via `max_nesting_depth`
  (`src/rules.rs:913-935`), which **already recurses every child of the function body**,
  not just top-level statements — so `deep-nesting` already sees nesting constructs inside
  `if` conditions, arguments, etc. The framing that `deep-nesting` "walks top-down from a
  function body" undersells it: `max_nesting_depth` full-recurses; it just only *does*
  something (adds depth) at `if_kind`/`nesting_kinds` nodes.
- `walk_blocks` (`src/rules.rs:758-766`) is the closer precedent: it recurses the **whole
  tree** unconditionally, independent of function boundaries, because "a `block_kind` node
  can be anywhere" (its own comment, `src/rules.rs:755-757`) — same property a dot-chain
  expression has (it can appear in a `return`, an `if` condition, a call argument, a
  variable initializer, anywhere).

**Recommendation: `hide_delegate` is a new function, `walk_chains(node, cfg, src,
findings)`, called from `SyntaxRulesChecker::check()` alongside `walk_declarations`/
`walk_blocks`** — not scoped to function bodies, not a new field on the existing
`check_declaration`/`max_nesting_depth` call path. Two additions to `LangRuleConfig`
mirror the existing `body_finder`/`params_finder` split (field-based vs. positional,
`src/rules.rs:99-105`):

```rust
/// Node kinds for one "hop" of a dot-chain: a method call or a field/property access.
/// Not named `chain_kinds` — that field already exists (`src/rules.rs:90-96`) for a
/// different concept, `if`/`elif` chaining used by `max_nesting_depth`'s `walk_if_chain`.
/// Reusing the name would be a real collision, not just a style nit.
dot_chain_kinds: &'static [&'static str],
/// Given a dot_chain_kinds node, returns (receiver_node, accessor_name) — the node one
/// hop further toward the chain's base, and the method/field name text at this hop, for
/// the fluent-suppression heuristic (§4). Field-based for every grammar except Kotlin
/// (positional, see §3.2) — same split `body_finder` already has.
chain_step: fn(Node, &[u8]) -> Option<(Node<'_>, String)>,
```

`walk_chains` recurses the whole tree; at each `dot_chain_kinds` node, it checks whether
its **immediate parent** is *not* itself a `dot_chain_kinds` node (i.e. this node is the
outermost/topmost node of its own chain expression) — only outermost nodes trigger a
depth computation, exactly mirroring `check_block_for_unreachable`'s "flag at most one
statement per block" principle (`src/rules.rs:768-771`) applied to chains instead of dead
code. The depth computation then walks down via `chain_step` repeatedly, counting hops,
until `chain_step` returns `None` (reached the chain's base — an identifier, `this`/`self`,
a literal, or a call with no further receiver).

Because the walk is whole-tree (not spine-following from a single root), a chain nested
inside another chain's call arguments (`a.b(x.y.z().w()).c()`) is naturally visited too: the
inner `x.y.z().w()` is its own outermost chain at the point the recursion reaches the
argument subtree, independent of the outer chain's own depth computation. This satisfies
the requirement to catch chains "inside if conditions, assignments, arguments" without a
special case per construct — it falls out of whole-tree recursion the same way
`unreachable-code` already gets "any block, anywhere" for free.

## 3. Per-language node kinds — verified via vendored `node-types.json`, not guessed

`src/node_kind.rs:1-9` documents that this repo generates typed `<Lang>Kind` enums at build
time from `codegen/node-types/*.json` (each grammar's own vendored type manifest) precisely
so callers don't guess node-kind strings by analogy. Querying those JSON files directly
(`codegen/node-types/{go,java,kotlin,python,rust,typescript}.json`) gives ground-truth
node kinds and field names for every language in scope, cross-checked against two already-
verified in-repo usages: `src/symbol_extract.rs:568` (`callee_text_for`, matches
`"selector_expression" | "member_expression"` for Go/TS call targets) and
`src/java_lost_exception_cause.rs:79` / `src/java_swallowed_interrupt.rs:80` (both match
`"method_invocation"` for Java).

| Language | Field/property-access kind | Fields | Call kind | Fields |
|---|---|---|---|---|
| Go | `selector_expression` | `operand`, `field` | `call_expression` | `function`, `arguments` |
| TS/JS/TSX | `member_expression` | `object`, `property`, `optional_chain` | `call_expression` | `function`, `arguments` |
| Python | `attribute` | `object`, `attribute` | `call` | `function`, `arguments` |
| Java | `field_access` | `object`, `field` | `method_invocation` | `object`, `name`, `arguments` |
| Rust | `field_expression` | `value`, `field` | `call_expression` | `function`, `arguments` |
| Kotlin | `navigation_expression` | *(none — positional)* | `call_expression` | *(none — positional)* |

Two grammar-shape divergences worth flagging explicitly (found by reading the field tables,
not assumed):

### 3.1 Java bundles call+access into one node kind; everyone else nests two

In Go/TS/JS/Python/Rust, a method call `a.b().c()` is a `call_expression` whose `function`
field is a `selector_expression`/`member_expression`/`attribute`/`field_expression` — two
node kinds alternate per hop (call wraps access wraps call wraps access...). **Java's
`method_invocation` fuses this**: `a.b().c()` is `method_invocation{object:
method_invocation{object: a, name: b}, name: c}` — a single node kind chains directly via
its own `object` field, no separate `field_access` wrapper for the call case (`field_access`
only appears for a bare non-call property read, e.g. `a.b.c`). `dot_chain_kinds` for Java
must include both `method_invocation` and `field_access`, and `chain_step` must check
`object`'s kind against both to keep descending through a mixed call/field chain
(`a.b.c().d`).

### 3.2 Kotlin has no field names — positional children only (consistent with existing precedent)

`codegen/node-types/kotlin.json`'s `navigation_expression` and `call_expression` both report
`"fields": {}` — confirmed by direct inspection, not inferred. `navigation_expression`'s
children are positional: `[expression, identifier]` (receiver first, accessed name last).
`call_expression`'s children are positional: `[expression, (type_arguments)?,
(value_arguments)?, (annotated_lambda)?]` (callee expression first). This is the same
shape `rules.rs` already has a precedent for — `kotlin_body`/`kotlin_params`
(`src/rules.rs:397-411`) already exist specifically because "Kotlin's `function_declaration`/
`anonymous_function` expose no field names at all — only positional children" (comment at
`src/rules.rs:99-101`). `chain_step` for Kotlin follows that exact precedent: take the first
named child as the receiver, and for `navigation_expression` take the last named child
(`identifier`) as the accessor name.

### 3.3 TS inline type annotations — checked and ruled out as a usable signal

The task brief raised "TS chains sometimes carry inline type annotations" as a possible
per-hop type signal. Checked against `typescript.json`'s `call_expression` fields
(`arguments`, `function`, `type_arguments`) and `member_expression` fields (`object`,
`property`, `optional_chain`): `type_arguments` is for **explicit generic invocation**
(`foo.bar<T>()`), not a return-type annotation, and no field on either node carries a return
type. A TS variable declaration can carry `: Type`, but that annotates the *chain's overall
result*, not each intermediate hop's type — there is no per-hop syntactic type signal in TS
any more than in Go/Rust. This rules out (a) as stated in the brief for TS specifically, not
just Go/Rust.

## 4. Fluent-builder suppression: recommend a documented syntactic fallback (brief's option b), not literal type-sameness

Requirements.md AC #4 says: "a chain where every intermediate call returns the same type as
its receiver is not flagged," and Non-goals explicitly pre-authorizes deferring this with a
documented fallback heuristic, since tree-sitter gives no symbol/type resolution and this is
a single-file AST rule by design (no cross-file resolution, per Non-goals).

**The literal AC #4 heuristic doesn't even correctly describe its own example.**
Requirements.md's stated false-positive class is `.filter().map().collect()` — but that's a
Rust `Iterator` adapter chain, and each hop *changes* type
(`Iterator<T>` → `Filter<I, P>` → `Map<Filter<I,P>, F>` → `Vec<U>`): it is not a
"same-type-as-receiver" chain at all. A checker that literally implemented "same return type
as receiver" would **not** suppress the requirement's own canonical example. This is worth
surfacing to whoever writes plan.md: the "same type" framing describes classic OOP builder
chains (`StringBuilder.append()` returning `StringBuilder`, or `this`-returning setters),
not iterator/stream fluent APIs, which are a materially different false-positive class with
a different syntactic signature (no receiver-identity contract at all — it's chaining on a
family of related-but-distinct types).

Recommend splitting the fallback into two independent, narrow, syntactic heuristics —
narrower than a general "looks like a builder" guess, each defensible on its own terms:

1. **Per-language known-stdlib fluent-adapter allowlist.** A `const` list of method names
   per language whose stdlib/ecosystem contract is "chains freely, not an object-graph
   traversal" — e.g. Rust `Iterator`/`Option`/`Result` adapters (`map`, `filter`,
   `filter_map`, `flat_map`, `and_then`, `collect`, `fold`, `zip`, `chain`, `take`, `skip`,
   `enumerate`, `rev`, `sum`, `count`, `for_each`), Java `Stream`/`Optional` methods (`map`,
   `filter`, `collect`, `reduce`, `sorted`, `distinct`, `limit`, `flatMap`, `orElse`), JS/TS
   `Array`/`Promise` methods (`map`, `filter`, `reduce`, `flatMap`, `then`, `catch`, `sort`).
   A chain where every hop's accessor name is in this list is suppressed. This directly
   covers the requirement's own `.filter().map().collect()` example, which the literal
   same-type reading does not.
2. **Builder-verb-prefix heuristic**, for the classic OOP case AC #4's wording actually
   describes: suppress when every intermediate hop's name shares a common prefix from a
   small fixed set (`set`, `with`, `add`, `put`, `append`, `and`) — the real naming
   convention fluent builders use in Java/Kotlin/TS (`.setName(x).setAge(y).build()`).
   **Kotlin gets one additional, high-confidence special case**: `also` and `apply` are
   stdlib scope functions whose *language-level contract* (not a heuristic — documented
   Kotlin stdlib behavior) is "always returns the receiver's own type," so `.also{}.apply{}`
   hops are unconditionally exempt for Kotlin, no allowlist needed.

**Document both explicitly as syntactic proxies, not type inference**, per the Non-goals
allowance: they will under-suppress (a hand-rolled builder using neither list nor prefix
convention still gets flagged) and can over-suppress (a genuine object-graph traversal that
happens to use a `set`/`with`-prefixed accessor name, e.g. `a.withB().withC()` reaching
through unrelated types, would wrongly be exempted). State this limitation in
`docs/syntax-rules.md`'s `hide-delegate` row and in the rule's own doc comment — this is the
"resolved or explicitly deferred with a documented fallback heuristic" AC #4 requires,
landing on option (b) from the task brief: descope true type inference, ship a named,
bounded, documented proxy.

Per-language allowlist sizing is asymmetric and should be stated as such in plan.md: Rust
and Java/JS have obvious, bounded, well-known stdlib method-name sets worth hardcoding.
Python's dominant fluent-chaining idiom in the wild is pandas (`.groupby().agg()....`), which
is a third-party library, not stdlib — recommend **shipping Python with an empty
allowlist for v1** (detection still works; suppression doesn't) rather than guessing at
ecosystem-specific names, and stating that gap explicitly rather than silently
under-covering it. Go has no comparably prevalent fluent-chaining idiom in its standard
library, so an empty allowlist for Go is not a gap so much as an accurate reflection of the
ecosystem.

## 5. Language scope (AC #3): all 7, not a subset

Unlike the type-heuristic (which has real per-language asymmetry, §4), **chain detection
itself (the depth count, ACs #1-#3) is uniformly implementable across all 7 languages** —
§3's table shows every language has an identifiable access-kind and call-kind with either
field-based or positional (Kotlin) extraction, no language lacks the AST shape needed. There
is no technical reason to descope detection to a subset. Recommend: ship detection for all 7
(Go, TS, TSX, JS, Python, Java, Kotlin, Rust) matching `deep-nesting`'s existing coverage,
with the fluent-allowlist *suppression* quality varying by language as documented in §4 —
detection-without-suppression for Python/Go is still a correct, if noisier, v1 rather than a
reason to exclude those languages outright. This should be validated against the backtest
corpus (`CLAUDE.md`'s mandatory step) before finalizing — if Python's empty allowlist proves
too noisy in practice against the real repo corpus, *that* is the evidence-based reason to
descope it in plan.md, not a guess made here.

## 6. Threshold constant and `CATALOG`/`Finding` shape

Mirrors `MAX_NESTING_DEPTH` (`src/rules.rs:12-15`) exactly:

```rust
/// A single expression chaining more dot-accesses (method calls and/or field reads) than
/// this is flagged by `hide-delegate`. AC #2: flag ≥3 dot-accesses, so this const is the
/// max *allowed* hop count (2) — flagged when the walked hop count exceeds it.
const MAX_CHAIN_LINKS: usize = 2;
```

`CATALOG` entry alongside the existing five (`src/rules.rs:34-65`):

```rust
RuleMeta {
    id: "hide-delegate",
    category: "design",  // matches flag-argument's category, both are Fowler refactorings
    description: "Expression chains 3+ dot-accesses (method calls and/or field reads) — Law of Demeter / Fowler's Hide Delegate.",
    default_severity: Severity::Advisory,
},
```

`Finding` message follows the `[rule-id]` convention (`[hide-delegate] expression chains N
dot-accesses (over 2) — consider a delegating method on the base object`), same pattern as
`[deep-nesting]`/`[long-function]` (`src/rules.rs:826-829`, `836-839`).

## 7. Disposition: Extend as-is — confirmed

This is squarely an addition to an already-established, already-extensible pattern, not a
hotspot needing refactor-first or seam-isolation:

- Zero new abstractions needed at the `Checker`/registry/config.rs level (§1) — the
  `SyntaxRulesChecker` → `LangRuleConfig` → per-language-table shape was built precisely to
  take one more rule without structural change, and this is its sixth occupant, not its
  first stress test.
- The one genuinely new thing — a whole-tree, function-boundary-independent walk — already
  has a working precedent in the same file (`walk_blocks`/`check_block_for_unreachable`,
  §2), not a novel shape invented for this feature.
- The two new `LangRuleConfig` fields (§2) follow the file's own established field-vs-
  positional dual-implementation convention (`body_finder`/`kotlin_body`) byte-for-byte.
- The one real design risk — the type-inference heuristic — is explicitly pre-scoped by
  requirements.md's Non-goals as a single-file syntactic proxy, not a design gap this
  change introduces into an otherwise clean architecture.

No refactor of `rules.rs` or `LangRuleConfig` is warranted before landing this.

## Summary of concrete recommendations for plan.md

1. No `config.rs` changes: `hide-delegate` findings emit from the existing
   `SyntaxRulesChecker::check()` per-language pass, same as all five current rules.
2. New `walk_chains` function, sibling to `walk_declarations`/`walk_blocks`
   (`src/rules.rs:745-766`) — whole-tree recursion, "outermost node of its own chain"
   dedup rule (mirrors `check_block_for_unreachable`'s "flag once" pattern).
3. Two new `LangRuleConfig` fields: `dot_chain_kinds: &'static [&'static str]` and
   `chain_step: fn(Node, &[u8]) -> Option<(Node<'_>, String)>` — **not** named
   `chain_kinds` (collision with the existing if/elif-chaining field,
   `src/rules.rs:90-96`). Field-based extraction for Go/TS/JS/Python/Rust, positional for
   Kotlin (precedent: `kotlin_body`/`kotlin_params`), dual-kind (`method_invocation` +
   `field_access`) for Java (§3.1).
4. Node kinds per language, verified against vendored `codegen/node-types/*.json` (§3
   table) — not guessed.
5. Fluent-suppression is a documented two-part syntactic proxy (stdlib fluent-method-name
   allowlist + builder-verb-prefix heuristic, plus a Kotlin-specific `also`/`apply`
   language-contract exemption), explicitly *not* true type inference, per Non-goals'
   pre-authorized fallback (§4). Flag the requirements' own `.filter().map().collect()`
   example as a type-*changing*, not type-*same*, chain when writing plan.md's rationale.
6. Ship detection across all 7 languages (§5); let the mandatory backtest corpus run
   (`CLAUDE.md`'s "Writing a new check" gate) be the evidence basis for descoping any
   language's *suppression* allowlist, not a pre-emptive guess.
7. `MAX_CHAIN_LINKS: usize = 2` const, `CATALOG`/`Finding` shape mirrors `deep-nesting`
   exactly (§6).
8. Disposition: Extend as-is (§7) — this is the pattern's sixth rule, not an
   architecture-violation area.
