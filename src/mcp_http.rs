//! `kibitzer mcp --http`: the MCP server over streamable HTTP, so one long-lived process
//! can serve many clients instead of each client spawning its own `kibitzer mcp`.
//!
//! Each MCP session gets its own `KibitzerServer` (and so its own architecture-model
//! cache slot), matching what a per-client stdio process had. The endpoint has no
//! authentication: it binds loopback by default and, while bound to loopback, rejects
//! requests whose `Host`/`Origin` isn't loopback (DNS-rebinding protection), which
//! rmcp 0.2's `StreamableHttpService` does not do itself.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use axum::Router;
use axum::extract::Request;
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use rmcp::transport::streamable_http_server::session::local::{LocalSessionManager, SessionConfig};
use rmcp::transport::{StreamableHttpServerConfig, StreamableHttpService};
use tokio::net::TcpListener;
use tokio::sync::watch;

use crate::mcp::KibitzerServer;

pub const MCP_PATH: &str = "/mcp";

/// rmcp's default is no idle timeout, which would leak every session a client abandons
/// without a DELETE for the life of the server.
const SESSION_IDLE_TIMEOUT: Duration = Duration::from_secs(2 * 60 * 60);

/// Open SSE streams never finish on their own, so graceful shutdown would otherwise wait
/// on them forever.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

/// Refuses a non-loopback bind unless the caller explicitly opted in.
// kibitzer:ignore flag-argument -- mirrors the --allow-remote CLI switch one-to-one
pub fn check_bind_addr(addr: SocketAddr, allow_remote: bool) -> Result<()> {
    if !addr.ip().is_loopback() && !allow_remote {
        bail!(
            "refusing to bind the MCP HTTP server to non-loopback address {addr}: the \
             endpoint has no authentication, so anyone who can reach it can run every tool. \
             Pass --allow-remote to bind it anyway."
        );
    }
    Ok(())
}

pub async fn run_mcp_http_server(addr: SocketAddr, allow_remote: bool) -> Result<()> {
    check_bind_addr(addr, allow_remote)?;
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("binding MCP HTTP server to {addr}"))?;
    let local = listener.local_addr()?;
    eprintln!("[kibitzer] MCP streamable HTTP server listening on http://{local}{MCP_PATH}");

    let (tx, rx) = watch::channel(false);
    tokio::spawn(async move {
        wait_for_termination_signal().await;
        eprintln!("[kibitzer] shutting down MCP HTTP server");
        let _ = tx.send(true);
    });
    serve(listener, rx).await
}

async fn wait_for_termination_signal() {
    use tokio::signal::unix::{SignalKind, signal};
    let mut sigterm = match signal(SignalKind::terminate()) {
        Ok(s) => s,
        Err(_) => {
            let _ = tokio::signal::ctrl_c().await;
            return;
        }
    };
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = sigterm.recv() => {}
    }
}

/// Serves until `shutdown` flips to true, then drains for at most `SHUTDOWN_GRACE`.
async fn serve(listener: TcpListener, shutdown: watch::Receiver<bool>) -> Result<()> {
    let loopback_only = listener.local_addr()?.ip().is_loopback();
    let server = axum::serve(listener, router(loopback_only))
        .with_graceful_shutdown(until_shutdown(shutdown.clone()));
    tokio::select! {
        result = server => result.context("MCP HTTP server failed"),
        _ = async {
            until_shutdown(shutdown).await;
            tokio::time::sleep(SHUTDOWN_GRACE).await;
        } => Ok(()),
    }
}

async fn until_shutdown(mut shutdown: watch::Receiver<bool>) {
    if shutdown.wait_for(|stop| *stop).await.is_err() {
        // Sender dropped without signalling: never shut down on that alone.
        std::future::pending::<()>().await;
    }
}

// kibitzer:ignore flag-argument -- derived from the bound address, toggles one layer
fn router(loopback_only: bool) -> Router {
    let session_manager = LocalSessionManager {
        sessions: Default::default(),
        session_config: SessionConfig {
            keep_alive: Some(SESSION_IDLE_TIMEOUT),
            ..Default::default()
        },
    };
    let service = StreamableHttpService::new(
        || Ok(KibitzerServer::new()),
        Arc::new(session_manager),
        StreamableHttpServerConfig::default(),
    );
    let router = Router::new().route_service(MCP_PATH, service);
    if loopback_only {
        router.layer(middleware::from_fn(reject_non_loopback_host))
    } else {
        router
    }
}

async fn reject_non_loopback_host(req: Request, next: Next) -> Response {
    match check_loopback_headers(req.headers()) {
        Ok(()) => next.run(req).await,
        Err(why) => (StatusCode::FORBIDDEN, why).into_response(),
    }
}

fn check_loopback_headers(headers: &HeaderMap) -> Result<(), &'static str> {
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok());
    if !host.is_some_and(is_loopback_authority) {
        return Err("Forbidden: Host must be a loopback address");
    }
    if let Some(origin) = headers.get(header::ORIGIN) {
        let loopback_origin = origin
            .to_str()
            .ok()
            .and_then(|o| o.split_once("://"))
            .is_some_and(|(_, authority)| is_loopback_authority(authority));
        if !loopback_origin {
            return Err("Forbidden: Origin must be a loopback origin");
        }
    }
    Ok(())
}

/// `authority` is a `Host` header value: `host`, `host:port`, or `[v6]:port`.
fn is_loopback_authority(authority: &str) -> bool {
    let host = match authority.strip_prefix('[') {
        Some(rest) => match rest.split_once(']') {
            Some((h, tail)) if tail.is_empty() || tail.starts_with(':') => h,
            _ => return false,
        },
        None => authority.split(':').next().unwrap_or_default(),
    };
    host.eq_ignore_ascii_case("localhost")
        || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers(host: Option<&str>, origin: Option<&str>) -> HeaderMap {
        let mut h = HeaderMap::new();
        if let Some(host) = host {
            h.insert(header::HOST, HeaderValue::from_str(host).unwrap());
        }
        if let Some(origin) = origin {
            h.insert(header::ORIGIN, HeaderValue::from_str(origin).unwrap());
        }
        h
    }

    #[test]
    fn check_bind_addr_should_RefuseNonLoopback_When_NotOptedIn() {
        let err = check_bind_addr("0.0.0.0:8787".parse().unwrap(), false).unwrap_err();
        assert!(err.to_string().contains("--allow-remote"), "{err}");
        assert!(check_bind_addr("192.168.1.5:0".parse().unwrap(), false).is_err());
    }

    #[test]
    fn check_bind_addr_should_Allow_When_LoopbackOrOptedIn() {
        assert!(check_bind_addr("127.0.0.1:0".parse().unwrap(), false).is_ok());
        assert!(check_bind_addr("[::1]:0".parse().unwrap(), false).is_ok());
        assert!(check_bind_addr("0.0.0.0:0".parse().unwrap(), true).is_ok());
    }

    #[test]
    fn is_loopback_authority_should_AcceptLoopbackForms() {
        for ok in [
            "localhost",
            "LOCALHOST:8787",
            "127.0.0.1",
            "127.0.0.1:8787",
            "127.1.2.3:1",
            "[::1]",
            "[::1]:8787",
        ] {
            assert!(is_loopback_authority(ok), "{ok}");
        }
    }

    #[test]
    fn is_loopback_authority_should_RejectEverythingElse() {
        for bad in [
            "",
            "evil.example",
            "evil.example:8787",
            "localhost.evil.example",
            "10.0.0.1:8787",
            "[::2]:8787",
            "[::1]junk",
            "::1",
        ] {
            assert!(!is_loopback_authority(bad), "{bad}");
        }
    }

    #[test]
    fn check_loopback_headers_should_RejectRebindingAndForeignOrigin() {
        assert!(check_loopback_headers(&headers(Some("127.0.0.1:1"), None)).is_ok());
        assert!(
            check_loopback_headers(&headers(Some("localhost:1"), Some("http://localhost:3000")))
                .is_ok()
        );
        assert!(check_loopback_headers(&headers(None, None)).is_err());
        assert!(check_loopback_headers(&headers(Some("evil.example:1"), None)).is_err());
        assert!(
            check_loopback_headers(&headers(Some("127.0.0.1:1"), Some("https://evil.example")))
                .is_err()
        );
        assert!(check_loopback_headers(&headers(Some("127.0.0.1:1"), Some("null"))).is_err());
    }
}
