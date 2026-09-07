# UX Research: checker-plugin-system

Audience for this feature is entirely non-GUI: a terminal user (tstapler) and an AI
agent (Claude Code via MCP `list_checks`/`run_checks` and the PostToolUse/Stop hooks).
No visual design surface exists to review; this doc substitutes CLI-output-shape and
agent-legibility analysis for the usual screen-by-screen UX walkthrough.

## 1. Comparable CLI UX patterns

Two most relevant, picked because kibitzer's situation is a hybrid of both: it's a
single core binary gaining an *optional component* (rustup's model: components of one
toolchain, not independent packages) that is *fetched from a versioned release
artifact* (gh's model: install pulls a specific repo's released binary, with an
update/checksum story).

### `rustup component add/remove/list`

Command surface:
```
rustup component add <component> [--toolchain <name>] [--target <triple>]
rustup component remove <component> [--toolchain <name>] [--target <triple>]
rustup component list [--toolchain <name>] [--installed]
```
- `list` prints every known component, one per line, each suffixed `(installed)` or
  `(default)` — installed and available components share one flat list, not two
  separate views. `--installed` filters to only-installed.
- `add` on an already-installed component is a no-op that still exits 0 (idempotent),
  not an error — contrast with `gh extension install` below.
- `remove` on a component that isn't installed errors clearly:
  `error: toolchain 'x' does not have component 'y' installed`.
- No progress bar for small components; larger ones (e.g. `rust-src`) show a
  download progress line reused from rustup's toolchain-install code path.
- What makes it feel good: **one flat namespace, one verb pair (`add`/`remove`), and
  idempotent `add`.** The user never has to remember whether something is "installed
  or not" before acting — bad UX in that dimension is punishing the user for not
  tracking state the tool already has.
- What's clunky: `list`'s single flat view conflates "available" and "installed" in
  a way that requires reading each line's suffix; a scriptable consumer has to parse
  text, since there is no `--json` list output (confirmed still true as of the 2026
  docs surfaced above — rustup has no machine-readable component list).

### `gh extension install/list/remove`

Command surface:
```
gh extension install <owner>/<repo>
gh extension list
gh extension remove <name>
gh extension upgrade <name> | --all
```
- `install` resolves `<owner>/<repo>` to a GitHub Release, picks the OS/arch-matching
  release asset, downloads it, and marks it executable — the closest existing analogue
  to "fetch a companion binary and place it somewhere kibitzer's check-resolution can
  find it" from the requirements doc.
- `install` on an already-installed extension **errors** (`X is already installed`),
  unlike rustup's idempotent `add` — the two tools disagree on this point, so kibitzer
  has to pick one; recommendation below.
- `list` shows name, repo, and (when an update exists) a version delta — this is the
  "installed + update-available" signal a single flat rustup-style list can't cleanly
  convey.
- Errors surface as plain one-line stderr messages prefixed with nothing but the
  message itself (no `gh: ` prefix), relying on exit code alone for scriptability —
  weaker than kibitzer's existing `[kibitzer] ...` prefix convention seen in
  `Command::Daemon`'s `println!("[kibitzer] daemon stopped")` (`src/main.rs:199`).
- What makes it feel good: **the identity of what's installed is the same string used
  to install it** (`owner/repo`), so `list`'s output can be pasted directly back into
  `remove`. What's clunky: no dry-run, and a network failure mid-install can leave a
  half-extracted directory (open GitHub issues note cleanup gaps) — a cautionary
  example for kibitzer's own download-failure handling (§4).

(`cargo install --list` and `code --install-extension` were reviewed but are less
relevant: `cargo install --list` only enumerates *your own* prior `cargo install`
invocations with their binary names — no registry-driven "available but not
installed" concept at all — and `code --install-extension` targets a marketplace/ID
namespace (`publisher.extension`) that has no analogue in kibitzer's single-repo,
single-maintainer distribution model.)

## 2. User mental model — matching kibitzer's existing CLI vocabulary

`src/main.rs`'s `Command` enum (`src/main.rs:49-95`) already establishes the naming
convention this feature must match:

| Existing verb group | Subcommands |
|---|---|
| `kibitzer daemon` | `start`, `stop`, `status` |
| `kibitzer check` | `native`, `architecture`, `list`, `backtest` |
| `kibitzer architecture` | `export`, `diagram` |
| `kibitzer install` | (flat, `--global`/`--dry-run` flags) — **already means "install the Claude Code PostToolUse hook into settings.json"** |

Two consequences fall directly out of that table:

- **The verb-group pattern is `<noun> <verb>`, nested via `#[command(subcommand)]`.**
  `plugin install/list/remove/status` (mirroring `daemon start/stop/status`) is the
  only naming that's consistent with the existing tree — a bare `kibitzer install-plugin`
  or `kibitzer plugin-add` would be the odd one out.
- **`kibitzer install` is already taken** and means something unrelated (hook wiring).
  `kibitzer plugin install <name>` avoids the collision because it's a different noun
  group, but tstapler (or a future reader of `--help`) seeing both `kibitzer install`
  and `kibitzer plugin install` side by side needs the top-level help text to
  disambiguate them by one line each — worth flagging for the plan/implementation
  phase, not solvable by naming alone.

Given `daemon`'s `start/stop/status` and `check`'s `list`, the expected `plugin` verb
set is:
```
kibitzer plugin install <name>     # fetch + place + register
kibitzer plugin list               # installed plugins + (maybe) available ones
kibitzer plugin remove <name>      # unregister + delete the binary
kibitzer plugin status [<name>]    # like `daemon status`: is it there and runnable
```
`status` deserves its own subcommand (not folded into `list`) because `daemon status`
already establishes that pattern for "is this thing alive/working" as distinct from
"list the things" — relevant since a plugin binary can be present-but-broken (wrong
checksum survived an old install, arch mismatch) in a way `list` alone wouldn't
surface.

Behaviorally, tstapler already knows (from `docs/output-formats.md` and
`Check.command`/`output_format: sarif`) that a check is "a command plus a SARIF flag
in `.claude/inspect.json`." The single biggest expectation `plugin install` must meet
is: **it writes (or updates) a `Check` entry with `command` pointing at the installed
binary and `output_format: sarif` set**, per the Success Metrics' "wires it into the
effective check config ... without the user hand-editing `.claude/inspect.json`
themselves." Given the Rabbit Holes' open question about *where* that registration
lives (auto-write into `.claude/inspect.json` vs. a separate kibitzer-owned manifest),
the mental-model risk is: if it writes into the user's own `.claude/inspect.json`,
that's a file tstapler hand-edits and expects to own — a plugin install silently
appending to it is a surprise diff in a repo the tool doesn't otherwise touch outside
`kibitzer install`'s explicit hook-merge (which already sets the precedent of "kibitzer
merges into files it doesn't own, but only via an explicit subcommand the user typed,
and `--dry-run` exists precisely so the user can preview that merge before it
happens" — `Command::Install`'s `dry_run` flag, `src/main.rs:87`). `plugin install`
should offer the same `--dry-run` affordance for the same reason.

## 3. Agent-facing legibility (`list_checks`/`run_checks`, MCP + hooks)

This is the least precedented lens of the five, so it gets the most detail.

**Current state** (`src/mcp.rs:272-289`): `list_checks` renders each check as
`"{name} ({severity:?}, scope={scope:?})"` — no field distinguishes "configured and
runnable" from "configured but its command doesn't exist" from "configured but named
in `disabled`" (`disabled` already exists as a concept — `src/config.rs:379`, a
`Vec<String>` overlay that drops a default check by name — but a disabled check is
already *absent* from `config.checks` by the time `list_checks` renders it, so today's
agent-visible states are just "present" vs. "silently absent," with no signal
explaining *why* something absent is absent).

**What happens today if a plugin's binary is missing** — traced through
`run_check` (`src/check.rs:140-178`): a `Check.command` referencing a not-yet-installed
plugin binary is still shelled out via `Command::new("sh").arg("-c").arg(&cmd_str)`.
`sh` itself spawns fine; the *inner* command fails with "command not found," `sh`
exits 127, and `passed_raw` becomes `false`. This surfaces through `run_checks`
(`src/mcp.rs:474-495`) as an ordinary failing check: `[Blocking] plugin-check-name:
sh: line 1: kibitzer-plugin-foo: command not found`. **An agent cannot distinguish
this from a real finding** — both are "check failed, here's stderr text." This is the
exact ambiguity requirements.md's research question flags, and it's a real
correctness gap: an agent hitting this today would plausibly try to "fix" the missing
binary by editing source code, or report a false positive to the user, rather than
running `kibitzer plugin install foo`.

**Three states an agent needs to tell apart** (mapped to concrete signal):

1. **Ran, no findings** — today's `"all checks passed"` / absence from the failures
   list. No change needed; this is already unambiguous.
2. **Disabled** (named in `.claude/inspect.json`'s `disabled`, or a plugin check
   whose plugin was deliberately removed) — currently invisible to `list_checks`
   entirely (filtered out by `merge_checks`, `src/config.rs:601-607`, before
   `list_checks` ever sees it). Recommendation: **`list_checks` should surface
   disabled defaults explicitly** (e.g. one line per name in `local.disabled`, tagged
   `(disabled)`), not just silently omit them — otherwise an agent asking "what
   checks exist" gets a catalog that quietly shrank with no way to know a default was
   turned off versus never having existed. This is a `list_checks` output change with
   no dependency on the plugin mechanism, but the plugin work is what makes the gap
   costly enough to fix.
3. **Configured but plugin-not-installed** (the new case) — recommendation: give this
   a distinct, structured signal rather than relying on `sh`'s stderr text pattern.
   Concretely:
   - At config-merge or `list_checks` time, kibitzer should proactively verify a
     plugin-backed check's `command` binary resolves (e.g. a `which`-style existence
     check on the resolved path before ever shelling out), not wait for `sh` to fail.
   - `list_checks` should render that check as e.g.
     `"comment-quality-semantic (Blocking, scope=[...], plugin=not-installed)"` —
     a machine-greppable tag an agent (or a human skimming) can act on immediately,
     distinct from the exit-127 text-parsing an agent would otherwise have to do.
   - `run_checks`/the PostToolUse hook should skip attempting to run it at all and
     report a distinct line, e.g. `[skipped] plugin-check-name: plugin 'foo' not
     installed — run 'kibitzer plugin install foo'` — actionable (names the exact
     fix command) and structurally different from `[Blocking]`/`[Advisory]` failure
     lines, so an agent's failure-triage logic doesn't misfile it as a code defect.
   - This mirrors how `CheckResult.describe()` already treats `message` as "why" and
     `output` as "where" (`src/check.rs:47-52`) — a plugin-missing report should reuse
     that shape: a fixed "why" (not installed) plus a "where"/fix (the exact install
     command), not a wall of shell stderr.

Net: the requirements doc's Success Metrics don't currently mention agent-facing
`list_checks`/`run_checks` output changes at all — this research surfaces that as a
real gap Phase 3 planning should size in, since without it the plugin mechanism ships
with a foreseeable false-positive/false-negative confusion for its actual primary
consumer (Claude Code sessions, which outnumber tstapler's own terminal invocations
per the existing PostToolUse hook design).

## 4. Error states — console output shapes

Following kibitzer's existing `[kibitzer] ...` prefix convention
(`src/main.rs:199,201,207,209`) and its existing `anyhow::Context`-wrapped error style
(`.with_context(|| format!("no checker named '{name}' registered"))`,
`src/main.rs:232`):

- **Plugin not found in manifest**
  ```
  error: no plugin named 'comment-quality-semantic' in the plugin manifest
  (run 'kibitzer plugin list --available' to see known plugins)
  ```
  Exit non-zero. Mirrors `checker::lookup`'s existing "no checker named 'x'
  registered" phrasing (`src/main.rs:232`) for vocabulary consistency.

- **Download failed (network)**
  ```
  error: failed to download comment-quality-semantic v0.1.0 from <url>: <transport error>
  (no files were modified; try again once network access is available)
  ```
  Must not leave a partially-written binary in the plugin directory — write to a temp
  path and rename on success only, the exact gap gh's own extension-install issues
  cite as a real-world failure mode worth avoiding.

- **Checksum mismatch**
  ```
  error: checksum mismatch for comment-quality-semantic v0.1.0
    expected: <sha256 from manifest>
    got:      <sha256 of downloaded file>
  refusing to install — the download may be corrupted or tampered with; the
  downloaded file was not kept.
  ```
  This is the one error state directly tied to the requirements doc's stated trust
  concern ("a downloaded-and-executed companion binary is still a real trust
  boundary"). It should be loud and refuse to proceed, not warn-and-continue.

- **Already installed (re-running install)**
  Recommendation: **follow rustup's idempotent-`add` model, not gh's error-on-reinstall
  model** — `kibitzer plugin install foo` again should succeed with
  `[kibitzer] comment-quality-semantic is already installed (v0.1.0) — no changes made`
  and exit 0, since this is a single-user local tool where "did I already run this"
  is a common, harmless question to ask by just re-running the command. Reserve a
  non-zero exit + distinct message for the case a *different version* is requested
  (`--version` flag) than what's installed, prompting toward an explicit
  `plugin upgrade` rather than silently swapping versions under `install`.

- **Uninstalling a plugin referenced by a live `.claude/inspect.json` check**
  ```
  error: 'comment-quality-semantic' is referenced by check 'comment-quality-semantic'
  in .claude/inspect.json:<line>
  remove that check entry first, or rerun with --force to remove the plugin and
  leave the check pointing at a missing command (it will then fail as
  plugin-not-installed — see above).
  ```
  This directly composes with the §3 recommendation: since a stale reference becomes
  the "plugin not installed" state (not a silent no-op), it's safe to offer `--force`
  here rather than hard-blocking — the resulting state is legible instead of a mystery
  shell failure.

## 5. Job-to-be-done

- **Functional job**: get the optional heavy checker (initially the embedding-based
  comment checker) running against the repo with minimal ceremony — one command,
  no manual dependency wrangling (no separately installing an ONNX runtime, no
  hand-authoring the `.claude/inspect.json` `Check` entry), matching the Success
  Metrics' "without the user hand-editing `.claude/inspect.json` themselves."
- **Emotional job**: trust that running `kibitzer plugin install <name>` won't
  silently do something bad — the specific fear named in requirements.md's Feasibility
  Risks and Non-functional Requirements ("a downloaded-and-executed companion binary
  is still a real trust boundary"). Concretely, that trust is earned by: the download
  being an explicit, user-typed action (never triggered implicitly by a normal check
  run — already a stated Constraint), a checksum check that refuses to proceed rather
  than warning-and-continuing (§4), and every state the mechanism can end up in being
  visibly reportable rather than silent (§3) — an agent or a human should never be
  left wondering whether a check "isn't running" because it's disabled, not installed,
  or genuinely passing.
- **Social job**: explicitly out of scope/not applicable. This is confirmed by the
  requirements doc's own Users/Consumers section — "single user today, tstapler ...
  No external/downstream users yet" — so there is no audience to signal status to,
  no shared registry reputation to build, and no reason to design for a multi-user
  trust/discovery experience (e.g. a plugin marketplace) in this phase; requirements.md
  independently excludes that ("Any plugin marketplace/discovery UI ... beyond basic
  artifact integrity" is Out of Scope). Noting the absence here rather than silently
  skipping the lens, per the research brief.

## Summary of concrete recommendations for Phase 3 planning

1. Command surface: `kibitzer plugin install|list|remove|status`, matching the
   `daemon`/`check`/`architecture` nested-subcommand convention already in
   `src/main.rs`. Give `install` a `--dry-run` flag mirroring `Command::Install`'s.
2. `install` is idempotent (rustup model) on exact-version match; a version mismatch
   needs an explicit `--version`/`upgrade` path, not a silent overwrite.
3. Checksum verification is mandatory and fails closed (no partial/corrupted file
   left behind) — this is the one place the trust-boundary risk is concrete enough to
   over-invest in relative to the rest of the mechanism.
4. `list_checks` needs a third visible state beyond "present"/"silently absent":
   surface `disabled` defaults and a new `plugin=not-installed` tag, so an agent can
   distinguish "ran clean" / "disabled" / "plugin missing" without parsing shell
   stderr — this is a real gap in today's `list_checks`/`run_checks` output that the
   plugin mechanism will make costly if left unaddressed, and should be sized into
   Phase 3's task breakdown rather than treated as implied.
5. `remove` should refuse (with a clear message and a `--force` escape hatch) when a
   live `.claude/inspect.json` check still references the plugin, rather than leaving
   a dangling `command` to fail mysteriously later.
