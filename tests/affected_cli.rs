//! CLI-contract tests for `kibitzer architecture affected`, spawning the real built
//! binary against fixture git repos — matching `hook_contract.rs`/
//! `false_positives_cli.rs`'s convention of testing the actual stdin/stdout/exit-code
//! contract a caller (here, a CI wrapper) relies on, not just the underlying library
//! function.

use std::path::PathBuf;
use std::process::Command;

struct TempRepo {
    dir: PathBuf,
}

impl TempRepo {
    fn new(name: &str) -> Self {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let unique = format!(
            "{}-{name}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );
        let dir = std::env::temp_dir().join(format!("kibitzer-affected-cli-{unique}"));
        std::fs::create_dir_all(&dir).unwrap();
        Self::run(&dir, &["init", "-q", "-b", "main"]);
        Self::run(&dir, &["config", "user.email", "test@example.com"]);
        Self::run(&dir, &["config", "user.name", "test"]);
        Self { dir }
    }

    fn run(dir: &PathBuf, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(dir)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?} failed");
    }

    fn write(&self, rel: &str, contents: &str) {
        let path = self.dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, contents).unwrap();
    }

    fn commit_all(&self, msg: &str) {
        Self::run(&self.dir, &["add", "-A"]);
        Self::run(&self.dir, &["commit", "-q", "-m", msg]);
    }

    fn head_sha(&self) -> String {
        let output = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&self.dir)
            .output()
            .unwrap();
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    /// Runs `kibitzer architecture affected --path <self> --base <base>` as a real
    /// subprocess, returning (exit_code, stdout, stderr).
    fn run_affected(&self, base: &str) -> (i32, String, String) {
        let output = Command::new(env!("CARGO_BIN_EXE_kibitzer"))
            .args([
                "architecture",
                "affected",
                "--path",
                self.dir.to_str().unwrap(),
                "--base",
                base,
            ])
            .output()
            .unwrap();
        (
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stdout).to_string(),
            String::from_utf8_lossy(&output.stderr).to_string(),
        )
    }
}

impl Drop for TempRepo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn affected_subcommand_help_lists_path_and_base_flags_and_the_ci_wrapper_guard() {
    let output = Command::new(env!("CARGO_BIN_EXE_kibitzer"))
        .args(["architecture", "affected", "--help"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("--base <BASE>"), "got: {stdout}");
    assert!(stdout.contains("--path <PATH>"), "got: {stdout}");
    assert!(stdout.contains("[ -z \"$PKGS\" ]"), "got: {stdout}");
}

#[test]
fn affected_subcommand_without_required_base_flag_exits_nonzero_with_usage_error() {
    let output = Command::new(env!("CARGO_BIN_EXE_kibitzer"))
        .args(["architecture", "affected"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).is_empty());
    assert!(!String::from_utf8_lossy(&output.stderr).is_empty());
}

#[test]
fn affected_cli_prints_newline_separated_package_list_and_exits_success() {
    let repo = TempRepo::new("list");
    repo.write("go.mod", "module example.com/app\n\ngo 1.21\n");
    repo.write("widget/widget.go", "package widget\n\nfunc W() {}\n");
    repo.commit_all("init");
    let base = repo.head_sha();

    repo.write(
        "widget/widget.go",
        "package widget\n\nfunc W() { println(1) }\n",
    );
    repo.commit_all("edit widget");

    let (code, stdout, _stderr) = repo.run_affected(&base);

    assert_eq!(code, 0);
    assert_eq!(stdout, "example.com/app/widget\n");
}

#[test]
fn affected_cli_prints_all_sentinel_to_stdout_and_reason_to_stderr_on_bail_out() {
    let repo = TempRepo::new("bailout");
    repo.write("go.mod", "module example.com/app\n\ngo 1.21\n");
    repo.write("a/a.go", "package a\n\nfunc A() {}\n");
    repo.commit_all("init");
    let base = repo.head_sha();

    repo.write("go.mod", "module example.com/app\n\ngo 1.22\n");
    repo.commit_all("bump go.mod");

    let (code, stdout, stderr) = repo.run_affected(&base);

    assert_eq!(code, 0);
    assert_eq!(stdout, "__ALL__\n");
    assert!(stderr.contains("go.mod"), "got: {stderr}");
}

/// Confirms `run_affected`'s `config::find_config` branch (main.rs's `Some((config,
/// root))` arm) actually takes effect end-to-end: `Makefile` isn't one of
/// `default_bail_out_globs`, so this bail-out can only fire if the repo's own
/// `.claude/inspect.json` `affected.extra_bail_out_globs` was loaded and consulted.
#[test]
fn affected_cli_bails_out_on_an_extra_glob_configured_in_claude_inspect_json() {
    let repo = TempRepo::new("extra-glob-config");
    repo.write("go.mod", "module example.com/app\n\ngo 1.21\n");
    repo.write("a/a.go", "package a\n\nfunc A() {}\n");
    repo.write(
        ".claude/inspect.json",
        r#"{"affected": {"extra_bail_out_globs": ["Makefile"]}}"#,
    );
    repo.commit_all("init");
    let base = repo.head_sha();

    repo.write("Makefile", "build:\n\techo hi\n");
    repo.write("a/a.go", "package a\n\nfunc A() { println(1) }\n");
    repo.commit_all("touch Makefile");

    let (code, stdout, stderr) = repo.run_affected(&base);

    assert_eq!(code, 0);
    assert_eq!(stdout, "__ALL__\n");
    assert!(
        stderr.contains("Makefile"),
        "expected the .claude/inspect.json extra_bail_out_globs entry to drive the \
         bail-out, got: {stderr}"
    );
}

#[test]
fn affected_cli_prints_empty_stdout_with_stderr_note_when_nothing_affected() {
    let repo = TempRepo::new("empty");
    repo.write("go.mod", "module example.com/app\n\ngo 1.21\n");
    repo.write("a/a.go", "package a\n\nfunc A() {}\n");
    repo.commit_all("init");
    let base = repo.head_sha();

    let (code, stdout, stderr) = repo.run_affected(&base);

    assert_eq!(code, 0);
    assert_eq!(stdout, "");
    assert!(stderr.contains("AFFECTED: 0 packages"), "got: {stderr}");
}

#[test]
fn affected_cli_exits_zero_for_bail_out_and_nonzero_for_an_unresolvable_base_ref() {
    let repo = TempRepo::new("contrast");
    repo.write("go.mod", "module example.com/app\n\ngo 1.21\n");
    repo.write("a/a.go", "package a\n\nfunc A() {}\n");
    repo.commit_all("init");
    let base = repo.head_sha();
    repo.write("go.mod", "module example.com/app\n\ngo 1.22\n");
    repo.commit_all("bump go.mod");

    let (bail_out_code, bail_out_stdout, _) = repo.run_affected(&base);
    let (err_code, err_stdout, err_stderr) = repo.run_affected("does-not-exist-ref");

    assert_eq!(bail_out_code, 0);
    assert_eq!(bail_out_stdout, "__ALL__\n");
    assert_ne!(err_code, 0);
    assert_eq!(err_stdout, "");
    assert!(
        err_stderr.contains("does-not-exist-ref"),
        "got: {err_stderr}"
    );
}

#[test]
fn affected_cli_exits_nonzero_with_empty_stdout_when_path_is_not_a_git_repository() {
    let dir = std::env::temp_dir().join(format!(
        "kibitzer-affected-cli-not-a-repo-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_kibitzer"))
        .args([
            "architecture",
            "affected",
            "--path",
            dir.to_str().unwrap(),
            "--base",
            "main",
        ])
        .output()
        .unwrap();

    std::fs::remove_dir_all(&dir).ok();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).is_empty());
}

/// The Risk Control section's Consumer Contract note, made concrete: the exact CI
/// shell shape (`PKGS="$(...)"; [ -z "$PKGS" ] && exit 0 || go test $PKGS`) reaches its
/// guard branch for a zero-affected-packages result, rather than silently degrading to
/// an unguarded `go test` with no package argument.
#[test]
fn empty_affected_output_reaches_the_ci_wrapper_guard_branch() {
    let repo = TempRepo::new("guard");
    repo.write("go.mod", "module example.com/app\n\ngo 1.21\n");
    repo.write("a/a.go", "package a\n\nfunc A() {}\n");
    repo.commit_all("init");
    let base = repo.head_sha();

    let (_, stdout, _) = repo.run_affected(&base);
    assert_eq!(stdout, "");

    let output = Command::new("sh")
        .args([
            "-c",
            r#"PKGS="$1"; [ -z "$PKGS" ] && echo SKIPPED || echo "go test $PKGS""#,
            "--",
            &stdout,
        ])
        .output()
        .unwrap();

    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "SKIPPED");
}
