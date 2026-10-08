# UX research: inline-ignore-syntax

Consumers: mostly AI agents reading hook `additionalContext` / MCP output; secondarily developers and maintainers. Accessibility N/A.

## Job to be done (5)

- Agent: "When a finding fires that I (or the user) already judged, let me record that judgment next to the code in one edit, so the hook stops re-feeding it and I stop spending tokens re-triaging."
- Developer: "Make accepted tradeoffs visible in the diff with a reason, without a separate JSON file."
- Maintainer: "Give me a greppable worklist of checker misfires so each becomes a regression test."

The ignore is a *communication* artifact (to the next reader) as much as a filter.

## How findings render today (1)

VERIFIED by reading source:

- Hook advisory (`src/hook.rs:207-219`): one line per failed check, `"{check_name}: {describe()}"`, then a fixed ~70-word footer linking `reporting-false-positives.md` and `suppressing-checks.md`. Appended once per hook call, not per finding. Blocking mode (`hook.rs:195-203`) prints a similar one-line link to `suppressing-checks.md` on stderr.
- MCP `run_checks` (`src/mcp.rs:840-852`): `[advisory|blocking] file:line: message` per finding. No per-finding hint. The server `instructions` (`mcp.rs:1439`) tell agents to call `report_false_positive`; nothing mentions how to suppress.
- Finding messages from `syntax-rules-*`/`comment-quality-*` self-prefix `[rule-id]` (per `docs/accepting-findings.md`); single-rule checkers don't, so the rule id is not always in the message.

Gap: the only teaching surface is a footer pointing at docs (an agent must fetch a URL to learn the syntax), and it points at the *wrong* levers for an accepted tradeoff.

### Recommendation: teach once, not per finding

Token math (rough, 4 chars per token): a per-finding hint like `  // kibitzer:ignore <rule> -- <reason>` is about 40 chars = about 10 tokens. At 5 findings that is 50 tokens, at 30 findings 300, and it repeats every edit while a finding persists. The footer is one fixed block per call.

- Put the copy-pasteable syntax in the existing footer, once, replacing the URL-only pointer. Example (about 45 tokens, replaces some of the current 90):
  `To dismiss a finding you've judged: add a comment on/above the flagged line: kibitzer:ignore <rule> -- <why>  (use kibitzer:false-positive <rule> -- <why> if the check misfired).`
  Use the file's own comment leader if known (the hook knows the edited file's extension), so the sample is literally pasteable: `// ...`, `# ...`, `<!-- ... -->`.
- Make `<rule>` concrete only where cheap: because the id is not always in the message, the one place a per-finding token earns its cost is a finding whose rule id isn't self-prefixed. Option: have the renderer always show `[rule-id]` (it is already there for most), so the footer can say "use the bracketed id". Do not add a per-finding hint.
- MCP: add one sentence to server `instructions` (`mcp.rs:1439`) next to the false-positive sentence, and one trailing line in the `run_checks` output only when findings > 0. Same one-time cost.
- Agents tend to follow an in-context example over a doc link; the link costs a tool call (more tokens than the example).
- Keep the footer short on repeat: if the same call has no findings, emit nothing (already true).

## Agent mental model (2)

INFERRED (from common agent behavior; not measured here):

- Agents know `# noqa`, `// eslint-disable-next-line`, `// nolint`, `@SuppressWarnings`. They will reach for those spellings first and may write `kibitzer: ignore`, `kibitzer-disable`, or put it after the code on the same line. Design implication: accept a small set of tolerant variants (space after colon, either same line or the line above), and when a near-miss is seen (`kibitzer:disable`, `kibitzer: ignore`), report it as a malformed-ignore finding with the exact correct form rather than silently not matching.
- Agents expect `-- reason` or `: reason`, and expect a rule list (`a,b`). Decide this up front; if comma lists are unsupported, say so in the malformed error.
- Agents suppress to make output go away (a known failure mode: ignore-spam). The mandatory reason is the guardrail; add a lint-like check that a reason is not boilerplate only if backtesting shows spam (UNVERIFIED need).
- Prefer "line above" as the documented form: same-line trailing comments are awkward in markdown/HTML and in Rust format-on-save, and "above" survives line reflow. Same-line can be accepted silently as an alias.
- Ignore comments must not trip `commented-out-code`/`verbose-comment`/`over-commented` (requirements rabbit hole); otherwise the agent sees a new finding caused by its own fix and loops.

## Malformed ignore UX (3)

Existing precedent: a malformed `.kibitzer/accepted/` entry is a hard error, not a no-op (`docs/accepting-findings.md:60-62`). Keep that stance, but make the message self-repairing, since the reader is an agent that will act on it literally.

Proposed messages (one finding each, reported at the comment's own line, advisory unless the check is blocking, rule id `ignore-syntax`):

| Case | Message |
|---|---|
| Missing reason | `[ignore-syntax] kibitzer:ignore flag-argument has no reason. Write: kibitzer:ignore flag-argument -- <why this is acceptable>` |
| Missing rule | `[ignore-syntax] kibitzer:ignore needs a rule id. Write: kibitzer:ignore <rule> -- <why>` (list the rules that fired on the next line if any, which is the cheap "did you mean") |
| Unknown rule | `[ignore-syntax] unknown rule 'flag-arg' — did you mean 'flag-argument'?` (edit-distance suggestion; otherwise point to `kibitzer check list`) |
| Near-miss marker | `[ignore-syntax] 'kibitzer: ignore' not recognized; use 'kibitzer:ignore'` |
| Unused ignore (open question) | `[unused-ignore] kibitzer:ignore flag-argument suppresses nothing — remove it` |

Notes:

- A malformed ignore must NOT suppress (fail closed), and must show the original finding too, so the agent sees both the problem and the fix.
- Unknown-rule validation needs the set of rule ids; checkers that don't self-prefix use the checker name (known from `kibitzer check list`). Validate against rule ids plus checker names; if a rule list is incomplete, prefer a warning-level message to a hard error to avoid blocking on a registry gap.
- Recommend reporting unused ignores, advisory only, scoped to the file being checked (stale ignores are the lint-debt that a content-hash-free design otherwise invites). It also gives the maintainer a signal when a checker was fixed and the misfire ignore can be deleted.

## False-positive flow: tradeoff vs misfire (4)

Distinction already drawn in docs: a misfire is "doesn't apply to the code" (`reporting-false-positives.md`); an accepted tradeoff is a "genuine, correctly-flagged finding" kept on purpose (`accepting-findings.md`). Agents conflate them (the current hook footer conflates them too by linking both docs back to back).

Recommendation: two markers, not one marker plus an optional tag, because the agent picks a marker at write time and a separate verb is a more forceful decision prompt than an optional flag it may omit.

- `kibitzer:ignore <rule> -- <reason>`: genuine finding, accepted tradeoff. Reason = why the cost is acceptable.
- `kibitzer:false-positive <rule> -- <reason>`: checker is wrong here. Reason = why the pattern doesn't apply. Both suppress identically.

Maintainer worklist: `kibitzer check false-positives` already exists as the queue review command (`src/main.rs:299-317`) for MCP-filed reports; add inline markers as a second source there (for example `kibitzer check false-positives --inline [path]`) listing `file:line  rule  reason  (+ the flagged line's text)`, grouped by rule, since one rule's misfires cluster into one test fixture. Output should be paste-ready for `docs/<checker>-false-positives.md`. This answers the open question: put it under the existing command so there is one place for maintainers to look.

Guardrail: the footer should say a misfire marker is still a suppression; it does not replace filing via `report_false_positive` when the checker is wrong in general (the report carries `what_changed`/`mechanism`, which a one-line reason cannot). Cheap bridge: when a `false-positive` marker is written, the hook (or `check false-positives --inline`) can print a one-line nudge to also file the report. Do not auto-file.

## Summary of decisions to carry into the plan

1. Teach in the footer once, with a language-appropriate comment leader; no per-finding hint.
2. Two markers (`ignore`, `false-positive`), "line above" documented, same-line accepted.
3. Malformed/unknown/near-miss are findings with exact repair text; fail closed; unused-ignore advisory.
4. Maintainer worklist via `kibitzer check false-positives --inline`.

Open items not resolved by source reading: agent behavior claims in section 2 are INFERRED; no transcript backtest was run.
