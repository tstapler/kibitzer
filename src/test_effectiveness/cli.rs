//! Argument surface of the hidden `kibitzer tests report` command (Task 2.3.1a) and the `--kill-completeness`
//! flag with its precedence against the run sidecar (Task 2.2.9b).

use std::path::PathBuf;

use clap::{Args, Subcommand, ValueEnum};

use super::adapters::guard::DEFAULT_MAX_INPUT_BYTES;
use super::adapters::sidecar::{self, Sidecar};
use super::matrix::{CoverageAttribution, KillCompleteness, Warning};

#[derive(Subcommand)]
pub enum TestsCommand {
    /// Rank tests whose coverage or kills no other test needs, from ingested third-party reports.
    Report(ReportArgs),
}

/// `--kill-completeness` spelling (hyphenated; JSON values are snake_case).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum KillCompletenessArg {
    Full,
    FirstKiller,
    Sampled,
    BudgetLimited,
}

impl KillCompletenessArg {
    pub fn into_domain(self) -> KillCompleteness {
        match self {
            Self::Full => KillCompleteness::Full,
            Self::FirstKiller => KillCompleteness::FirstKillerOnly,
            Self::Sampled => KillCompleteness::Sampled,
            Self::BudgetLimited => KillCompleteness::BudgetLimited,
        }
    }
}

/// `--attribution` spelling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum AttributionArg {
    Isolated,
    InOrder,
    LeaveOneOut,
}

impl AttributionArg {
    pub fn into_domain(self) -> CoverageAttribution {
        match self {
            Self::Isolated => CoverageAttribution::Isolated,
            Self::InOrder => CoverageAttribution::InOrder,
            Self::LeaveOneOut => CoverageAttribution::LeaveOneOut,
        }
    }
}

#[derive(Args, Debug, Clone)]
pub struct ReportArgs {
    /// coverage.py JSON report produced with `--show-contexts`.
    #[arg(long, value_name = "FILE")]
    pub coveragepy: Option<PathBuf>,
    /// Directory of per-test Go coverage profiles plus `manifest.json`.
    #[arg(long, value_name = "DIR")]
    pub go_profiles: Option<PathBuf>,
    /// Stryker mutation-testing-elements JSON report.
    #[arg(long, value_name = "FILE")]
    pub stryker: Option<PathBuf>,
    /// PIT `mutations.xml`.
    #[arg(long, value_name = "FILE")]
    pub pit: Option<PathBuf>,
    /// Output format: text, json or brief (validated by the command so a bad value gets the documented error).
    #[arg(long, default_value = "text")]
    pub format: String,
    /// Rows shown in text output; 0 means all. JSON is always complete.
    #[arg(long, default_value = "20")]
    pub top: String,
    /// How per-test coverage was attributed, when the input does not say.
    #[arg(long, value_enum)]
    pub attribution: Option<AttributionArg>,
    /// How complete the kill data is, when no run sidecar says; the less complete of flag and sidecar wins.
    #[arg(long, value_enum)]
    pub kill_completeness: Option<KillCompletenessArg>,
    /// Commit the inputs were produced at, when the input does not carry one.
    #[arg(long, value_name = "SHA")]
    pub input_commit: Option<String>,
    /// Largest input file read, in bytes. Defaults to the built-in ceiling.
    #[arg(long, default_value_t = DEFAULT_MAX_INPUT_BYTES)]
    pub max_input_bytes: u64,
    /// Also list signals outside the main-list rule, in their own section.
    #[arg(long)]
    pub experimental: bool,
    /// Write `[tests]` diagnostics to stderr.
    #[arg(long)]
    pub verbose: bool,
    #[arg(long, hide = true, value_name = "PATH")]
    pub emit_experiment_input: Option<PathBuf>,
}

/// Story 2.2.9 precedence: a sidecar and the flag that disagree resolve to the less complete value with a class W
/// warning naming both; either alone wins; neither stated is `Full`.
pub fn resolve_kill_flag(
    sidecar: Option<&Sidecar>,
    flag: Option<KillCompletenessArg>,
) -> (KillCompleteness, Vec<Warning>) {
    sidecar::resolve_kill_completeness(sidecar, flag.map(KillCompletenessArg::into_domain))
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;

    fn sidecar_saying(completeness: &str) -> Sidecar {
        let body = match completeness {
            "full" => r#"{"schema_version":1,"tool":"stryker","tool_version":"8","commit":"abc1234","mutants_total":10,"mutants_run":10}"#,
            _ => r#"{"schema_version":1,"tool":"stryker","tool_version":"8","commit":"abc1234","mutants_total":10,"mutants_run":3}"#,
        };
        sidecar::parse_sidecar(body.as_bytes()).unwrap().sidecar
    }

    #[test]
    fn kill_completeness_flag_should_ResolveToLessCompleteValue_When_SidecarAndFlagDisagree() {
        // Given a sidecar saying full and the flag saying sampled
        let side = sidecar_saying("full");
        // When resolved
        let (value, warnings) = resolve_kill_flag(Some(&side), Some(KillCompletenessArg::Sampled));
        // Then the less complete value wins and a warning names both
        assert_eq!(value, KillCompleteness::Sampled);
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].id, sidecar::WARN_KILL_COMPLETENESS_CONFLICT);
        let params: Vec<_> = warnings[0].params.iter().map(|(_, v)| v.as_str()).collect();
        assert_eq!(params, ["full", "sampled"]);
    }

    #[test]
    fn kill_completeness_flag_should_YieldToSidecar_When_FlagIsMoreComplete() {
        // Given a sidecar saying sampled and the flag saying full
        let side = sidecar_saying("sampled");
        let (value, warnings) = resolve_kill_flag(Some(&side), Some(KillCompletenessArg::Full));
        // Then the less complete sidecar value wins, still with the warning
        assert_eq!(value, KillCompleteness::Sampled);
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn kill_completeness_flag_should_StandAlone_When_NoSidecar() {
        let (value, warnings) = resolve_kill_flag(None, Some(KillCompletenessArg::FirstKiller));
        assert_eq!(value, KillCompleteness::FirstKillerOnly);
        assert!(warnings.is_empty());
        let (value, _) = resolve_kill_flag(None, None);
        assert_eq!(value, KillCompleteness::Full);
    }

    #[test]
    fn flag_spellings_should_MapToDomainValues() {
        assert_eq!(
            KillCompletenessArg::BudgetLimited.into_domain(),
            KillCompleteness::BudgetLimited
        );
        assert_eq!(
            AttributionArg::LeaveOneOut.into_domain(),
            CoverageAttribution::LeaveOneOut
        );
        assert_eq!(AttributionArg::InOrder.into_domain().label(), "in-order");
    }
}
