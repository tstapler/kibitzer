//! End-to-end `kibitzer mcp --socket` + `kibitzer mcp --connect`: the real binary as a shared
//! server on a Unix socket, and a second process bridging stdio MCP to it.
#![allow(non_snake_case)]
#![cfg(unix)]

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use serde_json::{Value, json};

/// Kills the process on drop so a failed assertion never leaks it.
struct Proc(Child);

impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// A private scratch dir, also usable as `XDG_RUNTIME_DIR`.
fn scratch(label: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("kbz-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    dir
}

/// `kibitzer mcp <args>` with `XDG_RUNTIME_DIR=runtime`, so the default socket path is private.
fn kibitzer_mcp(runtime: &Path, args: &[&Path]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_kibitzer"));
    command
        .arg("mcp")
        .args(args)
        .env("XDG_RUNTIME_DIR", runtime);
    command
}

fn start_server(runtime: &Path, socket: Option<&Path>) -> Proc {
    let mut args = vec![Path::new("--socket")];
    args.extend(socket);
    let mut child = kibitzer_mcp(runtime, &args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn kibitzer mcp --socket");
    let mut stderr = BufReader::new(child.stderr.take().unwrap());
    let mut line = String::new();
    stderr.read_line(&mut line).unwrap();
    assert!(line.contains("listening on unix socket"), "{line:?}");
    std::thread::spawn(move || {
        let _ = std::io::copy(&mut stderr, &mut std::io::sink());
    });
    Proc(child)
}

#[test]
fn mcp_connect_should_ListTools_Through_SharedSocketServer_AtTheDefaultPath() {
    let dir = scratch("e2e");
    // Neither process is given a path: both must resolve the same default.
    let socket = dir.join("kibitzer").join("mcp.sock");
    let mut server = start_server(&dir, None);
    assert!(socket.exists(), "server listens at the default path");

    let mut bridge = Proc(
        kibitzer_mcp(&dir, &[Path::new("--connect")])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn kibitzer mcp --connect"),
    );
    let mut stdin = bridge.0.stdin.take().unwrap();
    let mut stdout = BufReader::new(bridge.0.stdout.take().unwrap());
    let mut recv = || -> Value {
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap_or_else(|e| panic!("{e}: {line:?}"))
    };

    for message in [
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2025-03-26", "capabilities": {},
            "clientInfo": {"name": "mcp_socket test", "version": "0"}}}),
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
    ] {
        writeln!(stdin, "{message}").unwrap();
    }
    let init = recv();
    assert_eq!(init["id"], 1, "{init}");
    let list = recv();
    assert_eq!(list["id"], 2, "{list}");
    let names: Vec<&str> = list["result"]["tools"]
        .as_array()
        .unwrap_or_else(|| panic!("no tools: {list}"))
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
    for expected in ["list_checks", "run_checks", "report_false_positive"] {
        assert!(
            names.contains(&expected),
            "{expected} missing from {names:?}"
        );
    }

    drop(stdin);
    assert!(bridge.0.wait().unwrap().success(), "bridge exits 0 on EOF");

    // SAFETY: plain kill(2) on our own child's pid.
    assert_eq!(
        unsafe { libc::kill(server.0.id() as i32, libc::SIGTERM) },
        0
    );
    assert!(server.0.wait().unwrap().success());
    assert!(!socket.exists(), "server unlinks its socket on shutdown");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn mcp_socket_should_RefuseToStart_When_AnotherServerOwnsPath() {
    let dir = scratch("dup");
    let socket = dir.join("run").join("mcp.sock");
    let _server = start_server(&dir, Some(&socket));
    let out = kibitzer_mcp(&dir, &[Path::new("--socket"), &socket])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("already owns"), "{stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn mcp_should_RejectSocketAndConnectTogether() {
    let out = Command::new(env!("CARGO_BIN_EXE_kibitzer"))
        .args(["mcp", "--socket", "/x/a.sock", "--connect", "/x/a.sock"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!out.status.success());
}
