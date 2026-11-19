# Pre-Mortem: feature-envy-detection

**Date**: 2026-11-03  
**Scenario**: The checker shipped, ran against real codebases, and failed. What killed it?

Sources consulted: `implementation/plan.md`, `implementation/adversarial-review.md`,
`research/pitfalls.md`, `src/god_class.rs` (FileCache precedent).

---

## Failure 1 — Zero-own-access pure utility helpers fire at ratio=1.0 (P1)

**Failure**: Any Go method with 0 own-field accesses and ≥ 4 foreign accesses passes `MIN_TOTAL`
(total = 4+ ≥ 4) and fires at ratio = 1.0, producing a false positive for every pure
utility helper, every free function lifted onto a type for organizational reasons, and every
method whose own-field accesses were invisible to `ArchModel.field_accesses` due to the
receiver-variable-shadowing edge case in `method_field_access_ctx`.

**First symptom**: Backtest triage shows a cluster of false positives all sharing
`local_count = 0` in the finding message — the dominant pattern in the first 20 manually
reviewed findings from the public corpus run.

**Prevention**: Add the `methods_with_own_signal` pre-filter from `architecture_checks.rs:867`
to `feature_envy_findings_for_package` before the ratio check: skip any method where
`local_field_access_count` returns 0 and there are no resolved outgoing calls within the
same type. This is documented in `research/pitfalls.md §3` as a required guard and in
`implementation/adversarial-review.md` as an open CONCERN — it must be closed before Phase 5
rather than accepted as a known gap.

**Severity**: P1 — zero-own-access methods are common in every Go codebase (utility
helpers, adapter delegation, shadowed-receiver methods), so the false-positive flood is
near-certain; and once users see the checker firing on clearly correct code, they suppress
it globally rather than per-site, destroying its signal permanently.

---

## Failure 2 — Unnamed pure-delegation types bypass name-suffix suppression (P1)

**Failure**: The adapter suppression list (`Adapter`, `Wrapper`, `Proxy`, `Decorator`)
only catches types that advertise their role in their name. Any type that plays the
delegation role silently — `grpcTransport`, `httpRequestHandler`, `repositoryBridge`,
`metricsForwarder` — fires at ratio=1.0 for every method body, because pure delegation
is structurally indistinguishable from Feature Envy to a syntactic counter.

**First symptom**: Public corpus backtest against `kubernetes/kubernetes` or `servo/servo`
produces a dense cluster of findings on types in infrastructure/transport packages whose
names contain no suppression-list suffix; manually inspecting any three reveals they all
delegate to a single embedded field.

**Prevention**: Add `if sym.is_pure_delegation { continue; }` in the per-method loop in
`feature_envy_findings_for_package`. `SymbolNode.is_pure_delegation` is pre-computed by
`symbol_extract.rs` at model-build time — this is a one-line boolean check at zero
incremental cost. This mitigation is specified in `research/pitfalls.md` summary action
items (row 1) and flagged in `implementation/adversarial-review.md` as an open CONCERN.

**Severity**: P1 — unnamed wrapper types are ubiquitous in idiomatic Go (the language
encourages embedding and delegation without naming conventions); a checker that fires
systematically on this pattern in every infrastructure layer will be disabled from
`default_checks()` within one sprint of shipping.

---

## Failure 3 — FileCache stale-tree panic crashes the checker at runtime (P2)

**Failure**: If `FileCache` is constructed once at the `check()` call level (shared across
packages) rather than once per `feature_envy_findings_for_package` call, any package where
a later-processed file is shorter than the first file parsed will produce a
`byte-index-out-of-bounds` panic when `walk_selectors_on_typed_locals` indexes into the
stale tree with the new source string's byte offsets.

**First symptom**: `kibitzer run <repo>` panics with a byte-index error on a large repo
where packages contain multiple files of varying length; the panic is non-deterministic
across runs because package iteration order is HashMap-based.

**Prevention**: Add a Phase 4 regression test modeled on
`god_class.rs:file_cache_parses_a_second_shorter_file_correctly_after_a_longer_first_one`
that constructs two files where file 2 is shorter than file 1 and asserts the checker
does not panic. This test forces the FileCache lifetime issue to surface during `cargo test`
rather than in a user's CI run. The absence of this test is flagged in
`implementation/adversarial-review.md` as an open CONCERN.

**Severity**: P2 — likely (the per-package FileCache contract is easy to misplace; the
identical bug occurred in `god_class.rs` and was caught only by a dedicated regression test),
but recoverable (the panic produces a clear stack trace pointing to the FileCache
construction site, and the fix is mechanical).

---

## Failure 4 — `New*` constructor false positives erode trust at MIN_FOREIGN=3 (P2)

**Failure**: A `NewOrder(customer *Customer, pricer *Pricer, logger *Logger)` constructor
that sets fields on each parameter in its body will access ≥ 3 distinct `(foreign_type,
field)` pairs, triggering `MIN_FOREIGN = 3`. With `own_count = 0` (constructors don't
read the receiver's own fields — they're building it), the ratio is 1.0 and both
`move-function` and `move-field` findings fire. Go conventions produce one `New*`
constructor per exported type; a medium-sized package with 10 types generates 10 spurious
findings on its first kibitzer run.

**First symptom**: After running against any idiomatic Go service, the finding list is
dominated by `[feature-envy:move-function] pkg::Type.NewType ...` entries — the `New`
prefix is visible in every finding's method name.

**Prevention**: Add `if sym.name.starts_with("New") { continue; }` to the per-method loop
in `feature_envy_findings_for_package`, or document the known FP class in both the checker's
module-level doc comment and the Non-goals section of `plan.md`. The adversarial review
(`implementation/adversarial-review.md`, "New* constructor false positives") flags the
plan's `MIN_FOREIGN ≥ 3` rationale as insufficient and requires either suppression or
explicit documentation before shipping.

**Severity**: P2 — `New*` constructors are near-universal in idiomatic Go so the FP class
is certain to fire; recoverable by adding the prefix suppression post-ship or raising
`MIN_FOREIGN` to 5 (PMD's calibrated threshold), but each `New*` finding reported to users
before the fix is a credibility tax.

---

## Failure 5 — `needs_private_symbols = true` not enforced, checker silently blind (P3)

**Failure**: `FeatureEnvyChecker::needs_private_symbols` returns `true`, which signals the
model builder to include private method symbols. If a bug in the model-build pipeline causes
private symbols to be omitted despite the flag (e.g., a flag that is read but short-circuited
under certain invocation paths — `kibitzer check feature-envy` vs. `kibitzer run`), the checker
runs to completion with zero errors but evaluates only public methods. Because Feature Envy
is most actionable on private methods (the internal implementation details of a type), the
checker produces a systematically incomplete finding set that looks valid but misses the
majority of its intended targets.

**First symptom**: Findings are exclusively on public methods; manual inspection of the codebase
reveals private methods with obvious Feature Envy signals that the checker did not report.
Comparing `kibitzer check feature-envy <dir>` against `kibitzer run <dir>` (which uses the
full model build path) may show different finding counts.

**Prevention**: Add an assertion or smoke test in Phase 5 that verifies the model built for
the backtest corpus includes at least one private symbol per package (e.g., `assert!(
model.symbols.values().any(|s| !s.is_public))`), and explicitly document in the validation
artifact whether the backtest was run against the full private-symbol model or a public-only
one. Record the invocation command verbatim in `decisions/validation.md` so it can be
reproduced exactly.

**Severity**: P3 — a model-build flag regression is unlikely given the existing test coverage
for `arch_model::build_arch_model`, but the failure is catastrophic (the checker silently
underperforms for its entire production lifetime without any error signal) and the detection
path is non-obvious because all outputs look plausible.

---

## Summary

| # | Failure | Severity |
|---|---------|---------|
| 1 | Zero-own-access helpers pass MIN_TOTAL and fire at ratio=1.0 | P1 |
| 2 | Unnamed pure-delegation types bypass name-suffix suppression | P1 |
| 3 | FileCache stale-tree panic — no regression test guard | P2 |
| 4 | New* constructor FPs at MIN_FOREIGN=3 | P2 |
| 5 | needs_private_symbols=true silently not honored | P3 |

**Top failure mode**: The `methods_with_own_signal` pre-filter is absent from the plan — any
method with zero own-field accesses passes every guard and fires at ratio=1.0, flooding the
finding set with false positives on the most common, most defensible Go idioms (pure utility
helpers, shadowed-receiver methods, delegation without naming conventions) before any
real Feature Envy is visible.
