use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub const CONFIG_FILENAME: &str = "inspect.json";
pub const CONFIG_DIR: &str = ".claude";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Blocking,
    Advisory,
}

/// A structured output shape kibitzer knows how to parse from a `command` check's
/// stdout, instead of only reading the process exit code. See `docs/output-formats.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    /// SARIF 2.1.0 (`https://sarifweb.azurewebsites.net/`) — the format most linters
    /// with a `--format sarif`/`--output-format sarif` flag already emit.
    Sarif,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Check {
    pub name: String,
    /// Shell command to run. `{file}` is substituted with the triggering file path.
    ///
    /// When kibitzer knows which lines an edit actually touched (the Claude Code hook,
    /// not batch mode), a check can opt into diff-aware scoping two ways:
    ///
    /// - `{changed_lines}`: substituted with a comma-separated list of 1-indexed,
    ///   inclusive `start-end` ranges (e.g. `12-15,40-40`), or empty if no ranges are
    ///   known — pass this to a linter/tool that supports scoping its own scan to a
    ///   line range.
    /// - Automatic output filtering: if a check's command emits output lines in the
    ///   `{file}:{line}: message` convention (most linters do), kibitzer filters those
    ///   lines down to ones inside the changed ranges and recomputes pass/fail from
    ///   what survives — no command changes needed. Output that doesn't follow this
    ///   convention is left untouched (conservatively kept, so unrecognized output
    ///   can't be silently swallowed).
    ///
    /// Mutually exclusive with `checker`/`architecture_checker` — exactly one of the
    /// three must be set.
    #[serde(default)]
    pub command: Option<String>,
    /// Name of a natively implemented checker (looked up in `checker::registry()`) to
    /// run in-process instead of shelling out via `command`. Mutually exclusive with
    /// `command`/`architecture_checker`.
    #[serde(default)]
    pub checker: Option<String>,
    /// Name of a natively implemented whole-repo architecture checker (looked up in
    /// `architecture_checks::registry()`) that runs once per batch against the whole
    /// repo's import graph, instead of once per triggering file. Mutually exclusive
    /// with `command`/`checker`. `triggers` must be empty or `["batch"]` — rebuilding
    /// the import graph on every `PostToolUse` edit is too expensive.
    #[serde(default)]
    pub architecture_checker: Option<String>,
    pub severity: Severity,
    /// Glob patterns (supporting `**`) a file path must match for this check to apply.
    #[serde(default)]
    pub scope: Vec<String>,
    /// Which hook events / run triggers this check fires on (e.g. "PostToolUse", "batch").
    #[serde(default)]
    pub triggers: Vec<String>,
    /// Message shown to the agent alongside command output when the check fails.
    #[serde(default)]
    pub message: Option<String>,
    /// A structured shape kibitzer should parse from `command`'s stdout instead of
    /// treating the check as pure exit-code pass/fail. Requires `command` — native
    /// checkers already report structured findings. See `docs/output-formats.md`.
    #[serde(default)]
    pub output_format: Option<OutputFormat>,
}

/// How a [`Check`] is dispatched: once per triggering file, or once per whole-repo
/// batch invocation; and via a shell `command` or an in-process native checker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckKind {
    PerFileCommand,
    WholeRepoCommand,
    PerFileNative,
    WholeRepoNative,
}

impl Check {
    /// Which of the four dispatch shapes this check is.
    pub fn kind(&self) -> CheckKind {
        if self.architecture_checker.is_some() {
            return CheckKind::WholeRepoNative;
        }
        match (&self.command, &self.checker) {
            (Some(command), _) => {
                if command.contains("{file}") {
                    CheckKind::PerFileCommand
                } else {
                    CheckKind::WholeRepoCommand
                }
            }
            (None, Some(_)) => CheckKind::PerFileNative,
            (None, None) => CheckKind::PerFileNative,
        }
    }

    /// Whether this check is scoped to a single triggering file, as opposed to
    /// whole-repo (no `{file}` placeholder in `command`, a whole-repo `command`, or a
    /// whole-repo `architecture_checker`).
    pub fn is_per_file(&self) -> bool {
        matches!(
            self.kind(),
            CheckKind::PerFileCommand | CheckKind::PerFileNative
        )
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub checks: Vec<Check>,
}

fn validate(config: &Config, config_path: &Path) -> Result<()> {
    for check in &config.checks {
        let set_count = [
            check.command.is_some(),
            check.checker.is_some(),
            check.architecture_checker.is_some(),
        ]
        .into_iter()
        .filter(|set| *set)
        .count();
        if set_count > 1 {
            anyhow::bail!(
                "{}: check '{}' sets more than one of `command`/`checker`/`architecture_checker` \
                 — these are mutually exclusive",
                config_path.display(),
                check.name
            );
        }
        if set_count == 0 {
            anyhow::bail!(
                "{}: check '{}' sets none of `command`/`checker`/`architecture_checker` — \
                 exactly one is required",
                config_path.display(),
                check.name
            );
        }
        if let Some(checker_name) = &check.checker
            && crate::checker::lookup(checker_name).is_none()
        {
            anyhow::bail!(
                "{}: check '{}' references unknown checker '{}' — run `kibitzer check list` \
                 for available checkers",
                config_path.display(),
                check.name,
                checker_name
            );
        }
        if let Some(arch_name) = &check.architecture_checker {
            if crate::architecture_checks::lookup(arch_name).is_none() {
                anyhow::bail!(
                    "{}: check '{}' references unknown architecture checker '{}'",
                    config_path.display(),
                    check.name,
                    arch_name
                );
            }
            if check.triggers.iter().any(|t| t != "batch") {
                anyhow::bail!(
                    "{}: check '{}' sets `architecture_checker` with a trigger other than \
                     `batch` — whole-repo architecture checks may only run in batch mode, \
                     never on a per-edit trigger like `PostToolUse`",
                    config_path.display(),
                    check.name
                );
            }
        }
        if check.output_format.is_some() && check.command.is_none() {
            anyhow::bail!(
                "{}: check '{}' sets `output_format` without `command` — structured output \
                 parsing only applies to shell-command checks",
                config_path.display(),
                check.name
            );
        }
    }
    Ok(())
}

/// Walk upward from `start` looking for `.claude/inspect.json`, returning the parsed
/// config and the directory it was found in (the repo root, by convention).
pub fn find_config(start: &Path) -> Result<Option<(Config, PathBuf)>> {
    let mut dir = if start.is_dir() {
        start.to_path_buf()
    } else {
        start
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    };

    loop {
        let candidate = dir.join(CONFIG_DIR).join(CONFIG_FILENAME);
        if candidate.is_file() {
            let raw = std::fs::read_to_string(&candidate)
                .with_context(|| format!("reading {}", candidate.display()))?;
            let config: Config = serde_json::from_str(&raw)
                .with_context(|| format!("parsing {}", candidate.display()))?;
            validate(&config, &candidate)?;
            return Ok(Some((config, dir)));
        }
        match dir.parent() {
            Some(parent) => dir = parent.to_path_buf(),
            None => return Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(json: &str) -> Result<Config> {
        let config: Config = serde_json::from_str(json)?;
        validate(&config, Path::new(".claude/inspect.json"))?;
        Ok(config)
    }

    #[test]
    fn accepts_checker_without_command() {
        let config = parse(
            r#"{"checks": [{"name": "n", "checker": "primitive-obsession", "severity": "advisory"}]}"#,
        )
        .unwrap();
        assert_eq!(
            config.checks[0].checker.as_deref(),
            Some("primitive-obsession")
        );
        assert!(config.checks[0].command.is_none());
    }

    #[test]
    fn accepts_command_without_checker() {
        let config = parse(
            r#"{"checks": [{"name": "n", "command": "true {file}", "severity": "advisory"}]}"#,
        )
        .unwrap();
        assert_eq!(config.checks[0].command.as_deref(), Some("true {file}"));
        assert!(config.checks[0].checker.is_none());
    }

    #[test]
    fn rejects_both_command_and_checker() {
        let err = parse(
            r#"{"checks": [{"name": "n", "command": "true", "checker": "x", "severity": "advisory"}]}"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"));
    }

    #[test]
    fn rejects_neither_command_nor_checker() {
        let err = parse(r#"{"checks": [{"name": "n", "severity": "advisory"}]}"#).unwrap_err();
        assert!(err.to_string().contains("exactly one is required"));
    }

    #[test]
    fn rejects_unknown_checker_name() {
        let err = parse(
            r#"{"checks": [{"name": "n", "checker": "does-not-exist", "severity": "advisory"}]}"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("unknown checker"));
    }

    #[test]
    fn accepts_output_format_with_command() {
        let config = parse(
            r#"{"checks": [{"name": "n", "command": "lint --sarif {file}", "severity": "advisory", "output_format": "sarif"}]}"#,
        )
        .unwrap();
        assert_eq!(config.checks[0].output_format, Some(OutputFormat::Sarif));
    }

    #[test]
    fn rejects_output_format_without_command() {
        let err = parse(
            r#"{"checks": [{"name": "n", "checker": "primitive-obsession", "severity": "advisory", "output_format": "sarif"}]}"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("output_format"));
    }

    #[test]
    fn accepts_architecture_checker_with_batch_trigger() {
        let config = parse(
            r#"{"checks": [{"name": "n", "architecture_checker": "import-cycles", "severity": "advisory", "triggers": ["batch"]}]}"#,
        )
        .unwrap();
        assert_eq!(
            config.checks[0].architecture_checker.as_deref(),
            Some("import-cycles")
        );
        assert_eq!(config.checks[0].kind(), CheckKind::WholeRepoNative);
        assert!(!config.checks[0].is_per_file());
    }

    #[test]
    fn accepts_architecture_checker_with_no_triggers() {
        parse(
            r#"{"checks": [{"name": "n", "architecture_checker": "import-cycles", "severity": "advisory"}]}"#,
        )
        .unwrap();
    }

    #[test]
    fn rejects_architecture_checker_alongside_checker() {
        let err = parse(
            r#"{"checks": [{"name": "n", "checker": "primitive-obsession", "architecture_checker": "import-cycles", "severity": "advisory"}]}"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"));
    }

    #[test]
    fn rejects_unknown_architecture_checker_name() {
        let err = parse(
            r#"{"checks": [{"name": "n", "architecture_checker": "does-not-exist", "severity": "advisory"}]}"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("unknown architecture checker"));
    }

    #[test]
    fn rejects_architecture_checker_on_non_batch_trigger() {
        let err = parse(
            r#"{"checks": [{"name": "n", "architecture_checker": "import-cycles", "severity": "advisory", "triggers": ["PostToolUse"]}]}"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("batch"));
    }

    #[test]
    fn whole_repo_command_check_is_not_per_file() {
        let config = parse(
            r#"{"checks": [{"name": "n", "command": "true", "severity": "advisory"}]}"#,
        )
        .unwrap();
        assert_eq!(config.checks[0].kind(), CheckKind::WholeRepoCommand);
        assert!(!config.checks[0].is_per_file());
    }
}
