//! Which tests are not assessed, and why (Story 2.1.3): failing, skipped, generated, no-data, stale, no-source and
//! unstable tests are excluded explicitly so the absence of a flag is never read as endorsement.
//! Pure functions: the changed-file set and test states are injected, never computed here.

use std::collections::{BTreeMap, BTreeSet};

use super::matrix::{CoverageDimension, CoverageUnit, TestId, TestMatrix, Tier};
use crate::test_code::TestKind;

/// Why a test was not assessed. A single input is deliberately not a reason: see `Assessment::stability_footer`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ExclusionReason {
    Flaky,
    Failing,
    Skipped,
    Generated,
    NoData,
    Stale,
    NoSource,
}

impl ExclusionReason {
    pub const ALL: [ExclusionReason; 7] = [
        Self::Failing,
        Self::Skipped,
        Self::Generated,
        Self::NoData,
        Self::Stale,
        Self::NoSource,
        Self::Flaky,
    ];

    /// The observation, not a cause: `Flaky` prints as `inputs disagree`.
    pub fn label(self) -> &'static str {
        match self {
            Self::Flaky => "inputs disagree",
            Self::Failing => "failing",
            Self::Skipped => "skipped",
            Self::Generated => "generated",
            Self::NoData => "no-data",
            Self::Stale => "stale",
            Self::NoSource => "no-source",
        }
    }
}

/// Run outcome and provenance of the test file, as reported by the input or the repo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestState {
    Ok,
    Failing,
    Skipped,
    Generated,
}

/// A test as the exclusion model sees it. `source_file` is `None` when the id resolves to no unique file and line
/// (`NotFound` or `Ambiguous`).
#[derive(Debug, Clone)]
pub struct CandidateTest {
    pub id: TestId,
    pub kind: TestKind,
    pub state: TestState,
    pub source_file: Option<String>,
}

impl CandidateTest {
    pub fn new(id: TestId, kind: TestKind, source_file: Option<&str>) -> Self {
        Self {
            id,
            kind,
            state: TestState::Ok,
            source_file: source_file.map(str::to_owned),
        }
    }
}

pub const SINGLE_RUN_FOOTER: &str = "Stability not assessed: single run";

#[derive(Debug, Clone, Default)]
pub struct Assessment {
    pub assessed: Vec<TestId>,
    pub excluded: BTreeMap<TestId, ExclusionReason>,
    /// Number of independent inputs the stability check compared.
    pub inputs: usize,
}

impl Assessment {
    pub fn total(&self) -> usize {
        self.assessed.len() + self.excluded.len()
    }

    /// Every reason with its count, zeros included, so the parts sum to the number not assessed.
    pub fn counts(&self) -> Vec<(ExclusionReason, usize)> {
        ExclusionReason::ALL
            .iter()
            .map(|&r| (r, self.excluded.values().filter(|&&x| x == r).count()))
            .collect()
    }

    /// `Not assessed: failing 0, skipped 0, ...` with the report labels.
    pub fn not_assessed_line(&self) -> String {
        let parts: Vec<String> = self
            .counts()
            .iter()
            .map(|(r, n)| format!("{} {n}", r.label()))
            .collect();
        format!("Not assessed: {}", parts.join(", "))
    }

    /// A single input is a footer note, never an exclusion and never a tier cap; no test is labeled stable.
    pub fn stability_footer(&self) -> Option<&'static str> {
        (self.inputs == 1).then_some(SINGLE_RUN_FOOTER)
    }
}

fn unique_units_of(d: &CoverageDimension, test: &TestId) -> BTreeSet<CoverageUnit> {
    let Some(ordinal) = d.ordinal_of(test) else {
        return BTreeSet::new();
    };
    d.singleton_units()
        .filter(|&(_, owner)| owner == ordinal)
        .map(|(u, _)| u.clone())
        .collect()
}

fn covered_files<'a>(d: &'a CoverageDimension, test: &TestId) -> BTreeSet<&'a str> {
    let Some(ordinal) = d.ordinal_of(test) else {
        return BTreeSet::new();
    };
    d.index
        .iter()
        .filter(|(_, p)| p.tests.binary_search(&ordinal).is_ok())
        .map(|(u, _)| u.file.as_str())
        .collect()
}

fn has_data(inputs: &[&CoverageDimension], test: &TestId) -> bool {
    inputs.iter().any(|d| !covered_files(d, test).is_empty())
}

fn is_stale(
    test: &CandidateTest,
    inputs: &[&CoverageDimension],
    changed_files: &BTreeSet<String>,
) -> bool {
    let own = test.source_file.as_deref().unwrap_or(&test.id.file);
    changed_files.contains(own)
        || inputs.iter().any(|d| {
            covered_files(d, &test.id)
                .iter()
                .any(|f| changed_files.contains(*f))
        })
}

fn disagrees(inputs: &[&CoverageDimension], test: &TestId) -> bool {
    let mut it = inputs.iter().map(|d| unique_units_of(d, test));
    let Some(first) = it.next() else { return false };
    it.any(|u| u != first)
}

/// Classifies each candidate once. Precedence when several reasons apply: generated, skipped, failing, no-source,
/// stale, no-data, then inputs-disagree (needs two or more inputs).
pub fn assess(
    tests: &[CandidateTest],
    inputs: &[&CoverageDimension],
    changed_files: &BTreeSet<String>,
) -> Assessment {
    let mut out = Assessment {
        inputs: inputs.len(),
        ..Assessment::default()
    };
    for t in tests {
        let reason = match t.state {
            TestState::Generated => Some(ExclusionReason::Generated),
            TestState::Skipped => Some(ExclusionReason::Skipped),
            TestState::Failing => Some(ExclusionReason::Failing),
            TestState::Ok if t.source_file.is_none() => Some(ExclusionReason::NoSource),
            TestState::Ok if is_stale(t, inputs, changed_files) => Some(ExclusionReason::Stale),
            TestState::Ok if !has_data(inputs, &t.id) => Some(ExclusionReason::NoData),
            TestState::Ok if inputs.len() > 1 && disagrees(inputs, &t.id) => {
                Some(ExclusionReason::Flaky)
            }
            TestState::Ok => None,
        };
        match reason {
            Some(r) => {
                out.excluded.insert(t.id.clone(), r);
            }
            None => out.assessed.push(t.id.clone()),
        }
    }
    out
}

/// Upper bound on a coverage-derived finding from the test's kind alone: integration and snapshot tests are
/// never flagged above `Low` by coverage (pitfalls P8, P10).
pub fn kind_cap(kind: TestKind) -> Option<Tier> {
    matches!(kind, TestKind::Integration | TestKind::Snapshot).then_some(Tier::Low)
}

/// Tightest cap for a coverage-derived finding: kind, the coverage dimension's own cap, and `Medium` while no
/// kill dimension is supplied (coverage is a stand-in, Q3).
pub fn coverage_finding_cap(kind: TestKind, matrix: &TestMatrix) -> Tier {
    [
        kind_cap(kind),
        matrix.coverage.as_ref().and_then(|d| d.cap),
        matrix.kills.is_none().then_some(Tier::Medium),
    ]
    .into_iter()
    .flatten()
    .min()
    .unwrap_or(Tier::High)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubtestMode {
    Collapse,
    Expand,
}

/// Collapses table-driven subtests (`TestParse/case_1`, `test_x[param]`) into their parent test unless
/// `SubtestMode::Expand` (`--subtests`).
pub fn candidates(ids: &[TestId], mode: SubtestMode) -> Vec<TestId> {
    let mut out: BTreeSet<TestId> = BTreeSet::new();
    for id in ids {
        if mode == SubtestMode::Expand {
            out.insert(id.clone());
            continue;
        }
        let name = id.name.split(['/', '[']).next().unwrap_or(&id.name);
        out.insert(TestId::new(id.file.clone(), name));
    }
    out.into_iter().collect()
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::super::matrix::test_support::*;
    use super::super::matrix::{CoverageBuilder, CoverageUnit};
    use super::*;

    fn cand(name: &str) -> CandidateTest {
        CandidateTest::new(tid(name), TestKind::Unit, Some("t.go"))
    }

    fn files(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|n| (*n).to_owned()).collect()
    }

    fn none() -> BTreeSet<String> {
        BTreeSet::new()
    }

    #[test]
    fn exclusion_should_MarkFlaky_When_UniqueCoverageDiffersAcrossTwoInputs() {
        let a = cov(&[("TestC", &[1, 2]), ("TestD", &[3])]);
        let b = cov(&[("TestC", &[1]), ("TestD", &[3])]);
        let r = assess(&[cand("TestC"), cand("TestD")], &[&a, &b], &none());
        assert_eq!(r.excluded[&tid("TestC")], ExclusionReason::Flaky);
        assert_eq!(r.assessed, vec![tid("TestD")]);
        assert_eq!(ExclusionReason::Flaky.label(), "inputs disagree");
        assert_eq!(r.stability_footer(), None);
    }

    #[test]
    fn stability_should_PrintFooterNoteAndExcludeNothing_When_OnlyOneInput() {
        let names: Vec<String> = (0..212).map(|i| format!("Test{i:03}")).collect();
        let rows: Vec<(&str, &[u32])> = names.iter().map(|n| (n.as_str(), &[1u32][..])).collect();
        let d = cov(&rows);
        let tests: Vec<_> = names.iter().map(|n| cand(n)).collect();
        let r = assess(&tests, &[&d], &none());
        assert_eq!(r.assessed.len(), 212);
        assert!(r.excluded.is_empty());
        assert_eq!(
            r.stability_footer(),
            Some("Stability not assessed: single run")
        );
        assert!(!r.not_assessed_line().contains("single"));
    }

    #[test]
    fn exclusion_reason_should_NotContainSingleRun_When_EnumInspected() {
        let labels: Vec<_> = ExclusionReason::ALL.iter().map(|r| r.label()).collect();
        assert_eq!(labels.len(), 7);
        assert!(labels.iter().all(|l| !l.contains("single")));
        // Exhaustive match: adding a variant breaks the build here until ALL and label are updated.
        for r in ExclusionReason::ALL {
            match r {
                ExclusionReason::Flaky
                | ExclusionReason::Failing
                | ExclusionReason::Skipped
                | ExclusionReason::Generated
                | ExclusionReason::NoData
                | ExclusionReason::Stale
                | ExclusionReason::NoSource => {}
            }
        }
    }

    #[test]
    fn kind_should_CapAtLow_When_IntegrationOrSnapshotZeroUniqueCoverage() {
        let id = TestId::new("tests/e2e/login_test.go", "TestLogin");
        let mut b = CoverageBuilder::coverage(prov(), isolated());
        b.record(&id, CoverageUnit::new("p.go", 1), 1);
        b.record(&tid("Other"), CoverageUnit::new("p.go", 1), 1);
        let m = TestMatrix {
            coverage: Some(b.build().unwrap()),
            kills: Some(kills(&[("Other", &["m"])])),
        };
        assert_eq!(coverage_finding_cap(TestKind::Integration, &m), Tier::Low);
        assert_eq!(coverage_finding_cap(TestKind::Snapshot, &m), Tier::Low);
        assert_eq!(coverage_finding_cap(TestKind::Unit, &m), Tier::High);
        let no_kills = TestMatrix { kills: None, ..m };
        assert_eq!(
            coverage_finding_cap(TestKind::Unit, &no_kills),
            Tier::Medium
        );
        assert_eq!(kind_cap(TestKind::Fuzz), None);
    }

    #[test]
    fn candidates_should_BeOneTest_When_GoTestHas40SubtestsAndNoSubtestsFlag() {
        let mut ids: Vec<TestId> = (0..40)
            .map(|i| TestId::new("parse_test.go", format!("TestParse/case_{i}")))
            .collect();
        ids.push(TestId::new("parse_test.go", "TestParse"));
        assert_eq!(
            candidates(&ids, SubtestMode::Collapse),
            vec![TestId::new("parse_test.go", "TestParse")]
        );
    }

    #[test]
    fn candidates_should_ExpandToSubtests_When_SubtestsFlagGiven() {
        let ids: Vec<TestId> = (0..40)
            .map(|i| TestId::new("parse_test.go", format!("TestParse/case_{i}")))
            .collect();
        assert_eq!(candidates(&ids, SubtestMode::Expand).len(), 40);
        let py = [
            TestId::new("t.py", "test_x[a]"),
            TestId::new("t.py", "test_x[b]"),
        ];
        assert_eq!(
            candidates(&py, SubtestMode::Collapse),
            vec![TestId::new("t.py", "test_x")]
        );
    }

    #[test]
    fn exclusion_should_MarkStale_When_InjectedChangedFilesContainsTestFile() {
        let d = cov(&[("TestA", &[1]), ("TestB", &[2])]);
        let mut in_changed = cand("TestA");
        in_changed.source_file = Some("a_test.go".into());
        let r = assess(&[in_changed, cand("TestB")], &[&d], &files(&["a_test.go"]));
        assert_eq!(r.excluded[&tid("TestA")], ExclusionReason::Stale);
        assert_eq!(r.assessed, vec![tid("TestB")]);
        assert!(r.counts().contains(&(ExclusionReason::Stale, 1)));
    }

    #[test]
    fn exclusion_should_MarkStale_When_TestFileChangedSinceInputCommit() {
        let d = cov(&[("TestA", &[1])]);
        let r = assess(&[cand("TestA")], &[&d], &files(&["p.go"]));
        assert_eq!(
            r.excluded[&tid("TestA")],
            ExclusionReason::Stale,
            "covered production file changed"
        );
        let r = assess(&[cand("TestA")], &[&d], &files(&["other.go"]));
        assert_eq!(r.assessed.len(), 1);
    }

    #[test]
    fn exclusion_should_MarkNoSourceAndExitZero_When_14IdsResolveNowhere() {
        let d = cov(&[("Good", &[1])]);
        let mut tests = vec![cand("Good")];
        tests.extend(
            (0..14).map(|i| CandidateTest::new(tid(&format!("Ghost{i}")), TestKind::Unit, None)),
        );
        let r = assess(&tests, &[&d], &none());
        assert_eq!(r.assessed, vec![tid("Good")]);
        assert!(r.counts().contains(&(ExclusionReason::NoSource, 14)));
        assert!(r.not_assessed_line().contains("no-source 14"));
        assert!(r.not_assessed_line().contains("stale 0"));
        assert_eq!(r.total(), 15);
        assert_eq!(r.assessed.len() + r.excluded.len(), tests.len());
    }

    #[test]
    fn assess_should_ApplyStateAndNoDataReasons_When_MixedTests() {
        let d = cov(&[("Ok", &[1]), ("Empty", &[])]);
        let mut failing = cand("Failing");
        failing.state = TestState::Failing;
        let mut skipped = cand("Skipped");
        skipped.state = TestState::Skipped;
        let mut generated = cand("Gen");
        generated.state = TestState::Generated;
        let tests = [
            cand("Ok"),
            cand("Empty"),
            cand("Absent"),
            failing,
            skipped,
            generated,
        ];
        let r = assess(&tests, &[&d], &none());
        assert_eq!(r.excluded[&tid("Empty")], ExclusionReason::NoData);
        assert_eq!(r.excluded[&tid("Absent")], ExclusionReason::NoData);
        assert_eq!(r.excluded[&tid("Failing")], ExclusionReason::Failing);
        assert_eq!(r.excluded[&tid("Skipped")], ExclusionReason::Skipped);
        assert_eq!(r.excluded[&tid("Gen")], ExclusionReason::Generated);
        let sum: usize = r.counts().iter().map(|(_, n)| n).sum();
        assert_eq!(sum, r.excluded.len());
    }
}
