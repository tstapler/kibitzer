# Plan: comment-quality-pep723-exemption

## Summary

One Rust file to edit (`src/checkers/comment_quality.rs`), one doc file to update
(`docs/comment-quality-false-positives.md`). No new files required. The change is
confined to the `comment-quality-python` checker; no other checker or language is
touched.

## Implementation steps

### Step 1 — Add PEP 723 sentinel helpers

Add two private functions to `src/checkers/comment_quality.rs` (near the other
small helper functions, e.g. after `is_brace_closing_annotation`):

```rust
/// Whether `text` is the opening sentinel of a PEP 723 inline script metadata block
/// (`# /// <type>`). The `<type>` must start with an alphanumeric character so that the
/// closing sentinel `# ///` (which has nothing after `///`) is never mistaken for an
/// open.
fn is_pep723_sentinel_open(text: &str) -> bool {
    let s = text.trim();
    let Some(rest) = s.strip_prefix('#') else {
        return false;
    };
    let rest = rest.trim_start();
    let Some(rest) = rest.strip_prefix("///") else {
        return false;
    };
    let rest = rest.trim_start();
    !rest.is_empty() && rest.chars().next().is_some_and(|c| c.is_alphanumeric())
}

/// Whether `text` is the closing sentinel of a PEP 723 inline script metadata block
/// (`# ///`). Trailing whitespace is tolerated.
fn is_pep723_sentinel_close(text: &str) -> bool {
    let s = text.trim();
    let Some(rest) = s.strip_prefix('#') else {
        return false;
    };
    rest.trim() == "///"
}
```

### Step 2 — Thread `in_pep723_block` through the comment loop

In the `check` method (around line 229), add `in_pep723_block` and the skip logic
immediately before the existing `is_directive_only_comment` guard:

```rust
let mut in_fence = false;
let mut in_pep723_block = false;   // ← add this
for comment in &comments {
    let text = comment.utf8_text(ctx.source.as_bytes()).unwrap_or("");
    // PEP 723 inline script metadata blocks (Python-only) are structured TOML, not
    // programmer commentary — exempt the entire block from all checks.
    if self.lang == Language::Python {
        if is_pep723_sentinel_open(text) {
            in_pep723_block = true;
            continue;
        }
        if in_pep723_block {
            if is_pep723_sentinel_close(text) {
                in_pep723_block = false;
            }
            continue;
        }
    }
    if is_directive_only_comment(text) {
        continue;
    }
    let text = &*without_directive_lines(text);
    check_verbose_phrases(*comment, text, &mut findings);
    in_fence =
        check_commented_out_code(*comment, text, ctx.source, in_fence, &mut findings);
}
```

The `continue` on the opening sentinel line means the opener is skipped before any
check. Body lines (`in_pep723_block == true`) are also skipped. The closing sentinel
resets the flag and is also skipped (its own text is trivially non-flaggable, but
skipping it is consistent).

### Step 3 — Add regression tests

In the `#[cfg(test)]` module of `comment_quality.rs`, add two tests:

**Test 1 — Exact repro case: no findings inside PEP 723 block**

```rust
#[test]
fn pep723_inline_script_metadata_block_is_not_flagged_as_commented_out_code() {
    let src = "#!/usr/bin/env python3\n\
               # /// script\n\
               # requires-python = \">=3.11\"\n\
               # dependencies = [\n\
               #   \"rich>=13.0.0\",\n\
               #   \"typer>=0.9.0\",\n\
               # ]\n\
               # ///\n\
               \n\
               def main():\n\
               \    pass\n";
    let findings = run(Language::Python, src);
    assert!(
        !findings
            .iter()
            .any(|f| f.message.contains("[commented-out-code]")),
        "findings: {findings:?}"
    );
}
```

**Test 2 — Real dead code after a closed PEP 723 block is still caught**

```rust
#[test]
fn real_dead_code_after_pep723_block_is_still_flagged() {
    let src = "# /// script\n\
               # dependencies = [\"rich\"]\n\
               # ///\n\
               # x = compute(1, 2)\n\
               def main():\n\
               \    pass\n";
    let findings = run(Language::Python, src);
    assert!(
        findings
            .iter()
            .any(|f| f.message.contains("[commented-out-code]")),
        "findings: {findings:?}"
    );
}
```

### Step 4 — Update docs/comment-quality-false-positives.md

Add a new entry under `## Fixed`:

```markdown
### PEP 723 inline script metadata block misread as commented-out code (2026-10)

- **Symptom**: a Python file with a `# /// script` … `# ///` PEP 723 inline metadata
  block fired `[commented-out-code]` on body lines such as `# dependencies = [`, which
  `is_assignment` read as a real Python assignment (lhs = `dependencies`, rhs = `[`).
- **Mechanism**: PEP 723 body lines use `key = value` TOML syntax, which is syntactically
  identical to Python assignment statements. `is_assignment_lhs` and `is_assignment_rhs`
  both pass on `dependencies = [`.
- **Fixed by**: tracking `in_pep723_block` state in the `check` method's comment-node
  loop (parallel to the `in_fence` tracking inside `check_commented_out_code`). All
  comment-quality checks are skipped for every line inside a PEP 723 block. Guarded by
  `pep723_inline_script_metadata_block_is_not_flagged_as_commented_out_code` and
  `real_dead_code_after_pep723_block_is_still_flagged`.
- **Known trade-off**: if a file contains a *malformed* PEP 723 block (opening sentinel
  with no matching close), lines from the unclosed sentinel to EOF are silently skipped.
  This is acceptable: PEP 723's own tooling (`uv run`) would also reject the malformed
  file, so kibitzer surfacing fewer findings than usual is not worse than the current
  false positive.
```

## Adversarial review

### "What if `in_pep723_block` is never reset (unclosed block)?"

If the file has an opening sentinel but no closing sentinel, `in_pep723_block` stays
`true` for the rest of the file. All comment nodes below the opener are silently skipped.
The trade-off note in the doc update acknowledges this. Since PEP 723 tooling itself
rejects unclosed blocks, this is acceptable — false silences on a broken file beat false
positives on a valid one.

### "Does this break the `in_fence` tracking inside `check_commented_out_code`?"

No. `in_fence` is threaded through `check_commented_out_code`'s return value and only
updated when that function is called. When a PEP 723 block is active, the call to
`check_commented_out_code` is skipped entirely (via `continue`), so `in_fence` is
neither read nor written for those nodes — its state from before the block is preserved
intact for after the block.

### "Could a Go/Rust file accidentally have `# /// script` matched?"

No. `is_pep723_sentinel_open` only runs when `self.lang == Language::Python`, so other
language variants are unaffected.

### "Are the sentinel helpers correct per the PEP 723 spec?"

`is_pep723_sentinel_open` requires: `#` prefix, then `///`, then at least one
alphanumeric character. This matches PEP 723's slug requirement (`[a-zA-Z0-9]…`) and
excludes the close sentinel `# ///`. `is_pep723_sentinel_close` requires: `#` prefix,
then `///` (with leading/trailing whitespace tolerated). Both handle the common
`#!/usr/bin/env python3` shebang correctly: `strip_prefix('#')` → `!/usr/bin/env python3`
→ `trim_start()` → `!/usr/bin/…` which does not `strip_prefix("///")` → returns `false`.

### "Why not also use `in_fence` reset on `# ///` lines to avoid double-counting state?"

`in_fence` and `in_pep723_block` are independent signals. A PEP 723 block body line
could theoretically contain a backtick-fenced sub-example (unlikely but not forbidden
by the spec). By skipping the entire block at the outer `check` loop level, we avoid
any interaction between the two states.
