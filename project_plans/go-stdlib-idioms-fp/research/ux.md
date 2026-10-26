# UX Research: go-stdlib-idioms false-positive reduction

## No direct UI/UX surface

kibitzer is a CLI tool and editor hook — there is no GUI, web UI, or interactive
surface. This change (reducing false positives in the `go-stdlib-idioms` checker)
has no UI/UX surface to design or evaluate.

## Developer UX impact

The relevant "user" is a developer whose editor (via `kibitzer run` or the hook)
displays checker findings inline.

**Before:** The `go-stdlib-idioms` checker fires on idiomatic Go patterns that are
technically valid alternatives to their stdlib equivalents (e.g. a hand-rolled loop
that does what `strings.Builder` or `slices.Contains` would do) even when the
author's choice was deliberate and the code is not objectively worse. The developer
sees a noisy finding, adds it to their mental noise filter, or adds a
`kibitzer:ignore` comment that obscures real issues.

**After:** The checker fires only when a stdlib replacement is unambiguously
preferable (correctness, performance, or readability benefit is clear; no
plausible stylistic reason to keep the hand-rolled form). The developer sees fewer
findings per diff; the ones that do appear carry higher signal-to-noise, and
editors that show inline annotations are less cluttered.

The change is invisible to developers who never triggered the false-positive
patterns; for those who did, the improvement is passive — they simply stop seeing
the spurious annotation.

## Finding message guidance for future suppression-reason explanations

If a future release adds per-finding suppression-reason text (e.g. "suppressed
because: pattern matched idiomatic alternative"), the message for a
`go-stdlib-idioms` finding should follow this structure:

```
go-stdlib-idioms: prefer <stdlib.Function> over <hand-rolled pattern>
  → replaces <N>-line construct; <brief rationale, e.g. "avoids allocation" or "nil-safe">
```

When the checker elects *not* to fire on a borderline case, no message is emitted.
If a suppression-reason log is ever added (debug mode), the log line should state
the rule name, the matched pattern, and the specific heuristic that cleared it
(e.g. "skipped: receiver type implements io.Writer; Builder replacement inapplicable").

Keep finding messages short: one line identifying the preferred alternative, one
optional continuation line with the rationale. Do not include stack traces,
AST node descriptions, or multi-paragraph explanations — those belong in
`kibitzer check explain go-stdlib-idioms` output, not in the inline finding.
