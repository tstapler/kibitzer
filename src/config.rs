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
    /// Mutually exclusive with `checker` — exactly one of the two must be set.
    #[serde(default)]
    pub command: Option<String>,
    /// Name of a natively implemented checker (looked up in `checker::registry()`) to
    /// run in-process instead of shelling out via `command`. Mutually exclusive with
    /// `command`.
    #[serde(default)]
    pub checker: Option<String>,
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
}

impl Check {
    /// Whether this check is scoped to a single triggering file, as opposed to
    /// whole-repo (no `{file}` placeholder in `command`, or a native checker — which
    /// always runs against one file at a time).
    pub fn is_per_file(&self) -> bool {
        match (&self.command, &self.checker) {
            (Some(command), _) => command.contains("{file}"),
            (None, Some(_)) => true,
            (None, None) => true,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub checks: Vec<Check>,
}

fn validate(config: &Config, config_path: &Path) -> Result<()> {
    for check in &config.checks {
        match (&check.command, &check.checker) {
            (Some(_), Some(_)) => anyhow::bail!(
                "{}: check '{}' sets both `command` and `checker` — these are mutually exclusive",
                config_path.display(),
                check.name
            ),
            (None, None) => anyhow::bail!(
                "{}: check '{}' sets neither `command` nor `checker` — exactly one is required",
                config_path.display(),
                check.name
            ),
            (None, Some(checker_name)) if crate::checker::lookup(checker_name).is_none() => {
                anyhow::bail!(
                    "{}: check '{}' references unknown checker '{}' — run `kibitzer check list` \
                     for available checkers",
                    config_path.display(),
                    check.name,
                    checker_name
                )
            }
            _ => {}
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
}
