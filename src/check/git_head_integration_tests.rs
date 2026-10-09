use super::*;
use crate::config::Severity;

struct TempRepo {
    dir: PathBuf,
}

impl TempRepo {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "kibitzer-check-test-{}-{name}-{}",
            std::process::id(),
            TMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        run(&dir, &["init", "-q"]);
        run(&dir, &["config", "user.email", "test@example.com"]);
        run(&dir, &["config", "user.name", "test"]);
        Self { dir }
    }

    fn write_and_commit(&self, rel_path: &str, content: &str, msg: &str) {
        let path = self.dir.join(rel_path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, content).unwrap();
        run(&self.dir, &["add", rel_path]);
        run(&self.dir, &["commit", "-q", "-m", msg]);
    }

    fn write_uncommitted(&self, rel_path: &str, content: &str) {
        std::fs::write(self.dir.join(rel_path), content).unwrap();
    }

    fn path(&self, rel_path: &str) -> PathBuf {
        self.dir.join(rel_path)
    }
}

impl Drop for TempRepo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn run(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .expect("git available");
    assert!(status.success(), "git {args:?} failed");
}

/// A "check" that fails whenever the file contains the string "BAD".
fn bad_marker_check() -> Check {
    Check {
        name: "no-bad-marker".to_string(),
        command: Some("! grep -n BAD {file}".to_string()),
        checker: None,
        architecture_checker: None,
        severity: Severity::Blocking,
        scope: vec![],
        triggers: vec![],
        message: Some("found BAD marker".to_string()),
        output_format: None,
        options: None,
    }
}

/// A whole-repo counterpart to `bad_marker_check`: no `{file}` in the command, so it
/// scans the whole tree it's run from rather than a single file.
fn repo_wide_bad_marker_check() -> Check {
    Check {
        name: "no-bad-marker-repo".to_string(),
        command: Some("! grep -rn BAD .".to_string()),
        checker: None,
        architecture_checker: None,
        severity: Severity::Blocking,
        scope: vec![],
        triggers: vec![],
        message: Some("found BAD marker in repo".to_string()),
        output_format: None,
        options: None,
    }
}

#[test]
fn baseline_passes_when_violation_is_genuinely_new() {
    let repo = TempRepo::new("genuinely-new");
    repo.write_and_commit("foo.txt", "line1\nline2\nline3\n", "init");
    repo.write_uncommitted("foo.txt", "line1\nBAD\nline3\n");

    let result = check_against_git_head(
        &bad_marker_check(),
        &repo.dir,
        &repo.path("foo.txt"),
        Some(&[(2, 2)]),
    );
    assert_eq!(result, Some(true));
}

#[test]
fn baseline_fails_when_violation_predates_the_edit() {
    let repo = TempRepo::new("pre-existing");
    repo.write_and_commit("foo.txt", "line1\nBAD\nline3\n", "init");
    repo.write_uncommitted("foo.txt", "line1\nBAD\nline3-changed\n");

    let result = check_against_git_head(
        &bad_marker_check(),
        &repo.dir,
        &repo.path("foo.txt"),
        Some(&[(2, 2)]),
    );
    assert_eq!(result, Some(false));
}

#[test]
fn baseline_ignores_unrelated_violation_shifted_by_earlier_insertion() {
    // HEAD has a BAD marker at line 2. The current file inserts 3 new lines before
    // it (pushing it to line 5) and introduces a brand-new BAD marker at line 6 via
    // the edit under test. Without line-shift mapping, scoping the baseline to
    // current-file lines (6,6) would land on HEAD's line 6 (out of range / not the
    // marker), or — depending on direction of the bug — could accidentally line up
    // with the pre-existing marker. This asserts the new marker is correctly reported
    // as genuinely new despite the unrelated shifted violation elsewhere in the file.
    let repo = TempRepo::new("shifted");
    repo.write_and_commit("foo.txt", "line1\nBAD\nline3\nline4\n", "init");
    repo.write_uncommitted(
        "foo.txt",
        "line1\ninserted1\ninserted2\ninserted3\nBAD\nline3\nBAD-new\nline4\n",
    );

    let result = check_against_git_head(
        &bad_marker_check(),
        &repo.dir,
        &repo.path("foo.txt"),
        Some(&[(7, 7)]),
    );
    assert_eq!(result, Some(true));
}

#[test]
fn baseline_is_none_when_there_is_no_head_commit() {
    let repo = TempRepo::new("no-head");
    repo.write_uncommitted("foo.txt", "line1\nBAD\nline3\n");

    let result = check_against_git_head(
        &bad_marker_check(),
        &repo.dir,
        &repo.path("foo.txt"),
        Some(&[(2, 2)]),
    );
    assert_eq!(result, None);
}

#[test]
fn baseline_is_none_when_the_file_is_untracked_at_head() {
    let repo = TempRepo::new("untracked-file");
    repo.write_and_commit("committed.txt", "line1\n", "init");
    repo.write_uncommitted("new.txt", "line1\nBAD\n");

    let result = check_against_git_head(
        &bad_marker_check(),
        &repo.dir,
        &repo.path("new.txt"),
        Some(&[(2, 2)]),
    );
    assert_eq!(result, None);
}

#[test]
fn run_check_downgrades_severity_when_violation_predates_edit() {
    let repo = TempRepo::new("run-check-downgrade");
    repo.write_and_commit("foo.txt", "line1\nBAD\nline3\n", "init");
    repo.write_uncommitted("foo.txt", "line1\nBAD\nline3-changed\n");

    let result = run_check(
        &bad_marker_check(),
        &repo.dir,
        &repo.path("foo.txt"),
        Some(&[(2, 2)]),
        &Registry::default(),
        &RunContext::default(),
    )
    .unwrap();
    assert!(!result.passed);
    assert_eq!(result.severity, Severity::Advisory);
    assert!(result.message.unwrap().contains("predates your edits"));
}

#[test]
fn repo_wide_baseline_fails_when_violation_predates_the_edit() {
    let repo = TempRepo::new("repo-wide-pre-existing");
    repo.write_and_commit("foo.txt", "line1\nBAD\nline3\n", "init");
    repo.write_uncommitted("foo.txt", "line1\nBAD\nline3-changed\n");

    let result = check_against_git_head_repo(&repo_wide_bad_marker_check(), &repo.dir);
    assert_eq!(result, Some(false));
}

#[test]
fn repo_wide_baseline_passes_when_violation_is_genuinely_new() {
    let repo = TempRepo::new("repo-wide-genuinely-new");
    repo.write_and_commit("foo.txt", "line1\nline2\nline3\n", "init");
    repo.write_uncommitted("foo.txt", "line1\nBAD\nline3\n");

    let result = check_against_git_head_repo(&repo_wide_bad_marker_check(), &repo.dir);
    assert_eq!(result, Some(true));
}

#[test]
fn repo_wide_baseline_is_none_when_there_is_no_head_commit() {
    let repo = TempRepo::new("repo-wide-no-head");
    repo.write_uncommitted("foo.txt", "line1\nBAD\nline3\n");

    let result = check_against_git_head_repo(&repo_wide_bad_marker_check(), &repo.dir);
    assert_eq!(result, None);
}

#[test]
fn repo_wide_baseline_concurrent_calls_do_not_interfere() {
    let repo = TempRepo::new("repo-wide-concurrent");
    repo.write_and_commit("foo.txt", "line1\nBAD\nline3\n", "init");
    repo.write_uncommitted("foo.txt", "line1\nBAD\nline3-changed\n");

    let handles: Vec<_> = (0..8)
        .map(|_| {
            let dir = repo.dir.clone();
            std::thread::spawn(move || {
                check_against_git_head_repo(&repo_wide_bad_marker_check(), &dir)
            })
        })
        .collect();

    for handle in handles {
        assert_eq!(handle.join().unwrap(), Some(false));
    }
}

#[test]
fn repo_wide_baseline_cleans_up_snapshot_dir_when_archive_fails_to_spawn() {
    let pid = std::process::id();
    let prefix = format!("kibitzer-head-snapshot-{pid}-");
    let list_matching = || -> std::collections::HashSet<PathBuf> {
        std::fs::read_dir(std::env::temp_dir())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with(&prefix))
                    .unwrap_or(false)
            })
            .collect()
    };

    let before = list_matching();
    // A nonexistent repo_root makes `Command::current_dir` fail at the OS level before
    // `git` even execs, exercising the same "archive command fails to spawn" path a
    // missing `git` binary would hit.
    let missing_repo_root = std::env::temp_dir().join(format!("kibitzer-does-not-exist-{pid}"));
    let result = check_against_git_head_repo(&repo_wide_bad_marker_check(), &missing_repo_root);
    assert_eq!(result, None);

    let after = list_matching();
    assert!(
        after.is_subset(&before),
        "check_against_git_head_repo leaked a snapshot dir on spawn failure: {:?}",
        after.difference(&before).collect::<Vec<_>>()
    );
}

#[test]
fn run_check_downgrades_severity_for_repo_wide_check_when_violation_predates_edit() {
    let repo = TempRepo::new("run-check-repo-wide-downgrade");
    repo.write_and_commit("foo.txt", "line1\nBAD\nline3\n", "init");
    repo.write_uncommitted("foo.txt", "line1\nBAD\nline3-changed\n");

    let result = run_check(
        &repo_wide_bad_marker_check(),
        &repo.dir,
        &repo.dir,
        None,
        &Registry::default(),
        &RunContext::default(),
    )
    .unwrap();
    assert!(!result.passed);
    assert_eq!(result.severity, Severity::Advisory);
    assert!(result.message.unwrap().contains("predates your edits"));
}

// --- Story 2.2.3: git-HEAD-baseline downgrade for Declaration-kind checkers
// (BLOCKER fix) ---

fn content_rules_check() -> Check {
    Check {
        name: "content-rules".to_string(),
        command: None,
        checker: None,
        architecture_checker: Some("content-rules".to_string()),
        severity: Severity::Blocking,
        scope: vec![],
        triggers: vec![],
        message: Some("content rule violation".to_string()),
        output_format: None,
        options: None,
    }
}

fn domain_content_rules_arch_config() -> crate::config::ArchitectureConfig {
    crate::config::ArchitectureConfig {
        components: vec![crate::config::Component {
            name: "domain".to_string(),
            paths: vec!["**/domain".to_string(), "**/domain/**".to_string()],
        }],
        content_rules: vec![crate::config::ContentRule {
            component: "domain".to_string(),
            allowed_kinds: vec!["struct".to_string()],
        }],
        ..Default::default()
    }
}

#[test]
fn content_rules_blocking_violation_downgrades_when_it_predates_head() {
    let repo = TempRepo::new("content-rules-predates-head");
    repo.write_and_commit(
        "domain/domain.go",
        "package domain\n\ntype Order struct {\n\tID string\n}\n\n\
         func Validate(o Order) error { return nil }\n",
        "init",
    );
    // Unrelated uncommitted edit — the content-rules violation itself is
    // untouched by it, i.e. it predates this "current edit."
    repo.write_uncommitted("README.md", "unrelated edit\n");

    let arch_config = domain_content_rules_arch_config();
    let files = walk_and_collect_files(&repo.dir).unwrap();

    let result =
        run_architecture_check(&content_rules_check(), &repo.dir, &files, &arch_config).unwrap();

    assert!(!result.passed);
    assert_eq!(result.severity, Severity::Advisory);
    assert!(result.message.unwrap().contains("predates your edits"));
}

#[test]
fn content_rules_blocking_violation_stays_blocking_when_new_since_head() {
    let repo = TempRepo::new("content-rules-new-since-head");
    repo.write_and_commit(
        "domain/domain.go",
        "package domain\n\ntype Order struct {\n\tID string\n}\n",
        "init",
    );
    // Uncommitted edit introduces the violating function — absent at HEAD.
    repo.write_uncommitted(
        "domain/domain.go",
        "package domain\n\ntype Order struct {\n\tID string\n}\n\n\
         func Validate(o Order) error { return nil }\n",
    );

    let arch_config = domain_content_rules_arch_config();
    let files = walk_and_collect_files(&repo.dir).unwrap();

    let result =
        run_architecture_check(&content_rules_check(), &repo.dir, &files, &arch_config).unwrap();

    assert!(!result.passed);
    assert_eq!(result.severity, Severity::Blocking);
    assert!(!result.message.unwrap().contains("predates your edits"));
}

fn naming_rules_check() -> Check {
    Check {
        name: "naming-rules".to_string(),
        command: None,
        checker: None,
        architecture_checker: Some("naming-rules".to_string()),
        severity: Severity::Blocking,
        scope: vec![],
        triggers: vec![],
        message: Some("naming rule violation".to_string()),
        output_format: None,
        options: None,
    }
}

#[test]
fn naming_rules_blocking_violation_downgrades_when_it_predates_head() {
    let repo = TempRepo::new("naming-rules-predates-head");
    repo.write_and_commit(
        "infra/infra.go",
        "package infra\n\ntype OrderStore struct {\n\tID string\n}\n",
        "init",
    );
    repo.write_uncommitted("README.md", "unrelated edit\n");

    let arch_config = crate::config::ArchitectureConfig {
        components: vec![crate::config::Component {
            name: "infra".to_string(),
            paths: vec!["**/infra".to_string(), "**/infra/**".to_string()],
        }],
        naming_rules: vec![crate::config::NamingRule {
            component: "infra".to_string(),
            kind: "struct".to_string(),
            pattern: ".*Repository$|.*Client$".to_string(),
        }],
        ..Default::default()
    };
    let files = walk_and_collect_files(&repo.dir).unwrap();

    let result =
        run_architecture_check(&naming_rules_check(), &repo.dir, &files, &arch_config).unwrap();

    assert!(!result.passed);
    assert_eq!(result.severity, Severity::Advisory);
    assert!(result.message.unwrap().contains("predates your edits"));
}

fn instability_check() -> Check {
    Check {
        name: "instability".to_string(),
        command: None,
        checker: None,
        architecture_checker: Some("instability".to_string()),
        severity: Severity::Blocking,
        scope: vec![],
        triggers: vec![],
        message: Some("instability violation".to_string()),
        output_format: None,
        options: None,
    }
}

/// Regression guard for the `AnyArchitectureChecker::Model` dispatch arm added
/// alongside `InstabilityChecker`/`DipConcreteCouplingChecker`: proves
/// `run_architecture_check` actually builds an `ArchModel` and reaches the checker
/// (not just that the checker's own unit tests pass against a hand-built model), and
/// — via the blocking-severity downgrade path — that `check_native_against_git_head_repo`
/// resolves the `Model` arm too, matching the coverage the `Import`/`Declaration` arms
/// already have (`naming_rules_blocking_violation_downgrades_when_it_predates_head`
/// above).
#[test]
fn instability_model_checker_blocking_violation_downgrades_when_it_predates_head() {
    let repo = TempRepo::new("instability-predates-head");
    repo.write_and_commit(
        "go.mod",
        "module kibitzer.example/instabilitytest\n\ngo 1.21\n",
        "init",
    );
    repo.write_and_commit(
        "stable/stable.go",
        "package stable\n\ntype Widget struct{}\n",
        "init",
    );
    repo.write_and_commit(
        "consumer/consumer.go",
        "package consumer\n\nimport \"kibitzer.example/instabilitytest/stable\"\n\n\
         var _ = stable.Widget{}\n",
        "init",
    );
    repo.write_uncommitted("README.md", "unrelated edit\n");

    let files = walk_and_collect_files(&repo.dir).unwrap();
    let result = run_architecture_check(
        &instability_check(),
        &repo.dir,
        &files,
        &crate::config::ArchitectureConfig::default(),
    )
    .unwrap();

    assert!(!result.passed);
    assert!(result.output.contains("[instability]"));
    assert_eq!(result.severity, Severity::Advisory);
    assert!(result.message.unwrap().contains("predates your edits"));
}

fn unreferenced_private_symbol_check() -> Check {
    Check {
        name: "unreferenced-private-symbol".to_string(),
        command: None,
        checker: None,
        architecture_checker: Some("unreferenced-private-symbol".to_string()),
        severity: Severity::Advisory,
        scope: vec![],
        triggers: vec![],
        message: Some("unreferenced private symbol".to_string()),
        output_format: None,
        options: None,
    }
}

/// Regression guard for `ArchModelChecker::needs_private_symbols()`: this checker's
/// whole subject is unexported symbols, so it only works at all if
/// `run_architecture_check` actually builds its `ArchModel` with
/// `PruneConfig { include_private: true }` rather than the default `false` every other
/// `ArchModelChecker` gets. Every unit test in `unreferenced_symbols.rs` calls the
/// module-private finder function directly against a hand-built model with
/// `include_private: true` hardcoded — none of them would catch a regression where
/// `checker.needs_private_symbols()` stopped being threaded through to
/// `build_arch_model_for_check` here, which would silently prune every private symbol
/// before the checker ever saw one and leave it permanently finding nothing in real use.
#[test]
fn unreferenced_private_symbol_checker_sees_private_symbols_through_run_architecture_check() {
    let repo = TempRepo::new("unreferenced-private-symbol-wiring");
    repo.write_and_commit(
        "pkg/a.go",
        "package pkg\n\nfunc dead() {}\n\nfunc Live() {}\n",
        "init",
    );

    let files = walk_and_collect_files(&repo.dir).unwrap();
    let result = run_architecture_check(
        &unreferenced_private_symbol_check(),
        &repo.dir,
        &files,
        &crate::config::ArchitectureConfig::default(),
    )
    .unwrap();

    assert!(!result.passed, "got: {result:?}");
    assert!(result.output.contains("`dead`"), "got: {result:?}");
}

fn layering_check() -> Check {
    Check {
        name: "layering".to_string(),
        command: None,
        checker: None,
        architecture_checker: Some("layering".to_string()),
        severity: Severity::Blocking,
        scope: vec![],
        triggers: vec![],
        message: Some("layering violation".to_string()),
        output_format: None,
        options: None,
    }
}

// Regression guard: `check_native_against_git_head_repo` had zero prior test
// coverage (verified — no existing test in this module names `layering`,
// `import-cycles`, or `coupling`), so Task 2.2.3a/b's rewrite (dispatching through
// `lookup_any_architecture_checker` and branching on the enum) needs its own new
// test proving the Import-kind path still behaves exactly as before.
#[test]
fn import_kind_checker_head_baseline_downgrade_still_works_through_dual_registry_dispatch() {
    let repo = TempRepo::new("import-kind-regression");
    repo.write_and_commit("go.mod", "module fixture\ngo 1.21\n", "init");
    repo.write_and_commit(
        "handlers/handlers.go",
        "package handlers\n\nfunc Do() {}\n",
        "add handlers",
    );
    repo.write_and_commit(
        "domain/domain.go",
        "package domain\n\nimport \"fixture/handlers\"\n\nfunc Do() { handlers.Do() }\n",
        "add domain violating layering",
    );
    repo.write_uncommitted("README.md", "unrelated edit\n");

    let arch_config = crate::config::ArchitectureConfig {
        layers: vec!["handlers".to_string(), "domain".to_string()],
        ..Default::default()
    };
    let files = walk_and_collect_files(&repo.dir).unwrap();

    let result =
        run_architecture_check(&layering_check(), &repo.dir, &files, &arch_config).unwrap();

    assert!(!result.passed);
    assert_eq!(result.severity, Severity::Advisory);
    assert!(result.message.unwrap().contains("predates your edits"));
}

#[test]
#[allow(non_snake_case)]
fn architecture_check_should_NotApplyInlineIgnores_When_WholeRepoCheck() {
    let repo = TempRepo::new("arch-no-inline");
    repo.write_and_commit("go.mod", "module fixture\ngo 1.21\n", "init");
    repo.write_and_commit(
        "handlers/handlers.go",
        "package handlers\n\nfunc Do() {}\n",
        "add handlers",
    );
    repo.write_and_commit(
        "domain/domain.go",
        "package domain\n\n// kibitzer:ignore layering -- fixture covers the import\nimport \"fixture/handlers\"\n\n// kibitzer:ignore layering -- fixture covers the call\nfunc Do() { handlers.Do() }\n",
        "add domain violating layering",
    );
    let arch_config = crate::config::ArchitectureConfig {
        layers: vec!["handlers".to_string(), "domain".to_string()],
        ..Default::default()
    };
    let files = walk_and_collect_files(&repo.dir).unwrap();

    let result =
        run_architecture_check(&layering_check(), &repo.dir, &files, &arch_config).unwrap();

    assert!(!result.passed, "{result:?}");
    assert!(result.inline.dropped.is_empty(), "{result:?}");
}
