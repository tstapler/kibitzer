# Suppressing a check

kibitzer runs a built-in default catalog everywhere — pylint-style, no
`.kibitzer/inspect.json` required (`config::default_checks()` in `src/config.rs`;
the full list is in `docs/syntax-rules.md` and `docs/comment-quality.md`, plus
`markdown-link-integrity`, `primitive-obsession`, `duplicate-code`,
`duplicate-code-cross-file`, `file-complexity`, `go-blank-imports`,
`go-ignored-error`, `go-error-context`). A local
`.kibitzer/inspect.json`
overlays that catalog rather than replacing it — see
`config::find_effective_config`. To dismiss one finding you judged acceptable,
write a `kibitzer:ignore` comment at the flagged spot (see
[Dismiss one finding inline](#dismiss-one-finding-inline)). The other levers in
this doc are config-based and cover whole checkers, files, or directories.

## Turn a default check off entirely

Add its `name` to `disabled` in the repo's `.kibitzer/inspect.json`:

```json
{
  "disabled": ["primitive-obsession"]
}
```

This only removes a *default*. It has no effect on a check you added
yourself — just delete it from `checks` instead.

## Change a default's severity or scope

Add a `checks` entry that reuses the default's exact `name`. It replaces the
default outright rather than running alongside it:

```json
{
  "checks": [
    {
      "name": "markdown-link-integrity",
      "checker": "markdown-link-integrity",
      "severity": "advisory",
      "scope": ["**/*.md"]
    }
  ]
}
```

## Exclude one file or directory, keep the check everywhere else

`scope` patterns prefixed with `!` are exclusions (`src/glob.rs::matches_scope`),
checked after the positive patterns — gitignore-style:

```json
{
  "checks": [
    {
      "name": "primitive-obsession",
      "checker": "primitive-obsession",
      "severity": "advisory",
      "scope": ["**/*.go", "!vendor/**", "!internal/generated/api.go"]
    }
  ]
}
```

A `checks` entry with only negative patterns (no positive ones) matches
everything except what it excludes — equivalent to starting from `**/*`.

## Dismiss one finding inline

The levers above are whole-checker or whole-file/directory. For a genuine hit at
one specific line that you've deliberately decided to keep — a real
`flag-argument` match on a CLI's standard `-v` toggle, say — disabling the whole
checker for that file would also silence every other rule it covers there. Put a
comment at the flagged spot instead:

```go
// kibitzer:ignore flag-argument -- standard -v toggle, matches the CLI contract
func run(verbose bool) {}
```

In Markdown the comment is an HTML comment:

```markdown
<!-- kibitzer:ignore markdown-link-integrity -- placeholder until the page lands -->
```

### Grammar

```text
<comment leader> kibitzer:ignore <rule>[,<rule>...] -- <reason>
```

- **One marker.** Only `kibitzer:ignore` exists. `kibitzer:false-positive`,
  `kibitzer:disable` and similar near-misses are reported as `[ignore-syntax]`
  and suppress nothing.
- **Leaders.** `//`, `#`, `/* */`, `<!-- -->` and the doc-comment forms, in a real
  comment (a string literal never counts). The directive must start the comment.
- **`<rule>`** is the finding's `[rule]` prefix, or the checker name. Several
  rules go in one comma-separated list. Findings with no `[rule]` prefix answer to
  their checker name; the hook footer lists the ids to copy. Rule ids are lowercase
  `[a-z0-9-]`, at most 64 characters, and a directive names at most 16 of them.
- **`<reason>`** is required: at least two words, at most 300 characters, and not
  just the rule id repeated. Use `--`; an em dash is rejected. An over-long rule id,
  rule list or reason is reported as `[ignore-syntax]` and suppresses nothing.
- **Native checkers only.** Directives apply to kibitzer's own per-file checkers (the
  ones `kibitzer check list` shows). A shell `command` check, a plugin, an
  architecture check and a file type with no native checker ignore them, so the hook
  and MCP footers offer the syntax only when a failing native finding could use it.
- **Scope.** A comment on its own line covers its own rows and the row directly
  below. If comment lines follow it with no blank row between (a reason continued on
  `//` lines), it covers the first row after that stack. A trailing comment after
  code covers only its own row. In Markdown, list (`-`, `1.`) and blockquote (`>`)
  markers before the comment do not make it trailing.
- **Misplaced directives.** A directive after other text in a comment is reported
  (`[ignore-syntax]`) only when a bare `TODO`/`FIXME`/`NOTE`-style label precedes it;
  prose that merely describes the syntax is left alone. An upper-case marker such as
  `KIBITZER:IGNORE` is reported as a near miss.
- **Never suppressible:** the directive diagnostics themselves (`inline-ignore`,
  `ignore-syntax`, `unused-ignore`, `ignore-volume`, `blocking-suppressed`).
- A malformed directive never suppresses; its `[ignore-syntax]` message says the
  exact fix.

### Where the comment goes

Most checkers anchor on the flagged statement: the comment goes on the line above
(or at the end of that line). The exceptions:

| Checker | Where the ignore goes |
|---|---|
| `file-size` (also its per-language checker names such as `python-file-size`) | First 10 lines of the file, or the reported (last) line |
| `file-complexity` | Above one function to dismiss only it; in the first 10 lines to dismiss all of them |
| `duplicate-code` | On its own row directly above the last block's first line. A trailing comment becomes part of the block text and re-anchors the finding |
| `duplicate-code-cross-file` | Same, in each file you want quiet; one file's ignore does not cover the other |
| `markdown-link-integrity` | Name the checker, not the link label: `markdown-link-integrity`, because the `[label]` in its messages is data |
| `over-commented`, `commented-out-code` | Directly above the first flagged comment row; for dead code, delete it instead |

### Limits worth knowing

- **Stale ignores.** In the PostToolUse hook an unused (stale) ignore is reported
  only when the directive sits inside the lines just edited. A stale ignore
  elsewhere in a file is not reported by the hook, and `kibitzer run` does not
  audit them yet. `kibitzer run` does flag a rule name that matches no known rule
  or checker, as a probable typo.
- **Visibility.** `kibitzer run` prints how many findings were suppressed inline
  (and how many came from blocking checks); `kibitzer run --no-inline-ignores`
  shows them again. A directive that silences a blocking finding raises
  `[blocking-suppressed]` in the hook so the agent tells the user. That also holds
  for a whole-file `Write`, where every directive counts as just added (at most 10
  advisories, then one count line). The advisory also fires when an edit touches the
  row of the silenced finding (a directive planted earlier now hiding a new finding),
  and after a pure deletion for every blocking finding silenced in that file. The advisory shows the reason in quotes, cut to
  160 characters.
- **Failing CI on a suppressed blocking finding.** `kibitzer run` exits 0 when the
  only blocking finding was suppressed inline. Add `--deny-blocking-suppression` to
  exit 1 instead; the default is unchanged.
- **Untrusted text.** Two sanitizer tiers (`src/inline_ignores/sanitize.rs`):
  - *Strict* (`echo`): directive reasons, rule text, near-miss markers and file paths
    that kibitzer composes into its own `[ignore-syntax]`, `[unused-ignore]` and
    `[blocking-suppressed]` lines. Only printable text survives (an allow-list: no
    control, format, variation-selector, tag, bidi, filler or other invisible
    characters), whitespace collapses to one line, and the text is shortened; a
    reason is shown in quotes as data.
  - *Lenient* (`strip_unsafe`): the finding text other checkers produce, on hook
    stderr, hook `additionalContext`, `run_checks` MCP output and `kibitzer run`
    stdout. It removes escapes and other control characters (carriage return
    included), bidi embeddings, overrides and isolates, line and paragraph
    separators, Unicode tag characters and variation selectors other than VS16. It
    keeps tabs, newlines, ZWJ, ZWNJ and VS16, which emoji, Persian and Indic text need.
  - A file path in lenient output has its newlines and other control characters
    shown as escapes (`\n`), so a file name cannot start a forged line. Paths that a
    checker prints in a form kibitzer cannot match (for example relative to another
    directory) are protected only by the lenient pass; a file name's own newline is
    escaped wherever its base name appears.
  - Not covered: subcommands that print findings directly (`kibitzer check native`
    and the other `kibitzer check` diagnostics in `src/main.rs`) show checker output
    unfiltered.
- **Footers never suggest meta rules.** When the only failure is an `[ignore-syntax]`
  repair, the footer omits the ignore hint instead of suggesting a directive that
  cannot work.
- **`kibitzer check native <name> <file>`** runs the raw checker and bypasses
  inline ignores (and `accepted/`), so it shows what an ignore is hiding.
- **Fallback-only files.** The malformed-directive and unused-ignore diagnostics run
  on grammar-backed languages and Markdown. Other files (shell, YAML, and so on)
  honor whole-line `//`, `#`, `;` and `--` directives but never report a malformed one.
- **Dev builds.** The daemon's `cache.json` is keyed on the package version, so a
  rebuild at the same version can serve stale results. After rebuilding, delete
  `cache.json` (under `$XDG_CACHE_HOME/kibitzer/` or `~/.cache/kibitzer/`) or
  restart the daemon. A daemon left running across an upgrade is retired
  automatically: every reply carries its version, and a client that sees a different
  one (or none) stops using that daemon and asks it to exit.

## Accept one specific, correctly-flagged finding

Where a location cannot hold a comment (generated code, a file you must not edit),
see `docs/accepting-findings.md` for `.kibitzer/accepted/`, a per-finding lever
kept in a checked-in file. It requires a written reason and covers only that one
location.

## If a finding looks flat-out wrong

That's not a suppression question — see `docs/reporting-false-positives.md`
for how to report it so the underlying check gets fixed.
