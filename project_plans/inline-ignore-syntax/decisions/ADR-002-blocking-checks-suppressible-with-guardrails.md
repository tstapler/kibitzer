# ADR-002: Blocking checks stay inline-suppressible; guardrails instead of exclusion

**Status**: Accepted (pending implementation)  **Date**: 2026-10-07

## Context
An agent can silence the hook by appending an ignore comment (pitfalls.md item 10). One mitigation is to exempt `Severity::Blocking` checks. The only default-blocking native check is `markdown-link-integrity` (`src/config.rs:782`); a blocking hook exits 2 (`src/hook.rs:195-203`), so an unsuppressible blocking finding that is a genuine misfire would trap the agent in a loop with only `accepted/` as an exit.

## Decision
Blocking checks ARE inline-suppressible. Gaming is bounded instead by: (1) mandatory reason with a minimum-quality rule: at least two words and not equal to the rule id, otherwise `[ignore-syntax]` and no suppression; (2) meta-rules `ignore-syntax`, `unused-ignore`, `ignore-volume`, `blocking-suppressed` can never be ignored inline; (3) advisory `[ignore-volume]` when a file holds 5 or more directives, anchored at the row of the 5th directive (not line 1) so it survives PostToolUse diff-scoping; this guardrail is not on the scope cut list; (4) a repo-wide suppressed-findings count in the `kibitzer run` footer, with the share from blocking checks (the `false-positive` share was dropped in round 3; `list --inline` gives it at review), so 1-4 ignores per file across many files is still visible; (5) a hook advisory `[blocking-suppressed]` when a directive added inside `changed_lines` suppresses a `Severity::Blocking` finding, naming the rule and reason so the suppression shows up for the agent and in the transcript; on a blocking (exit 2) failure the hook also prints every failing `inline-ignore` result to stderr, so a malformed or misplaced directive's repair text reaches the agent when it is blocked; (6) `[unused-ignore]` in hook diff-scoped mode only for directives inside `changed_lines` (an agent's freshly added, misplaced directive), never for directives on unedited lines, because `scope_output_to_changed_lines` (`src/check.rs:473`) would make every ignore outside the edit look unused; this hook advisory is core. The unscoped `kibitzer run` variant is a pre-declared first cut and may ship as a follow-up PR.

## Alternatives rejected
- Exempt Blocking: breaks the main success metric on the one default blocking check.
- Separate stronger marker for Blocking: adds a third form agents must learn; revisit only if backtests show abuse.

## Consequences
If transcripts show ignore-spam, the next lever is a stricter reason check (beyond the two-word floor) or a lower `ignore-volume` threshold, not a syntax change. The two-word floor stops boilerplate, not a determined lie; the footer counts and the `[blocking-suppressed]` advisory are what make a lie reviewable. Round 3 (2026-10-07): the two-word floor is kept deliberately, with the 30-day sample of reasons deciding whether to raise it (more than 10% boilerplate passing the floor triggers the stricter check). Updated 2026-10-07 after the pre-mortem (failure #2, P1).
