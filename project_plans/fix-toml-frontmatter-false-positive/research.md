# Research: fix-toml-frontmatter-false-positive

## Mechanism confirmed

### Problem 1: frontmatter parsed as markdown

**Location**: `src/checkers/markdown_link_integrity.rs`, `check_source` (line 79).

The function takes `body: &str` and passes it directly to `pulldown_cmark::Parser::new_with_broken_link_callback(body, opts, Some(callback))` with no preprocessing. `Options::ENABLE_TASKLISTS | Options::ENABLE_WIKILINKS` are set, but there is no metadata-block or frontmatter option in pulldown-cmark for the `+++` TOML delimiter; the `---` YAML delimiter is also not handled.

Hugo TOML frontmatter looks like:
```
+++
title = "My Post"
categories = ["Software Engineering", "Infrastructure"]
tags = ["kotlin", "java", "coroutines"]
+++
```

pulldown-cmark tokenizes `["Software Engineering", "Infrastructure"]` as:
- Literal `[` starts a potential link/reference
- `"Software Engineering", "Infrastructure"` is the label text
- `]` closes the label, no following `(url)` or `[ref]`, making it `LinkType::ShortcutUnknown`

The broken-link callback in `check_source` (`|_broken| Some(("", ""))`) fires for every unknown link, so each array element's first word becomes a dangling shortcut reference and fires a "used but never defined" finding.

Confirmed by reading `check_source` in full: no branch strips or skips the frontmatter block. No test covers this case.

### Problem 2: message prefix confuses output

**Location**: `src/config.rs`, `core_checks()` (line 868-871):

```rust
Check {
    message: Some("broken markdown link/anchor".to_string()),
    ..native_check("markdown-link-integrity", Severity::Blocking, &["**/*.md"])
},
```

`CheckResult::describe()` (`src/check_result.rs:122`) returns `{message}\n{output}` when both are present. The Stop hook formats each result as `{file}: {check_name}: {describe()}`. When `describe()` is multi-line, the first line is `"index.md: markdown-link-integrity: broken markdown link/anchor"` and subsequent lines are the per-line findings.

This is not a bug in itself — it's intended to give context — but it creates confusing output for frontmatter false positives since the first line looks like a summary of a broken link that never gets specified.

### Problem 3: Stop hook repeating findings

**Location**: `src/task_stop.rs`, `stop_hook_context` (lines 203-241).

The offset-advance code is:
```rust
let _ = SessionOffset { offset: new_offset }.save(&offsets_path);
```

The `let _ = ...` discards the `Result`. If `save` fails (directory doesn't exist, permissions error, disk full), `new_offset` is computed but never persisted. The next Stop firing loads `SessionOffset::load(&offsets_path)` which returns `SessionOffset { offset: 0 }` on a missing file, re-processing all transcript lines from the beginning including the original `index.md` edit.

`SessionOffset::save` calls `std::fs::create_dir_all(parent)` before writing, so a missing directory is handled. But a permissions error on the cache directory (or the file itself) would silently discard the offset update. Under normal conditions this doesn't happen, but the user's report of "5 more times after report_false_positive was filed" is consistent with this failure mode.

An independent possibility: multiple Stop hook registrations (global + project-level) fire sequentially in the same process. The first one loads offset 0, computes new_offset, saves it, and runs checks. The second loads the now-updated offset and finds no new lines (correct behavior). This suggests the "5 more firings" are more likely separate Claude turns where the transcript kept growing with new tool_use entries that happened to include `index.md` again — OR the offset save was failing.

## pulldown-cmark frontmatter support

Checked `Cargo.toml`: `pulldown-cmark = "0.13"`. As of 0.13, there is no built-in `Options::ENABLE_FRONT_MATTER` or similar flag for `+++` TOML or `---` YAML frontmatter. The `---` YAML delimiter is not part of CommonMark spec, and `+++` is entirely custom to Hugo. Manual stripping before parsing is the standard approach used by other Rust markdown tools (e.g., `gray-matter`, `front-matter-rs`).

## Fix approach: pre-strip frontmatter

The cleanest fix for Problem 1 is to strip the frontmatter block from `body` before parsing, while adjusting finding line numbers to remain relative to the original file. Steps:

1. In `check_source`, call a new `strip_frontmatter(body)` helper that returns `(stripped_body: &str, line_offset: usize)`.
2. Parse `stripped_body` instead of `body`.
3. Add `line_offset` to every `Finding::line` before returning.

The `strip_frontmatter` helper:
- If the first non-empty line is exactly `+++` or `---` (the delimiter), scan forward for a matching closing delimiter on its own line.
- Return `(body_after_closing_delimiter, number_of_frontmatter_lines_including_both_delimiters)`.
- If no closing delimiter is found (malformed or no frontmatter), return `(body, 0)` unchanged.

**`+++` ambiguity**: `+++` has no other meaning in CommonMark. Safe to strip whenever it appears as the first line.

**`---` ambiguity**: `---` is a valid CommonMark thematic break. A file that opens with `---` as a thematic break rather than YAML frontmatter would be incorrectly treated as having frontmatter. Mitigation: only strip a `---`-delimited block if it has a matching closing `---` AND at least one line inside looks like a YAML key-value pair (`key: value` pattern). This heuristic is imperfect but reduces the false-strip rate for files that open with a thematic break.

An alternative: only handle `+++` in the first version (matches the reported bug exactly), and defer `---` to a follow-up since the YAML case is more ambiguous and not in the user's specific repro. The plan below includes both but notes this as a decision point.

## Line number preservation

`check_source` builds `line_starts = line_start_offsets(body)` and uses `line_for_offset(line_starts, offset)` to convert pulldown-cmark byte offsets to 1-indexed line numbers. After stripping frontmatter, findings' line numbers are relative to `stripped_body`. Adding `line_offset` (the number of lines in the frontmatter block) corrects them to original-file line numbers.

Example: frontmatter is 8 lines (lines 1-8 in the original, with the closing `+++` on line 8). Body starts at line 9. A finding at line 1 of `stripped_body` → original line 9. Adding `line_offset = 8` gives 9. ✓

## Existing test coverage

`src/checkers/markdown_link_integrity_tests.rs` has 50+ tests but none for frontmatter. No test currently passes a file starting with `+++` or `---` to `check_source`. The fix needs new tests:
- TOML `+++` frontmatter arrays: zero findings
- YAML `---` frontmatter arrays: zero findings
- Dangling link in body after frontmatter: exactly one finding at the correct (body) line number
- File without frontmatter: no regression

## Backtest corpus

Standard repos for markdown coverage: `microsoft/vscode` (heavy `.md` docs), `kubernetes/kubernetes` (many `.md` files with YAML frontmatter in some places). The backtest command: `kibitzer check backtest markdown-link-integrity`.

Hugo blog posts in the user's own repo are the primary repro case but are not part of the public corpus — the fix is validated via unit tests plus the standard backtest corpora.

## Prior art in this checker

The pattern of "detect a special markdown construct and skip it" has been done twice already:
- `Options::ENABLE_WIKILINKS` for Obsidian/Logseq wiki-links (a pulldown-cmark option)
- `Options::ENABLE_TASKLISTS` for GFM task-list checkboxes (a pulldown-cmark option)
- `is_non_link_bracket_usage` for `[SEVERITY: High]`-style tags (a heuristic filter)

Frontmatter stripping is the same category but requires pre-processing (not a parser option), since pulldown-cmark has no frontmatter support.
