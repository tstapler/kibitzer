//! PIT `mutations.xml` adapter (Story 2.2.5): per-test kills and mutant-site coverage.
//!
//! `killingTests` (pipe-separated) credits every killer; `killingTest` is the first killer only, so a report whose
//! killed mutants carry nothing but `killingTest` was run without `fullMutationMatrix` and is capped `Low`.
//!
//! Bias to carry into any Z_cov versus Z_kill comparison (H3, Story 3.3.2): PIT `coveringTests` is a superset of
//! the killers by construction (a test that kills a mutant necessarily covers its site) and both come from this one
//! file, so the comparison is biased toward containment and weakly informative. `coveringTests` is therefore stored
//! as [`CoverageUnitKind::MutantSite`] and never mixed with line coverage.
//!
//! The input is untrusted (Story 2.2.7): the size guard runs before any read, a DOCTYPE is rejected outright so
//! no entity is ever declared, expanded or fetched (quick-xml resolves only the five predefined entities and
//! character references; anything else is an error), and element nesting is bounded.

use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::path::Path;

use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};

use super::guard::{self, GuardError};
use super::sidecar::{
    self, MutationRun, MutationScope, SidecarError, SidecarRead, resolve_kill_completeness,
};
use crate::test_effectiveness::matrix::{
    AttributionChoice, CoverageAttribution, CoverageBuilder, CoverageDimension, CoverageUnit,
    CoverageUnitKind, KillBuilder, KillCompleteness, KillDimension, MissingProvenance, MutantId,
    Provenance, TestId, TestMatrix, Tier, Warning, WarningClass,
};

pub const TOOL: &str = "pit";
pub const WARN_FULL_MATRIX_NOT_ENABLED: &str = "full-matrix-not-enabled";
pub const WARN_UNVERIFIED_TOOL_VERSION: &str = "unverified-tool-version";

/// `major.minor` lines the hand-written fixtures describe; any patch release of a listed line is accepted.
pub const VERIFIED_VERSIONS: [&str; 3] = ["1.15", "1.16", "1.17"];

/// Deeper than any real PIT report (root, mutation, field = 3); a hostile nest is rejected, not recursed into.
const MAX_DEPTH: usize = 16;
const UNKNOWN_VERSION: &str = "unknown";

#[derive(Debug, Clone, Default)]
pub struct PitOptions {
    pub max_input_bytes: u64,
    /// `--pit-version`; used only when the report carries no version.
    pub pit_version: Option<String>,
    /// `--kill-completeness`.
    pub kill_completeness: Option<KillCompleteness>,
    pub commit: Option<String>,
    /// Pre-read run sidecar; `parse_file` reads `<report>.run.json` when this is `None`.
    pub sidecar: Option<SidecarRead>,
}

#[derive(Debug, Clone)]
pub struct PitReport {
    /// `kills` always; `coverage` (unit kind `MutantSite`) when any mutation lists `coveringTests`.
    pub matrix: TestMatrix,
    pub mutation_run: MutationRun,
}

#[derive(Debug)]
pub enum PitError {
    Guard(GuardError),
    Sidecar(SidecarError),
    Provenance(MissingProvenance),
    NotUtf8,
    Xml(String),
    DoctypeNotAllowed,
    UnknownEntity(String),
    NestingTooDeep,
    NotPitReport,
    InvalidField { field: &'static str, value: String },
}

impl fmt::Display for PitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Guard(e) => e.fmt(f),
            Self::Sidecar(e) => e.fmt(f),
            Self::Provenance(e) => e.fmt(f),
            Self::NotUtf8 => f.write_str("not valid XML (input is not UTF-8)"),
            Self::Xml(why) => write!(f, "not valid XML ({why})"),
            Self::DoctypeNotAllowed => {
                f.write_str("not a PIT report (DOCTYPE declarations are not accepted)")
            }
            Self::UnknownEntity(name) => write!(f, "not valid XML (undefined entity &{name};)"),
            Self::NestingTooDeep => f.write_str("not valid XML (nesting too deep)"),
            Self::NotPitReport => f.write_str("not a PIT mutations.xml (root is not <mutations>)"),
            Self::InvalidField { field, value } => {
                write!(f, "not a valid PIT report ({field} is {value:?})")
            }
        }
    }
}

impl std::error::Error for PitError {}

/// Reads a `mutations.xml` through the size guard, then parses it.
pub fn parse_file(path: &Path, opts: &PitOptions) -> Result<PitReport, PitError> {
    let bytes = guard::read_bounded(path, opts.max_input_bytes).map_err(PitError::Guard)?;
    let mut opts = opts.clone();
    if opts.sidecar.is_none() {
        let sidecar_path = sidecar::sidecar_path_for(path);
        if sidecar_path.is_file() {
            opts.sidecar = Some(
                sidecar::read_sidecar(&sidecar_path, opts.max_input_bytes)
                    .map_err(PitError::Sidecar)?,
            );
        }
    }
    parse_bytes(&bytes, &opts)
}

/// Parses bytes already admitted by the size guard.
pub fn parse_bytes(bytes: &[u8], opts: &PitOptions) -> Result<PitReport, PitError> {
    let text = std::str::from_utf8(bytes).map_err(|_| PitError::NotUtf8)?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let parsed = scan(text)?;
    build(parsed, opts)
}

#[derive(Default)]
struct RawMutation {
    status: String,
    fields: HashMap<&'static str, String>,
}

const FIELDS: [&str; 8] = [
    "sourceFile",
    "mutatedClass",
    "mutatedMethod",
    "lineNumber",
    "mutator",
    "killingTest",
    "killingTests",
    "succeedingTests",
];
const COVERING: &str = "coveringTests";
const INDEX: &str = "index";
const BLOCK: &str = "block";
const DESCRIPTION: &str = "methodDescription";

fn known_field(name: &str) -> Option<&'static str> {
    FIELDS
        .iter()
        .copied()
        .chain([COVERING, INDEX, BLOCK, DESCRIPTION])
        .find(|f| *f == name)
}

struct Scanned {
    version: Option<String>,
    mutations: Vec<RawMutation>,
}

fn xml_err(e: impl fmt::Display) -> PitError {
    PitError::Xml(e.to_string())
}

fn attr(e: &BytesStart<'_>, key: &str) -> Result<Option<String>, PitError> {
    for a in e.attributes() {
        let a = a.map_err(xml_err)?;
        if a.key.as_ref() == key {
            return Ok(Some(
                a.normalized_value(XmlVersion::Implicit1_0)
                    .map_err(xml_err)?
                    .into_owned(),
            ));
        }
    }
    Ok(None)
}

fn push_entity(buf: &mut String, name: &str) -> Result<(), PitError> {
    let c = match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        other => {
            let digits = other.strip_prefix('#');
            let code = match digits {
                Some(h) if h.starts_with(['x', 'X']) => u32::from_str_radix(&h[1..], 16).ok(),
                Some(d) => d.parse().ok(),
                None => None,
            };
            code.and_then(char::from_u32)
                .ok_or_else(|| PitError::UnknownEntity(other.chars().take(32).collect()))?
        }
    };
    buf.push(c);
    Ok(())
}

fn scan(text: &str) -> Result<Scanned, PitError> {
    let mut reader = Reader::from_str(text);
    let mut depth = 0usize;
    let mut version = None;
    let mut mutations = Vec::new();
    let mut current: Option<RawMutation> = None;
    let mut field: Option<&'static str> = None;
    let mut buf = String::new();
    let mut saw_root = false;
    loop {
        match reader.read_event().map_err(xml_err)? {
            Event::Eof => break,
            Event::DocType(_) => return Err(PitError::DoctypeNotAllowed),
            Event::Start(e) => {
                depth += 1;
                if depth > MAX_DEPTH {
                    return Err(PitError::NestingTooDeep);
                }
                if depth == 1 {
                    if e.name().as_ref() != "mutations" {
                        return Err(PitError::NotPitReport);
                    }
                    saw_root = true;
                    version = attr(&e, "version")?;
                } else if current.is_none() && e.name().as_ref() == "mutation" {
                    current = Some(raw_from(&e)?);
                } else if current.is_some() {
                    field = known_field(e.name().as_ref());
                    buf.clear();
                }
            }
            Event::Empty(e) => {
                if depth == 0 {
                    return Err(PitError::NotPitReport);
                }
                if current.is_none() && e.name().as_ref() == "mutation" {
                    mutations.push(raw_from(&e)?);
                }
            }
            Event::Text(t) if field.is_some() => buf.push_str(&t.into_inner()),
            Event::CData(c) if field.is_some() => {
                buf.push_str(&c.into_inner());
            }
            Event::GeneralRef(r) => {
                if field.is_some() {
                    push_entity(&mut buf, &r)?;
                } else {
                    // Validate even outside fields so an undefined entity is never silently skipped.
                    push_entity(&mut String::new(), &r)?;
                }
            }
            Event::End(e) => {
                depth = depth.saturating_sub(1);
                if let (Some(name), Some(m)) = (field.take(), current.as_mut()) {
                    m.fields.insert(name, std::mem::take(&mut buf));
                } else if e.name().as_ref() == "mutation" {
                    mutations.extend(current.take());
                }
            }
            _ => {}
        }
    }
    if !saw_root {
        return Err(xml_err("no root element"));
    }
    if depth != 0 {
        return Err(xml_err("unexpected end of file"));
    }
    Ok(Scanned { version, mutations })
}

fn raw_from(e: &BytesStart<'_>) -> Result<RawMutation, PitError> {
    Ok(RawMutation {
        status: attr(e, "status")?.unwrap_or_default(),
        fields: HashMap::new(),
    })
}

fn split_tests(raw: Option<&String>) -> impl Iterator<Item = &str> {
    raw.into_iter()
        .flat_map(|s| s.split('|'))
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// PIT names a test `pkg.Class.method(pkg.Class)` or a JUnit 5 unique id `...[class:pkg.Class]/[method:m()]`.
fn test_id(raw: &str) -> TestId {
    let (class, method) = if let Some(rest) = raw.split("[class:").nth(1) {
        let class = rest.split(']').next().unwrap_or(rest);
        let method = raw
            .split("[method:")
            .nth(1)
            .and_then(|m| m.split(']').next())
            .map(|m| m.trim_end_matches("()"))
            .unwrap_or(class);
        (class, method)
    } else if let Some((head, tail)) = raw.split_once('(') {
        let class = tail.trim_end_matches(')');
        let method = head.rsplit('.').next().unwrap_or(head);
        (class, method)
    } else {
        raw.rsplit_once('.').unwrap_or((raw, raw))
    };
    let outer = class.split('$').next().unwrap_or(class);
    TestId::new(format!("{}.java", outer.replace('.', "/")), method)
}

fn site_file(class: &str) -> String {
    let outer = class.split('$').next().unwrap_or(class);
    outer.replace('.', "/") + ".java"
}

fn parse_line(raw: Option<&String>) -> Result<u32, PitError> {
    match raw.map(|s| s.trim()) {
        None | Some("") => Ok(0),
        Some(s) => s.parse().map_err(|_| PitError::InvalidField {
            field: "lineNumber",
            value: s.chars().take(32).collect(),
        }),
    }
}

fn rank(c: KillCompleteness) -> u8 {
    match c {
        KillCompleteness::Full => 3,
        KillCompleteness::FirstKillerOnly => 2,
        KillCompleteness::Sampled => 1,
        KillCompleteness::BudgetLimited => 0,
    }
}

fn warning(id: &'static str, params: &[(&str, &str)]) -> Warning {
    Warning {
        id,
        class: WarningClass::W,
        params: params
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect(),
    }
}

fn version_is_verified(version: &str) -> bool {
    VERIFIED_VERSIONS
        .iter()
        .any(|v| version == *v || version.strip_prefix(v).is_some_and(|r| r.starts_with('.')))
}

fn lower(cap: &mut Option<Tier>, to: Tier) {
    *cap = Some(cap.map_or(to, |c| c.min(to)));
}

fn build(parsed: Scanned, opts: &PitOptions) -> Result<PitReport, PitError> {
    let version = parsed
        .version
        .filter(|v| !v.trim().is_empty())
        .or_else(|| opts.pit_version.clone())
        .unwrap_or_else(|| UNKNOWN_VERSION.to_owned());
    let sidecar = opts.sidecar.as_ref();
    let commit = opts
        .commit
        .as_deref()
        .or_else(|| sidecar.and_then(|s| s.sidecar.commit.as_deref()));
    let mut provenance = Provenance::new(TOOL, Some(&version)).with_commit(commit);
    provenance.language = Some("java".to_owned());

    let (flagged, mut extra) =
        resolve_kill_completeness(sidecar.map(|s| &s.sidecar), opts.kill_completeness);
    let mut assessed: BTreeSet<TestId> = BTreeSet::new();
    let mut kill_records: Vec<(TestId, MutantId)> = Vec::new();
    let mut cov_b = CoverageBuilder::coverage(
        provenance.clone(),
        AttributionChoice {
            attribution: CoverageAttribution::Isolated,
            assumed: false,
        },
    )
    .unit_kind(CoverageUnitKind::MutantSite);
    let (mut killed, mut killed_with_matrix, mut any_covering) = (0usize, 0usize, false);

    for m in &parsed.mutations {
        let get = |k: &str| m.fields.get(k).map_or("", String::as_str);
        let line = parse_line(m.fields.get("lineNumber"))?;
        let mutant = MutantId(format!(
            "{}.{}{}:{line}:{}:{}:{}",
            get("mutatedClass"),
            get("mutatedMethod"),
            get(DESCRIPTION),
            get("mutator").rsplit('.').next().unwrap_or_default(),
            get(INDEX),
            get(BLOCK),
        ));
        for t in split_tests(m.fields.get("succeedingTests")) {
            assessed.insert(test_id(t));
        }
        let site = CoverageUnit::new(site_file(get("mutatedClass")), line);
        for t in split_tests(m.fields.get(COVERING)) {
            let id = test_id(t);
            cov_b.record(&id, site.clone(), 1);
            assessed.insert(id);
            any_covering = true;
        }
        if m.status != "KILLED" {
            continue;
        }
        killed += 1;
        let matrix_killers = split_tests(m.fields.get("killingTests")).count();
        killed_with_matrix += usize::from(matrix_killers > 0);
        let killers: BTreeSet<&str> = split_tests(m.fields.get("killingTests"))
            .chain(split_tests(m.fields.get("killingTest")))
            .collect();
        for t in killers {
            kill_records.push((test_id(t), mutant.clone()));
        }
    }

    let mut dim_warnings = Vec::new();
    let mut completeness = flagged;
    if killed > 0 && killed_with_matrix == 0 {
        dim_warnings.push(warning(WARN_FULL_MATRIX_NOT_ENABLED, &[]));
        if rank(KillCompleteness::FirstKillerOnly) < rank(completeness) {
            completeness = KillCompleteness::FirstKillerOnly;
        }
    }
    if !version_is_verified(&version) {
        dim_warnings.push(warning(
            WARN_UNVERIFIED_TOOL_VERSION,
            &[("tool", TOOL), ("version", &version)],
        ));
    }
    dim_warnings.append(&mut extra);
    if let Some(s) = sidecar {
        dim_warnings.extend(s.warnings.iter().cloned());
    }
    let unverified = dim_warnings
        .iter()
        .any(|w| w.id == WARN_UNVERIFIED_TOOL_VERSION);

    let mut kills_b = KillBuilder::kills(provenance, completeness);
    for t in &assessed {
        kills_b.add_test(t);
    }
    for (t, m) in kill_records {
        kills_b.record(&t, m, 1);
    }
    let mut kills: KillDimension = kills_b.build().map_err(PitError::Provenance)?;
    kills.warnings.extend(dim_warnings);
    if unverified || rank(completeness) < rank(KillCompleteness::Full) {
        lower(&mut kills.cap, Tier::Low);
    }
    if let Some(cap) = sidecar.and_then(|s| s.cap) {
        lower(&mut kills.cap, cap);
    }

    let coverage: Option<CoverageDimension> = if any_covering {
        let mut c = cov_b.build().map_err(PitError::Provenance)?;
        if unverified {
            c.warnings.push(warning(
                WARN_UNVERIFIED_TOOL_VERSION,
                &[("tool", TOOL), ("version", &version)],
            ));
            lower(&mut c.cap, Tier::Low);
        }
        Some(c)
    } else {
        None
    };

    let mut mutation_run = sidecar.map_or(
        MutationRun {
            scope: MutationScope::Unknown,
            mutants_run: None,
            mutants_total: None,
        },
        |s| s.sidecar.mutation_run(),
    );
    if sidecar.is_some() || opts.kill_completeness.is_some() {
        mutation_run.scope = match flagged {
            KillCompleteness::Sampled => MutationScope::Sampled,
            KillCompleteness::BudgetLimited => MutationScope::BudgetLimited,
            _ if mutation_run.scope == MutationScope::Unknown && sidecar.is_none() => {
                MutationScope::Full
            }
            _ => mutation_run.scope,
        };
    }
    Ok(PitReport {
        matrix: TestMatrix {
            coverage,
            kills: Some(kills),
        },
        mutation_run,
    })
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;

    const MAX: u64 = 1 << 20;

    fn fixture(rel: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("testdata/test-effectiveness")
            .join(rel)
    }

    fn opts() -> PitOptions {
        PitOptions {
            max_input_bytes: MAX,
            commit: Some("abc123".to_owned()),
            ..PitOptions::default()
        }
    }

    fn parse(rel: &str) -> Result<PitReport, PitError> {
        parse_file(&fixture(rel), &opts())
    }

    fn kills_of(r: &PitReport) -> &KillDimension {
        r.matrix.kills.as_ref().expect("kills present")
    }

    fn ids(d: &KillDimension) -> Vec<&'static str> {
        d.warnings.iter().map(|w| w.id).collect()
    }

    #[test]
    fn parse_should_CreditBothKillers_When_KillingTestsAorB() {
        // Given mutation 1 with <killingTests>testAdd|testAddNegative</killingTests>
        let r = parse("pit/full-matrix.xml").unwrap();
        let kills = kills_of(&r);
        // When looking up the mutant's posting list
        let (_, posting) = kills
            .index
            .iter()
            .find(|(m, _)| m.0.contains("add(II)I:10"))
            .expect("mutant on line 10");
        // Then both tests kill it
        let killers: Vec<&str> = posting
            .tests
            .iter()
            .map(|&o| kills.test(o).name.as_str())
            .collect();
        assert_eq!(killers, vec!["testAdd", "testAddNegative"]);
        assert_eq!(kills.meta, KillCompleteness::Full);
        assert_eq!(kills.cap, None);
    }

    #[test]
    fn parse_should_TagCoveringTestsAsMutantSite_When_Parsed() {
        // Given a report with coveringTests
        let r = parse("pit/full-matrix.xml").unwrap();
        // Then coverage is a MutantSite dimension, kept apart from kills
        let cov = r.matrix.coverage.as_ref().expect("coverage present");
        assert_eq!(cov.unit_kind, Some(CoverageUnitKind::MutantSite));
        assert_eq!(cov.provenance.tool, "pit");
        let line10 = cov
            .index
            .get(&CoverageUnit::new("com/example/Calc.java", 10))
            .expect("site on line 10");
        assert_eq!(line10.tests.len(), 3);
        assert_eq!(kills_of(&r).unit_kind, None);
    }

    #[test]
    fn parse_should_WarnAndCapLow_When_FullMutationMatrixNotEnabled() {
        // Given killed mutants carrying only <killingTest> and an empty <killingTests/>
        let r = parse("pit/no-full-matrix.xml").unwrap();
        let kills = kills_of(&r);
        // Then the typed warning and Low cap apply
        assert!(ids(kills).contains(&WARN_FULL_MATRIX_NOT_ENABLED));
        assert_eq!(kills.cap, Some(Tier::Low));
        assert_eq!(kills.meta, KillCompleteness::FirstKillerOnly);
        // And the first killer is still credited
        assert_eq!(kills.index.len(), 2);
    }

    #[test]
    fn parse_should_WarnAndCapLow_When_VersionNotOnAllowList() {
        // Given a report from PIT 0.9.1
        let r = parse("pit/unlisted-version.xml").unwrap();
        let kills = kills_of(&r);
        // Then Ok with the typed warning, naming the version, and Low cap
        let w = kills
            .warnings
            .iter()
            .find(|w| w.id == WARN_UNVERIFIED_TOOL_VERSION)
            .expect("unverified-tool-version");
        assert!(
            w.params
                .contains(&("version".to_owned(), "0.9.1".to_owned()))
        );
        assert_eq!(kills.cap, Some(Tier::Low));
        assert_eq!(r.matrix.coverage.unwrap().cap, Some(Tier::Low));
    }

    #[test]
    fn parse_should_UsePitVersionParam_When_ReportHasNoVersion() {
        let listed = PitOptions {
            pit_version: Some("1.16.1".to_owned()),
            ..opts()
        };
        let r = parse_file(&fixture("pit/no-version.xml"), &listed).unwrap();
        assert_eq!(kills_of(&r).provenance.version.as_deref(), Some("1.16.1"));
        assert!(!ids(kills_of(&r)).contains(&WARN_UNVERIFIED_TOOL_VERSION));
        // Neither source: unknown version, warned
        let r = parse("pit/no-version.xml").unwrap();
        assert_eq!(kills_of(&r).provenance.version.as_deref(), Some("unknown"));
        assert!(ids(kills_of(&r)).contains(&WARN_UNVERIFIED_TOOL_VERSION));
    }

    #[test]
    fn kill_dimension_should_BeBudgetLimitedAndLow_When_SidecarSaysBudgetExhausted() {
        // Given a hand-written sidecar with budget_exhausted
        let sidecar_path = fixture("sidecar/valid/mutation-budget-exhausted.json");
        let sidecar = sidecar::read_sidecar(&sidecar_path, MAX).unwrap();
        let o = PitOptions {
            sidecar: Some(sidecar),
            ..opts()
        };
        // When parsing a full-matrix report
        let r = parse_file(&fixture("pit/full-matrix.xml"), &o).unwrap();
        // Then the dimension is budget-limited, Low, and the run scope says so
        let kills = kills_of(&r);
        assert_eq!(kills.meta, KillCompleteness::BudgetLimited);
        assert_eq!(kills.cap, Some(Tier::Low));
        assert_eq!(r.mutation_run.scope, MutationScope::BudgetLimited);
        assert_eq!(r.mutation_run.mutants_run, Some(412));
        assert_eq!(r.mutation_run.mutants_total, Some(1930));
    }

    #[test]
    fn parse_should_ResolveFlagAndSidecarConflictToLessComplete_When_Disagree() {
        let sidecar =
            sidecar::read_sidecar(&fixture("sidecar/valid/mutation-sampled.json"), MAX).unwrap();
        let o = PitOptions {
            sidecar: Some(sidecar),
            kill_completeness: Some(KillCompleteness::BudgetLimited),
            ..opts()
        };
        let r = parse_file(&fixture("pit/full-matrix.xml"), &o).unwrap();
        assert_eq!(kills_of(&r).meta, KillCompleteness::BudgetLimited);
        assert!(ids(kills_of(&r)).contains(&"kill-completeness-conflict"));
    }

    #[test]
    fn parse_should_ReadNoFile_When_InputOverCap() {
        let o = PitOptions {
            max_input_bytes: 10,
            ..opts()
        };
        let e = parse_file(&fixture("pit/full-matrix.xml"), &o).unwrap_err();
        assert!(matches!(e, PitError::Guard(GuardError::TooLarge(_))), "{e}");
    }

    #[test]
    fn test_id_should_SplitClassAndMethod_When_LegacyOrJupiterName() {
        let legacy = test_id("com.example.CalcTest.testAdd(com.example.CalcTest)");
        assert_eq!(legacy, TestId::new("com/example/CalcTest.java", "testAdd"));
        let jupiter =
            test_id("[engine:junit-jupiter]/[class:com.example.CalcTest]/[method:testAdd()]");
        assert_eq!(jupiter, TestId::new("com/example/CalcTest.java", "testAdd"));
    }
}
