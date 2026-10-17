# Validation: Remove Middle Man checker

## Acceptance criteria verification

Pre-implementation validation — this file records open questions resolved during
planning, and will be updated with backtesting results during implementation.

### AC 1 — `delegation_field` field on `SymbolNode`
PLAN-CONFIRMED: The additive-field pattern is established (`is_pure_delegation` used it
in PR #48, `severity_override` used it for `ArchFinding`). `#[serde(default)]` is the
correct approach. No API-breaking change.

### AC 2 — `symbol_extract.rs` AST extraction
PLAN-CONFIRMED: The AST structure for Go (`selector_expression` chain) and TS/JS
(`member_expression` chain) is consistent with existing `symbol_extract.rs` comments
and verified tree-sitter node kind tables. The two-level selector chain is
unambiguous: outer selector's operand must itself be a selector (not a plain identifier
or call) for the field to be extractable.

Risk: TS grammar uses `member_expression` + `object`/`property`, not Go's
`selector_expression` + `operand`/`field`. Must be verified against real `to_sexp()`
output during implementation — the plan's node kind names are inferred from grammar
docs and symbol_extract.rs comments, not run directly. Flag this in the test plan.

### AC 3 — `model_registry()` registration
PLAN-CONFIRMED: Pattern is identical to all other `ArchModelChecker` registrations.
New `src/middle_man.rs` file preferred (precedent: `src/single_call_site_delegation.rs`,
`src/god_class.rs`, `src/isp_fat_interface.rs`).

### AC 4 — Ratio and threshold logic
PLAN-CONFIRMED: Computing max delegation fraction across all fields (not just checking
if all methods delegate to the same field) correctly handles the 4/5 case.
Edge case: type with 0 methods — `methods_by_type` returns empty, `min_methods` gate
returns early. Verified by inspection of `LcomChecker`'s identical early-return pattern.

### AC 5 — Named threshold constants
PLAN-CONFIRMED: `MIDDLE_MAN_MIN_METHODS = 3`, `MIDDLE_MAN_DELEGATION_RATIO = 0.8`.
Per backlog item proposal.

### AC 6 — Finding message format
PLAN-CONFIRMED: Message format mirrors `LcomChecker`'s verbose format with Fowler
citation. File/line attribution: unlike `LcomChecker` (which has no per-file location),
the Middle Man checker can attribute to the type's first method file/line or to the
*first* delegating method. Prefer the first delegating method's location so the
finding points to actionable code, not the type declaration.

### AC 7 — Unit tests
PLAN-CONFIRMED: Test plan covers positive and negative cases. The `model_from` helper
in `single_call_site_delegation.rs` tests uses `arch_model::build_model` from real
source strings — this is the right approach for a checker that depends on
`delegation_field` being populated by `symbol_extract.rs`.

### AC 8 — Backtesting
PENDING: Must run before marking done:
- `kibitzer check backtest remove-middle-man`
- Corpus scan: `kubernetes/kubernetes`, `apache/cassandra`, `servo/servo`,
  `BurntSushi/ripgrep`, `denoland/deno`, `microsoft/vscode`, `stapler-squad`

Expected result: low fire rate in Go/TS codebases (Middle Man types are genuinely
uncommon in well-maintained codebases). High fire rate would indicate threshold is
too low or the AST extraction is matching unintended patterns.

### AC 9 — `#[serde(default)]` backward compatibility
PLAN-CONFIRMED: `delegation_field: None` is a safe default for all existing cached
symbols. The first run after upgrade will regenerate the cache with the new field.

## Risk register (from adversarial review in plan.md)

| Risk | Mitigation |
|---|---|
| TS `member_expression` node kind not verified | Add TS delegation test in CI; fails loudly if wrong |
| `methods_by_type` returns ids, need nodes | Resolve id → SymbolNode via `pkg.symbols.iter().find()` in `middle_man_findings_for_package` |
| `needs_private_symbols` override missing | Pre-mortem item in plan.md; test with unexported-method-only type |
| Go pointer/value receiver double-counting | `methods_by_type` already groups by bare type name — verified against LcomChecker's use |
| Advisory-only finding surprises | Consistent with `LcomChecker` — Middle Man is a design suggestion, not a bug |

## Suggested threshold calibration

The 80% threshold and 3-method minimum from the backlog proposal are reasonable
starting points. If backtesting against the public repo corpus reveals >5 findings
per KLOC in Go or TS repos, reconsider:
- Raise `MIDDLE_MAN_DELEGATION_RATIO` to 0.9 (≥90% delegation)
- Raise `MIDDLE_MAN_MIN_METHODS` to 4 or 5
- Add carve-out for types whose name ends in `Wrapper`/`Proxy`/`Adapter`/`Decorator`
  (common intentional delegation patterns) — but only if corpus evidence supports it.
