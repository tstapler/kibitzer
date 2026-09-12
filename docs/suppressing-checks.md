# Suppressing a check

kibitzer runs a built-in default catalog everywhere — pylint-style, no
`.claude/inspect.json` required (`config::default_checks()` in `src/config.rs`;
the full list is in `docs/syntax-rules.md` and `docs/comment-quality.md`, plus
`markdown-link-integrity`, `primitive-obsession`, `duplicate-code`,
`duplicate-code-cross-file`, `file-complexity`, `go-blank-imports`,
`go-ignored-error`, `go-error-context`). A local
`.claude/inspect.json`
overlays that catalog rather than replacing it — see
`config::find_effective_config`. There is no inline/per-line suppression
comment (`// kibitzer:disable ...`, `# noqa`, etc.) — the per-finding lever
below (`docs/accepting-findings.md`) covers that granularity via a checked-in
file instead, kept reviewable rather than scattered through source. The
levers in this doc are all config-based.

## Turn a default check off entirely

Add its `name` to `disabled` in the repo's `.claude/inspect.json`:

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

## Accept one specific, correctly-flagged finding

The levers above are whole-checker or whole-file/directory. For a genuine hit at
one specific line that you've deliberately decided to keep — a real
`flag-argument` match on a CLI's standard `-v` toggle, say — disabling the whole
checker for that file would also silence every other rule it covers there. See
`docs/accepting-findings.md` for `.claude/kibitzer-accepted.json`, a per-finding
lever that requires a written reason and only that one location.

## If a finding looks flat-out wrong

That's not a suppression question — see `docs/reporting-false-positives.md`
for how to report it so the underlying check gets fixed.
