# Plan: fix-toml-frontmatter-false-positive

## Summary

Three related problems. The primary fix is small and contained to
`src/checkers/markdown_link_integrity.rs`: add a `strip_frontmatter` helper that removes
`+++` (TOML) and `---` (YAML) frontmatter blocks before the markdown parser sees them,
adjusting finding line numbers so they remain correct relative to the original file.
A secondary fix tightens error handling in the Stop hook's offset-save path so a silent
save failure does not cause repeated false-positive reports across subsequent turns.

## Work breakdown

### Phase 1 — Add `strip_frontmatter` to `markdown_link_integrity.rs`

**File**: `src/checkers/markdown_link_integrity.rs`

Add a private helper function:

```rust
/// Returns the markdown body after the leading frontmatter block is removed, plus
/// the number of frontmatter lines (both delimiter lines included). If the body does
/// not start with a recognized frontmatter delimiter (`+++` or `---`) or has no
/// matching closing delimiter, returns `(body, 0)` unchanged.
///
/// For `---`: only strips if at least one line inside the block contains `:` (a minimal
/// YAML key-value heuristic), to avoid misidentifying a leading thematic break `---`
/// as a frontmatter opener. This heuristic is not exhaustive but covers the common case.
fn strip_frontmatter(body: &str) -> (&str, usize) {
    for delimiter in &["+++", "---"] {
        if let Some(result) = try_strip_frontmatter(body, delimiter) {
            return result;
        }
    }
    (body, 0)
}

fn try_strip_frontmatter<'a>(body: &'a str, delimiter: &str) -> Option<(&'a str, usize)> {
    // First line must be exactly the delimiter (no trailing whitespace matters, but we
    // trim to tolerate Windows-style CRLF as well).
    let first_newline = body.find('\n')?;
    if body[..first_newline].trim() != delimiter {
        return None;
    }

    let mut pos = first_newline + 1;
    let mut inner_line_count = 0usize;
    let mut has_kv_pair = false; // for `---` disambiguation

    while pos < body.len() {
        let line_end = body[pos..].find('\n').map(|i| pos + i).unwrap_or(body.len());
        let line = body[pos..line_end].trim();
        inner_line_count += 1;

        if line == delimiter {
            // Reject `---`-frontmatter that has no key: value line inside — it's
            // probably a thematic break followed by body content, not YAML metadata.
            if *delimiter == "---" && !has_kv_pair {
                return None;
            }
            let after_close = if line_end < body.len() { line_end + 1 } else { body.len() };
            // +1 for the opening delimiter line, +1 for the closing delimiter line
            let total_lines = 1 + inner_line_count;
            return Some((&body[after_close..], total_lines));
        }
        if line.contains(':') {
            has_kv_pair = true;
        }
        pos = if line_end < body.len() { line_end + 1 } else { body.len() };
    }
    None // No closing delimiter found — not valid frontmatter; leave body untouched.
}
```

Modify `check_source` to call `strip_frontmatter` and apply the line offset:

```rust
pub fn check_source(path: &Path, body: &str) -> Result<Vec<Finding>> {
    let (parse_body, frontmatter_lines) = strip_frontmatter(body);
    let line_starts = line_start_offsets(parse_body);
    // ... (rest of current logic using parse_body instead of body) ...
    
    // Adjust line numbers to be relative to the original file.
    if frontmatter_lines > 0 {
        for f in findings.iter_mut() {
            f.line += frontmatter_lines;
        }
    }
    findings.sort_by_key(|f| f.line);
    findings.dedup();
    Ok(findings)
}
```

**Scope note**: only `check_source` needs updating. The file-read path (`MarkdownLinkIntegrityChecker::check`) calls `check_source(file, ctx.source)` unchanged — `ctx.source` is the full file content, and the stripping happens inside.

### Phase 2 — Add regression tests

**File**: `src/checkers/markdown_link_integrity_tests.rs`

New test cases:

```rust
#[test]
fn toml_frontmatter_arrays_are_not_flagged() {
    let body = "+++\ntitle = \"My Post\"\ncategories = [\"Software Engineering\", \"Infrastructure\"]\ntags = [\"kotlin\", \"java\"]\n+++\n\nBody content.\n";
    let findings = check_source(&path(), body).unwrap();
    assert!(findings.is_empty(), "frontmatter arrays must not be flagged: {findings:?}");
}

#[test]
fn yaml_frontmatter_arrays_are_not_flagged() {
    let body = "---\ntitle: My Post\ncategories:\n  - Software Engineering\n  - Infrastructure\n---\n\nBody content.\n";
    let findings = check_source(&path(), body).unwrap();
    assert!(findings.is_empty(), "yaml frontmatter must not be flagged: {findings:?}");
}

#[test]
fn dangling_ref_in_body_after_toml_frontmatter_is_flagged_at_correct_line() {
    // Frontmatter is lines 1-5; body starts at line 6.
    // The dangling reference is in the body, so it should be reported at line 7
    // (the second body line).
    let body = "+++\ntitle = \"Post\"\n+++\n\nSome text.\n\nSee [thing][missing].\n";
    let findings = check_source(&path(), body).unwrap();
    assert_eq!(findings.len(), 1);
    assert!(findings[0].message.contains("[missing] used but never defined"));
    // Line 7 in the full file (3 frontmatter lines + blank + "Some text." + blank + "See...")
    assert_eq!(findings[0].line, 7, "line number must reflect original file position");
}

#[test]
fn leading_thematic_break_without_kv_is_not_stripped_as_frontmatter() {
    // A `---` thematic break followed by content — no YAML key: value inside.
    let body = "---\n\nBody paragraph with [thing][missing].\n";
    let findings = check_source(&path(), body).unwrap();
    // If incorrectly treated as frontmatter, the `[missing]` finding would be
    // suppressed or misplaced. It should fire normally.
    assert_eq!(findings.len(), 1, "body link must still be flagged");
}

#[test]
fn unclosed_frontmatter_delimiter_does_not_strip_anything() {
    // A `+++` with no matching closing `+++` must not silently eat the whole file.
    let body = "+++\ntitle = \"Broken\"\n\nThis body has [thing][missing].\n";
    let findings = check_source(&path(), body).unwrap();
    // The file is not valid frontmatter (no closing `+++`), so body is parsed whole.
    // The `+++` and array lines themselves may or may not fire — what must NOT happen
    // is `[missing]` being silently dropped.
    assert!(findings.iter().any(|f| f.message.contains("[missing] used but never defined")));
}
```

### Phase 3 — Fix Stop hook offset save error handling

**File**: `src/task_stop.rs`, `stop_hook_context` (line 228):

Change from:
```rust
let _ = SessionOffset { offset: new_offset }.save(&offsets_path);
```

To:
```rust
if let Err(e) = (SessionOffset { offset: new_offset }.save(&offsets_path)) {
    // If the offset can't be saved, re-reporting the same findings on the next Stop
    // is worse than reporting nothing at all — the Stop hook is advisory-only.
    // Log to stderr (visible during debugging) and skip the findings report.
    eprintln!("[kibitzer] warning: could not save stop-hook offset ({e:#}); skipping report to avoid repeated findings");
    return Ok(None);
}
```

This converts a silent failure that causes repeated findings into an explicit diagnostic + no-op, which is safer since the Stop hook is already advisory-only.

### Phase 4 — Update false-positives log

**File**: `docs/markdown-link-integrity-false-positives.md`

Add a new log entry under `## Log` (after the most recent existing entry):

```markdown
### 2026-11-10 — Hugo blog post — TOML `+++` frontmatter arrays misread as undefined shortcut references

- **What changed**: edited a Hugo blog post (`content/blog/<slug>/index.md`) with TOML
  frontmatter (`+++` delimiters). Lines 5-7 were `categories`/`tags`/`keywords` arrays.
- **Why it's a false positive**: `["Software Engineering", "Infrastructure"]` inside a
  `+++` frontmatter block is not markdown — it's TOML syntax. pulldown-cmark tokenizes it
  as shortcut reference link candidates with `LinkType::ShortcutUnknown`.
- **Mechanism**: `check_source` did not strip the frontmatter block before parsing.
- **Status: resolved.** `strip_frontmatter` now removes the leading `+++` or `---` block
  before parsing, and adjusts finding line numbers to remain correct relative to the
  original file. Tests: `toml_frontmatter_arrays_are_not_flagged`,
  `yaml_frontmatter_arrays_are_not_flagged`,
  `dangling_ref_in_body_after_toml_frontmatter_is_flagged_at_correct_line`.
```

### Phase 5 — Backtest

Run `kibitzer check backtest markdown-link-integrity` against session transcripts.
Run `kibitzer check native markdown-link-integrity` against `microsoft/vscode` and
`kubernetes/kubernetes` from the standard backtest corpus (`scripts/clone-backtest-repos.sh`).

Record before/after false-positive counts and any new regressions in this plan or the
false-positives doc.

## Decision points

**D1: `---` disambiguation heuristic.** The plan includes a `has_kv_pair` check for
`---` frontmatter to avoid misidentifying thematic breaks. An alternative is to skip `---`
stripping entirely in v1 (TOML `+++` is unambiguous and fixes the reported bug). If the
heuristic proves fragile in backtesting, strip `+++` only and defer `---`.

**D2: Stop hook no-op vs. log-and-continue.** Phase 3 proposes skipping the findings
report entirely if the offset save fails. An alternative: log the warning but still report
findings anyway (accepting the repeat risk). The plan chooses no-op because repeated advisory
findings on a correct file are more disruptive than a single missing report cycle.

**D3: Line number adjustment scope.** `strip_frontmatter` returns a `(&str, usize)` and the
adjustment happens inside `check_source`. The alternative is to adjust at the call site in
`MarkdownLinkIntegrityChecker::check`, but that leaks an internal detail through the
`Checker` trait boundary. Keeping it inside `check_source` is cleaner.

## Adversarial review

**Q: Can a valid markdown link look like TOML/YAML and be incorrectly stripped?**
A: Only if the markdown body itself starts with `+++` or `---` on its very first line —
an extremely unlikely document structure. Any document that starts with `+++` and has a
matching `+++` later would be silently stripped of its opening block. The risk is low and
accepted.

**Q: Does stripping frontmatter affect the `extract_headings` helper used for cross-file anchors?**
A: No. `extract_headings` is called in `reference_target_finding` to build the anchor set for a
*cross-file* reference target. It reads another file's content via `std::fs::read_to_string`.
That path does NOT call `check_source`, so it doesn't strip frontmatter from the target file
when building its heading set. This is acceptable: frontmatter headings are not real HTML
anchors in a Hugo-rendered site. If a cross-file reference targets a frontmatter "heading",
it would still be missed, but that's an unlikely use case and a separate issue.

**Q: Will the line offset break the Stop hook's `predates_git_head` check?**
A: No. `predates_git_head` calls `check_native_against_git_head`, which runs the checker
against the git-HEAD version of the file. That path also calls `check_source` (via
`run_checker_against_source`), which will strip frontmatter consistently, so both the
live and HEAD results use the same line numbers. ✓

**Q: What if the same file uses both `+++` and `---` delimiters (malformed)?**
A: `strip_frontmatter` tries `+++` first. If the file starts with `+++` but only has a
`---` closer (or vice versa), no match is found and the file is passed through unchanged.
This is the safest fallback for malformed files.
