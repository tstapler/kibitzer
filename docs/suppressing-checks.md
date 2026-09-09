# Suppressing a check

kibitzer runs a built-in default catalog everywhere — pylint-style, no
`.claude/inspect.json` required (`config::default_checks()` in `src/config.rs`;
the full list is in `docs/syntax-rules.md` and `docs/comment-quality.md`, plus
`markdown-link-integrity`, `primitive-obsession`, `duplicate-code`,
`duplicate-code-cross-file`, `go-blank-imports`, `go-ignored-error`,
`go-error-context`). A local
`.claude/inspect.json`
overlays that catalog rather than replacing it — see
`config::find_effective_config`. There is no inline/per-line suppression
comment (`// kibitzer:disable ...`, `# noqa`, etc.) — see
`docs/reporting-false-positives.md`'s "What this is not" section for why. The
levers below are all config-based.

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

## If a finding looks flat-out wrong

That's not a suppression question — see `docs/reporting-false-positives.md`
for how to report it so the underlying check gets fixed.
