//! Unit tests for `analyze`: the pure step from a loaded matrix to a report.

#![allow(non_snake_case)]
use super::*;
use crate::test_effectiveness::confidence::{G2_VERDICTS, Signal, Verdict};
use crate::test_effectiveness::matrix::test_support::{cov, kills, tid};
use crate::test_effectiveness::report::json;

fn located(names: &[&str]) -> BTreeMap<TestId, Located> {
    names
        .iter()
        .enumerate()
        .map(|(i, n)| {
            (
                tid(n),
                Located {
                    file: "t.go".to_string(),
                    line: i + 1,
                    kind: TestKind::Unit,
                },
            )
        })
        .collect()
}

fn loaded(
    coverage: Option<CoverageDimension>,
    kills: Option<super::super::matrix::KillDimension>,
) -> Loaded {
    let mut l = Loaded::default();
    if coverage.is_some() {
        l.coverage_input = Some(0);
    }
    if kills.is_some() {
        l.kills_input = Some(usize::from(coverage.is_some()));
    }
    l.matrix = TestMatrix { coverage, kills };
    l
}

fn opts(verdicts: &[(Signal, Verdict)]) -> AnalyzeOptions<'_> {
    AnalyzeOptions {
        format: Format::Json,
        experimental: false,
        verdicts,
    }
}

const DUPLICATES: &[(&str, &[u32])] = &[("A", &[1]), ("B", &[1]), ("C", &[2])];

#[test]
fn analyze_should_FlagTestsWithNoUniqueCoverage_And_MarkFlaggedPartners() {
    let l = loaded(Some(cov(DUPLICATES)), None);
    let r = analyze(&l, &located(&["A", "B", "C"]), &opts(G2_VERDICTS));
    let ids: Vec<&str> = r.findings.iter().map(|f| f.test_id.as_str()).collect();
    assert_eq!(ids, ["t.go::A", "t.go::B"]);
    assert_eq!(r.findings[0].overlaps_with, ["t.go::B"]);
    assert_eq!(r.findings[0].overlaps_also_flagged, ["t.go::B"]);
    assert_eq!(r.envelope.summary.individually_flagged, 2);
    assert_eq!(r.envelope.summary.redundant_only_with_flagged, 2);
    assert_eq!(r.flagged[&Signal::ZeroUniqueCoverage].len(), 2);
}

#[test]
fn analyze_should_KeepUnflaggedPartnerOutOfAlsoFlagged_When_PartnerHasUniqueCoverage() {
    // B covers 1 and 2; A covers only 1; A is flagged, B is not.
    let l = loaded(Some(cov(&[("A", &[1]), ("B", &[1, 2])])), None);
    let r = analyze(&l, &located(&["A", "B"]), &opts(G2_VERDICTS));
    assert_eq!(r.findings.len(), 1);
    assert_eq!(r.findings[0].overlaps_with, ["t.go::B"]);
    assert!(r.findings[0].overlaps_also_flagged.is_empty());
    assert_eq!(r.envelope.summary.redundant_only_with_flagged, 0);
}

#[test]
fn analyze_should_ExcludeTestsWithoutSourceAndCountThem() {
    let l = loaded(Some(cov(DUPLICATES)), None);
    // C has no source: it is a no-source exclusion, and A/B keep their findings.
    let r = analyze(&l, &located(&["A", "B"]), &opts(G2_VERDICTS));
    assert_eq!(r.envelope.header.scope.tests_in_input, 3);
    assert_eq!(r.envelope.header.scope.not_assessed(), 1);
    assert_eq!(r.not_assessed[0].id, "t.go::C");
    assert_eq!(r.not_assessed[0].file, None);
    assert_eq!(r.universe.len(), 3);
}

#[test]
fn analyze_should_FlagKillOnlyTests_And_ReadmitThemFromNoData() {
    let k = kills(&[("A", &["m1"]), ("B", &["m1"]), ("C", &["m2"])]);
    let l = loaded(None, Some(k));
    let r = analyze(&l, &located(&["A", "B", "C"]), &opts(G2_VERDICTS));
    assert_eq!(r.envelope.header.scope.not_assessed(), 0);
    let rules: Vec<_> = r.findings.iter().map(|f| f.rule).collect();
    assert_eq!(rules, [Signal::ZeroUniqueKills, Signal::ZeroUniqueKills]);
    assert_eq!(r.findings[0].attribution, None);
    assert!(matches!(
        r.findings[0].value,
        FindingValue::UniqueKills {
            unique_mutants: 0,
            killed_mutants: 1
        }
    ));
}

#[test]
fn analyze_should_MoveCappedRowsToExperimentalSection_When_IntegrationKind() {
    let l = loaded(Some(cov(DUPLICATES)), None);
    let mut locs = located(&["A", "B", "C"]);
    locs.get_mut(&tid("A")).unwrap().kind = TestKind::Integration;
    let r = analyze(&l, &locs, &opts(G2_VERDICTS));
    let a = r.findings.iter().find(|f| f.test.name == "A");
    assert!(
        a.is_none(),
        "an experimental row is hidden without the flag"
    );
    let with = AnalyzeOptions {
        experimental: true,
        ..opts(G2_VERDICTS)
    };
    let r = analyze(&l, &locs, &with);
    let a = r.findings.iter().find(|f| f.test.name == "A").unwrap();
    assert_eq!(a.section, Section::Experimental);
    assert_eq!(a.confidence, Tier::Low);
    assert_eq!(
        r.findings.last().unwrap().test.name,
        "A",
        "experimental rows rank last"
    );
}

#[test]
fn report_mode_should_BeLeastValidatedSignalModeAmongMainRows_When_VerdictsMixed() {
    let l = loaded(
        Some(cov(DUPLICATES)),
        Some(kills(&[("A", &["m1"]), ("B", &["m1"])])),
    );
    let mixed = [
        (Signal::ZeroUniqueCoverage, Verdict::Tiered),
        (Signal::ZeroUniqueKills, Verdict::Descriptive),
    ];
    let r = analyze(&l, &located(&["A", "B", "C"]), &opts(&mixed));
    assert_eq!(r.envelope.mode, ReportMode::Descriptive);
    let all_tiered = [
        (Signal::ZeroUniqueCoverage, Verdict::Tiered),
        (Signal::ZeroUniqueKills, Verdict::Tiered),
    ];
    let r = analyze(&l, &located(&["A", "B", "C"]), &opts(&all_tiered));
    assert_eq!(r.envelope.mode, ReportMode::Tiered);
}

#[test]
fn report_json_should_OmitConfidenceAndNameListed_When_ModeIsReportOnlyOrDescriptive() {
    let l = loaded(Some(cov(DUPLICATES)), None);
    for verdict in [Verdict::Descriptive, Verdict::ReportOnly] {
        let table = [(Signal::ZeroUniqueCoverage, verdict)];
        let r = analyze(&l, &located(&["A", "B", "C"]), &opts(&table));
        let doc: serde_json::Value = serde_json::from_str(&json::render(&r)).unwrap();
        assert_eq!(doc["mode"], verdict.signal_mode().as_str());
        assert!(doc["findings"][0].get("confidence").is_none());
        assert_eq!(doc["summary"]["listed"], 2);
        assert!(doc["summary"].get("redundant_only_with_flagged").is_none());
        // Rows follow test-id order in these modes.
        let ids: Vec<_> = doc["findings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["test"]["id"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(ids, ["t.go::A", "t.go::B"]);
    }
}

#[test]
fn ranking_should_FollowTierThenRuleThenTestId_And_IgnoreCost() {
    let l = loaded(
        Some(cov(&[("B", &[1]), ("A", &[1]), ("C", &[2])])),
        Some(kills(&[("A", &["m1"]), ("B", &["m1"]), ("C", &["m2"])])),
    );
    let mut r = analyze(&l, &located(&["A", "B", "C"]), &opts(G2_VERDICTS));
    for (i, f) in r.findings.iter_mut().enumerate() {
        f.cost_seconds = Some(if i % 2 == 0 { 0.01 } else { 0.2 });
    }
    let order: Vec<_> = r
        .findings
        .iter()
        .map(|f| (f.rule.rule_id(), f.test.name.as_str()))
        .collect();
    assert_eq!(
        order,
        [
            ("test-zero-unique-coverage", "A"),
            ("test-zero-unique-coverage", "B"),
            ("test-zero-unique-kills", "A"),
            ("test-zero-unique-kills", "B"),
        ]
    );
}

#[test]
fn analyze_should_ReportGoTestsByPackage_And_UseSourcePathInIds() {
    let mut l = loaded(Some(cov(DUPLICATES)), None);
    l.go_tests.insert(tid("A"));
    let r = analyze(&l, &located(&["A", "B", "C"]), &opts(G2_VERDICTS));
    let a = r.universe.iter().find(|t| t.test == tid("A")).unwrap();
    assert_eq!(a.package, "t.go");
    let b = r.universe.iter().find(|t| t.test == tid("B")).unwrap();
    assert_eq!(b.package, "");
}
