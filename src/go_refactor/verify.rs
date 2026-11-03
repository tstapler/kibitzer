//! Post-apply verification: `go build`, `go vet`, `go test`, captured verbatim (each stream
//! capped at 1 MiB, with a note when it is) under a wall-clock timeout.

use std::path::Path;
use std::time::Duration;

use super::types::{CommandOutcome, VerifyResult};
use crate::refactor_core::exec::{self, EnvPolicy};

/// Default budget per command; `go test ./...` on a large module is slow, not hung.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30 * 60);

fn run(root: &Path, program: &str, args: &[String], timeout: Duration) -> CommandOutcome {
    let out = exec::run(&exec::Spec {
        program,
        args,
        dir: root,
        timeout,
        // The operator runs this CLI on their own checkout and may pin GOTOOLCHAIN/GOFLAGS.
        env: EnvPolicy::Inherit,
    });
    CommandOutcome {
        command: out.command,
        exit_code: out.exit_code,
        output: out.output,
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
    timeout: Duration,
) -> VerifyResult {
    let targets: Vec<String> =
        if !affected_only || affected.is_empty() || affected.len() > MAX_EXPLICIT_PACKAGES {
            vec!["./...".to_string()]
        } else {
            affected.to_vec()
        };
    let hook = post_apply.map(|cmd| run(root, "sh", &["-c".to_string(), cmd.to_string()], timeout));
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
        build: run(root, "go", &with(&["build"]), timeout),
        vet: run(root, "go", &with(&["vet"]), timeout),
        test: run(root, "go", &with(&["test", "-count=1"]), timeout),
    }
}
