# Check ideas

Candidates for new kibitzer checks, produced by the `kibitzer-check-brainstorm`
skill. See that skill for methodology.

## Methodology note — evidence search came back empty

Per the skill's Step 1, the first pass was full-corpus mining of
`~/.claude/projects/*/*.jsonl` (and subagent/workflow sub-transcripts) for
user turns that read as corrections of agent mistakes, using two approaches:

- a narrow correction-phrase regex restricted to the last 3 days of sessions
  (`/tmp/kibitzer_corrections_3d.tsv`, 40 matches on full triage)
- a broader, higher-specificity phrase set with no date restriction, run
  against an 87-file `rg`-shortlisted candidate set
  (`/tmp/kibitzer_tight_extracted.tsv`, 29 matches)

Every match in both passes was a false positive on manual read-through. The
40-row 3-day file broke down as:

- ~30 rows of auto-generated context-compaction summary boilerplate ("This
  session is being continued from a previous conversation... Summary: 1.
  Primary Request and Intent:")
- ~6 rows of subagent/skill task-prompt briefs (bundled-skill reference text,
  "Update Config Skill" instructions, a `<task-notification>` block) — task
  instructions, not user corrections
- a handful of genuine bug-fix requests ("Please fix all of the failures...",
  "Okay please fix the attribution logic", "Please fix this: [MCP error]") —
  real user asks, but *not* corrections of something the agent did wrong;
  they're initial bug reports
- one design-doc session
  (`-Users-tstapler-Documents-design-docs/dcb5a7eb-7e8e-4ef4-93bf-2e37a7ccbbaf.jsonl`)
  where "wrong"/"revert"-family words matched inside the *document content*
  being edited (a milestone-planning doc describing something as "wrong",
  code comments about a regex test being "wrong") rather than in the user
  correcting the agent

None of the 69 combined matches across both passes is a human turn correcting
a specific agent mistake mid-session. This is consistent with the existing
`docs/checking-invocations.md` lesson that raw text matching over these
transcripts overcounts and conflates unrelated content — it applies to
correction-language mining just as much as it applies to tool-invocation
mining.

**Result: zero evidenced recurring-mistake patterns found** from free-text
correction mining within the effort spent on this pass. The candidates below
are therefore **not** backed by cited transcript occurrences (Step 2 of the
skill was not reached) and must not be read with the same confidence as an
evidenced finding — they're placed in the untested section per Step 5.

The skill's own suggested next signal — structural detection of
Edit/Write immediately followed by another Edit to the same file within the
same or next turn, as a proxy for "got it wrong on the first try" — has not
yet been implemented or run. That's the natural next step before trusting
this list further.

## Evidenced candidates

None yet, after triaging all 69 correction-phrase matches across both mining
passes (see methodology note above) — no pattern reached the skill's Step 2
bar of 2+ independent occurrences of the same recurring agent mistake. Re-run
this skill with the structural edit-churn signal (Edit/Write immediately
followed by another Edit to the same file), or a narrower single-repo scope,
before populating this section.

## Untested ideas (no observed occurrences yet)

These are plausible check ideas given kibitzer's existing model (config-driven
shell command per check, run against `{file}`, scoped by glob and trigger,
advisory or blocking, with the git-HEAD baseline downgrade in
[`src/check.rs`](../src/check.rs)) — not findings. Do not implement any of
these on the strength of this list alone; validate against real occurrences
first.

- **Leftover debug output** — `console.log`/`dbg!`/`fmt.Println`-style debug
  statements left in committed code. Feasible as a whole-file regex/AST scan
  per language; same shape as the existing primitive-obsession check.
- **Dead/unused imports after an edit** — likely needs a language-aware tool
  (`goimports -l`, `ruff`, `eslint --rule no-unused-vars`) shelled out per
  file rather than a new regex, so may be better served by wiring an existing
  linter into kibitzer's `Check.command` config than by writing a new check.
- **Secret-looking strings introduced by an edit** — diff-aware is important
  here specifically (a pre-existing secret in the repo is a different, likely
  bigger, problem than one just introduced); would need the git-HEAD
  baseline comparison kibitzer already has, applied to a regex/entropy scan
  instead of an exit-code check.

None of these have a confirmed transcript occurrence backing them yet.
