# Build vs. buy: citing Fowler refactorings in `long-function`/`deep-nesting` messages

## 1. Abstraction level: literal `format!` strings vs. a shared const map

**Finding: use plain literal strings, matching the existing precedent exactly. No new
abstraction.**

The codebase already has two instances of this exact pattern, and neither uses a shared
constant — the citation is inlined twice, independently, at the two places a finding's
message is built:

- `RuleMeta.description` in the `CATALOG` array (self-documentation only, not read by
  checker logic — `src/rules.rs:23` has `#[allow(dead_code)]` on the struct precisely
  because nothing consumes it at runtime):
  - `src/rules.rs:56` (`flag-argument`): `"... — Fowler's Remove Flag Argument."`
  - `src/rules.rs:62` (`unreachable-code`): `"... — Fowler's Remove Dead Code."`
- The actual `Finding.message` `format!` string the checker emits at runtime:
  - `src/rules.rs:865` (`flag-argument`): `"[flag-argument] boolean parameter \`{name}\` is
    branched on directly in the body — Fowler's Remove Flag Argument: split into two named
    functions or replace with a small enum"`
  - `src/rules.rs:791` (`unreachable-code`): message ends `"— an unconditional \`{label}\` on
    line {term_line} ends this block first"` (this one doesn't carry the citation in the
    runtime message, only in `CATALOG.description` — so precedent is actually inconsistent
    on *whether* the runtime message repeats the catalog citation, not just on mechanism).

Neither existing instance references refactoring.com URLs at all — both cite only the
refactoring's *name* ("Remove Flag Argument", "Remove Dead Code"), no link. The requirements
for this task ask for name **and** URL, which is new territory these two precedents don't
cover, but the *mechanism* (inline string literal, duplicated per call site) is established
twice over.

With the task scope being exactly 2 checks (`long-function`, `deep-nesting`), a shared
`const`/table (e.g. `HashMap<&str, (&str, &str)>` from check-id to name+URL, or a
`FOWLER_CITATIONS: &[(&str, &str, &str)]`) would be introducing a lookup abstraction that:
- has no existing precedent in this file (the 2 existing citations don't share one),
- adds a runtime or const-eval indirection for what is otherwise two `format!` calls,
- only pays off once there are enough check-id → citation pairs that hand-duplication
  becomes error-prone — not the case at n=2, and even at n=4 (all four Fowler-citing
  checks) it's borderline.

**Recommendation:** follow precedent directly — extend the two `format!` strings in place
(the runtime messages at `src/rules.rs:827` and `:837`, mirroring the phrasing style
already used at `:865`), and update the matching `CATALOG.description` entries at
`src/rules.rs:38` and `:44`. No new constant, no new module, no new dependency. If a third
or fourth check later needs the same treatment, that's the natural point to reconsider a
shared table — not now.

## 2. Link-validation crate to reuse for CI/test URL checking

**Finding: no existing dependency does this, and the codebase has a deliberate,
documented boundary against doing it.**

`Cargo.toml` (`src/../Cargo.toml:17-38`) has no linkcheck/URL-validation crate. The one
HTTP-capable dependency, `ureq = "3"` (`Cargo.toml:37`), is used for two unrelated purposes
found in `src/plugin.rs` (fetching/resolving redirects for plugin release artifacts) — not
wired into any check.

More importantly, the repo already has a check whose entire job is link integrity —
`markdown-link-integrity` (`src/markdown_link_integrity.rs`, referenced in this repo's
`CLAUDE.md` default-checks list) — and it explicitly **excludes** `http(s)://` targets by
design:

```
src/markdown_link_integrity.rs:347: /// for a live target (or an `http(s)://` URL, never followed). `target_cache` memoizes a
src/markdown_link_integrity.rs:357:     if target.starts_with("http://") || target.starts_with("https://") {
```

i.e., the function that resolves a link target short-circuits on `http(s)://` and treats it
as always-valid without an actual network fetch — a deliberate scope boundary (no
network-dependent, flaky-in-CI checks), not an oversight. Reusing or extending this checker
to validate the two Fowler URLs would mean reversing that documented design decision for a
one-time, non-recurring need (these two URLs don't change once cited).

**Recommendation:** no crate to add, no checker to extend. The acceptance criteria's
one-time manual `curl -sI` against the two refactoring.com URLs is the right-sized approach
— consistent with the codebase's existing choice not to do live HTTP validation inside any
check, and proportionate to a citation that, once correct, has no ongoing drift risk (unlike
markdown docs, which change over time and motivate the existing internal-link checker).

## 3. Prior art: linters that cite a pattern catalog in-message

Both of kibitzer's own existing precedents (`flag-argument`, `unreachable-code`) already
demonstrate the target style in this codebase — `"<what's wrong> — Fowler's <Refactoring
Name>[: <optional next step>]"` — and are the most relevant, in-repo prior art to imitate;
no external research was needed to find a model, since the codebase already has one twice
over.

For external corroboration of the general pattern (name-checking a canonical
catalog/source in a lint message) common examples worth knowing about, by
reputation/memory rather than freshly fetched today:
- **Clippy** (`rustc`'s lint suite) commonly appends a `note:` / `help:` suggesting the
  idiomatic replacement, and many lints link out to `rust-lang/rust-clippy`'s own docs
  page per-lint (e.g. `clippy::needless_return`), but does not typically cite an external
  book/catalog by name inline — the citation lives in the lint's own doc page, not the
  message string.
- **ESLint** rules occasionally reference external sources (e.g.
  `eslint-plugin-import`'s messages), but again mostly link to their own rule docs rather
  than a third-party catalog with an inline name+URL.

Neither is a closer match than kibitzer's own two existing Fowler citations. The two
in-repo instances (`src/rules.rs:56,62` for description; `:865` for the runtime message)
are the style to copy verbatim, extended with a URL suffix per the requirements — there's
no external phrasing worth importing over what's already established locally.

## Summary

1. No crate needed for the citation itself — use literal `format!` strings, following the
   two existing precedents at `src/rules.rs:56,62` (`CATALOG.description`) and `:865`
   (runtime message), not a new shared constant. At n=2 checks, a lookup table is
   premature abstraction the codebase's own precedent doesn't support.
2. No link-validation crate to add or reuse — `markdown-link-integrity`
   (`src/markdown_link_integrity.rs:347,357`) deliberately never follows `http(s)://`
   URLs, so extending it would reverse a documented design choice for a one-time need.
   The acceptance criteria's manual `curl -sI` is correctly scoped.
3. No external linter's phrasing is a better model than kibitzer's own existing
   `flag-argument`/`unreachable-code` citations — copy that style directly.
