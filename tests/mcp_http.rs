//! End-to-end `kibitzer mcp --http`: spawns the real binary on an ephemeral loopback port
//! and speaks MCP streamable HTTP to it (initialize, initialized, tools/list).
#![allow(non_snake_case)]
#![cfg(unix)]

use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Command, Stdio};

use serde_json::{Value, json};

const ACCEPT: &str = "application/json, text/event-stream";

/// Kills the server on drop so a failed assertion never leaks a listening process.
struct Server {
    child: Child,
    url: String,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn start_server() -> Server {
    let mut child = Command::new(env!("CARGO_BIN_EXE_kibitzer"))
        .args(["mcp", "--http", "127.0.0.1:0"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn kibitzer mcp --http");
    let mut stderr = BufReader::new(child.stderr.take().unwrap());
    let mut line = String::new();
    stderr.read_line(&mut line).unwrap();
    let url = line
        .split_whitespace()
        .find(|w| w.starts_with("http://"))
        .unwrap_or_else(|| panic!("no listen URL in startup line: {line:?}"))
        .to_string();
    // Keep draining stderr so the server never blocks on a full pipe.
    std::thread::spawn(move || {
        let _ = std::io::copy(&mut stderr, &mut std::io::sink());
    });
    Server { child, url }
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .into()
}

/// POSTs one JSON-RPC message; returns (status, session id header, first SSE `data:` JSON).
fn post(
    agent: &ureq::Agent,
    url: &str,
    session: Option<&str>,
    body: Value,
) -> (u16, Option<String>, Option<Value>) {
    let mut req = agent
        .post(url)
        .header("Content-Type", "application/json")
        .header("Accept", ACCEPT);
    if let Some(session) = session {
        req = req.header("Mcp-Session-Id", session);
    }
    let resp = req.send(body.to_string()).expect("HTTP request");
    let status = resp.status().as_u16();
    let session_id = resp
        .headers()
        .get("mcp-session-id")
        .map(|v| v.to_str().unwrap().to_string());
    let is_sse = resp
        .headers()
        .get("content-type")
        .is_some_and(|v| v.to_str().unwrap().starts_with("text/event-stream"));
    let mut reader = BufReader::new(resp.into_body().into_reader());
    let data = if is_sse {
        // The stream may stay open (keep-alive), so stop at the first event rather than EOF.
        reader.by_ref().lines().find_map(|line| {
            let line = line.expect("read SSE line");
            line.strip_prefix("data:")
                .map(|d| serde_json::from_str(d.trim()).expect("SSE data is JSON"))
        })
    } else {
        None
    };
    (status, session_id, data)
}

fn initialize(agent: &ureq::Agent, url: &str) -> String {
    let (status, session, data) = post(
        agent,
        url,
        None,
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2025-03-26",
            "capabilities": {},
            "clientInfo": {"name": "mcp_http test", "version": "0"}
        }}),
    );
    assert_eq!(status, 200);
    let data = data.expect("initialize response event");
    assert!(
        data["result"]["instructions"]
            .as_str()
            .is_some_and(|s| s.starts_with("kibitzer:")),
        "{data}"
    );
    session.expect("initialize sets Mcp-Session-Id")
}

#[test]
fn mcp_http_should_ListTools_When_ClientCompletesHandshake() {
    let server = start_server();
    let agent = agent();
    let session = initialize(&agent, &server.url);

    let (status, _, _) = post(
        &agent,
        &server.url,
        Some(&session),
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
    );
    assert_eq!(status, 202);

    let (status, _, data) = post(
        &agent,
        &server.url,
        Some(&session),
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
    );
    assert_eq!(status, 200);
    let data = data.expect("tools/list response event");
    let names: Vec<&str> = data["result"]["tools"]
        .as_array()
        .unwrap_or_else(|| panic!("no tools array: {data}"))
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
    for expected in [
        "list_checks",
        "run_checks",
        "architecture_assessment",
        "report_false_positive",
    ] {
        assert!(
            names.contains(&expected),
            "{expected} missing from {names:?}"
        );
    }
}

#[test]
fn mcp_http_should_IssueDistinctSessions_When_TwoClientsInitialize() {
    let server = start_server();
    let agent = agent();
    let a = initialize(&agent, &server.url);
    let b = initialize(&agent, &server.url);
    assert_ne!(a, b);
}

#[test]
fn mcp_http_should_RejectRequest_When_NoSessionAndNotInitialize() {
    let server = start_server();
    let (status, _, _) = post(
        &agent(),
        &server.url,
        None,
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
    );
    assert_eq!(status, 422);
}

#[test]
fn mcp_http_should_RefuseNonLoopbackBind_When_AllowRemoteAbsent() {
    let out = Command::new(env!("CARGO_BIN_EXE_kibitzer"))
        .args(["mcp", "--http", "0.0.0.0:0"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("non-loopback"), "{stderr}");
    assert!(stderr.contains("--allow-remote"), "{stderr}");
}

#[test]
fn mcp_should_RejectAllowRemote_When_HttpAbsent() {
    let out = Command::new(env!("CARGO_BIN_EXE_kibitzer"))
        .args(["mcp", "--allow-remote"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!out.status.success());
}
