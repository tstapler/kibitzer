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

## 6. `markdown-link-integrity` scope — markdown files only, does not apply

[`src/markdown_link_integrity.rs:67`](src/markdown_link_integrity.rs#L67) implements `Checker`'s
`glob_patterns()` as `&["**/*.md"]`, and [`language()`](src/markdown_link_integrity.rs#L61)
returns `None` (language-agnostic, but file-pattern-gated). The checker runner
(`src/checker.rs`) dispatches per-checker by matching a file against its `glob_patterns()`
before invoking it, so `.rs` files never reach this checker regardless of what string literals
they contain. The two new URLs, embedded inside Rust `format!` string literals in `src/rules.rs`,
are structurally invisible to `markdown-link-integrity` — it never parses `.rs` source, only
`.md` documents. No risk of this check firing on, or gatekeeping, the new URLs.

(Requirement 3's "already confirmed 200 OK... before ship" step is a plain `curl -sI`/`WebFetch`,
not this checker — see §1 above, which already covers live-URL verification.)

## 7. clippy / rustfmt / CI build risk — none found

- **`cargo fmt --all --check`** ([`.github/workflows/ci.yml:28`](.github/workflows/ci.yml#L28))
  is in the CI gate. Checked whether appending ~55–70 chars of URL text to the two `format!`
  string literals could produce a diff `cargo fmt` would want to make (which would fail this
  check): the *existing*, currently-shipping literal at
  [`src/rules.rs:827`](src/rules.rs#L827) (`"[long-function] body spans {body_lines} lines (over
  {LONG_FUNCTION_LINES}) — consider splitting it up"`) is already 123 characters on its own line
  (`awk 'NR==827{print length($0)}' src/rules.rs` → `123`), well past rustfmt's default 100-char
  `max_width`, and CI passes on `master` today with this line as-is. rustfmt does not split or wrap the contents of a string literal — it only reflows
  surrounding code (call/argument layout) — so an overlong string-literal line is not something
  `cargo fmt --check` flags. Making the literals longer by appending catalog text carries the
  same non-risk as the current code. Running `cargo fmt` locally after the edit (as normal
  practice) is still the right verification step, but no failure is expected.
- **`cargo clippy --workspace --all-targets -- -D warnings`**
  ([`.github/workflows/ci.yml:30`](.github/workflows/ci.yml#L30)) treats every clippy warning as
  a build failure. No clippy lint targets plain string-literal *content* (URL text, punctuation,
  or length) inside a non-doc `format!` call — lints like `clippy::needless_raw_string_hashes` or
  `clippy::doc_markdown` apply to raw-string syntax or `///` doc comments respectively, neither of
  which is in play here (this is an ordinary `"..."` literal inside a runtime `format!`, not a doc
  comment). No applicable lint found. No `clippy.toml` exists in the repo (checked: absent), so
  there's no project-specific lint config that could add one.
- **Em dash / non-ASCII encoding**: the existing messages already contain a literal em dash
  (`—`, U+2014) in both target lines (`"...(over {LONG_FUNCTION_LINES}) — consider..."`,
  `"...(over {MAX_NESTING_DEPTH}) — consider..."`) and the file is plain UTF-8 like every other
  `.rs` file in the repo — Rust source files are UTF-8 by spec, so no new encoding risk from
  reusing the same character or adding ASCII URL/parenthesis text alongside it.
- No `rustfmt.toml` or `.rustfmt.toml` exists in the repo (checked: absent), so rustfmt runs on
  its all-default config — consistent with the `max_width`-ignores-string-literals behavior above.

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
