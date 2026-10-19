# Validation: comment-quality-pep723-exemption

## Plan vs requirements check

| AC | Plan coverage | Status |
|----|--------------|--------|
| 1. No `[commented-out-code]` inside PEP 723 block | Step 2 skips `check_commented_out_code` for all `in_pep723_block` lines; Step 3 test 1 guards it | ✅ covered |
| 2. No `[verbose-comment]` or `[over-commented]` inside block | Step 2's `continue` skips both `check_verbose_phrases` and the `walk_declarations_for_proportionality` call (that walk is outside the loop but only visits tree-sitter declaration nodes; comment nodes inside a PEP 723 block will not be attached to any function declaration, so `over-commented` would not fire anyway — still, `continue` is correct) | ✅ covered |
| 3. Real dead code outside the block still flagged | Step 3 test 2 guards this | ✅ covered |
| 4. Exemption is Python-specific | `if self.lang == Language::Python` guard in Step 2 | ✅ covered |
| 5. Non-`script` block types also exempted | `is_pep723_sentinel_open` accepts any alphanumeric type name, not just `script` | ✅ covered |
| 6. Close sentinel tolerates trailing whitespace | `is_pep723_sentinel_close` uses `.trim()` on the whole line and `rest.trim() == "///"` | ✅ covered |
| 7. Regression test added | Step 3 (two tests) | ✅ covered |
| 8. `docs/comment-quality-false-positives.md` updated | Step 4 | ✅ covered |

## Over-commented edge case (AC 2)

`walk_declarations_for_proportionality` runs after the comment loop; it walks the
tree-sitter tree looking for function declaration nodes, then counts their attached
leading comment nodes. A PEP 723 block at the top of the file is not attached to any
function declaration (it is a module-level comment before any `def`), so
`check_proportionality` never sees those nodes. AC 2 for `[over-commented]` is thus
satisfied incidentally as well as by the `continue`.

## Risk: unclosed PEP 723 block

If a file has an opening `# /// script` but no matching `# ///`, `in_pep723_block`
stays `true` and the rest of the file's comments are silently skipped. This is a
latent silent-failure mode. Mitigation options:

1. Emit a single `[commented-out-code]` (or a dedicated finding) if `in_pep723_block`
   is still `true` at the end of the file. This is an improvement but is out of scope
   for this fix; tracked as a suggestion in the suggestions list.
2. Accept the current behavior: a malformed PEP 723 file that `uv run` would reject
   anyway generates no findings — worse than ideal but not worse than the status quo
   (the current false positive).

Decision: accept risk for now; note it in docs.

## Risk: `in_fence` interaction

The research doc confirmed `in_fence` and `in_pep723_block` are independent. The
`in_fence` state is only updated inside `check_commented_out_code`, which is not called
for `in_pep723_block` nodes. No interaction hazard.

## Open questions

None that block implementation. The one residual risk (unclosed block) is documented
and accepted.
