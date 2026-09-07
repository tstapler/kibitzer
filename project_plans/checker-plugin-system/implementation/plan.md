# Implementation Plan: checker-plugin-system

**Feature**: An install/register/run/uninstall lifecycle for optional, separately-downloadable
external checkers, built on `Check.command`/`output_format: sarif`, proven end-to-end with a
throwaway `kibitzer-stub-plugin` crate published as its own `cargo-dist` release artifact.
**Date**: 2026-09-06
**Status**: Ready for implementation
**Sizing**: **Medium (1-2 weeks)** — resolves requirements.md's Appetite ("TBD — Phase 3
planning sizes this after research") now that the plan is written: 47 tasks across 12 epics
and 17 stories, spanning a real Cargo workspace restructuring (Phase 1), a new domain model
and HTTPS+checksum trust boundary (Phases 2-3), and a shared-dispatch tech-debt remediation
(Phase 4) — more than a Small, single-seam change, but no new runtime/architecture layer
(it's additive plumbing on the existing `Check.command` dispatch path), so not Large either.
Raw per-task time estimates below sum to roughly 200 minutes (~3-4 hours) of pure
hands-on-keyboard implementation time across all tasks — the Medium (1-2 weeks) bucket isn't
a claim that the tasks themselves take that long; it accounts for review/testing/iteration
overhead on top of the raw sum (PR review rounds, empirical verification steps like Task
1.2.1b, environment setup, and debugging time not captured in any single task's estimate).
**ADRs**:
- [ADR-001](../decisions/ADR-001-real-cargo-workspace-with-stub-plugin-package.md) — real Cargo workspace + stub plugin package, lockstep versioning
- [ADR-002](../decisions/ADR-002-checksum-verified-https-only-plugin-trust-model.md) — HTTPS-only + mandatory SHA-256 verify-before-write trust model
- [ADR-003](../decisions/ADR-003-min-kibitzer-version-enforced-at-install-not-runtime.md) — hand-rolled version comparator, enforced once at install

All file/line citations below were re-verified directly against the current repo at commit
`ae1d8cdb59ba03bb689540813ae3254dd75211ba` (`git rev-parse HEAD` at planning time — the exact
commit `research/architecture.md` was written against; no drift found).

---

## Step 0.5 — Creative pass: alternatives for the plugin-wiring mechanism

(Requirements.md already locked the *outer* decision — extend `Check.command`/SARIF, not a
dylib/WASM loader. This pass is about *how an installed plugin becomes part of the effective
check list* — the one piece of real design freedom left.)

1. **Kibitzer-owned registry, auto-injected into `find_effective_config`** (chosen).
   Strength: satisfies the stated success metric literally — zero user file-editing — and
   is additive: zero plugins installed means `registered_plugin_checks()` returns `vec![]`
   and every downstream consumer (`run.rs`, `daemon.rs`, `hook.rs`, `mcp.rs`) is provably
   unchanged, since none of them branch on *why* a `Check` exists (confirmed by reading
   all four call sites). Weakness: install becomes global-and-silent — a plugin installed
   once runs in every repo on the machine from then on, with no per-repo confirmation.
2. **Registered-but-inert until named in `.claude/inspect.json`** (kibitzer places the
   binary + prints a copy-pasteable `Check` JSON snippet; the user still adds it).
   Strength: no surprise global behavior change — every repo opts in explicitly, matching
   `.claude/inspect.json`'s existing "user-authored" convention. Weakness: directly fails
   the requirement's own wording ("without the user hand-editing `.claude/inspect.json`
   themselves") — this is why it's rejected, not a close call.
3. **Auto-write the `Check` entry directly into `.claude/inspect.json`** (pre-commit's
   `repos:`-is-the-registration model, reusing `install.rs::merge_hook`'s merge shape).
   Strength: one config file to look at, no second manifest concept to learn. Weakness:
   `docs/suppressing-checks.md` already treats `.claude/inspect.json` as strictly
   user-authored; unlike `settings.json`'s `hooks.PostToolUse` array (which has no
   name-collision risk because kibitzer only ever appends its own entry), `checks` entries
   share one namespace with the user's own hand-written checks, so a non-destructive merge
   is a harder, less precedented problem — and it doesn't fit the "install once per
   machine, works in every repo" model the embedding checker (this mechanism's real first
   consumer) actually wants, since `.claude/inspect.json` is per-repo, checked-in state.

**Chosen: option 1**, with the disclosure mitigation architecture.md recommends: `kibitzer
plugin install` prints an explicit line stating the check now runs in every repo and how to
opt out per-repo (`disabled`) or remove it entirely (`plugin remove`).

Recorded in the Pattern Decisions table below (component: "Check synthesis /
`find_effective_config` wiring").

---

## Domain Glossary

| Term | Definition | Notes |
|------|-----------|-------|
| `PluginName` | A validated plugin identifier — a newtype wrapping `String`, constructed only via `PluginName::parse(s: &str) -> Result<Self>`, which accepts (allowlist, not denylist) only `^[A-Za-z0-9_-]{1,64}$` — so `/`, `\`, `..`, and empty strings can never reach a filesystem join. Parsed once at the CLI boundary (a clap value parser on `PluginAction`'s three `name` fields); every downstream consumer (`InstalledPlugin.name`, `Registry` lookups, `default_plugin_dir().join(...)`, `Check.name`) takes the already-validated type instead of a raw `String`. | New type, `src/plugin.rs`. Parse-at-boundary / type-driven design (see Pattern Decisions). |
| `PluginManifest` | A JSON document (fetched over HTTPS or read from a local path) describing one plugin release: `name`, `version`, `min_kibitzer_version`, and a `targets` map keyed by Rust target-triple. Never persisted long-term — consumed once at install time. Note: `manifest.name` is untrusted download content and is never used as the `InstalledPlugin`/path identity — that identity always comes from the CLI-supplied, already-validated `PluginName`. | New type, `src/plugin.rs`. |
| `PluginTarget` | One entry inside `PluginManifest.targets`: `{ url: String, sha256: String }` — the download location and expected digest for one target-triple. | New type, `src/plugin.rs`. |
| `InstalledPlugin` | One entry in the local `Registry`: `name: PluginName`, `version`, `min_kibitzer_version`, `sha256` (the digest actually verified at install time), `binary_path`, plus the `Check`-construction fields (`severity`, `scope`, `triggers`, `output_format`). | New type, `src/plugin.rs`, `Serialize + Deserialize`. |
| `Registry` | The kibitzer-owned, durable collection of `InstalledPlugin`s persisted at `default_registry_path()`. `Registry::load`/`Registry::save` mirror `Cache::load`/`Cache::save` (`src/cache.rs:57-70`). | New type, `src/plugin.rs`. |
| `default_plugin_dir()` | `$XDG_DATA_HOME/kibitzer/plugins`, falling back to `$HOME/.local/share/kibitzer/plugins` — mirrors `cache::default_cache_path()`'s exact env-var-then-`$HOME` shape (`src/cache.rs:148-157`) against a durable (not cache-clearable) XDG base. | New fn, `src/plugin.rs`. |
| `default_registry_path()` | `default_plugin_dir().join("registry.json")`. | New fn, `src/plugin.rs`. |
| `PluginAction` | The `#[derive(Subcommand)]` enum for `kibitzer plugin {install,list,remove,status}`, nested exactly like `DaemonAction` (`src/main.rs:168-175`). Its three `name`/`Remove{name}`/`Status{name}` fields are typed `PluginName`, parsed by a clap `value_parser` at argument-parsing time — an invalid name never reaches `main()`'s dispatch. | New enum, `src/main.rs`. |
| `install_plugin` | `fn install_plugin(name: &PluginName, source: &str, force: bool) -> Result<ExitCode>` — the install lifecycle: fetch manifest → resolve target-triple → version-compat gate (ADR-003) → fetch binary → verify checksum (ADR-002) → place + `chmod +x` → write `Registry` entry. | New fn, `src/plugin.rs`. |
| `remove_plugin` | `fn remove_plugin(name: &PluginName, force: bool) -> Result<ExitCode>` — the uninstall lifecycle (named `remove` per `research/ux.md`'s CLI-verb recommendation, matching `gh extension remove`/prospective `rustup component remove`, not "uninstall" literally). Refuses (unless `force`) when a live `.claude/inspect.json` in the current repo still references the plugin by name. | New fn, `src/plugin.rs`. |
| `list_plugins` | `fn list_plugins() -> Result<Vec<InstalledPlugin>>` — backs `kibitzer plugin list`. | New fn, `src/plugin.rs`. |
| `plugin_status` | `fn plugin_status(name: &PluginName) -> Result<PluginStatusReport>` — backs `kibitzer plugin status <name>`: re-hashes the on-disk binary, checks it still exists, and re-runs the version-compatibility comparison against the currently-running kibitzer. | New fn, `src/plugin.rs`. |
| `PluginStatusReport` | `{ installed: InstalledPlugin, binary_present: bool, hash_matches: bool, version_compatible: bool }` — the structured result `plugin_status` renders as text. | New type, `src/plugin.rs`. |
| `registered_plugin_checks()` | `fn registered_plugin_checks() -> Vec<Check>` — loads the `Registry` and maps each `InstalledPlugin` to a synthesized `Check` (mirrors `config.rs`'s `native_check()` helper shape, `config.rs:512-524`). The one function `config::find_effective_config` calls into. | New fn, `src/plugin.rs`. |
| `missing_binary_for` | `fn missing_binary_for(check_name: &str) -> Option<PathBuf>` — looks up the `Registry` by check/plugin name (comparing against each `InstalledPlugin.name.as_ref()`), returns `Some(binary_path)` if that path doesn't exist on disk. Takes `&str`, not `PluginName`, because it's called for *every* `Check.name` including hand-authored, non-plugin checks whose names aren't `PluginName`-shaped — those simply never match and return `None`. Backs the "legible not-installed" signal (Tech Debt item c). | New fn, `src/plugin.rs`. |
| `version_meets_minimum` | `fn version_meets_minimum(min: &str, current: &str) -> Result<bool>` — parses both as `(u64, u64, u64)` and compares (ADR-003). | New fn, `src/plugin.rs`. |
| `parse_plain_version` | `fn parse_plain_version(s: &str) -> Result<(u64, u64, u64)>` — the hand-rolled `MAJOR.MINOR.PATCH` parser `version_meets_minimum` calls twice. | New fn, `src/plugin.rs`. |
| `PLUGIN_HOST_ALLOWLIST` | `const PLUGIN_HOST_ALLOWLIST: &[&str] = &["github.com", "api.github.com"]`, plus a suffix check for any host ending in `.githubusercontent.com` (GitHub's release-asset redirect target, whose exact subdomain has changed over time) — the hardcoded host allowlist a remote `source`/manifest URL and any redirect target must stay within (ADR-002). | New const + suffix check, `src/plugin.rs`. |
| `is_allowed_host` | `fn is_allowed_host(host: &str) -> bool` — the single implementation of the `PLUGIN_HOST_ALLOWLIST`-membership-or-`.githubusercontent.com`-suffix check (ADR-002). Both `fetch_manifest` (Task 3.2.1a) and `fetch_target_bytes` (Task 3.2.1c) call this one function on a parsed URL's host instead of each re-deriving the allowlist rule independently (architecture-review.md Concern, Story 3.2.1). | New fn, `src/plugin.rs`. |
| `CheckResult.plugin_missing` | New `#[serde(default)] pub plugin_missing: bool` field on the existing `CheckResult` struct (`src/check.rs:20-52`) — `true` when `run_check` short-circuited on a missing plugin binary rather than actually dispatching. | Modified existing type, `src/check.rs`. |
| `COMMAND_TIMEOUT` | `const COMMAND_TIMEOUT: Duration = Duration::from_secs(30)` — the wall-clock limit on `run_check`'s `sh -c` dispatch (Tech Debt item a). | New const, `src/check.rs`. |
| `kibitzer-stub-plugin` | The new, throwaway Cargo package at `crates/kibitzer-stub-plugin/` — a trivial binary that always emits one canned SARIF finding, proving the install→register→run path end-to-end. Also the plugin's registered `name` used in CLI examples throughout this plan. | New crate. |

---

## Pattern Decisions

| Component | Pattern Chosen | Source | Alternative Rejected | Reason |
|-----------|---------------|--------|---------------------|--------|
| Plugin registry/manifest persistence | Repository pattern — `Registry` is an in-memory collection with `load`/`save`, isolating persistence from the `InstalledPlugin` data shape, mirroring `Cache::load`/`Cache::save` (`src/cache.rs:57-70`) | PoEAA (Repository) | Active Record — `InstalledPlugin` grows its own `.save()`/`.delete()` methods | Would couple the domain struct to file I/O and duplicate the read-modify-write shape `Cache` already establishes in this codebase; no benefit for a single-collection, single-file store. |
| Check synthesis / `find_effective_config` wiring | Auto-inject via a chained collection source (`default_checks().into_iter().chain(registered_plugin_checks())`), evaluated before `merge_checks` | This project's Step 0.5 creative pass, option 1; confirmed zero-downstream-change by `research/architecture.md` §1's grep of every `find_effective_config` caller | Registered-but-inert (manual per-repo `.claude/inspect.json` opt-in) | Fails the requirement's literal wording ("without the user hand-editing `.claude/inspect.json`") — not a close call, a stated success metric. |
| Check synthesis / `find_effective_config` wiring | (same as above) | (same as above) | Auto-write into `.claude/inspect.json` (pre-commit's `repos:`-is-registration model) | Blurs `.claude/inspect.json`'s existing user-authored-only convention (`docs/suppressing-checks.md`) and has no collision-free namespace for merge, unlike `settings.json`'s `hooks.PostToolUse` array. |
| Install/uninstall lifecycle | Command pattern — each `PluginAction` variant (`Install`/`List`/`Remove`/`Status`) maps to one discrete, independently-testable function (`install_plugin`/`list_plugins`/`remove_plugin`/`plugin_status`), mirroring `DaemonAction`'s existing shape (`src/main.rs:168-175`, `192-213`) | GoF (Command) | Template Method — a shared `InstallStep` trait with polymorphic pipeline stages | Overengineered for ~80 lines of linear orchestration (`research/build-vs-buy.md` §1's own estimate) with exactly one install flow and no variation to abstract over. |
| Version-compatibility check | Guard-clause fail-fast validation over a parsed `(u64,u64,u64)` value type — parse once, compare with derived `Ord`, never re-parse the raw string elsewhere (Parse-Don't-Validate) | Type-driven design | Lexicographic string comparison of the raw version strings | Incorrect the moment either version has a two-digit component (`"0.10.0"` sorts before `"0.9.0"` as strings) — a real bug class this avoids by construction, not a style preference. |
| Legible "plugin not installed" signal | Isolate via seam — a preflight lookup (`missing_binary_for`) at the one point `run_check`/`list_checks` already exist, producing a distinct, greppable `CheckResult.plugin_missing` flag, rather than parsing `sh`'s exit-127 stderr text | This project's Tech Debt Disposition (item c) | Teach `run_check` to distinguish exit-127 from a real failure by parsing stderr text | Brittle (locale/shell-dependent stderr wording) and conflates "command doesn't exist" with "command exists but genuinely failed" for *every* `command` check, not just plugin ones — the registry-backed lookup only needs to ask "is this specific name a plugin, and is its binary present," which is strictly narrower and doesn't touch generic dispatch. |
| Plugin `name` CLI/domain boundary | Parse-Don't-Validate — a `PluginName` smart constructor validates the raw CLI string once (`^[A-Za-z0-9_-]{1,64}$`, via a clap value parser at argument-parsing time), and every downstream consumer (`InstalledPlugin`, `Registry` lookups, `default_plugin_dir().join(...)`, `Check.name`) takes the already-validated type instead of a raw `String` | Type-driven design (Parse-Don't-Validate); architecture-review.md BLOCKER | Validating inline at each of `install_plugin`/`remove_plugin`/`plugin_status`/`fs::remove_dir_all`'s call sites | Duplicates the same rejection logic at every call site — the same anti-pattern already flagged for `fetch_manifest`/`fetch_target_bytes`'s host-allowlist check — and leaves a window for one call site to be missed entirely, which is exactly how a raw `String` reached `fs::remove_dir_all` unchecked in the pre-fix plan. |

---

## Tech Debt Disposition

| Area | Existing Issue | Disposition | Justification |
|------|----------------|--------------|----------------|
| `src/check.rs:156-160` (`run_check`'s `Command::new("sh").arg("-c")...output()?`) | No timeout anywhere on the command-dispatch path — a hung `command` check (including a plugin binary waiting on a model load that never completes) blocks indefinitely, with no distinction from kibitzer itself hanging. | **Refactor-first** | This project is explicitly designing for plugin binaries — the exact class of `command` check most likely to genuinely hang (a downloaded ONNX runtime, a first-run model load) — to be routinely installed/swapped. `research/pitfalls.md` §3 calls this "cheap enough to just fix generally, not plugin-scope it" (a 5-10 line fix at the shared dispatch point); shipping the plugin mechanism without it would make the new failure mode this project introduces strictly worse than the pre-existing one. Fixed once in `run_check`, benefiting every `command` check, not a plugin-only wrapper. |
| `src/cache.rs` (`Cache::get`/`put`'s `config_stamp` fingerprints only `repo_root/.claude/inspect.json`, `src/daemon.rs:149,240`) | Installing/uninstalling a plugin changes `registered_plugin_checks()`'s output without touching that one file, so a running daemon can serve stale cached results for a repo whose own `inspect.json` didn't change. | **Fix: fold `registry.json` into `config_stamp`** | Escalated from "Extend as-is" by pre-mortem.md's Failure #2 (P1): the persistent `kibitzer daemon` is the *primary* path for Claude Code hook/MCP calls — tstapler's actual daily use, not an edge case — so silently serving a stale plugin check list there is routine, not a narrow self-inflicted race. `research/architecture.md` already called this fix "small and contained." Implemented as Story 4.1.2: `CacheEntry` gains a `registry_stamp: Option<Stamp>` field alongside `config_stamp`, fingerprinting `crate::plugin::default_registry_path()` (`registry.json`) the same way `config_stamp` fingerprints `.claude/inspect.json`; `Cache::get`/`put` compare/store it too, so a `plugin install`/`remove` invalidates the daemon's cached result on the very next request — no daemon restart needed. See pre-mortem.md's P1 checklist item, now marked resolved. |
| `src/mcp.rs` (`list_checks`/`run_checks`), `src/hook.rs`, `src/config.rs::merge_checks` | No distinction between "check exists but plugin not installed," "check ran and found nothing," and "disabled" — a missing/wrong-arch plugin binary degrades to a raw `sh` exit-127/exec-format-error, indistinguishable from a real finding. | **Isolate via seam** | `research/ux.md` §3 identifies this as a foreseeable false-positive/false-negative confusion for the mechanism's actual primary consumer (Claude Code agent sessions via MCP/hooks, which outnumber terminal invocations). Fixed at one seam (`missing_binary_for`, called from `run_check` before dispatch and from `list_checks`/render sites) rather than refactoring `merge_checks`/the generic dispatch path — `Check` itself gains no new field, so every non-plugin check is provably unaffected. |
| `default_registry_path()`/`default_plugin_dir()` (`$XDG_DATA_HOME/kibitzer/plugins`) — plugin binary path portability across this user's dotfiles-synced machines (Manjaro/Ubuntu/macOS/WSL2), per `research/pitfalls.md`'s "Architecture/OS mismatch across synced machines" risk | A registered plugin's `binary_path` is an absolute, per-machine, per-architecture path; if the registry itself ever traveled to a different machine, that path could be missing or the wrong architecture's binary. | **Document as accepted v1 limitation** | The plugin registry is machine-local by construction: it lives under `$XDG_DATA_HOME` (`~/.local/share`), which this user's dotfiles sync never touches — verified by inspecting `~/.cfgcaddy.yml`, which lists only specific files under `.claude/` (agents, commands, skills, `CLAUDE.md`, etc.) and neither `.claude/inspect.json` nor anything under `~/.local/share`. This matches the existing precedent that `.claude/inspect.json` itself (a per-repo file) also isn't part of the dotfiles-synced `.claude/` tree. A plugin install therefore never crosses machines today, so the cross-arch-path risk `research/pitfalls.md` calls a "hard requirement" doesn't arise in practice; making the registry itself portable (target-triple-aware path resolution) is a distinct, out-of-scope follow-up if that ever changes. |

---

## Migration Plan

Omit — no existing on-disk schema or data changes. `registry.json` is a wholly new file this
project introduces; nothing pre-existing is migrated into or out of it.

## Observability Plan

- **Logs**: every `plugin install`/`remove`/`status` action prints a `[kibitzer] ...`-prefixed
  status line to stdout/stderr, matching the existing `install.rs`/`main.rs` convention
  (`src/main.rs:199,201,207,209`) — no new logging framework.
- **Metrics**: none — `requirements.md`'s Observability Requirements explicitly states
  "no new metrics/alerting needed for a single-user local tool."
- **Alerts**: none, same reason.

## Risk Control

- **Feature flag**: none needed — the mechanism is fully additive (a new subcommand tree
  and one chained collection in `find_effective_config`); `requirements.md`'s own Risk
  Control section states rollback is `git revert` if something's wrong before it ships.
- **Rollback procedure**: `git revert` the merged PR. Since `registered_plugin_checks()`
  returns `vec![]` whenever `registry.json` doesn't exist or fails to parse (same
  fail-open-to-empty precedent as `Cache::load`, `src/cache.rs:57-62`), a rollback that
  removes `src/plugin.rs` but leaves a stray `registry.json` on disk from a pre-rollback
  install is harmless — the file is simply never read again.
- **Staged rollout**: not applicable — single-user local tool, no fleet/environment
  staging concept.

## Unresolved Questions

- [ ] **Automating manifest generation from `dist`'s own `*-dist-manifest.json`** (so the
  maintainer doesn't hand-copy per-target URLs/SHA-256 digests into
  `crates/kibitzer-stub-plugin/manifest.json` at every release) is deliberately deferred —
  ADR-002's Consequences section names this explicitly as a scope cut for a
  single-plugin, single-maintainer mechanism. Blocks nothing in this plan (the manual
  demo and automated test both use a hand- or test-authored manifest); owner: tstapler,
  revisit only if the manual step proves error-prone across a few real releases.
- [ ] **Whether `dist plan` under the installed `cargo-dist-version = "0.32.0"` actually
  supports two lockstep-versioned Apps releasing from one pushed tag**, per ADR-001 —
  blocks Story 1.2.1's completion criterion, resolved empirically by Task 1.2.1b, not
  assumed. Owner: whoever implements Phase 1 (falls back to ADR-001's rejected
  prefixed-tag alternative if the dry run shows otherwise).
- [ ] **`research/ux.md` §3's "surface `disabled` defaults explicitly in `list_checks`"
  recommendation** (a real gap, but one that exists independently of the plugin mechanism
  and isn't named in `requirements.md`'s Scope) is intentionally **not** built in this
  plan — scoped out per "do what has been asked, nothing more." Owner: tstapler, a
  candidate for a small standalone follow-up, not blocking anything here.

## Dependency Visualization

```
Phase 1 (workspace + dist)
  1.1.1 Cargo workspace ──▶ 1.1.2 kibitzer-stub-plugin crate ──▶ 1.2.1 dist-workspace.toml + dist plan
                                                                        │
Phase 2 (domain types) ◀───────────────────────────────────────────────┘  (needs a real
  2.1.1 Manifest/Registry/InstalledPlugin types                           release-shaped
      │                                                                    crate to model
      ├──▶ 2.1.2 Registry load/save                                       manifest targets
      └──▶ 2.1.3 version_meets_minimum                                    against)
              │
Phase 3 (CLI lifecycle) ◀────────────────────┘
  3.1.1 Command::Plugin/PluginAction
      │
      ├──▶ 3.2.1 fetch (manifest + binary) ──▶ 3.2.2 verify/place/register (needs 2.1.2, 2.1.3, 3.2.1)
      │                                              │
      └──▶ 3.3.1 list/status (needs 3.2.2 registry)  └──▶ 3.3.2 remove (needs 3.2.2 registry)

Phase 4 (effective-config wiring + tech debt) ◀── needs only 2.1.1/2.1.2 (Registry read/write);
  its own tests (Task 4.1.1c, 4.1.2c) construct a fixture registry.json directly rather than
  requiring a real `plugin install` (Story 3.2.2) to have completed first, so Phase 4 can
  proceed in parallel with Phase 3, not strictly after it
  4.1.1 registered_plugin_checks() + find_effective_config chain
  4.1.2 daemon cache invalidation (registry_stamp) (needs 4.1.1's Registry-backed synthesis)
  4.2.1 command-dispatch timeout (independent — touches only src/check.rs)
  4.3.1 missing_binary_for + CheckResult.plugin_missing (needs 2.1.1's Registry read, and 4.1.1's synthesized Check)

Phase 5 (test + docs) ◀── needs everything above
  5.1.1 end-to-end automated test (needs 1.1.2, 3.2.2, 4.1.1)
  5.1.2 second-plugin genericity test (needs 5.1.1)
  5.2.1 docs/plugins.md + CLAUDE.md (needs the mechanism to describe)
  5.3.1 manual demo capture (needs 5.1.1 passing first, as a sanity check the walkthrough will work)
```

---

## Phase 1: Workspace & Distribution Foundation

### Epic 1.1: Real Cargo workspace + stub plugin package
**Goal**: Turn `Cargo.toml`'s disabled `[workspace]` into a real multi-member workspace and
add a throwaway `kibitzer-stub-plugin` crate, so a second, independently-buildable package
exists to publish via `cargo-dist` (ADR-001).

#### Story 1.1.1: Convert `Cargo.toml` into a multi-member workspace
**As a** maintainer, **I want** `crates/kibitzer-stub-plugin` recognized as a real workspace
member, **so that** it builds and can later be published as its own `cargo-dist` App.
**Acceptance Criteria**:
- `cargo build --workspace` succeeds after the member is added.
  - *Given* `Cargo.toml`'s `[workspace]` table has no `members` key (today's state,
    `Cargo.toml:1-3`), *When* it is changed to `[workspace]\nmembers = [".", "crates/kibitzer-stub-plugin"]`
    and `crates/kibitzer-stub-plugin/Cargo.toml` exists, *Then* `cargo build --workspace`
    exits 0 and `cargo metadata --no-deps` lists exactly two packages: `kibitzer` and
    `kibitzer-stub-plugin`.
**Files**: `/home/tstapler/code/github.com/tstapler/kibitzer/Cargo.toml`

##### Task 1.1.1a: Add `members` to `Cargo.toml`'s `[workspace]` table (~2 min)
- Replace the bare `[workspace]` (`Cargo.toml:1-3`, including its now-outdated comment) with
  `[workspace]\nmembers = [".", "crates/kibitzer-stub-plugin"]` and a comment explaining why
  (opts the repo out of the parent `stapler-scripts` workspace while adding the plugin
  crate as a sibling member).
- Files: `Cargo.toml`

##### Task 1.1.1b: Verify the workspace builds (~2 min)
- Run `cargo build --workspace` and `cargo metadata --no-deps --format-version 1 | jq '.packages[].name'`
  (or equivalent); confirm both `kibitzer` and `kibitzer-stub-plugin` appear and the build
  exits 0. Paste the command + exit status into the task's completion note — this is the
  proof-of-run this project's own Engineering Discipline requires before claiming done.
- Files: none (verification only)

#### Story 1.1.2: Create the `kibitzer-stub-plugin` crate
**As a** maintainer, **I want** a trivial binary that always emits one canned SARIF finding,
**so that** the install→register→run path has a genuine, throwaway artifact to exercise
end-to-end.
**Acceptance Criteria**:
- Running the built binary against any file argument prints valid SARIF 2.1.0 with exactly
  one result.
  - *Given* `crates/kibitzer-stub-plugin/src/main.rs` is built, *When* invoked as
    `kibitzer-stub-plugin somefile.rs`, *Then* stdout parses as JSON with
    `runs[0].results[0].ruleId == "kibitzer-stub-plugin-finding"` and
    `runs[0].results[0].level == "warning"`, and the process exits `0` (a finding is
    reported via SARIF content, not a failing exit code — consistent with
    `docs/output-formats.md`'s SARIF convention, where pass/fail comes from
    `output.status.success()` at `src/check.rs:162`, independent of finding count).
**Files**: `crates/kibitzer-stub-plugin/Cargo.toml`, `crates/kibitzer-stub-plugin/src/main.rs`

##### Task 1.1.2a: Create `crates/kibitzer-stub-plugin/Cargo.toml` (~2 min)
- Minimal package manifest: `name = "kibitzer-stub-plugin"`, `version = "0.1.0"`,
  `edition = "2024"`, one dependency — `serde_json = "1"` (for constructing the SARIF JSON;
  do not depend on the `kibitzer` package itself, per ADR-001's "zero dependency on
  `kibitzer`" decision).
- Files: `crates/kibitzer-stub-plugin/Cargo.toml`

##### Task 1.1.2b: Write `crates/kibitzer-stub-plugin/src/main.rs` (~5 min)
- Read the first CLI arg as the triggering file path (mirroring `{file}` substitution,
  matching what a real `Check.command` invocation passes); build and print a minimal SARIF
  2.1.0 document (`version: "2.1.0"`, one `runs[]` entry, `tool.driver.name:
  "kibitzer-stub-plugin"`, one `results[]` entry with `ruleId`, `level: "warning"`,
  `message.text`, and a `locations[]` entry pointing at the given file, line 1) via
  `serde_json::json!` + `println!`. Exit `0` unconditionally.
- Files: `crates/kibitzer-stub-plugin/src/main.rs`

##### Task 1.1.2c: Verify the stub's SARIF output is well-formed (~3 min)
- Run `cargo run -p kibitzer-stub-plugin -- somefile.rs` and pipe stdout through
  `jq '.runs[0].results[0].ruleId'` (or read it back with `serde_json::from_str` in a throwaway
  test) to confirm it parses and matches the acceptance criterion above. Paste the command +
  output as proof.
- Files: none (verification only)

### Epic 1.2: `cargo-dist` wiring and dry-run verification
**Goal**: Make the stub plugin a second, independently-releasable `cargo-dist` App in the same
GitHub Release pipeline, and empirically confirm (not assume) that lockstep versioning works
under the installed `cargo-dist-version = "0.32.0"` (ADR-001).

#### Story 1.2.1: Add the second `dist-workspace.toml` member and dry-run `dist plan`
**As a** maintainer, **I want** confirmation that `dist` treats `kibitzer-stub-plugin` as an
independent App before relying on that in later stories, **so that** Phase 3's install logic
isn't built against an assumption that turns out false.
**Acceptance Criteria**:
- `dist plan` lists two independent Apps for one hypothetical tag, with no tag actually
  pushed.
  - *Given* `dist-workspace.toml`'s `[workspace] members` is `["cargo:.", "cargo:crates/kibitzer-stub-plugin"]`
    and both packages carry `version = "0.1.13"` (lockstep, per ADR-001), *When* `dist plan`
    is run locally (no `git tag`/`git push`), *Then* its output enumerates two Apps —
    `kibitzer` and `kibitzer-stub-plugin` — each with its own set of the four existing
    `targets` (`aarch64-apple-darwin`, `aarch64-unknown-linux-gnu`, `x86_64-apple-darwin`,
    `x86_64-unknown-linux-gnu`) and its own archive name, not one merged archive.
- `kibitzer-stub-plugin` never gets a Homebrew formula, closing adversarial-review.md's
  BLOCKER (a second, permanent public formula published to `tstapler/homebrew-tap` for a
  "throwaway" crate on every future release).
  - *Given* `crates/kibitzer-stub-plugin/Cargo.toml` carries its own
    `[package.metadata.dist]` table with `installers = ["shell"]` — a **package-local**
    setting (confirmed against `cargo-dist`'s reference docs at the exact pinned tag
    `v0.32.0`: `installers` has been `[package-local]` since 0.0.3, meaning a package's
    own `[package.metadata.dist]` entry replaces, not merges with, the workspace-level
    list for that package only) — overriding `dist-workspace.toml`'s workspace-wide
    `installers = ["shell", "homebrew"]` (`dist-workspace.toml:11`), *When* `dist plan` is
    run, *Then* its output shows `kibitzer` with both a shell installer and a Homebrew
    formula (published to `tstapler/homebrew-tap` per `dist-workspace.toml`'s `tap`
    setting, `dist-workspace.toml:13`, which stays workspace-global — confirmed
    `[global-only]` — but is inert for a package that never opts into the `"homebrew"`
    installer), while `kibitzer-stub-plugin` shows only a shell installer and zero
    Homebrew formulas anywhere in the plan output. `publish-jobs = ["homebrew"]`
    (`dist-workspace.toml:16`, also confirmed `[global-only]`) is unaffected and needs no
    change — it publishes whatever Homebrew formulas were actually generated, which after
    this override is exactly one (`kibitzer`'s).
**Files**: `dist-workspace.toml`, `crates/kibitzer-stub-plugin/Cargo.toml`

##### Task 1.2.1a: Add `kibitzer-stub-plugin` to `dist-workspace.toml`, scoped to skip Homebrew (~3 min)
- Change `[workspace] members = ["cargo:."]` (`dist-workspace.toml:1-2`) to
  `members = ["cargo:.", "cargo:crates/kibitzer-stub-plugin"]`.
- Add a `[package.metadata.dist]\ninstallers = ["shell"]` table to
  `crates/kibitzer-stub-plugin/Cargo.toml`. This is the fix for adversarial-review.md's
  BLOCKER: `dist-workspace.toml`'s `installers = ["shell", "homebrew"]` (`dist-workspace.toml:11`)
  is workspace-wide and would otherwise apply to *every* App by default, including the
  stub — publishing a second, permanent, public `kibitzer-stub-plugin.rb` formula to
  `tstapler/homebrew-tap` on every future release. `installers` is documented
  `[package-local]` (settable per-package, overriding the workspace value) as of
  cargo-dist 0.0.3 — confirmed still true at the exact pinned `cargo-dist-version = "0.32.0"`
  (`dist-workspace.toml:7`) by reading `book/src/reference/config.md` at GitHub tag
  `v0.32.0`. `tap` and `publish-jobs` remain workspace-global (`[global-only]`, unable to
  be scoped per-package) but need no change: `publish-jobs = ["homebrew"]` only publishes
  formulas that some App's `installers` list actually generated, so once
  `kibitzer-stub-plugin` never opts into `"homebrew"`, there is no second formula for that
  job to find or for `tap` to receive.
- Files: `dist-workspace.toml`, `crates/kibitzer-stub-plugin/Cargo.toml`

##### Task 1.2.1b: Run `dist plan` and record its output (~4 min)
- Run `dist plan` (installing the pinned `cargo-dist-version = "0.32.0"` via `dist selfupdate`
  or the repo's existing CI-equivalent invocation if `dist` isn't already on `PATH`). Confirm
  both acceptance criteria above against the real output — the two-Apps/four-targets shape,
  *and* that `kibitzer-stub-plugin` shows no Homebrew installer/formula while `kibitzer`
  still does; paste the relevant excerpt as proof. If `dist plan` errors, shows the two
  packages merged into one App, or still shows a Homebrew formula planned for
  `kibitzer-stub-plugin` despite the Task 1.2.1a override, stop and escalate — ADR-001's
  fallback (prefixed per-package tags) applies for the first two failure modes, and a
  re-verification of the `installers` override's documented behavior applies for the third,
  and Task 1.2.1c changes accordingly.
- Files: none (verification only)

##### Task 1.2.1c: If needed, fall back to prefixed tags and update ADR-001 (~5 min, conditional)
- Only if Task 1.2.1b's dry run shows lockstep doesn't work as expected: switch
  `kibitzer-stub-plugin`'s `Cargo.toml` to an independent version (e.g. `0.1.0`), re-run
  `dist plan` to confirm prefixed-tag-style per-package releases work, and update
  ADR-001's Status/Decision text to record the actual outcome rather than leaving the
  contingent language in place.
- Files: `crates/kibitzer-stub-plugin/Cargo.toml`, `project_plans/checker-plugin-system/decisions/ADR-001-real-cargo-workspace-with-stub-plugin-package.md`

---

## Phase 2: Plugin Domain Model

### Epic 2.1: Manifest, registry, and version-compatibility types
**Goal**: Introduce `src/plugin.rs` with the data shapes and pure functions every later phase
builds on — no CLI, no network I/O yet, matching `src/install.rs`'s precedent of
in-memory-testable logic first.

#### Story 2.1.1: `PluginManifest` / `PluginTarget` / `InstalledPlugin` / `Registry` types
**As a** developer extending this module later, **I want** the exact JSON shapes for a
fetched manifest and the local registry defined once, **so that** every other story
(install, list, remove, status, config-wiring) shares one source of truth.
**Acceptance Criteria**:
- A manifest JSON string deserializes into `PluginManifest` with all fields populated.
  - *Given* the JSON string
    `{"name":"kibitzer-stub-plugin","version":"0.1.0","min_kibitzer_version":"0.1.13","severity":"advisory","scope":["**/*"],"triggers":["batch"],"output_format":"sarif","targets":{"x86_64-unknown-linux-gnu":{"url":"file:///tmp/fixtures/kibitzer-stub-plugin-x86_64-unknown-linux-gnu","sha256":"a3f5...64hex"}}}`,
    *When* parsed via `serde_json::from_str::<PluginManifest>`, *Then*
    `manifest.targets["x86_64-unknown-linux-gnu"].sha256 == "a3f5...64hex"` and
    `manifest.min_kibitzer_version == "0.1.13"`.
- `default_plugin_dir()` respects `XDG_DATA_HOME` exactly like `default_cache_path()`
  respects `XDG_CACHE_HOME`.
  - *Given* `XDG_DATA_HOME=/tmp/xdgdata` is set, *When* `default_plugin_dir()` is called,
    *Then* it returns `/tmp/xdgdata/kibitzer/plugins`.
- A malicious plugin name is rejected at construction, before it ever reaches a filesystem
  path (architecture-review.md BLOCKER).
  - *Given* the raw string `"../../etc"` (or `"foo/bar"`), *When* `PluginName::parse(s)` is
    called, *Then* it returns `Err` containing `"invalid plugin name"`, and — because
    `PluginAction`'s `name` fields are typed `PluginName` via a clap value parser (Task
    3.1.1a) — `kibitzer plugin install ../../etc --source ...`/`kibitzer plugin remove
    foo/bar` fail with a clap argument-parsing error printed to stderr and a non-zero exit,
    *before* `install_plugin`/`remove_plugin` is ever called and before any
    `default_plugin_dir().join(...)` or `fs::remove_dir_all` executes.
**Files**: `src/plugin.rs` (new)

##### Task 2.1.1a: Implement the `PluginName` smart constructor (~3 min)
- `#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)] #[serde(transparent)]
  struct PluginName(String)`; `PluginName::parse(s: &str) -> Result<Self>` accepts only
  strings matching `^[A-Za-z0-9_-]{1,64}$` (an allowlist, not a denylist — closes the door
  on `/`, `\`, `..`, and any future problem character at once), `anyhow::bail!("invalid
  plugin name: {s:?}")` otherwise. Add `impl AsRef<str> for PluginName` and `impl
  std::fmt::Display for PluginName` (delegating to the inner `String`) so callers can get a
  plain `&str`/`String` back for path joins and `Check.name` without re-exposing the
  unvalidated constructor. This is the parse-at-boundary fix for
  architecture-review.md's BLOCKER (unvalidated `name: String` flowing into
  `default_plugin_dir().join(name)` and `fs::remove_dir_all`).
- Files: `src/plugin.rs`

##### Task 2.1.1b: Create `src/plugin.rs` with `PluginManifest`/`PluginTarget` (~4 min)
- Define both structs (`#[derive(Debug, Clone, Deserialize)]`), field shapes exactly as in
  the Domain Glossary above. Reuse `crate::config::{Severity, OutputFormat}` for the
  `severity`/`output_format` fields rather than redefining them.
- Files: `src/plugin.rs`

##### Task 2.1.1c: Add `InstalledPlugin` and `Registry` (~4 min)
- `#[derive(Debug, Clone, Serialize, Deserialize)] struct InstalledPlugin { name:
  PluginName, version, min_kibitzer_version, sha256, binary_path: PathBuf, severity, scope,
  triggers, output_format }`; `#[derive(Debug, Default, Serialize, Deserialize)] struct
  Registry { plugins: Vec<InstalledPlugin> }`.
- Files: `src/plugin.rs`

##### Task 2.1.1d: Add `default_plugin_dir()`/`default_registry_path()` (~3 min)
- Mirror `cache::default_cache_path()` (`src/cache.rs:148-157`) exactly, substituting
  `XDG_DATA_HOME`/`.local/share` for `XDG_CACHE_HOME`/`.cache`, and `plugins`/`registry.json`
  for `cache.json`.
- Files: `src/plugin.rs`

##### Task 2.1.1e: Unit test manifest deserialization, `default_plugin_dir`, and `PluginName` (~5 min)
- Add `#[cfg(test)] mod tests` covering the Given/When/Then examples above: manifest
  round-trip; `XDG_DATA_HOME` override (set/restore the env var within the test using the
  same pattern `cache.rs`'s or `daemon.rs`'s existing env-var tests use, if one exists (grep
  first), otherwise a straightforward `std::env::set_var`/`remove_var` guarded by a
  `#[test]`-local mutex if tests run in parallel touch the same var); and `PluginName::parse`
  rejecting `"../../etc"`, `"foo/bar"`, and `""`, while accepting
  `"kibitzer-stub-plugin"`.
- Files: `src/plugin.rs`

#### Story 2.1.2: `Registry` persistence
**As a** developer, **I want** `Registry::load`/`Registry::save` to round-trip through a real
file, **so that** install/list/remove/status share one tested persistence path.
**Acceptance Criteria**:
- A saved registry loads back with identical content.
  - *Given* a `Registry { plugins: vec![InstalledPlugin { name: "kibitzer-stub-plugin",
    version: "0.1.0", .. }] }` saved via `Registry::save(path)` to a temp file, *When*
    `Registry::load(path)` reads that same path, *Then* the loaded `Registry.plugins` has
    length 1 and `plugins[0].name == "kibitzer-stub-plugin"`.
- Loading a nonexistent or corrupt registry file never errors.
  - *Given* `path` does not exist, *When* `Registry::load(path)` is called, *Then* it
    returns `Registry::default()` (empty `plugins`), matching `Cache::load`'s
    fail-open-to-empty precedent (`src/cache.rs:57-62`).
**Files**: `src/plugin.rs`

##### Task 2.1.2a: Implement `Registry::load`/`Registry::save` (~4 min)
- Mirror `Cache::load`/`Cache::save` (`src/cache.rs:57-70`) exactly: `load` reads-and-parses,
  falling back to `Self::default()` on any I/O or parse error; `save` creates parent dirs
  and writes pretty JSON (use `serde_json::to_string_pretty` since, unlike `cache.json`,
  `registry.json` is a small file a maintainer may reasonably inspect by hand).
- Files: `src/plugin.rs`

##### Task 2.1.2b: Unit test the round-trip and the empty-on-missing-file case (~3 min)
- Two `#[test]` functions covering the Given/When/Then examples above, writing to a real
  temp file (`std::env::temp_dir().join(...)` with a unique suffix, following
  `tests/hook_contract.rs`'s `TempRepo` uniqueness pattern at a smaller scale).
- Files: `src/plugin.rs`

#### Story 2.1.3: Version-compatibility comparator
**As a** developer, **I want** a correct, dependency-free version comparison, **so that**
`install_plugin` can enforce `min_kibitzer_version` without the classic string-comparison bug
(ADR-003).
**Acceptance Criteria**:
- A plugin requiring a newer kibitzer than what's running is correctly rejected; an older
  requirement is correctly accepted; multi-digit components compare numerically, not
  lexicographically.
  - *Given* `min = "0.2.0"`, `current = "0.1.13"`, *When* `version_meets_minimum(min, current)`
    is called, *Then* it returns `Ok(false)`.
  - *Given* `min = "0.1.9"`, `current = "0.1.13"`, *When* called, *Then* it returns `Ok(true)`.
  - *Given* `min = "0.9.0"`, `current = "0.10.0"`, *When* called, *Then* it returns `Ok(true)`
    (proves numeric, not lexicographic, comparison — `"0.10.0" < "0.9.0"` as strings would
    wrongly return `Ok(false)`).
- A short or prerelease-tagged version string is a clean `Err`, never a panic or a silent
  truncation.
  - *Given* `s = "1.2"` (too few components) or `s = "0.2.0-rc1"` (a non-numeric trailing
    component), *When* `parse_plain_version(s)` is called, *Then* it returns `Err` naming
    `s`, and the process does not panic.
**Files**: `src/plugin.rs`

##### Task 2.1.3a: Implement `parse_plain_version`/`version_meets_minimum` (~4 min)
- `parse_plain_version(s: &str) -> Result<(u64, u64, u64)>`: split on `.`, expect exactly 3
  parts, `u64::from_str` each, `anyhow::bail!` with the offending string on any failure.
  Anything that isn't exactly three dot-separated numeric components — a short form like
  `"1.2"`, a prerelease/build-metadata suffix like `"0.2.0-rc1"` or `"0.2.0+build5"`, or
  non-numeric input — must return a clear `Err` naming the offending string; it must never
  panic (no `.unwrap()`/indexing past a short split) and never silently truncate or ignore a
  trailing component. `version_meets_minimum(min: &str, current: &str) -> Result<bool>`:
  parse both, return `Ok(parse_plain_version(current)? >= parse_plain_version(min)?)`.
- Files: `src/plugin.rs`

##### Task 2.1.3b: Unit test all Given/When/Then cases (~4 min)
- Three `#[test]` functions for `version_meets_minimum`'s cases, plus negative cases for
  `parse_plain_version`: `"not-a-version"`, `"1.2"` (too few components), and `"0.2.0-rc1"`
  (prerelease suffix) all return `Err`, none panics.
- Files: `src/plugin.rs`

---

## Phase 3: Install/List/Remove/Status CLI Lifecycle

### Epic 3.1: CLI wiring
**Goal**: Add the `kibitzer plugin` subcommand tree, matching `Command::Daemon`'s existing
nested-subcommand shape.

#### Story 3.1.1: `Command::Plugin` / `PluginAction`
**As a** user, **I want** `kibitzer plugin install|list|remove|status` to exist and dispatch
correctly, **so that** the lifecycle functions from Phase 3's other epics are reachable from
the CLI.
**Acceptance Criteria**:
- `kibitzer plugin list` with zero plugins installed prints a clear, non-error message.
  - *Given* `default_registry_path()` doesn't exist on disk, *When*
    `kibitzer plugin list` runs, *Then* stdout is exactly `[kibitzer] no plugins installed`
    and the process exits `0`.
- A malicious/malformed plugin name is rejected by clap before any subcommand body runs
  (architecture-review.md BLOCKER — see also Story 2.1.1's `PluginName::parse` unit tests).
  - *Given* the invocation `kibitzer plugin remove ../../etc`, *When* clap parses the CLI
    args, *Then* it fails with a non-zero exit and an error on stderr naming the invalid
    value (clap's standard `value_parser` failure message), and neither
    `remove_plugin`/`fs::remove_dir_all` nor any other filesystem operation is ever invoked.
**Files**: `src/main.rs`

##### Task 3.1.1a: Add `Command::Plugin` and `PluginAction` (~4 min)
- Add `Plugin { #[command(subcommand)] action: PluginAction }` to `enum Command`
  (`src/main.rs:50-95`), immediately after the existing `Architecture` variant. Define
  `enum PluginAction { Install { #[arg(value_parser = PluginName::parse)] name: PluginName,
  #[arg(long)] source: String, #[arg(long)] force: bool }, List, Remove { #[arg(value_parser
  = PluginName::parse)] name: PluginName, #[arg(long)] force: bool }, Status { #[arg(value_parser
  = PluginName::parse)] name: Option<PluginName> } }` alongside `DaemonAction`
  (`src/main.rs:167-175`). Typing `name` as `PluginName` (Task 2.1.1a) rather than `String`
  means clap itself rejects `..`/`/`-containing or empty names at argument-parsing time,
  before `main()`'s dispatch runs — the fix for architecture-review.md's BLOCKER.
- Files: `src/main.rs`

##### Task 3.1.1b: Wire the `match` arm (~3 min)
- Add `Command::Plugin { action } => match action { ... }` to `fn main()`
  (`src/main.rs:177-`), calling `plugin::install_plugin`/`plugin::list_plugins`/
  `plugin::remove_plugin`/`plugin::plugin_status` (added in later stories — this task can
  stub `list_plugins`/`plugin_status` bodies with `todo!()` if written before Story 3.2.1
  lands, or be sequenced after; recommend implementing this task last within Epic 3.1 once
  the functions it calls exist).
- Files: `src/main.rs`

### Epic 3.2: `install_plugin` orchestration
**Goal**: The actual fetch → verify → place → register pipeline (ADR-002, ADR-003). This
epic adds the mechanism's only two new dependencies to the core binary's `Cargo.toml` —
`ureq` (HTTP) and `sha2` (checksum), both small and non-ML. `requirements.md`'s Success
Metrics scopes "no new dependencies" to mean no ML/heavy runtime (`ort`, model weights),
not literally zero — a real install-a-binary mechanism always needs some lightweight
HTTP+checksum capability to implement itself.

#### Story 3.2.1: Fetch manifest and target binary (HTTP or local path)
**As a** developer, **I want** one function that fetches bytes from either an HTTPS URL or a
local filesystem path, **so that** the automated test (local fixture) and the real install
path (GitHub Release) share the same downstream verify/place logic.
**Acceptance Criteria**:
- A local-path `source` is read directly, with no `ureq` call and no network access.
  - *Given* `source = "/tmp/fixtures/manifest.json"` (a plain absolute path, not a URL),
    *When* `fetch_manifest(source)` is called, *Then* it reads the file via `std::fs::read_to_string`
    and parses it as `PluginManifest`, and no `ureq::get` invocation occurs (verified by the
    test having no network access / no mock server running).
- An `https://` `source` is rejected if its host isn't on the allowlist.
  - *Given* `source = "https://evil.example.com/manifest.json"`, *When* `fetch_manifest(source)`
    is called, *Then* it returns `Err` with a message containing `"not an allowed host"`,
    before any `ureq::get` call is attempted.
- A redirect that lands off-allowlist is rejected too, not just the originally-requested URL
  (ADR-002: "reject any URL **or redirect target** outside that allowlist").
  - *Given* an allowlisted `source` URL whose server responds with a redirect to
    `https://evil.example.com/payload`, *When* `fetch_manifest(source)`/`fetch_target_bytes`
    completes the request, *Then* the code inspects the response's final, post-redirect URI
    via `ureq`'s `ResponseExt::get_uri()` (confirmed present on `ureq = "3"`'s `Response` type —
    `fn get_uri(&self) -> &Uri`, "The Uri that ultimately this Response is about. This can
    differ from the request uri when we have followed redirects," verified against
    `docs.rs/ureq/latest` at task-planning time, not assumed) and calls `is_allowed_host` on
    that final host too, returning `Err` containing `"not an allowed host"` if it fails —
    exactly like the pre-request check, so a redirect can't be used to smuggle bytes from an
    off-allowlist origin into a manifest/binary kibitzer trusts.
**Files**: `src/plugin.rs`

##### Task 3.2.1a: Implement `is_allowed_host` and `fetch_manifest(source: &str) -> Result<PluginManifest>` (~5 min)
- Implement `fn is_allowed_host(host: &str) -> bool` once — `PLUGIN_HOST_ALLOWLIST.contains(&host)
  || host.ends_with(".githubusercontent.com")` — as the single, shared implementation of
  ADR-002's allowlist rule (architecture-review.md Concern, Story 3.2.1: this check was
  previously going to be re-derived independently in both this task and Task 3.2.1c; both
  now call this one function instead).
- In `fetch_manifest`: branch on whether `source` parses as `http://`/`https://`; reject
  `http://` outright (HTTPS-only), then call `is_allowed_host` on the parsed host before
  `ureq::get(source).call()?`; on success, also call `is_allowed_host` on the *response's*
  final URI host via `ResponseExt::get_uri()` (`use ureq::ResponseExt;`) before trusting the
  body, rejecting with `"not an allowed host"` if the post-redirect host fails the check too
  (ADR-002's redirect-target requirement) — then `.into_string()?`. If `source` doesn't parse
  as `http(s)://`, treat it as a filesystem path and `std::fs::read_to_string`. Parse the
  resulting string as `PluginManifest` either way.
- Files: `src/plugin.rs`

##### Task 3.2.1b: Implement target-triple resolution (~3 min)
- `fn current_target_triple() -> String` using `std::env::consts::{ARCH, OS}` mapped to the
  four triples `dist-workspace.toml` already declares (`aarch64-apple-darwin`,
  `aarch64-unknown-linux-gnu`, `x86_64-apple-darwin`, `x86_64-unknown-linux-gnu`); bail with
  a clear "unsupported platform" error for anything else, per `research/features.md`'s
  edge-case table ("No published artifact for the user's OS/arch").
- Files: `src/plugin.rs`

##### Task 3.2.1c: Implement `fetch_target_bytes(target: &PluginTarget) -> Result<Vec<u8>>` (~3 min)
- Same HTTP-vs-local branch as Task 3.2.1a, applied to `target.url` (a `PluginTarget`'s
  `url` may itself be a `file://`-prefixed or plain local path in the manifest, for the
  automated test's fixture — strip a `file://` prefix if present before treating as a path).
  Reuse Task 3.2.1a's `is_allowed_host` for both the pre-request and post-redirect host
  checks rather than re-deriving the allowlist rule here — this was the second of the two
  independent copies architecture-review.md flagged; there is now exactly one.
- Files: `src/plugin.rs`

#### Story 3.2.2: Verify, place, and register
**As a** user, **I want** `plugin install` to refuse a corrupted/incompatible download and
never leave a partial file behind, **so that** I can trust a successful install (ADR-002,
ADR-003).
**Acceptance Criteria**:
- A manifest whose own `name` field disagrees with the CLI-supplied name is refused, rather
  than silently registered under either name (a real correctness gap: without this check, `kibitzer
  plugin install foo --source ...` against a manifest describing a plugin called `bar` would
  install and register successfully under the CLI-supplied name `foo`, silently discarding the
  manifest's own claim about what it is).
  - *Given* the CLI-supplied name is `"foo"` and the fetched manifest's `name` field is
    `"bar"`, *When* `install_plugin` runs, *Then* it returns `Err` containing `"manifest name
    does not match"` before any version-compat check, download, or `Registry` write occurs,
    and `Registry::load(default_registry_path())` has zero plugins named `"foo"` afterward.
- A checksum mismatch hard-fails and leaves no trace.
  - *Given* a manifest's `targets["x86_64-unknown-linux-gnu"].sha256` doesn't match the
    actual SHA-256 of the fetched bytes, *When* `install_plugin("kibitzer-stub-plugin", source,
    false)` runs, *Then* it returns `Err` containing `"checksum mismatch"`, no file exists
    under `default_plugin_dir().join("kibitzer-stub-plugin")`, and `Registry::load(default_registry_path())`
    has zero plugins.
- A version-incompatible plugin is refused before any download.
  - *Given* the manifest's `min_kibitzer_version = "99.0.0"` (higher than
    `env!("CARGO_PKG_VERSION")`), *When* `install_plugin` runs, *Then* it returns `Err`
    containing `"requires kibitzer"`, and no `fetch_target_bytes` call is ever made (the
    version gate runs before the binary fetch).
- Re-installing the same version is a no-op that still exits 0.
  - *Given* `kibitzer-stub-plugin` v0.1.0 is already installed, *When*
    `install_plugin("kibitzer-stub-plugin", source, false)` runs again with a manifest whose
    `version` is still `"0.1.0"`, *Then* it returns `Ok(())`, prints
    `already installed (v0.1.0) — no changes made`, and performs no download or registry
    write.
- A version bump prints an explicit upgrade announcement rather than silently overwriting
  (design/ux.md; locks the exact wording against implementation drift).
  - *Given* `kibitzer-stub-plugin` v0.1.0 is already installed, *When*
    `install_plugin("kibitzer-stub-plugin", source, false)` runs again with a manifest whose
    `version` is `"0.2.0"`, *Then* stdout contains the literal line
    `[kibitzer] upgrading kibitzer-stub-plugin v0.1.0 -> v0.2.0`, install proceeds through the
    normal fetch/verify/place/register steps, and `Registry::load(default_registry_path())`
    afterward shows exactly one entry named `"kibitzer-stub-plugin"` at `version == "0.2.0"`.
- Re-installing the same version with `--force` restates the consequence rather than
  printing an identical unqualified success line (design/ux.md's stated principle for forced
  actions, already applied to `remove --force`).
  - *Given* `kibitzer-stub-plugin` v0.1.0 is already installed, *When*
    `install_plugin("kibitzer-stub-plugin", source, true)` runs again with a manifest whose
    `version` is still `"0.1.0"`, *Then* stdout contains the literal line
    `[kibitzer] kibitzer-stub-plugin v0.1.0 already installed — reinstalling due to --force`,
    and (unlike the non-`--force` no-op case) the normal fetch/verify/place/register steps do
    run.
**Files**: `src/plugin.rs`

##### Task 3.2.2a: Implement the manifest-name, version-compat, and already-installed gates (~6 min)
- At the top of `install_plugin`, immediately after fetching the manifest: first, compare
  `manifest.name` against the CLI-supplied `name: &PluginName` (`.as_ref()`); if they differ,
  `anyhow::bail!("manifest name does not match: CLI name '{name}', manifest name
  '{}'", manifest.name)` before anything else runs — the manifest's own claim about what
  plugin it is must agree with what the user asked to install (closes the correctness gap:
  previously nothing checked this, and `manifest.name` was only ever used-and-discarded in
  favor of the CLI name at registration time, so an install could silently succeed under a
  name that didn't match the downloaded content). Then call
  `version_meets_minimum(&manifest.min_kibitzer_version, env!("CARGO_PKG_VERSION"))`, bail
  with a clear message on `false`; then load the `Registry`. If an entry with the same `name`
  and `version` already exists: without `--force`, short-circuit with the "already installed"
  message (no download, no registry write); *with* `--force`, print
  `[kibitzer] <name> v<version> already installed — reinstalling due to --force` and fall
  through to the normal fetch/verify/place/register steps below, so a forced re-run restates
  the consequence instead of printing the same unqualified success line as an ordinary first
  install (design/ux.md's stated principle for forced actions, already applied to `remove
  --force`). If an entry with the same `name` exists but a *different* `version`, print
  `[kibitzer] upgrading <name> v<old> -> v<new>` before falling through to the normal
  fetch/verify/place/register steps below — otherwise a version bump silently overwrites
  with no announcement (design/ux.md).
- Files: `src/plugin.rs`

##### Task 3.2.2b: Implement checksum verification (~4 min)
- `use sha2::{Sha256, Digest}`; hash the fetched bytes, hex-encode, compare against
  `target.sha256` (case-insensitive compare). Bail with the expected-vs-actual hashes in the
  message on mismatch, before any filesystem write under `default_plugin_dir()`.
- Files: `src/plugin.rs`, `Cargo.toml` (add `sha2 = "0.11"`)

##### Task 3.2.2c: Write the binary to a temp path, then rename + `chmod +x` (~4 min)
- Write fetched bytes to a temp file in the same directory as the final destination
  (`default_plugin_dir().join(name.as_ref())`, where `name: &PluginName` — already
  validated by clap at Task 3.1.1a's boundary, so this join can never escape
  `default_plugin_dir()`), then `fs::rename` into `.../<name>/<name>` and, on Unix,
  `fs::set_permissions` with the executable bit set (`std::os::unix::fs::PermissionsExt`) —
  only after the checksum check in Task 3.2.2b passes, per ADR-002's ordering requirement.
- Files: `src/plugin.rs`

##### Task 3.2.2d: Write the `Registry` entry (~3 min)
- Build an `InstalledPlugin` from the CLI-supplied `PluginName` (not `manifest.name`, which
  is untrusted download content) + the rest of the manifest + resolved `binary_path` +
  verified `sha256`; `Registry::load` → replace-by-name-or-push → `Registry::save`, printing
  a success line that also states the disclosure mitigation from Step 0.5 ("now runs in
  every repo on this machine; disable per-repo via `disabled` in `.claude/inspect.json`, or
  `kibitzer plugin remove kibitzer-stub-plugin` to remove it everywhere").
- Files: `src/plugin.rs`

##### Task 3.2.2e: Add `ureq` and wire the new dependencies into `Cargo.toml` (~2 min)
- Add `ureq = "3"` to `[dependencies]` (`Cargo.toml:14-32`) alongside `sha2` (added in Task
  3.2.2b). Run `cargo build` to confirm both resolve cleanly against `rustc 1.98.0`.
- Files: `Cargo.toml`

### Epic 3.3: `list` / `remove` / `status`
**Goal**: The remaining three subcommands, reusing the `Registry` from Epic 3.2.

#### Story 3.3.1: `list` and `status`
**As a** user, **I want** to see what's installed and whether it's actually runnable, **so
that** I can diagnose a broken plugin without reading source.
**Acceptance Criteria**:
- `status` reports a missing binary distinctly from a healthy one.
  - *Given* `kibitzer-stub-plugin` v0.1.0 is registered but its file at
    `default_plugin_dir().join("kibitzer-stub-plugin").join("kibitzer-stub-plugin")` has been
    manually deleted, *When* `kibitzer plugin status kibitzer-stub-plugin` runs, *Then*
    stdout contains `binary missing`.
  - *Given* the binary is present and its SHA-256 still matches the registry's recorded
    `sha256`, *When* `kibitzer plugin status kibitzer-stub-plugin` runs, *Then* stdout
    contains `ok`.
**Files**: `src/plugin.rs`

##### Task 3.3.1a: Implement `list_plugins` and its CLI rendering (~3 min)
- `list_plugins() -> Result<Vec<InstalledPlugin>>` (just `Registry::load(...).plugins`);
  `main.rs`'s `PluginAction::List` arm prints one line per plugin
  (`"{name} v{version} — {binary_path}"`) or the empty-list message from Story 3.1.1.
- Files: `src/plugin.rs`, `src/main.rs`

##### Task 3.3.1b: Implement `plugin_status`/`PluginStatusReport` (~4 min)
- Look up the named `InstalledPlugin` (bail "no plugin named ... installed" if absent, per
  `research/ux.md`'s vocabulary-consistency recommendation with `checker::lookup`'s existing
  phrasing at `src/main.rs:232`); check `binary_path.exists()`, re-hash it if present and
  compare to the recorded `sha256`, and re-run `version_meets_minimum` against the
  currently-running `CARGO_PKG_VERSION`. Render as `ok` when all three hold, otherwise the
  specific failing dimension(s).
- Files: `src/plugin.rs`, `src/main.rs`

#### Story 3.3.2: `remove`
**As a** user, **I want** `plugin remove` to refuse when a live check config still points at
the plugin, **so that** I don't accidentally leave a dangling `command` reference.
**Acceptance Criteria**:
- Refuses by default when referenced; proceeds with `--force`.
  - *Given* the current directory's `.claude/inspect.json` has a hand-authored check entry
    named `"kibitzer-stub-plugin"`, *When* `kibitzer plugin remove kibitzer-stub-plugin` runs
    (no `--force`), *Then* it exits non-zero, prints a message containing
    `is referenced by check`, and the plugin remains in `Registry::load(default_registry_path())`.
  - *Given* the same setup, *When* `kibitzer plugin remove kibitzer-stub-plugin --force` runs,
    *Then* it exits `0`, the registry no longer contains an entry named
    `"kibitzer-stub-plugin"`, and `default_plugin_dir().join("kibitzer-stub-plugin")` no
    longer exists on disk.
**Files**: `src/plugin.rs`

##### Task 3.3.2a: Implement the `.claude/inspect.json` reference guard (~4 min)
- Reuse `config::find_config(&std::env::current_dir()?)` to read the *local* config (not
  `find_effective_config`, which would already include the plugin's own synthesized
  `Check`); if any `local.checks` entry's `name` matches `name.as_ref()`, bail with the
  message above unless `force`.
- Files: `src/plugin.rs`

##### Task 3.3.2b: Implement `remove_plugin`'s deletion + registry update (~3 min)
- `fs::remove_dir_all(default_plugin_dir().join(name.as_ref()))` (ignore `NotFound`), where
  `name: &PluginName` — already validated by clap at Task 3.1.1a's boundary, closing
  architecture-review.md's BLOCKER (an unvalidated raw name reaching this recursive
  delete) — then `Registry::load` → filter out the named entry → `Registry::save`.
- Files: `src/plugin.rs`, `src/main.rs`

---

## Phase 4: Effective-Config Wiring & Tech Debt Remediation

### Epic 4.1: Auto-inject plugin checks into `find_effective_config`
**Goal**: The one-line change that makes an installed plugin actually run, per Step 0.5's
chosen approach.

#### Story 4.1.1: `registered_plugin_checks()` + the `find_effective_config` chain
**As an** agent or human running any kibitzer entry point, **I want** an installed plugin's
check to appear in the effective config automatically, **so that** the success metric ("wires
it into the effective check config... without the user hand-editing `.claude/inspect.json`")
is literally true.
**Acceptance Criteria**:
- An installed plugin's check appears alongside every default.
  - *Given* `Registry::load(default_registry_path())` contains one `InstalledPlugin` named
    `"kibitzer-stub-plugin"` and no `.claude/inspect.json` exists above the test directory,
    *When* `find_effective_config(&test_dir)` is called, *Then* `config.checks` contains a
    `Check` with `name == "kibitzer-stub-plugin"` in addition to every entry
    `default_checks()` produces.
- Zero plugins installed leaves default behavior byte-for-byte unchanged.
  - *Given* `default_registry_path()` doesn't exist (or `Registry::load` returns an empty
    `Registry`), *When* `find_effective_config(&test_dir)` is called, *Then* `config.checks`'
    names, in order, exactly match `default_checks()`'s names, in order — proving the success
    metric "uninstalling/never installing the plugin leaves... `default_checks()` catalog
    completely unchanged."
**Files**: `src/config.rs`, `src/plugin.rs`

##### Task 4.1.1a: Implement `registered_plugin_checks() -> Vec<Check>` (~4 min)
- Load the `Registry`; map each `InstalledPlugin` to a `Check { name: p.name.to_string(),
  command: Some(format!("{} {{file}}", p.binary_path.display())), checker: None,
  architecture_checker: None, severity: p.severity, scope: p.scope.clone(), triggers:
  p.triggers.clone(), message: None, output_format: p.output_format }` — mirrors
  `native_check()`'s helper shape (`config.rs:512-524`). `p.name.to_string()` is used because
  `Check.name` is a plain `String`; `PluginName`'s `Display` impl (Task 2.1.1a) hands back
  the already-validated string.
- Files: `src/plugin.rs`

##### Task 4.1.1b: Chain it into `find_effective_config` (~3 min)
- At `config.rs:628-650`, replace both `default_checks()` call sites' direct use with
  `default_checks().into_iter().chain(crate::plugin::registered_plugin_checks()).collect::<Vec<_>>()`,
  feeding that combined `Vec` into `merge_checks`/the `None` branch exactly where
  `default_checks()` is used today — a one-line-per-branch change, per
  `research/architecture.md` §2(b)'s recommended shape.
- Files: `src/config.rs`

##### Task 4.1.1c: Unit tests for both Given/When/Then cases (~4 min)
- Add two `#[cfg(test)]` tests in `config.rs`'s existing `mod tests` (`config.rs:652-`),
  using a temp `XDG_DATA_HOME` (via `std::env::set_var` scoped to the test, following
  whatever env-guarding pattern Task 2.1.1d established) to point `default_registry_path()`
  at a controlled fixture registry.json for the "one plugin installed" case, and an
  unset/empty one for the "zero plugins" case.
- Files: `src/config.rs`

#### Story 4.1.2: Fold `registry.json` into the daemon's cache invalidation
**As an** agent calling the persistent `kibitzer daemon` (the primary path for Claude Code
hook/MCP calls), **I want** a `plugin install`/`remove` to invalidate cached results on the
very next request, **so that** I never see a stale check list just because the daemon wasn't
restarted (pre-mortem.md Failure #2, P1).
**Acceptance Criteria**:
- A newly-installed plugin's check is picked up by a running daemon without a restart.
  - *Given* `kibitzer daemon start` is running against a repo and has already cached a
    result for some file under trigger `"batch"`, *When* `kibitzer plugin install
    kibitzer-stub-plugin --source <fixture>` runs (rewriting `registry.json`) and the same
    file is then re-checked through the daemon, *Then* the response's check list includes
    `kibitzer-stub-plugin`'s synthesized `Check`, with no `kibitzer daemon stop`/`start` in
    between.
- A `plugin remove` is likewise observed on the next request.
  - *Given* the same running daemon with `kibitzer-stub-plugin` now installed and cached,
    *When* `kibitzer plugin remove kibitzer-stub-plugin --force` runs, *Then* the next
    check request through the daemon no longer includes `kibitzer-stub-plugin`'s `Check`
    and does not attempt to invoke its now-removed binary.
- Cache behavior for repos with no plugins installed is unchanged.
  - *Given* `default_registry_path()` doesn't exist, *When* `Cache::get`/`put` run, *Then*
    their hit/miss decisions are identical to before this story — `registry_stamp` being
    `None` on both sides of the comparison never itself causes a miss.
**Files**: `src/cache.rs`, `src/daemon.rs`

##### Task 4.1.2a: Add `registry_stamp` to `CacheEntry` and thread it through `Cache::get`/`put` (~4 min)
- Add `registry_stamp: Option<Stamp>` to `CacheEntry` (`src/cache.rs:33-42`), alongside the
  existing `config_stamp: Stamp`. Change `Cache::get`/`put` (`src/cache.rs:72-110`) to accept
  a second path — `registry_path: &Path` — fingerprint it with the existing `stamp()` helper
  (which already returns `None` for a nonexistent file, matching "no plugins installed"),
  and compare/store it the same way `config_stamp` is: a cache hit now also requires
  `entry.registry_stamp == stamp(registry_path)`.
- Files: `src/cache.rs`

##### Task 4.1.2b: Pass `default_registry_path()` at both `daemon.rs` call sites (~3 min)
- At `src/daemon.rs:149-174` and `src/daemon.rs:240-255` (the two `cache.get`/`cache.put`
  call sites the Tech Debt Disposition row cites), pass
  `&crate::plugin::default_registry_path()` as the new `registry_path` argument alongside
  the existing `config_path`.
- Files: `src/daemon.rs`

##### Task 4.1.2c: Test that install/remove invalidates a live cache entry (~4 min)
- Extend `src/cache.rs`'s test module with a case that: `put`s a cache entry while
  `registry.json` doesn't exist (`registry_stamp == None`), then writes a `registry.json`
  file at that path and calls `get` again, asserting it now misses (proves an install
  invalidates); and a companion case where `registry.json`'s content and stamp are
  unchanged between `put` and `get`, asserting it still hits (proves no unrelated
  regression to the existing `config_stamp`-only cache-hit behavior).
- Files: `src/cache.rs`

### Epic 4.2: Command-dispatch timeout (Tech Debt item a)
**Goal**: A wall-clock timeout on `run_check`'s `sh -c` dispatch, fixed once at the shared
call site (Tech Debt Disposition: Refactor-first).

#### Story 4.2.1: Timeout + kill on hang
**As a** user (or agent) running checks, **I want** a hung `command` check to fail loudly
after a bounded time instead of blocking forever, **so that** a misbehaving plugin (or any
hand-configured command) can't freeze the whole check run.
**Acceptance Criteria**:
- A command that never exits is killed and reported as a timeout, not left hanging.
  - *Given* a `Check` with `command: "sleep 60"` and (for the test only) `COMMAND_TIMEOUT`
    reduced to `Duration::from_millis(200)` via a test-only override, *When* `run_check` is
    called, *Then* it returns within ~1 second (not 60), `result.passed == false`, and
    `result.message`/`result.output` contains the substring `timed out`.
- A normal, fast-exiting command is completely unaffected.
  - *Given* a `Check` with `command: "true"`, *When* `run_check` is called, *Then* it
    behaves exactly as before this change (`result.passed == true`, no timeout-related text
    anywhere in `result.output`).
**Files**: `src/check.rs`

##### Task 4.2.1a: Extract a `run_command_with_timeout` helper (~15-20 min)
- Estimate revised up from an earlier ~5 min: this is concurrency-sensitive (a
  thread/channel/kill race, not a straightforward refactor) with a known, already-documented
  incomplete edge case below — ~5 min was optimistic for getting the thread/channel/kill
  handshake right and writing a test that actually exercises the timeout path.
- Replace `Command::new("sh").arg("-c").arg(&cmd_str).current_dir(repo_root).output()?`
  (`src/check.rs:156-160`) with a helper that: spawns the child with piped stdout/stderr,
  records `let pid = child.id();` *before* moving `child` into a thread that calls
  `child.wait_with_output()` (reusing `wait_with_output`'s own correct concurrent
  pipe-draining — avoiding a hand-rolled deadlock risk), sends the result over an
  `mpsc::channel`, and the caller does `rx.recv_timeout(COMMAND_TIMEOUT)`; on
  `RecvTimeoutError::Timeout`, shells out to
  `Command::new("kill").arg("-KILL").arg(pid.to_string()).status()` (Unix; matches
  `daemon.rs`'s existing Unix-only assumption) — which unblocks the thread's
  `wait_with_output()` so it doesn't leak — and returns a distinct timeout error.
  Document the one accepted limitation explicitly in a comment: if the child writes more
  than the OS pipe buffer (~64KB on Linux) before exiting, it can block on write() before
  the timeout thread notices — acceptable given today's checks all produce small,
  single-file lint output. Also document a second, related known v1 limitation: `kill -KILL
  <pid>` only signals the direct child (`sh`), so a compound/piped command string (e.g. `foo
  | bar`) can leave orphaned grandchild processes running after the timeout fires — closing
  this fully would need process-group/session-based killing (e.g. `setsid` + `killpg`),
  which is out of scope for this pass.
- Files: `src/check.rs`

##### Task 4.2.1b: Wire the helper into `run_check` (~3 min)
- Replace the direct `.output()?` call at `src/check.rs:156-160` with the new helper,
  keeping everything downstream (`passed_raw`, SARIF branch, etc.) unchanged — the helper
  returns the same `std::process::Output`-shaped data on success.
- Files: `src/check.rs`

##### Task 4.2.1c: Add the two tests from the story's acceptance criteria (~4 min)
- Add `COMMAND_TIMEOUT` as a `const` with a `#[cfg(test)]`-only override mechanism (e.g. a
  thread-local or a parameterized private helper `run_check_with_timeout(check, repo_root,
  file_path, changed_lines, timeout)` that `run_check` calls with the real constant, and
  tests call directly with a short one) — add both tests to `check.rs`'s existing test
  module.
- Files: `src/check.rs`

### Epic 4.3: Legible plugin-not-installed signal (Tech Debt item c)
**Goal**: A missing plugin binary produces a distinct, actionable signal instead of raw shell
noise (Tech Debt Disposition: Isolate via seam).

#### Story 4.3.1: Preflight detection + distinct rendering
**As an** agent consuming `list_checks`/`run_checks` over MCP, **I want** to tell "plugin not
installed" apart from "check ran and failed," **so that** I don't misdiagnose a missing
install as a code defect.
**Acceptance Criteria**:
- `list_checks` tags a plugin-backed check whose binary is missing.
  - *Given* `kibitzer-stub-plugin` is registered in `Registry` but its `binary_path` doesn't
    exist on disk, *When* the MCP `list_checks` tool renders the effective config
    (`src/mcp.rs:272-289`), *Then* the line for `"kibitzer-stub-plugin"` contains the literal
    substring `plugin=not-installed`.
- `run_checks` reports `[skipped]`, not `[Blocking]`/`[Advisory]`, for the same case.
  - *Given* the same missing-binary setup, *When* `run_checks_for_trigger` runs that check,
    *Then* the resulting `CheckResult.plugin_missing == true`, and `mcp.rs::run_checks`'s
    rendered failure line for it starts with `[skipped]` rather than `[Advisory]`/`[Blocking]`.
**Files**: `src/check.rs`, `src/mcp.rs`, `src/hook.rs`, `src/plugin.rs`

##### Task 4.3.1a: Implement `missing_binary_for(check_name: &str) -> Option<PathBuf>` (~3 min)
- Load the `Registry`, find an `InstalledPlugin` whose `name == check_name`, return
  `Some(binary_path)` iff `!binary_path.exists()`, else `None` (including when no such
  plugin is registered at all — a hand-authored, non-plugin check always returns `None`,
  leaving it fully unaffected).
- Files: `src/plugin.rs`

##### Task 4.3.1b: Add `CheckResult.plugin_missing` and the `run_check` early return (~4 min)
- Add `#[serde(default)] pub plugin_missing: bool` to `CheckResult` (`src/check.rs:20-52`),
  following the same `#[serde(default)]`-for-cache-compatibility reasoning already documented
  on `command`/`findings`. At the top of `run_check` (`src/check.rs:140-`), before building
  `cmd_str`, call `crate::plugin::missing_binary_for(&check.name)`; if `Some(path)`, return
  early with `CheckResult { plugin_missing: true, passed: false, severity:
  Severity::Advisory, message: Some(format!("plugin '{}' is not installed (expected binary
  at {}) — run `kibitzer plugin install {}`", check.name, path.display(), check.name)),
  output: String::new(), command: String::new(), .. }` (severity forced to `Advisory`
  regardless of `check.severity`, so a missing install never blocks).
- Files: `src/check.rs`

##### Task 4.3.1c: Update `mcp.rs::run_checks` and `hook.rs`'s rendering (~4 min)
- At `src/mcp.rs:485`, branch: `if r.plugin_missing { format!("[skipped] {}: {}", r.check_name,
  r.describe()) } else { format!("[{:?}] {}: {}", r.severity, r.check_name, r.describe()) }`.
  `hook.rs` has no severity-bracket precedent to mirror (verified: its non-blocking context
  builder at `src/hook.rs:182-186` renders bare `"{check_name}: {describe}"`, and its
  blocking path at `src/hook.rs:167-179` prints `"[kibitzer] {name} (blocking): ..."` — a
  `plugin_missing` result is always forced to `Severity::Advisory` by Task 4.3.1b, so it can
  never reach the blocking branch at all). Add the same `if r.plugin_missing` branch to the
  `src/hook.rs:182-186` `.map()` closure only, prefixing with `"[skipped] "` when true.
- Files: `src/mcp.rs`, `src/hook.rs`

##### Task 4.3.1d: Update `mcp.rs::list_checks` to append the tag (~3 min)
- At `src/mcp.rs:279`, after building each check's line, append
  `if plugin::missing_binary_for(&c.name).is_some() { ", plugin=not-installed" } else { "" }`
  before joining.
- Files: `src/mcp.rs`

---

## Phase 5: Testing, Docs, and Demo

### Epic 5.1: Automated end-to-end test
**Goal**: One test proving install → register → run works, per the requirements' Success
Metrics, extending `tests/hook_contract.rs`'s real-subprocess pattern.

#### Story 5.1.1: `install → register → run` via `CARGO_BIN_EXE` harness
**As a** maintainer, **I want** CI to catch a regression in any of install/register/run
without a real network call, **so that** the mechanism stays proven on every PR.
**Acceptance Criteria**:
- A full local-fixture install followed by a batch run surfaces the stub's finding.
  - *Given* a fresh temp repo with no `.claude/inspect.json`, an isolated `XDG_DATA_HOME`
    pointing at a private temp dir, and a manifest fixture built from the real, compiled
    `kibitzer-stub-plugin` binary (via `env!("CARGO_BIN_EXE_kibitzer-stub-plugin")`) with a
    correctly-computed SHA-256 in its `targets` entry, *When* the test spawns
    `env!("CARGO_BIN_EXE_kibitzer")` as `kibitzer plugin install kibitzer-stub-plugin
    --source <fixture-manifest-path>` and then `kibitzer run <repo> --trigger batch` (both
    with the same `XDG_DATA_HOME` env var), *Then* the second command's stdout contains
    `kibitzer-stub-plugin-finding` (the stub's canned SARIF `ruleId`).
**Files**: `tests/plugin_contract.rs` (new), `crates/kibitzer-stub-plugin/Cargo.toml`

##### Task 5.1.1a: Expose `kibitzer-stub-plugin` as a dev-dependency of `kibitzer` (~2 min)
- Add `kibitzer-stub-plugin = { path = "crates/kibitzer-stub-plugin" }` under
  `[dev-dependencies]` in the root `Cargo.toml`, so Cargo auto-exposes
  `CARGO_BIN_EXE_kibitzer-stub-plugin` to `kibitzer`'s own integration tests (per
  `research/stack.md` §9's confirmed mechanism — no separate build step needed).
- Files: `Cargo.toml`

##### Task 5.1.1b: Write `tests/plugin_contract.rs`'s fixture setup (~5 min)
- Following `tests/hook_contract.rs`'s `TempRepo` shape (unique temp dirs keyed by pid +
  counter): copy `env!("CARGO_BIN_EXE_kibitzer-stub-plugin")`'s binary bytes to a fixture
  path, compute its real SHA-256 (reuse `sha2` directly in the test), and write a
  `PluginManifest`-shaped JSON manifest file pointing at that fixture path as the
  `targets[<current-triple>].url` (a plain local path, exercising Task 3.2.1a/c's
  local-file branch).
- Files: `tests/plugin_contract.rs`

##### Task 5.1.1c: Write the install+run assertions (~4 min)
- Spawn `kibitzer plugin install kibitzer-stub-plugin --source <manifest-path>` as a real
  subprocess (`Command::new(env!("CARGO_BIN_EXE_kibitzer"))`, with `XDG_DATA_HOME` set to
  a private temp dir per `TempRepo`'s existing `XDG_CACHE_HOME` isolation pattern), assert
  exit `0`; then spawn `kibitzer run <dir> --trigger batch` with the same env, assert exit
  reflects the stub's advisory-severity finding and stdout contains
  `kibitzer-stub-plugin-finding`.
- Files: `tests/plugin_contract.rs`

#### Story 5.1.2: Two independently-registered plugins both surface distinctly
**As a** maintainer, **I want** proof that this mechanism is the generic extension point
requirements.md's Scope requires ("not special-cased to only the embedding checker"), **so
that** a future second plugin isn't discovered to silently collide with the first one only
after it ships.
**Acceptance Criteria**:
- Installing two differently-named plugins produces two distinct, independent `Check`s.
  - *Given* `kibitzer-stub-plugin` is installed under the CLI name `kibitzer-stub-plugin`
    (canned finding `ruleId: "kibitzer-stub-plugin-finding"`) and a second, differently-shaped
    stub binary is installed under the CLI name `kibitzer-stub-plugin-second` (canned finding
    `ruleId: "kibitzer-stub-plugin-second-finding"`), *When* `kibitzer plugin list` and
    `kibitzer run <dir> --trigger batch` are run against the same repo, *Then* `plugin
    list`'s output names both plugins, `find_effective_config`'s `config.checks` contains
    two distinct entries (`"kibitzer-stub-plugin"` and `"kibitzer-stub-plugin-second"`, not
    one overwriting the other), and the batch run's stdout contains both
    `kibitzer-stub-plugin-finding` and `kibitzer-stub-plugin-second-finding`.
**Files**: `crates/kibitzer-stub-plugin/Cargo.toml`, `crates/kibitzer-stub-plugin/src/bin/second.rs` (new), `tests/plugin_contract.rs`

##### Task 5.1.2a: Add a second trivial binary target to `kibitzer-stub-plugin` (~3 min)
- Add `crates/kibitzer-stub-plugin/src/bin/second.rs` — a near-copy of Task 1.1.2b's
  `main.rs` with only the `ruleId`/`tool.driver.name` strings changed to
  `"kibitzer-stub-plugin-second"` — and register it as a second `[[bin]]` in
  `crates/kibitzer-stub-plugin/Cargo.toml`, named `kibitzer-stub-plugin-second`. No shared
  logic extraction needed for a throwaway fixture this small. Cargo's dev-dependency
  mechanism (Task 5.1.1a) exposes `CARGO_BIN_EXE_kibitzer-stub-plugin-second` automatically
  once this exists — no separate build wiring.
- Files: `crates/kibitzer-stub-plugin/Cargo.toml`, `crates/kibitzer-stub-plugin/src/bin/second.rs`

##### Task 5.1.2b: Extend `tests/plugin_contract.rs` to install both and assert independence (~4 min)
- Reusing Task 5.1.1b's fixture-manifest pattern, build a second manifest fixture from
  `env!("CARGO_BIN_EXE_kibitzer-stub-plugin-second")` and install it under the CLI name
  `kibitzer-stub-plugin-second` (same `XDG_DATA_HOME`, same temp repo, after the first
  plugin's install from Task 5.1.1c). Assert `kibitzer plugin list`'s stdout names both
  plugins, and that `kibitzer run <dir> --trigger batch`'s stdout contains both canned
  `ruleId`s from the acceptance criterion above — proving `registered_plugin_checks()`
  produces two independent `Check` entries rather than one overwriting the other by name
  collision or by only ever reading the last `Registry` entry.
- Files: `tests/plugin_contract.rs`

### Epic 5.2: Documentation
**Goal**: A doc-per-mechanism page (matching `docs/output-formats.md`'s convention) plus the
one-sentence `CLAUDE.md` mention `research/features.md` flags as a real convention-violation
risk if skipped.

#### Story 5.2.1: `docs/plugins.md` + `CLAUDE.md` mention
**As a** future reader (including future-tstapler), **I want** the plugin mechanism
documented in the same place/style as every other kibitzer mechanism, **so that** it's
discoverable the same way `output-formats.md`/`suppressing-checks.md` are.
**Acceptance Criteria**:
- The doc exists and `CLAUDE.md` links it.
  - *Given* `docs/plugins.md` doesn't exist yet, *When* it's written (title, one-paragraph
    problem statement, an `install`/`list`/`remove`/`status` example block, a "known
    limitations" section covering the daemon-cache staleness item from Tech Debt Disposition),
    *Then* `CLAUDE.md`'s existing "Default check catalog" paragraph gains one sentence
    naming `docs/plugins.md`, in the same style it already names `comment-quality-<lang>`
    and `syntax-rules-<lang>` by name.
**Files**: `docs/plugins.md` (new), `CLAUDE.md`

##### Task 5.2.1a: Write `docs/plugins.md` (~5 min)
- Follow `docs/output-formats.md`'s structure: title, a short problem-statement paragraph,
  a fenced example of `kibitzer plugin install kibitzer-stub-plugin --source <manifest-url>`
  through `kibitzer plugin list`/`status`/`remove`, and a "Known limitations" section stating
  the daemon-cache-staleness accepted limitation (Tech Debt Disposition, item b) in one or
  two sentences with the exact remediation ("restart the daemon: `kibitzer daemon stop`").
- Files: `docs/plugins.md`

##### Task 5.2.1b: Add the `CLAUDE.md` mention (~2 min)
- Add one sentence to the "Default check catalog" paragraph in
  `/home/tstapler/code/github.com/tstapler/kibitzer/CLAUDE.md`, naming `docs/plugins.md` the
  same way `comment-quality-<lang>` and `syntax-rules-<lang>` are already named there.
- Files: `CLAUDE.md`

### Epic 5.3: Manual demo capture
**Goal**: The live terminal walkthrough the Success Metrics require as a PR-body artifact,
distinct from the automated test.

#### Story 5.3.1: Terminal walkthrough script for the PR body
**As a** reviewer (or future-tstapler) reading the PR, **I want** real, pasted terminal
output showing install → run → finding → remove, **so that** the feature's actual behavior
is demonstrated, not narrated.
**Acceptance Criteria**:
- The PR body contains a real, unedited terminal transcript, not prose summarizing it.
  - *Given* a machine with no `kibitzer-stub-plugin` currently installed (a fresh
    `$XDG_DATA_HOME` or an explicit `kibitzer plugin remove kibitzer-stub-plugin` first),
    *When* the maintainer runs, in order: `kibitzer plugin install kibitzer-stub-plugin
    --source <real-or-local-manifest>`, `kibitzer plugin list`, `kibitzer run . --trigger
    batch` (showing the finding appear), `kibitzer plugin remove kibitzer-stub-plugin`,
    `kibitzer run . --trigger batch` again (showing the finding gone), *Then* the PR
    description includes the pasted stdout of all five commands verbatim.
**Files**: none (process artifact, captured in the PR description, not the repo)

##### Task 5.3.1a: Run the five-command walkthrough and paste output into the PR body (~5 min)
- Execute the five commands from the acceptance criterion against a real (or `file://`
  local, if no tagged release exists yet at PR time) plugin source; paste the complete,
  unedited terminal output into the pull request description under a "Manual demo" heading.
- Files: none (PR description only)
