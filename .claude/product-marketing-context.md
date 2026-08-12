# Product Marketing Context
*Type: open-source*
*Last updated: 2026-08-12*

## Project Overview
**One-liner:** Advisory, diff-aware code and doc checks built for how AI agents actually edit — locally, in CI, or wired into Claude Code.
**What it does:** kibitzer runs targeted checks (Go primitive-obsession, Markdown link integrity, and more over time) scoped to the files that actually changed, not a full-repo scan. A caching daemon keeps repeat checks near-instant. It ships as a CLI, an MCP server, and a Claude Code `PostToolUse` hook — Claude Code is the first integration, not the ceiling.
**Category:** AI-native code quality tool / diff-aware linter framework
**Type:** CLI + MCP server + agent hook (one binary, three surfaces)
**License:** MIT

## Audience
**Primary users:** Developers and teams using AI coding agents (Claude Code today, more agent runtimes over time) who want fast, low-noise quality feedback inside the edit loop itself, not just at PR time.
**Secondary users:** Teams who want diff-scoped local/CI checks even without an agent in the loop — the CLI and CI surfaces stand on their own.
**Contributors:** Rust developers interested in tree-sitter-based checks, and anyone who wants to add a check for a language/pattern kibitzer doesn't cover yet (today: Go primitive-obsession, Markdown links).
**Not for:** Teams wanting a SonarQube-scale static-analysis suite or a security scanner — kibitzer is intentionally narrow: fast, diff-scoped, severity-aware checks, not exhaustive analysis.

## Problem & Differentiation
**The problem:** Traditional linters run full-repo and assume a human-paced edit/commit/CI cycle. That's a bad fit for AI agents, which edit fast, iterate mid-thought, and need signal *before* a PR exists — a full lint pass mid-edit is slow and drowns real issues in mid-edit false positives.
**Alternatives fall short because:** `golangci-lint`/`markdownlint`-style tools scan everything, every time, with no notion of "what just changed" or "is this still being edited." Raw shell hooks in `.claude/settings.json` have no caching, no severity model, and no portability to CI.
**Core philosophy:** Diff-aware + severity-aware (advisory vs. blocking, matching the `Severity` enum in the code) + fast enough to run inline in an agent's loop, with the *same* checks portable to a human's pre-commit hook or a CI gate.
**Word-of-mouth pitch:** "It's the code-quality layer built for how AI agents actually edit — advisory while you're mid-thought, enforceable once it lands."

## Brand Voice
**Personality:** precise, dry, unglamorous, trustworthy, a little skeptical
**Technical depth:** expert-first (assumes familiarity with agent hooks, tree-sitter, Rust)
**Writing style:** terse and precise, caveat-heavy — explicit about false positives and what counts as real evidence (see `docs/checking-invocations.md`)
**Use:** "advisory", "diff-aware", "scoped", "false positive", "severity"
**Avoid:** "AI-powered", "seamless", "magic", "revolutionize", "next-gen"
**Voice example:** "A raw string match on the word `kibitzer` in a transcript is not evidence of a real invocation."

## Visual Direction
**Color mood:** dark + technical, with a functional two-color accent split that mirrors the product's own severity model
**Colors:**
- Base: `#0d0f12` (near-black, terminal)
- Text: `#d8dee9` (soft off-white)
- Advisory: `#e0a030` (amber — maps to `Severity::Advisory`)
- Blocking: `#c0392b` (red — maps to `Severity::Blocking`)
- Muted: `#4b5563` (slate — secondary text, borders)
**Typography:** monospace throughout (JetBrains Mono / Berkeley Mono style) — CLI output, docs, and any future site
**Aesthetic:** inline GitHub PR review comments (the annotation, not the diff) crossed with ripgrep's terminal output style
**Logo:** icon-only — a small speech-bubble/annotation mark, like a review comment pinned to a line of code. Leans into the name itself: a kibitzer heckles from the sidelines. Not yet designed — hand to `ui-logo-designer` when ready.

## Adoption Goals
**Primary metric:** inferred, needs confirmation — proposing *integrations adopted* (agent runtimes + CI usage) over vanity metrics like stars, since the ambition is being the quality layer other tools wire into, not a standalone destination.
**Discovery path:** GitHub repo + Homebrew tap today; no other channel yet. Broadening beyond Claude Code will need a discovery path that isn't Claude-Code-specific (crates.io listing, a "works with any agent" positioning in the README).
**Trust signals:** MIT license, small transparent codebase, explicit false-positive-handling docs, and — once broadened — evidence it works the same in CI as it does in an agent loop.
**Adoption barrier:** currently narrow (Claude Code specifically); broadening to "general-purpose, AI-native, local-or-CI" removes that barrier but needs the README/positioning to catch up first.
**"Aha" moment:** the moment someone runs the *same* check locally, in an agent hook, and in CI and gets consistent advisory/blocking behavior in all three — proving it's one quality layer, not three separate configs.

## Key Messages
**Headline:** "Advisory checks scoped to exactly what changed — in your agent, your terminal, or your CI."
**Supporting:**
- Diff-aware, not full-repo — checks only what actually changed
- Severity-aware: advisory in the loop, blocking where it matters, same model everywhere it runs
- AI-native by design, not agent-locked — Claude Code is the first integration, not a hard dependency
**CTA:** install and wire into `.claude/settings.json` as a `PostToolUse` hook, or run `kibitzer run` locally/in CI

## GitHub Presence
**README purpose:** quick start (current README leads with install/usage for the CLI/hook/MCP surfaces)
**Social proof:** none yet
**Contribution posture:** inferred, needs confirmation — proposing "welcoming contributions," especially new checks and language coverage beyond Go, since the broadened ambition needs more check types than one person will write alone.
**Topics/tags:** `claude-code`, `ai-agents`, `rust`, `cli`, `code-quality`, `ci`
