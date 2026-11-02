//! `kibitzer mcp --socket`: the MCP server as streamable HTTP over a Unix domain socket, so one
//! long-lived process can serve many clients instead of each client spawning `kibitzer mcp`.
//!
//! There is no authentication beyond the OS: the socket is 0600 in a directory only this user
//! can write, and each connection's peer uid must equal ours. Any process running as this user
//! can still connect, exactly as it could run `kibitzer mcp` itself. Each MCP session gets its
//! own `KibitzerServer` (and so its own architecture-model cache slot), like a stdio process.

use std::os::unix::fs::{DirBuilderExt, FileTypeExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use axum::Router;
use axum::serve::Listener;
use rmcp::transport::streamable_http_server::session::local::{LocalSessionManager, SessionConfig};
use rmcp::transport::{StreamableHttpServerConfig, StreamableHttpService};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::watch;

use crate::daemon_lock::{accept_xdg_dir, current_uid, ensure_socket_dir, open_lock_file};
use crate::mcp::KibitzerServer;

pub const MCP_PATH: &str = "/mcp";

const OWNER_ONLY_DIR: u32 = 0o700;
const OWNER_ONLY_FILE: u32 = 0o600;

/// rmcp's default is no idle timeout. After this, an idle session's worker (and its
/// `KibitzerServer`) ends and rmcp drops it from the session map (see
/// `idle_session_should_LeaveSessionMap_When_KeepAliveExpires`); `--connect` re-initializes.
const SESSION_IDLE_TIMEOUT: Duration = Duration::from_secs(2 * 60 * 60);

/// Open SSE streams never finish on their own, so graceful shutdown would otherwise wait
/// on them forever.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

/// The socket `--socket`/`--connect` use: `path` if given, else the shared default.
pub fn socket_path_or_default(path: Option<PathBuf>) -> Result<PathBuf> {
    match path {
        Some(path) => Ok(path),
        None => default_socket_path_from(|name| std::env::var_os(name)),
    }
}

/// `$XDG_RUNTIME_DIR/kibitzer/mcp.sock` when that directory is trustworthy, else
/// `<cache dir>/kibitzer/mcp/mcp.sock`, the cache dir being `$XDG_CACHE_HOME` or
/// `$HOME/.cache`. Relative or empty values are skipped, so the default never lands under
/// the working directory; with no usable candidate it's an error.
fn default_socket_path_from(env: impl Fn(&str) -> Option<std::ffi::OsString>) -> Result<PathBuf> {
    let absolute = |name: &str| env(name).map(PathBuf::from).filter(|p| p.is_absolute());
    if let Some(Ok(dir)) = absolute("XDG_RUNTIME_DIR").map(|dir| accept_xdg_dir(&dir)) {
        return Ok(dir.join("kibitzer").join("mcp.sock"));
    }
    let cache = absolute("XDG_CACHE_HOME")
        .or_else(|| absolute("HOME").map(|home| home.join(".cache")))
        .context(
            "no default socket path: XDG_RUNTIME_DIR, XDG_CACHE_HOME and HOME are all unset, \
             empty or relative; pass a path explicitly",
        )?;
    Ok(cache.join("kibitzer").join("mcp").join("mcp.sock"))
}

/// Removes the socket when the server stops; the held lock keeps a second server off `path`.
struct SocketFile {
    path: PathBuf,
    _lock: std::fs::File,
}

impl Drop for SocketFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

pub(crate) struct BoundSocket {
    listener: UnixListener,
    file: SocketFile,
}

/// Binds `path` owner-only: 0700 parent, 0600 socket, never reachable by others even briefly
/// (it is bound in a private staging directory, chmodded, then renamed into place). Refuses a
/// path another server owns or answers on, or that isn't a socket; removes a stale socket.
pub(crate) fn bind_socket(path: &Path) -> Result<BoundSocket> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .with_context(|| format!("socket path {} has no parent directory", path.display()))?;
    check_path_fits(path)?;
    ensure_socket_dir(parent)
        .map_err(|why| anyhow::anyhow!("refusing socket directory {}: {why}", parent.display()))?;

    let lock_path = path.with_extension("lock");
    let lock = open_lock_file(&lock_path)
        .with_context(|| format!("opening lock {}", lock_path.display()))?;
    if lock.try_lock().is_err() {
        bail!(
            "another kibitzer MCP server already owns {}",
            path.display()
        );
    }

    remove_stale_socket(path)?;
    let listener = bind_owner_only(parent, path)?;
    listener.set_nonblocking(true)?;
    Ok(BoundSocket {
        listener: UnixListener::from_std(listener)?,
        file: SocketFile {
            path: path.to_path_buf(),
            _lock: lock,
        },
    })
}

/// Removes a socket nothing answers on; refuses a live one or a non-socket.
fn remove_stale_socket(path: &Path) -> Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if !meta.file_type().is_socket() => {
            bail!(
                "{} exists and is not a socket; refusing to replace it",
                path.display()
            )
        }
        Ok(_) if std::os::unix::net::UnixStream::connect(path).is_ok() => {
            bail!("a server is already answering on {}", path.display())
        }
        Ok(_) => std::fs::remove_file(path)
            .with_context(|| format!("removing stale socket {}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("checking {}", path.display())),
    }
}

/// Longest socket path `bind` accepts: `sun_path` (108 bytes on Linux, 104 on macOS and the
/// BSDs) less its terminating NUL.
const MAX_SOCKET_PATH: usize = if cfg!(target_os = "linux") { 107 } else { 103 };

fn check_path_fits(path: &Path) -> Result<()> {
    let len = path.as_os_str().len();
    if len > MAX_SOCKET_PATH {
        bail!(
            "socket path {} is {len} bytes, over the {MAX_SOCKET_PATH}-byte limit for Unix \
             sockets; pass a shorter path",
            path.display()
        );
    }
    Ok(())
}

/// Binds `path` so it's only ever reachable by this user. In a private (0700) `parent`
/// nobody else can reach it anyway, so it's bound in place; otherwise (e.g. 0755) it's
/// bound in a fresh 0700 staging directory and chmodded before being renamed into place.
fn bind_owner_only(parent: &Path, path: &Path) -> Result<std::os::unix::net::UnixListener> {
    use std::os::unix::fs::MetadataExt;
    let parent_mode = std::fs::metadata(parent)
        .with_context(|| format!("checking {}", parent.display()))?
        .mode();
    if parent_mode & 0o077 == 0 {
        let listener = std::os::unix::net::UnixListener::bind(path)
            .with_context(|| format!("binding {}", path.display()))?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(OWNER_ONLY_FILE))?;
        return Ok(listener);
    }
    bind_via_private_staging(parent, path)
}

fn bind_via_private_staging(
    parent: &Path,
    path: &Path,
) -> Result<std::os::unix::net::UnixListener> {
    // Short names, since the staged path must fit the same length limit as the final one.
    let staging = parent.join(format!(".k{}", std::process::id()));
    let staged = staging.join("s");
    if staged.as_os_str().len() > MAX_SOCKET_PATH {
        bail!(
            "socket path {} is too long to stage safely in {}, which others can read; use a \
             0700 directory or a shorter path",
            path.display(),
            parent.display()
        );
    }
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::DirBuilder::new()
        .mode(OWNER_ONLY_DIR)
        .create(&staging)
        .with_context(|| format!("creating {}", staging.display()))?;
    let bound = std::os::unix::net::UnixListener::bind(&staged)
        .and_then(|l| {
            std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(OWNER_ONLY_FILE))?;
            std::fs::rename(&staged, path)?;
            Ok(l)
        })
        .with_context(|| format!("binding {}", path.display()));
    let _ = std::fs::remove_dir_all(&staging);
    bound
}

/// Accepts only connections whose peer runs as `uid`; others are dropped unanswered.
struct SameUidListener {
    inner: UnixListener,
    uid: u32,
}

impl Listener for SameUidListener {
    type Io = UnixStream;
    type Addr = tokio::net::unix::SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            let (stream, addr) = Listener::accept(&mut self.inner).await;
            match stream.peer_cred() {
                Ok(cred) if cred.uid() == self.uid => return (stream, addr),
                Ok(cred) => eprintln!(
                    "[kibitzer] rejected MCP socket connection from uid {}",
                    cred.uid()
                ),
                Err(e) => eprintln!("[kibitzer] rejected MCP socket connection: {e}"),
            }
        }
    }

    fn local_addr(&self) -> std::io::Result<Self::Addr> {
        self.inner.local_addr()
    }
}

pub async fn run_mcp_socket_server(path: PathBuf) -> Result<()> {
    let bound = bind_socket(&path)?;
    eprintln!(
        "[kibitzer] MCP server listening on unix socket {} (streamable HTTP at {MCP_PATH})",
        path.display()
    );
    let (tx, rx) = watch::channel(false);
    tokio::spawn(async move {
        wait_for_termination_signal().await;
        eprintln!("[kibitzer] shutting down MCP server");
        let _ = tx.send(true);
    });
    serve(
        bound,
        rx,
        session_manager(SESSION_IDLE_TIMEOUT),
        current_uid(),
    )
    .await
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

fn session_manager(idle_timeout: Duration) -> Arc<LocalSessionManager> {
    Arc::new(LocalSessionManager {
        sessions: Default::default(),
        session_config: SessionConfig {
            keep_alive: Some(idle_timeout),
            ..Default::default()
        },
    })
}

/// Serves until `shutdown` flips to true, drains for at most `SHUTDOWN_GRACE`, then removes
/// the socket.
async fn serve(
    bound: BoundSocket,
    shutdown: watch::Receiver<bool>,
    sessions: Arc<LocalSessionManager>,
    allowed_uid: u32,
) -> Result<()> {
    let BoundSocket { listener, file } = bound;
    let service = StreamableHttpService::new(
        || Ok(KibitzerServer::new()),
        sessions,
        StreamableHttpServerConfig::default(),
    );
    let app = Router::new().route_service(MCP_PATH, service);
    let listener = SameUidListener {
        inner: listener,
        uid: allowed_uid,
    };
    let server =
        axum::serve(listener, app).with_graceful_shutdown(until_shutdown(shutdown.clone()));
    let result = tokio::select! {
        result = server => result.context("MCP server failed"),
        _ = async {
            until_shutdown(shutdown).await;
            tokio::time::sleep(SHUTDOWN_GRACE).await;
        } => Ok(()),
    };
    drop(file);
    result
}

async fn until_shutdown(mut shutdown: watch::Receiver<bool>) {
    if shutdown.wait_for(|stop| *stop).await.is_err() {
        // Sender dropped without signalling: never shut down on that alone.
        std::future::pending::<()>().await;
    }
}

#[cfg(test)]
#[allow(non_snake_case)]
pub(crate) mod tests {
    use std::os::unix::fs::MetadataExt;

    use super::*;
    use crate::test_support::unique_temp_dir;

    pub(crate) struct TestServer {
        pub path: PathBuf,
        pub sessions: Arc<LocalSessionManager>,
        stop: watch::Sender<bool>,
        task: tokio::task::JoinHandle<Result<()>>,
    }

    impl TestServer {
        pub(crate) async fn start(dir: &Path, idle: Duration, allowed_uid: u32) -> Self {
            let path = dir.join("run").join("mcp.sock");
            let bound = bind_socket(&path).unwrap();
            let sessions = session_manager(idle);
            let (stop, rx) = watch::channel(false);
            let task = tokio::spawn(serve(bound, rx, Arc::clone(&sessions), allowed_uid));
            Self {
                path,
                sessions,
                stop,
                task,
            }
        }

        pub(crate) async fn stop(self) {
            self.stop.send(true).unwrap();
            self.task.await.unwrap().unwrap();
        }
    }

    fn mode(path: &Path) -> u32 {
        std::fs::symlink_metadata(path).unwrap().mode() & 0o777
    }

    fn default_with(vars: &[(&str, &str)]) -> Result<PathBuf> {
        let vars: Vec<(String, String)> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        default_socket_path_from(|name| vars.iter().find(|(k, _)| k == name).map(|(_, v)| v.into()))
    }

    #[test]
    fn default_socket_path_should_UseXdgRuntimeDir_When_Trusted() {
        let dir = unique_temp_dir("mcpsock-xdg");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(OWNER_ONLY_DIR)).unwrap();
        let got = default_with(&[("XDG_RUNTIME_DIR", dir.to_str().unwrap()), ("HOME", "/h")]);
        assert_eq!(got.unwrap(), dir.join("kibitzer").join("mcp.sock"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn default_socket_path_should_FallBackToCacheDir_When_XdgMissingUntrustedOrRelative() {
        let want = PathBuf::from("/h/.cache/kibitzer/mcp/mcp.sock");
        assert_eq!(default_with(&[("HOME", "/h")]).unwrap(), want);
        let dir = unique_temp_dir("mcpsock-xdg-loose");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).unwrap();
        let loose = dir.to_str().unwrap();
        assert_eq!(
            default_with(&[("XDG_RUNTIME_DIR", loose), ("HOME", "/h")]).unwrap(),
            want
        );
        assert_eq!(
            default_with(&[("XDG_RUNTIME_DIR", "run"), ("HOME", "/h")]).unwrap(),
            want
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn default_socket_path_should_SkipEmptyOrRelativeCacheVars_And_ErrorWithNoCandidate() {
        assert_eq!(
            default_with(&[("XDG_CACHE_HOME", "/c"), ("HOME", "/h")]).unwrap(),
            PathBuf::from("/c/kibitzer/mcp/mcp.sock")
        );
        for cache in ["", "cache"] {
            assert_eq!(
                default_with(&[("XDG_CACHE_HOME", cache), ("HOME", "/h")]).unwrap(),
                PathBuf::from("/h/.cache/kibitzer/mcp/mcp.sock"),
                "XDG_CACHE_HOME={cache:?}"
            );
        }
        for vars in [
            &[][..],
            &[("HOME", "")],
            &[("HOME", "home"), ("XDG_RUNTIME_DIR", "run")],
        ] {
            let err = default_with(vars).unwrap_err();
            assert!(err.to_string().contains("no default socket path"), "{err}");
        }
        assert_eq!(
            socket_path_or_default(Some(PathBuf::from("/x/s.sock"))).unwrap(),
            PathBuf::from("/x/s.sock")
        );
    }

    #[tokio::test]
    async fn bind_socket_should_CreatePrivateDirAndOwnerOnlySocket() {
        let dir = unique_temp_dir("mcpsock-perm");
        let path = dir.join("run").join("mcp.sock");
        let bound = bind_socket(&path).unwrap();
        assert_eq!(mode(path.parent().unwrap()), OWNER_ONLY_DIR);
        assert_eq!(mode(&path), OWNER_ONLY_FILE);
        assert!(
            std::fs::symlink_metadata(&path)
                .unwrap()
                .file_type()
                .is_socket()
        );
        // Nothing left behind from staging.
        let leftovers: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .filter(|n| n.to_string_lossy().starts_with(".k"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
        drop(bound);
        assert!(!path.exists(), "dropping the bound socket unlinks it");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn bind_socket_should_RefuseSecondOwner_And_LiveServer() {
        let dir = unique_temp_dir("mcpsock-live");
        let path = dir.join("run").join("mcp.sock");
        let _first = bind_socket(&path).unwrap();
        let err = bind_socket(&path).err().unwrap();
        assert!(err.to_string().contains("already owns"), "{err}");

        // A socket something answers on, without our lock (e.g. a pre-lock server).
        let other = dir.join("other").join("mcp.sock");
        std::fs::create_dir_all(other.parent().unwrap()).unwrap();
        let _live = std::os::unix::net::UnixListener::bind(&other).unwrap();
        let err = bind_socket(&other).err().unwrap();
        assert!(err.to_string().contains("already answering"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn bind_socket_should_ReplaceStaleSocket_And_RefuseNonSocket() {
        let dir = unique_temp_dir("mcpsock-stale");
        let path = dir.join("run").join("mcp.sock");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        drop(std::os::unix::net::UnixListener::bind(&path).unwrap());
        assert!(
            path.exists(),
            "a dropped std listener leaves its socket file"
        );
        let bound = bind_socket(&path).unwrap();
        assert_eq!(mode(&path), OWNER_ONLY_FILE);
        drop(bound);

        std::fs::write(&path, "not a socket").unwrap();
        let err = bind_socket(&path).err().unwrap();
        assert!(err.to_string().contains("not a socket"), "{err}");
        assert!(path.exists(), "a non-socket file is never removed");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn bind_socket_should_StageOwnerOnlySocket_When_DirIsReadableByOthers() {
        let dir = unique_temp_dir("mcpsock-0755");
        let run = dir.join("run");
        std::fs::create_dir(&run).unwrap();
        std::fs::set_permissions(&run, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = run.join("mcp.sock");
        let bound = bind_socket(&path).unwrap();
        assert_eq!(mode(&path), OWNER_ONLY_FILE);
        assert_eq!(
            mode(&run),
            0o755,
            "a directory we didn't make is left alone"
        );
        let leftovers = std::fs::read_dir(&run).unwrap().count();
        assert_eq!(leftovers, 2, "only the socket and its lock remain");
        drop(bound);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `dir/<padding>/s.sock`, exactly `len` bytes long.
    fn path_of_len(dir: &Path, len: usize) -> PathBuf {
        let base = dir.as_os_str().len() + "/".len() + "/s.sock".len();
        let path = dir.join("p".repeat(len - base)).join("s.sock");
        assert_eq!(path.as_os_str().len(), len);
        path
    }

    #[tokio::test]
    async fn bind_socket_should_BindPathAtTheLimit_And_RefuseLongerWithClearError() {
        let dir = PathBuf::from(format!("/tmp/kbz-len-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let at_limit = path_of_len(&dir, MAX_SOCKET_PATH);
        let bound = bind_socket(&at_limit).unwrap();
        assert_eq!(mode(&at_limit), OWNER_ONLY_FILE);
        drop(bound);
        let near = path_of_len(&dir, 100);
        drop(bind_socket(&near).unwrap());
        let too_long = path_of_len(&dir, MAX_SOCKET_PATH + 1);
        let err = bind_socket(&too_long).err().unwrap();
        assert!(err.to_string().contains("byte limit"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn bind_socket_should_RefuseDirWritableByOthers() {
        let dir = unique_temp_dir("mcpsock-loose");
        let run = dir.join("run");
        std::fs::create_dir(&run).unwrap();
        std::fs::set_permissions(&run, std::fs::Permissions::from_mode(0o777)).unwrap();
        let err = bind_socket(&run.join("mcp.sock")).err().unwrap();
        assert!(
            err.to_string().contains("refusing socket directory"),
            "{err}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn serve_should_DropConnection_When_PeerUidDiffers() {
        let dir = unique_temp_dir("mcpsock-uid");
        let server = TestServer::start(&dir, SESSION_IDLE_TIMEOUT, current_uid() + 1).await;
        let reply = crate::mcp_bridge::tests::initialize_raw(&server.path).await;
        assert!(
            reply.is_err(),
            "a foreign-uid peer must get no answer: {reply:?}"
        );
        server.stop().await;
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn serve_should_AnswerInitialize_And_UnlinkSocketOnShutdown() {
        let dir = unique_temp_dir("mcpsock-serve");
        let server = TestServer::start(&dir, SESSION_IDLE_TIMEOUT, current_uid()).await;
        let path = server.path.clone();
        let session = crate::mcp_bridge::tests::initialize_raw(&path)
            .await
            .unwrap();
        assert!(!session.is_empty());
        assert_eq!(server.sessions.sessions.read().await.len(), 1);
        server.stop().await;
        assert!(!path.exists(), "shutdown unlinks the socket");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn idle_session_should_LeaveSessionMap_When_KeepAliveExpires() {
        let dir = unique_temp_dir("mcpsock-idle");
        let server = TestServer::start(&dir, Duration::from_millis(200), current_uid()).await;
        crate::mcp_bridge::tests::initialize_raw(&server.path)
            .await
            .unwrap();
        assert_eq!(server.sessions.sessions.read().await.len(), 1);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while !server.sessions.sessions.read().await.is_empty() {
            assert!(
                tokio::time::Instant::now() < deadline,
                "idle session never left the map"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        server.stop().await;
        let _ = std::fs::remove_dir_all(&dir);
    }
}
