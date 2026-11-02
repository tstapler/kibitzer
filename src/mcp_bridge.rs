//! `kibitzer mcp --connect`: a stdio MCP server that forwards every message to a shared
//! `kibitzer mcp --socket` server, so each MCP client runs this cheap proxy instead of its own
//! `KibitzerServer`. Speaks newline-delimited JSON-RPC on stdio and streamable HTTP (one POST
//! per message, SSE replies) over the Unix socket.
//!
//! Limits: no standalone GET stream, so server-initiated messages outside a request's own
//! reply stream are not relayed (kibitzer sends none); batches are passed through as-is,
//! which rmcp 0.2 answers with 501.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde_json::{Value, json};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::sync::{Mutex, mpsc};
use tokio::task::JoinSet;

use crate::mcp_socket::MCP_PATH;

const SESSION_HEADER: &str = "mcp-session-id";
const ACCEPT: &str = "application/json, text/event-stream";

pub async fn run_mcp_bridge(socket: PathBuf) -> Result<()> {
    bridge(
        socket,
        tokio::io::BufReader::new(tokio::io::stdin()),
        tokio::io::stdout(),
    )
    .await
}

/// Relays `input` lines to the server and its replies to `output` until `input` ends.
/// Requests run concurrently (a slow tool call doesn't hold up others); initialize,
/// notifications and responses are forwarded in order, each awaited before the next line.
pub(crate) async fn bridge<R, W>(socket: PathBuf, input: R, output: W) -> Result<()>
where
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let (tx, rx) = mpsc::unbounded_channel::<Value>();
    let writer = tokio::spawn(write_lines(rx, output));
    let session = Arc::new(Session::new(socket));
    let mut in_flight = JoinSet::new();
    let mut lines = input.lines();
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let message: Value = match serde_json::from_str(&line) {
            Ok(m) => m,
            Err(e) => {
                let _ = tx.send(rpc_error(Value::Null, -32700, &format!("parse error: {e}")));
                continue;
            }
        };
        if is_request(&message) && !is_initialize(&message) {
            let (session, tx) = (Arc::clone(&session), tx.clone());
            in_flight.spawn(async move { session.forward(message, &tx).await });
        } else {
            session.forward(message, &tx).await;
        }
    }
    // The client hung up: nobody is left to read in-flight replies.
    in_flight.abort_all();
    session.close().await;
    drop(tx);
    writer.await.context("bridge writer task")??;
    Ok(())
}

async fn write_lines<W: AsyncWrite + Unpin>(
    mut rx: mpsc::UnboundedReceiver<Value>,
    mut output: W,
) -> Result<()> {
    while let Some(message) = rx.recv().await {
        let mut line = serde_json::to_vec(&message)?;
        line.push(b'\n');
        output.write_all(&line).await?;
        output.flush().await?;
    }
    Ok(())
}

fn is_request(message: &Value) -> bool {
    message.get("method").is_some() && message.get("id").is_some()
}

fn is_initialize(message: &Value) -> bool {
    message.get("method").and_then(Value::as_str) == Some("initialize")
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

#[derive(Default)]
struct SessionState {
    id: Option<String>,
    /// Replayed to open a fresh session when the server has dropped ours (idle expiry,
    /// server restart).
    initialize: Option<Value>,
}

struct Session {
    socket: PathBuf,
    state: Mutex<SessionState>,
}

impl Session {
    fn new(socket: PathBuf) -> Self {
        Self {
            socket,
            state: Mutex::new(SessionState::default()),
        }
    }

    /// Forwards one message; a failed request is answered with a JSON-RPC error so the
    /// client never waits forever.
    async fn forward(&self, message: Value, out: &mpsc::UnboundedSender<Value>) {
        let id = is_request(&message).then(|| message["id"].clone());
        if let Err(e) = self.exchange(&message, out).await {
            match id {
                Some(id) => {
                    let _ = out.send(rpc_error(
                        id,
                        -32603,
                        &format!("kibitzer mcp bridge: {e:#}"),
                    ));
                }
                None => eprintln!("[kibitzer] mcp bridge: {e:#}"),
            }
        }
    }

    async fn exchange(&self, message: &Value, out: &mpsc::UnboundedSender<Value>) -> Result<()> {
        if is_initialize(message) {
            let (session_id, reply) = open_session(&self.socket, message).await?;
            let mut state = self.state.lock().await;
            state.id = Some(session_id);
            state.initialize = Some(message.clone());
            let _ = out.send(reply);
            return Ok(());
        }
        let session_id = self.state.lock().await.id.clone();
        let mut response = post(&self.socket, message, session_id.as_deref()).await?;
        if response.status() == StatusCode::UNAUTHORIZED && session_id.is_some() {
            let fresh = self.reopen(session_id.as_deref()).await?;
            response = post(&self.socket, message, Some(&fresh)).await?;
        }
        let status = response.status();
        if !status.is_success() {
            let body = response.into_body().collect().await?.to_bytes();
            bail!(
                "server answered {status}: {}",
                String::from_utf8_lossy(&body)
            );
        }
        if is_sse(&response) {
            relay_sse(response.into_body(), message.get("id"), out).await?;
        }
        Ok(())
    }

    /// Replaces the session the server no longer knows (`stale`) with a fresh one, unless a
    /// concurrent request already did.
    async fn reopen(&self, stale: Option<&str>) -> Result<String> {
        let mut state = self.state.lock().await;
        if let Some(current) = state.id.as_deref()
            && Some(current) != stale
        {
            return Ok(current.to_string());
        }
        let init = state
            .initialize
            .clone()
            .context("server dropped the session before initialize was seen")?;
        let (session_id, _reply) = open_session(&self.socket, &init).await?;
        let initialized = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
        let ack = post(&self.socket, &initialized, Some(&session_id)).await?;
        if !ack.status().is_success() {
            bail!("re-initialize: server answered {}", ack.status());
        }
        state.id = Some(session_id.clone());
        Ok(session_id)
    }

    async fn close(&self) {
        let Some(session_id) = self.state.lock().await.id.take() else {
            return;
        };
        let request = Request::builder()
            .method(Method::DELETE)
            .uri(MCP_PATH)
            .header(hyper::header::HOST, "localhost")
            .header(SESSION_HEADER, session_id)
            .body(Full::new(Bytes::new()));
        if let Ok(request) = request {
            let _ = send(&self.socket, request).await;
        }
    }
}

/// Sends `initialize` without a session; returns the new session id and the reply.
async fn open_session(socket: &Path, initialize: &Value) -> Result<(String, Value)> {
    let response = post(socket, initialize, None).await?;
    let status = response.status();
    if !status.is_success() {
        bail!("initialize: server answered {status}");
    }
    let session_id = response
        .headers()
        .get(SESSION_HEADER)
        .and_then(|v| v.to_str().ok())
        .context("initialize reply carried no session id")?
        .to_string();
    let (tx, mut rx) = mpsc::unbounded_channel();
    relay_sse(response.into_body(), initialize.get("id"), &tx).await?;
    let reply = rx.recv().await.context("initialize got no reply")?;
    Ok((session_id, reply))
}

fn is_sse(response: &Response<Incoming>) -> bool {
    response
        .headers()
        .get(hyper::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("text/event-stream"))
}

async fn post(socket: &Path, message: &Value, session: Option<&str>) -> Result<Response<Incoming>> {
    let mut request = Request::builder()
        .method(Method::POST)
        .uri(MCP_PATH)
        .header(hyper::header::HOST, "localhost")
        .header(hyper::header::CONTENT_TYPE, "application/json")
        .header(hyper::header::ACCEPT, ACCEPT);
    if let Some(session) = session {
        request = request.header(SESSION_HEADER, session);
    }
    let body = Full::new(Bytes::from(serde_json::to_vec(message)?));
    send(socket, request.body(body)?).await
}

/// One HTTP/1.1 exchange on a fresh connection to `socket`.
async fn send(socket: &Path, request: Request<Full<Bytes>>) -> Result<Response<Incoming>> {
    let stream = UnixStream::connect(socket)
        .await
        .with_context(|| format!("connecting to {}", socket.display()))?;
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .context("HTTP handshake")?;
    tokio::spawn(async move {
        let _ = connection.await;
    });
    sender
        .send_request(request)
        .await
        .context("sending request to the MCP server")
}

/// Forwards each SSE event's JSON payload until the reply to `request_id` (the stream may be
/// held open by keep-alives after it) or the end of the stream.
async fn relay_sse(
    mut body: Incoming,
    request_id: Option<&Value>,
    out: &mpsc::UnboundedSender<Value>,
) -> Result<()> {
    let mut buffer: Vec<u8> = Vec::new();
    while let Some(frame) = body.frame().await {
        let Ok(data) = frame?.into_data() else {
            continue;
        };
        buffer.extend_from_slice(&data);
        while let Some(end) = find_event_end(&buffer) {
            let event: Vec<u8> = buffer.drain(..end.0 + end.1).collect();
            let Some(message) = parse_sse_event(&event[..end.0])? else {
                continue;
            };
            let done = request_id.is_some_and(|id| message.get("id") == Some(id))
                && (message.get("result").is_some() || message.get("error").is_some());
            let _ = out.send(message);
            if done {
                return Ok(());
            }
        }
    }
    Ok(())
}

/// Offset of the blank line ending the first event in `buffer`, and its length.
fn find_event_end(buffer: &[u8]) -> Option<(usize, usize)> {
    let lf = buffer.windows(2).position(|w| w == b"\n\n").map(|i| (i, 2));
    let crlf = buffer
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|i| (i, 4));
    match (lf, crlf) {
        (Some(a), Some(b)) => Some(if a.0 <= b.0 { a } else { b }),
        (a, b) => a.or(b),
    }
}

/// The JSON in an event's `data:` lines, or `None` for a comment/keep-alive event.
fn parse_sse_event(event: &[u8]) -> Result<Option<Value>> {
    let text = String::from_utf8_lossy(event);
    let data: Vec<&str> = text
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(|d| d.strip_prefix(' ').unwrap_or(d))
        .collect();
    if data.is_empty() {
        return Ok(None);
    }
    Ok(Some(
        serde_json::from_str(&data.join("\n")).context("SSE data is not JSON")?,
    ))
}

#[cfg(test)]
#[allow(non_snake_case)]
pub(crate) mod tests {
    use std::time::Duration;

    use super::*;
    use crate::daemon_lock::current_uid;
    use crate::mcp_socket::tests::TestServer;
    use crate::test_support::unique_temp_dir;
    use tokio::io::{AsyncBufReadExt, BufReader, DuplexStream};

    fn initialize_message(id: u64) -> Value {
        json!({"jsonrpc": "2.0", "id": id, "method": "initialize", "params": {
            "protocolVersion": "2025-03-26",
            "capabilities": {},
            "clientInfo": {"name": "kibitzer bridge test", "version": "0"}
        }})
    }

    /// Opens a session directly over the socket, without the bridge; returns its id.
    pub(crate) async fn initialize_raw(socket: &Path) -> Result<String> {
        let opened = tokio::time::timeout(
            Duration::from_secs(10),
            open_session(socket, &initialize_message(1)),
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
        fn start(socket: &Path) -> Self {
            let (to_bridge, bridge_in) = tokio::io::duplex(1 << 16);
            let (bridge_out, from_bridge) = tokio::io::duplex(1 << 16);
            let task = tokio::spawn(bridge(
                socket.to_path_buf(),
                BufReader::new(bridge_in),
                bridge_out,
            ));
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

        async fn finish(self) {
            drop(self.to_bridge);
            self.task.await.unwrap().unwrap();
        }
    }

    #[tokio::test]
    async fn bridge_should_RelayHandshakeAndToolsList_Then_CloseSessionOnEof() {
        let dir = unique_temp_dir("mcpbr-list");
        let server = TestServer::start(&dir, Duration::from_secs(3600), current_uid()).await;
        let mut client = Client::start(&server.path);
        client.handshake().await;
        let names = client.tool_names(2).await;
        for expected in ["list_checks", "run_checks", "architecture_assessment"] {
            assert!(names.iter().any(|n| n == expected), "{expected}: {names:?}");
        }
        client.finish().await;
        // EOF on stdin DELETEs the session.
        assert!(server.sessions.sessions.read().await.is_empty());
        server.stop().await;
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn bridge_should_Reinitialize_When_ServerDroppedIdleSession() {
        let dir = unique_temp_dir("mcpbr-idle");
        let server = TestServer::start(&dir, Duration::from_millis(200), current_uid()).await;
        let mut client = Client::start(&server.path);
        client.handshake().await;
        tokio::time::sleep(Duration::from_millis(800)).await;
        assert!(server.sessions.sessions.read().await.is_empty());
        let names = client.tool_names(2).await;
        assert!(names.iter().any(|n| n == "run_checks"), "{names:?}");
        client.finish().await;
        server.stop().await;
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn bridge_should_AnswerWithRpcError_When_InputIsBadOrServerIsGone() {
        let dir = unique_temp_dir("mcpbr-err");
        let mut client = Client::start(&dir.join("nothing.sock"));
        client.send("{not json").await;
        let reply = client.recv().await;
        assert_eq!(reply["error"]["code"], -32700, "{reply}");
        client.send(&initialize_message(7).to_string()).await;
        let reply = client.recv().await;
        assert_eq!(reply["id"], 7);
        assert_eq!(reply["error"]["code"], -32603, "{reply}");
        // A notification that fails has nobody to answer; it must not wedge the bridge.
        client
            .send(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
            .await;
        client.finish().await;
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn bridge_should_ReportServerError_When_RequestRejected() {
        let dir = unique_temp_dir("mcpbr-422");
        let server = TestServer::start(&dir, Duration::from_secs(3600), current_uid()).await;
        let mut client = Client::start(&server.path);
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
        client.finish().await;
        server.stop().await;
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_sse_event_should_JoinDataLines_And_SkipComments() {
        assert_eq!(parse_sse_event(b": keep-alive").unwrap(), None);
        assert_eq!(
            parse_sse_event(b"id: 1\ndata: {\"a\":\ndata: 1}").unwrap(),
            Some(json!({"a": 1}))
        );
        assert!(parse_sse_event(b"data: nope").is_err());
        assert_eq!(find_event_end(b"data: x\r\n\r\nrest"), Some((7, 4)));
        assert_eq!(find_event_end(b"data: x\n\nrest"), Some((7, 2)));
        assert_eq!(find_event_end(b"data: x\n"), None);
    }
}
