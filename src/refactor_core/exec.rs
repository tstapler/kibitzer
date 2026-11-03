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
            let mut chunk = [0u8; 8192];
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
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
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

    #[test]
    fn timeout_kills_the_command_and_its_descendants_quickly() {
        let pidfile =
            std::env::temp_dir().join(format!("kibitzer-exec-pid-{}", std::process::id()));
        let _ = std::fs::remove_file(&pidfile);
        let started = Instant::now();
        let o = sh(
            &format!("sleep 60 & echo $! > {}; wait", pidfile.display()),
            Duration::from_millis(300),
            EnvPolicy::Inherit,
        );
        assert!(
            started.elapsed() < Duration::from_secs(15),
            "{:?}",
            started.elapsed()
        );
        assert_eq!(o.exit_code, None);
        assert!(o.output.contains("timed out"), "{}", o.output);
        let grandchild: i32 = std::fs::read_to_string(&pidfile)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        // The group kill is synchronous, but give the kernel a moment to reap.
        let mut gone = false;
        for _ in 0..50 {
            // SAFETY: signal 0 only probes for existence.
            if unsafe { libc::kill(grandchild, 0) } == -1 {
                gone = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(gone, "grandchild {grandchild} survived the timeout");
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
    fn a_descendant_holding_the_pipe_open_cannot_hang_the_runner() {
        let started = Instant::now();
        // The shell exits at once; the background sleep keeps stdout open for 5 s.
        let o = quick("sleep 5 & echo done");
        assert!(
            started.elapsed() < Duration::from_secs(4),
            "{:?}",
            started.elapsed()
        );
        assert_eq!(o.exit_code, Some(0));
        assert!(o.output.contains("done"), "{}", o.output);
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
