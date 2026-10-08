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
  their checker name; the hook footer lists the ids to copy.
- **`<reason>`** is required: at least two words, and not just the rule id
  repeated. Use `--`; an em dash is rejected.
- **Scope.** A comment on its own line covers its own rows and the row directly
  below. A trailing comment after code covers only its own row.
- **Never suppressible:** the directive diagnostics themselves (`inline-ignore`,
  `ignore-syntax`, `unused-ignore`, `ignore-volume`, `blocking-suppressed`).
- A malformed directive never suppresses; its `[ignore-syntax]` message says the
  exact fix.

### Where the comment goes

Most checkers anchor on the flagged statement: the comment goes on the line above
(or at the end of that line). The exceptions:

| Checker | Where the ignore goes |
|---|---|
| `file-size` | First 10 lines of the file, or the reported (last) line |
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
  `[blocking-suppressed]` in the hook so the agent tells the user.
- **`kibitzer check native <name> <file>`** runs the raw checker and bypasses
  inline ignores (and `accepted/`), so it shows what an ignore is hiding.
- **Fallback-only files.** The malformed-directive and unused-ignore diagnostics run
  on grammar-backed languages and Markdown. Other files (shell, YAML, and so on)
  honor whole-line `//`, `#`, `;` and `--` directives but never report a malformed one.
- **Dev builds.** The daemon's `cache.json` is keyed on the package version, so a
  rebuild at the same version can serve stale results. After rebuilding, delete
  `cache.json` (under `$XDG_CACHE_HOME/kibitzer/` or `~/.cache/kibitzer/`) or
  restart the daemon.

## Accept one specific, correctly-flagged finding

Where a location cannot hold a comment (generated code, a file you must not edit),
see `docs/accepting-findings.md` for `.kibitzer/accepted/`, a per-finding lever
kept in a checked-in file. It requires a written reason and covers only that one
location.

## If a finding looks flat-out wrong

That's not a suppression question — see `docs/reporting-false-positives.md`
for how to report it so the underlying check gets fixed.
