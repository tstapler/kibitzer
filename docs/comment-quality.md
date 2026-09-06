# Comment quality catalog

`comment-quality-<lang>` (`src/comment_quality.rs`) is a native checker,
registered once per language (Go, TypeScript, TSX, JavaScript, Python, Java,
Kotlin), that flags comments working against their own purpose instead of
for it. It's one of kibitzer's built-in defaults (`config::default_checks()`)
— it runs everywhere, no `.claude/inspect.json` required; see
`docs/suppressing-checks.md` to disable or scope it.

It reuses `rules::lang_config()`'s per-language function/body node-kind table
(`LangRuleConfig`, `pub(crate)`) rather than re-deriving comment/function node
kinds — the same table `syntax-rules-<lang>` (`docs/syntax-rules.md`) is
built on.

| Language   | Checker name                    | File globs                              |
|------------|----------------------------------|------------------------------------------|
| Go         | `comment-quality-go`             | `**/*.go`                                 |
| TypeScript | `comment-quality-typescript`     | `**/*.ts`                                 |
| TSX        | `comment-quality-tsx`            | `**/*.tsx`                                |
| JavaScript | `comment-quality-javascript`     | `**/*.js`, `**/*.jsx`, `**/*.mjs`, `**/*.cjs` |
| Python     | `comment-quality-python`         | `**/*.py`                                 |
| Java       | `comment-quality-java`           | `**/*.java`                               |
| Kotlin     | `comment-quality-kotlin`         | `**/*.kt`, `**/*.kts`                     |

## Findings

| Finding prefix | What it catches |
|-----------------|------------------|
| `[verbose-comment]` | A comment contains marketing/hedge-word filler (`seamlessly`, `powerful`, `robust`, `enterprise-grade`, `leverage`, `utilize`, `it's worth noting`, `as mentioned above`, `note that`, `needless to say`, `it is important to note that`), invented rationale (`designed to improve`, `supports future`), bureaucratic wordy-filler phrases hand-picked from Vale's write-good `TooWordy.yml` style pack (`in order to`, `due to/because of/by virtue of/in spite of the fact that`, `in the event that`, `with regard(s) to`, `for the purpose of`), or references the change instead of the code (`used by`, `added for the`, `this fix`, `this PR`, `handles the case from issue`) — that belongs in the commit message, not a comment that outlives it. Matched case-insensitively, substring. `Weasel.yml` and the rest of `TooWordy.yml` were deliberately not imported wholesale — both flag ordinary technical vocabulary (`currently`, `correctly`, `eliminate`, `employ`) that reads fine in a code comment. |
| `[commented-out-code]` | A comment line looks like dead code rather than prose — ends in `;`/`{`, is a bare `}`, or looks like a call/assignment expression (`strip_comment_markers` + `looks_like_code`, deliberately conservative: biased toward missing real dead code over flagging prose). |
| `[over-commented]` | A declaration's total comment lines (leading doc comment plus any comments inside its body) are both at least `MIN_COMMENT_LINES_FOR_RATIO` (4) and at least `COMMENT_TO_CODE_RATIO` (2.0×) the body's code-line count — tuned so a well-justified "why" comment as long as the function it documents (ratio ~1.0) does *not* fire; see `examples/*/comment-good.*`. |

## Wiring into `.claude/inspect.json`

Runs by default already — an explicit entry is only needed to change its
severity or scope for one repo (see `docs/suppressing-checks.md`):

```json
{
  "checks": [
    {
      "name": "comment-quality-go",
      "checker": "comment-quality-go",
      "severity": "advisory",
      "scope": ["**/*.go"]
    }
  ]
}
```
