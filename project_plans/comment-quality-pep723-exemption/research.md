# Research: comment-quality-pep723-exemption

## PEP 723 specification

[PEP 723](https://peps.python.org/pep-0723/) defines inline script metadata for
single-file Python scripts. The block is delimited by:

- **Opening sentinel**: `# /// <type>` where `<type>` matches `[a-zA-Z0-9]([a-zA-Z0-9-]*[a-zA-Z0-9])?`
  (a non-empty slug). The most common type is `script`.
- **Closing sentinel**: `# ///` (nothing after the `///`, possibly trailing whitespace).
- **Body lines**: each must start with `# ` (hash + space) or be exactly `#`; the TOML
  content is obtained by stripping the leading `# ` from each body line.

Example:

```python
#!/usr/bin/env python3
# /// script
# requires-python = ">=3.11"
# dependencies = [
#   "rich>=13.0.0",
#   "typer>=0.9.0",
# ]
# ///
```

The spec does not restrict the block to the top of the file, though every real-world
usage observed places it there (often after a shebang). Multiple blocks of different
types are allowed in one file.

## Why the false positive fires

In `src/checkers/comment_quality.rs`, `check_commented_out_code` calls `looks_like_code`
on each stripped comment line. For `# dependencies = [`:

1. `strip_comment_markers("# dependencies = [")` → strips `#` prefix → `"dependencies = ["`
2. `looks_like_code("dependencies = [")` calls `is_assignment("dependencies = [")`
3. `is_assignment`: finds `=` at position 14; `before` = ` ` (space, not `=`/`!`/`<`/`>`),
   `after` = ` ` (space, not `=`/`>`); calls `is_assignment_lhs("dependencies")` → `true`
   (alphabetic, no spaces, valid chars); calls `is_assignment_rhs("[")` → `true` (non-empty,
   no `=`, no `. `).
4. `is_assignment` returns `true` → `looks_like_code` returns `true` → finding emitted.

Lines like `# requires-python = ">=3.11"` do NOT fire because `requires-python` contains
`-`, which fails `is_assignment_lhs`'s character class check. Lines like
`#   "rich>=13.0.0",` do not fire because `>=` is excluded by `is_assignment`'s
comparison-operator guard.

## How analogous exemptions work

Three prior exemptions inform the design:

### 1. `in_fence` (backtick code examples in doc comments)

`check_commented_out_code` carries `in_fence: bool` state across consecutive comment
nodes (Rust `///` lines are each their own tree-sitter node). When a line stripped to
` ``` ` is seen, the fence toggles; lines inside the fence are skipped. The state is
threaded through the main `check` loop via return value.

### 2. `is_directive_only_comment`

In `check`'s loop, a comment that is only a kibitzer directive is `continue`d before any
check function is called. PEP 723 blocks can be handled the same way — skip the whole
node when inside the block.

### 3. `is_brace_closing_annotation`

A positional, per-file-line check (not text-content based) that exempts `}` closing-brace
annotation comments from the dead-code check. The exemption is computed once per node and
held as a `bool` rather than threaded state.

## Chosen implementation approach

Add `in_pep723_block: bool` state to the `check` method's comment-iteration loop,
parallel to the way `in_fence` is tracked inside `check_commented_out_code`:

```rust
let mut in_fence = false;
let mut in_pep723_block = false;   // NEW
for comment in &comments {
    let text = comment.utf8_text(ctx.source.as_bytes()).unwrap_or("");
    // PEP 723 inline script metadata (Python-only): exempt the whole block.
    if self.lang == Language::Python {
        if is_pep723_sentinel_open(text) {
            in_pep723_block = true;
            continue;
        }
        if in_pep723_block {
            if is_pep723_sentinel_close(text) {
                in_pep723_block = false;
            }
            continue;
        }
    }
    // existing checks follow unchanged
    ...
}
```

The two sentinel helpers check the raw comment text (not stripped), so they are
independent of `strip_comment_markers`'s ordering:

```rust
fn is_pep723_sentinel_open(text: &str) -> bool {
    // Matches "# /// <type>" — hash, space, ///, space, non-empty type name
    let s = text.trim();
    let Some(rest) = s.strip_prefix('#') else { return false; };
    let rest = rest.trim_start();
    let Some(rest) = rest.strip_prefix("///") else { return false; };
    // Must have whitespace then at least one alphanumeric char (the type name)
    let rest = rest.trim_start();
    !rest.is_empty() && rest.chars().next().is_some_and(|c| c.is_alphanumeric())
}

fn is_pep723_sentinel_close(text: &str) -> bool {
    // Matches "# ///" with nothing (or only whitespace) after the ///
    let s = text.trim();
    let Some(rest) = s.strip_prefix('#') else { return false; };
    let rest = rest.trim_start();
    rest.trim_end() == "///"
}
```

`is_pep723_sentinel_close` uses `trim_end()` to tolerate trailing whitespace on the
closing line. `is_pep723_sentinel_open` requires at least one alphanumeric character
after `///` so a bare `# ///` (close sentinel) is not misread as an open sentinel.

## Alternatives considered

### A. Fix `looks_like_code` / `is_assignment` to exclude TOML-shaped lines

Not viable: TOML and Python assignment syntax are identical (`key = value`). Any
narrowing of `is_assignment` to exclude TOML would also exclude real Python dead code.

### B. Pre-scan source text (byte offsets) to mark exempt line ranges

Would work, but requires duplicating the sentinel detection in a pre-pass separate from
the comment-node loop. The `in_pep723_block` state approach is simpler and exactly
parallel to `in_fence`.

### C. Add a new exemption inside `check_commented_out_code`

`check_commented_out_code` receives one comment node's text at a time and cannot see
across node boundaries to determine whether the current node is inside a PEP 723 block
without threading more state. The `check` loop already threads `in_fence` across calls;
adding `in_pep723_block` at the same level is the natural extension.

## Test scope

The regression test should cover the exact repro case: a Python file with shebang +
`# /// script` block containing `dependencies = [` lines. A second test should verify
that real commented-out code immediately after a closed PEP 723 block is still flagged
(guards against an off-by-one in the close-sentinel logic).
