# Research: Architecture — checker-plugin-system

Scope: current-state read of `src/config.rs` (1079 lines, read in full), `src/checker.rs`
(421 lines, read ~1-160), `src/check.rs` (2049 lines, read the `run_check`/SARIF/dispatch
sections), `src/daemon.rs` (260 lines, read in full), `src/install.rs` (245 lines, read in
full — the closest existing precedent for a lifecycle CLI module), `src/main.rs` (476
lines, read ~1-260), `src/mcp.rs` (read `list_checks`/`run_checks`/`architecture_assessment`
sections), `src/cache.rs` (read `default_cache_path`/`Cache::get`/`put`), `src/glob.rs` and
`docs/output-formats.md` (both read in full). All line numbers below are against
`ae1d8cdb59ba03bb689540813ae3254dd75211ba` (current `master` at research time,
2026-09-06).

Prior-doc reconciliation: `project_plans/kibitzer/research/architecture.md` (2026-08-22,
the architecture_checker/Component-system research) is **not directly reusable for this
project** — it's about a different extension axis (whole-repo `ArchitectureChecker`/
`DeclarationChecker` rules, not `command`-based external checks) — but two of its
findings *did* land in the two weeks since and are load-bearing precedent here:

1. Its §6 recommendation ("a merge-point function resolving between two registries,
   `AnyArchitectureChecker`") was implemented almost exactly as proposed: `check.rs:816-821`
   is `lookup_any_architecture_checker`, dispatching to `architecture_checks::lookup`
   ([`src/architecture_checks.rs:45`](https://github.com/tstapler/kibitzer/blob/ae1d8cdb59ba03bb689540813ae3254dd75211ba/src/architecture_checks.rs#L45))
   or `declaration_checks::lookup`
   ([`src/declaration_checks.rs:207`](https://github.com/tstapler/kibitzer/blob/ae1d8cdb59ba03bb689540813ae3254dd75211ba/src/declaration_checks.rs#L207)).
   This is a proof-of-precedent that **this codebase already extends its checker
   surface by adding a merge-point function, not by widening an existing trait or
   `Check` field** — the same shape this project's plugin mechanism should follow (§3).
2. §1's mutual-exclusion/registry table is still accurate in kind but stale in line
   numbers; re-verified below (§1).

## 1. Exact current shape of the extension points

### `Check` struct — [`src/config.rs:27-77`](https://github.com/tstapler/kibitzer/blob/ae1d8cdb59ba03bb689540813ae3254dd75211ba/src/config.rs#L27-L77)

```rust
pub struct Check {
    pub name: String,
    pub command: Option<String>,             // shell command, {file}/{changed_lines} substituted
    pub checker: Option<String>,              // checker::registry() lookup key
    pub architecture_checker: Option<String>, // lookup_any_architecture_checker() lookup key
    pub severity: Severity,
    pub scope: Vec<String>,                   // glob.rs::matches_scope patterns
    pub triggers: Vec<String>,
    pub message: Option<String>,
    pub output_format: Option<OutputFormat>,  // currently only Sarif
}
```

Mutual exclusion (`command`/`checker`/`architecture_checker`, exactly one) is enforced in
`validate()` at [`src/config.rs:399-423`](https://github.com/tstapler/kibitzer/blob/ae1d8cdb59ba03bb689540813ae3254dd75211ba/src/config.rs#L399-L423)
(count-and-bail, same shape the prior doc described, now at a different line number —
confirms "re-verify, don't trust" was the right call). `output_format` requires `command`
(`config.rs:454-461`). `architecture_checker` requires `triggers ⊆ {"batch"}`
(`config.rs:444-452`).

`Check::kind()`/`is_per_file()` ([`config.rs:89-117`](https://github.com/tstapler/kibitzer/blob/ae1d8cdb59ba03bb689540813ae3254dd75211ba/src/config.rs#L89-L117))
derive dispatch shape from which field is set — a plugin check that sets
`command`+`output_format: sarif` is indistinguishable, at this layer, from a hand-authored
one. That's the whole point: **nothing here needs to change for a plugin check to work**,
only for it to *appear* in the effective check list without the user typing it.

### `Checker` trait + `registry()` — [`src/checker.rs:129-171`](https://github.com/tstapler/kibitzer/blob/ae1d8cdb59ba03bb689540813ae3254dd75211ba/src/checker.rs#L129-L171)

In-process only (`name/description/language/file_globs/check`), a flat
`Vec<Box<dyn Checker>>` built fresh on every call — **explicitly out of scope** per the
requirements ("Changing or generalizing the existing native `Checker` trait/`registry()`
path — this project targets the external/optional path only"). Confirmed: nothing here
needs to change.

### `find_effective_config()` / overlay logic — [`src/config.rs:618-640`](https://github.com/tstapler/kibitzer/blob/ae1d8cdb59ba03bb689540813ae3254dd75211ba/src/config.rs#L618-L640)

```rust
pub fn find_effective_config(start: &Path) -> Result<(Config, PathBuf)> {
    match find_config(start)? {                       // walks upward for .claude/inspect.json
        Some((local, root)) => {
            let checks = merge_checks(default_checks(), &local);  // §below
            Ok((Config { checks, architecture: local.architecture, disabled: vec![] }, root))
        }
        None => Ok((Config { checks: default_checks(), .. }, start_dir(start))),
    }
}
```

`merge_checks()` ([`config.rs:601-616`](https://github.com/tstapler/kibitzer/blob/ae1d8cdb59ba03bb689540813ae3254dd75211ba/src/config.rs#L601-L616)):
drops any default whose `name` is in `local.disabled`, then for each `local.checks` entry
either replaces a same-named survivor in place or appends it. `default_checks()`
([`config.rs:534-599`](https://github.com/tstapler/kibitzer/blob/ae1d8cdb59ba03bb689540813ae3254dd75211ba/src/config.rs#L534-L599))
is a hardcoded `Vec<Check>` built from a `native_check()` helper
([`config.rs:512-524`](https://github.com/tstapler/kibitzer/blob/ae1d8cdb59ba03bb689540813ae3254dd75211ba/src/config.rs#L512-L524)) —
this is the exact struct-literal shape a synthesized plugin `Check` should mirror (see §3).

**This is the single seam every consumer goes through** — confirmed by grep, not
assumption: `src/run.rs:110`, `src/daemon.rs:149,240` (via `try_run_checks_via_daemon`/
`run_checks_smart`), `src/mcp.rs:274,301,476` (`list_checks`, `architecture_assessment`,
`run_checks`) all call `find_effective_config` and then iterate `config.checks` generically.
None of them branch on *why* a `Check` exists (native default vs. hand-authored vs., after
this project, plugin-registered) — they only branch on which of `command`/`checker`/
`architecture_checker` is set. This is the load-bearing fact for §3(b): **injecting a
plugin's `Check` into the `Vec` returned by `find_effective_config` requires zero changes
to `run.rs`, `daemon.rs`, `hook.rs`, or `mcp.rs`.**

### Dispatch + SARIF parsing — [`src/check.rs:140-222`](https://github.com/tstapler/kibitzer/blob/ae1d8cdb59ba03bb689540813ae3254dd75211ba/src/check.rs#L140-L222)

`run_check()`: if `check.checker.is_some()`, dispatch to `run_native_check`; otherwise
build `cmd_str` via `substitute_command()` (`{file}`/`{changed_lines}` replacement,
[`check.rs:387-403`](https://github.com/tstapler/kibitzer/blob/ae1d8cdb59ba03bb689540813ae3254dd75211ba/src/check.rs#L387-L403)),
run it with `Command::new("sh").arg("-c").arg(&cmd_str).current_dir(repo_root)`
(`check.rs:155-160`), and branch on `check.output_format`:
`Some(OutputFormat::Sarif) => render_sarif_output(&output.stdout)` (falls back to raw
stdout+stderr if parsing fails, `check.rs:164-178`) or `None` → raw text. SARIF parsing
(`SarifLog`/`SarifRun`/`SarifResult`/…, `check.rs:409-520`) is a `serde_json`-only
deserialization of the subset of the schema kibitzer reads (level, message, physical
location, region) — no new crate, no schema validation library. `run_checks_for_trigger`
([`check.rs:1122-1141`](https://github.com/tstapler/kibitzer/blob/ae1d8cdb59ba03bb689540813ae3254dd75211ba/src/check.rs#L1122-L1141))
filters by `triggers`/`scope` then calls `run_check` per surviving `Check` — again fully
generic, no per-check-origin branching.

**Conclusion for §1**: every extension point a plugin check would flow through already
exists, is already generic over "why does this `Check` exist," and needs zero
modification. The only genuinely new code is (a) something that produces `Check` values
from an installed-plugin registry and (b) splicing that `Vec<Check>` into
`find_effective_config`'s output.

## 2. Where a new "plugin" concept plumbs in

### (a) Manifest/registry format and location

Precedent already in this repo for "kibitzer's own stuff on disk," both following the
same shape (env-var override, then a `$HOME`-relative fallback — no `dirs`/`directories`
crate dependency, confirmed absent from `Cargo.toml:14-32`):

- `cache::default_cache_path()` ([`src/cache.rs:148-157`](https://github.com/tstapler/kibitzer/blob/ae1d8cdb59ba03bb689540813ae3254dd75211ba/src/cache.rs#L148-L157)):
  `$XDG_CACHE_HOME/kibitzer/cache.json`, else `$HOME/.cache/kibitzer/cache.json`.
  Semantically a *cache* — the code already tolerates it vanishing (`Cache::load` returns
  a fresh empty `Cache` on any read/parse failure, `cache.rs:57-62`).
- `daemon::default_socket_path()` ([`src/daemon.rs:40-46`](https://github.com/tstapler/kibitzer/blob/ae1d8cdb59ba03bb689540813ae3254dd75211ba/src/daemon.rs#L40-L46)):
  `$XDG_RUNTIME_DIR/kibitzer-$USER.sock`, else `$TMPDIR`. Explicitly ephemeral —
  `XDG_RUNTIME_DIR` is tmpfs, reset on reboot by design; the doc comment says so.
- The installed `kibitzer` binary itself and Claude Code's `~/.claude/settings.json`
  (written by `install::run_install`, [`src/install.rs:14-55`](https://github.com/tstapler/kibitzer/blob/ae1d8cdb59ba03bb689540813ae3254dd75211ba/src/install.rs#L14-L55)) —
  not kibitzer-owned storage, a different program's config file kibitzer merges into.

**None of these fit a plugin binary + its registration metadata**, which must survive a
reboot (unlike the socket) and must not be silently discardable (unlike the cache — an
"installed" plugin that vanishes on cache-clear would be a confusing regression, since
`rm -rf ~/.cache` is a normal troubleshooting step users already associate with "safe to
delete"). Recommend a **new, `kibitzer`-owned, durable directory** — not reusing either
existing one — following the identical env-var-then-`$HOME`-fallback shape as
`default_cache_path()`, just against `$XDG_DATA_HOME` (durable per-user app data, the
correct XDG category for "installed add-ons," distinct from `$XDG_CACHE_HOME`):

```rust
// new fn, e.g. in src/plugin.rs, mirroring cache::default_cache_path's exact shape
pub fn default_plugin_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("XDG_DATA_HOME") {
        return PathBuf::from(dir).join("kibitzer").join("plugins");
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".local").join("share").join("kibitzer").join("plugins")
}
```

Three concrete options for what lives there and in what shape:

1. **One registry JSON file + per-plugin binary subdirectories** (recommended): 
   `~/.local/share/kibitzer/plugins/registry.json` (a `Vec<InstalledPlugin>`, read/write
   via plain `serde_json`, same load/save shape as `Cache::load`/`save`) plus
   `~/.local/share/kibitzer/plugins/<name>/<name>` (the executable, one directory per
   plugin so a future plugin with companion data files — a model weights file, for the
   real embedding checker this project sets up for — has somewhere to put them without
   colliding with another plugin's files). `InstalledPlugin` needs at minimum: `name`,
   `binary_path`, `version`, `sha256` (the checksum actually verified at install time —
   recording it lets `plugin list`/a future `plugin verify` re-check it without
   re-downloading), and the `Check`-construction inputs (`severity`, `scope`, `triggers`,
   `output_format`). This is the shape assumed in §4's data-flow trace below.
2. **One JSON-per-plugin file, no central registry**: `~/.local/share/kibitzer/plugins/
   <name>.json` next to `<name>/<name>` (or flattened, `<name>` binary + `<name>.json`
   metadata siblings). Avoids read-modify-write races on a single shared file if multiple
   `kibitzer plugin install` invocations somehow overlapped (irrelevant for a
   single-user, foreground CLI subcommand — no daemon-mediated concurrent writer exists
   for this path) — genuinely simpler for `uninstall` (delete two paths, no JSON
   surgery) but means "list installed plugins" requires a directory scan instead of one
   parse, and there's no single place to bump a schema version later. Reasonable
   alternative; (1)'s only real edge over it is the future schema-version field.
3. **Repo-local, inside `.claude/`**: e.g. `.claude/kibitzer-plugins.json` alongside
   `inspect.json`, or an extra key inside `inspect.json` itself. Rejected: the embedding
   checker (and "any future one with a similar profile," per requirements) is a
   *machine-level* capability — install it once, use it in every repo you touch on that
   machine, exactly like the `kibitzer` binary itself or the Claude Code hook install
   (`install::run_install`'s `--global` flag écho this same "user-level, not per-repo"
   default). A repo-local registry would need re-installing per clone and would put a
   binary path (`~/.local/share/kibitzer/plugins/howvswhy/howvswhy`) inside
   version-controlled repo config — a machine-specific absolute path checked into git,
   which is exactly the class of mistake `.gitignore`-worthy local state exists to avoid.

**Recommendation: option 1**, at `default_plugin_dir()`/`~/.local/share/kibitzer/plugins/`.

### (b) Wiring an installed plugin into the *effective* check config

Two options, both compatible with (a):

1. **Auto-inject inside `find_effective_config`** (recommended — this is what the
   requirements' success metric actually asks for: "wires it into the effective check
   config... without the user hand-editing `.claude/inspect.json`"). Add one more source
   alongside `default_checks()`:

   ```rust
   pub fn find_effective_config(start: &Path) -> Result<(Config, PathBuf)> {
       let plugin_checks = crate::plugin::registered_plugin_checks(); // new
       match find_config(start)? {
           Some((local, root)) => {
               let base = default_checks().into_iter().chain(plugin_checks).collect();
               let checks = merge_checks(base, &local);
               Ok((Config { checks, .. }, root))
           }
           None => Ok((Config { checks: default_checks().into_iter().chain(plugin_checks).collect(), .. }, start_dir(start))),
       }
   }
   ```

   A plugin check becomes indistinguishable from a built-in default at this layer — it
   runs in every repo the user touches on that machine, and `merge_checks`'s existing
   `local.disabled`-by-name suppression (`config.rs:601-616`) already gives per-repo
   opt-out for free, no new suppression mechanism needed (same convention
   `docs/suppressing-checks.md` documents for the built-in catalog today). This is the
   smallest change that satisfies the stated success metric, and it's additive: with zero
   plugins installed, `registered_plugin_checks()` returns `vec![]` and behavior is
   byte-for-byte unchanged (satisfies "uninstalling/never installing... leaves default
   behavior... completely unchanged").

   Trade-off to flag for Phase 3: this makes install *global and silent* — installing a
   heavy/slow plugin once means every future `kibitzer run`/hook firing on *every* repo
   on the machine now shells out to it, with no per-repo confirmation step. For a
   single-user personal tool this matches the existing `default_checks()` philosophy
   (pylint-style, on by default) and is explicitly what the requirement asks for, but it
   is a real behavior-change surface worth one explicit line in `kibitzer plugin
   install`'s own output ("registered — now runs in every repo; disable per-repo via
   `disabled` in `.claude/inspect.json`, or `kibitzer plugin uninstall` to remove
   everywhere").

2. **Registered-but-inert until referenced by name**: `plugin install` only places the
   binary and records it in the registry; a user must still add a `.claude/inspect.json`
   entry (`{"name": "howvswhy", "command": "~/.local/share/kibitzer/plugins/howvswhy/howvswhy {file}", "output_format": "sarif", ...}`)
   to actually run it — `kibitzer plugin install` could print that exact JSON snippet to
   copy in, or a `kibitzer plugin check-block <name>` subcommand could emit it. Safer
   (explicit per-repo opt-in, no surprise global behavior change) but **directly fails
   the stated success metric** ("without the user hand-editing `.claude/inspect.json`
   themselves") — included here only to document why it's not the default recommendation,
   not as a live alternative to build both of.

**Recommendation: option 1**, with the disclosure-message mitigation noted.

### (c) Install/uninstall subcommand's module boundary

**New `src/plugin.rs`**, following `src/install.rs`'s existing shape almost exactly (a
single-purpose lifecycle module: pure functions taking explicit paths/args, a `run_*`
entry point returning `Result<ExitCode>`, `#[cfg(test)] mod tests` at the bottom exercising
the pure logic without touching the real filesystem where possible — `install.rs`'s tests
operate on an in-memory `serde_json::Value`, not real files; `plugin.rs`'s registry
read/write is the analogous in-memory-testable unit). Contents:

- `default_plugin_dir()` / `default_registry_path()` (§2a).
- `struct InstalledPlugin { name, binary_path, version, sha256, severity, scope, triggers, output_format }` (`Serialize + Deserialize`).
- `struct Registry { plugins: Vec<InstalledPlugin> }`, `Registry::load(path)`/`save(path)` (mirrors `Cache::load`/`save`, `cache.rs:57-70`).
- `pub fn registered_plugin_checks() -> Vec<Check>` — reads the registry, maps each `InstalledPlugin` to a `Check` (mirrors `config.rs`'s own `native_check()` helper shape, `config.rs:512-524`). This is the one function `config::find_effective_config` calls into (§2b) — a **`config.rs` → `plugin.rs` call dependency**, which is consistent with, not new relative to, this codebase's existing convention of `config.rs::validate()` already reaching into sibling modules (`crate::checker::lookup`, `crate::check::lookup_any_architecture_checker`, both called directly from `config.rs`, `config.rs:424-443`) — `config.rs` is already the place that "knows about" every other check-producing module, not a leaf. Note this isn't a clean one-directional module relationship, though: `plugin.rs` itself `use`s `Check`/`OutputFormat`/`Severity` back from `config.rs` for its own `PluginManifest`/`InstalledPlugin`/`registered_plugin_checks()` types, so the two modules are bidirectional at the `use`-import level even though the *call* graph (who invokes whom) only runs one way (`config.rs` → `plugin.rs`).
- `pub fn install_plugin(name, source, expected_sha256) -> Result<()>` and `pub fn uninstall_plugin(name) -> Result<()>`, `pub fn list_plugins() -> Result<Vec<InstalledPlugin>>` — the CLI-facing operations (§3 data flow).

New `Command::Plugin { action: PluginAction }` in `main.rs`, nested exactly like
`Command::Daemon { action: DaemonAction }` (`main.rs:65-68`/`167-175`/`192-`):

```rust
Plugin { #[command(subcommand)] action: PluginAction },
// ...
enum PluginAction {
    Install { name: String, #[arg(long)] source: Option<String>, #[arg(long)] checksum: Option<String> },
    Uninstall { name: String },
    List,
}
```

## 3. Integration points

- **CLI** (`src/main.rs`): new `Plugin`/`PluginAction` subcommand tree only (§2c). No
  changes to `Run`/`Hook`/`Mcp`/`Lsp`/`Daemon`/`Check`/`Architecture` — none of them need
  to know a check came from a plugin.
- **Daemon** (`src/daemon.rs`): **no code change needed for check dispatch** — confirmed
  by reading `try_run_checks_via_daemon`/`run_checks_smart` (`daemon.rs:212-260`), both of
  which call `find_effective_config` and then `run_checks_for_trigger` exactly like
  `run.rs`/`hook.rs` do. Once `find_effective_config` includes plugin checks (§2b), the
  daemon picks them up transparently on its next `find_effective_config` call — same as
  any other config change.

  **One real gap found, not hypothetical**: the daemon's result cache
  (`Cache::get`/`put`, [`src/cache.rs:72-110`](https://github.com/tstapler/kibitzer/blob/ae1d8cdb59ba03bb689540813ae3254dd75211ba/src/cache.rs#L72-L110))
  invalidates on `config_stamp`, a `Stamp` (mtime+len) of exactly one file:
  `repo_root.join(CONFIG_DIR).join(CONFIG_FILENAME)` — i.e. that repo's own
  `.claude/inspect.json` (`daemon.rs:149,240`). Installing or uninstalling a plugin
  changes `registered_plugin_checks()`'s output *without touching that file* — a repo
  whose `inspect.json` hasn't changed would keep serving cached results computed under
  the pre-install (or pre-uninstall) check set until something else invalidates the
  cache (a subsequent edit to the same file bumps `file_stamp`, but a *different* file in
  the repo wouldn't). **This needs either**: (a) `Stamp`ing the plugin registry file too
  and folding it into `config_stamp`'s equality check (the minimal fix — `cache.rs`'s
  `Stamp` type is already `pub(crate)` and reusable), or (b) documenting "restart the
  daemon after `plugin install`/`uninstall`" as the accepted v1 limitation (matching this
  project's stated low-risk/single-user posture). Flag for Phase 3 sizing — (a) is a
  small, contained change (a few lines in `cache.rs`/`daemon.rs`) but is a genuine
  correctness gap this research surfaced, not a hypothetical.

- **MCP server** (`src/mcp.rs`): confirmed no special-casing needed for either tool.
  `list_checks` ([`mcp.rs:272-289`](https://github.com/tstapler/kibitzer/blob/ae1d8cdb59ba03bb689540813ae3254dd75211ba/src/mcp.rs#L272-L289))
  just formats `config.checks` generically (`"{name} ({severity:?}, scope={scope:?})"`) —
  a plugin check shows up exactly like any other, no "not installed" vs. "disabled"
  distinction exists at this layer today, and none is needed for correctness: an
  *installed-but-somehow-missing-binary* plugin check would run via `run_check`'s normal
  `Command::new("sh").arg("-c")...` path, `sh` would report "command not found" (exit
  127) on stderr, and that surfaces through the completely ordinary
  `!passed_raw` → failing-check path (`check.rs:162-178`) — indistinguishable from any
  other broken shell command a user hand-configured, which is arguably *correct*
  behavior (no new failure-mode taxonomy needed), just not maximally friendly. A nicer
  message ("plugin 'x' binary not found — try `kibitzer plugin install x`") is a
  possible small enhancement but is not required for `list_checks`/`run_checks` to work
  correctly, since the underlying invariant (registry only lists what `install_plugin`
  successfully wrote to disk) should make "registered but missing" a rare
  self-inflicted-tampering case, not a normal state. `run_checks`
  ([`mcp.rs:474-495`](https://github.com/tstapler/kibitzer/blob/ae1d8cdb59ba03bb689540813ae3254dd75211ba/src/mcp.rs#L474-L495))
  and `architecture_assessment`'s per-file/per-check loop are equally generic — no
  changes needed.

## 4. Data flow — install → registration → run

```
kibitzer plugin install howvswhy --source <url-or-local-path> --checksum <sha256>
  └─ plugin::install_plugin()
       1. fetch artifact to a temp path              [NEW — mechanism TBD, see note below]
       2. verify sha256 against --checksum            [NEW, plugin.rs]
       3. mkdir -p default_plugin_dir()/howvswhy/     [NEW, plugin.rs]
       4. move artifact → .../howvswhy/howvswhy, chmod +x   [NEW, plugin.rs — Unix-only
          fs::Permissions/PermissionsExt is a new-to-this-module but not new-to-this-repo
          constraint; daemon.rs already assumes Unix via std::os::unix::net::UnixListener]
       5. Registry::load(default_registry_path()), push InstalledPlugin, .save()  [NEW]

  (later, any invocation — kibitzer run / hook / daemon / mcp)
  └─ config::find_effective_config(start)             [CHANGED — one line: chain in
                                                         plugin::registered_plugin_checks()]
       └─ plugin::registered_plugin_checks()           [NEW — Registry::load + map to Check]
       └─ merge_checks(default_checks() ++ plugin_checks, &local)  [UNCHANGED — already
                                                                      generic by Check.name]
  └─ run_checks_for_trigger / run_check                [UNCHANGED — dispatches on
                                                          command/output_format exactly as
                                                          for any hand-authored check]
  └─ check.rs's SARIF renderer                          [UNCHANGED]

kibitzer plugin uninstall howvswhy
  └─ plugin::uninstall_plugin(): rm -rf .../howvswhy/, Registry::load/remove-by-name/.save()
       (next find_effective_config call omits it — no other state to clean up, confirmed
       by the fact that nothing outside config.rs/plugin.rs/cache.rs even knows a check's
       provenance)
```

Functions needing new code: `plugin::install_plugin`, `uninstall_plugin`,
`list_plugins`, `registered_plugin_checks`, `Registry::{load,save}`,
`default_plugin_dir`/`default_registry_path`, plus one line in
`config::find_effective_config` and the new `main.rs` subcommand plumbing. Everything
downstream of `find_effective_config` (`run.rs`, `daemon.rs`, `hook.rs`, `mcp.rs`,
`check.rs`) is confirmed untouched — the synthesized `Check` is not "a new kind of thing,"
it's data indistinguishable in shape from a hand-written `.claude/inspect.json` entry, so
every consumer's existing genericity over `Check` absorbs it for free. The daemon
cache-staleness gap (§3) is the one place that genericity has a real, not just
theoretical, hole.

**Explicitly out of this agent's scope** (architecture of the *mechanism*, not the
distribution pipeline): which HTTP mechanism `install_plugin`'s step 1 uses (shelling out
to a system `curl`, or adding a minimal HTTP-client crate) is a distribution/dependency
question the requirements' own Rabbit Holes/Feasibility Risks flag for separate research
— it doesn't change any of the module boundaries or data flow above, since step 1 is a
black box that must produce "bytes on disk at a known path" regardless of how.

## 5. EventStorming table — skipped

Per the requirement's own skip condition and the prior architecture doc's §7 precedent:
this is a CLI-driven lifecycle feature with one human actor (the maintainer running
`plugin install`/`uninstall`) and mechanical downstream consumers (daemon/CLI/MCP reading
config), not a multi-actor business domain with policies reacting to other actors' events.
A table would just restate §4's linear pipeline with extra ceremony.

## Summary of concrete recommendations for Phase 3

1. New durable storage location: `default_plugin_dir()` at `$XDG_DATA_HOME/kibitzer/plugins`
   (fallback `~/.local/share/kibitzer/plugins`), mirroring `cache::default_cache_path`'s
   exact env-var-then-`$HOME` shape but a new base (`XDG_DATA_HOME`, not `XDG_CACHE_HOME`)
   since installed plugins are durable data, not a discardable cache.
2. Registry shape: one `registry.json` (`Vec<InstalledPlugin>`) plus one subdirectory per
   plugin for its binary (and future companion files, e.g. model weights).
3. Auto-inject: `find_effective_config` chains `plugin::registered_plugin_checks()`
   alongside `default_checks()` before `merge_checks` — satisfies the stated success
   metric, reuses the existing `disabled`-by-name suppression mechanism for per-repo
   opt-out, and is a one-line change at the actual call site.
4. New module `src/plugin.rs`, following `src/install.rs`'s existing lifecycle-module
   shape; new `Command::Plugin{action}` in `main.rs` nested like `Command::Daemon`.
5. Confirmed zero changes needed in `run.rs`/`daemon.rs`'s check-dispatch path,
   `hook.rs`, or `mcp.rs`'s `list_checks`/`run_checks`/`architecture_assessment` — all
   already iterate `Config.checks` generically via `find_effective_config`.
6. One genuine gap found (not hypothetical): the daemon's `Cache` invalidates only on
   the repo's own `inspect.json` `Stamp`, not on the plugin registry file — installing/
   uninstalling a plugin while a daemon is running can serve stale cached results for a
   repo whose `inspect.json` didn't change. Needs a decision in Phase 3: fold the
   registry's `Stamp` into `config_stamp` (small, contained fix) vs. document "restart
   daemon after plugin install/uninstall" as an accepted v1 limitation.
7. No MCP-level "not installed vs. disabled" distinction is required for correctness —
   a missing plugin binary already surfaces as an ordinary failing shell command via the
   existing exit-127-on-`sh` path. A friendlier error message is a nice-to-have, not a
   architectural requirement.
