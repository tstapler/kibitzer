use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;

use crate::check::{CheckResult, run_check, run_checks_for_trigger, walk_and_collect_files};
use crate::config::{Check, Severity, find_config};

fn report(file_display: &str, result: &CheckResult) {
    if result.passed {
        return;
    }
    println!(
        "[{}] {} — {}: {}",
        match result.severity {
            Severity::Blocking => "BLOCKING",
            Severity::Advisory => "advisory",
        },
        file_display,
        result.check_name,
        result.describe()
    );
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
    let Some((config, repo_root)) = find_config(&dir)? else {
        eprintln!(
            "[kibitzer] no .claude/inspect.json found above {}",
            dir.display()
        );
        return Ok(ExitCode::SUCCESS);
    };

    let (file_checks, repo_checks): (Vec<Check>, Vec<Check>) =
        config.checks.into_iter().partition(Check::is_per_file);

    let mut any_blocking_failure = false;

    for check in &repo_checks {
        if !check.triggers.is_empty() && !check.triggers.iter().any(|t| t == trigger) {
            continue;
        }
        let result = run_check(check, &repo_root, &repo_root, None)?;
        if !result.passed && result.severity == Severity::Blocking {
            any_blocking_failure = true;
        }
        report(&repo_root.display().to_string(), &result);
    }

    let files = walk_and_collect_files(&dir)?;
    for file in files {
        for result in run_checks_for_trigger(&file_checks, trigger, &repo_root, &file, None)? {
            if !result.passed && result.severity == Severity::Blocking {
                any_blocking_failure = true;
            }
            report(&file.display().to_string(), &result);
        }
    }

    if any_blocking_failure {
        Ok(ExitCode::from(1))
    } else {
        Ok(ExitCode::SUCCESS)
    }
}

#[cfg(test)]
mod tests {
    use crate::config::{Check, Severity};

    fn checker_check() -> Check {
        Check {
            name: "native".to_string(),
            command: None,
            checker: Some("primitive-obsession".to_string()),
            severity: Severity::Advisory,
            scope: vec![],
            triggers: vec![],
            message: None,
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
