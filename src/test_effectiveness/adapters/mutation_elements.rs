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
    Tier, Warning, WarningClass,
};

pub const WARN_FIRST_KILLER_ONLY: &str = "first-killer-only";
pub const WARN_UNVERIFIED_TOOL_VERSION: &str = "unverified-tool-version";
pub const WARN_SINGLE_KILLER_SHARE: &str = "single-killer-share-implausible";

/// A `schemaVersion` this adapter was fixtured against, with the single-killer share range recorded
/// with that fixture (see `testdata/test-effectiveness/stryker/README.md`).
struct FixturedVersion {
    schema_version: &'static str,
    single_killer_share: (f64, f64),
}

const FIXTURED_VERSIONS: &[FixturedVersion] = &[FixturedVersion {
    schema_version: "2",
    single_killer_share: (0.30, 1.00),
}];

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
    let stats = KillStats::of(&report);
    let completeness = match resolved {
        KillCompleteness::Full if stats.is_first_killer_only() => KillCompleteness::FirstKillerOnly,
        other => other,
    };
    let mut builder = KillBuilder::kills(provenance, completeness);

    let test_ids = register_tests(&report, &mut builder);
    let covered_by = record_mutants(&report, &test_ids, &mut builder);

    let mut kills = builder
        .build()
        .map_err(MutationElementsError::MissingProvenance)?;
    kills.warnings.extend(completeness_warnings);
    kills.warnings.extend(drift_warnings(&report, &stats));
    if kills
        .warnings
        .iter()
        .any(|w| w.id == WARN_UNVERIFIED_TOOL_VERSION)
    {
        kills.cap = Some(kills.cap.map_or(Tier::Low, |c| c.min(Tier::Low)));
    }

    Ok(StrykerReport {
        kills,
        mutation_run: mutation_run(sidecar, flag, resolved),
        covered_by,
    })
}

/// Killed mutants, and how many of them list exactly one distinct killer.
struct KillStats {
    killed: u64,
    single_killer: u64,
}

impl KillStats {
    fn of(report: &Report) -> Self {
        let mut stats = Self {
            killed: 0,
            single_killer: 0,
        };
        let killers = report
            .files
            .values()
            .flat_map(|f| &f.mutants)
            .filter(|m| m.status == STATUS_KILLED)
            .map(|m| m.killed_by.iter().collect::<BTreeSet<_>>().len())
            .filter(|&n| n > 0);
        for n in killers {
            stats.killed += 1;
            stats.single_killer += u64::from(n == 1);
        }
        stats
    }

    /// Bail-on-first signature: at least one kill, and every kill has a single killer.
    fn is_first_killer_only(&self) -> bool {
        self.killed > 0 && self.single_killer == self.killed
    }

    fn share(&self) -> Option<f64> {
        (self.killed > 0).then(|| self.single_killer as f64 / self.killed as f64)
    }
}

fn warning(id: &'static str, params: &[(&str, String)]) -> Warning {
    Warning {
        id,
        class: WarningClass::W,
        params: params
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect(),
    }
}

/// Class W warnings for truncated kill sets, an unlisted `schemaVersion`, and an implausible single-killer share.
fn drift_warnings(report: &Report, stats: &KillStats) -> Vec<Warning> {
    let truncated = stats.is_first_killer_only();
    let mut warnings = Vec::new();
    if truncated {
        warnings.push(warning(
            WARN_FIRST_KILLER_ONLY,
            &[("killed_mutants", stats.killed.to_string())],
        ));
    }
    let version = report.schema_version.as_ref().map(|v| match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    });
    let fixtured = version
        .as_deref()
        .and_then(|v| FIXTURED_VERSIONS.iter().find(|f| f.schema_version == v));
    let Some(fixtured) = fixtured else {
        let shown = version.unwrap_or_else(|| "missing".to_string());
        warnings.push(warning(
            WARN_UNVERIFIED_TOOL_VERSION,
            &[("schema_version", shown)],
        ));
        return warnings;
    };
    let (lo, hi) = fixtured.single_killer_share;
    let implausible = stats
        .share()
        .filter(|s| !truncated && !(lo..=hi).contains(s));
    if let Some(share) = implausible {
        warnings.push(warning(
            WARN_SINGLE_KILLER_SHARE,
            &[
                ("share", format!("{share:.2}")),
                ("expected", format!("{lo:.2}-{hi:.2}")),
            ],
        ));
    }
    warnings
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

    fn with_killers(killers: &[usize], version: &str) -> Vec<u8> {
        let mutants: Vec<String> = killers
            .iter()
            .enumerate()
            .map(|(i, &n)| {
                let by: Vec<String> = (0..n).map(|t| format!("\"t{t}\"")).collect();
                format!(
                    r#"{{"id":"{i}","status":"Killed","killedBy":[{}]}}"#,
                    by.join(",")
                )
            })
            .collect();
        format!(
            r#"{{"schemaVersion":"{version}","framework":{{"name":"StrykerJS","version":"9.0.1"}},
            "files":{{"a.js":{{"mutants":[{}]}}}},
            "testFiles":{{"a.test.js":{{"tests":[{{"id":"t0","name":"A"}},{{"id":"t1","name":"B"}}]}}}}}}"#,
            mutants.join(",")
        )
        .into_bytes()
    }

    fn ids(r: &StrykerReport) -> Vec<&'static str> {
        r.kills.warnings.iter().map(|w| w.id).collect()
    }

    fn parse_inline(bytes: &[u8]) -> StrykerReport {
        parse_bytes(bytes, Some(&sidecar_with_commit()), None).unwrap()
    }

    #[test]
    fn parse_should_TagFirstKillerOnlyAndDowngrade_When_AllKilledBySingle() {
        let r = parse_inline(&with_killers(&[1; 40], "2"));
        assert_eq!(r.kills.meta, KillCompleteness::FirstKillerOnly);
        assert_eq!(r.kills.cap, Some(Tier::Low));
        assert_eq!(ids(&r), vec![WARN_FIRST_KILLER_ONLY]);
        let w = &r.kills.warnings[0];
        assert_eq!(w.class, WarningClass::W);
        assert_eq!(
            w.params,
            vec![("killed_mutants".to_string(), "40".to_string())]
        );
    }

    #[test]
    fn parse_should_StayFull_When_SomeKilledMutantHasTwoKillers() {
        let r = parse("report-v2.json");
        assert_eq!(r.kills.meta, KillCompleteness::Full);
        assert_eq!(r.kills.cap, None);
        assert!(ids(&r).is_empty());
    }

    #[test]
    fn parse_should_NotTagFirstKillerOnly_When_NoMutantWasKilled() {
        let r = parse_inline(&with_killers(&[], "2"));
        assert_eq!(r.kills.meta, KillCompleteness::Full);
        assert!(ids(&r).is_empty());
    }

    #[test]
    fn parse_should_KeepBudgetLimitedAndStillWarn_When_AllSingleKillersUnderBudgetStop() {
        let bytes = std::fs::read(fixture("report-budget.run.json")).unwrap();
        let sc = sidecar::parse_sidecar(&bytes).unwrap().sidecar;
        let r = parse_bytes(&with_killers(&[1; 5], "2"), Some(&sc), None).unwrap();
        assert_eq!(r.kills.meta, KillCompleteness::BudgetLimited);
        assert_eq!(ids(&r), vec![WARN_FIRST_KILLER_ONLY]);
    }

    #[test]
    fn parse_should_WarnAndCapLow_When_SchemaVersionNotOnAllowList() {
        let r = parse("report-unlisted-version.json");
        assert_eq!(ids(&r), vec![WARN_UNVERIFIED_TOOL_VERSION]);
        assert_eq!(
            r.kills.warnings[0].params,
            vec![("schema_version".to_string(), "99".to_string())]
        );
        assert_eq!(r.kills.cap, Some(Tier::Low));
        assert_eq!(r.kills.meta, KillCompleteness::Full);
    }

    #[test]
    fn parse_should_WarnAndCapLow_When_SchemaVersionMissing() {
        let r = parse_inline(br#"{"framework":{"name":"S","version":"1"},"files":{}}"#);
        assert_eq!(ids(&r), vec![WARN_UNVERIFIED_TOOL_VERSION]);
        assert_eq!(r.kills.cap, Some(Tier::Low));
    }

    #[test]
    fn parse_should_WarnAboutPlausibility_When_SingleKillerShareBelowFixtureRange() {
        let r = parse_inline(&with_killers(&[2, 2, 2, 2, 2, 2, 2, 2, 2, 1], "2"));
        assert_eq!(ids(&r), vec![WARN_SINGLE_KILLER_SHARE]);
        assert_eq!(r.kills.meta, KillCompleteness::Full);
        assert_eq!(r.kills.cap, None);
    }

    #[test]
    fn parse_should_NotWarnAboutPlausibility_When_ShareInsideFixtureRange() {
        let r = parse_inline(&with_killers(&[2, 1, 1, 2], "2"));
        assert!(ids(&r).is_empty());
    }

    #[test]
    fn parse_should_CountDistinctKillers_When_KilledByRepeatsATest() {
        let r = parse_inline(
            br#"{"schemaVersion":"2","framework":{"name":"S","version":"1"},
            "files":{"a.js":{"mutants":[{"id":"0","status":"Killed","killedBy":["t0","t0"]}]}}}"#,
        );
        assert_eq!(r.kills.meta, KillCompleteness::FirstKillerOnly);
    }

    #[test]
    fn parse_should_LeaveRunScopeUnknown_When_NothingStatesCompleteness() {
        let r = parse("report-v2.json");
        assert_eq!(r.mutation_run.scope, MutationScope::Unknown);
    }
}
