# `comment-quality-<lang>` — known false positives

Tracks confirmed false-positive firings of the `comment-quality-<lang>` checkers
(`src/comment_quality.rs`) — `[verbose-comment]`, `[commented-out-code]`, and
`[over-commented]`. Check new occurrences against this list, and against the entries
below already fixed, before re-investigating a firing from scratch. See
`docs/reporting-false-positives.md` for how to file a new one.

The `[over-commented]` proportionality check (comment lines >= 4 and >= 2.0x the
declaration's body code-line count) has no external benchmark behind its threshold —
unlike `[commented-out-code]`, which mirrors SonarQube's long-shipped S125 rule, no
other tool or published research validates a specific comment-to-code ratio (see the
2026-09-06 research session referenced in kibitzer's `CLAUDE.md`). Treat `2.0x`/`4
lines` as a starting hypothesis to tune against real firings, not a validated constant.

## Fixed

### Semicolon-terminated bullet-list doc comment misread as commented-out code

- **Symptom**: a doc comment written as a semicolon-separated bullet list (a common
  technical-writing style — "- validates input;", "- normalizes casing;") fired
  `[commented-out-code]` on every line, even though none of it was code.
- **Mechanism**: `looks_like_code`'s `text.ends_with(';')` branch treated any
  semicolon-terminated clause as code-shaped, with no check that it also carried any
  code-only punctuation — the same documented gap as SonarQube's S125
  (community reports of prose semicolons being misflagged).
- **Fixed by**: requiring the semicolon-ending branch to also see a code-only
  punctuation character (`(){}[]=<>+*/&|!` — deliberately excluding `.`/`,`, both
  common in ordinary prose). Regression-guarded by
  `comment_quality::tests::does_not_flag_a_semicolon_terminated_bullet_list`.
- **Known trade-off**: a punctuation-free single-word statement like a bare `return;`
  or `break;` is no longer flagged as commented-out code. Accepted per this checker's
  stated bias toward missing real dead code over flagging prose.

## Log
