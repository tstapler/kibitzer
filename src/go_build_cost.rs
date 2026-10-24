//! `kibitzer go build-cost`: summarizes `go build -debug-actiongraph` for a module as a
//! cold build followed by a warm no-change build. `-debug-actiongraph` is internal Go
//! diagnostic machinery, so it only runs on toolchains this module was tested against.
//!
//! Both builds use a private `GOCACHE`, so the user's shared cache is neither cleared nor
//! written. The action graph goes to a real temp file (not `/dev/null`) for Windows.

use std::fmt;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

const MIN_SUPPORTED_MINOR: u32 = 19;
/// Newest Go minor version whose action-graph JSON this parser was checked against.
const MAX_TESTED_MINOR: u32 = 26;

#[derive(Debug, PartialEq, Eq)]
pub enum VersionGate {
    Supported,
    Skip(String),
}

pub fn parse_go_version(stdout: &str) -> Option<(u32, u32)> {
    let token = stdout
        .split_whitespace()
        .find_map(|t| t.strip_prefix("go1."))?;
    let minor: String = token.chars().take_while(char::is_ascii_digit).collect();
    Some((1, minor.parse().ok()?))
}

pub fn gate(version: (u32, u32)) -> VersionGate {
    let (major, minor) = version;
    if major != 1 || minor < MIN_SUPPORTED_MINOR {
        VersionGate::Skip(format!(
            "go build-cost: -debug-actiongraph requires Go >= 1.{MIN_SUPPORTED_MINOR} (found go{major}.{minor})"
        ))
    } else if minor > MAX_TESTED_MINOR {
        VersionGate::Skip(format!(
            "go build-cost: go{major}.{minor} is newer than the newest tested version (go1.{MAX_TESTED_MINOR}); \
             the action-graph schema is unversioned, so no report is produced"
        ))
    } else {
        VersionGate::Supported
    }
}

#[derive(Deserialize)]
struct Action {
    #[serde(rename = "Mode")]
    mode: String,
    #[serde(rename = "Cmd")]
    cmd: Option<serde_json::Value>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct PassStats {
    pub actions: usize,
    /// `build` actions that ran a compiler command.
    pub compiled: usize,
    /// `build` actions satisfied from the build cache.
    pub cache_hits: usize,
}

pub fn parse_action_graph(json: &str) -> Result<PassStats> {
    let actions: Vec<Action> =
        serde_json::from_str(json).context("parsing -debug-actiongraph JSON")?;
    let mut stats = PassStats {
        actions: actions.len(),
        ..PassStats::default()
    };
    for action in actions.iter().filter(|a| a.mode == "build") {
        if action.cmd.as_ref().is_some_and(|c| !c.is_null()) {
            stats.compiled += 1;
        } else {
            stats.cache_hits += 1;
        }
    }
    Ok(stats)
}

pub enum BuildCostOutcome {
    Report(BuildCostReport),
    /// The toolchain is missing or outside the tested range; the message says why.
    Skipped(String),
}

#[derive(Debug)]
pub struct BuildCostReport {
    pub go_version: (u32, u32),
    pub cold: PassStats,
    pub warm: PassStats,
}

impl fmt::Display for BuildCostReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "go build-cost (go1.{}):", self.go_version.1)?;
        for (label, s) in [("cold", &self.cold), ("warm", &self.warm)] {
            writeln!(
                f,
                "  {label}: {} actions, {} compiled, {} cache hits",
                s.actions, s.compiled, s.cache_hits
            )?;
        }
        Ok(())
    }
}

fn detect_version() -> Result<(u32, u32)> {
    let out = Command::new("go")
        .arg("version")
        .output()
        .context("running `go version`")?;
    parse_go_version(&String::from_utf8_lossy(&out.stdout))
        .context("could not parse `go version` output")
}

/// Runs the cold and warm builds, or returns the gate's diagnostic when the toolchain is
/// outside the tested range.
pub fn run_build_cost(dir: &Path) -> Result<BuildCostOutcome> {
    let Ok(version) = detect_version() else {
        return Ok(BuildCostOutcome::Skipped(
            "go build-cost: no usable `go` binary found on PATH".to_string(),
        ));
    };
    if let VersionGate::Skip(msg) = gate(version) {
        return Ok(BuildCostOutcome::Skipped(msg));
    }
    // A leftover dir (crash, reused pid) would pre-warm the "cold" cache, so start clean
    // and fail if something recreates it.
    let scratch = std::env::temp_dir().join(format!("kibitzer-build-cost-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir(&scratch).with_context(|| format!("creating {}", scratch.display()))?;
    let result = (|| {
        let cold = build_pass(dir, &scratch, "cold")?;
        let warm = build_pass(dir, &scratch, "warm")?;
        Ok(BuildCostReport {
            go_version: version,
            cold,
            warm,
        })
    })();
    let _ = std::fs::remove_dir_all(&scratch);
    result.map(BuildCostOutcome::Report)
}

fn build_pass(dir: &Path, scratch: &Path, label: &str) -> Result<PassStats> {
    let graph = scratch.join(format!("{label}.json"));
    let output = Command::new("go")
        .args(["build", "-debug-actiongraph"])
        .arg(&graph)
        .arg("./...")
        .current_dir(dir)
        .env("GOCACHE", scratch.join("gocache"))
        .output()
        .context("running `go build`")?;
    if !output.status.success() {
        bail!(
            "`go build ./...` failed in {} ({label} pass): {}",
            dir.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let json =
        std::fs::read_to_string(&graph).with_context(|| format!("reading {}", graph.display()))?;
    parse_action_graph(&json)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_major_minor_from_go_version_output() {
        assert_eq!(
            parse_go_version("go version go1.21.3 linux/amd64"),
            Some((1, 21))
        );
        assert_eq!(
            parse_go_version("go version go1.18 darwin/arm64"),
            Some((1, 18))
        );
        assert_eq!(parse_go_version("not go"), None);
    }

    #[test]
    fn gate_skips_old_versions_with_diagnostic_and_accepts_tested_range() {
        let VersionGate::Skip(msg) = gate((1, 18)) else {
            panic!("1.18 must be skipped")
        };
        assert!(msg.contains("requires Go >= 1.19 (found go1.18)"), "{msg}");
        assert_eq!(gate((1, 19)), VersionGate::Supported);
        assert_eq!(gate((1, MAX_TESTED_MINOR)), VersionGate::Supported);
    }

    #[test]
    fn gate_is_bounded_above_for_untested_versions() {
        assert!(matches!(
            gate((1, MAX_TESTED_MINOR + 1)),
            VersionGate::Skip(_)
        ));
    }

    #[test]
    fn counts_compiled_and_cached_build_actions_only() {
        let json = r#"[
            {"Mode":"build","Cmd":[{"x":1}]},
            {"Mode":"build","Cmd":null},
            {"Mode":"build check cache","Cmd":null},
            {"Mode":"link","Cmd":[{"x":1}]}
        ]"#;
        assert_eq!(
            parse_action_graph(json).unwrap(),
            PassStats {
                actions: 4,
                compiled: 1,
                cache_hits: 1
            }
        );
    }

    #[test]
    fn real_go_build_has_compiles_cold_and_cache_hits_warm() {
        let Ok(version) = detect_version() else {
            return;
        };
        if gate(version) != VersionGate::Supported {
            return;
        }
        let dir = crate::test_support::unique_temp_dir("build-cost");
        std::fs::write(dir.join("go.mod"), "module example.test/bc\n\ngo 1.21\n").unwrap();
        std::fs::write(dir.join("m.go"), "package main\n\nfunc main() {}\n").unwrap();
        let BuildCostOutcome::Report(report) = run_build_cost(&dir).unwrap() else {
            panic!("skipped")
        };
        assert!(report.cold.compiled > 0, "{report}");
        assert_eq!(report.warm.compiled, 0, "{report}");
        assert!(report.warm.cache_hits > 0, "{report}");
    }
}
