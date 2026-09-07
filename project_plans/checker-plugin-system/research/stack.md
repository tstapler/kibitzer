# Stack Research: checker-plugin-system

Agent 1 (Stack) — SDD Phase 2 research for `kibitzer plugin install <name>`, a plugin
manifest format, and reusing `cargo-dist` to publish a stub companion binary.

## 1. Current dependency baseline (confirmed by reading `Cargo.toml`/`Cargo.lock`)

- `Cargo.toml` has **no HTTP client, no checksum crate, no `toml` crate** today — a
  `grep` for `reqwest|ureq|hyper|sha1|sha2|toml|dirs|directories` across all 157 crates
  in `Cargo.lock` returns nothing. Everything needed for downloading + verifying a
  plugin is a genuinely new dependency, not something already transitively present.
- `rmcp = "0.2"` (the MCP server dep) pulls in `tokio`, `base64`, `chrono`, `futures`,
  `tracing` — but **no HTTP stack**; it's JSON-RPC over stdio (`transport-io`), not
  network transport.
- `tokio = { version = "1", features = ["rt-multi-thread", "macros", "io-std"] }` has no
  `net` feature enabled, and `fn main() -> Result<ExitCode>` ([`src/main.rs:177`](https://github.com/tstapler/kibitzer/blob/master/src/main.rs#L177))
  is synchronous — `tokio::runtime::Runtime::new()` is only spun up explicitly inside the
  `Mcp`/`Lsp` match arms. **A plugin-install subcommand should stay synchronous** rather
  than pulling the async runtime into a code path that doesn't need it.
- Config parsing repo-wide uses `serde_json` only; there is no existing `toml` usage
  anywhere in `src/`.
- Directory resolution (`src/cache.rs:148-157`, `src/daemon.rs:41`, `src/backtest.rs:170-177`)
  is hand-rolled: `std::env::var("XDG_CACHE_HOME")` with a `$HOME/.cache` fallback, no
  `dirs`/`directories` crate. This is a deliberate, repeated idiom — a plugin directory
  helper should follow the same pattern rather than adding a new crate for path
  resolution.

## 2. HTTP client: `ureq` 3.x, not `reqwest`

**Recommendation: `ureq = "3"`.**

- `reqwest` is async-first and pulls in `hyper`, `h2`, a TLS stack, and effectively the
  full Tokio ecosystem even for a single blocking call — disproportionate for a CLI
  subcommand that fires once per invocation and otherwise never touches the network.
  `main()` is sync; using `reqwest::blocking` still drags the async dependency graph
  into the binary.
- `ureq` is sync-only, has a much smaller dependency footprint and 5-10x faster clean
  builds relative to reqwest, and is the community-recommended default in 2026 for
  exactly this profile (CLI tool, blocking, compile-time-sensitive). (WebSearch,
  September 2026: "reqwest vs ureq vs hyper: Which Rust HTTP Client in 2026?" —
  Rustify.)
- **Version note**: `ureq` 3.x (current stable: 3.3.0, released ~March 2026) is a
  rewrite of the 2.x API — new `Agent`/request-builder shape built on the `http` crate.
  Pin `"3"`, not `"2"`; do not copy 2.x-era example code.
- This keeps the "zero new hard dependencies from this work" success metric honest
  where it matters (the ML/embedding path) while accepting that the plugin mechanism
  itself needs a small, deliberately-scoped new dependency — the requirements doc treats
  those as separate concerns (`Cargo.toml` gains zero *ML* deps; the plugin
  install/verify machinery is explicitly in scope).

## 3. Checksum verification: `sha2`

**Recommendation: `sha2 = "0.11"`** (RustCrypto; current stable as of Sept 2026;
MSRV 1.85.0, well under the repo's `rustc 1.98.0`).

- Matches the requirements doc's "checksum check against a known-good manifest" minimum
  bar (Feasibility Risks / Rabbit Holes) without building real signing infra.
- Usage shape: download to a temp file, hash with `Sha256::digest`, compare hex against
  the manifest's `sha256` field for that platform before `fs::rename` into the plugins
  directory and `chmod +x` (Unix) — never execute-then-verify.

## 4. Manifest format: reuse `serde_json`, don't add `toml`

The research question flags `toml`/`serde` as a candidate for the plugin manifest
(name, per-platform download URLs, checksum, version). Two real options:

- **`serde_json` (recommended, zero new dependency)** — every other kibitzer config
  surface (`.claude/inspect.json`, `cache.json`, hook payloads) is JSON via
  `serde_json`, already a hard dependency. A plugin manifest fetched over HTTP or
  bundled as a release asset gains nothing from being TOML instead, and JSON keeps the
  file consistent with the rest of the repo's config story.
- **`toml = "0.9"` (current stable: 0.9.8)** — only worth it if the manifest is meant to
  be hand-edited by a human (TOML's readability edge over JSON for nested tables), which
  doesn't clearly apply to a machine-generated/machine-read plugin registry. Available as
  a fallback if Phase 3 planning decides authorability matters, but it is a new
  dependency for marginal benefit given the existing all-JSON convention.

**Verdict for planning**: default to JSON unless there's a concrete reason (e.g. the
manifest doubles as something the maintainer hand-edits per release) to introduce `toml`.

## 5. `cargo-dist`: publishing the stub plugin as a second artifact

Repo currently pins `cargo-dist-version = "0.32.0"` in `dist-workspace.toml`
([dist-workspace.toml:8](https://github.com/tstapler/kibitzer/blob/master/dist-workspace.toml#L8))
— confirmed still current in September 2026 (WebSearch: no newer 0.x/1.x release found;
latest changelog entries are npm-installer/Actions-version bumps, nothing affecting
workspace/multi-package behavior).

Key mechanics from the `cargo-dist` workspace guide
(https://axodotdev.github.io/cargo-dist/book/workspaces/workspace-guide.html):

- **Each Cargo *package* that defines a binary becomes its own independent "App"** in
  cargo-dist's model — separate archives/installers, but all published to the *same*
  GitHub Release. This is exactly "separately installable from the same release" that
  the requirements doc wants for the stub plugin.
- **Multiple `[[bin]]` targets (or `src/bin/*.rs`) within the *same* package are bundled
  into one archive**, not split. So adding the stub as `src/bin/kibitzer-stub-plugin.rs`
  inside the existing `kibitzer` crate would NOT give it an independently downloadable
  artifact — it would ship inside `kibitzer`'s own tarball, defeating the "optional,
  separately installable" goal and (worse) reintroducing exactly the coupling this
  project exists to avoid.
- **The correct shape is a second workspace member package**, e.g.
  `crates/kibitzer-stub-plugin/` with its own `Cargo.toml` and `src/main.rs`, added to
  `dist-workspace.toml`'s `members` as `"cargo:crates/kibitzer-stub-plugin"` alongside
  the existing `"cargo:."`.
- **This requires converting `Cargo.toml`'s currently-empty `[workspace]` table into a
  real one.** Today's `Cargo.toml` ([Cargo.toml:1-3](https://github.com/tstapler/kibitzer/blob/master/Cargo.toml#L1-L3))
  has a bare `[workspace]` purely to opt the crate *out* of the parent
  `stapler-scripts` workspace (per its own comment) — it isn't yet a real multi-member
  workspace. Adding `members = [".", "crates/kibitzer-stub-plugin"]` there (or moving
  the main crate under `crates/kibitzer/` — a bigger, likely out-of-scope reshuffle) is
  the concrete Phase 3 planning item this implies.
- A package can opt out of `dist` publishing via `publish = false` in its `Cargo.toml`
  or `dist = false` under `[package.metadata.dist]` if a workspace member shouldn't ship
  — not needed here since the stub plugin's whole point is to ship, but useful if a
  future internal-only crate is added to the workspace.
- This satisfies the "real GitHub Release artifact vs. local-fixture" open question in
  Feasibility Risks with a concrete, low-cost answer: yes, a second small `cargo-dist`
  app in the existing release pipeline, no new CI system needed.

## 6. `clap` subcommand pattern

`src/main.rs` already has the exact nested-subcommand shape to copy
([src/main.rs:49-95](https://github.com/tstapler/kibitzer/blob/master/src/main.rs#L49-L95)):
top-level `Command` variants like `Daemon { #[command(subcommand)] action: DaemonAction }`
and `Check { #[command(subcommand)] check: CheckCommand }`, each with its own nested enum
(`DaemonAction`, `CheckCommand`, `ArchitectureAction`). The plugin mechanism should add:

```rust
Plugin {
    #[command(subcommand)]
    action: PluginAction,
},
```
```rust
enum PluginAction {
    Install { name: String },
    List,
    Uninstall { name: String },
}
```

No new `clap` dependency or feature flag needed — `features = ["derive"]` is already
enabled.

## 7. Registration into `.claude/inspect.json`: reuse `src/install.rs`'s pattern

`src/install.rs` (`run_install`, [src/install.rs:14-50](https://github.com/tstapler/kibitzer/blob/master/src/install.rs#L14-L50))
is the direct template for "read existing JSON config, merge in a new entry, support
`--dry-run`, write back" — it already does this for `settings.json`'s hook list. The
plugin-install subcommand doing the same read-merge-write against `.claude/inspect.json`
(adding a `Check { command, output_format: Sarif, .. }` entry, per
[`src/config.rs:28-77`](https://github.com/tstapler/kibitzer/blob/master/src/config.rs#L28-L77))
is structurally the same operation, not a new pattern to invent.

## 8. SARIF-as-transport: a confirmed real gap, not hypothetical

`render_sarif_output` (`src/check.rs:468`, exercised by `mod sarif_tests` at
`src/check.rs:521`) only reads `result.level`, `result.message.text`, `result.ruleId`,
and `result.locations` from a SARIF `results[]` entry. It does **not** read
`result.properties` — the SARIF 2.1.0 bag where a `rank`/confidence score or extra
structured metadata (e.g. "which comment/identifier pair triggered this") would have to
live for a future embedding checker. Confirmed by reading the parser directly, not
inferred from the spec: today, any such metadata would be silently dropped even though
SARIF format itself supports it. This is a concrete, scoped extension `render_sarif_output`
will need (reading `properties` into the flattened text or a new `CheckResult` field) —
flagged for Phase 3 planning as the answer to the "does SARIF lose information" Feasibility
Risk, rather than "TBD."

## 9. Testing pattern for the stub plugin

`tests/hook_contract.rs` already establishes the right integration-test shape: spawn the
real compiled binary via `env!("CARGO_BIN_EXE_kibitzer")` ([tests/hook_contract.rs:44](https://github.com/tstapler/kibitzer/blob/master/tests/hook_contract.rs#L44))
against an isolated temp repo with a private `.claude/inspect.json` and a private
`XDG_CACHE_HOME`. If the stub plugin becomes its own workspace package/binary target,
Cargo automatically exposes `CARGO_BIN_EXE_kibitzer-stub-plugin` (or however the package
is named) to the `kibitzer` package's own test binaries as long as it's declared as a
`[dev-dependencies]` path dependency — no separate build step needed to make the test
find it. The automated test the requirements doc wants (install → register → run →
assert output) should extend this exact harness rather than build a new one.

## Dependency additions summary

| Crate | Version | Purpose | New hard dep? |
|---|---|---|---|
| `ureq` | `"3"` (3.3.0 current) | Sync HTTP download of plugin manifest + binary | Yes — small, sync-only |
| `sha2` | `"0.11"` | Verify downloaded binary's checksum | Yes — small, no transitive bloat |
| `serde_json` | already present | Plugin manifest format (reuse, don't add `toml`) | No |
| `clap` | already present (`derive`) | `Plugin { Install, List, Uninstall }` subcommand | No |
| `toml` | `"0.9"` (0.9.8 current) | Only if Phase 3 planning decides the manifest should be hand-editable | Deferred/optional |

None of these touch `checker::registry()` or `default_checks()` — the ONNX/embedding
dependency stays fully isolated in the plugin binary's own `Cargo.toml`, satisfying the
"core binary gains zero new ML dependencies" success metric.
