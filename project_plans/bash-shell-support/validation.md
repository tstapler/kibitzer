# Validation: bash-shell-support

## Adversarial review

### AC1 — Cargo.toml dep version risk

**Challenge**: The plan specifies `tree-sitter-bash = "0.23"`. This repo uses
`tree-sitter = "0.26"`. The `tree-sitter` pre-1.0 semver convention means `0.26` is not
automatically compatible with crates that declare `^0.23` — if `tree-sitter-bash 0.23.x`
internally declares `tree-sitter = "0.23"` as an exact minor requirement, Cargo will pull
two copies of `tree-sitter` into the build. While this usually resolves quietly (Cargo
deduplicates compatible minor versions), it's a real risk.

**Verdict**: The plan correctly identifies this as a medium-risk blocker and mandates
checking `Cargo.lock` after `cargo add`. No gap — the mitigation is sufficient.

---

### AC5 — `LangRuleConfig` fields based on unverified assumptions

**Challenge**: The plan's `bash_lang_config()` sketch uses node kinds like
`compound_statement`, `c_style_for_statement`, `case_statement`, `subshell`, and
`return_statement`. These are plausible for bash's grammar but were not verified against the
actual vendored `bash.json` in this document. Shipping code with unverified node-kind
strings silently misfires — `node.kind()` comparisons just never match, producing no
findings, with no compile error.

**Verdict**: The plan includes a mandatory `node-types.json` verification step in Task 2b
and a `// UNVERIFIED` comment convention for any field that can't be confirmed. The existing
Kotlin precedent (documented missing `break`/`continue` node kinds) shows the codebase has
established this pattern. A pre-implementation `node-types.json` review is the correct gate.

**Residual risk**: The `param_counter: |_| 0` stub is correct (bash has no formal params),
but `long-parameter-list` silently never fires. This is **by design** and must be documented
in the code comment, not just in this plan.

---

### AC6/AC7 — `file_globs` overlap between `DuplicateCodeChecker::file_globs()` and `core_checks()`

**Challenge**: There are two places that control what files `duplicate-code` runs on:
(a) `DuplicateCodeChecker::file_globs()` in `duplicate_code.rs`, and (b) the glob override
inside `core_checks()` in `config.rs`. The plan says to add `.sh`/`.bash` to the
`core_checks()` glob list, but `DuplicateCodeChecker::file_globs()` is not mentioned.

**Finding**: Looking at `duplicate_code.rs:43-55`, `file_globs()` is the checker's own
declaration of what it handles. The `core_checks()` function's glob override in
`config.rs:818-830` duplicates this information at the config layer. Both must be updated
for consistency — the plan omits `duplicate_code.rs`. **Gap identified.**

**Resolution**: Add a task to also update `DuplicateCodeChecker::file_globs()` in
`duplicate_code.rs`. Similarly, `duplicate_cross_file_checker.rs` may have its own
`file_globs()` that needs the same addition.

---

### AC3 — `from_extension_recognizes_every_language_variant` test

**Challenge**: The test in `checker.rs` iterates over `Language::ALL` and calls
`from_extension` for each language's `extensions()`. If `Language::Bash` is added to `ALL`
and `extensions()` but not correctly, the test catches it. However, the test file name
itself isn't shown in the plan — implementors might miss it.

**Verdict**: The test is compiler-enforced via the exhaustive match on `Language` in
`extensions()`. No gap, but the plan should explicitly reference the test name so reviewers
can verify it still passes.

---

### Bash-specific grammar gaps not covered by plan

**`replace-magic-literal` with bash `word` tokens**: The plan correctly scopes
`literal_kinds` to `string`/`number` only to avoid over-firing on `word` tokens. However,
if tree-sitter-bash's `number` node kind doesn't exist (bash doesn't have a true numeric
literal token — numbers are just `word` tokens with digit content), `numeric_literal_kinds`
would be empty and no numeric findings would fire. This is acceptable for v1 but needs a
comment.

**`extract-variable` density check**: The `call_kinds: &["command"]` sketch is reasonable
but bash uses `command` for any external program invocation, not function calls. This could
produce either too many or too few findings. For v1, this is acceptable as-is since the
density threshold is high (10), but a backtest in Task 8 is necessary.

---

### Omitted: `duplicate_code.rs` `file_globs` update

The plan's Task 6 covers `config.rs` globs but doesn't explicitly cover
`DuplicateCodeChecker::file_globs()` in `src/checkers/duplicate_code.rs`. This is a
**gap** that must be addressed — otherwise `duplicate-code` glob in config won't match the
checker's declared file scope.

---

## Plan verdict

**APPROVED with one gap to close**: Add an explicit sub-task to Task 6 updating
`DuplicateCodeChecker::file_globs()` in `duplicate_code.rs` (and verify
`duplicate_cross_file_checker.rs`'s equivalent). All other acceptance criteria are covered.

The grammar version conflict risk (Task 1) is the highest-priority item to resolve first —
it gates everything else and cannot be simulated without actually running `cargo add`.
