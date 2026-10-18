# Requirements: comment-quality-pep723-exemption

## Background

`comment-quality-python`'s `[commented-out-code]` check fires false positives on
[PEP 723](https://peps.python.org/pep-0723/) inline script metadata blocks — the
`# /// script` … `# ///` convention used by `uv run` / `pip run` to declare
per-script dependencies in a single Python file without a separate `pyproject.toml`.

The false positive was confirmed against `tools/stapler_logseq_tools/knowledge_maintain.py`
in tstapler's dotfiles repo on 2026-09-08:

```
knowledge_maintain.py:4: [commented-out-code] this line looks like dead code, not a
comment — delete it or explain why it's kept
```

The flagged file starts with:

```python
#!/usr/bin/env python3
# /// script
# requires-python = ">=3.11"
# dependencies = [
#   "rich>=13.0.0",
#   "typer>=0.9.0",
#   "pydantic>=2.5.0",
# ]
# ///
```

Root cause: `# dependencies = [` is parsed by `is_assignment` (lhs = `dependencies`,
rhs = `[` — both pass their validity checks), so `looks_like_code` returns `true`. The
block is live, standardized metadata, not commented-out code.

## Scope

- **In scope**: exempt PEP 723 fenced blocks from the `[commented-out-code]` check in
  `comment-quality-python`. Skip all comment-quality checks for lines inside the block
  (the TOML content is not programmer prose and shouldn't be judged by verbose-comment or
  over-commented either).
- **Out of scope**: changes to any other checker; changes to Go/Rust/etc. language
  variants; modifying `looks_like_code` itself (the sentinel-based exemption is narrower
  and safer).

## Acceptance Criteria

1. A Python file containing a PEP 723 `# /// script` … `# ///` block at the top
   (optionally preceded by a shebang `#!/usr/bin/env python3`) produces **no**
   `[commented-out-code]` findings for any line inside the block.
2. Lines inside the block also produce no `[verbose-comment]` or `[over-commented]`
   findings (TOML content is not programmer commentary).
3. A Python file with real commented-out code **outside** a PEP 723 block is still
   flagged normally.
4. The exemption is Python-specific: the same `# /// script` … `# ///` pattern in
   a Go or Rust file (if it ever appeared) would not be exempted.
5. PEP 723 blocks of types other than `script` (the spec allows arbitrary `<type>`
   names) are also exempted, not just `# /// script`.
6. The closing `# ///` sentinel itself is recognized regardless of trailing whitespace.
7. A regression test covering at least the exact repro case (shebang + `# /// script`
   block with `dependencies = [` lines) is added to `comment_quality.rs`'s test module.
8. `docs/comment-quality-false-positives.md` is updated with an entry for this fix.
