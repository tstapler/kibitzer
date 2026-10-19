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
