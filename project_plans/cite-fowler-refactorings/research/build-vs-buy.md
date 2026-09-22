# Build vs. Buy: cite-fowler-refactorings

## Bottom line

There is no build-vs-buy dimension here. This is a two-`format!`-call string
edit; nothing to build, nothing to buy. The three questions below are answered
for completeness, not because any of them surfaced a real decision.

## 1. Lookup table vs. inline string literals

No existing rule-id → Fowler-name/URL table exists, and this change shouldn't
add one.

`RuleMeta` (`src/rules.rs:24-29`) has no `refactoring_name`/`refactoring_url`
field, and `CATALOG` (`src/rules.rs:34-46`) is explicitly dead code today
(`#[allow(dead_code)]` at line 33, doc comment: "not read by the checker logic
itself"). It exists purely as self-documentation for a hypothetical future
`kibitzer rules list` command — findings in `check_declaration`
(`src/rules.rs:818-841`) are built directly as `format!` strings and never
consult `CATALOG` at runtime. Adding a lookup table there would connect two
things that are architecturally disconnected today and buys nothing: the
`format!` calls at lines 826-828 and 836-838 would still need the name/URL
inlined to actually change the finding text, so a table would be a second
source of truth to keep in sync with the string literals, not a replacement
for them.

The codebase's own precedent argues the same way — at least one check already
cites a Fowler refactoring by name directly in its `message` string, inline,
no table: `flag-argument`'s finding at `src/rules.rs:865` reads `"... Fowler's
Remove Flag Argument: split into two named functions or replace with a small
enum"`, a plain inline string literal in the `format!` call, matching exactly
the shape this item's two edits should take.

Precedent is mixed, not unanimous, though: `unreachable-code`'s actual finding
message (`src/rules.rs:791`, `"[unreachable-code] statement is unreachable —
an unconditional \`{label}\` on line {term_line} ends this block first"`)
does *not* cite Fowler at all — only its `CATALOG` description (line 62,
dead code today per below) and the `docs/syntax-rules.md` table row (line 34)
name "Remove Dead Code". So the codebase has one check that names its
refactoring in the live message (`flag-argument`) and one that names it only
in inert doc-only locations (`unreachable-code`). This item's acceptance
criteria explicitly require the message change for both `long-function` and
`deep-nesting` (AC #1-2), so `flag-argument`'s pattern — inline in the
`message`, not just in `CATALOG`/docs — is the one to match. Either way, a
lookup table would be new structure that neither precedent uses.

Out of scope note: the item's own AC #6 flags `CATALOG`'s `description` field
as an optional, separate follow-up ("open question") for consistency — that's
a different, lower-stakes edit (doc-only, no finding-text change) and doesn't
change the answer for the two required message edits.

## 2. URL placement: inline in source vs. `docs/syntax-rules.md`

Keep the URLs inline in the `message` string in `src/rules.rs`; don't move
them to `docs/syntax-rules.md`.

`docs/syntax-rules.md:28-34` already documents both rules in its catalog
table, including the `flag-argument` and `unreachable-code` rows citing
"Fowler's *Remove Flag Argument*" / "Fowler's *Remove Dead Code*" by name —
but neither row links refactoring.com, and the `long-function`/`deep-nesting`
rows (lines 30-31) don't name a refactoring at all yet. So the two edits this
item makes are (a) adding the refactoring name + URL to the `message` text in
`src/rules.rs` per the acceptance criteria, and optionally (b) updating the
`docs/syntax-rules.md` table rows to match, the same way the existing
`flag-argument`/`unreachable-code` rows already name their refactoring. That
second edit isn't in this item's acceptance criteria (AC #6 only mentions
`CATALOG`'s `description`, not `docs/syntax-rules.md`) — it's a reasonable
consistency nice-to-have but not required, and it's a separate file, not a
place to move the URL to instead of having it in the message.

There's a related but distinct piece of prior art:
`docs/refactoring-catalog-analysis.md:24-29` already flags a *duplication*
risk one level up — the refactoring.com catalog *index* page is a JS-rendered
SPA, so that document's own chapter-grouping data was hand-verified against
rendered output rather than scraped. That's a caveat about the *index* page
structure, not about individual catalog-entry URLs like
`/catalog/extractFunction.html`, which are the ones this item cites and which
both returned HTTP 200 when checked directly (see below). It doesn't change
the placement answer, but it's worth knowing the same doc's author already
found refactoring.com's markup non-trivial to scrape/verify at the index
level — supports treating "URLs resolve" as a one-time manual check (as AC #3
already frames it) rather than something to automate.

No mechanism in this codebase makes "duplicating a URL between source and
docs" a real cost worth designing around: there's no existing doc-generation
step that pulls strings out of `src/rules.rs` into `docs/syntax-rules.md` (that
file is hand-maintained prose, confirmed by reading its content directly), so
there's nothing to keep "in sync" beyond eyeballing two files if `docs/syntax-rules.md`
is also touched.

**URL liveness (checked directly, not asserted):**
```
$ curl -s -o /dev/null -w "%{http_code}" https://refactoring.com/catalog/extractFunction.html
200
$ curl -s -o /dev/null -w "%{http_code}" https://refactoring.com/catalog/replaceNestedConditionalWithGuardClauses.html
200
```
Both resolve today (2026-09-22). This satisfies AC #3's "verified before ship"
for the current point in time — it's not a regression test, so it doesn't
need to be encoded as an automated check; re-verify at actual ship time if
that's meaningfully later.

## 3. External dependency, library, or SaaS relevance

None is relevant: the change is two Rust string-literal edits inside
`format!` calls already in `src/rules.rs`, with no new runtime behavior, so
there is nothing for a crate, library, or service to build or replace.
`Cargo.toml` already includes `ureq = "3"` (used today only by
`src/plugin.rs` for plugin fetching, confirmed via `grep -rl ureq src/`), but
even the one place this item touches HTTP — manually checking the two catalog
URLs resolve — is a one-off `curl`/browser check per AC #3, not a runtime
dependency, so `ureq` isn't relevant here either.
