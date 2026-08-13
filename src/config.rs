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
    pub command: String,
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

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub checks: Vec<Check>,
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
            return Ok(Some((config, dir)));
        }
        match dir.parent() {
            Some(parent) => dir = parent.to_path_buf(),
            None => return Ok(None),
        }
    }
}
