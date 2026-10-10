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
| `hollow-qualifier-phrasing` | A heading, or a paragraph's first sentence, ending `, <adverb>` ("The primitive, generically"; default adverbs: generically, fundamentally, essentially, basically, broadly, generally, abstractly, conceptually, theoretically, ultimately, nominally) | One user-reported occurrence so far (below the repo's 2+ bar), and a conversational trailing hedge ("...personal information, essentially") matches the same shape; see "Backtest: hollow-qualifier-phrasing" below |

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

### Customizing a checker's word list/thresholds

Every checker in the table above reads its word/phrase list and thresholds
from a `Check.options` object, falling back to its own hardcoded defaults for
anything `options` doesn't set — no code change needed to add a house-style
word or tune a threshold for one project. See each checker's own module doc
comment (`src/checkers/*.rs`) for its exact `options` shape; for example:

```json
{
  "checks": [
    {
      "name": "ai-vocabulary-density",
      "checker": "ai-vocabulary-density",
      "severity": "advisory",
      "scope": ["**/*.md"],
      "options": { "words": ["synergy", "circle back"], "threshold": 2 }
    },
    {
      "name": "sentence-length-uniformity",
      "checker": "sentence-length-uniformity",
      "severity": "advisory",
      "scope": ["**/*.md"],
      "options": { "min_sentences": 10 }
    },
    {
      "name": "hollow-qualifier-phrasing",
      "checker": "hollow-qualifier-phrasing",
      "severity": "advisory",
      "scope": ["**/*.md"],
      "options": { "adverbs": ["generically", "fundamentally", "in essence"] }
    }
  ]
}
```

`hollow-qualifier-phrasing`'s `adverbs` replaces the whole default list when set
(it does not append to it).

`options` is validated at config-load time (`kibitzer run`/`kibitzer check`
fails immediately with the config path and check name, not a silent
fallback) — a typo like `"threshold": "two"` is caught before any check
actually runs. A checker with nothing to configure (e.g. the two default,
non-opt-in checks above) ignores `options` entirely rather than erroring.

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

## Backtest: hollow-qualifier-phrasing

Run 2026-10-10 against every repo in `docs/backtest-repos.md` (all 15 clones present
locally, code and prose), over each repo's `.md` files:

```sh
find ~/code/github.com/<owner>/<repo> -name '*.md' -type f -print0 \
  | xargs -0 -n1 -P8 kibitzer check native hollow-qualifier-phrasing
```

| Repo | `.md` files | Findings |
|---|---|---|
| `kubernetes/kubernetes` | 590 | 0 |
| `apache/cassandra` | 91 | 0 |
| `servo/servo` | 388 | 0 |
| `BurntSushi/ripgrep` | 23 | 0 |
| `denoland/deno` | 129 | 0 |
| `microsoft/vscode` | 353 | 0 |
| `android/nowinandroid` | 43 | 0 |
| `kubernetes/website` | 8,239 | 1 |
| `gitlabhq/gitlabhq` | 8,491 | 0 |
| `docker/docs` | 1,375 | 0 |
| `mdn/content` | 14,688 | 0 |
| `microsoft/vscode-docs` | 847 | 0 |
| `rust-lang/book` | 478 | 0 |
| `rust-lang/rustc-dev-guide` | 248 | 0 |
| `tstapler/stapler-squad` | 2,616 | 0 |
| **Total** | **38,599** | **1** |

A `kibitzer check backtest hollow-qualifier-phrasing` run over local Claude Code
transcripts (1,645 reconstructable snapshots) produced 1 more finding.

- **Corpus FP (1 of 1):** `kubernetes/website`
  `content/en/blog/_posts/2020/kubernetes-1-18-release-interview.md:95`, a spoken
  interview line ending `..., essentially`. Conversational hedging, not a hollow heading.
- **Transcript FP (1 of 1):** a planning doc enumerating the adverb list itself in code
  spans (`generically`, `fundamentally`, ...) whose last item matched. Docs that describe
  the checker will trip it; suppress with `kibitzer:ignore`.
- **False-positive rate:** 2 findings, 0 true positives. 1 per ~38,600 corpus files.
  With no true positive, precision cannot be estimated, only that noise is very low.
- **No second true occurrence turned up**, and no heading matched anywhere, so the
  "2+ independent occurrences" bar from `docs/check-ideas.md` is still unmet. The check
  ships opt-in and advisory for that reason.
