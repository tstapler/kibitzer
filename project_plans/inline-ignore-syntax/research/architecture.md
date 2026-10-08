# Architecture research: inline-ignore-syntax

Line refs are against master @ 3660e64.

## 1. How `filter_accepted` is wired

Every entry point funnels into one function, `check::run_native_check` (`src/check.rs:~409-522`):

- **Entry points** load `AcceptedFindings` once per batch and pass it down:
  `run.rs:129` (then `run_check` at :138 and `run_checks_for_trigger` at :147), `daemon.rs:170` and `:350` (`run_checks_smart`), `lsp.rs:85`, `mcp.rs:778` (`run_check` at :886) and `mcp.rs:970`.
- **`hook.rs` has no direct reference.** It calls `daemon::run_checks_smart` (`hook.rs:161`), which reaches `run_checks_for_trigger` (`check.rs:1453`), then `run_check` (`check.rs:165`), then `run_native_check`.
- **Inside `run_native_check`**: `run_checker_against_file` (`check.rs:576`) reads the file, then `run_checker_against_source` (`check.rs:549`) runs the checker. Structured `Vec<Finding>` is flattened to `"{path}:{line}: {message}"` text at `check.rs:558-562`. Diff-scoping (`check.rs:473`) runs next, and `drop_accepted_findings` (`check.rs:482-489`, defined :528) runs only when `!passed`. A git-HEAD baseline downgrade follows at :494.
- **Native-only.** Shell-out checks never call it. Architecture checks are out of scope per `accepted_findings.rs:128-130`.

## 2. What `filter_accepted` receives

- **Input** (`accepted_findings.rs:131-137`): flattened output text, `file_path`, `rel_file`, `checker_name`, `&AcceptedFindings`. There are no structured findings and no file content.
- **Source lines**: it re-reads the file from disk (`accepted_findings.rs:147`), but only if some entry names that file (fast path :141). The file was already read once at `check.rs:586`, so reading again is wasteful.
- **Line number and rule**: re-parsed from text. It strips the `"{path}:"` prefix, splits on `": "` and parses the line (`accepted_findings.rs:150-158`). `extract_rule` (:105-110) takes the leading `[rule-id]` from the message, else falls back to `checker_name`. The original `Finding.line` (1-based, `checker.rs:12-15`) is thrown away and recovered by string parsing.

## 3. Best seam for inline ignores

**`run_checker_against_source` (`check.rs:549-564`), before the `Vec<Finding>` is flattened to text.** The source string and structured findings are both in hand, so there is no second read and no text re-parse.

- Add a new module (suggested `src/inline_ignores.rs`) with a pure function: `apply_inline_ignores(findings: Vec<Finding>, source: &str, checker_name: &str, ext/Language) -> (kept: Vec<Finding>, malformed: Vec<Finding>)`. Reuse `extract_rule` (make it `pub(crate)`).
- Why this beats extending `filter_accepted`:
  - The comment is part of the file content, so unlike `accepted/` it needs no config load.
  - It needs the source text, which `filter_accepted` lacks.
  - It runs before diff-scoping, so a suppressed finding never reaches `scope_output_to_changed_lines`.
  - It also covers the baseline path: `check_native_against_git_head` (`check.rs:594`) reuses `run_checker_against_source`, so a HEAD violation already ignored at HEAD stays consistent.
- Caveats:
  - `run_checker_against_source` is also called for HEAD content and by `kibitzer check native`, so confirm no caller wants raw findings. An `ignores: bool` parameter or a thin wrapper is the escape hatch.
  - The "malformed ignore" finding must be emitted here too, as an extra `Finding` with its own rule id (e.g. `[inline-ignore]`).
  - Anchor and listing for `false-positives`: the listing command should scan source directly with the same parser, not go through the check pipeline.
- Optional: `lsp.rs` and `mcp.rs:886` need no change, because they go through `run_check`.

## 4. Non-line-anchored or multi-location findings

- `file-size` (`checkers/file_size.rs:120-126`): reports at `line: lines`, the last line of the file. A "flagged line" comment is awkward there. Decide a convention, e.g. an ignore anywhere in the file, or on the last line.
- `duplicate-code-cross-file` (`checkers/duplicate_cross_file_checker.rs:261-269`): reports at `start + 1` in this file only and lists other files in the message text (`:256`). Each file's finding is independent, so the ignore goes in the file that is flagged. No `[rule]` prefix, so the rule falls back to the checker name. Note the other locations are not suppressed by one ignore.
- `duplicate-code` (`checkers/duplicate_code.rs:115`): anchors at the last occurrence (`starts[len-1] + 1`), so the ignore belongs on the last duplicate block's first line, not the first.
- Multi-line constructs (functions with `complexity`, `file-complexity`): anchor line is the Finding's line, not necessarily the flagged statement. Same-line-or-line-above should be defined relative to `Finding.line`.
- Markdown checks: `markdown-link-integrity` has multiple `Finding` constructors (`markdown_link_integrity.rs:262-395`), including a reference-definition finding on a different line than the use. `<!-- -->` placement can render or interfere. Not a blocker, but needs fixtures.
- Message prefix is inconsistent: any checker without `[rule]` uses the checker name (e.g. `syntax-rules-<lang>`), and `extract_rule` is the existing resolution. An ignore naming a rule that is not the self-prefix would silently not match, so unknown-rule ignores should be reported (ties to the stale-ignore open question).

## 5. Existing analysis and size

- `project_plans/checker-plugin-system/research/architecture.md:92-100` states that every consumer (`run.rs`, `daemon.rs`, `hook.rs`, `mcp.rs`) goes through `config.checks` generically, and plugins are injected with zero changes to the entry points. That is the same property here: the seam below `run_check` needs no entry-point change.
- No hotspot or churn doc covers `check.rs`. `project_plans/hide-delegate-check/research/architecture.md:259` and `type-hierarchy-graph/research/architecture.md:184` reached similar conclusions for other files.
- `src/check.rs` is 3043 lines and has ~25 top-level fns (grep `^fn |^pub fn `), including `run_checks_for_trigger`, `run_native_check`, and the git-HEAD baseline logic. `accepted_findings.rs` is 422 lines (largely tests). `mcp.rs` is 3350 lines. Adding to `check.rs` is a few lines, but logic should not accumulate there.

## 6. Disposition: Isolate via seam

Put the parser/filter in a new `src/inline_ignores.rs` and add one call in `run_checker_against_source`. `check.rs` is large but this touch is small and the plugin-system research confirms entry points are already uniform; a refactor-first pass is not warranted.

## 7. ADR note

This reverses a documented decision: `docs/suppressing-checks.md:11` ("There is no inline/per-line suppression...") and `docs/accepting-findings.md:80` ("There's still no inline suppression comment (`// kibitzer:accept ...`)"). Write an ADR (`/sdd:adr`) recording why: the `accepted/` file keyed on `(rule, file, line, content)` goes stale on edit and has high authoring cost for agents. The "reason required" and "visible in the diff" properties that motivated the stance are preserved by mandatory reasons and in-file comments. Update both docs in the same change.
