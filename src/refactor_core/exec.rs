//! Bounded command execution: no shell unless asked, a wall-clock timeout that kills the whole
//! process group, capped output, and a child that dies with its parent. Used for
//! `go build/vet/test` and the operator's hook.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Per-stream cap; the report says so when it truncates.
pub const MAX_OUTPUT_BYTES: usize = 1024 * 1024;

/// After the child exits, how long to wait for its output pipes to close. A descendant that
/// outlives it (`server &` in a hook) holds them open; we report what we have instead of hanging.
const DRAIN_GRACE: Duration = Duration::from_secs(2);

/// Read buffer size for each captured stream.
const READ_CHUNK_BYTES: usize = 8192;

/// How often [`wait_bounded`] checks whether the child has exited.
const POLL_INTERVAL: Duration = Duration::from_millis(20);

/// What the child inherits from the parent's environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvPolicy {
    /// The operator's own environment (CLI): a pinned `GOTOOLCHAIN`/`GOFLAGS`/`GOWORK` is theirs.
    Inherit,
    /// Untrusted plan source (MCP): `GOTOOLCHAIN=local` stops a repo's go.mod from downloading
    /// and running another toolchain; `-mod=readonly` stops the build rewriting go.mod/go.sum.
    /// `GOWORK` is left alone: a `go.work` workspace is a legitimate way to resolve sibling modules.
    Locked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub command: String,
    /// `None` when the process never started, timed out, or was killed by a signal.
    pub exit_code: Option<i32>,
    /// stdout then stderr, each capped at [`MAX_OUTPUT_BYTES`].
    pub output: String,
}

impl Outcome {
    pub fn ok(&self) -> bool {
        self.exit_code == Some(0)
    }
}

pub struct Spec<'a> {
    pub program: &'a str,
    pub args: &'a [String],
    pub dir: &'a Path,
    pub timeout: Duration,
    pub env: EnvPolicy,
}

fn display(program: &str, args: &[String]) -> String {
    std::iter::once(program)
        .chain(args.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(" ")
}

/// A reader thread filling a shared buffer. The buffer stays readable even if the thread never
/// sees EOF.
struct Capture {
    buf: Arc<Mutex<(Vec<u8>, bool)>>,
    done: Receiver<()>,
}

fn capture(stream: Option<impl Read + Send + 'static>) -> Capture {
    let buf = Arc::new(Mutex::new((Vec::new(), false)));
    let (tx, done) = channel();
    let shared = Arc::clone(&buf);
    std::thread::spawn(move || {
        if let Some(mut s) = stream {
            let mut chunk = [0u8; READ_CHUNK_BYTES];
            // Keep reading past the cap so a chatty child never blocks on a full pipe.
            while let Ok(n) = s.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                let mut g = shared
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let room = MAX_OUTPUT_BYTES.saturating_sub(g.0.len());
                g.0.extend_from_slice(&chunk[..n.min(room)]);
                g.1 |= n > room;
            }
        }
        let _ = tx.send(());
    });
    Capture { buf, done }
}

impl Capture {
    fn finish(self, name: &str, deadline: Instant) -> String {
        let wait = deadline.saturating_duration_since(Instant::now());
        let closed = self.done.recv_timeout(wait).is_ok();
        let g = self
            .buf
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut text = String::from_utf8_lossy(&g.0).into_owned();
        if g.1 {
            text.push_str("\n[output truncated at 1 MiB]\n");
        }
        if !closed {
            text.push_str(&format!(
                "\n[{name} still held open by a descendant process; later output dropped]\n"
            ));
        }
        text
    }
}

#[cfg(unix)]
fn kill_group(child: &mut std::process::Child) {
    // The child leads its own group (see `process_group(0)`), so this reaches its descendants too.
    let pid = child.id() as i32;
    // SAFETY: `kill` takes plain integers and has no memory-safety preconditions.
    unsafe {
        libc::kill(-pid, libc::SIGKILL);
    }
    let _ = child.kill();
}

#[cfg(not(unix))]
fn kill_group(child: &mut std::process::Child) {
    let _ = child.kill();
}

pub fn run(spec: &Spec) -> Outcome {
    let command = display(spec.program, spec.args);
    let mut cmd = Command::new(spec.program);
    cmd.args(spec.args)
        .current_dir(spec.dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if spec.env == EnvPolicy::Locked {
        cmd.env("GOTOOLCHAIN", "local")
            .env("GOFLAGS", "-mod=readonly");
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        cmd.process_group(0);
        #[cfg(target_os = "linux")]
        // SAFETY: only async-signal-safe `prctl` runs between fork and exec. The child gets
        // SIGKILL if this thread (which waits for it below) dies, so a crashed or killed parent
        // does not leave `go test` running against a half-applied tree.
        unsafe {
            cmd.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
                Ok(())
            });
        }
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return Outcome {
                command,
                exit_code: None,
                output: format!("failed to run {}: {e}", spec.program),
            };
        }
    };
    let out = capture(child.stdout.take());
    let err = capture(child.stderr.take());
    let (code, note) = wait_bounded(&mut child, spec.timeout, &command);
    // Descendants (`server &` in a hook) must not outlive the command or hold its pipes; killing an
    // already-empty group is harmless.
    kill_group(&mut child);
    // One grace period for both streams, not one each.
    let deadline = Instant::now() + DRAIN_GRACE;
    let mut output = out.finish("stdout", deadline);
    output.push_str(&err.finish("stderr", deadline));
    output.push_str(&note);
    Outcome {
        command,
        exit_code: code,
        output,
    }
}

fn wait_bounded(
    child: &mut std::process::Child,
    timeout: Duration,
    command: &str,
) -> (Option<i32>, String) {
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(st)) => return (st.code(), String::new()),
            Ok(None) if start.elapsed() > timeout => {
                kill_group(child);
                let _ = child.wait();
                let note = format!(
                    "\n`{command}` timed out after {}s and was killed\n",
                    timeout.as_secs()
                );
                return (None, note);
            }
            Ok(None) => std::thread::sleep(POLL_INTERVAL),
            Err(e) => return (None, format!("\nwaiting for `{command}`: {e}\n")),
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn sh(script: &str, timeout: Duration, env: EnvPolicy) -> Outcome {
        let args = vec!["-c".to_string(), script.to_string()];
        run(&Spec {
            program: "sh",
            args: &args,
            dir: Path::new("/"),
            timeout,
            env,
        })
    }

    fn quick(script: &str) -> Outcome {
        sh(script, Duration::from_secs(20), EnvPolicy::Inherit)
    }

    #[test]
    fn reports_exit_code_and_both_streams() {
        let o = quick("echo out; echo err >&2; exit 3");
        assert_eq!(o.exit_code, Some(3));
        assert!(!o.ok());
        assert!(
            o.output.contains("out") && o.output.contains("err"),
            "{}",
            o.output
        );
        assert!(quick("exit 0").ok());
    }

    #[test]
    fn a_missing_program_is_an_outcome_not_a_panic() {
        let o = run(&Spec {
            program: "definitely-not-a-program-xyz",
            args: &[],
            dir: Path::new("/"),
            timeout: Duration::from_secs(5),
            env: EnvPolicy::Inherit,
        });
        assert_eq!(o.exit_code, None);
        assert!(o.output.contains("failed to run"), "{}", o.output);
    }

    fn process_exists(pid: i32) -> bool {
        // SAFETY: signal 0 only probes for existence.
        unsafe { libc::kill(pid, 0) != -1 }
    }

    fn wait_until(limit: Duration, mut cond: impl FnMut() -> bool) -> bool {
        let start = Instant::now();
        while start.elapsed() < limit {
            if cond() {
                return true;
            }
            std::thread::sleep(POLL_INTERVAL);
        }
        cond()
    }

    #[test]
    fn timeout_kills_the_command_and_its_descendants_quickly() {
        let pidfile =
            std::env::temp_dir().join(format!("kibitzer-exec-pid-{}", std::process::id()));
        let _ = std::fs::remove_file(&pidfile);
        let started = Instant::now();
        let o = sh(
            &format!("sleep 60 & echo $! > {}; wait", pidfile.display()),
            Duration::from_secs(2),
            EnvPolicy::Inherit,
        );
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "{:?}",
            started.elapsed()
        );
        assert_eq!(o.exit_code, None);
        assert!(o.output.contains("timed out"), "{}", o.output);
        let read_pid = || {
            std::fs::read_to_string(&pidfile)
                .ok()
                .and_then(|t| t.trim().parse::<i32>().ok())
        };
        assert!(
            wait_until(Duration::from_secs(10), || read_pid().is_some()),
            "the script never recorded its background pid"
        );
        let grandchild = read_pid().unwrap();
        assert!(
            wait_until(Duration::from_secs(5), || !process_exists(grandchild)),
            "grandchild {grandchild} survived the timeout"
        );
        let _ = std::fs::remove_file(&pidfile);
    }

    #[test]
    fn output_is_capped_with_a_note_and_the_child_is_not_blocked() {
        let o = quick("head -c 3000000 /dev/zero | tr '\\0' a");
        assert_eq!(
            o.exit_code,
            Some(0),
            "a chatty child must finish, not stall on a full pipe"
        );
        assert!(
            o.output.len() < MAX_OUTPUT_BYTES + 200,
            "{}",
            o.output.len()
        );
        assert!(
            o.output.contains("output truncated"),
            "missing truncation note"
        );
    }

    #[test]
    fn a_descendant_of_a_child_that_exited_is_killed_and_cannot_hang_the_runner() {
        let started = Instant::now();
        // The shell exits at once; the background sleep would keep stdout open for 30 s.
        let o = quick("sleep 30 & echo done");
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "{:?}",
            started.elapsed()
        );
        assert_eq!(o.exit_code, Some(0));
        assert!(o.output.contains("done"), "{}", o.output);
        assert!(!o.output.contains("still held open"), "{}", o.output);
    }

    #[test]
    fn a_descendant_that_left_the_group_is_reported_not_waited_for() {
        if Command::new("setsid").arg("--version").output().is_err() {
            return;
        }
        let started = Instant::now();
        // The pause lets `setsid` leave the group before the shell exits; otherwise the group
        // kill can reach `sleep` first and the pipe closes, so nothing is "still held open".
        let o = quick("setsid sleep 30 & sleep 0.5; echo done");
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "{:?}",
            started.elapsed()
        );
        assert_eq!(o.exit_code, Some(0));
        assert!(o.output.contains("still held open"), "{}", o.output);
    }

    #[test]
    fn locked_env_pins_the_toolchain_but_leaves_go_work_alone() {
        let o = sh(
            "echo \"$GOTOOLCHAIN $GOFLAGS [${GOWORK-unset}]\"",
            Duration::from_secs(5),
            EnvPolicy::Locked,
        );
        assert!(o.output.contains("local -mod=readonly"), "{}", o.output);
        if std::env::var_os("GOWORK").is_none() {
            assert!(
                o.output.contains("[unset]"),
                "Locked must not force GOWORK: {}",
                o.output
            );
        }
    }

    #[test]
    fn inherit_env_passes_the_operators_settings_through() {
        // PATH is always set; a stand-in for "the child sees the parent's environment".
        let o = quick("test -n \"$PATH\" && echo has-path");
        assert!(o.output.contains("has-path"), "{}", o.output);
    }
}
