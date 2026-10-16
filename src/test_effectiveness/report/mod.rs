//! Report-surface submodules; one file per concern so Tasks 2.3.x do not contend for one file.
//!
//! This file holds the data model (`TestFinding`, `Report`) and `analyze`, the pure step from a loaded
//! `TestMatrix` to a report. Renderers (`json`, `text`) and the experiment export read a `Report`.
pub mod brief;
pub mod envelope;
pub mod export;
pub mod hygiene;
pub mod joint;
pub mod json;
pub mod ranking;
pub mod suppress;
pub mod text;

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use crate::test_code::TestKind;

use super::confidence::{CapInputs, ReportMode, Signal, Verdict, derive_tier, verdict_in};
use super::exclusions::{Assessment, CandidateTest, ExclusionReason, TestState, assess};
use super::matrix::{CoverageDimension, CoverageUnitKind, TestId, TestMatrix, Tier};
use envelope::{
    Format, Header, HeaderInput, Origin, ReportEnvelope, Scope, Summary, WarningDoc,
    describe_warning,
};

/// Which flag supplied an input, for `header.inputs[].flag`.
#[derive(Debug, Clone)]
pub struct InputInfo {
    pub header: HeaderInput,
    pub path: PathBuf,
}

/// Everything the adapters produced, before analysis.
#[derive(Debug, Default)]
pub struct Loaded {
    pub matrix: TestMatrix,
    pub inputs: Vec<InputInfo>,
    pub coverage_input: Option<usize>,
    pub kills_input: Option<usize>,
    pub failing: Vec<TestId>,
    /// Tests whose `TestId.file` is a Go import path, not a file.
    pub go_tests: BTreeSet<TestId>,
    pub extra_warnings: Vec<WarningDoc>,
}

/// Where a test's source was found. A test missing from the map has no source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Located {
    pub file: String,
    pub line: usize,
    pub kind: TestKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Main,
    Experimental,
}

impl Section {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Main => "main",
            Self::Experimental => "experimental",
        }
    }
}

/// The `value` object of a record, discriminated by `kind`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FindingValue {
    UniqueCoverage {
        unique_units: usize,
        covered_units: usize,
        unit_kind: &'static str,
    },
    UniqueKills {
        unique_mutants: usize,
        killed_mutants: usize,
    },
    Static {
        evidence: String,
    },
}

/// Report-surface record. `checker::Finding` is deliberately not widened (ADR-002).
#[derive(Debug, Clone, PartialEq)]
pub struct TestFinding {
    pub rule: Signal,
    pub section: Section,
    pub signal_mode: ReportMode,
    /// Index into `header.inputs`; `None` for static rules.
    pub input: Option<usize>,
    pub test: TestId,
    /// `<repo-relative path>::<qualified name>`.
    pub test_id: String,
    pub file: String,
    pub line: usize,
    pub value: FindingValue,
    pub baseline: String,
    pub attribution: Option<&'static str>,
    pub tool: String,
    pub tool_version: Option<String>,
    pub commit: Option<String>,
    pub confidence: Tier,
    pub cost_seconds: Option<f64>,
    pub overlaps_with: Vec<String>,
    pub overlaps_also_flagged: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotAssessed {
    pub id: String,
    pub file: Option<String>,
    pub line: Option<usize>,
    pub reason: ExclusionReason,
}

/// A test of the whole input universe, flagged or not (the export needs all of them).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UniverseTest {
    pub test: TestId,
    pub id: String,
    pub package: String,
    pub kind: TestKind,
}

#[derive(Debug)]
pub struct Report {
    pub envelope: ReportEnvelope,
    /// Ranking order; experimental-section rows are present only when requested.
    pub findings: Vec<TestFinding>,
    pub not_assessed: Vec<NotAssessed>,
    pub universe: Vec<UniverseTest>,
    /// Main-section, unsuppressed, per rule: the sets the export and the summary count.
    pub flagged: BTreeMap<Signal, Vec<TestId>>,
}

#[derive(Debug, Clone, Copy)]
pub struct AnalyzeOptions<'a> {
    pub format: Format,
    pub experimental: bool,
    pub verdicts: &'a [(Signal, Verdict)],
}

fn display_id(t: &TestId, locations: &BTreeMap<TestId, Located>) -> String {
    locations
        .get(t)
        .map_or_else(|| t.to_string(), |l| format!("{}::{}", l.file, t.name))
}

fn package_of(t: &TestId, loaded: &Loaded, locations: &BTreeMap<TestId, Located>) -> String {
    if loaded.go_tests.contains(t) {
        return t.file.clone();
    }
    let file = locations
        .get(t)
        .map_or(t.file.as_str(), |l| l.file.as_str());
    file.rsplit_once('/').map_or("", |(dir, _)| dir).to_string()
}

fn unit_kind_label(d: &CoverageDimension) -> &'static str {
    match d.unit_kind {
        Some(CoverageUnitKind::MutantSite) => "mutant_site",
        _ => "line",
    }
}

/// Per test ordinal: units owned alone, and units covered at all.
fn unit_counts<U: Ord, M: super::matrix::DimensionMeta>(
    d: &super::matrix::Dimension<U, M>,
) -> (Vec<usize>, Vec<usize>) {
    let mut unique = vec![0usize; d.tests.len()];
    let mut covered = vec![0usize; d.tests.len()];
    for posting in d.index.values() {
        for &t in &posting.tests {
            covered[t as usize] += 1;
        }
        if let [only] = posting.tests.as_slice() {
            unique[*only as usize] += 1;
        }
    }
    (unique, covered)
}

/// Tests sharing any unit with a flagged test, per flagged test ordinal.
fn overlap_map<U: Ord, M: super::matrix::DimensionMeta>(
    d: &super::matrix::Dimension<U, M>,
    flagged: &BTreeSet<u32>,
) -> BTreeMap<u32, BTreeSet<u32>> {
    let mut out: BTreeMap<u32, BTreeSet<u32>> =
        flagged.iter().map(|&t| (t, BTreeSet::new())).collect();
    for posting in d.index.values() {
        let mine: Vec<u32> = posting
            .tests
            .iter()
            .copied()
            .filter(|t| flagged.contains(t))
            .collect();
        for &f in &mine {
            let set = out.entry(f).or_default();
            set.extend(posting.tests.iter().copied().filter(|&t| t != f));
        }
    }
    out
}

struct Candidates {
    assessment: Assessment,
}

fn assess_universe(
    loaded: &Loaded,
    universe: &BTreeSet<TestId>,
    locations: &BTreeMap<TestId, Located>,
) -> Candidates {
    let candidates: Vec<CandidateTest> = universe
        .iter()
        .map(|t| {
            let loc = locations.get(t);
            let mut c = CandidateTest::new(
                t.clone(),
                loc.map_or(TestKind::Unit, |l| l.kind),
                loc.map(|l| l.file.as_str()),
            );
            if loaded.failing.contains(t) {
                c.state = TestState::Failing;
            }
            c
        })
        .collect();
    let dims: Vec<&CoverageDimension> = loaded.matrix.coverage.iter().collect();
    let mut assessment = assess(&candidates, &dims, &BTreeSet::new());
    // `assess` reads coverage only; a test present in the kill dimension has data there.
    if let Some(kills) = &loaded.matrix.kills {
        let readmit: Vec<TestId> = assessment
            .excluded
            .iter()
            .filter(|(t, r)| **r == ExclusionReason::NoData && kills.ordinal_of(t).is_some())
            .map(|(t, _)| t.clone())
            .collect();
        for t in readmit {
            assessment.excluded.remove(&t);
            assessment.assessed.push(t);
        }
        assessment.assessed.sort();
    }
    Candidates { assessment }
}

/// A finding before ranking: shared by the coverage and kill rules.
struct RuleInputs<'a> {
    loaded: &'a Loaded,
    locations: &'a BTreeMap<TestId, Located>,
    opts: &'a AnalyzeOptions<'a>,
}

impl RuleInputs<'_> {
    fn caps(&self, kind: TestKind, commit_known: bool, unlisted: bool) -> CapInputs {
        let m = &self.loaded.matrix;
        CapInputs {
            kind,
            attribution: m.coverage.as_ref().map(|c| c.meta),
            kill_completeness: m.kills.as_ref().map(|k| k.meta),
            pit_without_full_matrix: m
                .kills
                .as_ref()
                .is_some_and(|k| k.warnings.iter().any(|w| w.id == "full-matrix-not-enabled")),
            commit_known,
            unlisted_schema: unlisted,
        }
    }

    fn finish(
        &self,
        rule: Signal,
        test: &TestId,
        loc: &Located,
        spec: RecordSpec,
        overlaps: (Vec<String>, Vec<String>),
    ) -> TestFinding {
        let tier = derive_tier(
            rule,
            &self.caps(loc.kind, spec.commit.is_some(), spec.unlisted),
        );
        TestFinding {
            rule,
            section: if tier == Tier::Low {
                Section::Experimental
            } else {
                Section::Main
            },
            signal_mode: verdict_in(self.opts.verdicts, rule).signal_mode(),
            input: spec.input,
            test: test.clone(),
            test_id: format!("{}::{}", loc.file, test.name),
            file: loc.file.clone(),
            line: loc.line,
            value: spec.value,
            baseline: spec.baseline,
            attribution: spec.attribution,
            tool: spec.tool,
            tool_version: spec.tool_version,
            commit: spec.commit,
            confidence: tier,
            cost_seconds: None,
            overlaps_with: overlaps.0,
            overlaps_also_flagged: overlaps.1,
        }
    }
}

struct RecordSpec {
    input: Option<usize>,
    value: FindingValue,
    baseline: String,
    attribution: Option<&'static str>,
    tool: String,
    tool_version: Option<String>,
    commit: Option<String>,
    unlisted: bool,
}

fn zero_unique<U: Ord, M: super::matrix::DimensionMeta>(
    d: &super::matrix::Dimension<U, M>,
    assessed: &BTreeSet<&TestId>,
) -> (Vec<u32>, Vec<usize>, Vec<usize>) {
    let (unique, covered) = unit_counts(d);
    let flagged = d
        .tests
        .iter()
        .enumerate()
        .filter(|(i, t)| assessed.contains(t) && unique[*i] == 0)
        .map(|(i, _)| i as u32)
        .collect();
    (flagged, unique, covered)
}

/// Rule facts that differ between the coverage and the kill dimension.
struct RuleShape {
    signal: Signal,
    input: usize,
    baseline: String,
    attribution: Option<&'static str>,
    value: fn(unique: usize, total: usize, unit_kind: &'static str) -> FindingValue,
    unit_kind: &'static str,
}

/// One record per assessed test of `d` that owns no unit alone.
fn dimension_findings<U: Ord, M: super::matrix::DimensionMeta>(
    ctx: &RuleInputs,
    d: &super::matrix::Dimension<U, M>,
    shape: &RuleShape,
    assessed: &BTreeSet<&TestId>,
) -> Vec<TestFinding> {
    let (flagged, unique, total) = zero_unique(d, assessed);
    let flagged_set: BTreeSet<u32> = flagged.iter().copied().collect();
    let overlaps = overlap_map(d, &flagged_set);
    let id_of = |o: u32| display_id(d.test(o), ctx.locations);
    let mut out = Vec::new();
    for &o in &flagged {
        let test = d.test(o);
        let Some(loc) = ctx.locations.get(test) else {
            continue;
        };
        let partners = &overlaps[&o];
        let with = partners.iter().map(|&p| id_of(p)).collect();
        let also = partners
            .iter()
            .filter(|p| flagged_set.contains(p))
            .map(|&p| id_of(p))
            .collect();
        let spec = RecordSpec {
            input: Some(shape.input),
            value: (shape.value)(unique[o as usize], total[o as usize], shape.unit_kind),
            baseline: shape.baseline.clone(),
            attribution: shape.attribution,
            tool: d.provenance.tool.clone(),
            tool_version: d.provenance.version.clone(),
            commit: d.provenance.commit.clone(),
            unlisted: d.warnings.iter().any(|w| w.id == "unverified-tool-version"),
        };
        out.push(ctx.finish(shape.signal, test, loc, spec, (with, also)));
    }
    out
}

fn coverage_findings(ctx: &RuleInputs, assessed: &BTreeSet<&TestId>) -> Vec<TestFinding> {
    let (Some(d), Some(input)) = (&ctx.loaded.matrix.coverage, ctx.loaded.coverage_input) else {
        return Vec::new();
    };
    let shape = RuleShape {
        signal: Signal::ZeroUniqueCoverage,
        input,
        baseline: format!(
            "{} assessed tests; per-test coverage contexts",
            assessed.len()
        ),
        attribution: Some(d.meta.label()),
        value: |unique_units, covered_units, unit_kind| FindingValue::UniqueCoverage {
            unique_units,
            covered_units,
            unit_kind,
        },
        unit_kind: unit_kind_label(d),
    };
    dimension_findings(ctx, d, &shape, assessed)
}

fn kill_findings(ctx: &RuleInputs, assessed: &BTreeSet<&TestId>) -> Vec<TestFinding> {
    let (Some(d), Some(input)) = (&ctx.loaded.matrix.kills, ctx.loaded.kills_input) else {
        return Vec::new();
    };
    let shape = RuleShape {
        signal: Signal::ZeroUniqueKills,
        input,
        baseline: format!("{} mutants in report; {}", d.index.len(), d.meta.label()),
        attribution: None,
        value: |unique_mutants, killed_mutants, _| FindingValue::UniqueKills {
            unique_mutants,
            killed_mutants,
        },
        unit_kind: "mutant",
    };
    dimension_findings(ctx, d, &shape, assessed)
}

/// Ranking key: confidence tier (`High` first), then rule id, then test id; cost is never a key. Modes without a
/// confidence column order by test id alone.
fn rank(findings: &mut [TestFinding], mode: ReportMode) {
    findings.sort_by(|a, b| {
        let section =
            (a.section == Section::Experimental).cmp(&(b.section == Section::Experimental));
        let by_id = a.test_id.cmp(&b.test_id).then(a.rule.cmp(&b.rule));
        if mode.shows_confidence() {
            section
                .then(b.confidence.cmp(&a.confidence))
                .then(a.rule.rule_id().cmp(b.rule.rule_id()))
                .then(a.test_id.cmp(&b.test_id))
        } else {
            section.then(by_id)
        }
    });
}

fn redundant_only_with_flagged(findings: &[&TestFinding]) -> usize {
    findings
        .iter()
        .filter(|f| {
            !f.overlaps_with.is_empty()
                && f.overlaps_with
                    .iter()
                    .all(|p| f.overlaps_also_flagged.contains(p))
        })
        .count()
}

/// Matrix warnings (each id once per dimension) followed by the warnings the command itself raised.
fn collect_warnings(loaded: &Loaded) -> Vec<WarningDoc> {
    let mut out: Vec<WarningDoc> = Vec::new();
    let dims = [
        (
            Origin::Coverage,
            loaded.matrix.coverage.as_ref().map(|d| &d.warnings),
        ),
        (
            Origin::Kills,
            loaded.matrix.kills.as_ref().map(|d| &d.warnings),
        ),
    ];
    for (origin, warnings) in dims {
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        for w in warnings.into_iter().flatten() {
            if seen.insert(w.id) {
                out.push(describe_warning(w, origin));
            }
        }
    }
    out.extend(loaded.extra_warnings.iter().cloned());
    out
}

/// Builds the report from loaded inputs and the located tests. Pure: nothing here touches the filesystem.
pub fn analyze(
    loaded: &Loaded,
    locations: &BTreeMap<TestId, Located>,
    opts: &AnalyzeOptions,
) -> Report {
    let mut universe: BTreeSet<TestId> = BTreeSet::new();
    universe.extend(
        loaded
            .matrix
            .coverage
            .iter()
            .flat_map(|d| d.tests.iter().cloned()),
    );
    universe.extend(
        loaded
            .matrix
            .kills
            .iter()
            .flat_map(|d| d.tests.iter().cloned()),
    );

    let Candidates { assessment } = assess_universe(loaded, &universe, locations);
    let assessed: BTreeSet<&TestId> = assessment.assessed.iter().collect();
    let ctx = RuleInputs {
        loaded,
        locations,
        opts,
    };
    let mut all: Vec<TestFinding> = coverage_findings(&ctx, &assessed);
    all.extend(kill_findings(&ctx, &assessed));

    let main_modes = all
        .iter()
        .filter(|f| f.section == Section::Main)
        .map(|f| f.signal_mode);
    let mode = ReportMode::least_validated(main_modes);

    let main: Vec<&TestFinding> = all.iter().filter(|f| f.section == Section::Main).collect();
    let mut summary = Summary {
        individually_flagged: main.len(),
        redundant_only_with_flagged: redundant_only_with_flagged(&main),
        ..Summary::default()
    };
    let mut flagged: BTreeMap<Signal, Vec<TestId>> = BTreeMap::new();
    for f in &main {
        *summary.flagged_by_rule.entry(f.rule.rule_id()).or_default() += 1;
        flagged.entry(f.rule).or_default().push(f.test.clone());
    }
    if opts.experimental {
        let mut exp: BTreeMap<&'static str, usize> = BTreeMap::new();
        for f in all.iter().filter(|f| f.section == Section::Experimental) {
            *exp.entry(f.rule.rule_id()).or_default() += 1;
        }
        summary.experimental = Some(exp);
    }
    let mut findings: Vec<TestFinding> = all
        .into_iter()
        .filter(|f| opts.experimental || f.section == Section::Main)
        .collect();
    rank(&mut findings, mode);

    let not_assessed = not_assessed_list(&assessment, locations);
    let envelope = ReportEnvelope {
        format: opts.format,
        mode,
        header: Header {
            kibitzer_version: env!("CARGO_PKG_VERSION"),
            head_commit: None,
            inputs: loaded.inputs.iter().map(|i| i.header.clone()).collect(),
            scope: Scope {
                tests_in_input: assessment.total(),
                by_reason: assessment.counts(),
            },
        },
        warnings: collect_warnings(loaded),
        summary,
    };
    let universe = universe
        .iter()
        .map(|t| UniverseTest {
            id: display_id(t, locations),
            package: package_of(t, loaded, locations),
            kind: locations.get(t).map_or(TestKind::Unit, |l| l.kind),
            test: t.clone(),
        })
        .collect();
    Report {
        envelope,
        findings,
        not_assessed,
        universe,
        flagged,
    }
}

fn not_assessed_list(a: &Assessment, locations: &BTreeMap<TestId, Located>) -> Vec<NotAssessed> {
    let mut out: Vec<NotAssessed> = a
        .excluded
        .iter()
        .map(|(t, &reason)| NotAssessed {
            id: display_id(t, locations),
            file: locations.get(t).map(|l| l.file.clone()),
            line: locations.get(t).map(|l| l.line),
            reason,
        })
        .collect();
    out.sort_by(|x, y| x.id.cmp(&y.id));
    out
}

#[cfg(test)]
mod tests;
