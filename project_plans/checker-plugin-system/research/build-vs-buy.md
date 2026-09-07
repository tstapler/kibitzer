# Build vs. Buy: checker-plugin-system

Research question: for each sub-component of the plugin install/register/run/uninstall
mechanism, should kibitzer build it from scratch or depend on an existing crate/schema?

Repo facts checked directly (not inferred):

- `Cargo.lock` has no `reqwest`, `hyper`, `ureq`, `self_update`, `sha2`, `zip`, `tar`,
  `flate2`, or `toml` today — an HTTP client and a checksum crate would both be **net-new**
  dependencies, not already-transitive.
- `tokio` is present only for `rmcp`/`tower-lsp` (MCP/LSP server modes). `src/main.rs:177-190`
  shows the CLI is synchronous by default — `Command::Mcp`/`Command::Lsp` are the only
  arms that construct a `tokio::runtime::Runtime` and `block_on` it; every other
  subcommand (`Run`, `Hook`, `Daemon`, `Check`, `Status`, `Install`, `Architecture`) runs
  on the calling thread with no runtime. A new `kibitzer plugin install` subcommand would
  join that second group — it does not need to run inside tokio.
- `src/install.rs` is existing prior art in this repo for a similar-shaped problem
  (`kibitzer install` merges a hook entry into `.claude/settings.json`, resolving its own
  exe path via `std::env::current_exe()`), though it installs a *hook registration*, not a
  *binary*, so it doesn't directly answer the download/checksum question.
- `Check`/`OutputFormat` (`src/config.rs:10-77`) already model the dispatch target; the
  plugin mechanism's job is producing a `Check { command: Some(path), output_format:
  Some(Sarif), .. }` entry, not extending that struct.

## 1. Self-updating-binary / plugin-installer crates

| Crate | Version / last release | License | Maintenance | Verdict |
|---|---|---|---|---|
| [`self_update`](https://github.com/jaemk/self_update) | 1.3.0, 2026-09-02 | MIT | Active — 11.4M downloads, 960 stars, 0 open issues | **Not recommended** for this use, viable for the checksum feature alone |
| [`self-replace`](https://github.com/mitsuhiko/self-replace) | 1.5.0, 2024-09-01 | Apache-2.0 | Stable/narrow-scope, low churn expected | **Not recommended** — solves an unrelated problem |
| [`axoupdater`](https://github.com/axodotdev/cargo-dist) | 0.10.2, 2026-08-12 | MIT OR Apache-2.0 | Active — 937K downloads, same org as kibitzer's own `cargo-dist` release pipeline | **Not recommended**, but closest ecosystem alignment |
| `cargo-binstall` internal crates (`binstalk*`) | lockstep with `cargo-binstall` CLI | mixed | Active but not designed as a stable embeddable API | **Not recommended** — fragmented workspace, high integration cost for uncertain benefit |

Detail:

- **`self_update`** — its GitHub-backend `Update` builder does expose `bin_name` and
  `bin_install_path`, which *can* point at an arbitrary directory rather than the
  currently-running executable's own path, and it has real checksum verification
  (`checksums` feature, SHA-256/SHA-512) and even signature verification (`zipsign`
  feature). But its own README steers users who want to "install files that aren't the
  running executable" toward downloading and extracting the archive themselves rather
  than its higher-level self-update flow — i.e., the crate's ergonomic center of gravity
  and docs assume "replace this same tool in place," which is exactly the opposite of
  kibitzer's requirement (Non-functional Requirements: "the core `kibitzer` binary's
  `Cargo.toml` gains zero new hard dependencies from this work" — pulling in a crate
  designed around self-replacement to do something else is a conceptual mismatch even
  before counting dependency weight).
- **`self-replace`** solves one narrow problem: Windows refuses to let a process
  overwrite its own running `.exe`. `self_update` depends on it internally. Irrelevant
  here — writing a new file to a new path (`~/.cache/kibitzer/plugins/<name>`) has no
  running-executable conflict on any platform.
- **`axoupdater`** is the axodotdev/cargo-dist team's own library, and kibitzer's release
  pipeline is already `cargo-dist`-based (per this repo's `CLAUDE.md` "Cutting a
  release"), which makes it the most *ecosystem*-aligned candidate. But it's still built
  around "is a newer version of the tool that installed this updater available,"
  i.e. self-update semantics, not "fetch an arbitrarily-named companion binary I've never
  installed before." Using it here would mean fighting its assumptions more than it would
  save.
- **`cargo-binstall`'s internals** are technically on crates.io but are an internal,
  fast-moving multi-crate workspace not documented for external embedding. Skip.

**Net finding**: no existing crate cleanly fits "install a separately-named companion
binary" — they're all shaped around self-update-the-current-tool. The installer's actual
job here — HTTP GET a release asset, verify its checksum, write it to
`~/.cache/kibitzer/plugins/<name>/<name>`, `chmod +x` on Unix — is maybe 60-80 lines of
code once the HTTP and hashing pieces are handled by dedicated crates (§3, §4). Hand-roll
the orchestration logic; don't adopt a self-update-shaped dependency to do it.

## 2. Plugin-manifest/registry format

| Option | Verdict |
|---|---|
| asdf plugin registry (`asdf-plugins` short-name → git-url) | **Not recommended** — trivial schema not worth "aligning with"; the real convention is a bash `bin/install` script body, irrelevant to a Rust binary-download flow |
| npm `package.json` shape (`name`/`version`/`bin`/`os`/`cpu`) | **Not recommended** — solves a much bigger problem (registry, semver ranges, dependency graph) than kibitzer has |
| Homebrew formula (Ruby DSL) / JSON API | **Not recommended** — the JSON API is a *compiled view* of Ruby formulae, not a hand-authorable source format; no minimal spec to adopt wholesale |
| `cargo-binstall`'s `[package.metadata.binstall]` manifest shape | **Viable as a design template**, not as a dependency |
| Hand-rolled JSON via `serde`/`serde_json` (already dependencies) | **Recommended** |

`cargo-binstall`'s manifest (`pkg-url` templates with `{ repo }`/`{ version }`/
`{ target }`/`{ archive-suffix }` variables, `pkg-fmt`, `bin-dir`, per-target-triple
`[...overrides.<target-triple>]` blocks) is the one genuinely relevant precedent: it's a
Rust-ecosystem-native, target-triple-aware URL-templating scheme solving exactly
kibitzer's "download the right OS/arch asset" problem. `mise`'s `github:owner/repo`
backend spec is the other data point worth noting — it shows a manifest can be reduced to
almost nothing (just a repo pointer) if the tool constrains its own release-asset naming
convention tightly enough.

Given the solo-maintainer, personal-tool context and that kibitzer's own release assets
(via `cargo-dist`) already follow a predictable per-target naming convention, a
kibitzer-specific minimal JSON schema — plugin name, version, a map of target-triple to
`{url, sha256}` (or a URL template plus one hash per target, borrowing binstall's
variable-substitution idea) — is clearly right-sized. No existing schema buys enough to
be worth the indirection of learning/aligning with someone else's tooling assumptions.

## 3. HTTP client

`Cargo.lock` confirms no HTTP client crate rides along transitively today — `tokio` is
present solely for `rmcp`/`tower-lsp`, and neither pulls in `reqwest`/`hyper`/`ureq`. This
feature would add one fresh.

Given `src/main.rs` shows the CLI is synchronous everywhere except the `Mcp`/`Lsp`
subcommands (each of which spins up its own throwaway `tokio::runtime::Runtime` rather
than the binary running under a global runtime), `kibitzer plugin install` does not need
async and does not need to share a runtime with anything — it's a one-shot, blocking,
user-initiated download.

| Client | Verdict |
|---|---|
| `reqwest` | **Not recommended** — pulls in a full async stack (tokio ecosystem pieces, a TLS stack, connection pooling) for what is a single blocking GET per install; directly works against the "gains zero new hard dependencies... nothing ML-related, nothing heavy" spirit of the requirements even though tokio-the-crate is already present for unrelated reasons |
| `ureq` | **Recommended** — synchronous, small dependency footprint, actively maintained (3.3.0 released 2026-03-21 per crates.io/docs.rs), rustls-based TLS by default so no OpenSSL system-dependency surprise cross-platform |
| Hand-rolled (raw TCP/TLS) | **Not recommended** — reinventing TLS and HTTP framing is exactly the kind of security-adjacent bespoke code this project should avoid (see §4) |

**Recommendation**: `ureq`, invoked synchronously from the `plugin install` subcommand
handler, no `tokio::runtime` involved.

## 4. Checksum/verification logic

This is explicitly the kind of security-adjacent logic where hand-rolling carries real
correctness risk — a subtly wrong constant-time-vs-not comparison, a TOCTOU gap between
writing the file and verifying it, or an off-by-one in hex decoding are all easy bugs to
introduce and hard to notice.

- **Recommended**: `sha2` (RustCrypto/hashes) for the hash itself — MIT OR Apache-2.0,
  actively maintained (latest 0.11.0 line, 393M+ recent downloads per crates.io), a
  small, audited-by-ecosystem-scrutiny dependency with no transitive bloat.
- Verification-order discipline to bake into the installer regardless of crate choice:
  download to a temp path → hash the temp file → compare against the manifest's expected
  digest → only then `rename()` into the final plugins directory and `chmod +x`. Never
  execute or register a check pointing at a path before the compare succeeds. This
  ordering is design, not a library concern — no crate enforces it for you, so the plan
  phase should call it out explicitly as an implementation requirement, not leave it
  implicit.
- Use a constant-time-safe comparison only if the model shifts to signatures/HMACs later;
  a plain byte-equality check is fine for a SHA-256 digest compare (it's not a secret
  being compared against attacker-controlled input in a way timing matters for — the
  digest is public in the manifest).
- Other spots in this feature where "just write it" is fine, not reckless: the JSON
  manifest parsing (already gets `serde`'s established parser, not hand-rolled), the
  `.claude/inspect.json` check-entry merge (this repo already has this exact pattern
  proven in `src/install.rs`'s `merge_hook`), and target-triple detection (`std::env::consts::{OS,
  ARCH}` is sufficient for kibitzer's small support matrix — no need for a crate like
  `detect-targets`).

## 5. Fork or adapt: existing plugin-loader designs worth studying

Not adoptable as dependencies (different language/runtime or wrong shape), but useful as
design references:

- **mdBook preprocessors**
  ([docs](https://rust-lang.github.io/mdBook/for_developers/preprocessors.html),
  [`cmd.rs`](https://github.com/rust-lang/mdBook/blob/master/src/preprocess/cmd.rs)) — a
  directly borrowable *protocol* shape: mdBook first runs `$cmd supports $renderer` as a
  capability probe (exit 0 = yes), then for a real run pipes JSON on stdin and reads
  transformed JSON on stdout, with a non-zero exit treated as an error and stderr passed
  through. kibitzer's SARIF-over-stdout convention is already one-way (findings only,
  no negotiation), but the `supports`-style capability probe is worth considering later
  for "does this plugin apply to this trigger/language" — not required for the v1 stub,
  but a cheap extension point to leave room for. Notably, mdBook has **no install/registry
  mechanism at all** — preprocessors must already be on `PATH` or given an absolute path
  in `book.toml`, and distribution is entirely the user's problem. That's a useful negative
  data point: kibitzer's explicit goal (an install subcommand) is *more* than mdBook's
  design does, not a gap in mdBook worth copying around.
- **Ruff** has no public plugin system; a 2023 discussion
  ([astral-sh/ruff#8409](https://github.com/astral-sh/ruff/discussions/8409)) shows
  Astral favoring a monolithic, Rust-native rule set for performance/consistency. Cited as
  a counterpoint — the fast/opinionated-linter camp avoids plugin systems on purpose — not
  a design to copy, but relevant context for scoping kibitzer's plugin surface narrowly.
- **zellij**'s WASM/WASI plugin system confirms the tradeoff already named in
  `requirements.md`'s Alternatives Considered: real sandboxing, but the WASM engine
  itself is exactly the core-binary dependency weight this project exists to avoid.
  No new information beyond what's already documented there.
- **git's `git-<subcommand>` PATH convention** — any executable named `git-foo` on `PATH`
  becomes invokable as `git foo`, with zero registration step and zero manifest. This is
  the cheapest possible "plugin" dispatch model, and it's architecturally close to what
  kibitzer's `Check.command` already does (exec a named binary, parse its output). It
  validates that the underlying *dispatch* mechanism kibitzer already has — just exec a
  binary and read its output — is a proven, zero-ceremony pattern; it doesn't replace the
  need for kibitzer's own install step, since the requirements explicitly want "fetch and
  place" to be part of the mechanism, not left to the user the way git and mdBook both do.

## Overall recommended stack

- **Installer logic**: hand-rolled (no self-update-shaped crate fits; ~60-80 lines of
  orchestration: build a target-triple-qualified download URL, GET it, hash it, compare,
  rename into place, chmod +x, write/merge a check entry).
- **HTTP client**: `ureq` (sync, small footprint, actively maintained, no tokio runtime
  needed for the install subcommand).
- **Checksum**: `sha2` (RustCrypto), with an explicit download-hash-compare-then-place
  ordering enforced in code, not left implicit.
- **Manifest format**: a kibitzer-specific minimal JSON schema (via existing `serde`/
  `serde_json`) — plugin name, version, per-target-triple `{url, sha256}` (or a
  cargo-binstall-style URL template plus per-target hash) — not an adopted external
  schema.
- **Design references to cite in the plan** (not dependencies): mdBook's stdin/stdout
  JSON preprocessor protocol and `supports` capability probe as a future extension point;
  `cargo-binstall`'s `pkg-url`/`pkg-fmt`/`bin-dir` manifest shape as the template for
  kibitzer's own manifest fields; git's `git-<subcommand>` PATH convention as validation
  that the exec-and-parse dispatch half of this design (already built via `Check.command`)
  is sound and proven elsewhere.
- **New `Cargo.toml` dependencies this adds**: `ureq`, `sha2` — both small, mature,
  synchronous-friendly, and neither pulls in an ML/ONNX-adjacent or async-heavy
  dependency graph, keeping the "core binary gains zero new *heavy* hard dependencies"
  success metric intact (two small, focused crates, not a framework).
