# Accepting a genuine finding as a deliberate tradeoff

kibitzer's other two levers for a finding don't cover this case:

- **`docs/reporting-false-positives.md`** is for a checker *misfiring* — a finding
  that doesn't actually apply to the code it's pointing at. It's explicitly not a
  suppression mechanism: filing a report never silences the check.
- **`docs/suppressing-checks.md`**'s `.claude/inspect.json` (`disabled`, or a
  `checks` entry's `scope`) silences a whole checker, or a whole file/directory for
  that checker — not one specific, correctly-flagged line.

Neither fits a genuine, correctly-flagged finding at one specific location that you've
deliberately decided to keep — e.g. `flag-argument` correctly catching a CLI's
standard `-v`/`verbose bool` toggle, or the one boundary function that has to convert
an external `bool` flag into your own enum. Disabling the whole checker for that file
would also silence every other rule it covers there; a note in the repo's own
`CLAUDE.md` documents the reasoning for a human but is invisible to kibitzer, which
will re-flag the same line on every future edit.

## `.kibitzer/accepted/`

A checked-in directory of hand-authored files, one JSON file per accepted finding —
not one shared array, so two people accepting different findings on different
branches each add a new file instead of both editing the same list, and the merge
never conflicts. Filename doesn't matter (kibitzer globs every `*.json` file directly
in the directory); name it however makes the entry easy to find later, e.g.
`flag-argument-main-rs-42.json`:

```json
{
  "rule": "flag-argument",
  "file": "crates/cli/src/main.rs",
  "line": 42,
  "content": "pub fn from_dry_run_flag(dry_run: bool) -> Mode {",
  "reason": "the one boundary adapter converting a clap bool flag into a Mode enum — something has to do that conversion once"
}
```

- **`rule`** — the rule id, not the checker name, when the two differ. Most native
  checkers self-prefix their message with `[rule-id]` (`syntax-rules-*`'s
  `flag-argument`/`long-function`/`deep-nesting`, `comment-quality-*`'s
  `verbose-comment`/`commented-out-code`/`over-commented`, etc.) — copy that bracketed
  id verbatim. A checker whose messages don't self-prefix one (`primitive-obsession`,
  `duplicate-code`, `file-complexity`, `go-blank-imports`, `go-ignored-error`,
  `go-error-context`) has only one rule, so use the checker's own name instead
  (`kibitzer check list` prints every native checker's name).
- **`file`** — repo-root-relative, forward-slash-separated, matching the same
  convention `Check.scope` glob patterns use.
- **`line`** — the 1-indexed line the finding is reported on.
- **`content`** — the flagged line's exact source text (leading/trailing whitespace
  is ignored when comparing) at the time you wrote this entry. This is the
  staleness guard: if the line's *current* content no longer matches, the entry
  silently stops applying and the finding reappears — the code changed enough that
  the original judgment call needs a fresh look, not a rubber stamp on whatever's
  there now. There's no separate "stale" warning to check for; a reappeared finding
  *is* the signal.
- **`reason`** — required, and must be non-empty. A malformed entry (bad JSON, or an
  empty `reason`) is a hard error on the next check run, not a silent no-op — the
  whole point of this directory over `disabled` is that the tradeoff gets written down.

Walked upward from the checked file the same way `.claude/inspect.json` is (so one
`.kibitzer/accepted/` at the repo root covers the whole tree).

## Scope

Applies to **native per-file checkers only** (the ones invoked via `kibitzer check
native <name> <file>` — `default_checks()`'s catalog plus anything you've added to
`.claude/inspect.json`'s `checks` with a `checker` field). A shell-out (`command`)
check's pass/fail comes from its process exit code, not from whether its output text
is empty, so accepting away one of its output lines can't safely flip that check to
"passed" the way it can for a native one — out of scope for now. Whole-repo
architecture checks (`instability`, `layering`, `change-coupling`, etc.) are also out
of scope; they report at package/component granularity, not a specific line, so this
directory's `(rule, file, line, content)` key doesn't fit them.

There's still no inline suppression comment (`// kibitzer:accept ...`) — this directory
is the mechanism, kept checked-in and reviewable rather than scattered through source,
and consistent with `docs/suppressing-checks.md`'s existing config-based-only stance.

## Removing an entry

Delete its file once the accepted tradeoff no longer applies — a refactor removed the
flag argument, the file was deleted, or you've reconsidered and want to fix the
underlying finding instead. There's no automatic pruning: an entry whose line has
drifted just stops suppressing (see `content` above) but stays until someone deletes it.
