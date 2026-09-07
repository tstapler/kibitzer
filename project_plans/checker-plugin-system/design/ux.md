# UX Design: checker-plugin-system

**Phase**: 3 (design) — companion to `implementation/plan.md`
**Date**: 2026-09-06
**Scope note**: 100% CLI/agent-facing, no GUI. Per the design brief, the one surface a user
directly drives through a multi-step interaction (`kibitzer plugin install`) gets the full
wireframe/flow/error-table treatment; every other surface is non-interactive output (a
one-shot command result, or a machine-readable field an MCP client/hook reads) and gets the
condensed treatment: one representative sample + 3-5 acceptance bullets.

All command names, flags, message substrings, and output shapes below are taken directly from
`implementation/plan.md`'s Domain Glossary, Acceptance Criteria, and Task bodies — not invented.
Where `plan.md` specifies only a required substring (e.g. `"checksum mismatch"`,
`"not an allowed host"`) rather than the full message, the surrounding text is drafted here
in kibitzer's existing `[kibitzer] ...`/`anyhow::Context` voice
([src/main.rs:199,201,207,209](https://github.com/tstapler/kibitzer/blob/master/src/main.rs#L199),
[src/main.rs:232](https://github.com/tstapler/kibitzer/blob/master/src/main.rs#L232)) and flagged
"(drafted)" so implementation can treat it as a starting point, not a locked string, while the
bolded substring itself is the contract to preserve.

---

## Surfaces inventory (Step 1)

| # | Surface | Treatment | Plan reference |
|---|---------|-----------|-----------------|
| 1 | `kibitzer plugin install <name> --source <source> [--force]` | **Full** (interactive, multi-step, user-initiated) | Story 3.2.1, 3.2.2 |
| 2 | `kibitzer plugin list` | Condensed | Story 3.1.1, 3.3.1 |
| 3 | `kibitzer plugin status [<name>]` | Condensed | Story 3.3.1 |
| 4 | `kibitzer plugin remove <name> [--force]` | Condensed | Story 3.3.2 |
| 5 | MCP `list_checks` — plugin-not-installed tag | Condensed | Story 4.3.1, Task 4.3.1d |
| 6 | `run_checks`/hook — `[skipped]` line for a missing-plugin check | Condensed | Story 4.3.1, Task 4.3.1c |

Six surfaces total. Surface 1 alone carries seven distinct error/edge states (see its own
table); surfaces 2-4 carry a success and at least one degraded/error state each; surfaces 5-6
are single-shape outputs consumed by an agent, not a human, so they get one sample each rather
than a flow.

---

## Surface 1 (full treatment): `kibitzer plugin install <name> --source <source>`

### Interaction model — ASCII flow

Plan.md's actual pipeline order (`install_plugin`, Story 3.2.1/3.2.2) checks version
compatibility **before** downloading the binary — a deliberate fail-fast choice so a
version-incompatible install never touches the network for the payload, only for the small
manifest. That differs from a naive "download then verify" order and is preserved below rather
than reordered to a more generic-looking sequence.

```
┌────────────────────────────────────────────────────────────────────────────────┐
│  $ kibitzer plugin install <name> --source <source> [--force]                  │
└───────────────────────────────────┬────────────────────────────────────────────┘
                                     ▼
                     ┌───────────────────────────────┐
                     │ 1. fetch_manifest(source)      │──fail──▶ E1 not found / unreadable
                     │    (local path OR https://,    │──fail──▶ E2 host not on allowlist
                     │     allowlist-checked)          │
                     └───────────────┬───────────────┘
                                     ▼ Ok(PluginManifest)
                     ┌───────────────────────────────┐
                     │ 1b. manifest.name == CLI name? │──mismatch──▶ E7 manifest name mismatch
                     └───────────────┬───────────────┘
                                     ▼ match
                     ┌───────────────────────────────┐
                     │ 2. version_meets_minimum(      │──false──▶ E3 version incompatible
                     │    manifest.min_kibitzer_ver,  │           (no download attempted)
                     │    CARGO_PKG_VERSION)          │
                     └───────────────┬───────────────┘
                                     ▼ compatible
                     ┌───────────────────────────────┐
                     │ 3. already installed at this   │──same version, no --force──▶
                     │    exact version?              │   OK-idempotent: exit 0, no changes
                     └───────────────┬───────────────┘
                                     ▼ new / different version, or --force
                     ┌───────────────────────────────┐
                     │ 4. current_target_triple() +   │──unsupported──▶ E6 unsupported platform
                     │    fetch_target_bytes(target)  │──fail──▶ E4 download failed
                     │    (current-triple lookup)     │           (nothing written)
                     └───────────────┬───────────────┘
                                     ▼ Ok(bytes)
                     ┌───────────────────────────────┐
                     │ 5. sha256(bytes) == target.    │──mismatch──▶ E5 checksum mismatch
                     │    sha256 ?                    │              (nothing kept)
                     └───────────────┬───────────────┘
                                     ▼ match
                     ┌───────────────────────────────┐
                     │ 6. write temp → rename → chmod │
                     │    +x into default_plugin_dir()│
                     └───────────────┬───────────────┘
                                     ▼
                     ┌───────────────────────────────┐
                     │ 7. Registry::load → upsert →   │
                     │    Registry::save               │
                     └───────────────┬───────────────┘
                                     ▼
                     ┌───────────────────────────────┐
                     │ 8. print success + disclosure  │
                     │    line, exit 0                │
                     └───────────────────────────────┘
```

### Interaction flow — what the user does, what the system says

**Happy path, first install:**

```
$ kibitzer plugin install kibitzer-stub-plugin --source ./manifest.json
[kibitzer] installed kibitzer-stub-plugin v0.1.0 -> /home/tstapler/.local/share/kibitzer/plugins/kibitzer-stub-plugin/kibitzer-stub-plugin
[kibitzer] kibitzer-stub-plugin now runs as a check in every repo on this machine.
  - disable it for one repo: add "disabled": ["kibitzer-stub-plugin"] to that repo's .claude/inspect.json
  - remove it everywhere:    kibitzer plugin remove kibitzer-stub-plugin
```
Exit `0`. One command, no prompts — matches Task 3.2.2d's disclosure-mitigation requirement
from the Step 0.5 creative pass (the auto-inject design means install is a machine-wide,
silent-by-default change, so the tool states that consequence back to the user immediately
rather than leaving them to discover it later).

**Happy path, re-running install (idempotent, rustup model per `research/ux.md` §4):**

```
$ kibitzer plugin install kibitzer-stub-plugin --source ./manifest.json
[kibitzer] kibitzer-stub-plugin is already installed (v0.1.0) — no changes made
```
Exit `0`. No download, no registry write (Story 3.2.2's third acceptance criterion).

**Happy path, forced re-install of the same version (`--force`):**

```
$ kibitzer plugin install kibitzer-stub-plugin --source ./manifest.json --force
[kibitzer] kibitzer-stub-plugin v0.1.0 already installed — reinstalling due to --force
[kibitzer] installed kibitzer-stub-plugin v0.1.0 -> /home/tstapler/.local/share/kibitzer/plugins/kibitzer-stub-plugin/kibitzer-stub-plugin
```
Exit `0`. Unlike the unqualified no-op above, `--force` restates the consequence (a real
re-fetch/re-verify/re-place happens) instead of printing the same "no changes made" line —
mirroring this doc's principle for `remove --force` (Surface 4): a forced action confirms what
it actually did, not what an uneventful run would have printed (`plan.md` Task 3.2.2a).

**Verifying the result — the natural next command:**

```
$ kibitzer plugin list
kibitzer-stub-plugin v0.1.0 — /home/tstapler/.local/share/kibitzer/plugins/kibitzer-stub-plugin/kibitzer-stub-plugin
```

### Error and edge-case handling

| Case | Trigger point | Terminal output | Exit | State left behind |
|------|---------------|------------------|------|--------------------|
| **E1 — Not found / unreadable source** | Step 1 — `fetch_manifest` | `error: failed to read plugin manifest from './manifest.json': No such file or directory (os error 2)` (drafted; wraps the underlying `std::fs`/`ureq` error via `anyhow::Context`, same convention as `src/main.rs:232`'s `"no checker named '{name}' registered"`) | non-zero | Nothing written; registry untouched. |
| **E2 — Host not on allowlist** | Step 1 — `fetch_manifest`, HTTPS branch | `error: 'evil.example.com' is **not an allowed host** (expected github.com, api.github.com, or a *.githubusercontent.com subdomain)` — required substring per Story 3.2.1's AC | non-zero | No `ureq::get` call is ever made (verified by the AC itself) — refused before any network I/O. |
| **E7 — Manifest name mismatch** | Step 1b — manifest-name check | `error: manifest name does not match: CLI name 'foo', manifest name 'bar' — refusing to install; re-check --source or the plugin name you intended` (drafted around the required `"manifest name does not match"` substring, Story 3.2.2's AC) | non-zero | Nothing written; `Registry::load(default_registry_path())` has zero plugins named the CLI-supplied name — refused before the version-compat check, any download, or any Registry write (Story 3.2.2's AC). |
| **E3 — Version incompatible** | Step 2 — `version_meets_minimum` | `error: kibitzer-stub-plugin v0.3.0 **requires kibitzer** >= 99.0.0 (current: 0.1.13) — upgrade kibitzer, or point --source at an older compatible release's manifest` (drafted around the required `"requires kibitzer"` substring, Story 3.2.2's AC) | non-zero | No binary fetch is attempted (AC: "no `fetch_target_bytes` call is ever made") — fail-fast before touching the network for the payload. |
| **E4 — Download failed (network)** | Step 4 — `fetch_target_bytes` | `error: failed to download kibitzer-stub-plugin v0.1.0 (target x86_64-unknown-linux-gnu) from <url>: <transport error> — no files were written; try again once network access is available` (drafted, following `research/ux.md` §4's recommendation and the write-temp-then-rename design in Task 3.2.2c, which guarantees nothing partial lands in `default_plugin_dir()`) | non-zero | Nothing written — same guarantee `gh extension install`'s own bug reports show is worth stating explicitly (`research/ux.md` §1). |
| **E6 — Unsupported platform** | Step 4 — `current_target_triple` | `error: kibitzer-stub-plugin has no published artifact for your platform (<arch>-<os>, resolved to no known target triple) — supported targets: aarch64-apple-darwin, aarch64-unknown-linux-gnu, x86_64-apple-darwin, x86_64-unknown-linux-gnu` (drafted around Task 3.2.1b's "unsupported platform" error, `research/features.md`'s edge-case table) | non-zero | Nothing written — resolved and failed before `fetch_target_bytes` is ever called, so no network access is attempted for the payload either. |
| **E5 — Checksum mismatch** | Step 5 — sha256 compare | `error: **checksum mismatch** for kibitzer-stub-plugin v0.1.0 (target x86_64-unknown-linux-gnu)\n  expected: 3fa8…\n  got:      9c21…\nrefusing to install — the download may be corrupted or tampered with; no file was written.` (drafted around the required `"checksum mismatch"` substring, Story 3.2.2's AC — this is the one state the requirements doc's trust-boundary concern is directly about) | non-zero | Registry has zero plugins for this name (Story 3.2.2's AC), no file under `default_plugin_dir()`. |
| **OK — Already installed, same version** | Step 3 | `[kibitzer] kibitzer-stub-plugin is already installed (v0.1.0) — no changes made` | **0** | No download, no registry write — this is success, not an error, per the idempotent-`add` model `research/ux.md` §4 chose over `gh`'s error-on-reinstall. |
| **OK — Already installed, same version, `--force`** | Step 3 | `[kibitzer] kibitzer-stub-plugin v0.1.0 already installed — reinstalling due to --force` | **0** | Unlike the no-`--force` row above, falls through to Steps 4-8 and does re-fetch/re-verify/re-place — the message restates that consequence rather than repeating the "no changes made" text (Task 3.2.2a). |

No dead ends: every non-zero-exit row above either names the exact next command (`kibitzer
plugin install ... --source <older-compatible-manifest>` for E3) or states plainly that no
state changed and a retry is safe (E1, E2, E4, E5, E7) — the user is never left needing to
inspect `default_plugin_dir()` or `registry.json` by hand to know what happened.

**Accepted v1 tradeoff — no progress indicator or explicit timeout on the network step**: Steps
1 and 4 (manifest fetch, binary download) run as a single-shot, synchronous `ureq` call with no
progress bar and no explicit request timeout configured. This is a conscious scope cut, not an
unexamined gap: for a personal tool's occasional, human-initiated install of a small binary, a
silent wait is acceptable. If a slow release-asset download or a hung connection ever proves
this insufficient in practice, a progress indicator and/or an explicit request timeout are the
natural v2 follow-up — not designed here.

**Resolved** — version-upgrade announcement: an earlier revision of this doc flagged
**`--version`-style "different version already installed"** as a gap between `research/ux.md`
§4's recommendation and what `plan.md` actually gated (a version bump fell through to Step 4
and overwrote silently, with no announcement). `plan.md`'s Task 3.2.2a now implements the
recommended fix: when the manifest's `version` differs from the registry's, `install_plugin`
logs `[kibitzer] upgrading <name> v<old> -> v<new>` before falling through to the normal
fetch/verify/place/register steps, and Story 3.2.2 carries a Given/When/Then acceptance
criterion locking that exact wording. No open gap remains here.

**Resolved** — force-reinstall of the same version: `--force` re-running `install` against an
already-installed *same*-version plugin previously had no drafted message at all. Per this
doc's own principle for forced actions (see Surface 4's `remove --force` treatment: a forced
action should restate its consequence, not print an identical unqualified success line),
`plan.md`'s Task 3.2.2a now specifies `[kibitzer] <name> v<version> already installed —
reinstalling due to --force`, distinct from the ordinary no-`--force` no-op line
(`... already installed (vX) — no changes made`). See the updated happy-path examples below.

---

## Surface 2 (condensed): `kibitzer plugin list`

```
$ kibitzer plugin list
kibitzer-stub-plugin v0.1.0 — /home/tstapler/.local/share/kibitzer/plugins/kibitzer-stub-plugin/kibitzer-stub-plugin

$ kibitzer plugin list        # zero plugins installed
[kibitzer] no plugins installed
```

Acceptance criteria:
- Zero-plugin output is the exact literal string `[kibitzer] no plugins installed`, exit `0`
  (Story 3.1.1's stated AC) — never an error, since "no plugins" is the default, expected state.
- One line per installed plugin, format `"{name} v{version} — {binary_path}"` (Task 3.3.1a) —
  no truncation of the path, since it's the one piece of information a user needs to manually
  inspect or `rm` the binary if `remove` itself is ever broken.
- Output is stable, greppable plain text (no ANSI color codes, no table borders) — consistent
  with every other kibitzer list-style output and safe for an agent or script to parse with
  a simple split.
- `list` never mutates state (no registry write, no network call) regardless of what it finds.

---

## Surface 3 (condensed): `kibitzer plugin status [<name>]`

```
$ kibitzer plugin status kibitzer-stub-plugin
kibitzer-stub-plugin v0.1.0: ok

$ kibitzer plugin status kibitzer-stub-plugin    # binary deleted out from under the registry
kibitzer-stub-plugin v0.1.0: binary missing (expected at /home/tstapler/.local/share/kibitzer/plugins/kibitzer-stub-plugin/kibitzer-stub-plugin)

$ kibitzer plugin status does-not-exist
error: no plugin named 'does-not-exist' installed
```

Acceptance criteria:
- A healthy plugin's status line contains the literal substring `ok` (Story 3.3.1's AC).
- A plugin whose binary was deleted out-of-band renders `binary missing`, distinct from `ok`,
  without kibitzer attempting to shell out to a binary it just confirmed doesn't exist
  (Story 3.3.1's AC).
- Looking up a name that was never installed errors with `"no plugin named '<name>' installed"`
  — deliberately echoing `checker::lookup`'s existing `"no checker named '{name}' registered"`
  phrasing (`src/main.rs:232`) for cross-mechanism vocabulary consistency, per `research/ux.md`
  §4's explicit recommendation.
- `status` with no `<name>` argument (`PluginAction::Status { name: Option<String> }`,
  Task 3.1.1a) reports one line per installed plugin rather than requiring the user to already
  know a name to ask "is anything broken?" — mirrors `daemon status`'s zero-argument,
  "tell me what's true right now" ergonomics.

---

## Surface 4 (condensed): `kibitzer plugin remove <name> [--force]`

```
$ kibitzer plugin remove kibitzer-stub-plugin
[kibitzer] removed kibitzer-stub-plugin

$ kibitzer plugin remove kibitzer-stub-plugin     # .claude/inspect.json still references it
error: 'kibitzer-stub-plugin' is referenced by check 'kibitzer-stub-plugin' in .claude/inspect.json
remove that check entry first, or rerun with --force (the check will then report
plugin-not-installed instead of running — see `kibitzer plugin status`).

$ kibitzer plugin remove kibitzer-stub-plugin --force
[kibitzer] removed kibitzer-stub-plugin (--force: .claude/inspect.json still references it)
```

Acceptance criteria:
- Default (no `--force`) refuses with a message containing the substring `"is referenced by
  check"`, exits non-zero, and leaves the plugin registered and the binary in place (Story
  3.3.2's AC) — the guard only checks the *local* config
  (`config::find_config`, not `find_effective_config`, per Task 3.3.2a), so it reflects
  hand-authored references, not the plugin's own synthesized entry.
- `--force` proceeds: exits `0`, the registry no longer contains the entry, and
  `default_plugin_dir().join(name)` no longer exists on disk (Story 3.3.2's AC).
- The `--force` success message explicitly restates the consequence (a stale reference becomes
  a legible "plugin not installed" state, not a silent no-op) rather than printing an identical
  success line to the no-conflict case — so a user who typed `--force` gets confirmation of
  what they just accepted, not the same text as an uneventful removal.
- Removing a name that isn't installed at all uses the same `"no plugin named '<name>'
  installed"` error text as `status` (vocabulary consistency across all three lookup-by-name
  subcommands).

---

## Surface 5 (condensed): MCP `list_checks` — plugin-not-installed tag

```
comment-quality-semantic (Blocking, scope=[**/*.rs]), plugin=not-installed
kibitzer-stub-plugin (Advisory, scope=[**/*])
go-blank-imports (Blocking, scope=[**/*.go])
```

Acceptance criteria:
- A plugin-backed check whose binary is missing gets the exact literal suffix
  `, plugin=not-installed` appended to its existing `"{name} ({severity:?}, scope={scope:?})"`
  line (Task 4.3.1d) — no separate section, no reordering; an agent parsing this output line-by-line
  sees the tag inline on the one line it already reads for that check.
- A non-plugin check (native or hand-authored `command`) never gets this suffix under any
  circumstance — `missing_binary_for` returns `None` for any name that isn't in the plugin
  `Registry` at all (Task 4.3.1a), so the tag is provably scoped to plugin checks only.
- An installed-and-present plugin's line renders identically to any other check's line (no
  tag) — the tag communicates absence, not "this is a plugin."
- This is a pure read: computing the tag calls `missing_binary_for` (an `fs::exists`-style
  check plus a registry lookup), never spawns the plugin binary itself.

---

## Surface 6 (condensed): `run_checks`/hook — `[skipped]` line for a missing plugin

MCP `run_checks` line (`src/mcp.rs`, Task 4.3.1c):
```
[skipped] kibitzer-stub-plugin: plugin 'kibitzer-stub-plugin' is not installed (expected binary at /home/tstapler/.local/share/kibitzer/plugins/kibitzer-stub-plugin/kibitzer-stub-plugin) — run `kibitzer plugin install kibitzer-stub-plugin`
```

Hook (PostToolUse) advisory-context line (`src/hook.rs:182-186`, same branch):
```
[skipped] kibitzer-stub-plugin: plugin 'kibitzer-stub-plugin' is not installed (expected binary at /home/tstapler/.local/share/kibitzer/plugins/kibitzer-stub-plugin/kibitzer-stub-plugin) — run `kibitzer plugin install kibitzer-stub-plugin`
```

Acceptance criteria:
- The line is prefixed `[skipped]`, never `[Blocking]`/`[Advisory]` — `CheckResult.severity`
  is forced to `Advisory` on this path (Task 4.3.1b) specifically so a missing install can
  never block a commit/PostToolUse gate, and the `plugin_missing` flag (not severity) is what
  routes rendering to `[skipped]` (Task 4.3.1c).
- The message names the exact remedy command (`` `kibitzer plugin install <name>` ``) inline —
  an agent hitting this should never need to guess the fix or go search docs; this is the
  concrete answer to `research/ux.md` §3's stated risk ("an agent hitting this today would
  plausibly try to 'fix' the missing binary by editing source code").
- `run_check` never actually spawns the missing command for this case — the check is fully
  short-circuited before `cmd_str` is built (Task 4.3.1b), so there is no exit-127/stderr
  text anywhere in the output for an agent to misparse as a real finding.
- The blocking-path renderer in `hook.rs` (`src/hook.rs:167-179`) is provably unreachable for
  a `plugin_missing` result, since severity is always forced to `Advisory` — verified by
  Task 4.3.1c's own note, not left as an assumption for implementation to discover later.

---

## UX acceptance criteria (Step 3)

**Task completion / step count**
1. A user can install a plugin from a known source in **exactly 1 command**
   (`kibitzer plugin install <name> --source <source>`), with zero interactive prompts.
2. A user can confirm an install succeeded in **1 additional command** (`kibitzer plugin
   list` or `kibitzer plugin status <name>`) without reading any file on disk directly.
3. A user can fully undo an install in **1 command** (`kibitzer plugin remove <name>`), or
   **2 commands** when a live check reference requires either editing `.claude/inspect.json`
   first or passing `--force`.
4. Re-running `install` for a plugin already at the requested version is a **no-op success**
   (exit 0, no filesystem or registry mutation) — a user should never need to run `plugin
   list` first to check "did I already do this" before it's safe to type `install` again.

**Error-state clarity**
5. Every error state's message names either (a) the exact command that resolves it, or (b)
   an explicit statement that no state changed and a retry is safe. No error message is a bare
   stack trace, raw `ureq`/`std::io` error text with no surrounding sentence, or unexplained
   exit code.
6. The two trust-boundary error states (**E2** host not allowlisted, **E5** checksum
   mismatch) refuse outright rather than warn-and-continue — there is no `--skip-checksum`/
   `--insecure` escape hatch anywhere in the design, matching the requirements doc's stated
   trust concern and ADR-002.
7. No error state leaves a partially-written binary under `default_plugin_dir()` — verified
   directly by Story 3.2.2's own acceptance criteria (checksum-mismatch and version-incompatible
   cases both assert zero files/registry entries afterward).

**No dead ends**
8. Every one of the seven error rows in Surface 1's table has a stated exit path (retry, fix a
   config file, upgrade kibitzer, use a different machine/target, or point at a different
   source) — none require inspecting `registry.json` or `default_plugin_dir()` by hand to
   know what to do next.
9. `plugin remove`'s refusal state names the exact file and check name blocking it
   (`.claude/inspect.json`, the check's `name`) rather than a generic "in use" message, so the
   fix is find-and-edit, not guess-and-check.

**Agent legibility (the primary consumer per `research/ux.md`)**
10. An agent reading `list_checks` output can distinguish "present and runnable" from "present
    but plugin missing" **without** running the check or parsing shell exit codes — the
    `plugin=not-installed` tag is present at read time, computed from the registry and a
    filesystem existence check alone.
11. An agent reading a `run_checks`/hook result for a missing-plugin check can distinguish it
    from a genuine finding **without** string-matching on `sh`'s exit-127 stderr text — the
    `[skipped]` prefix and `plugin_missing` flag are the intended discriminator, not stderr
    content (Tech Debt Disposition's explicit rejection of "teach `run_check` to parse
    exit-127 text" as brittle).
12. A missing-plugin skip is never rendered with a `[Blocking]`/`[Advisory]` bracket that an
    agent's existing failure-triage logic might misfile as a code defect — enforced structurally
    by forcing `Severity::Advisory` plus branching on `plugin_missing` before the severity-bracket
    render path, not by convention alone.

**Accessibility**
13. N/A — this is a CLI-only feature with no visual UI, no color-only signal (all states are
    also distinguishable by literal text substrings per the acceptance criteria above, so a
    screen reader or a plain-text log has the same information as a terminal with color), and
    no keyboard-navigation surface. Noted explicitly per the design brief rather than omitted.

---

## Summary

- **Surfaces designed**: 6 (1 full — `kibitzer plugin install`; 5 condensed — `plugin list`,
  `plugin status`, `plugin remove`, MCP `list_checks`'s plugin tag, and the `run_checks`/hook
  `[skipped]` output).
- **UX acceptance criteria written**: 13, covering task/step-count completion (4), error-state
  clarity (3), no-dead-ends (2), agent legibility (3), and accessibility (1, N/A justified).
- **Two gaps surfaced in an earlier revision, now resolved**: `install`'s handling of
  "different version requested than installed" previously had no explicit gate in `plan.md`'s
  acceptance criteria; Task 3.2.2a now logs `[kibitzer] upgrading <name> vX -> vY` and Story
  3.2.2 carries a locking acceptance criterion (see Surface 1's error/edge-case section).
  Likewise, `install --force` re-running against an already-installed same-version plugin now
  has a drafted, consequence-restating message (`... already installed — reinstalling due to
  --force`) rather than no specified message at all.
