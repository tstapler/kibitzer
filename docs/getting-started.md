# Getting started in a new repo

Wiring kibitzer into a project you haven't used it in before.

## 1. Install

See the README's [Install](../README.md#install) section (Homebrew or
`cargo install --path .`).

## 2. Try it with zero config

`config::default_checks()` (`src/config.rs`) ships a full catalog —
markdown link integrity, per-language file-size/comment-quality/syntax
checks, duplicate-code detection, and more — that runs with no
`.claude/inspect.json` at all:

```bash
kibitzer run . --trigger batch
```

Example, against a repo with a dangling markdown anchor:

```
$ kibitzer run . --trigger batch
[BLOCKING] ./README.md — markdown-link-integrity: broken markdown link/anchor
./README.md:3: #does-not-exist -> no such heading in this doc
```

A `.claude/inspect.json` is only needed to *customize* this — add
architecture checks (component dependency rules, naming rules), suppress a
specific default, or shell out to a project-local linter. See
`kibitzer schema` and `docs/suppressing-checks.md` for that; skip it if the
defaults already cover what you need.

## 3. Register the `PostToolUse` hook

```bash
kibitzer install             # this project only: <cwd>/.claude/settings.json
kibitzer install --global    # every project: ~/.claude/settings.json
kibitzer install --dry-run   # preview the resulting file without writing it
```

This merges `PostToolUse` (runs on every `Edit`/`Write`) and `Stop` hook
entries into `settings.json` using the resolved absolute path to the
installed binary — idempotent, safe to rerun, and it won't clobber other
hooks already configured there.

## 4. The daemon (optional, auto-starts itself)

Repeat invocations (one per edit, via the hook) re-parse and re-check from
scratch unless a daemon is caching results. `kibitzer hook` auto-spawns one
in the background the first time it finds none reachable, so there's no
manual step — later hook calls in the same session just find it already
running. Manage it directly if you want:

```bash
kibitzer daemon start   # runs in the foreground — background it yourself
                         # ('&', a systemd unit, a launchd agent)
kibitzer daemon status  # "daemon is running" / "no daemon running"
kibitzer daemon stop
```

## 5. Verify it's actually firing

Don't trust a text grep for `"kibitzer"` in Claude Code's session
transcripts — it matches source code and conversation, not just real hook
runs. See `docs/checking-invocations.md` for the recipe that filters on
the structured `hook_success`/`hook_blocking_error` attachment types
instead.
