# `pull-up-constructor-body` check — known false positives and tuning

`pull-up-constructor-body` (`src/pull_up_constructor_body.rs`, engine in `src/pull_up_common.rs`) is
Fowler's *Pull Up Constructor Body*: ≥2 direct sibling subtypes have constructors with identical
bodies. Same engine, gates and skips as `pull-up-method` (see its doc), restricted to constructors:
TS/JS/Kotlin a method named `constructor`; Java a method named after its owning type. Go has no
constructor keyword and never fires. Advisory, batch-only, on by default.

Java `constructor_declaration` and Kotlin `secondary_constructor` were not extracted as symbols before
this check; they now are (Kotlin named `constructor`). Kotlin primary constructors are not symbols and are
not checked. A before/after run of 14 model-based checkers on cassandra (Java) and nowinandroid (Kotlin)
showed no change in any output.

## Known false-positive patterns

| Pattern | Status |
|---|---|
| Parameter *types* differ but the text matches (Java) | Fixed: the parameter-list text is part of the match. |
| Body refers to a per-class constant (`DEFAULT_URL`) | Fixed for `UPPER_SNAKE` names. |
| Idiom constructors that are deliberately repeated, e.g. JS "illegal constructor" guards (deno `AbortSignal`/`Window`) | Not filtered; 1 of 15 hits. |
| Vendored/forked copies of one class in different directories | Both copies are reported (each pair of siblings is real). |

## Backtest (release build, final; `kibitzer check architecture pull-up-constructor-body <repo>`)

cassandra 11, deno 2, vscode 2; kubernetes, nowinandroid, servo, ripgrep, stapler-squad 0. All 15 hits
read: 14 plausible (shared `super(...)` plus field assignment), 1 false positive (the JS illegal-constructor
idiom). **1 of 15 (6.7%)**, by one reviewer. The first build (22 hits) had 3 of 11 sampled wrong (27%): different
parameter types, a per-class constant, and the JS idiom. Transcript replay: 0 over 1818 snapshots.

## Suppression

Add `kibitzer:ignore pull-up-constructor-body -- <why>` above the flagged line, or disable the check per repo
(`disabled: ["pull-up-constructor-body"]`); see `docs/suppressing-checks.md`.
