# Requirements: Feature Envy detection — Move Function / Move Field candidates

item_id: 2e67ae3d-f2ef-44ad-a4cb-8da09ff8daae

## Background

A function that references another module's data/fields far more than its own is a
classic **Feature Envy** smell, and the fix is Fowler's **Move Function** (or **Move
Field** for the data-only case). This is one of the best-studied refactoring-detection
problems in the literature: Tsantalis & Chatzigeorgiou, "Identification of Move Method
Refactoring Opportunities" (IEEE TSE, 2009) — the algorithm behind JDeodorant's Move
Method feature — formalizes it as a distance-based metric comparing how much a method
depends on its own class's members vs. another class's.

Issue #38 (God-Class) already ships `ATFD > 5` / `ATFD > 4` foreign-data-access
counting at the type level. Issue #33 (call-graph edges, `ArchModel.call_edges`) and the
foreign-access AST walker in `src/god_class.rs` (`collect_foreign_field_accesses`,
`walk_selectors_on_typed_locals`) provide the prerequisite infrastructure this item
depends on.

## State of dependencies

Both stated prerequisites are now available in the codebase:

- **#33 (call-graph edges)**: `ArchModel.call_edges: Vec<CallEdge>` ships in
  `src/arch_model.rs`.
- **#38 (ATFD foreign-access counting)**: `src/god_class.rs` ships
  `distinct_foreign_field_accesses`, `collect_foreign_field_accesses`,
  `walk_selectors_on_typed_locals`, and `collect_typed_locals` — the exact AST-walking
  machinery needed for per-method foreign access counting.
- **#39 (field read/write-site tracking)**: Own-receiver field accesses are tracked in
  `ArchModel.field_accesses: Vec<FieldAccessEdge>` (`src/symbol_extract.rs`'s
  `RawFieldAccessSite` pipeline). Foreign-side tracking lives in `god_class.rs` but is
  not persisted in `ArchModel`. Feature Envy can be built by reusing the god_class AST
  walker per-method without requiring #39 to extend the model first (see Non-goals).

## Scope

**Go-only for v1** — consistent with every prerequisite: `FieldAccessEdge` is Go-only
(`field_access_supports` returns `true` only for `Language::Go`), and
`collect_foreign_field_accesses` is also Go-only. Cross-language Feature Envy is
explicitly deferred.

Both detection variants ship together in one checker:

- **Move Function candidate**: a method whose foreign-type accesses (field reads/writes
  + method calls to foreign types) outnumber its own-type field accesses above a ratio
  threshold.
- **Move Field candidate**: same computation restricted to field accesses only (no
  method calls counted on either side); a narrower signal when a field really lives
  closer to another type.

## Proposal

Implement `FeatureEnvyChecker` as a new `ArchModelChecker` in a new
`src/feature_envy.rs`, registered in `model_registry()` in
`src/architecture_checks.rs`. The checker:

1. Iterates over packages and groups methods by their owning type (reusing
   `methods_by_type` from `architecture_checks.rs`).
2. For each method, re-reads its source file (same `FileCache` pattern as
   `GodClassChecker`) and counts:
   - **Local accesses**: `ArchModel.field_accesses` entries where `from == method_id`
     (own-receiver field accesses already in the model — no re-parse needed for the
     local side).
   - **Foreign accesses**: runs `collect_foreign_field_accesses` (extracted from
     `god_class.rs` into a shared helper) across the method's AST node to count
     distinct `(foreign_type, field)` pairs.
3. Identifies the **dominant foreign type** (the type with the most accesses) — this
   becomes the suggested Move Function target in the finding message.
4. Flags the method when
   `foreign_count / (foreign_count + local_count) >= FEATURE_ENVY_RATIO_THRESHOLD`
   and `foreign_count >= FEATURE_ENVY_MIN_FOREIGN` (minimum to avoid noise on tiny
   methods).
5. Suppresses: test files, adapter/wrapper/proxy/decorator-named types (same list as
   `middle_man.rs`), and methods with fewer than `FEATURE_ENVY_MIN_TOTAL` total
   accesses.

## Repo conventions this check must follow

- Registered in `model_registry()` only (not `default_checks()`), same as
  `GodClassChecker`/`LcomChecker`/`InstabilityChecker` — requires an ArchModel and
  is opt-in until backtesting validates false-positive rate.
- Must not modify `ArchModel` or `src/arch_model.rs` — works from existing model data
  and re-parsed method bodies.
- Thresholds (`FEATURE_ENVY_RATIO_THRESHOLD`, `FEATURE_ENVY_MIN_FOREIGN`,
  `FEATURE_ENVY_MIN_TOTAL`) must be named `const`s at module top — never magic
  numbers.
- Must be backtested per `CLAUDE.md`'s "Writing a new check" section before being
  considered done: `kibitzer check backtest feature-envy` (session transcripts) and the
  public repo corpus (`scripts/clone-backtest-repos.sh`, triaged with
  `scripts/backtest-triage.py`).

## Acceptance criteria

1. A new `ArchModelChecker` named `feature-envy` is added to `src/feature_envy.rs`
   and registered in `model_registry()` in `src/architecture_checks.rs`.
2. The checker computes, per method, both a **foreign field-access count** (via the
   god_class AST walker, refactored into a shared module or re-exposed as `pub(crate)`)
   and a **local field-access count** (from `ArchModel.field_accesses`), and flags the
   method when the foreign ratio exceeds `FEATURE_ENVY_RATIO_THRESHOLD` and
   `foreign_count >= FEATURE_ENVY_MIN_FOREIGN`.
3. Each finding message names the **dominant foreign type** (the target of the
   suggested Move Function/Field) so the agent reading the finding knows where to
   move the method.
4. A narrower **Move Field candidate** variant fires when the same ratio holds with
   field accesses counted on both sides only (method calls excluded); findings are
   clearly labeled `[feature-envy:move-field]` vs `[feature-envy:move-function]`.
5. Test files (`_test.go` suffix), types named `*Adapter`/`*Wrapper`/`*Proxy`/
   `*Decorator`, and methods with fewer than `FEATURE_ENVY_MIN_TOTAL` total accesses
   are suppressed.
6. Unit tests cover: a flagged Move Function candidate, a flagged Move Field candidate,
   a suppressed test-file method, and a method close-to-but-below the threshold (true
   negative).
7. The checker is backtested against the session-transcript corpus
   (`kibitzer check backtest feature-envy`) and the public repo corpus before being
   marked done; the false-positive rate on the corpus is recorded in the validation
   artifact.
8. `needs_private_symbols` returns `true` (unexported methods are the most likely
   Feature Envy candidates and must not be pruned before the checker runs).

## Non-goals

- No changes to `ArchModel` structure or `src/arch_model.rs` — this is an
  `ArchModelChecker` that works from existing model data and re-parsed method bodies,
  same as `GodClassChecker`.
- No cross-language support in v1 — Go only.
- No automatic refactoring execution — the finding is advisory, naming a target type.
- No "Move Field" in the ArchModel-extension sense — the Move Field detection here is
  purely a filtering variant of the Feature Envy ratio, not a full write-site analysis.
- `FEATURE_ENVY_RATIO_THRESHOLD` is not wired to a per-repo config knob in v1 —
  the named `const` threshold is the only dial; per-repo override is a follow-up.
