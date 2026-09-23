# Research: Stack (Phase 2) — cite-fowler-refactorings

## Exact current code shape

File: `src/rules.rs`, `edition = "2024"` (`Cargo.toml:11`), no `rustfmt.toml` in
the repo (default `rustfmt` settings apply — irrelevant here anyway, since
`rustfmt` does not wrap string literal contents).

### The two `format!` call sites (`check_declaration`, lines 818-841)

```rust
fn check_declaration(decl: Node, cfg: &LangRuleConfig, src: &[u8], findings: &mut Vec<Finding>) {
    let line = decl.start_position().row + 1;

    if let Some(body) = (cfg.body_finder)(decl) {
        let body_lines = body.end_position().row - body.start_position().row + 1;
        if body_lines > LONG_FUNCTION_LINES {
            findings.push(Finding {
                line,
                message: format!(
                    "[long-function] body spans {body_lines} lines (over {LONG_FUNCTION_LINES}) — consider splitting it up"
                ),
            });
        }

        let depth = max_nesting_depth(body, 1, cfg);
        if depth > MAX_NESTING_DEPTH {
            findings.push(Finding {
                line,
                message: format!(
                    "[deep-nesting] body nests {depth} levels deep (over {MAX_NESTING_DEPTH}) — consider extracting a function or inverting a condition"
                ),
            });
        }
    }
    ...
```

- **`long-function`**: `src/rules.rs:826-828`. Interpolated captured
  identifiers: `{body_lines}` (computed local), `{LONG_FUNCTION_LINES}` (the
  module const, `= 40`, `src/rules.rs:11`). Both use Rust's captured-identifier
  `format!` syntax already — no positional/named args, confirming the pattern
  the requirements doc assumes. Appending `" — consider Extract Function
  (https://refactoring.com/catalog/extractFunction.html)"` after "splitting it
  up" (or replacing the trailing clause) is a same-shape edit: no new
  variables, no new imports.
- **`deep-nesting`**: `src/rules.rs:836-838`. Captured identifiers: `{depth}`,
  `{MAX_NESTING_DEPTH}` (`= 4`, `src/rules.rs:15`). Same shape.
- Constants (`LONG_FUNCTION_LINES`, `MAX_NESTING_DEPTH`) and the `if body_lines
  > ...` / `if depth > ...` trigger conditions are untouched by this change —
  satisfies acceptance criterion 4 by construction if only the string literals
  inside the two `format!` bodies are edited.

### `CATALOG` descriptions (`src/rules.rs:34-46`)

```rust
RuleMeta {
    id: "long-function",
    category: "complexity",
    description: "Function/method body spans more than 40 lines.",
    default_severity: Severity::Advisory,
},
RuleMeta {
    id: "deep-nesting",
    category: "complexity",
    description: "Function/method body nests if/for/switch/select/func_literal more than 4 levels deep.",
    default_severity: Severity::Advisory,
},
```
Plain `&'static str` literals, not `format!` — trivial to append the same
refactoring-name/link text if criterion 6's optional treatment is taken.

## Duplication check across the repo (would the same text need updating elsewhere?)

`grep -rn "long-function\|deep-nesting"` (excluding `src/rules.rs`) found 8 hits.
None duplicate the *message string* itself — they're either identifiers/rule
names or independent prose:

| File | Line | What it is | Needs updating for this change? |
|---|---|---|---|
| `docs/syntax-rules.md` | 30-31 | Rule-catalog markdown table (`Rule ID`/`Category`/`Threshold`/`Description` columns) — its own prose, not copied from the `format!` strings | No — but see "optional consistency" below |
| `README.md` | 206 | One-line mention: "`long-function` already uses (see `docs/syntax-rules.md`)" | No |
| `docs/accepting-findings.md` | 44 | Lists `flag-argument`/`long-function`/`deep-nesting` as example rule IDs | No |
| `src/mcp.rs` | 418 | Doc comment: "coupling/long-function/deep-nesting/long-parameter-list all do, inline" | No |
| `src/accepted_findings.rs` | 354 | Test fixture: `format!("{}:3: [long-function] body spans 41 lines...", ...)` — a **synthetic hand-written string**, not a call into `rules.rs`'s `check_declaration` | No — this test builds its own fake finding text to exercise `filter_accepted`'s parsing; it never calls the real message-producing code, so it's insensitive to the wording change |
| `project_plans/architecture-export/research/pitfalls.md` | — | Unrelated prior project's research doc | No |

Threshold-text search (`"40 lines"`, `"4 levels"`, `LONG_FUNCTION_LINES`,
`MAX_NESTING_DEPTH`) surfaced one additional relevant hit:

- `docs/syntax-rules.md:30-31` — the rule catalog table's `Threshold`/`Description`
  columns ("> 40 lines", "> 4 levels", "Function/method body spans more lines
  than this.", "Function/method body nests control-flow constructs deeper than
  this…"). This is prose describing the rule, not a copy of the finding
  message — out of scope per the requirements doc's own scope statement (no
  general doc rewrite requested), but **optionally** worth a one-line addition
  naming the refactoring for consistency with `RuleMeta.description` if
  criterion 6 is taken. Not required by any acceptance criterion.

No `README.md`, CLI help text, or other doc reproduces the literal `format!`
strings verbatim, so criterion set 1/2/5 is fully satisfied by editing only
the two `format!` bodies in `src/rules.rs` (plus optionally the two
`CATALOG` entries for criterion 6).

## Prior art already in the repo naming these two refactorings

`docs/refactoring-catalog-analysis.md` (an earlier research doc, not part of
this project) already documents this exact pairing and references what is
almost certainly this same backlog item as GitHub issue #41:

- Line 203: `**Extract Function** (Extract Method) | ✅ / 🔧 [#41](https://github.com/tstapler/kibitzer/issues/41) | `long-function` (>40 lines) already is the trigger signal; #41 just names the refactor in the message.`
- Line 260: `**Replace Nested Conditional with Guard Clauses** | ✅ / 🔧 [#41](https://github.com/tstapler/kibitzer/issues/41) | `deep-nesting` (>4 levels) already is the trigger signal.`

This confirms the requirements doc's problem statement independently and
gives the canonical refactoring names/URLs used elsewhere in-repo
(`refactoring.com/catalog/`, cited at `docs/refactoring-catalog-analysis.md:4`
and `:25`) — consistent with the URLs the requirements doc proposes.

## Test surface (criterion 5)

`grep -n '\.contains("\[long-function\]")\|\.contains("\[deep-nesting\]")'`
in `src/rules.rs` found 20 call sites (lines 1240-2287, in `#[cfg(test)]`
modules), all using `.message.contains("[long-function]")` /
`.contains("[deep-nesting]")` substring checks — none assert on the full
message or the trailing "consider ..." clause. Appending more text after the
existing rule-id prefix is safe for all of them, matching acceptance
criterion 5's own framing.

`src/accepted_findings.rs:354`'s test (see table above) hand-builds its
expected string and is likewise unaffected.

## Toolchain/stack conclusion

No new dependency, no new module, no config surface. This is a pure two-line
(plus optionally two more) string-literal edit inside existing `format!`
macros and `&'static str` fields in `src/rules.rs`, using the same
captured-identifier interpolation style (`{body_lines}`, `{LONG_FUNCTION_LINES}`,
etc.) already in use throughout the file — confirmed applicable to a longer
message string since `format!` capture syntax has no length constraint.
Acceptance criterion 3 (verify both URLs resolve) is a manual/CI-external
step, not a code or stack concern — no HTTP-check dependency needed for this
change itself.
