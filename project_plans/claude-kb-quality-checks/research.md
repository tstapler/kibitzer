# Research: Claude Code Skill Quality Checks

**Project:** claude-kb-quality-checks  
**Date:** 2026-11-04

## 1. Kibitzer Checker Infrastructure

### Checker Trait API

```rust
pub trait Checker {
    fn name(&self) -> &str;                   // stable identifier used in config + listings
    fn description(&self) -> &str;            // human-readable for `kibitzer check list`
    fn language(&self) -> Option<Language>;   // tree-sitter grammar, or None for raw-text
    fn file_globs(&self) -> &[&str];          // repo-relative **-glob patterns
    fn check(&self, file: &Path, ctx: &CheckContext) -> Result<Vec<Finding>>;

    // Optional overrides with defaults:
    fn configure(&self, opts: &serde_json::Value) -> Result<Option<Box<dyn Checker>>>;
    fn reads_cross_run_state(&self) -> bool { false }
    fn tolerates_unreadable_files(&self) -> bool { false }
}
```

`CheckContext` provides:
- `source: &str` — raw UTF-8 already loaded (checker never re-reads from disk)
- `tree: Option<&Tree>` — tree-sitter parse tree if `language()` returned `Some`

`Finding` is `{ line: usize, message: String }`.

### Registration Pattern

Each checker uses `inventory::submit!` in its own module file, adjacent to `impl Checker`:

```rust
inventory::submit! {
    crate::checker::CheckerFactory(|| vec![Box::new(MyChecker)])
}
```

The module must be declared in `src/checkers/mod.rs` and in `src/main.rs` via `mod`. The checker is then addressable by its `name()` via `inspect.json`'s `checker:` field, as well as `kibitzer check native <name>`.

### Cross-File State — Key Design Constraint

**A per-file checker receives only the current file's source via `CheckContext`.** It cannot read other files via the context API. For `claude-skill-xref`, which needs a registry of known skill IDs from `.claude/skills/`, the checker must:

1. Infer the repo root by walking `file`'s ancestors looking for `.claude/`.
2. Scan `.claude/skills/` itself inside `check()` (or via a `LazyLock<HashMap<PathBuf, HashSet<String>>>` keyed on repo root to amortize repeated calls within the same daemon session).
3. Fall back gracefully (emit no findings) when `.claude/skills/` is not found — most repos don't have a `.claude/skills/` tree at all.

The `duplicate-code-cross-file` checker uses `reads_cross_run_state() = true` for persistent cross-run indexing. The skill registry is lighter — a one-shot scan per root per daemon session, so `LazyLock` with `Mutex<HashMap<root, registry>>` is the right shape.

### Adding to `default_checks()`

`src/config.rs` wires checks into `default_checks()` by calling `native_check("check-name", Severity::Advisory, &["**/*.md"])`. The `native_check` helper produces a `Check` struct; `Check { message: Some("..."), ..native_check(...) }` adds a failure message shown to the agent.

The file-glob for skill files would be `**/.claude/skills/**/*.md`, `**/.claude/agents/**/*.md`, and `**/.claude/commands/**/*.md`.

Alternatively, a single `**/.claude/**/*.md` with an in-checker path-prefix guard reduces glob repetition.

## 2. Existing `lint-claude-kb` Reference Implementation

The script at `stapler-scripts/lint-claude-kb` already implements (stdlib-only Python):

| Check id | Severity | Logic |
|---|---|---|
| `name-mismatch` | error | frontmatter `name:` != path-implied id |
| `missing-description` | warn | frontmatter block present, no `description:` |
| `name-too-long` | error | > 64 chars |
| `name-invalid-chars` | error | outside `[a-z0-9-]` |
| `name-xml-tag` | error | paired `<tag>...</tag>` in name |
| `description-too-long` | error | > 1024 chars |
| `description-xml-tag` | error | paired XML tag in description |
| `broken-skill-ref` | error | `` `token` `` + "skill" where token not in known ids |
| `oversized-no-split` | warn/error | > 300/500 lines with no progressive-disclosure dir |
| `nested-references` | warn | subdirectory under `references/` |
| `windows-path` | warn | backslash-separated file path in body |
| `invalid-skill-location` | error | bare `<name>.md` directly under `.claude/skills/` |
| `dangling-symlink` | error | symlink with missing target |
| `broken-frontmatter-key` | error | documented-but-broken key (e.g. `paths:`) |
| `unknown-frontmatter-key` | error | key outside documented spec |
| `removed-builtin-command` | warn | reference to removed built-in slash command |

The script found 20 `name-mismatch` violations and ~46 `broken-skill-ref` violations (from a vendored Golang skill pack) on first run against the dotfiles repo.

### Frontmatter Parsing

The script hand-rolls a minimal `key: value` YAML parser (block scalars supported). This is appropriate — the kibitzer Rust implementation would do the same, since the SKILL.md frontmatter spec is a strict subset of YAML that never uses anchors, aliases, or multi-document streams.

### Path-to-ID Mapping

- Top-level skill: `.claude/skills/<name>/SKILL.md` → id = `<name>`
- Namespaced skill: `.claude/skills/<ns>/skills/<sub>/SKILL.md` → id = `<ns>:<sub>`
- Agent: `.claude/agents/<name>.md` → id = `<name>`
- Single-file skill (deprecated): `.claude/skills/<name>.md` → id = `<name>`

## 3. Scope of Native vs. LLM-Assisted Checks

### Checks that map cleanly to native (regex/filesystem):
- Frontmatter validation (all `lint-claude-kb` frontmatter checks)
- Skill cross-reference integrity (`broken-skill-ref`)
- Dangling symlink detection
- Invalid skill location (`<name>.md` directly under `skills/`)

### Checks that require LLM judgment:
- Description quality (too generic/vague to be useful in the picker) — no regex can reliably distinguish "Runs the foo workflow" from "Does stuff"
- Monolithic SKILL.md candidate — line count is a proxy; actual structural quality requires reading the content
- Near-duplicate skill clusters — requires semantic similarity, not text diff

### Checks with a heuristic proxy (could go either way):
- `oversized-no-split` — the 300/500-line threshold is a reasonable native proxy for "needs progressive disclosure"; acceptable to include in the native tier with caveats
- `nested-references` — purely structural, clearly native

## 4. Backtesting Feasibility

The backtest corpus (`docs/backtest-repos.md`) is public codebases like `kubernetes/kubernetes`, `apache/cassandra`, etc. **None of these have `.claude/skills/` trees.** The new checkers' file-glob scope (`**/.claude/**/*.md`) will match zero files in those repos, meaning:

- The backtest against the public corpus will show 0 findings — expected and correct (not a false positive, the checker correctly does nothing on repos without a `.claude/` skills tree).
- The meaningful backtest is against the **dotfiles repo** itself (or any other repo with a `.claude/skills/` tree), where the checker should reproduce the `lint-claude-kb` findings.

This is documented in `docs/backtest-repos.md` under the `stapler-squad` entry — the kibitzer repo itself is included as a corpus target. The dotfiles repo is NOT currently a corpus target but should be added.

## 5. Implementation Considerations

### Rust frontmatter parser

A simple line-by-line state machine matching `---\n` delimiters, then scanning for `key: value` lines, is sufficient. No `serde_yaml` dependency needed. The `pulldown-cmark` crate already used for `markdown-link-integrity` is NOT needed for frontmatter-only parsing.

### Skill ID registry scan

The scanner must handle:
- Symlinks (may point outside `.claude/`; must not loop)
- Mixed `SKILL.md` / bare `.md` layouts
- The `<ns>/skills/<sub>/SKILL.md` two-level namespace structure
- The `external_skills` allow-list (configurable via `configure()` with a JSON `{"external_skills": ["security-review"]}` options block)

### Glob scope

Using `**/.claude/skills/**/*.md` risks matching files in deeply nested submodule structures. A more defensive approach: match `**/*.md` and filter in-checker by checking whether the resolved file path contains `.claude/skills/` (or `.claude/agents/`, `.claude/commands/`) as a path segment. This avoids glob complexity while keeping the checker correctly scoped.
