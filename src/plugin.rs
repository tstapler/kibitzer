//! Domain model for the checker-plugin system: manifest/registry types and the
//! version-compatibility comparator (ADR-003). No CLI or network I/O lives here yet —
//! see the implementation plan's Phase 3/4 for `install_plugin`/`fetch_manifest`/CLI wiring.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use ureq::ResponseExt;

use crate::config::{self, Check, OutputFormat, Severity};

/// A validated plugin identifier. Constructed only via [`PluginName::parse`] — including
/// on deserialization, via `#[serde(try_from = "String")]` below, so a `registry.json`
/// (or any other wire source) can never construct one bypassing the allowlist — which
/// allowlists `^[A-Za-z0-9_-]{1,64}$` — closing the door on `/`, `\`, `..`, and any
/// future problem character at once, so an unvalidated name can never reach a
/// filesystem join (`default_plugin_dir().join(name)`, `fs::remove_dir_all`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct PluginName(String);

impl PluginName {
    pub fn parse(s: &str) -> Result<Self> {
        let valid = !s.is_empty()
            && s.len() <= 64
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
        if !valid {
            anyhow::bail!("invalid plugin name: {s:?}");
        }
        Ok(Self(s.to_string()))
    }
}

impl TryFrom<String> for PluginName {
    type Error = anyhow::Error;

    /// Backs `#[serde(try_from = "String")]` above — every deserialization of a
    /// `PluginName` (a hand-written `#[derive(Deserialize)]` would instead build the
    /// tuple field directly from the wire string, skipping `parse`'s validation
    /// entirely) routes through here, so a `registry.json` with a name like
    /// `"../../etc"` fails to deserialize instead of silently reopening the
    /// path-traversal invariant `parse` exists to close.
    fn try_from(s: String) -> Result<Self> {
        Self::parse(&s)
    }
}

impl AsRef<str> for PluginName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for PluginName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// One entry inside [`PluginManifest::targets`]: the download location and expected
/// digest for one Rust target-triple.
#[derive(Debug, Clone, Deserialize)]
pub struct PluginTarget {
    pub url: String,
    pub sha256: String,
}

/// A JSON document (fetched over HTTPS or read from a local path) describing one
/// plugin release. Never persisted long-term — consumed once at install time.
/// `name` is untrusted download content and is never used as the `InstalledPlugin`
/// identity; that identity always comes from the CLI-supplied, already-validated
/// [`PluginName`].
#[derive(Debug, Clone, Deserialize)]
pub struct PluginManifest {
    pub name: String,
    pub version: String,
    pub min_kibitzer_version: String,
    pub severity: Severity,
    pub scope: Vec<String>,
    pub triggers: Vec<String>,
    pub output_format: OutputFormat,
    pub targets: HashMap<String, PluginTarget>,
}

/// One entry in the local [`Registry`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledPlugin {
    pub name: PluginName,
    pub version: String,
    pub min_kibitzer_version: String,
    pub sha256: String,
    pub binary_path: PathBuf,
    pub severity: Severity,
    pub scope: Vec<String>,
    pub triggers: Vec<String>,
    pub output_format: OutputFormat,
}

/// The kibitzer-owned, durable collection of installed plugins persisted at
/// [`default_registry_path()`]. `load`/`save` mirror `Cache::load`/`Cache::save`
/// (`src/cache.rs:57-70`).
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Registry {
    pub plugins: Vec<InstalledPlugin>,
}

impl Registry {
    pub fn load(path: &Path) -> Self {
        fs::read_to_string(path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    pub fn save(path: &Path, registry: &Registry) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, serde_json::to_string_pretty(registry)?)?;
        Ok(())
    }

    /// Backs Task 4.3.1b's `run_check` preflight and `mcp.rs`'s `list_checks` tag:
    /// `Some(path)` iff `check_name` names a plugin in `self` whose binary no longer
    /// exists on disk (e.g. removed out-of-band, or the whole plugin directory deleted);
    /// `None` both when `check_name` isn't a registered plugin at all (a hand-authored
    /// check is unaffected) and when it is registered and its binary is present.
    ///
    /// Every caller (`run_check`/`run_checks_for_trigger` and their callers in
    /// `run.rs`/`daemon.rs`/`lsp.rs`/`mcp.rs`) loads the `Registry` once per batch/
    /// request and calls this directly, rather than each check reloading and
    /// reparsing `registry.json` from disk — wasted work even when zero plugins are
    /// installed, and multiplicative across a batch of files.
    pub fn missing_binary_for(&self, check_name: &str) -> Option<PathBuf> {
        self.plugins
            .iter()
            .find(|p| p.name.as_ref() == check_name)
            .filter(|p| !p.binary_path.exists())
            .map(|p| p.binary_path.clone())
    }
}

/// `$XDG_DATA_HOME/kibitzer/plugins`, falling back to `$HOME/.local/share/kibitzer/plugins`
/// — mirrors `cache::default_cache_path()`'s exact env-var-then-`$HOME` shape
/// (`src/cache.rs:148-157`) against a durable (not cache-clearable) XDG base.
pub fn default_plugin_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("XDG_DATA_HOME") {
        return PathBuf::from(dir).join("kibitzer").join("plugins");
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("kibitzer")
        .join("plugins")
}

pub fn default_registry_path() -> PathBuf {
    default_plugin_dir().join("registry.json")
}

/// Synthesizes one [`Check`] per installed plugin (Task 4.1.1a), in the same
/// `command`-with-`{file}`-substitution shape `config::native_check()` produces for a
/// hand-authored default — `find_effective_config` chains these in alongside
/// `default_checks()` so an installed plugin runs without editing `.claude/inspect.json`.
/// An empty/missing [`Registry`] (the common case: no plugins installed) yields an empty
/// `Vec`, leaving `default_checks()`'s catalog completely unchanged.
pub fn registered_plugin_checks() -> Vec<Check> {
    Registry::load(&default_registry_path())
        .plugins
        .into_iter()
        .map(|p| Check {
            name: p.name.to_string(),
            command: Some(format!("{} {{file}}", p.binary_path.display())),
            checker: None,
            architecture_checker: None,
            severity: p.severity,
            scope: p.scope,
            triggers: p.triggers,
            message: None,
            output_format: Some(p.output_format),
        })
        .collect()
}

/// Splits a plain `MAJOR.MINOR.PATCH` version string into a comparable tuple. Never
/// panics — anything that isn't exactly three numeric dot-separated components (too
/// few/many parts, non-numeric, a prerelease/build suffix like `"0.2.0-rc1"`) is a
/// clean `Err` naming the offending string.
pub fn parse_plain_version(s: &str) -> Result<(u64, u64, u64)> {
    let parts: Vec<&str> = s.split('.').collect();
    let [major, minor, patch] = parts.as_slice() else {
        anyhow::bail!("invalid version string (expected MAJOR.MINOR.PATCH): {s:?}");
    };
    let parse_component = |part: &str| {
        part.parse::<u64>().map_err(|_| {
            anyhow::anyhow!("invalid version string (expected MAJOR.MINOR.PATCH): {s:?}")
        })
    };
    Ok((
        parse_component(major)?,
        parse_component(minor)?,
        parse_component(patch)?,
    ))
}

/// Whether `current` satisfies `min` — parses both as `(u64, u64, u64)` and compares
/// numerically (ADR-003), never lexicographically (`"0.10.0" >= "0.9.0"` is `true`).
pub fn version_meets_minimum(min: &str, current: &str) -> Result<bool> {
    Ok(parse_plain_version(current)? >= parse_plain_version(min)?)
}

/// Hosts a remote plugin manifest/binary `source` (or a redirect target) is trusted to be
/// fetched from (ADR-002). `github.com`/`api.github.com` cover the manifest/API surface;
/// any `*.githubusercontent.com` subdomain covers GitHub's release-asset redirect target,
/// whose exact subdomain has changed over time.
pub const PLUGIN_HOST_ALLOWLIST: &[&str] = &["github.com", "api.github.com"];

/// The single implementation of ADR-002's allowlist rule — both `fetch_manifest` and
/// `fetch_target_bytes` call this on a parsed URL's host (pre-request) and on the
/// response's final, post-redirect host, instead of each re-deriving the rule.
pub fn is_allowed_host(host: &str) -> bool {
    PLUGIN_HOST_ALLOWLIST.contains(&host) || host.ends_with(".githubusercontent.com")
}

fn is_http_url(source: &str) -> bool {
    source.starts_with("http://") || source.starts_with("https://")
}

fn reject_disallowed_host(host: &str) -> Result<()> {
    if is_allowed_host(host) {
        Ok(())
    } else {
        anyhow::bail!(
            "'{host}' is not an allowed host (expected github.com, api.github.com, or a *.githubusercontent.com subdomain)"
        )
    }
}

/// Hops manually followed by [`fetch_source_bytes`] before giving up — generous enough
/// for any legitimate GitHub release-asset redirect chain (typically one hop,
/// `github.com`/`api.github.com` -> an `*.githubusercontent.com` object store) while
/// still bounding a redirect loop.
const MAX_REDIRECT_HOPS: u8 = 5;

/// Resolves a `Location` header value against the URI it was returned for. Handles an
/// absolute URI (the common case for GitHub's redirects) directly; a `//host/path`
/// network-path reference or a `/path` absolute-path reference against `base`'s
/// scheme/authority; and a bare relative reference against `base`'s path directory.
fn resolve_redirect_uri(base: &ureq::http::Uri, location: &str) -> Result<ureq::http::Uri> {
    if let Ok(parsed) = location.parse::<ureq::http::Uri>()
        && parsed.scheme().is_some()
    {
        return Ok(parsed);
    }

    let scheme = base.scheme_str().unwrap_or("https");
    let resolved = if let Some(rest) = location.strip_prefix("//") {
        format!("{scheme}://{rest}")
    } else {
        let authority = base
            .authority()
            .map(|a| a.as_str())
            .ok_or_else(|| anyhow::anyhow!("redirect base URI {base} has no authority"))?;
        let path = if location.starts_with('/') {
            location.to_string()
        } else {
            let base_path = base.path();
            let dir = &base_path[..base_path.rfind('/').map(|i| i + 1).unwrap_or(0)];
            format!("{dir}{location}")
        };
        format!("{scheme}://{authority}{path}")
    };
    resolved
        .parse()
        .with_context(|| format!("resolving redirect target {location:?} against {base}"))
}

/// One hop's outcome inside [`fetch_source_bytes`]'s manual redirect loop.
enum RedirectHop {
    Body(Vec<u8>),
    Redirect(ureq::http::Uri),
}

/// Performs exactly one request against `current` and returns either the (allowlist-
/// checked) response body or the next (allowlist-checked) hop to follow — the allowlist
/// check on a redirect target happens here, before [`fetch_source_bytes`]'s loop ever
/// issues the next request to it.
fn fetch_one_hop(agent: &ureq::Agent, current: &ureq::http::Uri) -> Result<RedirectHop> {
    let mut response = agent
        .get(current)
        .call()
        .with_context(|| format!("fetching {current}"))?;

    if !response.status().is_redirection() {
        reject_disallowed_host(response.get_uri().host().unwrap_or(""))?;
        let bytes = response
            .body_mut()
            .read_to_vec()
            .with_context(|| format!("reading response body from {current}"))?;
        return Ok(RedirectHop::Body(bytes));
    }

    let location = response
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| anyhow::anyhow!("redirect from {current} had no Location header"))?
        .to_string();
    let next = resolve_redirect_uri(current, &location)?;
    reject_disallowed_host(next.host().unwrap_or(""))?;
    Ok(RedirectHop::Redirect(next))
}

/// Fetches `source` as raw bytes: an `https://` URL is checked against
/// [`is_allowed_host`] before every request in the chain — including each redirect
/// hop's target, not just the final response — before its body is ever trusted;
/// `http://` is rejected outright (HTTPS-only); anything else is treated as a local
/// filesystem path and read directly, with no network access at all.
///
/// Automatic redirects are disabled (`max_redirects(0)`) and followed by hand, up to
/// [`MAX_REDIRECT_HOPS`] times, so the allowlist gates the actual outbound request to
/// each hop — not just the response ureq would otherwise have already fetched. ureq's
/// default agent follows up to 10 redirects on its own before any of this code runs,
/// which would let the real network request reach a disallowed/internal host during the
/// chain even though the old post-redirect-only check rejected the final response.
fn fetch_source_bytes(source: &str) -> Result<Vec<u8>> {
    if source.starts_with("http://") {
        anyhow::bail!("refusing to fetch over plain HTTP (HTTPS only): {source}");
    }
    if !is_http_url(source) {
        return std::fs::read(source)
            .with_context(|| format!("failed to read plugin manifest from {source:?}"));
    }

    let mut current: ureq::http::Uri = source
        .parse()
        .with_context(|| format!("parsing URL {source:?}"))?;
    reject_disallowed_host(current.host().unwrap_or(""))?;

    let agent: ureq::Agent = ureq::Agent::config_builder()
        .max_redirects(0)
        .build()
        .into();

    for _ in 0..=MAX_REDIRECT_HOPS {
        match fetch_one_hop(&agent, &current)? {
            RedirectHop::Body(bytes) => return Ok(bytes),
            RedirectHop::Redirect(next) => current = next,
        }
    }

    anyhow::bail!("too many redirects (> {MAX_REDIRECT_HOPS}) fetching {source}")
}

/// Fetches and parses a [`PluginManifest`] from `source` — an `https://` URL (subject to
/// the host allowlist) or a local filesystem path (Task 3.2.1a).
pub fn fetch_manifest(source: &str) -> Result<PluginManifest> {
    let raw = fetch_source_bytes(source)?;
    let raw = String::from_utf8(raw)
        .with_context(|| format!("plugin manifest at {source:?} was not valid UTF-8"))?;
    serde_json::from_str(&raw).with_context(|| format!("parsing plugin manifest from {source:?}"))
}

/// Fetches the raw bytes of one [`PluginTarget`]'s binary.
///
/// `allow_local` gates whether `target.url` may point at the local filesystem (a bare path
/// or `file://`-prefixed one, stripped for the automated test's fixtures): pass `true` only
/// when the manifest itself came from a local `--source` (Story 3.2.1a's local/test-fixture
/// path), which is the sole legitimate use of a local target. When `allow_local` is `false`
/// — a manifest fetched from a remote `https://` source — a manifest-supplied `target.url`
/// is trusted just as much as the manifest's own host, so letting it name a local path would
/// let any HTTPS-allowlisted-but-malicious manifest redirect the "download" to an arbitrary
/// file on disk (e.g. an SSH key), have it "checksum-verify" against a hash the same manifest
/// supplies, and get `chmod +x`'d and registered to run on every future check invocation —
/// defeating ADR-002's HTTPS-only trust model for the one artifact that matters most. In that
/// case only `https://` (and, for the same error path as `fetch_source_bytes`, an explicit
/// rejection of `http://`) is accepted.
pub fn fetch_target_bytes(target: &PluginTarget, allow_local: bool) -> Result<Vec<u8>> {
    if allow_local {
        let url = target.url.strip_prefix("file://").unwrap_or(&target.url);
        return fetch_source_bytes(url);
    }

    if !is_http_url(&target.url) || target.url.starts_with("http://") {
        anyhow::bail!(
            "plugin target must be an https:// URL when its manifest came from a remote source (got: {}) — a local/file:// target is only permitted when --source itself is a local path",
            target.url
        );
    }
    fetch_source_bytes(&target.url)
}

/// The four target triples `dist-workspace.toml` publishes release artifacts for.
const KNOWN_TARGET_TRIPLES: &[&str] = &[
    "aarch64-apple-darwin",
    "aarch64-unknown-linux-gnu",
    "x86_64-apple-darwin",
    "x86_64-unknown-linux-gnu",
];

/// Maps an (arch, os) pair — as reported by `std::env::consts::{ARCH, OS}` — to one of
/// `dist-workspace.toml`'s four published target triples. A separate, explicit-argument
/// function from [`current_target_triple`] so the mapping itself is unit-testable without
/// depending on the process's actual, unmockable `std::env::consts`.
fn target_triple_for(arch: &str, os: &str) -> Result<String> {
    let triple = match (arch, os) {
        ("aarch64", "macos") => "aarch64-apple-darwin",
        ("aarch64", "linux") => "aarch64-unknown-linux-gnu",
        ("x86_64", "macos") => "x86_64-apple-darwin",
        ("x86_64", "linux") => "x86_64-unknown-linux-gnu",
        _ => anyhow::bail!(
            "unsupported platform: {arch}-{os} (resolved to no known target triple) — supported targets: {}",
            KNOWN_TARGET_TRIPLES.join(", ")
        ),
    };
    Ok(triple.to_string())
}

/// The current process's target triple, resolved from `std::env::consts::{ARCH, OS}`.
pub fn current_target_triple() -> Result<String> {
    target_triple_for(std::env::consts::ARCH, std::env::consts::OS)
}

/// The outcome of comparing a fetched manifest's version against any already-registered
/// entry for the same [`PluginName`] — pure decision logic (Task 3.2.2a), kept separate
/// from `install_plugin`'s I/O so it's unit-testable in isolation.
#[derive(Debug, PartialEq, Eq)]
enum InstallDecision {
    /// No existing entry: run the full fetch/verify/place/register pipeline.
    Proceed,
    /// Same version already installed, no `--force`: no-op success.
    AlreadyInstalledNoOp,
    /// Same version already installed, `--force` given: restate and re-run the pipeline.
    ForceReinstallSameVersion,
    /// A different version is already installed: announce the upgrade and re-run the
    /// pipeline.
    Upgrade { old_version: String },
}

fn decide_install_action(
    existing: Option<&InstalledPlugin>,
    new_version: &str,
    force: bool,
) -> InstallDecision {
    match existing {
        None => InstallDecision::Proceed,
        Some(existing) if existing.version == new_version => {
            if force {
                InstallDecision::ForceReinstallSameVersion
            } else {
                InstallDecision::AlreadyInstalledNoOp
            }
        }
        Some(existing) => InstallDecision::Upgrade {
            old_version: existing.version.clone(),
        },
    }
}

/// Hex-encodes a byte slice. `sha2`/`digest` 0.11's `Output` type (a `hybrid-array::Array`)
/// no longer implements `LowerHex` directly the way the pre-0.11 `GenericArray` did, so
/// this repo has no `hex` crate dependency to reach for either — a one-line manual encode
/// is simpler than adding one for a single call site.
fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Hashes `bytes`, compares against `target.sha256` (ADR-002), and — only on a match —
/// writes them to a temp file beside the final destination, `chmod +x`s it (Unix), and
/// `fs::rename`s it into place. Returns the final binary path and the verified digest.
/// Never creates any directory or file under `default_plugin_dir()` when the checksum
/// doesn't match.
fn verify_and_place(
    name: &PluginName,
    bytes: &[u8],
    target: &PluginTarget,
    triple: &str,
    version: &str,
) -> Result<(PathBuf, String)> {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let actual_sha256 = hex_encode(hasher.finalize().as_ref());
    if !actual_sha256.eq_ignore_ascii_case(&target.sha256) {
        anyhow::bail!(
            "checksum mismatch for {name} v{version} (target {triple})\n  expected: {}\n  got:      {actual_sha256}\nrefusing to install — the download may be corrupted or tampered with; no file was written.",
            target.sha256
        );
    }

    let plugin_dir = default_plugin_dir().join(name.as_ref());
    fs::create_dir_all(&plugin_dir)
        .with_context(|| format!("creating plugin directory {}", plugin_dir.display()))?;
    let binary_path = plugin_dir.join(name.as_ref());
    let temp_path = plugin_dir.join(format!(".{name}.tmp"));
    fs::write(&temp_path, bytes)
        .with_context(|| format!("writing temp file {}", temp_path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Explicit mode, not `perms.mode() | 0o111` — OR-ing onto whatever the umask
        // produced can leave the binary group/world-writable under a permissive umask
        // (e.g. 0o002), which `| 0o111` would never clear.
        fs::set_permissions(&temp_path, fs::Permissions::from_mode(0o755))?;
    }
    fs::rename(&temp_path, &binary_path)
        .with_context(|| format!("renaming into place at {}", binary_path.display()))?;

    Ok((binary_path, actual_sha256))
}

/// The install lifecycle (Epic 3.2): fetch manifest -> validate the manifest's own `name`
/// against the CLI-supplied one -> version-compat gate (ADR-003) -> already-installed/
/// upgrade gate -> resolve target-triple -> fetch binary -> verify checksum (ADR-002) ->
/// place + `chmod +x` -> write the `Registry` entry.
pub fn install_plugin(name: &PluginName, source: &str, force: bool) -> Result<ExitCode> {
    let manifest = fetch_manifest(source)?;

    if manifest.name != name.as_ref() {
        anyhow::bail!(
            "manifest name does not match: CLI name '{name}', manifest name '{}' — refusing to install; re-check --source or the plugin name you intended",
            manifest.name
        );
    }

    if !version_meets_minimum(&manifest.min_kibitzer_version, env!("CARGO_PKG_VERSION"))? {
        anyhow::bail!(
            "{name} v{} requires kibitzer >= {} (current: {}) — upgrade kibitzer, or point --source at an older compatible release's manifest",
            manifest.version,
            manifest.min_kibitzer_version,
            env!("CARGO_PKG_VERSION")
        );
    }

    let registry_path = default_registry_path();
    let mut registry = Registry::load(&registry_path);
    let existing = registry.plugins.iter().find(|p| &p.name == name);

    match decide_install_action(existing, &manifest.version, force) {
        InstallDecision::AlreadyInstalledNoOp => {
            println!(
                "[kibitzer] {name} is already installed (v{}) — no changes made",
                manifest.version
            );
            return Ok(ExitCode::SUCCESS);
        }
        InstallDecision::ForceReinstallSameVersion => {
            println!(
                "[kibitzer] {name} v{} already installed — reinstalling due to --force",
                manifest.version
            );
        }
        InstallDecision::Upgrade { old_version } => {
            println!(
                "[kibitzer] upgrading {name} v{old_version} -> v{}",
                manifest.version
            );
        }
        InstallDecision::Proceed => {}
    }

    let triple = current_target_triple()?;
    let target = manifest.targets.get(&triple).ok_or_else(|| {
        anyhow::anyhow!(
            "{name} has no published artifact for your platform ({triple}, resolved to no known target triple) — supported targets: {}",
            KNOWN_TARGET_TRIPLES.join(", ")
        )
    })?;

    let allow_local_target = !is_http_url(source);
    let bytes = fetch_target_bytes(target, allow_local_target).with_context(|| {
        format!(
            "failed to download {name} v{} (target {triple}) — no files were written; try again once network access is available",
            manifest.version
        )
    })?;

    let (binary_path, actual_sha256) =
        verify_and_place(name, &bytes, target, &triple, &manifest.version)?;

    registry.plugins.retain(|p| &p.name != name);
    registry.plugins.push(InstalledPlugin {
        name: name.clone(),
        version: manifest.version.clone(),
        min_kibitzer_version: manifest.min_kibitzer_version.clone(),
        sha256: actual_sha256,
        binary_path: binary_path.clone(),
        severity: manifest.severity,
        scope: manifest.scope.clone(),
        triggers: manifest.triggers.clone(),
        output_format: manifest.output_format,
    });
    Registry::save(&registry_path, &registry)?;

    println!(
        "[kibitzer] installed {name} v{} -> {}",
        manifest.version,
        binary_path.display()
    );
    println!("[kibitzer] {name} now runs as a check in every repo on this machine.");
    println!(
        "  - disable it for one repo: add \"disabled\": [\"{name}\"] to that repo's .claude/inspect.json"
    );
    println!("  - remove it everywhere:    kibitzer plugin remove {name}");

    Ok(ExitCode::SUCCESS)
}

/// Backs `kibitzer plugin list` (Task 3.3.1a).
pub fn list_plugins() -> Result<Vec<InstalledPlugin>> {
    Ok(Registry::load(&default_registry_path()).plugins)
}

/// The structured result of `kibitzer plugin status <name>` (Task 3.3.1b): whether the
/// registered plugin's binary still exists, still hashes to what was verified at install
/// time, and is still compatible with the currently-running kibitzer.
#[derive(Debug)]
pub struct PluginStatusReport {
    pub installed: InstalledPlugin,
    pub binary_present: bool,
    pub hash_matches: bool,
    pub version_compatible: bool,
}

/// Backs `kibitzer plugin status <name>` (Task 3.3.1b). Errors with `"no plugin named
/// '<name>' installed"` (matching `checker::lookup`'s phrasing, `src/main.rs:232`) if
/// `name` isn't registered.
pub fn plugin_status(name: &PluginName) -> Result<PluginStatusReport> {
    let registry = Registry::load(&default_registry_path());
    let installed = registry
        .plugins
        .into_iter()
        .find(|p| &p.name == name)
        .ok_or_else(|| anyhow::anyhow!("no plugin named '{name}' installed"))?;

    let binary_present = installed.binary_path.exists();
    let hash_matches = binary_present
        && fs::read(&installed.binary_path)
            .map(|bytes| {
                let mut hasher = Sha256::new();
                hasher.update(&bytes);
                hex_encode(hasher.finalize().as_ref()).eq_ignore_ascii_case(&installed.sha256)
            })
            .unwrap_or(false);
    let version_compatible =
        version_meets_minimum(&installed.min_kibitzer_version, env!("CARGO_PKG_VERSION"))
            .unwrap_or(false);

    Ok(PluginStatusReport {
        installed,
        binary_present,
        hash_matches,
        version_compatible,
    })
}

/// The uninstall lifecycle (Task 3.3.2a/b): refuses (unless `force`) when the current
/// repo's *local* `.claude/inspect.json` (`config::find_config`, not
/// `find_effective_config` — which would already include the plugin's own synthesized
/// `Check`) still references `name` by a hand-authored check entry; otherwise (or with
/// `force`) deletes the plugin's binary directory and its `Registry` entry.
pub fn remove_plugin(name: &PluginName, force: bool) -> Result<ExitCode> {
    if !force
        && let Some((local_config, _dir)) = config::find_config(&std::env::current_dir()?)?
        && let Some(check) = local_config.checks.iter().find(|c| c.name == name.as_ref())
    {
        anyhow::bail!(
            "'{name}' is referenced by check '{}' in .claude/inspect.json\nremove that check entry first, or rerun with --force (the check will then report\nplugin-not-installed instead of running — see `kibitzer plugin status`).",
            check.name
        );
    }

    let plugin_dir = default_plugin_dir().join(name.as_ref());
    match fs::remove_dir_all(&plugin_dir) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e).with_context(|| format!("removing {}", plugin_dir.display())),
    }

    let registry_path = default_registry_path();
    let mut registry = Registry::load(&registry_path);
    registry.plugins.retain(|p| &p.name != name);
    Registry::save(&registry_path, &registry)?;

    if force {
        println!("[kibitzer] removed {name} (--force: .claude/inspect.json still references it)");
    } else {
        println!("[kibitzer] removed {name}");
    }

    Ok(ExitCode::SUCCESS)
}

/// Guards mutation of the shared `XDG_DATA_HOME` process env var so parallel `#[test]`
/// threads touching it don't race each other (no set/remove-var helper exists elsewhere
/// in this codebase to reuse — grepped `cache.rs`/`daemon.rs` first). `pub(crate)` (not
/// `mod tests`-private) so `config.rs`'s own `#[cfg(test)]` module — which also points
/// `default_registry_path()` at a controlled fixture via `XDG_DATA_HOME` — serializes
/// against these tests instead of racing them.
#[cfg(test)]
pub(crate) static XDG_DATA_HOME_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The `XDG_DATA_HOME`-mutating test helper, previously reimplemented independently (with
/// a real mutex-poisoning bug in at least one copy) in `plugin.rs`, `config.rs`, and
/// `check.rs`'s own `#[cfg(test)]` modules — now the one shared copy all three import.
/// `mcp.rs`'s async plugin-missing tests hold their own RAII guard instead (their setup
/// needs to outlive multiple `.await` points across a test body, which this
/// synchronous-closure shape can't express) but acquire the same `XDG_DATA_HOME_LOCK`
/// with the same poison-recovery fix applied directly at that call site.
#[cfg(test)]
pub(crate) mod test_support {
    use super::XDG_DATA_HOME_LOCK;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    fn unique_temp_dir(name: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        std::env::temp_dir().join(format!(
            "kibitzer-xdg-data-home-test-{name}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ))
    }

    /// Points `default_plugin_dir()`/`default_registry_path()` at a private, unique temp
    /// directory named after `name` for the duration of `f`, serialized against every
    /// other test (in any module, via `super::with_xdg_data_home`'s callers or this one)
    /// that touches `XDG_DATA_HOME` via `XDG_DATA_HOME_LOCK`. Restores the previous env
    /// var value and removes the temp directory even if `f` panics.
    ///
    /// Two poisoning-safety details, both required together:
    /// - `XDG_DATA_HOME_LOCK.lock()` recovers from a poisoned mutex instead of
    ///   propagating it (`unwrap_or_else(|e| e.into_inner())`, not `.unwrap()`) — a
    ///   single test panic while holding this lock must not cascade into every OTHER
    ///   test in the process that acquires it afterward.
    /// - The lock `guard` is dropped BEFORE `resume_unwind` re-raises a caught panic.
    ///   The previous, buggy shape kept `_guard` alive across `resume_unwind`, which
    ///   starts a fresh unwind through this frame — unwinding past a still-held
    ///   `MutexGuard` poisons the mutex (root cause of the bug this consolidation
    ///   fixes; the `unwrap_or_else` above is defense in depth, not a substitute).
    pub(crate) fn with_xdg_data_home<R>(name: &str, f: impl FnOnce(&Path) -> R) -> R {
        let guard = XDG_DATA_HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = unique_temp_dir(name);
        std::fs::create_dir_all(&dir).unwrap();
        let previous = std::env::var("XDG_DATA_HOME").ok();
        // SAFETY: `guard` serializes every test in the process that touches this var.
        unsafe {
            std::env::set_var("XDG_DATA_HOME", &dir);
        }

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(&dir)));

        unsafe {
            match previous {
                Some(value) => std::env::set_var("XDG_DATA_HOME", value),
                None => std::env::remove_var("XDG_DATA_HOME"),
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
        drop(guard);

        result.unwrap_or_else(|e| std::panic::resume_unwind(e))
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::with_xdg_data_home;
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn unique_temp_path(name: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let unique = format!(
            "{}-{name}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        std::env::temp_dir().join(format!("kibitzer-plugin-test-{unique}"))
    }

    #[test]
    fn plugin_manifest_deserializes_all_fields_from_json() {
        let json = r#"{"name":"kibitzer-stub-plugin","version":"0.1.0","min_kibitzer_version":"0.1.13","severity":"advisory","scope":["**/*"],"triggers":["batch"],"output_format":"sarif","targets":{"x86_64-unknown-linux-gnu":{"url":"file:///tmp/fixtures/kibitzer-stub-plugin-x86_64-unknown-linux-gnu","sha256":"a3f5646464646464646464646464646464646464646464646464646464"}}}"#;
        let manifest: PluginManifest = serde_json::from_str(json).unwrap();
        assert_eq!(
            manifest.targets["x86_64-unknown-linux-gnu"].sha256,
            "a3f5646464646464646464646464646464646464646464646464646464"
        );
        assert_eq!(manifest.min_kibitzer_version, "0.1.13");
        assert_eq!(manifest.name, "kibitzer-stub-plugin");
        assert_eq!(manifest.version, "0.1.0");
        assert_eq!(manifest.severity, Severity::Advisory);
        assert_eq!(manifest.scope, vec!["**/*".to_string()]);
        assert_eq!(manifest.triggers, vec!["batch".to_string()]);
        assert_eq!(manifest.output_format, OutputFormat::Sarif);
    }

    #[test]
    fn default_plugin_dir_respects_xdg_data_home_override() {
        // `.unwrap_or_else(|e| e.into_inner())`, not `.unwrap()`: an unrelated test
        // panicking while holding this lock must not poison it for every other test.
        let _guard = XDG_DATA_HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let previous = std::env::var("XDG_DATA_HOME").ok();
        // SAFETY: `XDG_DATA_HOME_LOCK` serializes every test in this module that
        // touches this env var, so no other thread observes it mid-mutation.
        unsafe {
            std::env::set_var("XDG_DATA_HOME", "/tmp/xdgdata");
        }

        let dir = default_plugin_dir();

        unsafe {
            match previous {
                Some(value) => std::env::set_var("XDG_DATA_HOME", value),
                None => std::env::remove_var("XDG_DATA_HOME"),
            }
        }

        assert_eq!(dir, PathBuf::from("/tmp/xdgdata/kibitzer/plugins"));
    }

    #[test]
    fn plugin_name_parse_rejects_path_traversal_and_slash_and_empty_strings() {
        for invalid in ["../../etc", "foo/bar", ""] {
            let err = PluginName::parse(invalid).unwrap_err();
            assert!(err.to_string().contains("invalid plugin name"));
        }
    }

    #[test]
    fn plugin_name_parse_accepts_valid_identifier() {
        let name = PluginName::parse("kibitzer-stub-plugin").unwrap();
        assert_eq!(name.as_ref(), "kibitzer-stub-plugin");
    }

    /// The regression this guards: `#[serde(transparent)]` on a tuple-struct `PluginName`
    /// deserializes straight from the wire string, bypassing `PluginName::parse`
    /// entirely — so a `registry.json` (or any other JSON source) naming
    /// `"../../etc"` would deserialize successfully despite `parse` rejecting it.
    /// `#[serde(try_from = "String")]` must route every deserialization through `parse`.
    #[test]
    fn plugin_name_deserialize_rejects_path_traversal_like_parse_does() {
        let err = serde_json::from_str::<PluginName>(r#""../../etc""#).unwrap_err();
        assert!(err.to_string().contains("invalid plugin name"));
    }

    /// Same regression, exercised the way it actually manifests: a `registry.json`-shaped
    /// document (an `InstalledPlugin` list) with one entry's `name` an invalid string —
    /// `Registry::load`'s `serde_json::from_str` must fail on it, not silently construct
    /// an unvalidated `PluginName`.
    #[test]
    fn registry_deserialize_rejects_invalid_plugin_name_in_installed_plugin() {
        let json = r#"{"plugins":[{"name":"../../etc","version":"0.1.0","min_kibitzer_version":"0.1.0","sha256":"deadbeef","binary_path":"/tmp/x","severity":"advisory","scope":[],"triggers":[],"output_format":"sarif"}]}"#;
        assert!(serde_json::from_str::<Registry>(json).is_err());
    }

    #[test]
    fn registry_save_then_load_round_trips_installed_plugin() {
        let path = unique_temp_path("registry-roundtrip");
        let registry = Registry {
            plugins: vec![InstalledPlugin {
                name: PluginName::parse("kibitzer-stub-plugin").unwrap(),
                version: "0.1.0".to_string(),
                min_kibitzer_version: "0.1.13".to_string(),
                sha256: "a3f5".to_string(),
                binary_path: PathBuf::from("/tmp/fixtures/kibitzer-stub-plugin"),
                severity: Severity::Advisory,
                scope: vec!["**/*".to_string()],
                triggers: vec!["batch".to_string()],
                output_format: OutputFormat::Sarif,
            }],
        };

        Registry::save(&path, &registry).unwrap();
        let loaded = Registry::load(&path);

        let _ = fs::remove_file(&path);
        assert_eq!(loaded.plugins.len(), 1);
        assert_eq!(loaded.plugins[0].name.as_ref(), "kibitzer-stub-plugin");
    }

    #[test]
    fn registry_load_returns_default_when_file_missing() {
        let path = unique_temp_path("registry-missing");
        let loaded = Registry::load(&path);
        assert!(loaded.plugins.is_empty());
    }

    #[test]
    fn registry_load_returns_default_when_file_contents_are_corrupt() {
        let path = unique_temp_path("registry-corrupt");
        fs::write(&path, "not valid json{{{").unwrap();

        let loaded = Registry::load(&path);

        let _ = fs::remove_file(&path);
        assert!(loaded.plugins.is_empty());
    }

    #[test]
    fn version_meets_minimum_returns_false_when_current_is_older_than_minimum() {
        assert!(!version_meets_minimum("0.2.0", "0.1.13").unwrap());
    }

    #[test]
    fn version_meets_minimum_returns_true_when_current_meets_minimum() {
        assert!(version_meets_minimum("0.1.9", "0.1.13").unwrap());
    }

    #[test]
    fn version_meets_minimum_compares_numerically_not_lexicographically() {
        assert!(version_meets_minimum("0.9.0", "0.10.0").unwrap());
    }

    #[test]
    fn parse_plain_version_rejects_non_numeric_version_string() {
        for invalid in ["not-a-version", "1.2", "0.2.0-rc1"] {
            assert!(parse_plain_version(invalid).is_err());
        }
    }

    fn sample_manifest_json(name: &str, version: &str, min_kibitzer_version: &str) -> String {
        format!(
            r#"{{"name":"{name}","version":"{version}","min_kibitzer_version":"{min_kibitzer_version}","severity":"advisory","scope":["**/*"],"triggers":["batch"],"output_format":"sarif","targets":{{}}}}"#
        )
    }

    #[test]
    fn fetch_manifest_reads_local_path_without_network_call() {
        let path = unique_temp_path("manifest-local");
        fs::write(
            &path,
            sample_manifest_json("kibitzer-stub-plugin", "0.1.0", "0.1.13"),
        )
        .unwrap();

        let manifest = fetch_manifest(path.to_str().unwrap()).unwrap();

        let _ = fs::remove_file(&path);
        assert_eq!(manifest.name, "kibitzer-stub-plugin");
        assert_eq!(manifest.version, "0.1.0");
    }

    #[test]
    fn fetch_manifest_rejects_disallowed_https_host_before_request() {
        let err = fetch_manifest("https://evil.example.com/manifest.json").unwrap_err();
        assert!(err.to_string().contains("not an allowed host"));
    }

    #[test]
    fn fetch_manifest_rejects_plain_http_url_before_any_request() {
        let err = fetch_manifest("http://github.com/manifest.json").unwrap_err();
        assert!(
            err.to_string()
                .contains("refusing to fetch over plain HTTP")
        );
    }

    #[test]
    fn is_allowed_host_accepts_the_documented_positive_cases() {
        assert!(is_allowed_host("github.com"));
        assert!(is_allowed_host("api.github.com"));
        assert!(is_allowed_host("objects.githubusercontent.com"));
    }

    /// Pure-logic coverage for the redirect-target resolution `fetch_source_bytes`'s
    /// manual redirect loop relies on (no network access, no mock server needed) — the
    /// three `Location` header shapes a real server can send.
    #[test]
    fn resolve_redirect_uri_handles_absolute_root_relative_and_path_relative_locations() {
        let base: ureq::http::Uri = "https://github.com/foo/bar/releases/download/v1/x.tar.gz"
            .parse()
            .unwrap();

        let absolute =
            resolve_redirect_uri(&base, "https://objects.githubusercontent.com/blob123").unwrap();
        assert_eq!(absolute.host(), Some("objects.githubusercontent.com"));

        let root_relative = resolve_redirect_uri(&base, "/other/path").unwrap();
        assert_eq!(root_relative.host(), Some("github.com"));
        assert_eq!(root_relative.path(), "/other/path");

        let path_relative = resolve_redirect_uri(&base, "sibling.tar.gz").unwrap();
        assert_eq!(path_relative.host(), Some("github.com"));
        assert_eq!(
            path_relative.path(),
            "/foo/bar/releases/download/v1/sibling.tar.gz"
        );
    }

    #[test]
    fn current_target_triple_resolves_known_os_arch_pair() {
        assert_eq!(
            target_triple_for("aarch64", "macos").unwrap(),
            "aarch64-apple-darwin"
        );
        assert_eq!(
            target_triple_for("aarch64", "linux").unwrap(),
            "aarch64-unknown-linux-gnu"
        );
        assert_eq!(
            target_triple_for("x86_64", "macos").unwrap(),
            "x86_64-apple-darwin"
        );
        assert_eq!(
            target_triple_for("x86_64", "linux").unwrap(),
            "x86_64-unknown-linux-gnu"
        );
    }

    #[test]
    fn current_target_triple_errors_on_unsupported_platform() {
        let err = target_triple_for("riscv64", "freebsd").unwrap_err();
        assert!(err.to_string().contains("unsupported platform"));
    }

    #[test]
    fn fetch_target_bytes_strips_file_url_prefix_before_reading_local_path() {
        let path = unique_temp_path("target-bytes-fixture");
        fs::write(&path, b"stub plugin binary bytes").unwrap();
        let target = PluginTarget {
            url: format!("file://{}", path.display()),
            sha256: "unused".to_string(),
        };

        let bytes = fetch_target_bytes(&target, true).unwrap();

        let _ = fs::remove_file(&path);
        assert_eq!(bytes, b"stub plugin binary bytes");
    }

    #[test]
    fn fetch_target_bytes_rejects_local_path_target_when_manifest_was_remote() {
        let path = unique_temp_path("target-bytes-remote-manifest-fixture");
        fs::write(&path, b"secret local file contents").unwrap();
        let target = PluginTarget {
            url: format!("file://{}", path.display()),
            sha256: "unused".to_string(),
        };

        let err = fetch_target_bytes(&target, false).unwrap_err();

        let _ = fs::remove_file(&path);
        assert!(err.to_string().contains("must be an https:// URL"));
    }

    fn sample_installed_plugin(name: &str, version: &str) -> InstalledPlugin {
        InstalledPlugin {
            name: PluginName::parse(name).unwrap(),
            version: version.to_string(),
            min_kibitzer_version: "0.1.0".to_string(),
            sha256: "deadbeef".to_string(),
            binary_path: PathBuf::from(format!("/tmp/fixtures/{name}")),
            severity: Severity::Advisory,
            scope: vec!["**/*".to_string()],
            triggers: vec!["batch".to_string()],
            output_format: OutputFormat::Sarif,
        }
    }

    #[test]
    fn install_gate_short_circuits_on_already_installed_same_version() {
        let existing = sample_installed_plugin("kibitzer-stub-plugin", "0.1.0");

        assert_eq!(
            decide_install_action(Some(&existing), "0.1.0", false),
            InstallDecision::AlreadyInstalledNoOp
        );
        assert_eq!(
            decide_install_action(Some(&existing), "0.1.0", true),
            InstallDecision::ForceReinstallSameVersion
        );
        assert_eq!(
            decide_install_action(Some(&existing), "0.2.0", false),
            InstallDecision::Upgrade {
                old_version: "0.1.0".to_string()
            }
        );
        assert_eq!(
            decide_install_action(None, "0.1.0", false),
            InstallDecision::Proceed
        );
    }

    #[test]
    fn list_plugins_returns_registry_entries_in_order() {
        with_xdg_data_home("list-plugins", |_dir| {
            let registry = Registry {
                plugins: vec![
                    sample_installed_plugin("plugin-a", "1.0.0"),
                    sample_installed_plugin("plugin-b", "2.0.0"),
                ],
            };
            Registry::save(&default_registry_path(), &registry).unwrap();

            let plugins = list_plugins().unwrap();

            assert_eq!(plugins.len(), 2);
            assert_eq!(plugins[0].name.as_ref(), "plugin-a");
            assert_eq!(plugins[1].name.as_ref(), "plugin-b");
        });
    }

    #[test]
    fn registered_plugin_checks_synthesizes_one_check_per_installed_plugin() {
        with_xdg_data_home("registered-plugin-checks", |_dir| {
            let plugin = sample_installed_plugin("kibitzer-stub-plugin", "0.1.0");
            Registry::save(
                &default_registry_path(),
                &Registry {
                    plugins: vec![plugin.clone()],
                },
            )
            .unwrap();

            let checks = registered_plugin_checks();

            assert_eq!(checks.len(), 1);
            assert_eq!(checks[0].name, "kibitzer-stub-plugin");
            assert_eq!(
                checks[0].command.as_deref(),
                Some(format!("{} {{file}}", plugin.binary_path.display()).as_str())
            );
            assert_eq!(checks[0].severity, plugin.severity);
            assert_eq!(checks[0].scope, plugin.scope);
            assert_eq!(checks[0].triggers, plugin.triggers);
            assert_eq!(checks[0].output_format, Some(plugin.output_format));
        });
    }

    #[test]
    fn registered_plugin_checks_is_empty_when_no_plugins_registered() {
        with_xdg_data_home("registered-plugin-checks-empty", |_dir| {
            assert!(registered_plugin_checks().is_empty());
        });
    }

    // `Registry::missing_binary_for` is a pure in-memory lookup (no `registry.json`
    // load) — these exercise it directly instead of round-tripping through disk/
    // `XDG_DATA_HOME`, which the free-function version this replaced required.
    #[test]
    fn missing_binary_for_returns_path_when_plugin_binary_absent() {
        let dir = unique_temp_path("missing-binary-present");
        let mut plugin = sample_installed_plugin("kibitzer-stub-plugin", "0.1.0");
        plugin.binary_path = dir.join("this-binary-does-not-exist");
        let registry = Registry {
            plugins: vec![plugin.clone()],
        };

        let result = registry.missing_binary_for("kibitzer-stub-plugin");

        assert_eq!(result, Some(plugin.binary_path));
    }

    #[test]
    fn missing_binary_for_returns_none_for_non_plugin_check_name() {
        let registry = Registry::default();
        assert_eq!(registry.missing_binary_for("not-a-registered-plugin"), None);
    }

    #[test]
    fn missing_binary_for_returns_none_when_plugin_binary_present() {
        let binary_path = unique_temp_path("missing-binary-present-ok-binary");
        fs::write(&binary_path, b"binary").unwrap();
        let mut plugin = sample_installed_plugin("kibitzer-stub-plugin", "0.1.0");
        plugin.binary_path = binary_path.clone();
        let registry = Registry {
            plugins: vec![plugin],
        };

        assert_eq!(registry.missing_binary_for("kibitzer-stub-plugin"), None);

        let _ = fs::remove_file(&binary_path);
    }

    #[test]
    fn plugin_status_errors_with_no_plugin_named_when_name_not_registered() {
        with_xdg_data_home("status-missing-name", |_dir| {
            let err = plugin_status(&PluginName::parse("does-not-exist").unwrap()).unwrap_err();
            assert!(
                err.to_string()
                    .contains("no plugin named 'does-not-exist' installed")
            );
        });
    }

    #[test]
    fn remove_plugin_deletes_registry_entry_and_binary_directory_when_unreferenced() {
        with_xdg_data_home("remove-unreferenced", |_dir| {
            let name = PluginName::parse("kibitzer-plugin-rm-test").unwrap();
            let plugin_dir = default_plugin_dir().join(name.as_ref());
            fs::create_dir_all(&plugin_dir).unwrap();
            fs::write(plugin_dir.join(name.as_ref()), b"binary").unwrap();
            let mut plugin = sample_installed_plugin(name.as_ref(), "0.1.0");
            plugin.binary_path = plugin_dir.join(name.as_ref());
            Registry::save(
                &default_registry_path(),
                &Registry {
                    plugins: vec![plugin],
                },
            )
            .unwrap();

            let exit = remove_plugin(&name, false).unwrap();

            assert_eq!(exit, ExitCode::SUCCESS);
            assert!(!plugin_dir.exists());
            assert!(Registry::load(&default_registry_path()).plugins.is_empty());
        });
    }
}
