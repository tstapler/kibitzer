# Plan: bash-shell-support

## Implementation order

The work is sequential (each step compiles the previous): grammar dep → codegen →
`Language` variant → comment kinds → `lang_config` arm → default-checks wiring → tests.

---

### Task 1 — Add `tree-sitter-bash` dependency

**File**: `Cargo.toml`

```toml
tree-sitter-bash = "0.23"
```

After adding, run `cargo build` and inspect `Cargo.lock` to verify the resolved version
doesn't conflict with `tree-sitter = "0.26"`. If there's a conflict, try `"0.22"` or `"0.21"`.
Record the resolved version in this plan.

**Estimate**: 0.5h (mostly version-conflict diagnosis)

---

### Task 2 — Vendor `bash.json` and wire `build.rs`

**Step 2a**: Locate `node-types.json` inside the `tree-sitter-bash` crate's registry checkout
(`~/.cargo/registry/src/*/tree-sitter-bash-*/…`) and copy it to
`codegen/node-types/bash.json`.

**Step 2b**: Before writing any `LangRuleConfig` code, examine this file for:
- Comment node kind(s) (expected: `comment`).
- Function-definition node kind (expected: `function_definition`).
- Whether `break_statement` and `continue_statement` exist as named node types.
- Block kind: `compound_statement` vs. `block` vs. something else.
- Nesting kinds: `for_statement`, `while_statement`, `c_style_for_statement`,
  `case_statement`, `subshell` — which are present as named node types?

Document findings in a comment inside `bash_lang_config()`.

**Step 2c**: Add to `build.rs`'s `GRAMMARS`:
```rust
("bash", "BashKind", "codegen/node-types/bash.json"),
```

**Step 2d**: Add to `src/node_kind.rs`:
```rust
pub mod bash {
    include!(concat!(env!("OUT_DIR"), "/bash_kind.rs"));
}
pub use bash::BashKind;
```

**Estimate**: 1h

---

### Task 3 — Add `Language::Bash` variant

**File**: `src/checker.rs`

Four mandatory edits (exhaustive match enforces build failure otherwise):
1. `enum Language` body: add `Bash`.
2. `Language::ALL`: add `Language::Bash`.
3. `Language::ts_language()`: add `Language::Bash => tree_sitter_bash::LANGUAGE.into()`.
4. `Language::extensions()`: add `Language::Bash => &["sh", "bash"]`.

**Estimate**: 0.5h

---

### Task 4 — Add `Language::Bash` to `comment_kinds`

**File**: `src/tree_walk.rs`

Add `Language::Bash` to the match arm that returns `&["comment"]`:
```rust
Language::Go
| Language::Python
| Language::TypeScript
| Language::Tsx
| Language::JavaScript
| Language::Bash => &["comment"],
```

(Confirmed from `node-types.json` research: bash comment kind is `comment`.)

**Estimate**: 0.25h

---

### Task 5 — Add `bash_lang_config()` and `lang_config` arm

**File**: `src/checkers/rules.rs`

Add `Language::Bash => bash_lang_config()` to `lang_config()` and implement:

```rust
fn bash_lang_config() -> LangRuleConfig {
    LangRuleConfig {
        name: "syntax-rules-bash",
        file_globs: &["**/*.sh", "**/*.bash"],
        // bash's function_definition has no formal parameter list — arguments are
        // positional ($1/$2) at call time. long-parameter-list will never fire for bash.
        function_kinds: &["function_definition"],
        if_kind: "if_statement",
        nesting_kinds: &[
            "for_statement",
            "c_style_for_statement",
            "while_statement",
            "case_statement",
            "subshell",
        ],
        else_wrapper_kinds: &["else_clause"],
        chain_kinds: &[],
        param_counter: |_| 0, // No formal params in bash
        body_finder: bash_body,       // compound_statement child
        params_finder: |_| None,      // No formal param list
        bool_param_finder: |_, _| vec![],
        ternary_kind: None,
        block_kind: "compound_statement",
        statement_container: identity_stmt,
        unwrap_statement: identity_stmt,
        // break_statement/continue_statement: include if present in node-types.json,
        // omit with a comment if absent (same pattern as Kotlin's missing break/continue).
        terminal_kinds: &["return_statement"],  // adjust after node-types.json check
        panic_detector: no_panic_detector,
        // Scope to string/number only — 'word' tokens are too broad and would fire on
        // variable names, flag arguments, command names, etc.
        literal_kinds: &["string", "number"],
        numeric_literal_kinds: &["number"],
        binding_finder: no_const_binding,
        return_kind: "return_statement",
        call_kinds: &["command"],
        operator_kinds: &["binary_expression"],
        density_boundary_kinds: &["subshell"],
        collection_literal_kinds: &[],
        slog_key_predicate: None,
    }
}
```

Add `bash_body` helper:
```rust
fn bash_body(node: Node) -> Option<Node> {
    // function_definition's body is a compound_statement (the last named child).
    // bash has no `body` field in tree-sitter — use last named child.
    let mut cursor = node.walk();
    node.named_children(&mut cursor).last()
}
```

All grammar details must be cross-checked against the vendored `bash.json` before landing.
Any field that can't be verified gets a `// UNVERIFIED — check node-types.json` comment.

**Estimate**: 2h (most complex task)

---

### Task 6 — Wire into `default_checks`

**File**: `src/config.rs`

Three edits:

**`syntax_rules_checks()`**:
```rust
native_check("syntax-rules-bash", Severity::Advisory, &["**/*.sh", "**/*.bash"]),
```

**`comment_quality_checks()`**:
```rust
native_check("comment-quality-bash", Severity::Advisory, &["**/*.sh", "**/*.bash"]),
```

**`core_checks()`** — both `duplicate-code` and `duplicate-code-cross-file` glob slices:
Add `"**/*.sh"` and `"**/*.bash"`.

**Estimate**: 0.5h

---

### Task 7 — Update / add tests

**File**: `src/config.rs` tests

Update `default_checks_still_registers_all_eight_syntax_rules_checkers` to include
`"syntax-rules-bash"` (or rename to `_nine_`).

**File**: `src/checkers/rules.rs` tests (or a new `bash_tests` module)

Add a minimal smoke test that feeds a bash function body through `SyntaxRulesChecker` and
confirms it either produces the expected finding (e.g. deep nesting) or produces no false
positives on a well-formed function. Pattern: follow the existing `rules.rs` module tests.

**Estimate**: 1h

---

### Task 8 — Backtest against real-world bash corpus

Per the `CLAUDE.md` backtest requirement, run the new checker against at least the repo's
own shell scripts (`scripts/`, `bootstrap/`) and the dotfiles repo's shell scripts, as a
proxy for the full public-repo backtest (the standard corpus doesn't include bash-heavy
repos but this repo does have `.sh` files).

```sh
kibitzer check native syntax-rules-bash bootstrap/run.sh
kibitzer check native comment-quality-bash bootstrap/run.sh
```

Record false-positive count (target: <40% FP rate per ADR-001's bar). Document results in
`docs/backtest-triage/` or inline as a comment in `rules.rs`.

**Estimate**: 1h

---

## Total estimate

| Task | Estimate |
|------|---------|
| 1. Add dep | 0.5h |
| 2. Vendor json + build.rs | 1h |
| 3. Language variant | 0.5h |
| 4. comment_kinds | 0.25h |
| 5. bash_lang_config | 2h |
| 6. default_checks wiring | 0.5h |
| 7. Tests | 1h |
| 8. Backtest | 1h |
| **Total** | **6.75h** |

## Critical path

Grammar version compatibility (Task 1) gates everything else. If `tree-sitter-bash 0.23.x`
conflicts with the `0.26` pin, the minor version resolution must happen before any grammar
code can compile.
