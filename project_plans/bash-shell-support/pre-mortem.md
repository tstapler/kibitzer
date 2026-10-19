# Pre-mortem: bash-shell-support
**Date**: 2026-10-18
**Framing**: retroactive. The change (`e8a422a`, `1dc4dc3`, `ec9dca8`) is merged and released as a kibitzer version, and it failed in the field. Why?

Evidence labels: **VERIFIED** = command run in this worktree on 2026-10-18 (command given); **INFERRED** = reasoned from source, not executed; **UNVERIFIED** = not checked. Scratch repros live in `~/.cache/ssq-e2e-tmp/pm/` (outside the repo). The binary used is the existing `target/debug/kibitzer` at HEAD `ec9dca8`.

## Failure Modes

| # | Failure | First Symptom | Prevention | Severity |
|---|---------|--------------|------------|----------|
| 1 | `duplicate-code-cross-file` (default-on) flags the identical license-header comment block at the top of nearly every shell script in a repo that has one, because `import_preamble.rs` has no shell arm and so never treats leading `#` comment lines as preamble. | A user runs `kibitzer run .` (or edits any `.sh`) in an OSS or corporate repo and gets a "6-line block also duplicated in N other file(s)" advisory on almost every script. On kubernetes: 284 cross-file findings over 299 `.sh` files, 1.96 MB of output. | Add a shell arm to `PreambleLang` in `src/checkers/import_preamble.rs` (ext `sh`/`bash`; accept blank lines and `#` lines, including `#!`, as preamble) and add a 3-file shared-license-header regression test. Re-run the kubernetes sample before release. | **P1** |
| 2 | Default-on `replace-magic-literal` and `long-function` bury shell scripts in low-value advisories, and the FP triage the repo's own done-bar requires was never done. | Agents and users see dozens of "extract a constant" and "body spans N lines" advisories per installer script, start ignoring `kibitzer` output, or add `.kibitzer/inspect.json` overrides. | Run `scripts/backtest-triage.py` over a committed shell sample and record it under `docs/backtest-triage/`. Exempt strings of 1 character or fewer and escape-only strings. Treat `"x$VAR"` comparisons as exempt. Consider a higher `long-function` threshold for `case`-driven bodies. | P2 |
| 3 | Scripts without a `.sh`/`.bash` extension (shebang-only `bin/*`, git hooks, `.envrc`, `.bashrc`, `.bats`, `.zsh`) get no coverage, silently, and the docs never say so. | A user reports "kibitzer says nothing about my `bin/deploy`". `kibitzer run .` over a dir with `deploy` and `x.sh` reports only `x.sh`. | Document the extension-only limit in `docs/syntax-rules.md`'s Bash caveats now. Add shebang sniffing in `Language::for_path` plus glob matching as a follow-up. | P2 |
| 4 | Finding output can blow up in size and time on real shell trees: the "also lines" list is uncapped, one 80-line `echo "Usage: $0"` function yields one message listing 80 line numbers, and a full `run` is slow. A non-UTF-8 `.sh` yields 5 "reading ... invalid utf-8" advisories. | A single hook run injects multi-KB lines into agent context. Max line length in the kubernetes output was 11,405 characters. `kibitzer run` over 299 scripts took 130 s on the debug build. | Cap the "also lines" list (for example first 10 plus "and N more"). Time a release build on the kubernetes sample. Treat unreadable-as-UTF-8 files as one skip note, not one advisory per check. | P2 |
| 5 | Users and agents read `syntax-rules-bash` as full parity with the other languages, but three rules cannot fire for bash (`long-parameter-list`, `flag-argument`, `unreachable-code`), numeric `replace-magic-literal` is nearly inert, and `deep-nesting` under-counts inside `elif`/`else`/later `then` statements. | A deeply nested installer passes clean, and someone concludes "bash is fine". | Add a test pinning the inert-rule set (architecture-review recommendation) so a future grammar bump that adds `return_statement` fails loudly. Give the `if_branches` under-count its own tracked fix. | P2 |
| 6 | `architecture export` / `architecture_assessment` treat `.sh` as supported but extract zero symbols, so a shell-only repo gets an empty model with no warning. | The model JSON lists the files with `"symbols": []`. | Add a test pinning the behavior, or emit one "no symbol extraction for shell" note. | P3 |
| 7 | The next language added repeats the miss class: globs are hand-maintained in three places, and `comment_leader` still ends in a `Some(_) => "//"` wildcard. | A new language's `kibitzer:ignore` hint shows `//` and some default check is never registered. | Make the `comment_leader` match exhaustive. Add a test over `Language::ALL` that every language has its default checks. Derive globs from `Language::extensions()`. | P3 |
| 8 | Release or grammar drift: the `tree-sitter-bash` dependency is 0.23.3 while AC1 says 0.25.x, and the new C scanner is only exercised by the tag-triggered cross-target build (`aarch64-unknown-linux-gnu` etc.). | A tag build fails on one target, or a modern-bash construct mis-parses. | Reconcile AC1 with the dependency. Watch the first tag build per `CLAUDE.md` "Cutting a release". | P3 |

## Evidence and status per P1 (and notable P2)

### #1 (P1): license-header duplicate blocks. Status: **RESOLVED in 41af6d3 (see "Resolution" at the end)**

Root cause (VERIFIED by reading):
- `find_duplicate_blocks` in `src/checkers/duplicate_code.rs:88` and the cross-file path at `duplicate_code.rs:159` and `src/checkers/duplicate_cross_file_checker.rs:302` all go through `non_preamble_windows`.
- That function skips windows inside a file's leading preamble. The preamble is "blank, comment, or import" lines. It is computed only when `PreambleLang::from_ext` recognises the extension (`src/checkers/import_preamble.rs:19-35`).
- The recognised extensions are `go`, `kt`/`kts`/`java`, `rs`, `py` and the JS family. There is no `sh`/`bash`, so `leading_import_end_for` returns 0 and nothing is suppressed.
- Line-window hashing has no AST or comment awareness (`duplicate_code.rs:75-107`). A 6-line license header over 60 characters (`MIN_BLOCK_CHARS`) in 3 or more files (`MIN_OCCURRENCES`) is therefore a finding.

Reproduction (VERIFIED):
- Three `.sh` files sharing a 10-line Apache header plus one differing command line: `kibitzer run .` gives `duplicate-code-cross-file: ./c.sh:3: 6-line block also duplicated in 2 other file(s) (./a.sh:3, ./b.sh:3)`.
- The same header in three `.py` files gives no finding, because Python has a preamble arm that accepts `#` lines.
- Real data: 299 kubernetes `.sh` files copied into a scratch git repo, `kibitzer run .`, took 130 s on the debug build and produced 476 advisories: 284 `duplicate-code-cross-file`, 118 `syntax-rules-bash`, 33 `comment-quality-bash`, 23 `bash-file-size`, 18 `duplicate-code`.
- By reported start line, 247 of the 284 cross-file findings start at line 3, 24 at line 4, 11 at line 2 and 2 at line 1. That is the license header position after a shebang, not shell logic. I did not open every finding, so the "mostly license header" share is INFERRED from line positions plus the one inspected file pair (`cluster/kube-up.sh` vs `cluster/get-kube.sh`, both beginning with the identical Apache header).

Why the plan's checks missed it:
- `research/pitfalls.md` section 3 tested "no header noise" with two files. `MIN_OCCURRENCES` is 3, so two files can never fire.
- The 150-script backtest ran per-file (`kibitzer check native <name> <file>`), which cannot surface cross-file findings.
- `research/architecture.md` section 4 and plan follow-up 4 call the missing `import_preamble.rs` arm "no-op, INFERRED correct". That inference is wrong.

Why P1: it is near-universal (any repo whose shell scripts carry a license header, which is most OSS and many corporate repos), default-on with no config, and fires per edited file, so a user's first impression is a flood. No test covers it: `grep -in bash src/checkers/import_preamble.rs src/checkers/duplicate_cross_file_checker.rs` finds nothing.

### P1 count
- Total P1: **1** (resolved in 41af6d3; 0 open).
- Open P1: **0** (#1 resolved in 41af6d3).

### #2 (P2): noise, partially mitigated
- Mitigated: the pure-expansion exemption (`rules.rs:1216-1234`, tests `bash_exempts_repeated_pure_expansion_string` and `bash_flags_repeated_plain_string_and_mixed_expansion_string`).
- Not mitigated: the `"x$VAR" = "x"` idiom, `"\n"` and `"y"`/`"n"` (VERIFIED FP classes in `research/pitfalls.md` section 2). `long-function` was 106 of 300 pooled findings. No triage exists under `docs/backtest-triage/`.
- `CLAUDE.md` says a checker without a run against the corpus "isn't done". Neither the review verdicts nor the plan status call this a blocker. Ship it as P2 only if the owner accepts that gap knowingly.

### #3 (P2)
- VERIFIED by `research/pitfalls.md` section 5 (extensionless `deploy` skipped by `run`).
- Also VERIFIED in this session: a truncated zsh-style `.sh` file (`${(f)...}`, unterminated `if`) produced no output from any bash check. Parse errors are silent, not noisy; the risk is coverage, not crashes.
- `grep -n -i "shebang\|extension" docs/syntax-rules.md` finds nothing: the limit is undocumented.

### #4 (P2)
- VERIFIED: a 4.9 MB generated `.sh` (200,000 identical lines) took 12 s for `syntax-rules-bash`. The equivalent `.py` took 20 s, so the cost is shared across languages, not a bash regression.
- VERIFIED: a Latin-1 `.sh` and `.py` each produce one "invalid utf-8" advisory per registered check (5 per file); also not bash-specific.
- Release-build timing for the 299-file run is UNVERIFIED (debug build only).

### Checked and found healthy (no failure mode raised)
- Modern bash constructs parse with 0.23.3: `declare -A`, `${var@Q}`, `[[ =~ ]]`, `;;&`, `<<<`, `coproc`, `$'..'`, `(( ))`, `select`. The repeated `"dup"` literal was counted correctly (4 occurrences). VERIFIED with `check native syntax-rules-bash mod.sh`. Cosmetic: the message prints `also lines 6, 6, 11`, a duplicated line number when two matches share a line.
- CRLF scripts analyse normally. Empty file, no output, no panic.
- `# shellcheck` / shebang lines produce no `comment-quality-bash` finding (VERIFIED in `research/pitfalls.md` section 4).
- The earlier follow-ups "no test pins `comment-quality-bash`/`bash-file-size` registration" and "no test pins the `#` comment leader" were closed by `ec9dca8` (`src/config.rs:1474` test, `src/tree_walk.rs:62` comment-kind test). The plan text still lists them as open.

## P1 Items (address before implementation)
- [ ] Failure #1 — add a shell (`sh`, `bash`) arm to `PreambleLang` in `src/checkers/import_preamble.rs` that treats leading blank and `#` lines (including the shebang) as preamble, add a regression test with 3 `.sh` files sharing a license header (expect no `duplicate-code-cross-file` finding) plus one with 3 files sharing a real 6-line code block (expect a finding), and re-run `kibitzer run` on the kubernetes shell sample before tagging a release. Status: **open**; the code already landed, so this is a follow-up fix before release, not a pre-implementation change.

## Resolution of the open P1 (added after the readiness-gate check)

P1 #1 (license-header comment block flagged by `duplicate-code-cross-file`) is fixed: `src/checkers/import_preamble.rs` gained a `Shell` variant (`sh`/`bash`) that treats leading blank and `#` lines, including the shebang, as preamble. Evidence:
- `does_not_flag_shell_license_header_cross_file` failed before the change (reproducing the finding) and passes after it; `still_flags_a_shell_code_block_repeated_cross_file` confirms real repeated code is still reported.
- A copy of 60 kubernetes `.sh` files (all with the Apache header) run through `kibitzer run . --trigger batch` now yields 0 cross-file findings; its one `duplicate-code` finding is a real 6-line block repeated within `update-codegen.sh`. Pre-fix counts on that exact sample were not captured (the pre-mortem's 284 cross-file findings came from 299 files).
- `cargo test --bin kibitzer`: 1710 passed, 0 failed. Open P1 count: 0.
