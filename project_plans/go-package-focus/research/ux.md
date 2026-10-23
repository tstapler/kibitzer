# Research: UX — go-package-focus

**N/A — no interactive user-facing surface beyond CLI finding output.**

This feature is a kibitzer architecture check. Its "UX" is the text of the findings it emits, which are rendered via the existing `ArchFinding { file, line, message }` struct and kibitzer's standard output formatter. There is no web UI, interactive flow, or browser surface.

## CLI finding output quality (the relevant UX concern)

The one UX concern is that finding messages must be scannable and actionable:

1. **Named responsibility hypothesis first**: lead with the proposed split name, not the metric. "Candidate split: `protocol` (decoder) vs `storage` (persistence)" reads better than "LCOM4=3, fan_out=12."
2. **Explicit uncertainty markers**: "Insufficient evidence to recommend split — 2 callers found, shared lock detected" is more useful than silence or a noisy false positive.
3. **Coverage disclaimer in finding text**: "Analysis covers 18/23 symbols (5 pruned: generated). Build config: default tags." Users must know the scope of what was analyzed.
4. **Alternatives section in message**: Every finding must include at least one "consider instead" option — reorganize files, in-package type refactor, or keep together — so the finding is never a dead end.
5. **Consistent severity**: advisory-only findings should use `Severity::Advisory` or the equivalent `severity_override`, not `Warning` or `Error`, to avoid alarming users who are not planning a refactor.
