//! coverage.py contexts adapter (Story 2.2.1): parses `coverage json --show-contexts` into per-test covered lines.
//!
//! Context handling:
//! - A context name is `module.function`, `module.Class.method` (`dynamic_context = test_function`) or a pytest
//!   nodeid `path.py::name` (pytest-cov `--cov-context=test`), optionally followed by a `|run`, `|setup` or
//!   `|teardown` phase suffix. The phase suffix is stripped to get the test id.
//! - The empty context is module-level code (imports, `def` lines). It is credited to every test, so it is never
//!   unique to anyone.
//! - A line reached only in setup/teardown phases is fixture code, which coverage.py charges to whichever test
//!   first triggers it. Such lines are credited to every test too, and counted in the `fixture-import-attribution`
//!   warning.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

use super::guard::{self, JsonError};
use super::sidecar::SidecarRead;
use crate::test_effectiveness::matrix::{
    AttributionChoice, CoverageAttribution, CoverageBuilder, CoverageDimension, CoverageUnit,
    CoverageUnitKind, MissingProvenance, Provenance, TestId, Warning, WarningClass,
};

pub const TOOL_NAME: &str = "coverage.py";
pub const WARN_FIXTURE_IMPORT_ATTRIBUTION: &str = "fixture-import-attribution";

#[derive(Debug)]
pub enum CoveragePyError {
    Json(JsonError),
    /// `meta.version` absent or empty; names coverage.py so the report can say which input to fix.
    MissingProvenance(MissingProvenance),
    /// No file carries a `contexts` map: the JSON was produced without `--show-contexts`.
    MissingContexts,
}

impl std::fmt::Display for CoveragePyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Json(e) => write!(f, "{TOOL_NAME} report: {e}"),
            Self::MissingProvenance(e) => write!(f, "{TOOL_NAME} report: {e}"),
            Self::MissingContexts => write!(
                f,
                "{TOOL_NAME} report has no per-line contexts; rerun `coverage json --show-contexts` with \
                 dynamic contexts enabled"
            ),
        }
    }
}

impl std::error::Error for CoveragePyError {}

#[derive(Deserialize)]
struct Report {
    #[serde(default)]
    meta: Meta,
    #[serde(default)]
    files: BTreeMap<String, FileEntry>,
}

#[derive(Deserialize, Default)]
struct Meta {
    version: Option<String>,
    timestamp: Option<String>,
}

#[derive(Deserialize, Default)]
struct FileEntry {
    contexts: Option<BTreeMap<String, Vec<String>>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Run,
    Setup,
}

/// Splits a context into its test and phase; `None` for the empty (module-level) context.
fn parse_context(raw: &str) -> Option<(TestId, Phase)> {
    if raw.is_empty() {
        return None;
    }
    let (name, phase) = match raw.rsplit_once('|') {
        Some((n, "run")) => (n, Phase::Run),
        Some((n, "setup" | "teardown")) => (n, Phase::Setup),
        _ => (raw, Phase::Run),
    };
    Some((test_id_from_name(name), phase))
}

/// `a/b.py::T::m` or dotted `a.b.T.m` (module segments end at the first capitalised segment, else before the last).
fn test_id_from_name(name: &str) -> TestId {
    if let Some((file, rest)) = name.split_once("::") {
        return TestId::new(file, rest);
    }
    let segments: Vec<&str> = name.split('.').collect();
    let split = segments
        .iter()
        .skip(1)
        .position(|s| s.starts_with(|c: char| c.is_uppercase()))
        .map_or(segments.len() - 1, |i| i + 1);
    if split == 0 {
        return TestId::new("", name);
    }
    let file = format!("{}.py", segments[..split].join("/"));
    TestId::new(file, segments[split..].join("::"))
}

fn warning(id: &'static str, params: Vec<(String, String)>) -> Warning {
    Warning {
        id,
        class: WarningClass::W,
        params,
    }
}

/// Parses a `coverage json --show-contexts` report.
///
/// Attribution resolves per Story 2.2.1: a sidecar-carried value, else `flag`, else an assumed `InOrder`.
pub fn parse_coveragepy(
    bytes: &[u8],
    flag: Option<CoverageAttribution>,
    sidecar: Option<&SidecarRead>,
) -> Result<CoverageDimension, CoveragePyError> {
    let report: Report = guard::parse_json_bounded(bytes).map_err(CoveragePyError::Json)?;
    let version = report
        .meta
        .version
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or(CoveragePyError::MissingProvenance(MissingProvenance {
            field: "version",
        }))?;
    if report.files.values().all(|f| f.contexts.is_none()) {
        return Err(CoveragePyError::MissingContexts);
    }

    let carried = sidecar.and_then(|s| s.sidecar.attribution);
    let choice = AttributionChoice::resolve(carried, flag, CoverageAttribution::InOrder);
    let mut provenance = Provenance::new(TOOL_NAME, Some(version))
        .with_commit(sidecar.and_then(|s| s.sidecar.commit.as_deref()));
    provenance.run_timestamp = report.meta.timestamp.clone();
    provenance.language = Some("python".to_owned());
    let mut builder =
        CoverageBuilder::coverage(provenance, choice).unit_kind(CoverageUnitKind::Line);

    let all_tests = collect_tests(&report);
    for test in &all_tests {
        builder.add_test(test);
    }
    let setup_lines = record_lines(&mut builder, &report, &all_tests);

    let mut dimension = builder
        .build()
        .map_err(CoveragePyError::MissingProvenance)?;
    if setup_lines > 0 {
        dimension.warnings.push(warning(
            WARN_FIXTURE_IMPORT_ATTRIBUTION,
            vec![("setup_lines".to_owned(), setup_lines.to_string())],
        ));
    }
    Ok(dimension)
}

/// Records every line's credited tests; returns how many lines were module-level or setup-only.
fn record_lines(
    builder: &mut CoverageBuilder,
    report: &Report,
    all_tests: &BTreeSet<TestId>,
) -> usize {
    let mut setup_lines = 0usize;
    for (file, entry) in &report.files {
        for (line, contexts) in entry.contexts.iter().flatten() {
            let Ok(line) = line.parse::<u32>() else {
                continue;
            };
            let unit = CoverageUnit::new(file.as_str(), line);
            for test in credited_tests(contexts, all_tests, &mut setup_lines) {
                builder.record(test, unit.clone(), 1);
            }
        }
    }
    setup_lines
}

fn collect_tests(report: &Report) -> BTreeSet<TestId> {
    report
        .files
        .values()
        .filter_map(|f| f.contexts.as_ref())
        .flat_map(|c| c.values().flatten())
        .filter_map(|raw| parse_context(raw))
        .map(|(test, _)| test)
        .collect()
}

/// Tests that get credit for one line. Module-level and setup-only lines are credited to every test.
fn credited_tests<'a>(
    contexts: &[String],
    all_tests: &'a BTreeSet<TestId>,
    setup_lines: &mut usize,
) -> Vec<&'a TestId> {
    let mut has_module_level = false;
    let mut ran = BTreeSet::new();
    let mut setup = BTreeSet::new();
    for raw in contexts {
        match parse_context(raw) {
            None => has_module_level = true,
            Some((test, Phase::Run)) => {
                ran.insert(test);
            }
            Some((test, Phase::Setup)) => {
                setup.insert(test);
            }
        }
    }
    if has_module_level || ran.is_empty() {
        *setup_lines += 1;
        return all_tests.iter().collect();
    }
    all_tests
        .iter()
        .filter(|t| ran.contains(*t) || setup.contains(*t))
        .collect()
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;

    const THREE_TEST: &[u8] =
        include_bytes!("../../../testdata/test-effectiveness/coveragepy/three-test/coverage.json");

    fn lines_of(d: &CoverageDimension, test: &str) -> Vec<u32> {
        let ord = d
            .ordinal_of(&TestId::new("test_app.py", test))
            .expect("test present");
        d.index
            .iter()
            .filter(|(_, p)| p.tests.contains(&ord))
            .map(|(u, _)| u.line)
            .collect()
    }

    /// One-file report; `lines` maps a line number to its context list.
    fn report(version: Option<&str>, lines: &[(u32, &[&str])]) -> Vec<u8> {
        let contexts: serde_json::Map<String, serde_json::Value> = lines
            .iter()
            .map(|(l, c)| (l.to_string(), serde_json::json!(c)))
            .collect();
        let meta = version.map_or(
            serde_json::json!({}),
            |v| serde_json::json!({ "version": v }),
        );
        serde_json::to_vec(&serde_json::json!({
            "meta": meta,
            "files": { "m.py": { "contexts": contexts } }
        }))
        .unwrap()
    }

    fn unique_for(d: &CoverageDimension, test: &str) -> Vec<u32> {
        d.singleton_units()
            .filter(|(_, o)| d.test(*o).name == test)
            .map(|(u, _)| u.line)
            .collect()
    }

    #[test]
    fn parse_should_GiveNoassertSameLinesAsPos_When_ThreeTestFixture() {
        let d = parse_coveragepy(THREE_TEST, None, None).unwrap();
        assert_eq!(lines_of(&d, "test_pos"), vec![2, 4]);
        assert_eq!(lines_of(&d, "test_noassert"), vec![2, 4]);
        assert_eq!(lines_of(&d, "test_neg"), vec![2, 3]);
        assert_eq!(unique_for(&d, "test_neg"), vec![3]);
        assert_eq!(d.provenance.version.as_deref(), Some("7.13.4"));
    }

    #[test]
    fn parse_should_CreditModuleLevelLinesToAllTests_When_ContextEmpty() {
        let bytes = report(
            Some("7.13.4"),
            &[(1, &[""]), (2, &["m.test_a|run"]), (3, &["m.test_b|run"])],
        );
        let d = parse_coveragepy(&bytes, None, None).unwrap();
        let line1 = &d.index[&CoverageUnit::new("m.py", 1)];
        assert_eq!(line1.tests.len(), 2);
        assert_eq!(unique_for(&d, "test_a"), vec![2]);
    }

    #[test]
    fn parse_should_NotCreditFixtureBodyAsUnique_When_SetupContextOfFirstTest() {
        let bytes = report(
            Some("7.13.4"),
            &[
                (1, &[""]),
                (5, &["m.test_a|setup"]),
                (6, &["m.test_a|setup"]),
                (7, &["m.test_a|run"]),
                (8, &["m.test_b|run"]),
            ],
        );
        let d = parse_coveragepy(&bytes, None, None).unwrap();
        assert_eq!(unique_for(&d, "test_a"), vec![7]);
        let w = d
            .warnings
            .iter()
            .find(|w| w.id == WARN_FIXTURE_IMPORT_ATTRIBUTION)
            .expect("warning");
        assert_eq!(w.class, WarningClass::W);
        assert_eq!(w.params, vec![("setup_lines".to_owned(), "3".to_owned())]);
    }

    #[test]
    fn parse_should_StripPhaseSuffixAndSplitModule_When_DottedContext() {
        let bytes = report(
            Some("7.13.4"),
            &[
                (2, &["tests.test_parse.test_empty|run"]),
                (3, &["tests.test_parse.TestC.test_m|run"]),
                (4, &["tests/test_x.py::test_n|run"]),
            ],
        );
        let d = parse_coveragepy(&bytes, None, None).unwrap();
        assert_eq!(
            d.tests,
            vec![
                TestId::new("tests/test_parse.py", "TestC::test_m"),
                TestId::new("tests/test_parse.py", "test_empty"),
                TestId::new("tests/test_x.py", "test_n"),
            ]
        );
    }

    #[test]
    fn parse_should_AssumeInOrderWithWarning_When_NoFlagOrSidecar() {
        let d = parse_coveragepy(THREE_TEST, None, None).unwrap();
        assert_eq!(d.meta, CoverageAttribution::InOrder);
        assert!(d.warnings.iter().any(|w| w.id == "attribution-in-order"));
    }

    #[test]
    fn parse_should_HonorFlagWithoutAssumedWarning_When_AttributionIsolated() {
        let d = parse_coveragepy(THREE_TEST, Some(CoverageAttribution::Isolated), None).unwrap();
        assert_eq!(d.meta, CoverageAttribution::Isolated);
        assert!(!d.warnings.iter().any(|w| w.id == "attribution-in-order"));
    }

    #[test]
    fn parse_should_UseSidecarAttributionWithoutAssumedWarning_When_SidecarCarriesIt() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("testdata/test-effectiveness/coveragepy/three-test/coverage.json.run.json");
        let sc = super::super::sidecar::read_sidecar(&path, 1 << 20).unwrap();
        let d =
            parse_coveragepy(THREE_TEST, Some(CoverageAttribution::InOrder), Some(&sc)).unwrap();
        assert_eq!(d.meta, CoverageAttribution::Isolated);
        assert!(!d.warnings.iter().any(|w| w.id == "attribution-in-order"));
        assert_eq!(d.provenance.commit.as_deref(), Some("298cc15"));
    }
}
