# Feature research: checker-plugin-system

Agent 2 (Features), SDD Phase 2. Scope: prior-art comparison, edge cases/failure
modes, and repo house-style fit for a `kibitzer plugin` install/register/run/uninstall
subcommand built on the existing `Check.command` + `output_format: sarif` mechanism
([src/config.rs:28-77](https://github.com/tstapler/kibitzer/blob/master/src/config.rs#L28-L77)).

## 1. Prior art: "optional companion tool, fetched on demand, wired into host config"

Picked three tools whose install/registration/versioning flow is closest to this
project's shape (single binary, GitHub Releases, per-OS/arch artifacts, a local
manifest that composes with hand-written config) — `gh extension install`,
`rustup component add`, and `pre-commit`'s `repos:` hook installer.

### `gh extension install` (closest analog — binary-from-GitHub-Release)

Source: [gh extension install manual](https://cli.github.com/manual/gh_extension_install).

- **Resolution**: `gh extension install <owner>/<repo>` first checks the repo's
  GitHub Releases for platform-matching binary assets ("binary extension"); if no
  release exists, it clones the repo instead ("script extension") and looks for an
  executable matching the repo name at its root.
- **Versioning**: `--pin <tag-or-sha>` records a specific release tag (binary) or
  commit (script) instead of tracking latest; a subsequent run without `--pin`
  updates to the newest release. `--force` re-installs/upgrades even if the pinned
  version is already current — i.e., install is idempotent by default and only
  overwrites on an explicit flag.
- **Registration**: no separate config file the user hand-edits — `gh` keeps its
  own extension manifest and looks up installed extensions by name at dispatch
  time (`gh <extname>` runs the installed binary). This is the "kibitzer-owned
  manifest, not auto-writing into the user's own config" model from this project's
  Rabbit Holes/Open Questions.
- **Local dev loop**: `gh extension install .` in a cloned repo installs it as a
  symlink to the local build instead of a downloaded release — directly relevant
  to this project's own stub-plugin/test story (install from a local fixture path,
  no network, same code path as a real install).

### `rustup component add`

Sources: [rustup component/channel docs](https://rust-lang.github.io/rustup/concepts/components.html), [rustup manifestation.rs](https://github.com/rust-lang/rustup/blob/main/src/dist/manifestation.rs).

- **Manifest-driven**: a per-channel `channel-manifest.toml` (server-published,
  versioned "manifest-version": "2") lists every component's per-target download
  URL and hash; rustup diffs "components in the manifest" against "components
  currently installed" to compute what to fetch, so add/remove is manifest-diff
  driven rather than ad hoc.
- **Verification and atomicity**: components are downloaded, hash-checked, then
  extracted into a staging area before being made live — a failed/partial
  component doesn't corrupt a working toolchain install.
- Less directly transferable here (rustup's manifest is server-authored and
  covers dozens of components across many targets); the useful takeaway is the
  **separate machine-readable manifest as the source of truth for "what's
  installed, from where, at what hash"**, distinct from user-facing config.

### pre-commit's `repos:` hook installer

Source: [pre-commit.com](https://pre-commit.com/) advanced docs.

- **Pinning is mandatory, not optional**: `.pre-commit-config.yaml` requires a
  `rev:` (tag or commit SHA) per hook repo — there is no "latest" mode in the
  config itself; `pre-commit autoupdate` is the explicit, separate command that
  bumps pins. This maps directly onto this project's Open Question about
  compatibility/versioning: pre-commit's answer is "always pin, upgrade is its
  own explicit action," which is cheap to adopt for a solo-maintainer tool.
  Cache is keyed by `(repo, rev)` so a rev is immutable once cached.
- **Registration is the same file as config**: unlike `gh extension`, pre-commit
  has no separate "installed extensions" manifest — the `.pre-commit-config.yaml`
  entry *is* the registration. Directly relevant to this project's central open
  question: whether kibitzer plugin registration should write into
  `.claude/inspect.json` (pre-commit's model) or a separate kibitzer-owned file
  (`gh`'s model, closer to rustup's).

### Recommendation implied by the three

None of them auto-write into a config file a human also hand-edits and expects to
read cleanly (pre-commit's config *is* hand-edited, but hooks are describable in
one YAML block a user chose to paste in). Given this repo's own
`find_effective_config()`/`.claude/inspect.json` overlay convention already
treats `.claude/inspect.json` as a *user-authored* overlay
(`docs/suppressing-checks.md`), auto-writing a `Check` entry into it from a CLI
command would blur that line — a plugin install/uninstall diffing the user's own
hand-edited JSON is exactly the kind of merge risk `src/install.rs::merge_hook`
already exists to solve for `settings.json`, but for `inspect.json` doing this
non-destructively is harder because unlike `hooks.PostToolUse`, `checks` entries
have no server-controlled name-collision-free namespace and only `disabled`
supports being told "not this default." A **separate kibitzer-owned plugin
registry file** (e.g. `~/.cache/kibitzer/plugins/registry.json` or
`~/.local/share/kibitzer/plugins.json`), consulted by `default_checks()`/
`find_effective_config()` the same way defaults already are, avoids the merge
problem entirely and matches `gh extension`'s model — this is the direction
Phase 3 planning should default to unless it finds a concrete reason to write
into `.claude/inspect.json` instead.

## 2. Edge cases and failure modes

| Case | Notes for the design |
|---|---|
| Partial/interrupted download | Download to a temp path (same filesystem/dir as the final destination) and rename-into-place only after full download + checksum verification succeeds; a half-written file must never be left at the path check-dispatch will execute. |
| Checksum mismatch | Hard-fail install with the expected vs. actual hash in the error; never register a checksum-failed binary. `cargo-dist`'s own release output already includes per-artifact hashes (confirmed: `.github/workflows/release.yml` comment says dist "builds artifacts... (archives, installers, hashes)") — if the stub plugin ships via the same `dist` pipeline, the hash to verify against already exists for free. |
| Network unavailable | Fail fast with a clear stderr message; per this project's Constraints, install must be the *only* place kibitzer touches the network, so a normal check run must never attempt a retry/refetch — a registered-but-unreachable plugin should degrade to a per-run error on that one check, not a hang or a silent skip (silent skip risks masking a real regression from the user). |
| Plugin binary crashes/hangs at run time | Already partially handled by the existing `Check.command` dispatch (it shells out via `std::process::Command` today) — confirm/inherit whatever timeout behavior exists for a `command` check generally; a plugin should not get special-cased weaker guarantees than a hand-configured `command` check has today. Worth explicitly stating in the plan whether a per-check timeout exists at all currently — if not, this is pre-existing scope, not new to plugins. |
| Plugin already installed | `install` should be idempotent like `src/install.rs::run_install` is for the hook (`merge_hook` returns `Ok(false)`/no-op on a second run) — re-running `plugin install foo` at the same version should say "already installed" rather than erroring or duplicating a registry entry; support an explicit `--force`/upgrade path (mirrors `gh extension install --force`). |
| No published artifact for the user's OS/arch | Must fail with a clear, actionable message naming the missing target — not a generic 404/panic. Since `dist-workspace.toml` already declares exactly 4 targets (`aarch64-apple-darwin`, `aarch64-unknown-linux-gnu`, `x86_64-apple-darwin`, `x86_64-unknown-linux-gnu`), any plugin distributed via the same `dist` pipeline is safe by construction for the same platforms the core binary already supports — Windows and other targets are out of scope on both sides symmetrically. |
| Registered in config but binary later deleted from disk | A check run should surface this as a normal check failure/error (missing command), not a crash — consistent with how a hand-authored `.claude/inspect.json` `command` pointing at a since-uninstalled tool would already fail today. `plugin` subcommand should also offer a way to detect/report this (e.g. `plugin list` marking an entry as "binary missing") rather than only failing silently at check-run time. |
| Uninstall while a check run is in progress | Lowest-severity edge case for a single-user local CLI tool (no daemon-wide plugin cache to worry about beyond the existing `daemon` process) — but note kibitzer *does* have a long-lived daemon (`src/daemon.rs`) that caches check results across invocations; if the daemon holds the process alive between runs, uninstalling a plugin's binary out from under a warm daemon is the more realistic version of this race than a mid-single-run race. Plan should state whether uninstall needs to signal/restart the daemon, or whether "next check run naturally re-resolves the check list" is good enough. |

## 3. Unstated needs / house-style fit (read from this repo's own conventions)

- **`src/install.rs` is the load-bearing precedent, not just `Check.command`.**
  It already solves "a CLI subcommand that idempotently merges a new entry into
  a JSON config file the user might have hand-edited, with `--dry-run` and
  `--global`-style flags, printing `[kibitzer] ...`-prefixed status lines, and
  unit-testing the merge logic in isolation from real file I/O" — see
  `merge_hook`/`read_settings` in [src/install.rs](https://github.com/tstapler/kibitzer/blob/master/src/install.rs)
  and its 6 unit tests covering idempotency, unrelated-key preservation, and a
  malformed-root error case. A `kibitzer plugin install`/`uninstall` subcommand
  should reuse this exact shape: `--dry-run` printing what would be written,
  idempotent re-install, tests against an in-memory `serde_json::Value` rather
  than real files on disk. This is a stronger, more specific precedent than the
  generic `Check.command` doc comment and should anchor Phase 3's design instead
  of being reinvented.
- **Subcommand nesting convention**: every existing multi-action subcommand
  (`daemon {start,stop,status}`, `check {native,architecture,list,backtest}`,
  `architecture {export,diagram}`) is a `#[derive(Subcommand)]` enum nested under
  a single top-level `Command` variant (`src/main.rs:49-95`). `plugin` should
  follow the same shape — `Command::Plugin { action: PluginAction }` with
  `PluginAction::{Install, Uninstall, List, ...}` — not a flat set of
  top-level `plugin-install`/`plugin-uninstall` commands.
- **Doc-per-mechanism convention**: `docs/output-formats.md` and
  `docs/suppressing-checks.md` are short, single-purpose reference docs linked
  from `CLAUDE.md`'s "Default check catalog" section. A new
  `docs/plugin-checks.md` (or similarly named) documenting the install/registry
  mechanism fits this pattern directly, and `CLAUDE.md` itself should gain one
  sentence pointing at it (matching how it already calls out
  `comment-quality-<lang>` and `syntax-rules-<lang>` by name) — this is a repo
  convention violation risk if skipped, not a nice-to-have.
- **Evidence/proportionality discipline applies to the PR itself, not just this
  research doc**: per `~/.claude/CLAUDE.md`'s Evidence and Claims section, the
  required "manual CLI walkthrough (install → run → see the stub's output)"
  success metric should be captured as terminal output pasted into the PR body,
  not narrated — consistent with "Run it, don't read it." The requirements doc
  already asks for this explicitly (Success Metrics); Phase 3 planning should
  make sure the task breakdown has a discrete step for capturing it, not just
  "write the automated test."
- **Zero-new-hard-dependency framing needs a precise reading.** The requirement
  says the core `Cargo.toml` must gain no new *ML-related* dependency, not zero
  dependencies whatsoever — confirmed by reading `Cargo.toml`'s
  `[dependencies]` block: today there is no HTTP client (no `reqwest`/`ureq`/
  `curl` crate) and no checksum crate (no `sha2`). An `install` subcommand that
  downloads from a GitHub Release necessarily adds at least an HTTP client and
  a hashing crate to the *core* binary (the CLI that runs `plugin install` is
  the same binary that runs checks) — small, common, non-ML dependencies, but
  still new hard dependencies. This is worth stating explicitly in the plan
  rather than letting the Success Metric read as "no new dependencies at all,"
  which isn't achievable for a real network install path.
- **Test-fixture-over-real-release is the pragmatic default already implied by
  house style.** The Feasibility Risks/Open Questions flag "real GitHub Release
  artifact vs. local-fixture-based install" as unresolved; nothing in this
  repo's existing test suite (`tests/hook_contract.rs` is the only integration
  test today, and it exercises the hook path without any network dependency)
  suggests network-dependent tests are an established pattern here — the
  automated test should install from a local fixture path/file URI, with the
  "real GitHub Release" case covered only by the separately-called-out manual
  walkthrough, not by CI.

## Sources consulted

- [src/config.rs](https://github.com/tstapler/kibitzer/blob/master/src/config.rs) (`Check`/`OutputFormat` definitions, lines 1-90)
- [src/main.rs](https://github.com/tstapler/kibitzer/blob/master/src/main.rs) (CLI subcommand structure, lines 40-220)
- [src/install.rs](https://github.com/tstapler/kibitzer/blob/master/src/install.rs) (hook-install precedent, full file)
- `docs/output-formats.md`, `docs/suppressing-checks.md` (read in full)
- `Cargo.toml`, `dist-workspace.toml`, `.github/workflows/release.yml` (dependency and release-artifact inventory)
- [gh extension install manual](https://cli.github.com/manual/gh_extension_install)
- [rustup components docs](https://rust-lang.github.io/rustup/concepts/components.html) and [manifestation.rs](https://github.com/rust-lang/rustup/blob/main/src/dist/manifestation.rs)
- [pre-commit.com](https://pre-commit.com/)
