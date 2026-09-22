# Pitfalls: cite-fowler-refactorings

## 1. URL resolution — VERIFIED, both pass

Checked two ways: `WebFetch` and a direct `curl -s -o /dev/null -w "%{http_code}"` (no redirect
chain, `-L` had no effect on either).

| URL | Status |
|---|---|
| `https://refactoring.com/catalog/extractFunction.html` | `200`, title "Extract Function" |
| `https://refactoring.com/catalog/replaceNestedConditionalWithGuardClauses.html` | `200`, title "Replace Nested Conditional with Guard Clauses" |

No redirects, no 404s. Acceptance criterion 3 is satisfiable as-is — no URL-slug fix needed
before ship.

## 2. Exact-string-equality tests on these two messages — none found

`grep -rn "assert_eq" -B2 -A2` across the repo, filtered for `long-function`/`deep-nesting`,
returned nothing. The `#[cfg(test)]` module in `src/rules.rs` (from line 1001) exercises both
rules exclusively via `.message.contains("[long-function]")` / `.contains("[deep-nesting]")` —
~15 call sites each, none anchored past the `[rule-id]` prefix. Appending text after the
existing message is safe for all of them (matches requirement 5's premise).

The one `assert_eq!` that looked relevant, `src/accepted_findings.rs:354`
(`filter_accepted_keeps_a_finding_for_a_different_rule_at_the_same_line`), is a false alarm on
closer read: the "expected" string is a synthetic fixture built inline in the same test
(`format!("{}:3: [long-function] body spans 41 lines...", ...)`, note the literal `...`), asserted
against `filter_accepted`'s pass-through behavior — it never touches the real formatter in
`src/rules.rs`. Changing the real message text does not affect this test.

No `testdata/`, `*.golden`, or `*.snap` fixtures reference either rule id (`find testdata -maxdepth 2`
shows only `comment-quality-corpus/` and `dogfood-architecture/`, neither related). No `tests/`
integration file references either rule id.

**Verdict: acceptance criterion 5 holds with zero test changes required**, but re-run
`cargo test rules::` after editing to confirm rather than trusting the grep.

## 3. Output formatters — no truncation/fixed-width risk

Traced every `Finding.message` / SARIF `result.message.text` consumer:

- `src/main.rs:399,445,650` — plain `println!("{}:{}: {}", file, line, message)`, no width cap.
- `src/check.rs:552,1196-1198` — same pattern, free-text line, no truncation.
- `src/check.rs:744` — SARIF text rendering (`lines.push(format!("{location}[{level}] {}{rule}", result.message.text))`), also unbounded.
- `src/mcp.rs:621` — MCP tool text output, same pattern.

The only `truncate()` helper in the codebase (`src/hook_log.rs:48`) is scoped to Claude
Code hook-log content byte-capping, unrelated to `Finding` rendering. `schema/inspect.schema.json`
has no `maxLength` on message-shaped fields. Nothing wraps or clips `message` anywhere in the
findings pipeline — a longer message with an embedded URL renders as one longer line, nothing
breaks structurally.

Minor readability note (not a defect): every current `Finding.message` in `src/rules.rs` is a
single unbroken line already (no internal `\n`), so a ~55-character URL tacked onto the end is
consistent with the existing style, just longer.

## 4. Backtest-triage baselines — not at risk, and none exist yet for these rules

`scripts/backtest-triage.py` keys its stored JSONL records by `(file, line, rule)`
(`key = (f["file"], f["line"], f["rule"])`, line 173) — **not** by message text. The persisted
JSONL schema (`docs/backtest-triage/*/*.jsonl`) stores `commit`, `file`, `line`, `note`,
`permalink`, `rule`, `verdict` — `message` itself is never persisted, only parsed transiently
from raw checker stdout via `FINDING_RE` at review time (line 32) and immediately discarded
after producing `rule`/`file`/`line`. A message-text change cannot invalidate a stored verdict.

Separately confirmed: no `long-function.jsonl` or `deep-nesting.jsonl` exists yet under
`docs/backtest-triage/*/` (only `flag-argument`, `commented-out-code`, `duplicate-code`,
`file-complexity`, `go-*`, `primitive-obsession`, `verbose-comment` have baselines) — so there's
no existing corpus to go stale for these two specific checks regardless.

## 5. No existing URL-in-message precedent — this would be the first

`grep -rn 'https\?://' src/*.rs` turns up plenty of URLs, but every one is in a `///`/`//` doc
comment (e.g. `src/duplicate_code.rs:317`, `src/comment_quality.rs:131,359`, `src/checker.rs:151`,
`src/config.rs:23`, `src/false_positive.rs:127`) or in `src/plugin.rs`'s URL-validation logic
(`is_http_url`, error strings about `https://` requirements) — none is inside a `Finding`'s
`message: format!(...)` that ships to an end user. This is the first native checker to put a
bare URL in a *finding* message, so there's no in-repo convention to match or deviate from
(e.g. no existing "wrap URL in `<>`" or "put it parenthetically" pattern to copy) — the item's
own proposed format (`consider <Name> (<url>)`) is as good a default as any, but should be
applied identically to both messages for internal consistency since it's establishing the
pattern, not following one.

`docs/checking-invocations.md:40-43` separately notes that kibitzer's own transcript-filtering
docs already treat finding *message text* as unstable/not a contract to match on (consumers are
told to filter on the hook `command` field, "more reliable than filtering on the content of the
message, since it doesn't depend on kibitzer's message format staying the same") — supporting
evidence that message-text changes are an accepted category of change in this codebase, not a
compatibility surface.

## Summary of risk

Low risk overall. The only real work items surfaced:

- None of the three "could break" checks (tests, formatters, backtest baselines) actually break —
  verified by direct inspection, not inferred.
- URLs verified live (200/200) at research time (2026-09-22) — re-verify at actual ship time per
  acceptance criterion 3, since this is a external, unversioned dependency that could change
  independent of this repo.
- Since there's no existing precedent for the "name + parenthetical URL" format in a finding
  message, whichever exact phrasing lands in `src/rules.rs:826-828` and `836-838` becomes the de
  facto convention — worth getting the wording right once rather than revisiting later if more
  checks get citations (explicitly out of scope per requirements, but the phrasing set here is
  the template a future item would copy).
