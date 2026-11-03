//! Bounded command execution: no shell unless asked, a wall-clock timeout that kills the whole
//! process group, and capped output. Used for `go build/vet/test` and the operator's hook.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Per-stream cap; the report says so when it truncates.
pub const MAX_OUTPUT_BYTES: usize = 1024 * 1024;

/// What the child inherits from the parent's environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvPolicy {
    /// The operator's own environment (CLI): a pinned `GOTOOLCHAIN`/`GOFLAGS` is theirs to choose.
    Inherit,
    /// Untrusted plan source (MCP): `GOTOOLCHAIN=local` stops a repo's go.mod from downloading
    /// and running another toolchain; `-mod=readonly` and `GOWORK=off` stop the build rewriting
    /// or escaping the module.
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

fn drain(stream: Option<impl std::io::Read + Send + 'static>) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let Some(mut s) = stream else {
            return String::new();
        };
        let mut kept = Vec::new();
        let mut chunk = [0u8; 8192];
        let mut truncated = false;
        // Keep reading past the cap so a chatty child never blocks on a full pipe.
        while let Ok(n) = s.read(&mut chunk) {
            if n == 0 {
                break;
            }
            let room = MAX_OUTPUT_BYTES.saturating_sub(kept.len());
            kept.extend_from_slice(&chunk[..n.min(room)]);
            truncated |= n > room;
        }
        let mut text = String::from_utf8_lossy(&kept).into_owned();
        if truncated {
            text.push_str("\n[output truncated at 1 MiB]\n");
        }
        text
    })
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
            .env("GOFLAGS", "-mod=readonly")
            .env("GOWORK", "off");
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        cmd.process_group(0);
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
    let out = drain(child.stdout.take());
    let err = drain(child.stderr.take());
    let (code, note) = wait_bounded(&mut child, spec.timeout, &command);
    let mut output = out.join().unwrap_or_default();
    output.push_str(&err.join().unwrap_or_default());
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
