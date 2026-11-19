# Validation: fix-toml-frontmatter-false-positive

## Acceptance criteria checklist

| AC | Description | Status |
|----|-------------|--------|
| AC-1 | TOML `+++` frontmatter arrays produce zero findings | pending |
| AC-2 | YAML `---` frontmatter arrays produce zero findings | pending |
| AC-3 | Body content after frontmatter still flagged normally | pending |
| AC-4 | No regression on files without frontmatter | pending |
| AC-5 | Unit tests for all frontmatter cases added | pending |
| AC-6 | Backtested with `kibitzer check backtest` and standard repos | pending |
| AC-7 | False-positives log entry added | pending |
| AC-8 | Stop hook offset save failure surfaced, not silently discarded | pending |

## Validation approach

### AC-1 and AC-2: frontmatter stripping

The new `toml_frontmatter_arrays_are_not_flagged` and `yaml_frontmatter_arrays_are_not_flagged`
tests cover these directly. Run:

```
cargo test -p kibitzer toml_frontmatter
cargo test -p kibitzer yaml_frontmatter
```

Expected: both pass, zero findings.

### AC-3: body content still checked

The `dangling_ref_in_body_after_toml_frontmatter_is_flagged_at_correct_line` test covers
this. Verifies both that the finding fires AND that its line number is correct relative to
the original file (not the stripped body).

### AC-4: no regression on files without frontmatter

All existing `markdown_link_integrity_tests.rs` tests cover files without frontmatter.
Run the full test suite:

```
cargo test -p kibitzer markdown_link_integrity
```

Expected: all pre-existing tests still pass.

### AC-5: test coverage

Confirm the following new test names exist in `src/checkers/markdown_link_integrity_tests.rs`:
- `toml_frontmatter_arrays_are_not_flagged`
- `yaml_frontmatter_arrays_are_not_flagged`
- `dangling_ref_in_body_after_toml_frontmatter_is_flagged_at_correct_line`
- `leading_thematic_break_without_kv_is_not_stripped_as_frontmatter`
- `unclosed_frontmatter_delimiter_does_not_strip_anything`

### AC-6: backtest

Run the two backtest tools and record results:

```bash
# Transcript-based backtest:
kibitzer check backtest markdown-link-integrity

# Corpus-based (need repos cloned first via scripts/clone-backtest-repos.sh):
find ~/code/backtest-repos/microsoft/vscode -name '*.md' | \
    xargs -I{} kibitzer check native markdown-link-integrity {} 2>/dev/null | \
    grep -v "^$" | wc -l

find ~/code/backtest-repos/kubernetes/kubernetes -name '*.md' | \
    xargs -I{} kibitzer check native markdown-link-integrity {} 2>/dev/null | \
    grep -v "^$" | wc -l
```

Record: before-count / after-count for each corpus. A drop in false-positive count
(files with frontmatter that previously triggered findings) is expected; a rise in
count would indicate a regression.

### AC-7: false-positives log

Confirm `docs/markdown-link-integrity-false-positives.md` contains a new entry dated
2026-11-10 describing the TOML frontmatter mechanism and linking to the fix.

### AC-8: Stop hook offset error handling

Manual verification: set `KIBITZER_CACHE_DIR` to a non-writable path and trigger a Stop
hook. Confirm:
- stderr contains `[kibitzer] warning: could not save stop-hook offset`
- No findings are reported for that Stop firing
- The process exits 0 (advisory only, no blocking)

(Note: the env-var injection requires the `default_offsets_dir()` function to respect
`XDG_CACHE_HOME`, which it already does — set `XDG_CACHE_HOME=/nonexistent`.)

## Edge cases to verify manually

1. **Empty frontmatter**: `+++\n+++\n` — should produce zero findings and not strip
   body content.

2. **Frontmatter followed by a dangling reference inside a code block**: the code block
   should still suppress the reference, same as without frontmatter.

3. **A file starting with `---` as a thematic break** (no YAML inside, no matching
   `---`): should not be stripped. The
   `leading_thematic_break_without_kv_is_not_stripped_as_frontmatter` test covers a
   simpler version of this.

4. **A Hugo post whose frontmatter references another `.md` file** (unlikely, but `+++`
   blocks can contain string values): the string values in TOML don't parse as markdown
   links under pulldown-cmark, so no false positive. Confirmed by the mechanism: only
   `[...]` bracket syntax in the parse stream can produce `ShortcutUnknown` events.

## Risks

**R1: `---` heuristic false negatives.** A YAML frontmatter file with only boolean or
numeric values (no `:` in an inner line) would not be stripped. For example:

```yaml
---
draft: true
---
```

This has `: ` in the `draft: true` line, so `has_kv_pair = true`. ✓

```yaml
---
true
---
```

This has no `:` and would NOT be stripped. However, this is not valid YAML frontmatter
for Hugo or Jekyll, so it's an unlikely real-world case. If it becomes an issue, loosen
the heuristic or remove it for `---`.

**R2: `---` YAML frontmatter that ends with `---` matching a later thematic break.**
If a file has `---` at the top (frontmatter start), content, then another `---` as a
horizontal rule, our logic would stop at the first matching `---` and correctly identify
the frontmatter block. The thematic break is after the frontmatter and would be in the
body. ✓

**R3: Very long frontmatter blocks.** No line-count cap is implemented. A pathological
file with frontmatter spanning thousands of lines would still be handled correctly, just
doing extra scanning. Not a performance concern for the file sizes involved.
