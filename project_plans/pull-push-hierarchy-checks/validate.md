# Validation: pull-push-hierarchy-checks

## Adversarial review

### Challenge 1: "The stated blockers are already done — is there a hidden blocker?"

The description explicitly names #40, #39, and #33 as blockers. All three are
verified shipped (see `research.md`). The two real gaps (`end_line`,
`struct_field_names`) are not mentioned in the issue because they were
discovered during implementation planning, not up-front design. The plan
addresses both; they add one day of model work before the checkers can be
written.

**Verdict**: The hidden blocker risk is real but bounded. Both model gaps have
mechanical fixes with clear implementation paths. No risk of a multi-week
research spiral.

### Challenge 2: "Pull Up Method body-hashing may have too many false negatives on Java/Kotlin due to `is_constant_return` not being populated."

`is_constant_return` is Go/TS-only. For Java/Kotlin, `end_line` will be
populated (tree-sitter Java/Kotlin grammars have good node-level positions),
but the constant-return detection used by `remove-subclass` does not apply
here — Pull Up Method doesn't need it. Body hashing is a different mechanism.
The only risk is that `extract_symbols_for_file` doesn't yet wire up `end_line`
for Java/Kotlin method nodes; that's exactly what Phase 1 task #1 fixes.

**Verdict**: No conflict with existing checks. Java/Kotlin support is a Phase 1
wiring task, not a research gap.

### Challenge 3: "Push Down Method will fire on private/unexported supertype methods that happen to have no callers."

The plan says "skip when callers is empty." A method with no *resolved* callers
is in the same "no evidence" bucket as `unreferenced_symbols.rs` treats private
symbols. The checker should use `needs_private_symbols: false` (the default)
so it only fires on exported methods, where "only one subtype calls it" is
actually surprising. Private/unexported methods are outside scope.

**Verdict**: Addressed by the no-evidence guard and the default
`needs_private_symbols: false`. Add a comment in the implementation to make
this explicit.

### Challenge 4: "Pull Up Field will produce false positives for common field names like `name`, `id`, `mu` (mutex)."

This is a legitimate concern. Two sibling subtypes both having a field called
`name` is extremely common and is often not a pull-up opportunity — the
supertype may already have a `name` field, or the two fields may serve different
semantic purposes despite the same name.

Mitigations in the plan:
1. Skip fields already declared on the supertype (redundant — the supertype
   already has it, so it's not "missing" from the parent).
2. Require the field to appear in ≥2 siblings; require ≥2 subtypes in the
   hierarchy (already in plan).
3. Add a noise guard: skip field names on a known-high-noise denylist
   (`id`, `name`, `mu`, `ctx`, `logger`, `err`) OR require matching *type text*
   in addition to name before firing. The plan says "type text as a stronger
   signal" but doesn't require it; this validation recommends requiring it to
   cut noise.

**Verdict**: Accept the type-text requirement as a mandatory guard rather than
advisory. Update the requirements to reflect this.

### Challenge 5: "Go embedding semantics make Push Down Field almost always wrong."

Go struct embedding puts embedded-type fields directly on the outer type. The
outer type's field set includes all embedded fields, but they "belong to" the
embedded type, not the outer type. If `ArchModel.struct_field_names` includes
embedded field names on the outer type, Push Down Field will fire on fields
that are actually correctly placed.

`extract_struct_fields_for_file` already skips anonymous (embedded) fields
(confirmed: `src/symbol_extract.rs:1999-2000` says "opposite branch from
`go_struct_fields`: a `field_declaration` with no `name` field-child is the
embed this extracts (skipped)"). So embedded-type fields do NOT appear in
`struct_field_names` for Go. This concern does not apply.

**Verdict**: No issue; the existing extraction already handles this correctly.

### Challenge 6: "The plan proposes `pull_up_constructor_body.rs` as a separate file, but constructors are just methods — why not handle them inside `pull_up_method.rs`?"

Constructors are syntactically methods in some languages (TS: `constructor()`,
Java: same-name-as-class). Separating them into a distinct checker allows
different suppression rules (constructors are frequently different; the signal
is narrower). However, if body-hashing is the detection mechanism, the core
logic is identical to `pull_up_method.rs`. The only difference is the
constructor-name filter.

Recommendation: implement `pull_up_constructor_body` as a thin filter wrapper
over shared logic in `pull_up_method.rs` (or factor a shared helper), not a
fully independent implementation. This reduces duplication.

**Verdict**: Merge the two into one file with a constructor-mode flag, or
factor a `pull_up_common.rs` helper. Update the plan accordingly.

### Challenge 7: "The backtest target (≤30% FP) is too loose for a default-on check."

`collapse-hierarchy` landed with an ~85% FP rate on Java (before the JVM guard),
causing post-ship noise. For OOP-heavy languages, default-on requires a stricter
bar. At 30%, this plan's target matches `extract-superclass` but is looser than
what was actually needed post-ship for `collapse-hierarchy`.

Recommendation: set the bar at ≤20% FP, matching the approach used for
`remove-subclass` (4 hits read, 3 TP / 1 FP from a fixture file). If initial
backtest numbers exceed 20%, add an aggressive guard (e.g. require ≥3 sibling
subtypes, not ≥2) before marking done.

**Verdict**: Tighten AC #10 to ≤20% FP. Update requirements.

## Plan completeness check

| AC | Covered by plan? |
|---|---|
| 1 — push-down-method detection | Phase 2a ✅ |
| 2 — push-down-field detection | Phase 2b ✅ |
| 3 — pull-up-method with end_line | Phase 1a + Phase 3b ✅ |
| 4 — pull-up-constructor-body | Phase 3c ✅ (consolidation suggested) |
| 5 — pull-up-field with field declarations | Phase 1b + Phase 3a ✅ |
| 6 — ArchModelChecker + default_checks | Phase 2+3 + Phase 5 ✅ |
| 7 — skip test/fixture files | Noted in plan; needs explicit in-code guard ✅ |
| 8 — skip unresolved edges | Implicit via `resolved_extends` helper ✅ |
| 9 — false-positive docs | Phase 5 ✅ |
| 10 — backtest ≤20% FP | Phase 4 ✅ (threshold tightened by this review) |
| 11 — end_line in SymbolNode | Phase 1a ✅ |
| 12 — struct_field_names in ArchModel | Phase 1b ✅ |

## Open questions

1. Should `pull_up_field` require identical type text, or just identical name?
   This validation recommends requiring both. The alternative is a two-tier
   finding (high/low confidence) — deferred to implementation decision.

2. Should `push_down_method` and `push_down_field` also check that the method/
   field is NOT called/accessed from outside the hierarchy entirely? (If an
   external package also calls a supertype method, pushing it down breaks that
   caller.) The current plan only checks "single subtype callers" — it should
   also gate on "no callers outside the hierarchy." Update the plan.

3. Constructor detection for Java (same name as class) requires the checker
   to know the enclosing type name when inspecting the method node. Is this
   available from `SymbolNode.parent`? Yes — `SymbolNode.parent` holds the
   owning type name for methods. A method with `name == parent` is a Java
   constructor. TS/Kotlin use the `constructor` keyword. This can be
   implemented without additional model data.
