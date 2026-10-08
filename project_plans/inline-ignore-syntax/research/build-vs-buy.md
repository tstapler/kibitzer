# Build vs Buy: inline ignore directives

**Date**: 2026-10-07. Sources: `Cargo.toml`, `src/checkers/comment_quality.rs:173-180`, `src/accepted_findings.rs:131`, `requirements.md`. Ecosystem claims about third-party tools are from general knowledge and are UNVERIFIED (no web lookup done).

## Finding: the dependencies already exist

`Cargo.toml` already has `regex`, `tree-sitter` 0.26, and grammars for Go, TypeScript, JavaScript, Python, Java, Kotlin, Rust, plus `pulldown-cmark` for Markdown. `comment_quality.rs` already has `comment_kinds(lang)`, mapping each language to its tree-sitter comment node kinds (`comment`, or `line_comment`/`block_comment` for Java/Kotlin/Rust). Adding a directive scanner needs no new crate.

## Option 1: OSS crate for directive or comment parsing

| Choice | Pros | Cons | Verdict |
|---|---|---|---|
| Reuse in-repo tree-sitter comment nodes + a single `regex` for the directive text | Zero new deps. Immune to comment-in-string false matches (the stated rabbit hole). Same code path the comment-quality checks use. Gives exact comment line/column. | Covers only the 7 tree-sitter languages. Markdown needs `pulldown-cmark` HTML-comment events instead. Needs a parse per file for any checker that does not already parse. | **Recommended** (tree-sitter for code languages, pulldown-cmark for Markdown) |
| Hand-rolled per-language comment-prefix table + line regex | Trivial, fastest, works on any file with no grammar. | Matches `// kibitzer:ignore` inside string literals, raw strings and heredocs. Needs maintenance per new language. Duplicates knowledge already in `comment_kinds`. | Viable as a fallback only for file types with no grammar (YAML, shell, TOML) |
| Third-party comment-extraction crate (e.g. generic "comment parser" crates) | Broad language list. | Unverified quality and maintenance. Regex-based ones share the string-literal flaw. Adds a second comment model next to tree-sitter. | Not recommended |
| Third-party suppression-directive crate | None known to exist for Rust that is language-agnostic (UNVERIFIED). | Nothing to adopt. | Not recommended |

## Option 2: SaaS

Not applicable. Kibitzer is a local, offline CLI/hook/daemon/LSP; a hosted service would add latency to the hook path (violating the performance NFR) and send source off-machine.

## Option 3: LLM-generated bespoke parser vs reusing a library

| Choice | Pros | Cons | Verdict |
|---|---|---|---|
| LLM-written bespoke multi-language comment lexer | Fast to draft. | Comment lexing is full of edge cases: nested block comments (Rust, Kotlin), raw strings, template literals, Python triple quotes, Go raw strings. A generated lexer is plausible-looking but untested against those. Needs a large fixture corpus to trust. | Not recommended |
| LLM-assisted code that delegates lexing to tree-sitter and owns only the directive grammar (`kibitzer:ignore <rule> -- <reason>`) | Correctness risk drops to a small, unit-testable regex. The project's backtest requirement (`CLAUDE.md`, "Writing a new check") then catches real-world noise. | The remaining logic (anchor line, multi-line findings) is still design work, not parsing. | **Recommended** |

## Option 4: adopt an existing directive syntax

| Choice | Pros | Cons | Verdict |
|---|---|---|---|
| Reuse `# noqa`, `// nolint`, `eslint-disable-next-line` or `nosemgrep` verbatim | Agents already know them. Zero doc cost. | Each is owned by another tool, which will also honor (or choke on) it. `nolint:rule` would silently suppress golangci-lint too, and `eslint-disable` with an unknown rule can make ESLint report an error. Reasons are optional in most (`noqa`, `nolint`), violating the mandatory-reason constraint. Rule namespaces differ. `requirements.md` already rejected this. | Not recommended |
| Own namespace, borrowing familiar shape: `kibitzer:ignore <rule> -- <reason>` (the `--` reason separator is ESLint's `-- description` form, and next-line/same-line scope follows `nolint`/`nosemgrep`) | Agents infer it from the shape. No cross-tool collision. Reason is enforceable. Can add `kibitzer:false-positive` or an `fp` tag for the worklist. | Needs docs plus one line in the hook output telling agents the syntax. | **Recommended** |
| Also accept `nosemgrep`/`NOLINT` as aliases | Marginal convenience. | Reintroduces coupling and the missing-reason problem, so aliases would need a separate rule. | Not recommended (revisit only if agents demonstrably guess these) |

## Summary

Build the directive grammar in-house (small regex), buy the hard part (comment detection) from tree-sitter and pulldown-cmark, which are already dependencies. Use a `kibitzer:`-namespaced directive modelled on ESLint's `-- reason` shape. No SaaS.

Gaps: no web verification of third-party crates or of ESLint/golangci-lint behavior with foreign directives; line-anchor handling for multi-line and cross-file findings is a design question this note does not resolve.
