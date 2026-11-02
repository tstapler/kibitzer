//! Tests for `mcp_bridge`, kept apart to keep that module readable.
#![allow(non_snake_case)]

use std::path::Path;
use std::time::Duration;

use super::*;
use crate::mcp_socket::tests::TestServer;
use crate::test_support::unique_temp_dir;
use tokio::io::{AsyncBufReadExt, BufReader, DuplexStream};

const LONG_IDLE: Duration = Duration::from_secs(3600);

fn initialize_message(id: u64) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": "initialize", "params": {
        "protocolVersion": "2025-03-26",
        "capabilities": {},
        "clientInfo": {"name": "kibitzer bridge test", "version": "0"}
    }})
}

fn ours(socket: &Path) -> ServerAddr {
    ServerAddr {
        socket: socket.to_path_buf(),
        uid: current_uid(),
    }
}

/// Opens a session directly over the socket, without the bridge; returns its id.
pub(crate) async fn initialize_raw(socket: &Path) -> Result<String> {
    let opened = tokio::time::timeout(
        Duration::from_secs(10),
        open_session(&ours(socket), &initialize_message(1)),
    )
    .await
    .context("initialize timed out")??;
    Ok(opened.0)
}

struct Client {
    to_bridge: DuplexStream,
    from_bridge: tokio::io::Lines<BufReader<DuplexStream>>,
    task: tokio::task::JoinHandle<Result<()>>,
}

impl Client {
    fn start(server: ServerAddr) -> Self {
        let (to_bridge, bridge_in) = tokio::io::duplex(1 << 16);
        let (bridge_out, from_bridge) = tokio::io::duplex(1 << 16);
        let task = tokio::spawn(bridge(server, BufReader::new(bridge_in), bridge_out));
        Self {
            to_bridge,
            from_bridge: BufReader::new(from_bridge).lines(),
            task,
        }
    }

    async fn send(&mut self, message: &str) {
        self.to_bridge.write_all(message.as_bytes()).await.unwrap();
        self.to_bridge.write_all(b"\n").await.unwrap();
    }

    async fn recv(&mut self) -> Value {
        let line = tokio::time::timeout(Duration::from_secs(30), self.from_bridge.next_line())
            .await
            .expect("bridge reply timed out")
            .unwrap()
            .expect("bridge closed its output");
        serde_json::from_str(&line).unwrap()
    }

    async fn handshake(&mut self) {
        self.send(&initialize_message(1).to_string()).await;
        let reply = self.recv().await;
        assert_eq!(reply["id"], 1);
        assert!(
            reply["result"]["instructions"]
                .as_str()
                .is_some_and(|s| s.starts_with("kibitzer:")),
            "{reply}"
        );
        self.send(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
            .await;
    }

    async fn tool_names(&mut self, id: u64) -> Vec<String> {
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": "tools/list"}).to_string())
            .await;
        let reply = self.recv().await;
        assert_eq!(reply["id"], id, "{reply}");
        reply["result"]["tools"]
            .as_array()
            .unwrap_or_else(|| panic!("no tools: {reply}"))
            .iter()
            .filter_map(|t| t["name"].as_str().map(str::to_string))
            .collect()
    }

    async fn finish(self) -> Result<()> {
        drop(self.to_bridge);
        self.task.await.unwrap()
    }
}

#[tokio::test]
async fn bridge_should_RelayHandshakeAndToolsList_Then_CloseSessionOnEof() {
    let dir = unique_temp_dir("mcpbr-list");
    let server = TestServer::start(&dir, LONG_IDLE, current_uid()).await;
    let mut client = Client::start(ours(&server.path));
    client.handshake().await;
    let names = client.tool_names(2).await;
    for expected in ["list_checks", "run_checks", "architecture_assessment"] {
        assert!(names.iter().any(|n| n == expected), "{expected}: {names:?}");
    }
    client.finish().await.unwrap();
    // EOF on stdin DELETEs the session.
    assert!(server.sessions.sessions.read().await.is_empty());
    server.stop().await;
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn bridge_should_RefuseServer_When_PeerUidIsNotTheExpectedOne() {
    let dir = unique_temp_dir("mcpbr-uid");
    let server = TestServer::start(&dir, LONG_IDLE, current_uid()).await;
    let mut client = Client::start(ServerAddr {
        socket: server.path.clone(),
        uid: current_uid() + 1,
    });
    client.send(&initialize_message(5).to_string()).await;
    let reply = client.recv().await;
    assert_eq!(reply["id"], 5);
    let message = reply["error"]["message"].as_str().unwrap_or_default();
    assert!(message.contains("refusing the MCP server"), "{reply}");
    client.finish().await.unwrap();
    assert!(
        server.sessions.sessions.read().await.is_empty(),
        "nothing reached the server"
    );
    server.stop().await;
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn bridge_should_Reinitialize_When_ServerDroppedIdleSession() {
    let dir = unique_temp_dir("mcpbr-idle");
    let server = TestServer::start(&dir, Duration::from_millis(200), current_uid()).await;
    let mut client = Client::start(ours(&server.path));
    client.handshake().await;
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert!(server.sessions.sessions.read().await.is_empty());
    let names = client.tool_names(2).await;
    assert!(names.iter().any(|n| n == "run_checks"), "{names:?}");
    client.finish().await.unwrap();
    server.stop().await;
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn bridge_should_AnswerWithRpcError_When_InputIsBadOrServerIsGone() {
    let dir = unique_temp_dir("mcpbr-err");
    let mut client = Client::start(ours(&dir.join("nothing.sock")));
    client.send("{not json").await;
    let reply = client.recv().await;
    assert_eq!(reply["error"]["code"], PARSE_ERROR, "{reply}");
    client.send(&initialize_message(7).to_string()).await;
    let reply = client.recv().await;
    assert_eq!(reply["id"], 7);
    assert_eq!(reply["error"]["code"], INTERNAL_ERROR, "{reply}");
    // A notification that fails has nobody to answer; it must not wedge the bridge.
    client
        .send(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
        .await;
    client.finish().await.unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn bridge_should_AnswerParseError_And_StillCloseSession_When_InputIsInvalidUtf8() {
    let dir = unique_temp_dir("mcpbr-utf8");
    let server = TestServer::start(&dir, LONG_IDLE, current_uid()).await;
    let mut client = Client::start(ours(&server.path));
    client.handshake().await;
    client.to_bridge.write_all(b"\xff\xfe{\n").await.unwrap();
    let reply = client.recv().await;
    assert_eq!(reply["error"]["code"], PARSE_ERROR, "{reply}");
    client.finish().await.unwrap();
    assert!(server.sessions.sessions.read().await.is_empty());
    server.stop().await;
    let _ = std::fs::remove_dir_all(&dir);
}

/// A reader whose first read fails, standing in for a broken stdin.
struct BrokenInput;

impl tokio::io::AsyncRead for BrokenInput {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
        _: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Err(std::io::Error::other("stdin broke")))
    }
}

#[tokio::test]
async fn bridge_should_ReturnError_But_StillClose_When_InputFails() {
    let dir = unique_temp_dir("mcpbr-broken");
    let output = tokio::io::sink();
    let err = bridge(
        ours(&dir.join("nothing.sock")),
        BufReader::new(BrokenInput),
        output,
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("stdin broke"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn bridge_should_ReportServerError_When_RequestRejected() {
    let dir = unique_temp_dir("mcpbr-422");
    let server = TestServer::start(&dir, LONG_IDLE, current_uid()).await;
    let mut client = Client::start(ours(&server.path));
    // No session yet and not an initialize: rmcp answers 422.
    client
        .send(r#"{"jsonrpc":"2.0","id":3,"method":"tools/list"}"#)
        .await;
    let reply = client.recv().await;
    assert_eq!(reply["id"], 3);
    assert!(
        reply["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("422")),
        "{reply}"
    );
    client.finish().await.unwrap();
    server.stop().await;
    let _ = std::fs::remove_dir_all(&dir);
}

async fn relay(body: &'static [u8], id: Option<&Value>, max: usize) -> Result<Vec<Value>> {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let result = relay_sse(Full::new(Bytes::from_static(body)), id, &tx, max).await;
    drop(tx);
    let mut got = Vec::new();
    while let Some(v) = rx.recv().await {
        got.push(v);
    }
    result.map(|()| got)
}

#[tokio::test]
async fn relay_sse_should_StopAtReply_And_ErrorWhenStreamEndsWithoutIt() {
    let id = json!(4);
    let got = relay(
        b": ping\n\ndata: {\"id\":9,\"result\":1}\n\ndata: {\"id\":4,\"result\":2}\n\ndata: {}\n\n",
        Some(&id),
        1024,
    )
    .await
    .unwrap();
    assert_eq!(got.len(), 2, "stops after the reply: {got:?}");
    let err = relay(b"data: {\"id\":9,\"result\":1}\n\n", Some(&id), 1024)
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("without answering request 4"),
        "{err}"
    );
    assert!(relay(b"", None, 1024).await.unwrap().is_empty());
}

#[tokio::test]
async fn relay_sse_should_Error_When_EventExceedsCap() {
    let err = relay(b"data: 0123456789", Some(&json!(1)), 8)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("over 8 bytes"), "{err}");
}

#[test]
fn parse_sse_event_should_JoinDataLines_And_SkipComments() {
    assert_eq!(parse_sse_event(b": keep-alive").unwrap(), None);
    assert_eq!(
        parse_sse_event(b"id: 1\ndata: {\"a\":\ndata: 1}").unwrap(),
        Some(json!({"a": 1}))
    );
    assert!(parse_sse_event(b"data: nope").is_err());
}

#[test]
fn find_event_end_should_FindDelimiter_IncludingOneStraddlingTheScannedTail() {
    assert_eq!(find_event_end(b"data: x\r\n\r\nrest", 0), Some((7, 4)));
    assert_eq!(find_event_end(b"data: x\n\nrest", 0), Some((7, 2)));
    assert_eq!(find_event_end(b"data: x\n", 0), None);
    // "\r\n" was already scanned; the rest of the delimiter arrived in the next frame.
    assert_eq!(find_event_end(b"data: x\r\n\r\n", 9), Some((7, 4)));
    assert_eq!(find_event_end(b"data: x\n\n", 8), Some((7, 2)));
    assert_eq!(find_event_end(b"ab", 10), None);
}
