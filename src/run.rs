use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Result;

use crate::check::{
    CheckResult, run_architecture_check, run_check, run_checks_for_trigger, walk_and_collect_files,
};
use crate::config::{Check, Severity, find_effective_config};

fn severity_label(severity: Severity) -> &'static str {
    match severity {
        Severity::Blocking => "BLOCKING",
        Severity::Advisory => "advisory",
    }
}

/// Whether `result.findings` (when non-empty) contains any finding whose EFFECTIVE
/// severity — `severity_override.unwrap_or(result.severity)` — is `Blocking`. Falls back
/// to `result.severity` alone when `findings` is empty (every `CheckResult` source other
/// than `run_architecture_check`'s native checkers, which don't populate `findings` at
/// all). Mirrors `mcp.rs::architecture_assessment`'s per-finding severity resolution
/// (fixed in f1a45a9) for the `kibitzer check run` / `run_batch` CI path: without this, a
/// `component-deps`/`content-rules`/`naming-rules` check configured `"severity":
/// "blocking"` would fail the whole batch (and print `[BLOCKING]`) on a harmless
/// zero-match-component advisory (`ArchFinding.severity_override: Some(Severity::Advisory)`,
/// set unconditionally by `zero_match_advisory`) just because the enclosing `Check`'s
/// configured severity was blocking — the same conflation the MCP tool path had.
fn has_blocking_finding(result: &CheckResult) -> bool {
    if result.findings.is_empty() {
        result.severity == Severity::Blocking
    } else {
        result
            .findings
            .iter()
            .any(|f| f.severity_override.unwrap_or(result.severity) == Severity::Blocking)
    }
}

/// Renders the report lines for `result` against `file_display`, without printing them —
/// factored out so tests can assert on exact rendered output (including which severity
/// label a given finding got) by calling `run_batch_collect` directly, instead of having
/// to capture the process's real stdout. `run_batch` prints these lines via `println!`.
///
/// One line per `CheckResult` when it has no structured `findings` (shell-out checks,
/// native syntax-rules checks — same rendering as before this fix). When `findings` is
/// non-empty (architecture/declaration checkers), one line PER FINDING using that
/// finding's own effective severity, instead of a single line stamped with
/// `result.severity` for the whole check — see `has_blocking_finding` above for why.
fn report_lines(file_display: &str, result: &CheckResult) -> Vec<String> {
    if result.passed {
        return Vec::new();
    }
    if result.findings.is_empty() {
        return vec![format!(
            "[{}] {} — {}: {}",
            severity_label(result.severity),
            file_display,
            result.check_name,
            result.describe()
        )];
    }
    result
        .findings
        .iter()
        .map(|finding| {
            let level = finding.severity_override.unwrap_or(result.severity);
            let location = match (&finding.file, finding.line) {
                (Some(file), Some(line)) => format!("{}:{}: ", file.display(), line),
                (Some(file), None) => format!("{}: ", file.display()),
                (None, _) => String::new(),
            };
            format!(
                "[{}] {} — {}: {}{}",
                severity_label(level),
                file_display,
                result.check_name,
                location,
                finding.message
            )
        })
        .collect()
}

/// Batch mode: run every check whose `triggers` includes `trigger` (or has no
/// triggers at all) against every file under `dir`, reporting all failures.
///
/// Checks are split by whether `command` references `{file}`: a check without it is
/// whole-repo-scoped (e.g. `lychee --config lychee.toml .`, `python3 scripts/doc_report.py`)
/// and must run exactly once per batch invocation, not once per matched file — otherwise an
/// N-file repo re-runs an already-whole-repo command N times (confirmed: ~22x, >90s, against
/// design-docs' ~22 markdown files).
pub fn run_batch(dir: PathBuf, trigger: &str) -> Result<ExitCode> {
    let (any_blocking_failure, lines) = run_batch_collect(&dir, trigger)?;
    for line in &lines {
        println!("{line}");
    }
    if any_blocking_failure {
        Ok(ExitCode::from(1))
    } else {
        Ok(ExitCode::SUCCESS)
    }
}

/// The actual batch/run logic behind `run_batch`, with printing deferred to the caller so
/// it can be exercised end to end in tests without capturing the process's real stdout:
/// returns whether any check hit a genuine blocking failure, plus every rendered report
/// line in print order (see `report_lines`).
fn run_batch_collect(dir: &Path, trigger: &str) -> Result<(bool, Vec<String>)> {
    let (config, repo_root) = find_effective_config(dir)?;

    let arch_config = config.architecture.clone();
    let (file_checks, repo_checks): (Vec<Check>, Vec<Check>) =
        config.checks.into_iter().partition(Check::is_per_file);

    let mut any_blocking_failure = false;
    let mut lines = Vec::new();

    let files = walk_and_collect_files(dir)?;

    for check in &repo_checks {
        if !check.triggers.is_empty() && !check.triggers.iter().any(|t| t == trigger) {
            continue;
        }
        let result = if check.architecture_checker.is_some() {
            run_architecture_check(check, &repo_root, &files, &arch_config)?
        } else {
            run_check(check, &repo_root, &repo_root, None)?
        };
        if !result.passed && has_blocking_finding(&result) {
            any_blocking_failure = true;
        }
        lines.extend(report_lines(&repo_root.display().to_string(), &result));
    }

    for file in &files {
        for result in run_checks_for_trigger(&file_checks, trigger, &repo_root, file, None)? {
            if !result.passed && has_blocking_finding(&result) {
                any_blocking_failure = true;
            }
            lines.extend(report_lines(&file.display().to_string(), &result));
        }
    }

    Ok((any_blocking_failure, lines))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use crate::config::{Check, Severity};

    use super::run_batch_collect;

    static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn tmp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "kibitzer-run-test-{}-{name}-{}",
            std::process::id(),
            TMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Same fixture as `mcp.rs`'s `write_zero_match_only_component_deps_fixture` (the
    /// test that proved the `f1a45a9` fix for the MCP tool path): a `component-deps`
    /// check configured `"severity": "blocking"` whose only real output is a zero-match
    /// advisory for component "ghost" (its glob matches no import-graph node — there are
    /// no imports at all in this fixture). Deliberately not a git repo, for the same
    /// reason as the MCP fixture: `check_native_against_git_head_repo`'s "predates your
    /// edits" baseline downgrade only fires for a `Blocking` check with non-empty
    /// findings, and this fixture would otherwise satisfy that trigger for an unrelated
    /// reason, masking whether `severity_override` wiring (not the baseline downgrade) is
    /// what keeps this advisory.
    fn write_zero_match_only_component_deps_fixture(dir: &std::path::Path) {
        std::fs::create_dir_all(dir.join("pkg")).unwrap();
        std::fs::create_dir_all(dir.join(".claude")).unwrap();
        std::fs::write(dir.join("go.mod"), "module fixture\ngo 1.21\n").unwrap();
        std::fs::write(dir.join("pkg/pkg.go"), "package pkg\n\nfunc F() {}\n").unwrap();
        std::fs::write(
            dir.join(".claude/inspect.json"),
            r#"{
  "architecture": {
    "components": [{"name": "ghost", "paths": ["**/ghost", "**/ghost/**"]}]
  },
  "checks": [
    { "name": "component-deps", "architecture_checker": "component-deps", "severity": "blocking" }
  ]
}"#,
        )
        .unwrap();
    }

    /// Companion fixture with a GENUINE `component-deps` violation under the same
    /// `"severity": "blocking"` config, so the test using it proves the fix didn't
    /// accidentally make every architecture finding advisory.
    fn write_real_component_deps_violation_fixture(dir: &std::path::Path) {
        std::fs::create_dir_all(dir.join("svcs")).unwrap();
        std::fs::create_dir_all(dir.join("ext")).unwrap();
        std::fs::create_dir_all(dir.join(".claude")).unwrap();
        std::fs::write(dir.join("go.mod"), "module fixture\ngo 1.21\n").unwrap();
        std::fs::write(
            dir.join("svcs/svcs.go"),
            "package svcs\n\nimport \"fixture/ext\"\n\nfunc Use() { ext.Do() }\n",
        )
        .unwrap();
        std::fs::write(dir.join("ext/ext.go"), "package ext\n\nfunc Do() {}\n").unwrap();
        std::fs::write(
            dir.join(".claude/inspect.json"),
            r#"{
  "architecture": {
    "components": [
      {"name": "svcs", "paths": ["**/svcs", "**/svcs/**"]},
      {"name": "ext", "paths": ["**/ext", "**/ext/**"]}
    ],
    "dependency_rules": [{"component": "svcs", "may_depend_on": []}]
  },
  "checks": [
    { "name": "component-deps", "architecture_checker": "component-deps", "severity": "blocking" }
  ]
}"#,
        )
        .unwrap();
    }

    /// The end-to-end regression test for the batch/CI path's counterpart to the
    /// `f1a45a9` MCP-tool fix: before this fix, `run_batch`/`run_batch_collect` derived
    /// `any_blocking_failure` and the `[BLOCKING]`/`[advisory]` print prefix from
    /// `result.severity` alone — the whole `Check`'s configured severity — so a
    /// `component-deps` check configured `"severity": "blocking"` whose only finding was
    /// a harmless zero-match-component advisory still failed the batch run and printed
    /// `[BLOCKING]`.
    #[test]
    fn run_batch_does_not_block_on_zero_match_advisory_under_blocking_severity_config() {
        let dir = tmp_dir("zero-match-under-blocking");
        write_zero_match_only_component_deps_fixture(&dir);

        let (any_blocking_failure, lines) = run_batch_collect(&dir, "manual").unwrap();

        std::fs::remove_dir_all(&dir).ok();

        assert!(
            !any_blocking_failure,
            "zero-match advisory must not trip any_blocking_failure, lines:\n{lines:#?}"
        );
        let finding_line = lines
            .iter()
            .find(|l| l.contains("[component]"))
            .unwrap_or_else(|| {
                panic!("expected a [component] zero-match finding, got:\n{lines:#?}")
            });
        assert!(
            finding_line.starts_with("[advisory]"),
            "zero-match advisory rendered with the wrong prefix, expected `[advisory] ...`, got: {finding_line}"
        );
        assert!(
            !lines.iter().any(|l| l.starts_with("[BLOCKING]")),
            "expected no [BLOCKING] line anywhere in batch output, got:\n{lines:#?}"
        );
    }

    /// Regression guard alongside the test above: a check configured `"severity":
    /// "blocking"` against a fixture with a REAL violation (not a zero-match advisory)
    /// must still trip `any_blocking_failure` (batch exit 1) and print `[BLOCKING]` —
    /// `severity_override` only ever narrows a finding toward `Advisory`, it never
    /// suppresses a genuine violation's severity.
    #[test]
    fn run_batch_still_blocks_on_a_real_violation_under_the_same_config() {
        let dir = tmp_dir("real-violation-stays-blocking");
        write_real_component_deps_violation_fixture(&dir);

        let (any_blocking_failure, lines) = run_batch_collect(&dir, "manual").unwrap();

        std::fs::remove_dir_all(&dir).ok();

        assert!(
            any_blocking_failure,
            "real component-deps violation must trip any_blocking_failure, lines:\n{lines:#?}"
        );
        let finding_line = lines
            .iter()
            .find(|l| l.contains("component-deps"))
            .unwrap_or_else(|| {
                panic!("expected a component-deps violation finding, got:\n{lines:#?}")
            });
        assert!(
            finding_line.starts_with("[BLOCKING]"),
            "real violation must still render [BLOCKING], got: {finding_line}"
        );
    }

    fn checker_check() -> Check {
        Check {
            name: "native".to_string(),
            command: None,
            checker: Some("primitive-obsession".to_string()),
            architecture_checker: None,
            severity: Severity::Advisory,
            scope: vec![],
            triggers: vec![],
            message: None,
            output_format: None,
        }
    }

    #[test]
    fn native_checker_check_is_per_file() {
        assert!(checker_check().is_per_file());
    }

    #[test]
    fn native_checker_check_partitions_into_file_checks() {
        let checks = vec![checker_check()];
        let (file_checks, repo_checks): (Vec<Check>, Vec<Check>) =
            checks.into_iter().partition(Check::is_per_file);
        assert_eq!(file_checks.len(), 1);
        assert!(repo_checks.is_empty());
    }
}
