# Stack research: tree-sitter-bash (retroactive check of research.md)

Labels: VERIFIED = command/file run in this worktree; INFERRED = reasoned, not run.

## Version facts

- VERIFIED (`Cargo.toml:25,41`): repo pins `tree-sitter = "0.26"` and `tree-sitter-bash = "0.23"`.
- VERIFIED (`Cargo.lock`): resolves `tree-sitter 0.26.12`, `tree-sitter-bash 0.23.3`, `tree-sitter-language 0.1.7`.
- VERIFIED (cargo registry `tree-sitter-bash-0.23.3/Cargo.toml` and `-0.25.1/Cargo.toml`): both
  depend only on `tree-sitter-language ^0.1` (+ build-dep `cc`). `tree-sitter` appears only as a
  **dev-dependency** (0.24 for 0.23.3; 0.25 for 0.25.1). Neither constrains the consumer's `tree-sitter` version.
- VERIFIED: `tree-sitter-go 0.25.0` (already in the repo, same `LANGUAGE.into()` pattern) uses the same
  `tree-sitter-language 0.1` indirection, so grammar crates of any 0.2x version coexist with `tree-sitter 0.26`.
- VERIFIED: the 0.25.1 source is present in the local registry cache, i.e. 0.25.x exists on crates.io.
- INFERRED (not built): swapping to `tree-sitter-bash = "0.25"` would compile with tree-sitter 0.26 for the same reason.
  It was not tried; do so with `cargo update -p tree-sitter-bash --precise 0.25.1` if wanted.

## Where the issue and AC diverge

- `requirements.md:21` says "0.25.x on crates.io"; the AC/landed code uses 0.23.x. Both are valid and
  both work with tree-sitter 0.26 (see above). 0.23.3 was chosen and shipped (commit e8a422a).
- The grammar crate's "version" tracks the tree-sitter ABI it was generated against, not a hard dependency.
  The runtime check is the ABI number in the generated `parser.c` vs. what tree-sitter 0.26 accepts (INFERRED; the
  landed tests pass against 0.23.3 per the commit message, which I did not re-run here).

- Why 0.23 satisfies the item: backlog AC 1 (item db3e0deb, "Cargo.toml declares tree-sitter-bash at a compatible
  version (0.23.x)") is the acceptance test and names 0.23.x; the description's Proposal step 1 ("0.25.x") is the
  conflicting text, and the AC takes precedence. Verified 2026-10-19: `Cargo.lock` resolves tree-sitter-bash
  0.23.3 alongside a single tree-sitter 0.26.12 (`cargo tree -i tree-sitter`), 0 build warnings.

## `tests/daemon_spawn.rs` ETXTBSY is pre-existing

Run in parallel (default), `cargo test --test daemon_spawn` fails with `ExecutableFileBusy` on base `7634824`
(4 of 4 runs: 2, 3, 6, 3 failures) and on this branch (4 of 4 runs: 4, 2, 1, 3 failures); the failing set varies
per run. On the branch with `-- --test-threads=1` it passes 14/14 (3 of 3). No file under `tests/` differs from
base. Root cause (VERIFIED with `strace -f -e trace=openat,execve,clone3,close,copy_file_range` on the test binary,
captured on the branch; tid 699434 = test thread A, 699445 = test thread B, same process 699431):

1. `/tmp` is tmpfs, `target/` is btrfs, so `Scratch::new`'s `hard_link` (`tests/daemon_spawn.rs:41`) cannot succeed and
   the `fs::copy` fallback runs; `copy_file_range` returns `EXDEV` and the copy is a read/write loop (~350 ms).
2. Thread A opens the copy `O_WRONLY|O_CREAT|O_TRUNC|O_CLOEXEC` as fd 9 (t=.9599) and closes it at t=.3078.
3. Thread B calls `clone3(CLONE_VM|CLONE_VFORK)` at t=.3077, 0.2 ms *before* A's `close(9)`. Test threads share one fd
   table, so the child (tid 699484) inherits the write fd; `O_CLOEXEC` releases it only at that child's `execve`,
   which completes at t=.3140.
4. Thread A's child (tid 699485) calls `execve` on the copy at t=.3104 and gets `ETXTBSY`, because tid 699484 still
   holds it open for writing.

So the race is a fork in one test thread overlapping the copy of another thread's binary: the fixture, not the bash
change. Same code runs on base, which fails identically.

## node-types.json: 0.23.3 vs 0.25.1

- VERIFIED: `codegen/node-types/bash.json` is byte-identical to `tree-sitter-bash-0.23.3/src/node-types.json`
  (`cmp`), so the vendored file matches the shipped crate.
- VERIFIED: it differs from 0.25.1 (112 diff lines). The set of 62 named node types is identical in both
  (python set diff empty). Differences are only extra allowed child types in field/children lists (e.g.
  `raw_string`, `binary_expression`, `command_substitution`, `expansion` added as permitted children).
  So `BashKind` variants would not change on an upgrade, but the vendored JSON must be re-vendored with the crate.
- VERIFIED (python over bash.json) named kinds present: `function_definition`, `if_statement`, `elif_clause`,
  `else_clause`, `for_statement`, `c_style_for_statement`, `while_statement`, `case_statement`, `subshell`,
  `compound_statement`, `comment`, `string`, `raw_string`, `ansi_c_string`, `number`, `word`, `heredoc_body`,
  `command`, `command_name`, `list`, `pipeline`, `variable_assignment`, `string_content`, `concatenation`.
- VERIFIED absent: `return_statement`, `break_statement`, `continue_statement` (and `until_statement`, per the commit message).
  `return`/`break`/`continue` parse as ordinary `command` nodes.
- VERIFIED: `function_definition` fields: `name` (word), `body` (one of `compound_statement`, `if_statement`,
  `subshell`, `test_command`), optional `redirect`. No parameter field.

## Where research.md was wrong or incomplete

1. WRONG (line 81-88): claimed `tree-sitter-bash 0.23.3` "requires `tree-sitter ^0.23`" and rated a version conflict
   Medium risk. It has no runtime dependency on `tree-sitter` at all; the risk was nil (Cargo.lock shows a single
   `tree-sitter 0.26.12`). The "try `cargo add` and inspect the lock" mitigation was right but the premise was not.
2. WRONG (line 43-44, 97-99): said `terminal_kinds` would be `return_statement` and that `break_statement`/`continue_statement`
   probably exist. None of the three exist; they are `command` nodes. The doc hedged on break/continue ("confirm") but
   asserted `return_statement` as fact.
3. WRONG (line 77-79): "latest crates.io version 0.23.3 (as of 2026-10)". 0.25.1 exists (in registry cache), so 0.23.3 was
   not the latest; it is merely the one used. Not verified against crates.io live, only the local cache.
4. WRONG (line 90-92): `function_definition` -> `word` + `compound_statement` only. The body may also be
   `if_statement`, `subshell`, or `test_command`; `body_finder` must not assume a `compound_statement`.
   The "last named child" heuristic can also hit the optional `redirect`.
5. INCOMPLETE: no mention that 0.25.1 is a drop-in with identical named kinds, nor that the vendored JSON must be
   re-vendored with any crate bump. No mention of the issue-vs-AC version mismatch (0.25 vs 0.23).
6. INCOMPLETE: line 44 uses `|_| 0` for param_counter; not checked against the landed code (not in scope here).

## `daemon_spawn` fixture fix

`Scratch::new` now hard-links the built binary into a `spawntest-*` dir beside it (same filesystem as `target/`),
so no test thread ever holds a write fd on an exec target; the `fs::copy` fallback remains for filesystems without
hard links. Result: `cargo test --test daemon_spawn` passed 14/14 in 5 of 5 default-parallel runs (was 0 of 8 on
base and branch), and a full `cargo test` had 0 failures and 0 warnings.
