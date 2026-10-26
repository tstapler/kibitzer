# Pitfalls: tree-sitter-bash for syntax-rules / comment-quality / duplicate-code

Retroactive pass (implementation in e8a422a, 1dc4dc3). VERIFIED = command run in this worktree
(`target/debug/kibitzer check native <name> <file>`, scratch in `~/.cache/ssq-e2e-tmp/pit/`).
INFERRED = reasoned from source/grammar, not executed.

## 1. Grammar irregularities

| Pitfall | Status | Effect |
|---|---|---|
| `return`/`break`/`continue` parse as plain `command`; no `return_statement` etc. (`until` is `while_statement`) | VERIFIED in `bash_lang_config` doc comment (`src/checkers/rules.rs:1236`) and by run: `return 0` followed by `echo unreachable` produced no finding | `unreachable-code` is dead for bash; `return_kind: "return_statement"` is a never-matching placeholder |
| `if_statement` is flat: then-statements, `elif_clause`, `else_clause` are siblings, no `consequence`/`alternative` | Documented at `rules.rs:1243`; locked by test `bash_deep_nesting_inside_else_branch_is_not_yet_counted` | `deep-nesting` under-counts inside later branches (false negatives only) |
| No formal params | VERIFIED in source (`zero_params`, `no_params`) | `long-parameter-list`, `flag-argument` never fire; correct, but a silent gap in the "7 rules" claim |
| `operator_kinds: ["binary_expression"]` | INFERRED | In bash this node appears in `[[ ]]` tests and `$(( ))` arithmetic, so `extract-variable` counts those; the 2 real-world hits (kubernetes `cluster/common.sh:356`, cassandra `.circleci/generate.sh:129`) look like genuine long conditions. Single `[ ]` tests are `command` nodes and are NOT counted (INFERRED) |
| Digits are bare `word`, not `number` | Documented `rules.rs:1248` | VERIFIED: `[ "$n" -gt 42 ]`, `8080`, `$((a+7))`, `(( i < 100 ))` produced zero findings. `numeric_literal_kinds` has almost nothing to match; numeric magic-number detection is effectively off for bash |
| Heredoc bodies | VERIFIED: three identical `"dup string"` lines inside `<<EOF` produced no finding; `# not a comment` lines inside a heredoc produced no comment-quality finding | Heredoc content is its own node kind, so no false positives; the same property means a long heredoc inflates `long-function` line counts (INFERRED) |
| `case` arms, `subshell` as nesting | INFERRED | Each `case` and `( ... )` adds a nesting level; a script idiom of `case` inside `while` inside `for` hits deep-nesting sooner than comparable code in other languages |

## 2. Noisy-rule risks

- `replace-magic-literal` is the only rule that fires broadly. VERIFIED on 300 findings from
  real scripts (`find ~/dotfiles ~/code -name '*.sh' -size +4k`, kubernetes/cassandra/own repos):
  106 long-function, 192 replace-magic-literal, 2 extract-variable.
- Top repeated strings are repeated platform-detection values, e.g. `"linux"` x10, `"x64"` x9,
  `"mac"` x9, `"aarch64"` x9, `"$temp_dir/$filename"` x9, `"clojure -Spath"` x8. Mostly true
  positives by the rule's definition (extract a constant) but low value as an advisory; the
  `case`/`uname` dispatch idiom is the main source (INFERRED for cause, VERIFIED for counts).
- **Confirmed false-positive class remaining:** the POSIX empty-string idiom
  `if [ "x$VAR" = "x" ]` (cassandra `.build/run-python-dtests.sh:82`, `run-tests.sh:28`;
  `"x"` flagged 3 to 12 times per file). Also `"\n"` separators (install.sh:108,
  install-service.sh:553) and `"y"`/`"n"` prompt answers. Candidate exemptions: strings of
  length <= 1 or escape-only strings.
- The pure-expansion exemption (`bash_is_expansion_string`, `rules.rs:1219`) is VERIFIED to
  work: `"$@"` and `"${1}"` x3 give 0 findings, while `"Usage: $0"` x3 gives 1 (mixed
  literal + expansion still flagged). Note the hook is wired through the field named
  `slog_key_predicate`, a misnomer that will confuse future readers.
- Also VERIFIED: an 80-line `echo "Usage: $0"` function reports a single finding whose message
  lists all 80 line numbers (very long message); cap the "also lines" list (INFERRED fix).
- `long-function` is the volume driver (106 of 300). Shell functions are legitimately long
  `case`-driven installers; the 40-line threshold is shared with other languages (INFERRED that
  it is too tight; not backtested against a bash-specific threshold).
- The "276 -> 106 (later 85)" numbers in the task prompt and commit message could not be
  reproduced exactly here (different corpus/size filter); my run is a different sample and gives
  192 magic-literal findings on ~300 pooled findings, so treat the earlier counts as the
  implementer's, UNVERIFIED by me.

## 3. duplicate-code on boilerplate

- VERIFIED: two files sharing a 6-line header (`#!/usr/bin/env bash`, `set -euo pipefail`,
  `IFS=$'\n\t'`, `cd "$(dirname ...)"`, `source ./lib.sh`, `usage()`) gave no finding from
  `duplicate-code-cross-file` or `check duplicates dir`; a 5-line block repeated 3x in one file
  (`echo/log/mkdir/cd/export`) gave no `duplicate-code` finding either. So the window is longer
  than typical headers: no header noise.
- Risk (INFERRED, not triggered): installer scripts with 10+ line identical preambles
  (arg parsing, color definitions, `die()` helpers) copied across a repo will be flagged by
  cross-file; that is arguably correct. No shebang/`set -e` special-casing exists and none
  appears to be needed at the current window size.

## 4. comment-quality on directives

- VERIFIED no false positive: `#!/usr/bin/env bash`, `# shellcheck disable=SC2086,SC2154`,
  and `# shellcheck source=lib/common.sh` produced no finding in `comment-quality-bash`.
  `strip_comment_markers` has a `#!` prefix (`comment_quality.rs:441`).
- VERIFIED false negatives: commented-out shell code is largely missed. Of
  `# cd "$dir"`, `# rm -rf "$build" && make clean`, `# for f in *.sh; do echo "$f"; done`,
  `# x=$(foo bar)`, `# if [ -f "$x" ]; then`, only `# x=$(foo bar)` was flagged (the
  heuristic is C-family-shaped: braces, trailing `;`, call-without-spaces). Documented intent
  is "biased toward missing", but shell loses most of its signal.
- VERIFIED: `verbose-comment` still works (`robust` flagged at b.sh:3).
- `# kibitzer:ignore replace-magic-literal -- ok` above a flagged line: the test script did not
  reach the 3-repeat threshold, so suppression itself is NOT VERIFIED here; the inline-ignore
  checker returned clean on well-formed directives (VERIFIED on a.sh). The `#` comment form is
  covered by `src/inline_ignores/tests.rs` additions in e8a422a (INFERRED from the diffstat,
  not opened).

## 5. Extension-less scripts (shebang-only)

- VERIFIED: `kibitzer check native comment-quality-bash ./deploy` (file with
  `#!/usr/bin/env bash`, no extension) analyzes it and emits a finding, but
  `kibitzer run .` over a dir containing both `deploy` and `x.sh` reported only `./x.sh`.
  Cause: language selection is by extension only (`Language::extensions()`,
  `checker.rs:137`, `["sh","bash"]`) plus `file_globs` `**/*.sh`/`**/*.bash`; there is no
  shebang detection (grep for `#!` finds only `import_preamble` JS and `comment_quality`).
  Dotfiles-style repos (`bin/foo`, `.zshrc`-like, hooks, Git `pre-commit`) are skipped
  silently.
- Related INFERRED gaps: `.zsh`, `.ksh`, `.bats` files, `.envrc`, `.bashrc`/`.bash_profile`
  (dotfiles with no extension) are not covered; `zsh` scripts parsed with the bash grammar would
  mis-parse `${(f)...}`-style expansions and produce ERROR nodes (no rule seems to skip
  error-heavy files for bash; not tested).

## 6. Recommendations (ranked)

1. Exempt single-character / escape-only strings from `replace-magic-literal` for bash (kills the
   `"x"` idiom; VERIFIED FP class).
2. Add shebang sniffing for extension-less files in language selection and glob matching (the
   only coverage gap that silently drops real scripts).
3. Truncate the "also lines" list in the magic-literal message.
4. Add shell-shaped commented-out-code patterns (`cd `, `rm `, `fi`/`done`/`then` alone,
   `VAR=...`) to comment-quality, with backtest.
5. Rename `slog_key_predicate` or add an adjacent comment explaining the bash reuse.
6. Consider a higher `long-function` threshold or exempting bodies dominated by `case` arms.
