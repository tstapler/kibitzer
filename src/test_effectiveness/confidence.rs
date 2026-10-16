//! Pre-G2 `ConfidenceTier` derivation and the G2-verdict table stub (Task 2.3.1k).
//!
//! A tier is a base tier per signal class, lowered by the structural caps of the plan's "Confidence caps" table.
//! Caps only ever lower a tier. This module invents no numeric boundary: the measured boundaries arrive with
//! `tiers.rs` (Story 4.2.1), and `G2_VERDICTS` is replaced by Task 4.2.1c.

use crate::test_code::TestKind;

use super::matrix::{CoverageAttribution, KillCompleteness, Tier};

/// How validated the shown signals are, ordered least to most: the derived `Ord` is the least-validated ordering
/// (`experimental` < `descriptive` < `report_only` < `tiered`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ReportMode {
    Experimental,
    Descriptive,
    ReportOnly,
    Tiered,
}

impl ReportMode {
    /// The JSON/config spelling (`report_only`, never hyphenated).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Experimental => "experimental",
            Self::Descriptive => "descriptive",
            Self::ReportOnly => "report_only",
            Self::Tiered => "tiered",
        }
    }

    /// Top-level mode: the least-validated mode among main-section rows; `Experimental` when there are none
    /// (no G2 verdict exists to speak for an empty list).
    pub fn least_validated(row_modes: impl IntoIterator<Item = ReportMode>) -> ReportMode {
        row_modes.into_iter().min().unwrap_or(Self::Experimental)
    }

    /// `confidence` is omitted from every record in these two modes.
    pub fn shows_confidence(self) -> bool {
        matches!(self, Self::Experimental | Self::Tiered)
    }
}

/// Finding kinds (plan Glossary `Signal`); each has one rule id string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Signal {
    NoAssertion,
    Tautology,
    MockOnly,
    ZeroUniqueCoverage,
    ZeroUniqueKills,
    PseudoTestedRegion,
}

impl Signal {
    pub const ALL: [Signal; 6] = [
        Self::NoAssertion,
        Self::Tautology,
        Self::MockOnly,
        Self::ZeroUniqueCoverage,
        Self::ZeroUniqueKills,
        Self::PseudoTestedRegion,
    ];

    /// The rule-id registry string (`[rule]` in text, `rule` in JSON).
    pub fn rule_id(self) -> &'static str {
        match self {
            Self::NoAssertion => "test-no-assertion",
            Self::Tautology => "test-tautology",
            Self::MockOnly => "test-mock-only",
            Self::ZeroUniqueCoverage => "test-zero-unique-coverage",
            Self::ZeroUniqueKills => "test-zero-unique-kills",
            Self::PseudoTestedRegion => "test-pseudo-tested-region",
        }
    }

    fn is_static(self) -> bool {
        matches!(self, Self::NoAssertion | Self::Tautology | Self::MockOnly)
    }
}

/// The G2 decision for one signal. `None` until `G2-decision.md` records one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    None,
    Descriptive,
    ReportOnly,
    Tiered,
}

impl Verdict {
    /// `experimental` means no verdict is recorded for the signal.
    pub fn signal_mode(self) -> ReportMode {
        match self {
            Self::None => ReportMode::Experimental,
            Self::Descriptive => ReportMode::Descriptive,
            Self::ReportOnly => ReportMode::ReportOnly,
            Self::Tiered => ReportMode::Tiered,
        }
    }
}

/// Stub of the G2 verdict table: every signal is `Verdict::None` until Task 4.2.1c fills it in.
pub const G2_VERDICTS: &[(Signal, Verdict)] = &[
    (Signal::NoAssertion, Verdict::None),
    (Signal::Tautology, Verdict::None),
    (Signal::MockOnly, Verdict::None),
    (Signal::ZeroUniqueCoverage, Verdict::None),
    (Signal::ZeroUniqueKills, Verdict::None),
    (Signal::PseudoTestedRegion, Verdict::None),
];

pub fn verdict_in(table: &[(Signal, Verdict)], signal: Signal) -> Verdict {
    table
        .iter()
        .find(|(s, _)| *s == signal)
        .map_or(Verdict::None, |&(_, v)| v)
}

/// Owner-pending proposals (template line A4), not measured: static `test-no-assertion` and `test-tautology`
/// start `High`, `test-mock-only` starts `Medium`, every dynamic signal starts `Medium`.
pub fn base_tier(signal: Signal) -> Tier {
    match signal {
        Signal::NoAssertion | Signal::Tautology => Tier::High,
        Signal::MockOnly
        | Signal::ZeroUniqueCoverage
        | Signal::ZeroUniqueKills
        | Signal::PseudoTestedRegion => Tier::Medium,
    }
}

/// The facts the structural caps read.
#[derive(Debug, Clone, Copy)]
pub struct CapInputs {
    pub kind: TestKind,
    /// Attribution of the coverage dimension the finding came from.
    pub attribution: Option<CoverageAttribution>,
    /// `None` when no kill dimension was supplied.
    pub kill_completeness: Option<KillCompleteness>,
    pub pit_without_full_matrix: bool,
    pub commit_known: bool,
    /// The producing tool's schema version is not on the adapter's allow-list.
    pub unlisted_schema: bool,
}

/// Base tier for `signal`, then every applicable cap; the result is never above the base tier.
pub fn derive_tier(signal: Signal, c: &CapInputs) -> Tier {
    let coverage_derived = signal == Signal::ZeroUniqueCoverage;
    let mut caps: Vec<Tier> = Vec::new();
    if coverage_derived {
        if matches!(c.kind, TestKind::Integration | TestKind::Snapshot) {
            caps.push(Tier::Low);
        }
        if c.attribution == Some(CoverageAttribution::InOrder) {
            caps.push(Tier::Medium);
        }
        if c.kill_completeness.is_none() {
            caps.push(Tier::Medium);
        }
    }
    if signal == Signal::ZeroUniqueKills
        && (c
            .kill_completeness
            .is_some_and(|k| k != KillCompleteness::Full)
            || c.pit_without_full_matrix)
    {
        caps.push(Tier::Low);
    }
    if !signal.is_static() && !c.commit_known {
        caps.push(Tier::Low);
    }
    if c.unlisted_schema {
        caps.push(Tier::Low);
    }
    caps.into_iter().fold(base_tier(signal), Tier::min)
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;

    fn clean() -> CapInputs {
        CapInputs {
            kind: TestKind::Unit,
            attribution: Some(CoverageAttribution::Isolated),
            kill_completeness: Some(KillCompleteness::Full),
            pit_without_full_matrix: false,
            commit_known: true,
            unlisted_schema: false,
        }
    }

    #[test]
    fn confidence_should_ApplyCapsWithoutRaising_When_InOrderAndNoKillDimension() {
        // Given in-order attribution
        let in_order = CapInputs {
            attribution: Some(CoverageAttribution::InOrder),
            ..clean()
        };
        // Then the coverage finding is Medium, not above its Medium base
        assert_eq!(
            derive_tier(Signal::ZeroUniqueCoverage, &in_order),
            Tier::Medium
        );
        // And no kill dimension also gives Medium
        let no_kills = CapInputs {
            kill_completeness: None,
            ..clean()
        };
        assert_eq!(
            derive_tier(Signal::ZeroUniqueCoverage, &no_kills),
            Tier::Medium
        );
        // And an unknown commit lowers to Low, and a clean input never raises past the base tier
        let no_commit = CapInputs {
            commit_known: false,
            ..in_order
        };
        assert_eq!(
            derive_tier(Signal::ZeroUniqueCoverage, &no_commit),
            Tier::Low
        );
        for s in Signal::ALL {
            assert!(derive_tier(s, &clean()) <= base_tier(s));
        }
    }

    #[test]
    fn confidence_should_CapLow_When_IntegrationKindOrIncompleteKillsOrUnlistedSchema() {
        let integ = CapInputs {
            kind: TestKind::Integration,
            ..clean()
        };
        assert_eq!(derive_tier(Signal::ZeroUniqueCoverage, &integ), Tier::Low);
        let partial = CapInputs {
            kill_completeness: Some(KillCompleteness::Sampled),
            ..clean()
        };
        assert_eq!(derive_tier(Signal::ZeroUniqueKills, &partial), Tier::Low);
        let pit = CapInputs {
            pit_without_full_matrix: true,
            ..clean()
        };
        assert_eq!(derive_tier(Signal::ZeroUniqueKills, &pit), Tier::Low);
        let schema = CapInputs {
            unlisted_schema: true,
            ..clean()
        };
        assert_eq!(derive_tier(Signal::NoAssertion, &schema), Tier::Low);
        // A static signal ignores the commit cap and the coverage-only caps
        let hostile = CapInputs {
            commit_known: false,
            kind: TestKind::Snapshot,
            ..clean()
        };
        assert_eq!(derive_tier(Signal::NoAssertion, &hostile), Tier::High);
        assert_eq!(derive_tier(Signal::MockOnly, &hostile), Tier::Medium);
    }

    #[test]
    fn verdict_table_should_DefaultAllSignalsToExperimental_When_NoG2Decision() {
        for s in Signal::ALL {
            assert_eq!(
                verdict_in(G2_VERDICTS, s).signal_mode(),
                ReportMode::Experimental
            );
        }
        assert_eq!(G2_VERDICTS.len(), Signal::ALL.len());
        assert_eq!(
            verdict_in(&[(Signal::MockOnly, Verdict::Tiered)], Signal::MockOnly).signal_mode(),
            ReportMode::Tiered
        );
    }

    #[test]
    fn report_mode_should_BeLeastValidatedAmongRows_When_VerdictsMixed() {
        use ReportMode::*;
        assert_eq!(
            ReportMode::least_validated([Tiered, Descriptive]),
            Descriptive
        );
        assert_eq!(
            ReportMode::least_validated([Tiered, ReportOnly]),
            ReportOnly
        );
        assert_eq!(ReportMode::least_validated([]), Experimental);
        assert!(!ReportOnly.shows_confidence() && !Descriptive.shows_confidence());
        assert!(Tiered.shows_confidence() && Experimental.shows_confidence());
        assert_eq!(ReportOnly.as_str(), "report_only");
    }

    #[test]
    fn rule_ids_should_match_registry() {
        let ids: Vec<_> = Signal::ALL.iter().map(|s| s.rule_id()).collect();
        assert_eq!(ids[3], "test-zero-unique-coverage");
        assert_eq!(ids[4], "test-zero-unique-kills");
    }
}
