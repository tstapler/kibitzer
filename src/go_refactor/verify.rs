//! Post-apply verification: `go build`, `go vet`, `go test`, captured verbatim.

use std::path::Path;
use std::process::Command;

use super::types::{CommandOutcome, VerifyResult};

fn run(root: &Path, program: &str, args: &[String]) -> CommandOutcome {
    let command = format!("{program} {}", args.join(" "));
    match Command::new(program).args(args).current_dir(root).output() {
        Ok(out) => CommandOutcome {
            command,
            exit_code: out.status.code(),
            output: format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            ),
        },
        Err(e) => CommandOutcome {
            command,
            exit_code: None,
            output: format!("failed to run {program}: {e}"),
        },
    }
}

/// Above this many packages the argument list is replaced by `./...`.
const MAX_EXPLICIT_PACKAGES: usize = 20;

/// Default is the whole module (`./...`); `affected_only` narrows to the plan's packages.
pub fn run_verify(
    root: &Path,
    affected: &[String],
    affected_only: bool,
    post_apply: Option<&str>,
) -> VerifyResult {
    let targets: Vec<String> =
        if !affected_only || affected.is_empty() || affected.len() > MAX_EXPLICIT_PACKAGES {
            vec!["./...".to_string()]
        } else {
            affected.to_vec()
        };
    let hook = post_apply.map(|cmd| run(root, "sh", &["-c".to_string(), cmd.to_string()]));
    let with = |head: &[&str]| -> Vec<String> {
        head.iter()
            .map(|s| s.to_string())
            .chain(targets.iter().cloned())
            .collect()
    };
    // The hook output is part of the verbatim report; a failing hook skips the Go commands.
    let skipped = |name: &str| CommandOutcome {
        command: name.into(),
        exit_code: None,
        output: "skipped: post-apply hook failed".into(),
    };
    if hook.as_ref().is_some_and(|h| !h.ok()) {
        return VerifyResult {
            post_apply_hook: hook,
            build: skipped("go build"),
            vet: skipped("go vet"),
            test: skipped("go test"),
        };
    }
    VerifyResult {
        post_apply_hook: hook,
        build: run(root, "go", &with(&["build"])),
        vet: run(root, "go", &with(&["vet"])),
        test: run(root, "go", &with(&["test", "-count=1"])),
    }
}
