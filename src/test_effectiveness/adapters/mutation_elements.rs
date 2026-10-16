//! mutation-testing-elements (Stryker) JSON adapter (Story 2.2.4): the kills dimension only.
//!
//! Reads `files.<path>.mutants[]` (`status`, `killedBy`, `coveredBy`) and the top-level
//! `testFiles.<file>.tests[]` id-to-name table. `coveredBy` is mutant-site coverage, so it is
//! returned beside the kills and never credited as line coverage.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::Deserialize;
use serde_json::Value;

use super::guard::{self, GuardError, JsonError};
use super::sidecar::{self, MutationRun, MutationScope, Sidecar};
use crate::test_effectiveness::matrix::{
    KillBuilder, KillCompleteness, KillDimension, MissingProvenance, MutantId, Provenance, TestId,
};

const TOOL_NAME: &str = "stryker";
const UNKNOWN_TEST_FILE: &str = "(unknown)";
const STATUS_KILLED: &str = "Killed";

#[derive(Debug, Deserialize)]
struct Report {
    #[serde(rename = "schemaVersion")]
    schema_version: Option<Value>,
    framework: Option<Framework>,
    #[serde(default)]
    files: BTreeMap<String, FileEntry>,
    #[serde(rename = "testFiles", default)]
    test_files: BTreeMap<String, TestFile>,
}

#[derive(Debug, Deserialize)]
struct Framework {
    name: Option<String>,
    version: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FileEntry {
    #[serde(default)]
    mutants: Vec<Mutant>,
}

#[derive(Debug, Deserialize)]
struct Mutant {
    #[serde(default)]
    id: String,
    #[serde(default)]
    status: String,
    #[serde(rename = "killedBy", default)]
    killed_by: Vec<String>,
    #[serde(rename = "coveredBy", default)]
    covered_by: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct TestFile {
    #[serde(default)]
    tests: Vec<TestEntry>,
}

#[derive(Debug, Deserialize)]
struct TestEntry {
    id: String,
    name: Option<String>,
}

#[derive(Debug)]
pub enum MutationElementsError {
    Guard(GuardError),
    Json(JsonError),
    MissingProvenance(MissingProvenance),
}

impl std::fmt::Display for MutationElementsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Guard(e) => e.fmt(f),
            Self::Json(e) => e.fmt(f),
            Self::MissingProvenance(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for MutationElementsError {}

/// The kills dimension plus what the report carries that the matrix does not hold.
#[derive(Debug, Clone)]
pub struct StrykerReport {
    pub kills: KillDimension,
    pub mutation_run: MutationRun,
    /// Mutant-site coverage (`coveredBy`); not line coverage and not credited as such.
    pub covered_by: BTreeMap<MutantId, Vec<TestId>>,
}

/// Reads a report through the size guard, then parses it.
pub fn parse_report(
    path: &Path,
    max_input_bytes: u64,
    sidecar: Option<&Sidecar>,
    flag: Option<KillCompleteness>,
) -> Result<StrykerReport, MutationElementsError> {
    let bytes = guard::read_bounded(path, max_input_bytes).map_err(MutationElementsError::Guard)?;
    parse_bytes(&bytes, sidecar, flag)
}

pub fn parse_bytes(
    bytes: &[u8],
    sidecar: Option<&Sidecar>,
    flag: Option<KillCompleteness>,
) -> Result<StrykerReport, MutationElementsError> {
    let report: Report = guard::parse_json_bounded(bytes).map_err(MutationElementsError::Json)?;

    let framework = report.framework.as_ref();
    let tool = framework
        .and_then(|f| f.name.as_deref())
        .or_else(|| sidecar.and_then(|s| s.tool.as_deref()))
        .unwrap_or(TOOL_NAME);
    let version = framework
        .and_then(|f| f.version.as_deref())
        .or_else(|| sidecar.and_then(|s| s.tool_version.as_deref()));
    let provenance =
        Provenance::new(tool, version).with_commit(sidecar.and_then(|s| s.commit.as_deref()));

    let (resolved, completeness_warnings) = sidecar::resolve_kill_completeness(sidecar, flag);
    let mut builder = KillBuilder::kills(provenance, resolved);

    let test_ids = register_tests(&report, &mut builder);
    let covered_by = record_mutants(&report, &test_ids, &mut builder);

    let mut kills = builder
        .build()
        .map_err(MutationElementsError::MissingProvenance)?;
    kills.warnings.extend(completeness_warnings);

    Ok(StrykerReport {
        kills,
        mutation_run: mutation_run(sidecar, flag, resolved),
        covered_by,
    })
}

/// Interns every test in `testFiles` (so a test that kills nothing still counts as assessed)
/// and returns the report-local id to `TestId` table.
fn register_tests(report: &Report, builder: &mut KillBuilder) -> BTreeMap<String, TestId> {
    let mut ids = BTreeMap::new();
    for (file, test_file) in &report.test_files {
        for test in &test_file.tests {
            let id = TestId::new(file.as_str(), test.name.as_deref().unwrap_or(&test.id));
            builder.add_test(&id);
            ids.insert(test.id.clone(), id);
        }
    }
    ids
}

/// Credits each killer of every `Killed` mutant; returns `coveredBy` per mutant for the caller.
fn record_mutants(
    report: &Report,
    test_ids: &BTreeMap<String, TestId>,
    builder: &mut KillBuilder,
) -> BTreeMap<MutantId, Vec<TestId>> {
    let resolve = |id: &String| {
        test_ids
            .get(id)
            .cloned()
            .unwrap_or_else(|| TestId::new(UNKNOWN_TEST_FILE, id.as_str()))
    };
    let mut covered_by = BTreeMap::new();
    let mutants = report.files.iter().flat_map(|(path, file)| {
        file.mutants
            .iter()
            .enumerate()
            .map(move |(i, m)| (path, i, m))
    });
    for (path, index, mutant) in mutants {
        let local = if mutant.id.is_empty() {
            index.to_string()
        } else {
            mutant.id.clone()
        };
        let mutant_id = MutantId(format!("{path}#{local}"));
        if mutant.status == STATUS_KILLED {
            for killer in &mutant.killed_by {
                builder.record(&resolve(killer), mutant_id.clone(), 1);
            }
        }
        if !mutant.covered_by.is_empty() {
            let tests: BTreeSet<TestId> = mutant.covered_by.iter().map(resolve).collect();
            covered_by.insert(mutant_id, tests.into_iter().collect());
        }
    }
    covered_by
}

fn mutation_run(
    sidecar: Option<&Sidecar>,
    flag: Option<KillCompleteness>,
    resolved: KillCompleteness,
) -> MutationRun {
    let from_sidecar = sidecar.map(Sidecar::mutation_run);
    let stated = flag.is_some()
        || from_sidecar
            .as_ref()
            .is_some_and(|r| r.scope != MutationScope::Unknown);
    let scope = if !stated {
        MutationScope::Unknown
    } else {
        match resolved {
            KillCompleteness::Full | KillCompleteness::FirstKillerOnly => MutationScope::Full,
            KillCompleteness::Sampled => MutationScope::Sampled,
            KillCompleteness::BudgetLimited => MutationScope::BudgetLimited,
        }
    };
    MutationRun {
        scope,
        mutants_run: from_sidecar.as_ref().and_then(|r| r.mutants_run),
        mutants_total: from_sidecar.as_ref().and_then(|r| r.mutants_total),
    }
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;
    use crate::test_effectiveness::matrix::Tier;
    use std::path::PathBuf;

    const MAX: u64 = 1 << 20;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("testdata/test-effectiveness/stryker")
            .join(name)
    }

    fn sidecar_with_commit() -> Sidecar {
        Sidecar {
            commit: Some("abc1234".to_string()),
            ..Sidecar::default()
        }
    }

    fn parse(name: &str) -> StrykerReport {
        parse_report(&fixture(name), MAX, Some(&sidecar_with_commit()), None).expect("parses")
    }

    fn test_named(report: &StrykerReport, name: &str) -> TestId {
        report
            .kills
            .tests
            .iter()
            .find(|t| t.name == name)
            .cloned()
            .unwrap_or_else(|| panic!("no test {name}"))
    }

    #[test]
    fn parse_should_CreditKillerWithUniqueMutant_When_KilledByListsOneTest() {
        let r = parse("report-v2.json");
        let t1 = test_named(&r, "calc adds two numbers");
        let ordinal = r.kills.ordinal_of(&t1).unwrap();
        let unique: Vec<_> = r
            .kills
            .singleton_units()
            .filter(|&(_, o)| o == ordinal)
            .map(|(m, _)| m.0.clone())
            .collect();
        assert_eq!(unique, vec!["src/calc.js#1".to_string()]);
    }

    #[test]
    fn parse_should_ListEveryKiller_When_KilledByHasTwoTests() {
        let r = parse("report-v2.json");
        let posting = &r.kills.index[&MutantId("src/calc.js#0".to_string())];
        assert_eq!(posting.tests.len(), 2);
    }

    #[test]
    fn parse_should_ResolveTestIdsToFileAndName_When_TestFilesPresent() {
        let r = parse("report-v2.json");
        assert!(
            r.kills
                .tests
                .contains(&TestId::new("test/calc.test.js", "calc rejects negatives"))
        );
    }

    #[test]
    fn parse_should_IgnoreSurvivedAndNoCoverageMutants_When_BuildingKills() {
        let r = parse("report-v2.json");
        assert_eq!(r.kills.index.len(), 4);
        assert!(
            !r.kills
                .index
                .contains_key(&MutantId("src/calc.js#4".to_string()))
        );
    }

    #[test]
    fn parse_should_KeepCoveredByApartFromKills_When_MutantSurvived() {
        let r = parse("report-v2.json");
        let covering = &r.covered_by[&MutantId("src/calc.js#4".to_string())];
        assert_eq!(covering.len(), 1);
        assert!(
            !r.kills
                .index
                .contains_key(&MutantId("src/calc.js#4".to_string()))
        );
    }

    #[test]
    fn parse_should_TakeProvenanceFromFramework_When_ReportCarriesVersion() {
        let r = parse("report-v2.json");
        assert_eq!(r.kills.provenance.tool, "StrykerJS");
        assert_eq!(r.kills.provenance.version.as_deref(), Some("9.0.1"));
    }

    #[test]
    fn parse_should_TakeVersionFromSidecar_When_ReportLacksFramework() {
        let sc = Sidecar {
            tool_version: Some("9.1.0".to_string()),
            ..sidecar_with_commit()
        };
        let r = parse_report(&fixture("report-no-framework.json"), MAX, Some(&sc), None).unwrap();
        assert_eq!(r.kills.provenance.tool, "stryker");
        assert_eq!(r.kills.provenance.version.as_deref(), Some("9.1.0"));
    }

    #[test]
    fn parse_should_ReturnMissingProvenance_When_NoVersionAnywhere() {
        let e = parse_report(&fixture("report-no-framework.json"), MAX, None, None).unwrap_err();
        assert!(matches!(
            e,
            MutationElementsError::MissingProvenance(MissingProvenance { field: "version" })
        ));
    }

    #[test]
    fn parse_should_ReturnGuardError_When_InputOverSizeCap() {
        let e = parse_report(&fixture("report-v2.json"), 10, None, None).unwrap_err();
        assert!(matches!(
            e,
            MutationElementsError::Guard(GuardError::TooLarge(_))
        ));
    }

    #[test]
    fn parse_should_ReturnJsonError_When_NotJson() {
        let e = parse_bytes(b"<xml/>", None, None).unwrap_err();
        assert!(matches!(e, MutationElementsError::Json(_)));
    }

    #[test]
    fn parse_should_CapLowWithTypedMutationRun_When_SidecarBudgetExhausted() {
        let bytes = std::fs::read(fixture("report-budget.run.json")).unwrap();
        let sc = sidecar::parse_sidecar(&bytes).unwrap().sidecar;
        let r = parse_report(&fixture("report-v2.json"), MAX, Some(&sc), None).unwrap();
        assert_eq!(r.kills.meta, KillCompleteness::BudgetLimited);
        assert_eq!(r.kills.cap, Some(Tier::Low));
        assert_eq!(
            r.mutation_run,
            MutationRun {
                scope: MutationScope::BudgetLimited,
                mutants_run: Some(4),
                mutants_total: Some(6),
            }
        );
    }

    #[test]
    fn parse_should_MarkSampled_When_FlagSaysSampledWithoutSidecar() {
        let r = parse_report(
            &fixture("report-v2.json"),
            MAX,
            Some(&sidecar_with_commit()),
            Some(KillCompleteness::Sampled),
        )
        .unwrap();
        assert_eq!(r.kills.meta, KillCompleteness::Sampled);
        assert_eq!(r.mutation_run.scope, MutationScope::Sampled);
    }

    #[test]
    fn parse_should_LeaveRunScopeUnknown_When_NothingStatesCompleteness() {
        let r = parse("report-v2.json");
        assert_eq!(r.mutation_run.scope, MutationScope::Unknown);
    }
}
