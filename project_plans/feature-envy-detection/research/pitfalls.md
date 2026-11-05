# Feature Envy checker — known pitfalls and risks

Sources: `src/god_class.rs`, `src/arch_model.rs` (esp. `FieldAccessEdge` and
`walk_field_accesses`), `src/symbol_extract.rs` (`walk_field_accesses`,
`method_field_access_ctx`), `src/architecture_checks.rs` (LcomChecker,
`methods_with_own_signal`), and existing false-positive docs in `docs/`.

---

## 1. False-positive patterns — high foreign-access ratio but NOT Feature Envy

**Adapter / wrapper / proxy / decorator types** are the most pervasive class. A
`func (a *Adapter) Foo()` that simply delegates to `a.inner.X` / `a.inner.Y` /
`a.inner.Z` has a foreign-access ratio of 1.0 by definition — that is the entire
job of a wrapper. The requirements already name this suppression, but the matching
heuristic must be robust: the name-suffix filter
(`adapter`/`wrapper`/`proxy`/`decorator`) won't catch types that play the role
without advertising it. Any type whose only method bodies are single-expression
delegations is structurally indistinguishable from Feature Envy even though the
pattern is intentional. Confirmed analogue: `god_class.rs`'s own doc comment
calls out `ErrorMatcher` in `k8s.io/apimachinery` as a fluent-builder/options
struct that fires all three God Class thresholds correctly yet is an intentional
options pattern. The same "technically correct signal, debatable advice" framing
applies to Feature Envy on adapters.

**Factory and constructor functions** (Go `New*` functions or methods that build
and return a foreign struct) routinely read several foreign fields — the new
instance's field initializers — in one place. Because `ArchModel.field_accesses`
tracks only self-receiver accesses (see §5 below), a `func NewFoo(bar *Bar)` free
function won't appear there at all and is therefore invisible to the own-access
denominator; but `walk_selectors_on_typed_locals` in god_class.rs *will* see the
foreign-field reads through `bar`, making the foreign count potentially high
while the own count is zero. The suppression path must handle `New*` and
constructor-shaped methods explicitly, or accept they fire.

**Conversion / serialization methods** (`func (x *X) ToProto() *proto.Y { ... }`
or `func (x *X) MarshalJSON() ([]byte, error)`) read many fields of `x` and
produce a foreign-typed value. Because all those reads are on the receiver they
count as own accesses — but a `MarshalJSON` that creates an intermediate struct
and populates its fields reads *those* fields as foreign. If the intermediate
struct lives in a foreign package (e.g. a generated protobuf type), the
walk_selectors path in god_class.rs counts each populated field as a foreign
access. These are not Feature Envy; they are the correct location for
serialization logic.

**Test helpers and table-driven test methods** explicitly reach into foreign
structs to set up or verify state. A test helper of the form `func
checkFooResult(t *testing.T, got *pkg.Foo, want *pkg.Foo)` that reads
`got.Field` and `want.Field` has zero own-field accesses and many foreign ones.
The requirements suppress test files via the `is_test_file` guard, but that only
fires on `_test.go` files; a shared test-helper package (a `testutil/` or
`testing/` subpackage that isn't itself a `_test.go` file) won't be caught.

**Low-level infrastructure methods** that work with many types by design — HTTP
handlers, middleware, router dispatch — routinely touch fields on request/
response structs from `net/http` and application-domain types. These are not
Feature Envy; the method is correctly located given the cross-cutting role.

---

## 2. `walk_selectors_on_typed_locals` — syntactic ceiling and accuracy impact

The ATFD walker in `god_class.rs` resolves foreign types **only** when the type
is written out explicitly at the declaration site:

- A parameter with a spelled-out type: `func (t T) M(o *Other) { o.Field }`  ✓
- A `var x Other` or `x := Other{...}` composite literal  ✓
- Anything returned from a function call: `x := BuildOther()` → type unknown  ✗
- An interface variable: `var x Doer = ...` → dynamic type invisible  ✗
- A generic instantiation: `x := Container[Other]{...}` → not resolved  ✗

For Feature Envy this matters in both directions:

*Under-counting foreign accesses* (missed true positives): a method that
accumulates most of its foreign-field reads through a function-return-value local
(`result := computeBar(); result.X; result.Y`) won't have those counted because
the type is not syntactically present. The method looks less envious than it is.

*Over-counting from same-type parameters* (false positives): a method that takes
a parameter of the same type as the receiver — `func (t T) Merge(other T)` —
should not count `other.X` as a foreign access. The `foreign_type_of` guard in
`god_class.rs` handles this: a bare same-name type compares equal to `own_type`
and returns `None`. Feature Envy must replicate this guard faithfully or it will
inflate foreign counts on value-object types like `Point`, `Duration`, `Decimal`
that commonly accept a same-type parameter (e.g. `Add(other Point)`).

---

## 3. Threshold calibration — small-method instability

With the ratio metric `foreign / (foreign + own)`, a method that makes a single
foreign-field read and zero own reads produces a ratio of 1.0 regardless of
whether that one access is meaningful. At 1 or 2 total accesses the ratio is
numerically useless as a signal.

The `FEATURE_ENVY_MIN_FOREIGN` minimum access count (from the requirements)
addresses one side of this: a method with only 1–2 foreign accesses won't fire
even if the ratio is 100%. But the companion risk is the own-access denominator
being zero for a method that genuinely does no self-state access — not because it
is feature-envious, but because:

- It is a pure function (no receiver-field reads) placed on a type for
  organizational reasons (a common Go idiom: `func (p *Printer) formatLine(s
  string) string` that only touches `s`).
- The own-field accesses exist but were invisible to `ArchModel.field_accesses`
  due to shadowing (see §4) or via a function-return-value local (see §2).

In both cases, `own_count = 0`, so `foreign_ratio = 1.0`. The minimum-foreign
gate is the primary defense; the low-total-access suppression (`foreign_count +
own_count < some minimum`) is the secondary. Both are needed: `MIN_FOREIGN` alone
does not catch a method with 6 foreign accesses and 0 own accesses that is in
fact a pure utility helper.

The LcomChecker already has the analogue: `methods_with_own_signal` in
`architecture_checks.rs` (lines 867–886) excludes methods with no connectivity
signal at all from LCOM4 — Feature Envy should use the same filter as a
precondition (skip any method `ArchModel.field_accesses` has no `from=method_id`
entry for *and* that has fewer than `FEATURE_ENVY_MIN_FOREIGN` foreign accesses)
rather than letting zero-own-access methods dominate the finding set.

---

## 4. `FileCache` cross-contamination — stale-tree panic risk

`god_class.rs` lines 103–137 document and fix this bug precisely:

> sharing one `GrammarCache` here across every file in a package meant every file
> after the first silently got back the *first* file's stale tree paired with the
> new file's source string, producing byte-index-out-of-bounds panics as soon as
> a later file was shorter than the first one parsed.

The fix is `FileCache`: a `HashMap<PathBuf, Option<(String, Tree)>>` that creates
a fresh `GrammarCache::new()` per file and caches the `(String, Tree)` pair by
path, so a second access to the same file returns the already-parsed tree without
re-parsing, and a different file never reuses a prior file's cached tree.

Feature Envy will face the same issue. Its `check()` method calls
`walk_selectors_on_typed_locals` per method, re-parsing each method's file for
every type in the package — the same multi-file-per-package shape that triggered
the panic in GodClassChecker before `FileCache` was introduced. The mitigation is
identical: instantiate and pass `FileCache::new()` from `feature_envy_findings_
for_package` (or its equivalent), one per package, never one per checker run. Do
**not** share a bare `GrammarCache` across files or reuse the checker-level
`check()` call's lifetime for tree storage.

The regression test at god_class.rs lines 529–562 (`file_cache_parses_a_second_
shorter_file_correctly_after_a_longer_first_one`) is the reference fixture for
verifying this behavior. Feature Envy should have an identical regression test.

---

## 5. Missing own-field accesses — same-type parameter distortion

`ArchModel.field_accesses` is populated by `extract_field_access_sites_for_file`
(`symbol_extract.rs`), which tracks **only** `selector_expression` nodes whose
operand matches the enclosing method's receiver identifier
(`node_text(operand, source) == c.receiver_var`). Two cases produce incorrect
own-access counts:

**Case A — same-type parameter**: `func (t T) Merge(other T) { t.X = other.X }`
The `other.X` read is tracked by `walk_field_accesses` as an *own*-type access
(receiver type is `T`, `other` is also `T`, but `other` is not the receiver
variable so `walk_field_accesses` never records it). `ArchModel.field_accesses`
therefore has an entry for `t.X` (own) but not `other.X`. The ATFD walker in
`god_class.rs` handles `other` on the foreign-access side only when `local_type
!= own_type`; since `local_type == "T" == own_type`, it correctly returns `None`
and drops the access. Feature Envy inherits the same `foreign_type_of` guard, so
`other.X` is rightly not counted as foreign. But it is also not counted as own
(because `walk_field_accesses` keyed on the receiver variable name, not the
receiver type). The net effect: a `Merge`-style method that accesses both
`t.Fields` (own) and `other.Fields` (same type, different variable) only gets
the `t.*` portion credited as own. This under-counts own accesses and
over-states the foreign ratio for methods that work on two instances of the
same type.

**Case B — shadowed receiver**: `method_field_access_ctx` in `symbol_extract.rs`
returns `None` (no ctx) for a method whose parameter list shadows the receiver
name (`func (t T) M(t int)`). Those methods produce zero `ArchModel.field_accesses`
entries. Feature Envy will see `own_count = 0` for these methods; they should be
excluded from evaluation entirely (same treatment LcomChecker gives methods with
no own-signal — `methods_with_own_signal`, `architecture_checks.rs:867`).

---

## 6. Backtest false-positive history — god-class and ATFD

The closest confirmed false-positive history from real-world backtesting is in
`god_class.rs`'s own doc comment (lines 76–87, confirmed against
`kubernetes/kubernetes`):

> a fluent-builder/options struct with many independent boolean toggle fields and
> one setter method per field (`ErrorMatcher` in `k8s.io/apimachinery`) reports a
> real, technically-correct God Class signal ... even though "split this up" is
> debatable advice for an intentional options pattern.

This exact pattern — many setters, each touching one or two foreign-struct fields
when performing the comparison — will also fire Feature Envy. `ErrorMatcher`'s
`WithField(field string)` methods read `ErrorMatcher`'s own fields to build a
comparison predicate, but the methods that execute the predicate (`Matches(err
error)`) read fields on whatever type `err` resolves to, which is foreign. The
"low-total-access" suppression from §3 is the main defense; a minimum-foreign
gate of 5 (the existing ATFD threshold) is calibrated against the God Class use
case; for Feature Envy the same threshold needs confirmation against this specific
pattern.

The LcomChecker backtest (`kubernetes/kubernetes`, 925 → 360 findings after the
state-independent-method filter) is the reference evidence that corpus-scale
testing is necessary before shipping any new threshold-based checker. Feature
Envy's threshold calibration pass must run against the full corpus
(`scripts/clone-backtest-repos.sh`), not only hand-built fixtures.

---

## Summary of action items

| Risk | Mitigation |
|---|---|
| Adapters/wrappers/pure-delegation methods | Suppress by name suffix *and* by detecting single-expression delegation (reuse `SymbolNode.is_pure_delegation`) |
| `New*` / constructor functions flagged | Suppress functions whose name starts with `New` or matches constructor conventions |
| Zero-own-access methods dominating | Gate on `ArchModel.field_accesses` having at least one entry for the method (`methods_with_own_signal` pattern) |
| Small-method ratio instability | Require `foreign_count + own_count >= MIN_TOTAL` (separate from `MIN_FOREIGN`) |
| FileCache stale-tree panics | Use the existing `god_class::FileCache` (or a copy) — never a shared `GrammarCache` across files |
| Same-type-parameter under-counting own accesses | Exclude methods whose only receiver-qualified accesses are zero; use `methods_with_own_signal` as a pre-filter |
| ATFD walker misses function-return-value locals | Accept as a known ceiling (same as GodClassChecker); document in the checker's doc comment |
| Corpus-scale noise | Mandatory backtest against the full 6-repo corpus before shipping |
