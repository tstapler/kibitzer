# Validation: fix-single-letter-bracket-labels

## Plan quality checks

### Does the fix address the root cause?

**Yes.** Root cause (VERIFIED via `src/checkers/markdown_link_integrity.rs`):
`is_non_link_bracket_usage` (line 236) exempts `[x]`/`[X]` via
`label.eq_ignore_ascii_case("x")` but not any other single ASCII letter. The proposed
fix widens this to `label.len() == 1 && label.chars().next().is_some_and(|c| c.is_ascii_alphabetic())`,
which covers `[H]`, `[W]`, `[R]` and all other single-letter labels.

### Is the change minimal?

**Yes.** One logical change: replace a single-char equality check with a single-char
ASCII-alphabetic range check. No new state, no new function, no API surface changes.

### Does it break the existing `[x]` exemption?

**No.** Every single-char ASCII-alphabetic label includes `x`. The widened condition is
a strict superset of the old one. All existing tests (`ignores_a_checkbox_mark_*`) remain
green.

### Are the existing `SEVERITY_TAGS` and colon-contains arms affected?

**No.** The three arms are OR'd independently. The change touches only the first arm.

### Is the false-negative risk acceptable?

**Yes, same trade-off as `[x]`.** A user writing `[H]` as a genuine shortcut reference
link without a definition is already authoring a broken document. If they define `[H]: url`,
`is_dangling` is false before the guard is reached. The suppression only fires on the
"dangling" case, which is the false-positive scenario.

### Does `label.len() == 1` work correctly for ASCII?

**Yes.** For single ASCII characters, byte length equals char length. `"H".len() == 1` is
true. Non-ASCII single characters (e.g. `é`, byte length 2) would not be matched —
correct, since inspection labels are always ASCII. An alternative `label.chars().count() == 1`
is more correct in general but more expensive; since the SEVERITY_TAGS and colon-contains
checks also assume ASCII, the `.len()` approach is consistent with the codebase.

### Are the proposed tests sufficient?

**Yes for AC-1 and AC-2.** The plan adds:
1. `ignores_single_letter_inspection_labels`: parameterised over H, W, R, A, Z —
   covers the repro directly and a cross-section of the full range.
2. `still_flags_explicit_reference_with_single_letter_if_dangling`: confirms the guard
   doesn't suppress `Reference`/`ReferenceUnknown` link types.

AC-0 (existing `[x]` tests) and AC-3 (defined reference) are already covered by
existing tests that will remain green.

### Risk of regressions in the backtest corpus?

**Low.** Single-letter shortcut references are extremely rare in real-world markdown.
The backtest spot-check (Task 4) provides empirical confirmation. Kubernetes and VSCode
docs are large enough to surface any widespread pattern.

## Open questions

1. **Should the doc comment mention inspection-label templates explicitly?**
   The current plan updates the comment to mention `[H]`/`[W]`/`[R]`. This is concrete
   and discoverable but ties the comment to one use case. Alternative: "any single
   ASCII letter, which is almost never a real reference label identifier." Either is fine;
   the implementer should choose what reads most naturally.

2. **`.len() == 1` vs `.chars().count() == 1`**: Both are correct for ASCII. If consistency
   with the broader codebase style matters, check whether other character-length checks in
   the file use one form or the other.

3. **`is_some_and` vs `map_or`**: `is_some_and` requires Rust 1.70+. Verify
   `Cargo.toml`'s `rust-edition` and MSRV before using it; `matches!(label.chars().next(), Some(c) if c.is_ascii_alphabetic())` is equivalent and works on all editions.

## Conclusion

The plan is sound, minimal, and directly addresses the reported false positive. No scope
creep, no new config surface, no breaking changes. Estimated implementation time: < 1h.
