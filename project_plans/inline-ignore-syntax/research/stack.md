# Research: Stack (inline-ignore-syntax)

All claims are VERIFIED by reading the cited file/line unless marked INFERRED.

## Existing deps (Cargo.toml `[dependencies]`)
- `tree-sitter 0.26` plus grammars for Go, TypeScript(+Tsx), JavaScript, Python, Java, Kotlin (kotlin-ng), Rust. These are the eight `Language` variants in `src/checker.rs:28-37`.
- `regex 1`, `pulldown-cmark 0.13` (markdown), `serde`/`serde_json`, `clap 4`, `anyhow`.
- No new crate is needed. No Vale/lang-detection/comment-parsing crate is present or required.

## Comment detection: already tree-sitter based
- `comment_quality.rs:173-182` `comment_kinds(lang)` maps Language to comment node kinds: `"comment"` for Go/Python/TS/Tsx/JS, `"line_comment"`/`"block_comment"` for Java/Kotlin/Rust.
- `comment_quality.rs:259-267` `collect_comments` is a private recursive collector (recurses per depth level, early-returns on a comment node). `comment_quality.rs:225-233` obtains the tree via `ctx.tree` and calls it.
- `src/tree_walk.rs:25` `pub(crate) fn walk_preorder(node, &mut impl FnMut(Node)->bool)` is the iterative, stack-safe walker (doc at `tree_walk.rs:3-24`). Preferred over copying `collect_comments`; `comment_kinds` would need to be made `pub(crate)` (or moved to a shared module) for reuse.
- Tree-sitter comment nodes avoid the comment-in-string false-match rabbit hole (a `// kibitzer:ignore` inside a string literal is not a comment node). Position: `comment.start_position().row + 1` gives the 1-based line (`comment_quality.rs:301`).
- Line-comment prefixes (`//`, `#`) need stripping from node text; block comments (`/* */`) and Python `#` also arrive as comment nodes. Per-language prefix stripping is the only extra per-language table needed (INFERRED; the node text includes the delimiter).

## Gaps for the feature
- Markdown and `.md`-only checkers have no tree-sitter grammar: `Language` has no Markdown variant (`src/checker.rs:28-37`), and `markdown_link_integrity.rs:5` uses `pulldown-cmark` events. `<!-- -->` ignores need either `pulldown_cmark::Event::Html`/`InlineHtml` scanning or a regex over source lines (INFERRED: Html events carry the raw comment text; not tested). Fenced-code `<!-- -->` would be correctly skipped by pulldown-cmark but not by a line regex.
- The `Checker::check` signature is `check(&self, file, &CheckContext{source, tree: Option<&Tree>})` (`src/checker.rs:112-115,132`). `tree` is `None` for non-language checkers, so a tree-sitter-only scan cannot cover all native checkers; a fallback is needed for them.

## Where the filter hooks in (reuse point)
- `accepted_findings::filter_accepted` (`src/accepted_findings.rs:131-176`) is line-oriented: it parses `{file}:{line}: {message}` output text, derives the rule via `extract_rule` (`accepted_findings.rs:105-110`: `[rule-id]` prefix, else checker name as fallback), reads the file from disk (`:149`), and drops matching lines.
- It early-returns without reading the file unless some accepted entry names that file (`accepted_findings.rs:142-147`); an inline-ignore pass cannot use that shortcut and would read/scan every file with findings (cheap: only runs when `!passed`, see below).
- The single call chain is `check.rs:482-490`: inside `if passed {..} else { drop_accepted_findings(...) }`, so it only runs when findings remain after diff-scoping. `drop_accepted_findings` (`check.rs:528-543`) just relativizes and delegates. Ignore handling placed here or beside it automatically covers all entry points that go through this function. Entry points load `AcceptedFindings` at `run.rs:129`, `daemon.rs:170,350`, `mcp.rs:778,970`, `lsp.rs:85`, and pass it into the same check path.
- `CheckResult.findings` stays empty for native/shell checks (`check.rs:~508`, `findings: Vec::new()`); structured `ArchFinding` is arch-only, so line-text filtering is the established pattern.
- Since `filter_accepted` already has `file_path` and re-reads the source, an inline-ignore filter can be a sibling function taking the same `(output, file_path, rel_file, checker_name)` and reusing `extract_rule` (make it `pub(crate)`). Reading source twice is avoidable by reading once and sharing.

## Design options
1. **Post-hoc text filter (recommended for appetite)**: after findings are produced, scan source for ignore comments, build `{line -> [(rule, reason, fp)]}`, drop output lines whose line (or line+1 for next-line form) has a matching rule. Reuses output format and `extract_rule`; covers all entry points (above). Language comment detection: tree-sitter via `walk_preorder` + `comment_kinds` when a `Language` applies, otherwise a regex fallback. Needs a parse here, since the tree is created inside `run_checker_configured` (`checker.rs:~220-234`) and not returned; re-parse is a cost, or use regex-only prefix scan.
2. **Regex-only line scan**: `regex` is already a dependency. One pass for `(?://|#|<!--|/\*)\s*kibitzer:ignore\s+...`. Cheapest and language-agnostic, but matches inside string literals/heredocs (rabbit hole in requirements). Mitigation: require the marker to be the first non-whitespace token on the line or after a comment prefix at end of code (INFERRED, would still have rare false matches).
3. **Integrate into checker dispatch** (filter `Vec<Finding>` before formatting in `check.rs:~555-565` `run_checker_against_source`, where `findings` are still structured with `.line`/`.message`): avoids text re-parsing, and has the tree in scope. Cost: changes a shared function and diverges from `accepted/`'s location; baseline-vs-HEAD logic (`check.rs:~497`) also calls the native checker, so ignores would apply there too (probably desirable, INFERRED).

## Gotchas relevant to stack choice
- `Finding` is only `{line, message}` (`checker.rs:12-15`): no column or end line, so "flagged statement spans multiple lines" anchoring must be by `line` only; the comment placement rule (same line or line above the reported line) is the only expressible scope.
- Cross-file duplicate findings carry one `line` in the file being checked (output is per file), so the ignore must be placed in the file being reported; not further verified.
- Ignore comments must not trigger `commented-out-code`/`verbose-comment`: comment_quality scans all comment nodes (`comment_quality.rs:239-244`), so it needs an explicit skip for comments that parse as ignore directives (INFERRED; not tested).
- `.kibitzer/accepted` entries use `rule` as `[rule-id]` or checker name (`accepted_findings.rs:21-24`), so the ignore syntax's `<rule>` should use the same `extract_rule` namespace for consistency.

## Not verified
- Whether the Kotlin/TS grammars emit additional comment kinds (e.g. `multiline_comment`, `hash_bang_line`): `comment_kinds` lists only the above and backtests exist for it, but I did not inspect grammar `node-types.json`.
- Performance of re-parsing vs regex scan: not measured.
