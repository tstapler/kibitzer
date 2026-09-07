//! Exercises `kibitzer plugin install|list|remove|status` as a real subprocess (not by
//! calling `src/plugin.rs` functions directly), the same way `tests/hook_contract.rs`
//! exercises `kibitzer hook`. Isolates the plugin registry/binary directory via a private
//! `XDG_DATA_HOME` so a test run can't collide with a real install on the machine running
//! the tests, or with a concurrently-running test.
//!
//! Story 3.2's fixtures are a local file (not a real GitHub release) — `fetch_manifest`/
//! `fetch_target_bytes` treat any `source`/`url` that doesn't parse as `http(s)://` as a
//! local path, so a plain temp file plus a hand-built manifest JSON exercises the exact
//! same downstream verify/place/register logic a real HTTPS install would.

use serde_json::json;
use sha2::{Digest, Sha256};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[cfg(all(target_arch = "aarch64", target_os = "macos"))]
const CURRENT_TARGET_TRIPLE: &str = "aarch64-apple-darwin";
#[cfg(all(target_arch = "aarch64", target_os = "linux"))]
const CURRENT_TARGET_TRIPLE: &str = "aarch64-unknown-linux-gnu";
#[cfg(all(target_arch = "x86_64", target_os = "macos"))]
const CURRENT_TARGET_TRIPLE: &str = "x86_64-apple-darwin";
#[cfg(all(target_arch = "x86_64", target_os = "linux"))]
const CURRENT_TARGET_TRIPLE: &str = "x86_64-unknown-linux-gnu";

/// Locates a sibling workspace package's compiled binary (`kibitzer-stub-plugin`,
/// `kibitzer-stub-plugin-second`) so these tests can exercise the *real* compiled crate
/// instead of a hand-rolled shell-script stand-in (Story 5.1.1/5.1.2).
///
/// `env!("CARGO_BIN_EXE_<name>")` only works for a binary target of the *same* package as
/// the test (`kibitzer`'s own `CARGO_BIN_EXE_kibitzer`, in `TempRepo::run` below) —
/// Cargo does not auto-expose it for a dependency's binaries on stable: that requires an
/// "artifact dependency" (`artifact = "bin"`), which needs the nightly-only `-Z bindeps`
/// (confirmed against cargo 1.98: a plain path `[dev-dependencies]` entry — what
/// `Cargo.toml` has — links only the sibling's *library* crate, and `artifact = "bin"`
/// itself fails to parse without `-Z bindeps`). So this resolves the binary at runtime
/// instead, next to `kibitzer`'s own binary (same target dir, same profile) — relying on
/// `cargo test`/`cargo llvm-cov`'s workspace-wide default build (no `default-members`
/// override in `Cargo.toml`, and CI's `cargo llvm-cov --workspace`) to have already built
/// every workspace member's binaries before any test runs.
fn sibling_workspace_binary(name: &str) -> PathBuf {
    let kibitzer_bin = PathBuf::from(env!("CARGO_BIN_EXE_kibitzer"));
    let path = kibitzer_bin
        .parent()
        .expect("CARGO_BIN_EXE_kibitzer has a parent dir")
        .join(name);
    assert!(
        path.exists(),
        "{} not found at {} — run `cargo build --workspace` first \
         (a lone `cargo test -p kibitzer` won't build workspace siblings)",
        name,
        path.display()
    );
    path
}

struct TempRepo {
    dir: PathBuf,
    data_dir: PathBuf,
    cache_dir: PathBuf,
}

impl TempRepo {
    fn new(name: &str) -> Self {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let unique = format!(
            "{}-{name}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );
        let dir = std::env::temp_dir().join(format!("kibitzer-plugin-contract-{unique}"));
        let data_dir = std::env::temp_dir().join(format!("kibitzer-plugin-contract-data-{unique}"));
        let cache_dir =
            std::env::temp_dir().join(format!("kibitzer-plugin-contract-cache-{unique}"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(&data_dir).unwrap();
        std::fs::create_dir_all(&cache_dir).unwrap();
        Self {
            dir,
            data_dir,
            cache_dir,
        }
    }

    fn path(&self, rel_path: &str) -> PathBuf {
        self.dir.join(rel_path)
    }

    fn write_inspect_json(&self, value: serde_json::Value) {
        std::fs::create_dir_all(self.dir.join(".claude")).unwrap();
        std::fs::write(
            self.dir.join(".claude").join("inspect.json"),
            serde_json::to_string(&value).unwrap(),
        )
        .unwrap();
    }

    fn run(&self, args: &[&str]) -> (i32, String, String) {
        let output = Command::new(env!("CARGO_BIN_EXE_kibitzer"))
            .args(args)
            .current_dir(&self.dir)
            .env("XDG_DATA_HOME", &self.data_dir)
            .output()
            .expect("spawn kibitzer");
        (
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    }

    fn plugin_dir(&self, name: &str) -> PathBuf {
        self.data_dir.join("kibitzer").join("plugins").join(name)
    }

    fn registry_path(&self) -> PathBuf {
        self.data_dir
            .join("kibitzer")
            .join("plugins")
            .join("registry.json")
    }

    /// Number of entries in the registry, or 0 if it doesn't exist yet — mirrors
    /// `Registry::load`'s missing-file default without depending on `src/plugin.rs`
    /// (this crate has no `lib.rs`, so integration tests can't call it directly).
    fn registry_plugin_count(&self) -> usize {
        let raw = match std::fs::read_to_string(self.registry_path()) {
            Ok(raw) => raw,
            Err(_) => return 0,
        };
        let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap();
        parsed["plugins"].as_array().map(|a| a.len()).unwrap_or(0)
    }

    /// Builds a manifest fixture (`manifest.json` in the repo dir) plus its target
    /// binary fixture (`<name>-binary` in the repo dir), both local files. `sha256`
    /// lets a test intentionally lie about the digest (checksum-mismatch case).
    fn write_manifest_fixture(
        &self,
        name: &str,
        version: &str,
        min_kibitzer_version: &str,
        binary_contents: &[u8],
        sha256: &str,
    ) -> PathBuf {
        let binary_path = self.path(&format!("{name}-fixture-binary"));
        std::fs::write(&binary_path, binary_contents).unwrap();

        let manifest = json!({
            "name": name,
            "version": version,
            "min_kibitzer_version": min_kibitzer_version,
            "severity": "advisory",
            "scope": ["**/*"],
            "triggers": ["batch"],
            "output_format": "sarif",
            "targets": {
                CURRENT_TARGET_TRIPLE: {
                    "url": format!("file://{}", binary_path.display()),
                    "sha256": sha256,
                }
            }
        });
        let manifest_path = self.path("manifest.json");
        std::fs::write(&manifest_path, serde_json::to_string(&manifest).unwrap()).unwrap();
        manifest_path
    }

    /// Drives `kibitzer mcp` as a real subprocess through the actual MCP stdio protocol
    /// (newline-delimited JSON-RPC 2.0 — `rmcp`'s `transport-io`, confirmed against this
    /// binary manually): an `initialize` handshake, `notifications/initialized`, then one
    /// `tools/call` for `run_checks`. Deliberately NOT going through `kibitzer hook` (which
    /// would prefer a live `kibitzer daemon` if one happens to be running on the host — a
    /// daemon started earlier resolves `XDG_DATA_HOME`/`XDG_CACHE_HOME` from its own
    /// environment at start time, not this test's overrides, silently bypassing the
    /// isolation below). `mcp.rs::run_checks` calls `find_effective_config` directly with
    /// no daemon involved, so this is both the more faithful "over MCP" exercise this
    /// test's name promises and the one immune to that host-state leak.
    fn call_run_checks_tool_over_mcp(&self, file_path: &Path, trigger: &str) -> String {
        let request = format!(
            "{}\n{}\n{}\n",
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": { "name": "plugin-contract-test", "version": "0.0.1" }
                }
            }),
            json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": {
                    "name": "run_checks",
                    "arguments": { "file_path": file_path.display().to_string(), "trigger": trigger }
                }
            }),
        );

        let mut child = Command::new(env!("CARGO_BIN_EXE_kibitzer"))
            .arg("mcp")
            .current_dir(&self.dir)
            .env("XDG_DATA_HOME", &self.data_dir)
            .env("XDG_CACHE_HOME", &self.cache_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn kibitzer mcp");
        // Deliberately keep `stdin` open (not `.take()`+drop) until both responses are
        // read: closing it early signals EOF, and the server can race its own shutdown
        // against still-in-flight `tools/call` processing, silently dropping the second
        // response — confirmed by reproducing that exact race against this binary.
        let mut stdin = child.stdin.take().unwrap();
        stdin.write_all(request.as_bytes()).unwrap();

        let mut reader = std::io::BufReader::new(child.stdout.take().unwrap());
        let mut initialize_response = String::new();
        let mut tool_call_response = String::new();
        std::io::BufRead::read_line(&mut reader, &mut initialize_response).unwrap();
        std::io::BufRead::read_line(&mut reader, &mut tool_call_response).unwrap();

        drop(stdin); // now signal EOF so the server exits.
        let _ = child.wait();

        let response: serde_json::Value = tool_call_response.parse().unwrap_or_else(|e| {
            panic!("tools/call response line wasn't valid JSON ({e}): {tool_call_response}")
        });
        response["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_else(|| panic!("no result.content[0].text in response: {response}"))
            .to_string()
    }

    /// Runs a real `kibitzer plugin install` against a correctly-checksummed fixture for
    /// `name`/`version`, asserting it succeeds — the shared setup step for tests that need
    /// an already-installed plugin (status, remove).
    fn install_valid_fixture(&self, name: &str, version: &str) {
        let contents = format!("stub plugin binary for {name} v{version}").into_bytes();
        let sha256 = sha256_hex(&contents);
        let manifest_path = self.write_manifest_fixture(name, version, "0.1.0", &contents, &sha256);
        let (code, stdout, stderr) = self.run(&[
            "plugin",
            "install",
            name,
            "--source",
            manifest_path.to_str().unwrap(),
        ]);
        assert_eq!(code, 0, "stdout: {stdout}\nstderr: {stderr}");
    }
}

impl Drop for TempRepo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
        let _ = std::fs::remove_dir_all(&self.data_dir);
        let _ = std::fs::remove_dir_all(&self.cache_dir);
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .as_slice()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn plugin_data_dir_exists(repo: &Path, data_dir: &Path) -> bool {
    let _ = repo;
    data_dir.join("kibitzer").join("plugins").exists()
}

/// Runs a binary directly (not through `kibitzer run`) and returns its stdout, asserting
/// it exited successfully — used to confirm an installed plugin binary really is the real
/// compiled crate (it emits the expected canned SARIF), independent of whatever `kibitzer
/// run`'s exit-status-gated rendering does or doesn't surface for that same invocation.
/// Spawns a stub-plugin binary directly and returns its stdout. Deliberately does not
/// assert on exit status: both real stub binaries exit `1` after printing their SARIF
/// finding (see `crates/kibitzer-stub-plugin/src/main.rs`'s doc comment) — that's the point
/// being demonstrated elsewhere, not a spawn failure. A genuine spawn failure (binary
/// missing, not executable) still panics via `unwrap_or_else` below.
fn run_binary_directly(path: &Path, file_arg: &str) -> String {
    let output = Command::new(path)
        .arg(file_arg)
        .output()
        .unwrap_or_else(|e| panic!("spawn {} directly: {e}", path.display()));
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn plugin_remove_rejects_path_traversal_name_via_clap_before_any_filesystem_op() {
    let repo = TempRepo::new("remove-path-traversal");

    let (code, _stdout, stderr) = repo.run(&["plugin", "remove", "../../etc"]);

    assert_ne!(code, 0);
    assert!(
        stderr.to_lowercase().contains("invalid"),
        "stderr: {stderr}"
    );
    assert!(!plugin_data_dir_exists(&repo.dir, &repo.data_dir));
}

#[test]
fn plugin_list_prints_no_plugins_installed_message_when_registry_empty() {
    let repo = TempRepo::new("list-empty");

    let (code, stdout, stderr) = repo.run(&["plugin", "list"]);

    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout.trim_end(), "[kibitzer] no plugins installed");
}

#[test]
fn plugin_remove_rejects_invalid_name_before_dispatch_reaches_remove_plugin() {
    let repo = TempRepo::new("remove-invalid-name");

    let (code, _stdout, stderr) = repo.run(&["plugin", "remove", "foo/bar"]);

    assert_ne!(code, 0);
    assert!(
        stderr.to_lowercase().contains("invalid"),
        "stderr: {stderr}"
    );
    assert!(!plugin_data_dir_exists(&repo.dir, &repo.data_dir));
}

#[test]
fn install_plugin_rejects_checksum_mismatch_and_leaves_no_trace() {
    let repo = TempRepo::new("install-checksum-mismatch");
    let contents = b"real plugin bytes".to_vec();
    let wrong_sha256 = "0".repeat(64);
    let manifest_path = repo.write_manifest_fixture(
        "kibitzer-stub-plugin",
        "0.1.0",
        "0.1.0",
        &contents,
        &wrong_sha256,
    );

    let (code, stdout, stderr) = repo.run(&[
        "plugin",
        "install",
        "kibitzer-stub-plugin",
        "--source",
        manifest_path.to_str().unwrap(),
    ]);

    assert_ne!(code, 0);
    assert!(
        format!("{stdout}{stderr}").contains("checksum mismatch"),
        "stdout: {stdout}\nstderr: {stderr}"
    );
    assert!(!repo.plugin_dir("kibitzer-stub-plugin").exists());
    assert_eq!(repo.registry_plugin_count(), 0);
}

#[test]
fn install_plugin_rejects_incompatible_min_kibitzer_version_before_download() {
    let repo = TempRepo::new("install-incompatible-version");
    let contents = b"real plugin bytes".to_vec();
    let sha256 = sha256_hex(&contents);
    let manifest_path = repo.write_manifest_fixture(
        "kibitzer-stub-plugin",
        "0.1.0",
        "99.0.0",
        &contents,
        &sha256,
    );

    let (code, stdout, stderr) = repo.run(&[
        "plugin",
        "install",
        "kibitzer-stub-plugin",
        "--source",
        manifest_path.to_str().unwrap(),
    ]);

    assert_ne!(code, 0);
    assert!(
        format!("{stdout}{stderr}").contains("requires kibitzer"),
        "stdout: {stdout}\nstderr: {stderr}"
    );
    assert!(!repo.plugin_dir("kibitzer-stub-plugin").exists());
    assert_eq!(repo.registry_plugin_count(), 0);
}

#[test]
fn install_plugin_is_idempotent_noop_when_already_installed_at_same_version() {
    let repo = TempRepo::new("install-idempotent");
    repo.install_valid_fixture("kibitzer-stub-plugin", "0.1.0");
    assert_eq!(repo.registry_plugin_count(), 1);

    let contents = b"stub plugin binary for kibitzer-stub-plugin v0.1.0".to_vec();
    let sha256 = sha256_hex(&contents);
    let manifest_path =
        repo.write_manifest_fixture("kibitzer-stub-plugin", "0.1.0", "0.1.0", &contents, &sha256);

    let (code, stdout, stderr) = repo.run(&[
        "plugin",
        "install",
        "kibitzer-stub-plugin",
        "--source",
        manifest_path.to_str().unwrap(),
    ]);

    assert_eq!(code, 0, "stdout: {stdout}\nstderr: {stderr}");
    assert!(stdout.contains("already installed"), "stdout: {stdout}");
    assert!(stdout.contains("no changes made"), "stdout: {stdout}");
    assert_eq!(repo.registry_plugin_count(), 1);
}

/// Story 3.2.2's first acceptance criterion: a manifest whose own `name` field disagrees
/// with the CLI-supplied name is refused before any version-compat check, download, or
/// `Registry` write — otherwise `plugin install foo --source ...` against a manifest
/// describing `bar` would silently register under `foo`, discarding the manifest's own
/// claim about what it is.
#[test]
fn install_plugin_rejects_manifest_name_mismatch_before_any_registry_write() {
    let repo = TempRepo::new("install-manifest-name-mismatch");
    let contents = b"real plugin bytes".to_vec();
    let sha256 = sha256_hex(&contents);
    // Manifest describes "bar"; CLI asks to install "foo".
    let manifest_path = repo.write_manifest_fixture("bar", "0.1.0", "0.1.0", &contents, &sha256);

    let (code, stdout, stderr) = repo.run(&[
        "plugin",
        "install",
        "foo",
        "--source",
        manifest_path.to_str().unwrap(),
    ]);

    assert_ne!(code, 0);
    assert!(
        format!("{stdout}{stderr}").contains("manifest name does not match"),
        "stdout: {stdout}\nstderr: {stderr}"
    );
    assert_eq!(repo.registry_plugin_count(), 0);
    assert!(!repo.plugin_dir("foo").exists());
    assert!(!repo.plugin_dir("bar").exists());
}

/// Story 3.2.2's "upgrade" acceptance criterion: a version bump prints the exact literal
/// upgrade line (design/ux.md; locked against implementation drift) rather than silently
/// overwriting.
#[test]
fn install_plugin_prints_upgrade_line_on_version_bump() {
    let repo = TempRepo::new("install-upgrade");
    repo.install_valid_fixture("kibitzer-stub-plugin", "0.1.0");

    let upgraded_contents = b"stub plugin binary for kibitzer-stub-plugin v0.2.0".to_vec();
    let upgraded_manifest = repo.write_manifest_fixture(
        "kibitzer-stub-plugin",
        "0.2.0",
        "0.1.0",
        &upgraded_contents,
        &sha256_hex(&upgraded_contents),
    );

    let (code, stdout, stderr) = repo.run(&[
        "plugin",
        "install",
        "kibitzer-stub-plugin",
        "--source",
        upgraded_manifest.to_str().unwrap(),
    ]);

    assert_eq!(code, 0, "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        stdout.contains("upgrading kibitzer-stub-plugin v0.1.0 -> v0.2.0"),
        "stdout: {stdout}"
    );
    assert_eq!(repo.registry_plugin_count(), 1);
}

/// Story 3.2.2's "force reinstall" acceptance criterion: re-installing the same version
/// with `--force` restates the consequence rather than printing an identical unqualified
/// success line (design/ux.md's stated principle for forced actions).
#[test]
fn install_plugin_prints_force_reinstall_line_on_same_version() {
    let repo = TempRepo::new("install-force-reinstall");
    repo.install_valid_fixture("kibitzer-stub-plugin", "0.1.0");
    let contents = b"stub plugin binary for kibitzer-stub-plugin v0.1.0".to_vec();
    let manifest_path = repo.write_manifest_fixture(
        "kibitzer-stub-plugin",
        "0.1.0",
        "0.1.0",
        &contents,
        &sha256_hex(&contents),
    );

    let (code, stdout, stderr) = repo.run(&[
        "plugin",
        "install",
        "kibitzer-stub-plugin",
        "--source",
        manifest_path.to_str().unwrap(),
        "--force",
    ]);

    assert_eq!(code, 0, "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        stdout.contains(
            "kibitzer-stub-plugin v0.1.0 already installed — reinstalling due to --force"
        ),
        "stdout: {stdout}"
    );
    assert_eq!(repo.registry_plugin_count(), 1);
}

#[test]
fn plugin_status_reports_binary_missing_when_file_deleted_out_of_band() {
    let repo = TempRepo::new("status-binary-missing");
    repo.install_valid_fixture("kibitzer-stub-plugin", "0.1.0");
    let binary_path = repo
        .plugin_dir("kibitzer-stub-plugin")
        .join("kibitzer-stub-plugin");
    assert!(binary_path.exists());
    std::fs::remove_file(&binary_path).unwrap();

    let (code, stdout, stderr) = repo.run(&["plugin", "status", "kibitzer-stub-plugin"]);

    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stdout.contains("binary missing"), "stdout: {stdout}");
}

#[test]
fn plugin_status_reports_ok_when_binary_present_and_hash_matches() {
    let repo = TempRepo::new("status-ok");
    repo.install_valid_fixture("kibitzer-stub-plugin", "0.1.0");

    let (code, stdout, stderr) = repo.run(&["plugin", "status", "kibitzer-stub-plugin"]);

    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stdout.contains("ok"), "stdout: {stdout}");
}

#[test]
fn remove_plugin_refuses_when_referenced_by_local_inspect_json_check() {
    let repo = TempRepo::new("remove-referenced");
    repo.install_valid_fixture("kibitzer-stub-plugin", "0.1.0");
    repo.write_inspect_json(json!({
        "checks": [{
            "name": "kibitzer-stub-plugin",
            "command": "true",
            "severity": "advisory",
        }]
    }));

    let (code, _stdout, stderr) = repo.run(&["plugin", "remove", "kibitzer-stub-plugin"]);

    assert_ne!(code, 0);
    assert!(
        stderr.contains("is referenced by check"),
        "stderr: {stderr}"
    );
    assert_eq!(repo.registry_plugin_count(), 1);
    assert!(repo.plugin_dir("kibitzer-stub-plugin").exists());
}

/// REQ-4.1.1c / REQ-5.1.1 (Epic 4.1, Epic 5.1): proves an installed plugin's check
/// surfaces through the full CLI/config stack — `kibitzer plugin install` followed by a
/// plain `kibitzer run`, with no `.claude/inspect.json` anywhere in the repo — using the
/// real, compiled `crates/kibitzer-stub-plugin` binary (`sibling_workspace_binary`, above)
/// rather than a hand-rolled shell-script stand-in, per ADR-001's rationale for that crate
/// existing at all.
///
/// `run_check`'s SARIF branch takes pass/fail from the process's exit status, not from
/// whether the SARIF payload's `results` array is non-empty (confirmed by reading
/// `src/check.rs`), and every renderer that surfaces check output (`run_batch`'s
/// `report_lines`, `hook.rs`, `mcp.rs::run_checks`) skips a `CheckResult` entirely once
/// `passed` is true. The real stub binary exits `1` after printing its SARIF finding
/// (see `crates/kibitzer-stub-plugin/src/main.rs`'s doc comment for why this changed from
/// an earlier exit-0 design) — matching `docs/output-formats.md`'s documented convention
/// for real SARIF-emitting linters (e.g. ESLint's own nonzero exit on a lint violation) —
/// so the finding genuinely reaches `kibitzer run --trigger batch`'s stdout. The manifest's
/// `severity: "advisory"` keeps the overall batch run's exit code `0` despite this one
/// check "failing" (see `has_blocking_finding` in `src/run.rs`).
///
/// This test proves install -> register -> run end-to-end: (1) install places the exact
/// real compiled bytes at the registered path (verified against the checksum via `plugin
/// status`), (2) that installed binary is genuinely the one built from
/// `crates/kibitzer-stub-plugin` — invoking it directly still emits the real canned SARIF
/// finding — and (3) `kibitzer run` auto-injects and executes that check with no
/// `.claude/inspect.json` anywhere, surfacing its finding in normal check output exactly as
/// the feature's success metric states, without ever being reported `[skipped]` (the
/// signal a *missing* plugin binary produces instead, per
/// `run_checks_over_mcp_reports_skipped_not_blocking_when_plugin_binary_missing` below).
#[test]
fn plugin_install_register_run_end_to_end_surfaces_stub_finding() {
    let repo = TempRepo::new("install-register-run-e2e");
    let stub_plugin_bytes = std::fs::read(sibling_workspace_binary("kibitzer-stub-plugin"))
        .expect("read real stub binary");
    let sha256 = sha256_hex(&stub_plugin_bytes);
    let manifest_path = repo.write_manifest_fixture(
        "kibitzer-stub-plugin",
        "0.1.0",
        "0.1.0",
        &stub_plugin_bytes,
        &sha256,
    );

    let (code, stdout, stderr) = repo.run(&[
        "plugin",
        "install",
        "kibitzer-stub-plugin",
        "--source",
        manifest_path.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "stdout: {stdout}\nstderr: {stderr}");

    // Install placed the exact real binary bytes, checksum-verified.
    let (code, stdout, stderr) = repo.run(&["plugin", "status", "kibitzer-stub-plugin"]);
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stdout.contains("ok"), "stdout: {stdout}");

    // The installed copy really is the compiled kibitzer-stub-plugin crate: invoking it
    // directly (as `run_check` would) still emits the real canned SARIF finding.
    let installed_binary = repo
        .plugin_dir("kibitzer-stub-plugin")
        .join("kibitzer-stub-plugin");
    let direct_stdout = run_binary_directly(&installed_binary, "target.txt");
    assert!(
        direct_stdout.contains("kibitzer-stub-plugin-finding"),
        "installed binary's SARIF output: {direct_stdout}"
    );

    std::fs::write(repo.path("target.txt"), "irrelevant content").unwrap();

    // No .claude/inspect.json exists anywhere above `repo.dir` — the plugin's check must
    // still be auto-injected and actually invoked (not merely present in the registry),
    // and its finding must appear in kibitzer's normal check output — the feature's
    // stated success metric, proven literally rather than only at the registry/status
    // layer.
    let (code, stdout, stderr) = repo.run(&["run", ".", "--trigger", "batch"]);

    assert!(
        stdout.contains("kibitzer-stub-plugin-finding"),
        "expected the plugin's canned SARIF finding in stdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        !stdout.contains("[skipped]"),
        "an installed, present binary must never render as skipped: {stdout}"
    );
    // Advisory severity: the check itself "fails" (exit 1, finding shown) but that never
    // blocks the overall batch run.
    assert_eq!(code, 0, "stdout: {stdout}\nstderr: {stderr}");
}

/// Story 5.1.2: proves the plugin mechanism is a generic extension point — a second,
/// differently-named plugin installs and registers independently of the first, with no
/// name collision — rather than something special-cased to a single embedded plugin.
///
/// Both real stub binaries now exit `1` after printing (see `main.rs`'s doc comment), so —
/// unlike an earlier design where a passing check's SARIF content never reached
/// `kibitzer run`'s stdout — this test asserts distinctness at every observable layer:
/// `plugin list` names both installs, `plugin status` independently checksum-verifies
/// each one, each installed binary is a genuinely different compiled artifact (invoking
/// them directly yields two different `ruleId`s, not the same binary copied under two
/// names), and a batch run with both auto-injected surfaces BOTH canned findings distinctly
/// — no crash, no `[skipped]`, no collision where one plugin's `Check` silently overwrites
/// the other's in `registered_plugin_checks()`.
#[test]
fn two_independently_registered_plugins_remain_distinct_end_to_end() {
    let repo = TempRepo::new("two-plugins-distinct");

    let first_bytes = std::fs::read(sibling_workspace_binary("kibitzer-stub-plugin"))
        .expect("read real first stub binary");
    let first_manifest = repo.write_manifest_fixture(
        "kibitzer-stub-plugin",
        "0.1.0",
        "0.1.0",
        &first_bytes,
        &sha256_hex(&first_bytes),
    );
    let (code, stdout, stderr) = repo.run(&[
        "plugin",
        "install",
        "kibitzer-stub-plugin",
        "--source",
        first_manifest.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "stdout: {stdout}\nstderr: {stderr}");

    let second_bytes = std::fs::read(sibling_workspace_binary("kibitzer-stub-plugin-second"))
        .expect("read real second stub binary");
    let second_manifest = repo.write_manifest_fixture(
        "kibitzer-stub-plugin-second",
        "0.1.0",
        "0.1.0",
        &second_bytes,
        &sha256_hex(&second_bytes),
    );
    let (code, stdout, stderr) = repo.run(&[
        "plugin",
        "install",
        "kibitzer-stub-plugin-second",
        "--source",
        second_manifest.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "stdout: {stdout}\nstderr: {stderr}");

    let (code, stdout, stderr) = repo.run(&["plugin", "list"]);
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(
        stdout.contains("kibitzer-stub-plugin-second"),
        "stdout: {stdout}"
    );
    assert!(
        stdout
            .lines()
            .any(|l| l.contains("kibitzer-stub-plugin") && !l.contains("-second")),
        "expected a line naming the first plugin distinctly from the second: {stdout}"
    );

    for name in ["kibitzer-stub-plugin", "kibitzer-stub-plugin-second"] {
        let (code, stdout, stderr) = repo.run(&["plugin", "status", name]);
        assert_eq!(code, 0, "{name} status stderr: {stderr}");
        assert!(stdout.contains("ok"), "{name} status stdout: {stdout}");
    }

    // Each installed copy is a genuinely different compiled artifact, not the same binary
    // placed under two names.
    let first_installed = repo
        .plugin_dir("kibitzer-stub-plugin")
        .join("kibitzer-stub-plugin");
    let second_installed = repo
        .plugin_dir("kibitzer-stub-plugin-second")
        .join("kibitzer-stub-plugin-second");
    let first_direct_stdout = run_binary_directly(&first_installed, "target.txt");
    let second_direct_stdout = run_binary_directly(&second_installed, "target.txt");
    assert!(
        first_direct_stdout.contains("kibitzer-stub-plugin-finding")
            && !first_direct_stdout.contains("kibitzer-stub-plugin-second-finding"),
        "first plugin's SARIF output: {first_direct_stdout}"
    );
    assert!(
        second_direct_stdout.contains("kibitzer-stub-plugin-second-finding"),
        "second plugin's SARIF output: {second_direct_stdout}"
    );

    std::fs::write(repo.path("target.txt"), "irrelevant content").unwrap();

    // No .claude/inspect.json anywhere — both checks must be auto-injected from the
    // registry's two independent entries, both surface distinctly, and neither collides
    // with or silently replaces the other.
    let (code, stdout, stderr) = repo.run(&["run", ".", "--trigger", "batch"]);
    assert!(
        stdout.contains("kibitzer-stub-plugin-finding"),
        "expected the first plugin's finding in stdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stdout.contains("kibitzer-stub-plugin-second-finding"),
        "expected the second plugin's finding in stdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        !stdout.contains("[skipped]"),
        "both installed binaries are present and must never render as skipped: {stdout}"
    );
    assert_eq!(code, 0, "stdout: {stdout}\nstderr: {stderr}");
}

/// REQ-4.3.1g (Epic 4.3): proves the `[skipped]` plugin-missing rendering (Task 4.3.1c)
/// works against a real spawned `kibitzer mcp` process speaking the actual MCP stdio
/// protocol — not just a `CheckResult` constructed directly in a unit test — and that no
/// exit-127/"command not found" shell noise leaks through once the binary is gone.
///
/// Manifest severity is deliberately `"blocking"`, to prove the missing-binary result is
/// forced down to advisory/skipped (Task 4.3.1b) rather than merely happening to already
/// be advisory.
#[test]
fn run_checks_over_mcp_reports_skipped_not_blocking_when_plugin_binary_missing() {
    let repo = TempRepo::new("plugin-missing-skipped-rendering");
    let contents = b"stub plugin binary for kibitzer-stub-plugin v0.1.0".to_vec();
    let sha256 = sha256_hex(&contents);
    let binary_fixture_path = repo.path("kibitzer-stub-plugin-fixture-binary");
    std::fs::write(&binary_fixture_path, &contents).unwrap();
    let manifest = json!({
        "name": "kibitzer-stub-plugin",
        "version": "0.1.0",
        "min_kibitzer_version": "0.1.0",
        "severity": "blocking",
        "scope": ["**/*"],
        "triggers": ["batch"],
        "output_format": "sarif",
        "targets": {
            CURRENT_TARGET_TRIPLE: {
                "url": format!("file://{}", binary_fixture_path.display()),
                "sha256": sha256,
            }
        }
    });
    let manifest_path = repo.path("manifest.json");
    std::fs::write(&manifest_path, serde_json::to_string(&manifest).unwrap()).unwrap();

    let (code, stdout, stderr) = repo.run(&[
        "plugin",
        "install",
        "kibitzer-stub-plugin",
        "--source",
        manifest_path.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "stdout: {stdout}\nstderr: {stderr}");

    let installed_binary = repo
        .plugin_dir("kibitzer-stub-plugin")
        .join("kibitzer-stub-plugin");
    assert!(installed_binary.exists());
    std::fs::remove_file(&installed_binary).unwrap();

    std::fs::write(repo.path("target.txt"), "irrelevant content").unwrap();
    let output = repo.call_run_checks_tool_over_mcp(&repo.path("target.txt"), "batch");

    assert!(
        output.contains("[skipped] kibitzer-stub-plugin"),
        "expected a [skipped] line, got: {output}"
    );
    assert!(
        !output.contains("[Blocking]") && !output.contains("[Advisory]"),
        "must not render as a real check outcome, got: {output}"
    );
    assert!(
        !output.to_lowercase().contains("not found")
            && !output.contains("exit status: 127")
            && !output.contains("No such file or directory"),
        "must not leak raw shell/exec noise, got: {output}"
    );
}

#[test]
fn remove_plugin_with_force_deletes_binary_and_registry_entry_despite_reference() {
    let repo = TempRepo::new("remove-force");
    repo.install_valid_fixture("kibitzer-stub-plugin", "0.1.0");
    repo.write_inspect_json(json!({
        "checks": [{
            "name": "kibitzer-stub-plugin",
            "command": "true",
            "severity": "advisory",
        }]
    }));

    let (code, stdout, stderr) = repo.run(&["plugin", "remove", "kibitzer-stub-plugin", "--force"]);

    assert_eq!(code, 0, "stdout: {stdout}\nstderr: {stderr}");
    assert_eq!(repo.registry_plugin_count(), 0);
    assert!(!repo.plugin_dir("kibitzer-stub-plugin").exists());
}
