# Plan: fix-single-letter-bracket-labels

## Summary

Extend `is_non_link_bracket_usage` in `src/checkers/markdown_link_integrity.rs` to
suppress any single ASCII alphabetic character used as a shortcut reference label. The
change is a one-line widening of the existing `[x]` exemption to cover `[H]`, `[W]`,
`[R]` and the full `[A]`–`[Z]`/`[a]`–`[z]` range. Add two unit tests and update the
false-positives log.

## Work breakdown

### Task 1 — Widen single-char guard in `is_non_link_bracket_usage`

**File**: `src/checkers/markdown_link_integrity.rs`

Change line 242 from:
```rust
    label.eq_ignore_ascii_case("x") || label.contains(':') || SEVERITY_TAGS.contains(&label)
```
to:
```rust
    (label.len() == 1 && label.chars().next().is_some_and(|c| c.is_ascii_alphabetic()))
        || label.contains(':')
        || SEVERITY_TAGS.contains(&label)
```

Update the doc comment (line 232–235). Replace the phrase:

```
a `[x]`/`[X]` mark
```

with:

```
any single ASCII letter (including `[x]`/`[X]` checkbox marks, `[H]`/`[W]`/`[R]` inspection labels, etc.)
```

**Why this change only**: the existing `SEVERITY_TAGS` list and colon-contains guard
remain intact — they cover multi-letter uppercase tags and colon-separated phrases. The
new arm replaces only the `eq_ignore_ascii_case("x")` clause and is strictly broader:
`x` is an ASCII alphabetic character, so every case the old guard caught is still caught.

### Task 2 — Add regression tests

**File**: `src/checkers/markdown_link_integrity_tests.rs`

Add after the existing `ignores_an_uppercase_checkbox_mark_mid_sentence` test (~line 253):

```rust
#[test]
fn ignores_single_letter_inspection_labels() {
    // [H] Hold / [W] Witness / [R] Review labels from planning templates
    for label in ["H", "W", "R", "A", "Z"] {
        let body = format!("- [{label}] Inspection point description\n");
        assert!(
            check_source(&path(), &body).unwrap().is_empty(),
            "expected no findings for [{label}]"
        );
    }
}

#[test]
fn still_flags_explicit_reference_with_single_letter_if_dangling() {
    // [text][H] is an explicit Reference, not a Shortcut — still flagged
    let findings = check_source(&path(), "See [details][H] here.\n").unwrap();
    assert_eq!(findings.len(), 1, "got: {findings:?}");
    assert!(findings[0].message.contains("[H] used but never defined"));
}
```

### Task 3 — Update false-positives log

**File**: `docs/markdown-link-integrity-false-positives.md`

Add a new dated entry (2026-10-23) documenting:
- Symptom: `[H]`/`[W]`/`[R]` shortcut labels on bullet lines flagged as "used but
  never defined"
- Root cause: `is_non_link_bracket_usage` only exempted `[x]`/`[X]`, not the full
  single-letter range
- Fix: widened guard to `label.len() == 1 && is_ascii_alphabetic()`
- Commit: `<sha>` (fill in at PR time)

### Task 4 — Backtest spot-check

Run the updated checker against two backtest repos to confirm no regressions:

```bash
# From repo root, after `cargo build`:
find ~/code/github.com/kubernetes/kubernetes -name '*.md' | \
    xargs -P4 -I{} ./target/debug/kibitzer check native markdown-link-integrity {} 2>&1 | \
    grep 'used but never defined' | grep -E '^\S+:[0-9]+: \[[A-Za-z]\]' | head -20
```

Expected: zero matches (no single-letter labels flagged after the fix).

## Adversarial review

**Q: Does widening `[x]` to all single letters suppress a real shortcut reference?**
A: A user who writes `[A]` intending a hyperlink almost certainly defines `[A]: url`
somewhere in the document. If they do, `is_dangling` is false and this guard is never
reached. The only suppressed case is `[A]` with no definition — already a document bug.
The risk is structurally identical to the already-accepted `[x]` exemption.

**Q: Is `label.len() == 1` byte-length or char-length?**
A: In Rust, `.len()` on `&str` is byte length. For single ASCII characters byte length
equals char length. The guard is correct. For safety, a comment could note this; or use
`.chars().count() == 1 && ...`. Prefer using `chars().next().is_some_and(|c| c.is_ascii_alphabetic() && c.len_utf8() == 1)` pattern for clarity, or just document the ASCII assumption.

**Q: What about `[ ]` (space-only label, i.e. unchecked task-list mid-sentence)?**
A: `[ ]` is whitespace-only and is not a valid CommonMark link label (spec §6.6: "A
link label consists of zero or more characters between the opening and closing
brackets. It cannot consist entirely of whitespace."). pulldown-cmark does not emit it
as a link event — confirmed by the existing spec commentary in the source file's
`ENABLE_TASKLISTS` comment block. No change needed.

**Q: Could `label.trim()` produce an empty string for `[H]`?**
A: No — `[H]` raw label is `H`, trimming gives `H`, length 1. Empty string only from
whitespace-only content.

**Q: Does this interact with `normalize_label` (lowercasing) in `record_link_or_image`?**
A: The call site passes `&id` (raw label), not `&normalized`. The function applies its
own `trim()` but not lowercase — so `raw_label` for `[H]` is `"H"`. After trim:
`"H".len() == 1` and `'H'.is_ascii_alphabetic()` → true. Correct.

## Decision record

- Chose single-char alphabetic only (not ALL_CAPS multi-letter) — consistent with the
  existing `[x]` precedent; narrower scope avoids the `[RFC]` debate that blocked the
  broader ALL_CAPS heuristic in the previous plan.
- No config toggle — single-char labels as reference links are too rare to need opt-out.
- No changes to `SEVERITY_TAGS` or colon-contains logic — those remain independent arms.
