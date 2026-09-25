# Prose/writing-quality checks

kibitzer ships mechanical, native checks for markdown prose quality alongside
its code checkers — see `config::prose_checks()` for what's on by default and
each checker's own module doc comment (`src/*.rs`) for its exact heuristic.

## On by default

| Check | Flags |
|---|---|
| `repetitive-sentence-structure` | 3+ consecutive sentences in a paragraph opening with the same common function word ("This...", "The...") |
| `missing-paragraph-break` | A long paragraph (7+ sentences) with a mid-paragraph contrastive transition ("However", "Additionally") |

## Opt-in (available, not on by default)

Backtesting these against ~21,000 real markdown files across 7 large
open-source doc repos (`docs/backtest-repos.md`) found each of these carries
a real risk of flagging legitimate human/technical writing, so none run
unless a repo explicitly adds them:

| Check | Flags | Why it's opt-in |
|---|---|---|
| `ai-vocabulary-density` | 3+ occurrences of a ~50-word "AI buzzword" list ("leverage", "robust", "seamless"...) in one paragraph | Several of these words ("robust", "leverage", "utilize") are ordinary in engineering prose |
| `filler-phrase-density` | 2+ hedge/filler phrases ("it is worth noting that", "needless to say"...) in one paragraph | Lowest hit rate of the five in backtesting (1 hit across all 7 repos), but still judgment-dependent |
| `formulaic-ai-openers` | A paragraph's first sentence starts with a formulaic transition ("in conclusion", "to summarize"...) | A single occurrence is enough to flag (no 3+ run required), and even the trimmed word list can appear in normal writing |
| `sentence-length-uniformity` | 6+ consecutive sentences with very low length variation | Backtesting found this flagging a deliberately parallel enumeration in real Kubernetes docs ("Signers must not... Signers should... Signers should...") with "vary sentence length" — actively wrong advice, since the parallelism was intentional |
| `em-dash-overuse` | 3+ em dashes (`—`) in one paragraph | Heavy em-dash use is also a deliberate style choice in well-edited technical writing — this repo's own doc comments trip it (confirmed: 5 hits across `docs/*.md` in this very repo) |

Enable one by adding it to `.kibitzer/inspect.json`'s `checks` array (see
`docs/suppressing-checks.md` for the general pattern):

```json
{
  "checks": [
    {
      "name": "ai-vocabulary-density",
      "checker": "ai-vocabulary-density",
      "severity": "advisory",
      "scope": ["**/*.md"]
    }
  ]
}
```

All five word/phrase lists are hardcoded per-checker (no per-project
word-list customization yet — see "Bring your own style rules" below for
today's alternative if a project needs that).

## Bring your own style rules: Vale

For a project that wants configurable, pluggable prose style rules (Google
style, Microsoft style, `write-good`, a custom house style) rather than
kibitzer's fixed mechanical heuristics, [Vale](https://vale.sh) already does
this well, and it plugs into kibitzer today with **no new kibitzer code** —
a plain `command`-based check:

```json
{
  "checks": [
    {
      "name": "vale",
      "command": "vale --output=line {file}",
      "severity": "advisory",
      "scope": ["**/*.md"]
    }
  ]
}
```

`vale --output=line` emits `path:line:col:message`, which kibitzer's
diff-scoping/baseline logic already parses correctly (it only needs the
`{file}:{line}:` prefix — a trailing `:col:` segment before the message is
fine). Two things worth knowing:

- Vale's exit code reflects only alerts at or above `MinAlertLevel` (error by
  default) — a style pack with only warning-level rules won't fail the
  command as configured above. Add `--minAlertLevel=warning` (or `suggestion`)
  to the command if you want those to count.
- kibitzer's own plugin system (`docs/plugins.md`) is a managed
  *binary-distribution* wrapper (install/verify/update a checker binary
  across machines) — it doesn't add anything here since Vale already has its
  own installer (`brew install vale`).

A native `src/vale.rs` checker that shells out to `vale --output=JSON` and
translates each alert's severity (error/warning/suggestion) into kibitzer's
own Blocking/Advisory would be a reasonable future addition if per-rule
severity mapping (rather than one flat command severity) turns out to
matter — no evidence yet that it does.
