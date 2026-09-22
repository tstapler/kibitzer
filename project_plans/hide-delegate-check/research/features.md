# Research: Hide Delegate / chained-access-depth check — feature landscape

Backing sources: `src/rules.rs`, `src/symbol_extract.rs`, `src/god_class.rs`,
`src/isp_fat_interface.rs`, `src/complexity.rs`, `docs/syntax-rules.md`,
`docs/refactoring-catalog-analysis.md`, GitHub issue
[tstapler/kibitzer#44](https://github.com/tstapler/kibitzer/issues/44), and
web search against PMD's issue tracker and Law-of-Demeter literature.

## 1. What already exists in this repo

### 1.1 The `deep-nesting` shape this check must mirror (`src/rules.rs`)

- Threshold as a documented `const` at module top: `MAX_NESTING_DEPTH: usize = 4`
  (`src/rules.rs:12-15`), sibling to `LONG_FUNCTION_LINES`/`LONG_PARAM_LIST_COUNT`.
- `RuleMeta` entry in `CATALOG` (`src/rules.rs:34-64`): `id`, `category`,
  `description`, `default_severity: Severity::Advisory` — every existing rule
  uses `Advisory`.
- `LangRuleConfig` (`src/rules.rs:72-144`) is the per-language node-kind table;
  `deep-nesting` adds `if_kind`, `nesting_kinds`, `else_wrapper_kinds`,
  `chain_kinds` fields to it and a dedicated walk (`max_nesting_depth`,
  `walk_if_chain`, `src/rules.rs:913-1013`) that flattens `else if` chains so
  they don't over-count. A new check for expression-chain depth needs its own
  parallel fields/walk, not a reuse of `nesting_kinds`.
- Finding messages are `[rule-id] <description> — <actionable suggestion>`,
  e.g. `"[deep-nesting] body nests {depth} levels deep (over
  {MAX_NESTING_DEPTH}) — consider extracting a function or inverting a
  condition"` (`src/rules.rs:837`). A `hide-delegate` message should follow
  the same shape and, per Fowler, name the fix: wrap the chain behind a method
  on the receiver.
- Per-language differences are hand-verified against real `to_sexp()` output,
  never guessed by analogy — stated explicitly at `src/rules.rs:67-71` and
  repeated as a comment on nearly every `LangRuleConfig` field. This is the
  binding convention for whatever node-kind table backs chain-depth
  detection too.
- `docs/syntax-rules.md` is the one place all five current rules are
  documented in a single table (line 28-34) plus per-language node-kind prose
  (line 36-117) — acceptance criterion 8 means adding a `hide-delegate` row
  and a paragraph here, not a new doc file.

### 1.2 Cross-file/member-access precedent — and its explicit limits

Three existing checks already walk selector/member-access expressions, but
**only for Go**, and each documents why cross-language support was deferred
rather than attempted:

- `src/symbol_extract.rs:757-761` (`field_access_supports`): *"Field-access
  extraction is Go-only for v1 — Go's fields are always accessed through an
  explicit `recv.Field` selector, which this extraction matches by identifier;
  other languages (attribute access in Python, class-field access in
  TS/JS/Java/Kotlin, `self.` in Rust) would each need their own selector-shape
  handling, deferred rather than guessed at."* This is the single strongest
  piece of internal evidence that acceptance criterion 3's "which languages
  are in v1 scope" question is real, not a formality — the codebase has
  already hit this exact wall once and chosen to defer rather than guess.
- `src/god_class.rs` (ATFD/ `ForeignAccessCtx`) and `src/isp_fat_interface.rs`
  (`walk_calls_resolving_interfaces`) both walk Go `selector_expression`
  nodes and both explicitly exclude the call-target case
  (`is_call_target`/`selector_is_call_target`, duplicated in
  `src/god_class.rs:451-478` and `src/symbol_extract.rs:784-795`) — *"a
  selector that's really a method call doesn't count as data access."* A
  chain-depth walk needs the opposite framing: it *wants* the call-target
  case (that's exactly what makes `a.B().C()` a chain), so this exclusion
  logic is not directly reusable, but the call-target detection pattern
  (compare byte ranges of the selector against the parent `call_expression`'s
  `function` field, since this tree-sitter version's `Node` has no identity
  equality) is.
- Go AST shape confirmed from these three: `a.B().C().D()` nests as
  `call_expression{function: selector_expression{operand:
  call_expression{function: selector_expression{operand: call_expression{...
  selector_expression{operand: identifier "a", field: "D"}...}, field:
  "C"}}, field: "B"}}` — i.e., walking down through alternating
  `call_expression.function` → `selector_expression.operand` gives exactly
  the chain, and each `selector_expression.field` name is a hop. A plain
  field chain with no calls (`a.b.c.d`) is a run of nested
  `selector_expression.operand` with no `call_expression` wrapper at all —
  both shapes need to count toward depth per the requirement ("method calls
  and/or field accesses").
- No existing check in this repo does *any* return-type inference — every
  `rules.rs` check and every `symbol_extract`/`god_class` walk works from
  syntax alone (declared parameter/field types at best, e.g. `bool_param_finder`
  matching literal type-annotation text). The "same-return-type" builder
  heuristic (req. #4) therefore has zero internal precedent to build on; it's
  new ground for this codebase, and per the non-goals section must stay
  syntactic (no cross-file/whole-program type resolution).

## 2. Industry prior art

### 2.1 PMD's `LawOfDemeter` rule — the closest real analog, and a cautionary tale

PMD ships a `LawOfDemeter` rule in its `java-design` ruleset. Its open GitHub
issues are a direct, evidenced catalog of the false-positive classes a
syntactic implementation runs into — useful as a pre-built edge-case list
rather than something to rediscover by trial and error:

- **`this.`-qualified chains undercounted** — a false *negative*: `this.foo().bar()`
  isn't flagged the way an equivalent chain through a separate receiver is.
  ([pmd/pmd#4414](https://github.com/pmd/pmd/issues/4414))
- **Indexed array/collection access miscounted as a chain hop**
  ([pmd/pmd#2181](https://github.com/pmd/pmd/issues/2181)) — `a[i].b[j]` is
  not the Law-of-Demeter-relevant case PMD's rule intends.
- **`this`/`super` dot-accessors counted toward the chain** when they
  shouldn't be, since they reference the object's own interface, not a
  stranger's ([pmd/pmd#2174](https://github.com/pmd/pmd/issues/2174)).
- **Static/pseudo-singleton accessors flagged** — `Thread.currentThread()`,
  `ThreadLocalRandom.current()` — treated as ordinary chain hops even though
  they're effectively static utility access, not object-graph traversal
  ([pmd/pmd#2180](https://github.com/pmd/pmd/issues/2180)).
- **Generic method calls double-counted** — `<T>foo()`-style angle-bracket
  type-witness syntax throws off the dot count
  ([pmd/pmd#2175](https://github.com/pmd/pmd/issues/2175)).
- **Casts to a derived type falsely trigger the rule**
  ([pmd/pmd#2189](https://github.com/pmd/pmd/issues/2189)).
- **Local-object exception not applied**: calling multiple methods on an
  object *constructed inside the same method* is flagged even though Law of
  Demeter's own definition exempts objects the method itself instantiates
  ([pmd/pmd#3840](https://github.com/pmd/pmd/issues/3840),
  [pmd/pmd#1014](https://github.com/pmd/pmd/issues/1014) for the lambda
  variant). This is the single most load-bearing exemption in the literature
  (see §2.2) and PMD's rule is documented as still getting it wrong years
  after shipping.
- **Lambda-expression chains inconsistently detected** — sometimes missed
  (false negative) depending on anonymous-class vs. lambda form
  ([pmd/pmd#4375](https://github.com/pmd/pmd/issues/4375)).

Takeaway: PMD's rule has been live for years and still has open, unresolved
false-positive/negative issues on `this`, casts, generics, static accessors,
and — most importantly — the locally-constructed-object exemption. A v1 scope
that explicitly punts on some of these (documented, not silently wrong) is
more defensible than trying to match PMD's ambition and inheriting its bug
list.

### 2.2 The Law of Demeter's own "one dot" nuance

The rule of thumb "use only one dot" is a simplification the original
literature and later commentary explicitly qualify:
[`a.m().n()` violates the law where `a.m()` does not](https://en.wikipedia.org/wiki/Law_of_Demeter)
— but the law's actual object-kind list (self, parameters, locally
*instantiated* objects, direct attributes, globals) means a chain on an
object the method just built (`new Builder().setX().setY().build()`) is not
a violation at all, regardless of dot count — see
[Yegor Bugayenko's "The Law of Demeter Doesn't Mean One Dot"](https://www.yegor256.com/2016/07/18/law-of-demeter.html)
and the Wikipedia summary's explicit fluent-API/builder carve-out. This is a
*second*, independent justification for the requirement's same-return-type
builder heuristic — it's not just a pragmatic false-positive dodge, it's the
literature's own stated exemption, just approximated syntactically (return
type) instead of semantically (locally-constructed receiver).

### 2.3 Weaker/thinner prior art elsewhere

- **ESLint ecosystem**: no first-party rule; third-party plugins are thin and
  low-adoption (`eslint-plugin-scissors` — "detect long call chains/nested
  expressions," configurable whitelist — and a near-unknown
  `eslint-plugin-chain-max-length`). Neither has meaningful adoption or a
  documented false-positive corpus to learn from; ESLint's own `max-len` is
  a line-length rule, not chain-depth. This suggests JS/TS tooling doesn't
  treat this as a high-value automated check today — worth noting as a
  risk (may be low-signal in practice) rather than an implementation
  blocker.
- **SonarQube**: no confirmed dedicated "message chain"/Law-of-Demeter rule
  found via search (unverified — the search did not surface a rule ID; do
  not cite a specific SonarQube rule number without opening the rule catalog
  directly).
- **Reek (Ruby)**: known for `FeatureEnvy`, `UtilityFunction`, and
  `DuplicateMethodCall` smells; no confirmed dedicated "Law of Demeter"/chain
  detector was found in search results (unverified — flagging as a gap, not
  asserting absence).
- **Checkstyle**: no built-in Law-of-Demeter-style rule is shipped in its
  default rulesets (this matches general knowledge; not independently
  re-verified against the Checkstyle rule catalog in this pass — treat as
  INFERRED).

## 3. Concrete edge cases to design against

Grouped by how directly each is already evidenced (repo precedent or PMD's
tracker) vs. reasoned from the AST shapes above.

**Evidenced (repo precedent or PMD issue):**
1. `this.foo().bar().baz()` — should this count fewer hops (or none) since
   `this` is the object's own interface? PMD's #4414/#2174 show real linters
   disagree/get this wrong; the plan must state a position.
2. Locally-constructed receiver: `new Foo().setX().setY().build()` /
   Rust `Foo::builder().x(1).y(2).build()` — Law of Demeter's own exemption
   (§2.2), and exactly the fluent-builder case req. #4 already calls out.
   The same-return-type heuristic covers *typical* builders (`Builder ->
   Builder -> Builder`) but not a builder whose intermediate methods return
   `&mut Self`/`Self` via a *different* named type at each stage, nor Rust's
   `impl Trait` return position obscuring the concrete type syntactically.
3. Static/namespaced calls: `std::mem::size_of::<T>()` (Rust),
   `Thread.currentThread()` (Java, PMD #2180) — a module-qualified path is
   syntactically a chain of `::`/`.` but isn't object-graph traversal at all.
   Rust's `scoped_identifier` (`std::mem::size_of`) is a different tree-sitter
   node kind from `field_expression`/method-call chains and should likely be
   excluded outright rather than heuristically suppressed.
4. Indexed access breaking up a chain: `a[i].b[j].c` (PMD #2181) — array/map
   indexing between dot-hops; decide whether it resets, continues, or is
   excluded from the count.
5. Casts (`(Foo) obj).bar()`) and generic type-witness calls
   (`this.<T>foo().bar()`) distorting a naive dot/hop count (PMD #2189,
   #2175).
6. Lambda/closure-body chains inconsistently walked depending on AST shape
   (PMD #4375) — this repo's own `deep-nesting` already treats
   lambdas/closures/`func_literal` as nesting constructs with their own body,
   so a chain check must decide whether it descends into a lambda passed
   mid-chain (`a.map(x -> x.b().c().d()).e()`) as a *separate* chain or folds
   it into the outer one.

**Reasoned from AST shapes / requirement text, not yet evidenced in a real
tracker:**

7. **Chain broken across statements via an intermediate variable** —
   `let b = a.getB(); b.getC().doThing();` — a syntactic per-expression walk
   (as scoped by the requirement: "≥3 dot-accesses in *one expression*")
   cannot see this, so it structurally dodges the check. This is arguably
   correct scope (matches `deep-nesting`'s per-declaration, not
   whole-program, analysis) but is worth stating explicitly as a known
   ceiling in the plan, the same way `symbol_extract.rs`'s shadowing
   comments document known ceilings rather than silently having them.
8. **Chain inside an `if` condition vs. an assignment RHS vs. a bare
   statement expression** — the requirement says "one expression," so the
   surrounding statement kind shouldn't matter, but the walk needs to find
   chain root expressions wherever they occur in a declaration body (mirrors
   `deep-nesting`'s full-body recursive walk, not just top-level statements).
9. **Optional-chaining operators** (`?.` in TS/Kotlin, `?` in Rust via `?`
   operator is different — Rust has no `?.` but Kotlin/TS do) — does
   `a?.b?.c` count the same as `a.b.c`? Likely yes (same coupling), but the
   node kind differs (`optional_chain`/`member_expression` sub-shapes in
   TS/JS: verify against real `to_sexp()` output per this repo's stated
   convention, not guessed).
10. **Macro-generated / derived code**: Rust's `derive_builder`,
    `#[derive(Builder)]`, protobuf-generated getters, Lombok `@Data`/`@Builder`
    accessors in Java/Kotlin. Tree-sitter parses the *call site*, not the
    macro expansion, so a chain of derived-builder calls looks identical at
    the syntax level to a hand-written builder chain — the same-return-type
    heuristic should still suppress these correctly *if* the derived
    builder's setters return `Self`/the builder type, which is the near-universal
    convention for both `derive_builder` and Lombok's `@Builder`. Protobuf
    getters (`msg.getFoo().getBar().getBaz()`) are the opposite case — each
    hop returns a *different* message type — and are exactly the pattern the
    check should flag, not suppress: a proto accessor chain reaching through
    nested messages is a textbook Law of Demeter violation, not a builder.
11. **Same-type-but-not-builder chains**: `list.get(0).get(0)` where both
    hops return, say, `Node` — same-return-type heuristic would incorrectly
    suppress this even though it's not a fluent builder, just an incidental
    type coincidence (e.g. tree/graph traversal). The heuristic is a
    reasonable syntactic proxy but has a known false-negative direction:
    real Demeter violations between same-typed hops slip through
    unflagged — worth stating as a documented tradeoff, not silently
    accepted.
12. **jQuery-style `$(...).find().addClass()`**: not applicable to this
    repo's supported language set (Go/TS/JS/Python/Java/Kotlin/Rust — no
    jQuery-specific handling needed), but conceptually the same
    "same-ish return type across a fluent DOM chain" shape as the builder
    case; JS/TS in-scope code using chained array methods
    (`.filter().map().reduce()`) is the directly relevant equivalent and
    the explicit example in the GitHub issue's scope notes.
13. **Mixed field-and-method chains**: `a.b.c().d.e()` — requirement text
    says "method calls and/or field accesses," so the walk must treat
    `selector_expression`/`member_expression`/`attribute`/`field_expression`
    hops and `call_expression`-wrapped hops uniformly when counting depth,
    not just count call hops.
14. **Per-language node-kind gaps**: `symbol_extract.rs`'s explicit
    Go-only precedent (§1.2) means TS `member_expression`, Python
    `attribute`, Java field/method access, Kotlin `navigation_expression`,
    and Rust `field_expression` all need their own from-scratch `to_sexp()`
    verification — none of this exists in the codebase yet for chain-walking
    purposes (only Go's `selector_expression` does, and only for the
    non-call-target case in `symbol_extract`/`god_class`/`isp_fat_interface`,
    which is the opposite of what a chain walk needs).

## 4. Unstated needs beyond the literal requirement text

- **A documented position on `this`/`self`-qualified chains** (edge case 1) —
  the requirement doesn't mention this, but PMD's years-open issue on it
  means silently picking a behavior without stating it invites the same
  complaint category PMD still gets.
- **A documented position on the locally-constructed-receiver exemption**
  (edge case 2) — the requirement's same-return-type heuristic covers the
  common case but the *literature's* actual exemption is about where the
  receiver came from (locally `new`'d vs. a stranger), not its return type.
  The plan should state explicitly that same-return-type is a syntactic
  approximation of that semantic exemption, and name the gap (edge case 11).
- **A decision on whether the check descends into lambda/closure bodies
  mid-chain** (edge case 6), consistent with how `deep-nesting` already
  treats lambda bodies as their own nested scope.
- **Since this is advisory and per-file, false-negative direction (edge
  case 11: same-type non-builder chains slipping through) is the safer
  failure mode than false-positive direction** — consistent with every
  other `rules.rs` check's stated bias (e.g. `unreachable-code`'s "only the
  first dead statement... flagged," `flag-argument`'s type-must-be-statically-known
  requirement) toward under-flagging over over-flagging. This should be
  stated as an explicit design principle in plan.md, not left implicit.
- **Backtesting is not optional decoration** — per `CLAUDE.md`'s "Writing a
  new check" section and req. #6, this check specifically (given PMD's
  documented false-positive history on the *same* rule concept) is a strong
  candidate for surfacing real noise the corpus repos should be run against
  before considering it done, not just unit-tested against hand-built
  fixtures.
- **`docs/check-ideas.md`'s "confirmed transcript occurrence" bar**
  (mentioned in requirements.md) — this doc currently has no entry for
  Hide Delegate/Law of Demeter/chain-depth at all (confirmed via grep); the
  backlog item and GitHub issue #44 provide the design rationale but not
  transcript evidence that this pattern recurs in real agent sessions. The
  plan should either run that evidence search before implementation or
  explicitly note it's proceeding on the backlog/issue's design rationale
  alone (Fowler + Lieberherr et al. citation) rather than a transcript-mined
  justification, since the two are different evidence bars per this repo's
  own stated convention.
