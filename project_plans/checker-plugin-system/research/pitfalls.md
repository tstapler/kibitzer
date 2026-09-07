# Research: Pitfalls & Risks — Plugin Mechanism

Agent 4 (Pitfalls), Phase 2 research for `checker-plugin-system`. See
`project_plans/checker-plugin-system/requirements.md` for full context.

## 1. Supply-chain / trust pitfalls

### What commonly goes wrong (prior art)

- **No integrity check at all, or a check that's easy to bypass.** The baseline failure
  mode across every "downloads and runs a binary" tool is skipping verification entirely,
  or verifying against a checksum fetched over the same unauthenticated channel as the
  artifact — which only detects corruption, not tampering, since an attacker who can swap
  the binary can swap the checksum file next to it too.
- **TOFU with no re-verification on update.** Trust-on-first-use is an accepted, deliberate
  model in prior art (SSH host keys, `krew`), but it only holds if every *subsequent*
  install/upgrade of the same plugin re-checks integrity — several tools silently replace
  an already-installed binary on update without re-checking anything, because the install
  path assumes "we already decided to trust this plugin name once."
- **A manifest/index file as an unverified trust root.** `krew`'s plugin manifests carry a
  `sha256` field for the plugin archive itself, but the manifest's own integrity comes from
  the index being a git repo with GPG-verified commits — i.e., the checksum only helps if
  the thing carrying the checksum is itself authenticated. A JSON manifest fetched over
  plain HTTPS with no signature is a weaker version of this: MITM or a compromised CDN edge
  can rewrite the manifest and the checksum together. (Kubernetes SIGs, [krew plugin
  manifest reference](https://github.com/kubernetes-sigs/krew/blob/master/site/content/docs/developer-guide/plugin-manifest.md);
  [krew architecture](https://github.com/kubernetes-sigs/krew/blob/master/docs/KREW_ARCHITECTURE.md))
- **Postinstall-script-class attacks generalize to "install subcommand downloads and
  execs."** The npm ecosystem's postinstall-script attacks (axios compromise, the 2025
  Shai-Hulud worm) follow the exact shape this feature introduces on a smaller scale:
  fingerprint the host, fetch a platform-specific payload, execute it with the user's full
  privileges. npm's own fix — `ignore-scripts=true` becoming the v12 default — is "stop
  auto-executing"; the analog here is already satisfied by the requirement that install is
  an explicit, user-typed subcommand, never triggered by a normal check run (see
  requirements.md "Constraints"). ([Sonatype: 176-package npm campaign](https://www.sonatype.com/blog/inside-a-176-package-npm-campaign-built-to-beat-your-internal-dependencies);
  [Huntress: axios compromise](https://www.huntress.com/blog/supply-chain-compromise-axios-npm-package);
  [Semgrep: npm v12 kills postinstall by default](https://semgrep.dev/blog/2026/rip-npm-postinstall-scripts-npm-v12-default-change/))
- **Release-asset tampering window.** Between a GitHub Release being published and a user's
  `install` running, the asset itself is static once uploaded (GitHub doesn't allow silently
  re-uploading an asset under the same filename+tag without deleting it first), but the
  *download* is only as trustworthy as the TLS connection fetching it — an unpinned `http://`
  fallback, a redirect through a non-GitHub mirror, or disabling TLS verification for
  convenience are the actual failure modes, not GitHub-side tampering.
- **No trust distinction for extensions that aren't first-party.** `gh` explicitly documents
  that CLI extensions are "not verified, signed, or endorsed by GitHub... you are trusting
  its publisher" — it ships no runtime verification at all beyond what an extension author
  opts into themselves. This project is narrower (out of scope explicitly excludes
  third-party plugins per requirements.md "Non-functional Requirements" / "Out of Scope"),
  which removes the hardest version of this problem (verifying an unknown publisher) but
  doesn't remove the need to verify *this repo's own* release artifact wasn't corrupted or
  swapped in transit. ([gh extension install docs](https://cli.github.com/manual/gh_extension_install))

### Minimum viable mitigation for a single-user personal tool

Do NOT reach for code signing services, Sigstore/cosign, SBOMs, or a separate index
repository — those are proportionate to a multi-publisher ecosystem, not one maintainer
shipping one plugin from one already-trusted repo. "Good enough" concretely:

1. **Fetch over HTTPS only**, from a hardcoded `github.com`/`api.github.com` host (or
   `githubusercontent.com` for the release asset redirect target) — reject any redirect
   that leaves that host set. This is a few lines of URL validation, not infrastructure.
2. **Publish a checksum file (`SHA256SUMS` or per-asset `.sha256`) as a second Release
   asset alongside the plugin binary, in the same `gh release create` step that already
   uploads artifacts** (`.github/workflows/release.yml:279`) — `cargo-dist` already
   produces a `*-dist-manifest.json` with per-artifact hashes for the core binary's own
   targets, so the plugin binary should ride the same manifest convention rather than a
   bespoke checksum format. Verify the downloaded binary's SHA-256 against this before
   `chmod +x`/first execution, refusing to install (not silently continuing) on mismatch.
3. **Re-verify on every install/update, not just first install** — TOFU is fine for "do I
   trust the publisher," but the checksum check itself is cheap enough that there is no
   reason to make it TOFU too. Check every time a binary is written to disk.
4. **Fail closed and say why.** A checksum mismatch is a hard error with an explicit
   message ("downloaded artifact does not match published checksum — refusing to install"),
   not a warning that install continues past.
5. **No auto-update.** Nothing above implies the plugin should ever update itself in the
   background — that reintroduces the "silently replaces an already-installed binary"
   failure mode from a different angle (an update triggered by something *other* than the
   user's explicit `install`/`update` invocation). Keep updates as explicit as install.

This is deliberately the same trust model `cargo-dist`'s own shell installer already uses
for the core `kibitzer` binary (checksum-verified download over HTTPS from a GitHub
Release) — reusing that pattern for the plugin means no new trust *concept* is being
introduced, just applying an existing one a second time.

## 2. Distribution pitfalls specific to `cargo-dist`

Current config: [`dist-workspace.toml`](../../../dist-workspace.toml) —
`cargo-dist-version = "0.32.0"`, `installers = ["shell", "homebrew"]`,
`targets = ["aarch64-apple-darwin", "aarch64-unknown-linux-gnu", "x86_64-apple-darwin",
"x86_64-unknown-linux-gnu"]`, single workspace member (`Cargo.toml:2-3` — `[workspace]
members = ["cargo:."]`, `name = "kibitzer"`, `version = "0.1.13"`).

- **cargo-dist's multi-binary model is per-*package*, not per-binary.** Per the `dist`
  book's ["More Complex Workspaces" guide](https://axodotdev.github.io/cargo-dist/book/workspaces/workspace-guide.html):
  if you add a second `[[bin]]` target *inside the existing `kibitzer` package*, `dist`
  treats the whole package as one "App" and bundles **both** binaries into every zip and
  shell/Homebrew installer it produces for `kibitzer` — there's no way to opt one binary
  out of that bundling from within a single package. That means a naive "just add
  `src/bin/kibitzer-plugin-stub.rs`" approach ships the stub binary inside the *core*
  `kibitzer` tarball/formula, which directly contradicts the requirement that a default
  install stays untouched (requirements.md "Success Metrics": "Uninstalling/never
  installing the plugin leaves kibitzer's default behavior... completely unchanged" — a
  binary silently present in `$(brew --prefix)/bin` from the default formula install isn't
  "unchanged," even if it's inert until invoked).
- **To get an independently-releasable artifact, the plugin needs its own workspace
  package**, each with its own `Cargo.toml` — `dist` then treats it as a fully independent
  "App" with its own zips/installers/versioning. This is real, supported behavior (not a
  workaround), but it has a concrete consequence for *this* repo: `Cargo.toml` currently
  disables the actual Cargo workspace (`[workspace]` with only `members = ["cargo:."]` and
  a comment `# Remove this when kibitzer is added to the workspace members list` —
  suggesting the repo isn't a real multi-crate workspace yet). Turning a stub plugin into a
  second package means either standing up a real Cargo workspace (a structural change
  beyond "add a plugin"), or accepting the dist-level "virtual workspace" `[workspace]
  members = ["cargo:."]` syntax already in `dist-workspace.toml` extended with a second
  `cargo:` member path — confirm this syntax's exact multi-member behavior against the
  installed `cargo-dist-version = "0.32.0"` before committing to it in planning, since dist
  virtual-workspace support has had active churn (see the open feature-resolution issue
  below).
- **Feature/dependency resolution across multiple binaries in one build is an open,
  actively-discussed cargo-dist limitation** — [axodotdev/cargo-dist#1740, "Workspace
  feature resolution for multiple binaries"](https://github.com/axodotdev/cargo-dist/issues/1740)
  tracks exactly the scenario where different binaries in a dist-managed workspace want
  different dependency/feature configurations. This is directly relevant here: the whole
  point of the plugin split is that the plugin (eventually, post-scope, the ONNX/embedding
  checker) needs heavy dependencies the core binary must never pull in — if that isolation
  leaks through shared workspace feature unification, the project's core success metric
  ("core binary's Cargo.toml gains zero new hard dependencies") is at risk even with
  separate packages, unless the packages are verified to resolve dependencies
  independently under the dist build. This needs to be validated empirically in planning/
  implementation, not assumed from the package-separation model alone.
- **Release cadence/tagging: cargo-dist releases are triggered by a single pushed tag**
  (per repo `CLAUDE.md` "Cutting a release": `git tag vX.Y.Z && git push origin vX.Y.Z`
  fires the one `release.yml` workflow). With two independently-versioned Apps in one
  `dist`-managed workspace, `dist`'s plan/tag-detection step has to decide, from one tag
  push, which App(s) that tag's version applies to — `dist`'s documented model for this is
  per-package version tags (e.g. `my-app-v1.2.3`) when multiple Apps need independent
  version numbers, which is a departure from this repo's current single `vX.Y.Z` tagging
  convention and needs to be an explicit planning decision (lockstep single version vs.
  prefixed independent tags), not discovered mid-implementation.
- **Naming collisions are avoidable but not automatic.** Release artifact filenames are
  derived from package name + target triple; as long as the plugin package is named
  distinctly from `kibitzer` (e.g. `kibitzer-plugin-stub`), there's no collision risk in
  the artifact filenames or the `gh release create artifacts/*` glob
  (`.github/workflows/release.yml:279`) that uploads everything in one release — but a
  same-tag release means both Apps' artifacts land in the *same* GitHub Release, so the
  Release page will show both `kibitzer-*` and `kibitzer-plugin-stub-*` assets together,
  which the install subcommand's asset-name matching needs to account for explicitly
  (filter by package-name prefix, not just architecture suffix).

**Bottom line for planning**: cargo-dist *can* do this, but "just add a second binary and
reuse the pipeline" undersells the real cost — it requires a second workspace package
(a structural change this repo hasn't made yet), a versioning-scheme decision, and
verification that dependency isolation actually holds under a shared dist build. The
Rabbit Hole callout in requirements.md ("reusing the existing `cargo-dist` machinery...
is cheap enough, versus deferring true multi-platform support") should be answered
"non-trivial but supported, budget for it explicitly" rather than assumed cheap.

## 3. Operational pitfalls (grounded in `src/check.rs`)

### No timeout on `command` checks today — this is a pre-existing gap, not new

[`src/check.rs:156-160`](../../../src/check.rs#L156-L160):

```rust
let output = Command::new("sh")
    .arg("-c")
    .arg(&cmd_str)
    .current_dir(repo_root)
    .output()?;
```

`Command::output()` blocks until the child exits, with **no timeout anywhere in the call
chain** — confirmed by grepping `src/check.rs` for `timeout`/`Duration` (no hits on the
command-dispatch path; the only `Duration`-adjacent hits are unrelated git-archive/tar
snapshot code). Every existing `command`-based check already has this gap, but the plugin
mechanism raises the stakes: a plugin binary is the one class of `command` check this
project is *designing for* to be routinely swapped, upgraded, and run against arbitrary
future embedding-model workloads that could genuinely hang (a downloaded ONNX model that
fails to load, a plugin binary waiting on stdin it never receives, network calls inside a
plugin despite the "no silent network access" constraint being about kibitzer itself, not
what a plugin does). A hung plugin process blocks the whole check-dispatch path for that
file/batch with no recovery — in the daemon (`src/daemon.rs`, which the design doc notes
spawns one thread per baseline check) or the Claude Code hook path, this reads as kibitzer
itself hanging, not "a plugin misbehaved."

**Design-against**: give plugin-invoking checks (at minimum) a wall-clock timeout using
`wait_timeout`-style polling or a `std::process::Command` + a killer thread, distinct from
todo-later generic timeout support for all `command` checks — but note the more useful fix
is almost certainly a repo-wide timeout on the `run_check` command path (line
`src/check.rs:156`), since a hang there hurts every `command` check equally, and it's a
5-10 line change vs. inventing a plugin-only timeout wrapper. Flag this to Phase 3 planning
as "cheap enough to just fix generally, not plugin-scope this."

### No installed-binary-missing detection

Because commands are dispatched via `sh -c "<cmd_str>"` rather than `Command::new(binary)`
directly, a missing plugin binary does **not** surface as a Rust-level `io::Error`
(`ErrorKind::NotFound`) that the `?` at line 160 would catch — `sh` itself spawns fine, and
`sh -c "nonexistent-binary --file foo.rs"` exits 127 with `sh: 1: nonexistent-binary: not
found` on stderr. That flows through the existing `command`-check code path as an ordinary
failing check (`passed_raw = false`), with the raw shell error text as the only signal —
no distinction between "the check legitimately found a problem" and "the plugin isn't
installed." For a plugin whose entire premise is "installed separately, may not be
present," this is a real usability gap: a user who never ran `plugin install` (or who
uninstalled it) sees a cryptic shell-not-found message mixed in with real findings, not a
clear "plugin X is not installed — run `kibitzer plugin install X`" message.

**Design-against**: the plugin mechanism's check-registration path should resolve and
validate the plugin binary's existence *before* falling through to the generic `command`
dispatch (or wrap the generated `command` string with a pre-flight existence check), so a
missing plugin produces a distinct, actionable error rather than a shell 127.

### Stale registration after uninstall

Per requirements.md's own Open Questions, whether plugin registration writes into
`.claude/inspect.json` directly or a separate kibitzer-owned manifest matters here
concretely: `merge_checks` (`src/config.rs:604-616`) overlays local checks onto
`default_checks()` **by name**, with no existence check on `command`'s target binary at
config-load time. If `plugin uninstall` deletes the binary but leaves a registered check
entry (in either file), every future run hits exactly the "missing binary" case above with
no automatic cleanup — `uninstall` must remove the check registration in the same
operation that removes the binary, not just the binary.

### Architecture/OS mismatch across synced machines — a real scenario for this user

This user's own dotfiles `CLAUDE.md` documents active use across Manjaro, Ubuntu, macOS,
and WSL2, with `.claude` itself symlinked via `cfgcaddy` and stated to "apply globally, not
just to this repo." A `.claude/inspect.json` plugin-check registration is exactly the kind
of file that convention syncs across machines (it's checked into a repo, or lives under a
dotfiles-managed `.claude/`), while the **plugin binary it points at is per-machine,
per-architecture, installed separately into some local cache/bin directory.** Concretely:
a registration entry written on an `aarch64-apple-darwin` machine pointing at
`~/.cache/kibitzer/plugins/checker-stub` (an aarch64 Mach-O binary) syncs via dotfiles to
an `x86_64-unknown-linux-gnu` Manjaro box where that path either doesn't exist (surfaces as
the "missing binary" case above) or — worse, if the plugin cache directory path happens to
also be synced/shared (e.g. a synced home directory, not the common case here but worth
naming) — exists but is the wrong architecture's binary, which `sh -c` will fail to exec
with a `Exec format error`, again showing up as an unhelptul raw shell error rather than a
clear message.

**Design-against**:
- Key the plugin cache/install path by target-triple (`~/.cache/kibitzer/plugins/<name>/<target-triple>/<bin>`
  or equivalent) so a registration entry is portable across the four `dist-workspace.toml`
  target triples, and `install`/the check-resolution path picks the right one for the
  current machine at *run* time (resolve `std::env::consts::ARCH`/`OS` → triple, not embed
  a resolved absolute path into the registration itself) — i.e. the registered `command`
  should reference a stable, OS-agnostic lookup (`kibitzer plugin run <name>` indirection,
  or a path template kibitzer resolves per-machine) rather than a literal per-machine
  absolute path baked in at install time, exactly because that registration is expected to
  travel across machines via dotfiles sync.
- Surface `Exec format error` / permission-denied-on-exec distinctly from "check found a
  problem," same rationale as the missing-binary case.

## 4. What should be explicitly designed against (prioritized)

1. **A registered plugin check must never look like an ordinary failing check when the
   plugin itself is the problem** — missing binary, wrong architecture, and (new) a hung
   process all need to surface as a distinct, actionable "plugin problem" signal, not
   folded into `passed=false` with shell noise as the message. This is the single biggest
   usability risk given this user's real multi-machine dotfiles-synced setup.
2. **The registered `command`/binary path must be portable across machines by
   construction** — resolve architecture/OS to the right binary at run time, don't bake a
   literal per-machine absolute path into a config file that's expected to sync via
   dotfiles. Treat this as a hard requirement, not a nice-to-have, given the cross-platform
   scenario is already real for this specific user today.
3. **A wall-clock timeout on the `command` dispatch path** (`src/check.rs:156`) — cheap to
   add, protects every `command` check (plugin or not) from an indefinite hang, and closes
   the gap the plugin mechanism otherwise makes more likely to trigger in practice.
4. **Checksum-verify every plugin binary write to disk (install AND update), fetched only
   over HTTPS from a hardcoded GitHub host** — minimum viable trust boundary; no TOFU
   exemption on re-installs/updates, no signing infrastructure, no separate index/registry
   service.
5. **`uninstall` must atomically remove both the binary and its check registration** — a
   dangling registration after uninstall recreates the "missing binary" failure mode
   immediately and would otherwise need its own bug report to discover.
6. **Decide the cargo-dist packaging shape (second workspace package vs. bundling into the
   `kibitzer` package) *before* implementation, not during** — bundling into the existing
   package silently violates the "default install unchanged" success metric; splitting
   into a second package requires standing up a real Cargo workspace (currently disabled
   per `Cargo.toml:2`'s TODO comment) and a versioning-tag decision, both real costs Phase
   3 planning should budget explicitly rather than treat as "the same pipeline, one more
   binary."
7. **No implicit/background updates** — every fetch-and-execute action (install, update)
   stays a distinct, user-typed subcommand invocation, consistent with the existing "no
   silent network access" constraint and avoiding a second, softer version of the same
   trust problem this list is otherwise mitigating.
