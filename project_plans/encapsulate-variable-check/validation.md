# Validation: Encapsulate Variable Check

## Adversarial Review Summary

The plan was reviewed against five identified weaknesses. Each is addressed below with
a disposition.

---

### V1 — DTO suffix list fragility

**Concern**: Types not in the suffix list but intentionally public-mutable generate
false positives; keeping the list current is ongoing maintenance.

**Disposition**: ACCEPTABLE. The suffix list covers the most common Go idioms. The
remaining escape hatches are:
1. `// kibitzer:ignore encapsulate-variable -- intentionally public` on the write site
2. `scope` restriction or `disabled` in `.kibitzer/inspect.json`

The list is a compile-time constant; adding a suffix is a one-line PR. Advisory
severity means a false positive is an annoyance, not a blocker.

**Recommendation**: Document the allowlist and all suppression paths in
`docs/go-encapsulate-variable-false-positives.md` before shipping.

---

### V2 — Go-only scope

**Concern**: The check is silent on TypeScript, Java, Kotlin, Python, Rust.

**Disposition**: ACCEPTABLE AND DOCUMENTED. This is explicitly a v1 scope decision
aligned with `field_access_supports(language)` in the extraction layer. The check name
prefix `go-` signals Go-only scope to users. Expanding to other languages is a
future-follow gated on extraction-layer additions per language.

---

### V3 — Syntactic write detection ceiling

**Concern**: Indirect writes via pointer aliasing (`&recv.Field` to a mutating callee)
are not detected.

**Disposition**: ACCEPTABLE. The `AccessKind` doc comment already describes this as a
known v1 ceiling. Advisory severity appropriately communicates that the check's
evidence is "likely" not "certain." The checker should not claim to be exhaustive.

---

### V4 — Generated code write sites

**Concern**: Protobuf/mock-generated code contains many cross-package field writes;
flagging them is noise.

**Disposition**: ACTION REQUIRED. Add a `looks_generated()` guard on the write site's
file. The function exists at `arch_model.rs:300–305` and skips files whose content
mentions "do not edit" / "code generated". The plan's Task 1 should add:

```rust
// Skip write sites in generated files
if crate::arch_model::looks_generated(&read_file_source(&edge.file)) { continue; }
```

This is a one-line addition to the core loop. Add to Task 1 scope.

**Update to plan**: Task 1 should read the write site's source file (or check the
`PruningSummary::files_with_parse_errors` / `looks_generated` path) and skip generated
write sites. Note: `ArchModel` doesn't store source text; the checker would need to
re-read the file, same pattern as `GodClassChecker`'s WMC step.

**CONFIRMED**: `build_model` already skips generated files at line 445–448 in
`arch_model.rs` — a `looks_generated(source)` check causes the whole file to be
skipped before any extraction, so generated files' write sites never appear in
`field_accesses`. No extra guard needed in the checker.

**VERIFIED** (`arch_model.rs:445–448`): `looks_generated(source)` is checked at the
top of the per-file loop with an immediate `continue` — the entire file is skipped,
including `raw_field_access_sites` collection. Generated files' write sites never
appear in `model.field_accesses`.

---

### V5 — Bare function write sites missed

**Concern**: `extract_field_access_sites_for_file` only extracts accesses from method
bodies, not bare functions. A bare function in package B writing a package A field is
silently missed.

**Disposition**: KNOWN GAP — DOCUMENT, DO NOT FIX IN THIS PR. This is a limitation of
the extraction layer (#39's scope), not this checker's design. The checker should not
paper over it. Document in `docs/go-encapsulate-variable-false-positives.md` under
"Known limitations":

> Bare functions (non-method) are not tracked — only method bodies are walked for
> field access sites. A bare function writing a cross-package field is a false
> negative. This ceiling is in the extraction layer and will be addressed separately.

---

## Validation of Acceptance Criteria Testability

| AC | Testable? | Test approach |
|----|-----------|---------------|
| AC-1 | Yes | Unit test with `FieldAccessEdge { access: Write, from: "pkgA::T.M", to: "pkgB::U.Field" }` |
| AC-2 | Yes | Same package prefix on `from` and `to` |
| AC-3 | Yes | `to` with lowercase field name after last `.` |
| AC-4 | Yes | `to` with type ending in `Config` |
| AC-5 | Yes | `AccessKind::Read` edge from outside package |
| AC-6 | Yes | `model_registry()` contains name `"encapsulate-variable"`; `default_checks()` contains name `"go-encapsulate-variable"` |
| AC-7 | Yes | Assert on `finding.message` string |
| AC-8 | Procedural | Backtest run, numbers recorded in PR body |
| AC-9 | Integration | Inline-ignore post-pass already tested; check just needs correct `name()` |
| AC-10 | Yes | All above covered by `cargo test` |

---

## Implementation Risk Assessment

**Low risk** overall:
- No new extraction infrastructure (re-uses existing `field_accesses`)
- No new config schema changes
- No new tree-sitter grammar work
- Follows an established `ArchModelChecker` pattern

**Medium risk**:
- Backtest noise (DTO patterns, test setup) — mitigated by suffix allowlist and test
  file exclusion; tunable before ship

**Blocking question before Task 1 starts**:
Confirm whether `arch_model.rs::build_model` already skips generated files before
populating `field_accesses`. Check the main loop at lines 426–540. If it does, V4's
generated-file guard is unnecessary. If it doesn't, add the guard to Task 1.
