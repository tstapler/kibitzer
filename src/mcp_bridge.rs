//! `kibitzer mcp --connect`: a stdio MCP server that forwards every message to a shared
//! `kibitzer mcp --socket` server, so each MCP client runs this cheap proxy instead of its own
//! `KibitzerServer`. Speaks newline-delimited JSON-RPC on stdio and streamable HTTP (one POST
//! per message, SSE replies) over the Unix socket.
//!
//! Limits: no standalone GET stream, so server-initiated messages outside a request's own
//! reply stream are not relayed (kibitzer sends none); batches are passed through as-is,
//! which rmcp 0.2 answers with 501.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use bytes::Bytes;
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::{Body, Incoming};
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde_json::{Value, json};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::sync::{Mutex, mpsc};
use tokio::task::JoinSet;

use crate::daemon_lock::current_uid;
use crate::mcp_socket::MCP_PATH;

const SESSION_HEADER: &str = "mcp-session-id";
const ACCEPT: &str = "application/json, text/event-stream";
const PARSE_ERROR: i64 = -32700;
const INTERNAL_ERROR: i64 = -32603;

/// Largest single SSE event or error body accepted from the server.
const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;

/// The shared server, and the uid it must run as: anyone else squatting the socket path
/// could feed fabricated tool results to the agent.
#[derive(Clone)]
pub(crate) struct ServerAddr {
    pub socket: PathBuf,
    pub uid: u32,
}

pub async fn run_mcp_bridge(socket: PathBuf) -> Result<()> {
    let server = ServerAddr {
        socket,
        uid: current_uid(),
    };
    bridge(
        server,
        tokio::io::BufReader::new(tokio::io::stdin()),
        tokio::io::stdout(),
    )
    .await
}

/// Relays `input` lines to the server and its replies to `output` until `input` ends or fails;
/// either way the session is then closed. Requests run concurrently (a slow tool call doesn't
/// hold up others); initialize, notifications and responses are forwarded in order.
pub(crate) async fn bridge<R, W>(server: ServerAddr, input: R, output: W) -> Result<()>
where
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let (tx, rx) = mpsc::unbounded_channel::<Value>();
    let writer = tokio::spawn(write_lines(rx, output));
    let session = Arc::new(Session::new(server));
    let mut in_flight = JoinSet::new();
    let read = relay_input(input, &session, &tx, &mut in_flight).await;
    // The client hung up (or its stdin broke): nobody is left to read in-flight replies.
    in_flight.abort_all();
    session.close().await;
    drop(tx);
    writer.await.context("bridge writer task")??;
    read
}

async fn relay_input<R: AsyncBufRead + Unpin>(
    mut input: R,
    session: &Arc<Session>,
    tx: &mpsc::UnboundedSender<Value>,
    in_flight: &mut JoinSet<()>,
) -> Result<()> {
    let mut raw = Vec::new();
    loop {
        raw.clear();
        if input.read_until(b'\n', &mut raw).await? == 0 {
            return Ok(());
        }
        // Lossy, so invalid UTF-8 gets a JSON-RPC parse error instead of ending the bridge.
        let line = String::from_utf8_lossy(&raw);
        if line.trim().is_empty() {
            continue;
        }
        let message: Value = match serde_json::from_str(&line) {
            Ok(m) => m,
            Err(e) => {
                let _ = tx.send(rpc_error(
                    Value::Null,
                    PARSE_ERROR,
                    &format!("parse error: {e}"),
                ));
                continue;
            }
        };
        if is_request(&message) && !is_initialize(&message) {
            let (session, tx) = (Arc::clone(session), tx.clone());
            in_flight.spawn(async move { session.forward(message, &tx).await });
        } else {
            session.forward(message, tx).await;
        }
    }
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

fn request_id(message: &Value) -> Option<&Value> {
    message.get("method").and(message.get("id"))
}

fn is_request(message: &Value) -> bool {
    request_id(message).is_some()
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
    server: ServerAddr,
    state: Mutex<SessionState>,
}

impl Session {
    fn new(server: ServerAddr) -> Self {
        Self {
            server,
            state: Mutex::new(SessionState::default()),
        }
    }

    /// Forwards one message; a failed request is answered with a JSON-RPC error so the
    /// client never waits forever.
    async fn forward(&self, message: Value, out: &mpsc::UnboundedSender<Value>) {
        let id = request_id(&message).cloned();
        if let Err(e) = self.exchange(&message, out).await {
            match id {
                Some(id) => {
                    let _ = out.send(rpc_error(
                        id,
                        INTERNAL_ERROR,
                        &format!("kibitzer mcp bridge: {e:#}"),
                    ));
                }
                None => eprintln!("[kibitzer] mcp bridge: {e:#}"),
            }
        }
    }

    async fn exchange(&self, message: &Value, out: &mpsc::UnboundedSender<Value>) -> Result<()> {
        if is_initialize(message) {
            let (session_id, reply) = open_session(&self.server, message).await?;
            let mut state = self.state.lock().await;
            state.id = Some(session_id);
            state.initialize = Some(message.clone());
            let _ = out.send(reply);
            return Ok(());
        }
        let session_id = self.state.lock().await.id.clone();
        let mut response = post(&self.server, message, session_id.as_deref()).await?;
        if response.status() == StatusCode::UNAUTHORIZED && session_id.is_some() {
            let fresh = self.reopen(session_id.as_deref()).await?;
            response = post(&self.server, message, Some(&fresh)).await?;
        }
        let status = response.status();
        if !status.is_success() {
            let body = Limited::new(response.into_body(), MAX_MESSAGE_BYTES)
                .collect()
                .await
                .map_err(|e| anyhow::anyhow!("reading {status} reply: {e}"))?
                .to_bytes();
            bail!(
                "server answered {status}: {}",
                String::from_utf8_lossy(&body)
            );
        }
        if is_sse(&response) {
            relay_sse(
                response.into_body(),
                request_id(message),
                out,
                MAX_MESSAGE_BYTES,
            )
            .await?;
        } else if let Some(id) = request_id(message) {
            bail!("server answered {status} without a reply to request {id}");
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
        let (session_id, _reply) = open_session(&self.server, &init).await?;
        let initialized = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
        let ack = post(&self.server, &initialized, Some(&session_id)).await?;
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
            let _ = send(&self.server, request).await;
        }
    }
}

/// Sends `initialize` without a session; returns the new session id and the reply.
async fn open_session(server: &ServerAddr, initialize: &Value) -> Result<(String, Value)> {
    let response = post(server, initialize, None).await?;
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
    relay_sse(
        response.into_body(),
        request_id(initialize),
        &tx,
        MAX_MESSAGE_BYTES,
    )
    .await?;
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

async fn post(
    server: &ServerAddr,
    message: &Value,
    session: Option<&str>,
) -> Result<Response<Incoming>> {
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
    send(server, request.body(body)?).await
}

/// Connects to `server.socket`, refusing (fail closed) a peer not running as `server.uid`.
async fn connect(server: &ServerAddr) -> Result<UnixStream> {
    let socket = &server.socket;
    let stream = UnixStream::connect(socket)
        .await
        .with_context(|| format!("connecting to {}", socket.display()))?;
    let peer = stream
        .peer_cred()
        .with_context(|| format!("reading the peer uid on {}", socket.display()))?
        .uid();
    if peer != server.uid {
        bail!(
            "refusing the MCP server on {}: it runs as uid {peer}, not uid {}",
            socket.display(),
            server.uid
        );
    }
    Ok(stream)
}

/// One HTTP/1.1 exchange on a fresh connection to the server.
async fn send(server: &ServerAddr, request: Request<Full<Bytes>>) -> Result<Response<Incoming>> {
    let stream = connect(server).await?;
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
/// held open by keep-alives after it). A stream that ends first, or an event over `max`
/// bytes, is an error.
async fn relay_sse<B>(
    mut body: B,
    request_id: Option<&Value>,
    out: &mpsc::UnboundedSender<Value>,
    max: usize,
) -> Result<()>
where
    B: Body<Data = Bytes> + Unpin,
    B::Error: std::error::Error + Send + Sync + 'static,
{
    let mut buffer: Vec<u8> = Vec::new();
    // Bytes already searched for an event end, so each frame only scans what's new.
    let mut scanned = 0;
    while let Some(frame) = body.frame().await {
        let Ok(data) = frame?.into_data() else {
            continue;
        };
        buffer.extend_from_slice(&data);
        while let Some((end, delimiter)) = find_event_end(&buffer, scanned) {
            let event: Vec<u8> = buffer.drain(..end + delimiter).collect();
            scanned = 0;
            let Some(message) = parse_sse_event(&event[..end])? else {
                continue;
            };
            let done = request_id.is_some_and(|id| message.get("id") == Some(id))
                && (message.get("result").is_some() || message.get("error").is_some());
            let _ = out.send(message);
            if done {
                return Ok(());
            }
        }
        if buffer.len() > max {
            bail!("server sent an SSE event over {max} bytes");
        }
        scanned = buffer.len();
    }
    match request_id {
        Some(id) => bail!("server ended the reply stream without answering request {id}"),
        None => Ok(()),
    }
}

/// Offset and length of the first blank line (`\n\n` or `\r\n\r\n`) ending at or after
/// `from` (bytes before it were already searched).
fn find_event_end(buffer: &[u8], from: usize) -> Option<(usize, usize)> {
    // A delimiter may straddle the old tail, so back up by one less than its longest form.
    let start = from.saturating_sub(3);
    let search = &buffer[start.min(buffer.len())..];
    let lf = search.windows(2).position(|w| w == b"\n\n").map(|i| (i, 2));
    let crlf = search
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|i| (i, 4));
    let first = match (lf, crlf) {
        (Some(a), Some(b)) => Some(if a.0 <= b.0 { a } else { b }),
        (a, b) => a.or(b),
    };
    first.map(|(i, len)| (start + i, len))
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
#[path = "mcp_bridge_tests.rs"]
pub(crate) mod tests;
