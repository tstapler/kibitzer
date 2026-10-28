//! `TestMatrix`: the single seam every adapter writes and every analysis reads (Task 2.1.1a).
//!
//! Each dimension (coverage, kills) is a sparse inverted index: per unit, a sorted `u32` posting list of test
//! ordinals plus hit counts (ADR-005). Test ordinals follow `TestId` order, so ordinal order is test-id order.

use std::collections::{BTreeMap, HashMap};

/// Stable identifier of a test: `(repo-relative file, qualified name)`, ordered by file then name.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TestId {
    pub file: String,
    pub name: String,
}

impl TestId {
    pub fn new(file: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            file: file.into(),
            name: name.into(),
        }
    }
}

impl std::fmt::Display for TestId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}::{}", self.file, self.name)
    }
}

/// A mutation identifier from a mutation tool's report; never produced by kibitzer.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MutantId(pub String);

/// A line a test can cover.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CoverageUnit {
    pub file: String,
    pub line: u32,
}

impl CoverageUnit {
    pub fn new(file: impl Into<String>, line: u32) -> Self {
        Self {
            file: file.into(),
            line,
        }
    }
}

/// Upper bound on a finding's confidence; caps only ever lower a tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tier {
    Low,
    Medium,
    High,
}

pub type ConfidenceTier = Tier;

/// How per-test coverage was produced (pitfall P3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CoverageAttribution {
    Isolated,
    InOrder,
    LeaveOneOut,
}

impl CoverageAttribution {
    pub fn label(self) -> &'static str {
        match self {
            Self::Isolated => "isolated",
            Self::InOrder => "in-order",
            Self::LeaveOneOut => "leave-one-out",
        }
    }
}

/// An attribution plus whether it was assumed (adapter default) rather than carried or supplied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttributionChoice {
    pub attribution: CoverageAttribution,
    pub assumed: bool,
}

impl AttributionChoice {
    /// Resolution order: value the input carries, then the `--attribution` flag, then the adapter default (assumed).
    pub fn resolve(
        carried: Option<CoverageAttribution>,
        flag: Option<CoverageAttribution>,
        adapter_default: CoverageAttribution,
    ) -> Self {
        match carried.or(flag) {
            Some(attribution) => Self {
                attribution,
                assumed: false,
            },
            None => Self {
                attribution: adapter_default,
                assumed: true,
            },
        }
    }
}

/// Whether a kill report lists every killer of each mutant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KillCompleteness {
    Full,
    FirstKillerOnly,
    Sampled,
    BudgetLimited,
}

impl KillCompleteness {
    pub fn label(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::FirstKillerOnly => "first-killer-only",
            Self::Sampled => "sampled",
            Self::BudgetLimited => "budget-limited",
        }
    }
}

/// PIT `coveringTests` is mutant-site coverage, not line coverage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CoverageUnitKind {
    Line,
    MutantSite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarningClass {
    /// Lowers confidence but the report still completes.
    W,
    /// Informational.
    I,
}

/// Typed warning: a stable slug plus parameters; the sentence is rendered by the report (Task 2.3.1f).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Warning {
    pub id: &'static str,
    pub class: WarningClass,
    pub params: Vec<(String, String)>,
}

impl Warning {
    fn new(id: &'static str, class: WarningClass) -> Self {
        Self {
            id,
            class,
            params: Vec::new(),
        }
    }
}

pub const WARN_INPUT_COMMIT_UNKNOWN: &str = "input-commit-unknown";
pub const WARN_ATTRIBUTION_IN_ORDER: &str = "attribution-in-order";

/// Where a dimension came from. Tool and version are required; the rest may be absent.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Provenance {
    pub tool: String,
    pub version: Option<String>,
    pub run_timestamp: Option<String>,
    pub commit: Option<String>,
    pub language: Option<String>,
    /// Set for coverage dimensions only.
    pub attribution: Option<AttributionChoice>,
}

impl Provenance {
    pub fn new(tool: impl Into<String>, version: Option<&str>) -> Self {
        Self {
            tool: tool.into(),
            version: version.map(str::to_owned),
            ..Self::default()
        }
    }

    pub fn with_commit(mut self, commit: Option<&str>) -> Self {
        self.commit = commit.map(str::to_owned);
        self
    }
}

/// Adapters reject inputs lacking tool and version; nothing else is an error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingProvenance {
    pub field: &'static str,
}

impl std::fmt::Display for MissingProvenance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "input provenance is missing the tool {}", self.field)
    }
}

impl std::error::Error for MissingProvenance {}

/// Per-dimension metadata that derives warnings and the confidence cap.
pub trait DimensionMeta: Copy {
    fn label(&self) -> &'static str;
    fn derive(&self, provenance: &Provenance) -> (Vec<Warning>, Option<Tier>);
}

impl DimensionMeta for CoverageAttribution {
    fn label(&self) -> &'static str {
        CoverageAttribution::label(*self)
    }

    fn derive(&self, provenance: &Provenance) -> (Vec<Warning>, Option<Tier>) {
        let mut warnings = Vec::new();
        let assumed = provenance.attribution.is_some_and(|a| a.assumed);
        if *self == CoverageAttribution::InOrder && assumed {
            warnings.push(Warning::new(WARN_ATTRIBUTION_IN_ORDER, WarningClass::W));
        }
        let cap = (*self == CoverageAttribution::InOrder).then_some(Tier::Medium);
        (warnings, cap)
    }
}

impl DimensionMeta for KillCompleteness {
    fn label(&self) -> &'static str {
        KillCompleteness::label(*self)
    }

    fn derive(&self, _: &Provenance) -> (Vec<Warning>, Option<Tier>) {
        (Vec::new(), (*self != Self::Full).then_some(Tier::Low))
    }
}

/// Test ordinals covering one unit, sorted ascending, with a parallel hit-count list.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Posting {
    pub tests: Vec<u32>,
    pub hits: Vec<u32>,
}

/// One dimension of the matrix: sparse inverted index plus its own provenance.
#[derive(Debug, Clone)]
pub struct Dimension<U, M> {
    pub provenance: Provenance,
    pub meta: M,
    pub unit_kind: Option<CoverageUnitKind>,
    /// Sorted by `TestId`; the index into this vector is the test ordinal.
    pub tests: Vec<TestId>,
    pub index: BTreeMap<U, Posting>,
    pub warnings: Vec<Warning>,
    /// Structural upper bound on any finding derived from this dimension.
    pub cap: Option<Tier>,
}

pub type CoverageDimension = Dimension<CoverageUnit, CoverageAttribution>;
pub type KillDimension = Dimension<MutantId, KillCompleteness>;

impl<U: Ord, M: DimensionMeta> Dimension<U, M> {
    pub fn test(&self, ordinal: u32) -> &TestId {
        &self.tests[ordinal as usize]
    }

    pub fn ordinal_of(&self, test: &TestId) -> Option<u32> {
        self.tests.binary_search(test).ok().map(|i| i as u32)
    }

    /// Units covered by exactly one test, found from posting-list length alone.
    pub fn singleton_units(&self) -> impl Iterator<Item = (&U, u32)> {
        self.index
            .iter()
            .filter(|(_, p)| p.tests.len() == 1)
            .map(|(u, p)| (u, p.tests[0]))
    }

    /// One-line provenance, e.g. `coverage.py 7.4 (in-order)`.
    pub fn describe(&self) -> String {
        let version = self.provenance.version.as_deref().unwrap_or("?");
        format!(
            "{} {} ({})",
            self.provenance.tool,
            version,
            self.meta.label()
        )
    }
}

pub type CoverageBuilder = DimensionBuilder<CoverageUnit, CoverageAttribution>;
pub type KillBuilder = DimensionBuilder<MutantId, KillCompleteness>;

pub struct DimensionBuilder<U, M> {
    provenance: Provenance,
    meta: M,
    unit_kind: Option<CoverageUnitKind>,
    interned: HashMap<TestId, u32>,
    names: Vec<TestId>,
    /// Units seen so far, numbered in first-seen order; entries carry the number, not an owned unit.
    unit_ids: BTreeMap<U, u32>,
    /// `(unit number, test number, hits)`: 12 bytes per entry however long the unit name is.
    entries: Vec<(u32, u32, u32)>,
}

impl<U: Ord, M: DimensionMeta> DimensionBuilder<U, M> {
    fn start(provenance: Provenance, meta: M) -> Self {
        Self {
            provenance,
            meta,
            unit_kind: None,
            interned: HashMap::new(),
            names: Vec::new(),
            unit_ids: BTreeMap::new(),
            entries: Vec::new(),
        }
    }

    pub fn unit_kind(mut self, kind: CoverageUnitKind) -> Self {
        self.unit_kind = Some(kind);
        self
    }

    fn intern(&mut self, test: &TestId) -> u32 {
        if let Some(&id) = self.interned.get(test) {
            return id;
        }
        let id = self.names.len() as u32;
        self.interned.insert(test.clone(), id);
        self.names.push(test.clone());
        id
    }

    /// Registers a test that may cover or kill nothing, so it still counts as assessed.
    pub fn add_test(&mut self, test: &TestId) {
        self.intern(test);
    }

    pub fn record(&mut self, test: &TestId, unit: U, hits: u32) {
        self.record_all(std::iter::once(test), unit, hits);
    }

    /// Records `unit` once for each of `tests`, looking the unit up a single time.
    pub fn record_all<'a>(
        &mut self,
        tests: impl IntoIterator<Item = &'a TestId>,
        unit: U,
        hits: u32,
    ) {
        let mut unit = Some(unit);
        let mut number = None;
        for test in tests {
            let test = self.intern(test);
            let n = *number
                .get_or_insert_with(|| self.unit_number(unit.take().expect("unit taken once")));
            self.entries.push((n, test, hits));
        }
    }

    /// Number of `unit`, assigned on first sight; only called once a test is recorded against it.
    fn unit_number(&mut self, unit: U) -> u32 {
        if let Some(&number) = self.unit_ids.get(&unit) {
            return number;
        }
        let number = self.unit_ids.len() as u32;
        self.unit_ids.insert(unit, number);
        number
    }

    /// Provenance fields only known once the input has been read (a version that follows the data).
    pub fn provenance_mut(&mut self) -> &mut Provenance {
        &mut self.provenance
    }

    /// Replaces the dimension metadata when it is only known after the input has been read.
    pub fn set_meta(&mut self, meta: M) {
        self.meta = meta;
    }

    pub fn build(self) -> Result<Dimension<U, M>, MissingProvenance> {
        if self.provenance.tool.trim().is_empty() {
            return Err(MissingProvenance { field: "name" });
        }
        if self
            .provenance
            .version
            .as_deref()
            .is_none_or(|v| v.trim().is_empty())
        {
            return Err(MissingProvenance { field: "version" });
        }
        let mut order: Vec<u32> = (0..self.names.len() as u32).collect();
        order.sort_by(|&a, &b| self.names[a as usize].cmp(&self.names[b as usize]));
        let mut ordinal_of = vec![0u32; order.len()];
        for (ordinal, &temp) in order.iter().enumerate() {
            ordinal_of[temp as usize] = ordinal as u32;
        }
        let tests: Vec<TestId> = order
            .iter()
            .map(|&t| self.names[t as usize].clone())
            .collect();

        let index = group_postings(self.unit_ids, self.entries, &ordinal_of, RELEASE_EVERY);

        let (mut warnings, mut cap) = self.meta.derive(&self.provenance);
        if self.provenance.commit.is_none() {
            warnings.push(Warning::new(WARN_INPUT_COMMIT_UNKNOWN, WarningClass::W));
            cap = Some(cap.map_or(Tier::Low, |c| c.min(Tier::Low)));
        }
        Ok(Dimension {
            provenance: self.provenance,
            meta: self.meta,
            unit_kind: self.unit_kind,
            tests,
            index,
            warnings,
            cap,
        })
    }
}

/// Turns `(unit number, temp test number, hits)` entries into the sorted unit index: renumbers units into key
/// order and tests into ordinal order, sorts in place, then reads each unit's run off as one posting.
fn group_postings<U: Ord>(
    unit_ids: BTreeMap<U, u32>,
    mut entries: Vec<(u32, u32, u32)>,
    ordinal_of: &[u32],
    release_every: usize,
) -> BTreeMap<U, Posting> {
    let mut units: Vec<U> = Vec::with_capacity(unit_ids.len());
    let mut rank = vec![0u32; unit_ids.len()];
    for (position, (unit, number)) in unit_ids.into_iter().enumerate() {
        rank[number as usize] = position as u32;
        units.push(unit);
    }
    for entry in &mut entries {
        *entry = (
            rank[entry.0 as usize],
            ordinal_of[entry.1 as usize],
            entry.2,
        );
    }
    entries.sort_unstable_by_key(|&(unit, test, _)| (unit, test));
    // Consume runs from the back and hand the tail pages back as we go, so the 12-byte entries and the 8-byte
    // postings are never both fully resident.
    let mut postings: Vec<Posting> = Vec::with_capacity(units.len());
    let mut end = entries.len();
    let mut consumed = 0;
    while end > 0 {
        let unit = entries[end - 1].0;
        let start = entries[..end]
            .iter()
            .rposition(|e| e.0 != unit)
            .map_or(0, |p| p + 1);
        let run = end - start;
        let mut posting = Posting {
            tests: Vec::with_capacity(run),
            hits: Vec::with_capacity(run),
        };
        for &(_, test, hits) in &entries[start..end] {
            if posting.tests.last() == Some(&test) {
                let last = posting.hits.last_mut().expect("parallel lists");
                *last = last.saturating_add(hits);
            } else {
                posting.tests.push(test);
                posting.hits.push(hits);
            }
        }
        postings.push(posting);
        end = start;
        consumed += run;
        if consumed >= release_every {
            entries.truncate(end);
            entries.shrink_to_fit();
            consumed = 0;
        }
    }
    postings.reverse();
    units.into_iter().zip(postings).collect()
}

/// Entries consumed between releases of the entry buffer's tail in `group_postings`.
const RELEASE_EVERY: usize = 1 << 20;

impl CoverageBuilder {
    pub fn coverage(mut provenance: Provenance, choice: AttributionChoice) -> Self {
        provenance.attribution = Some(choice);
        Self::start(provenance, choice.attribution)
    }
}

impl KillBuilder {
    pub fn kills(provenance: Provenance, completeness: KillCompleteness) -> Self {
        Self::start(provenance, completeness)
    }
}

/// Either dimension may be absent; they may come from different tools, runs and commits.
#[derive(Debug, Clone, Default)]
pub struct TestMatrix {
    pub coverage: Option<CoverageDimension>,
    pub kills: Option<KillDimension>,
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    /// xorshift64: seeded, std-only (ADR-005 amendment).
    pub struct Rng(pub u64);

    impl Rng {
        pub fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }

        pub fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    pub fn tid(name: &str) -> TestId {
        TestId::new("t.go", name)
    }

    pub fn prov() -> Provenance {
        Provenance::new("tool", Some("1")).with_commit(Some("abc"))
    }

    pub fn isolated() -> AttributionChoice {
        AttributionChoice {
            attribution: CoverageAttribution::Isolated,
            assumed: false,
        }
    }

    /// Coverage dimension from `(test, units)` pairs; lines are in file `p.go`.
    pub fn cov(rows: &[(&str, &[u32])]) -> CoverageDimension {
        let mut b = CoverageBuilder::coverage(prov(), isolated());
        for (t, units) in rows {
            b.add_test(&tid(t));
            for &u in *units {
                b.record(&tid(t), CoverageUnit::new("p.go", u), 1);
            }
        }
        b.build().expect("valid provenance")
    }

    pub fn kills(rows: &[(&str, &[&str])]) -> KillDimension {
        let mut b = KillBuilder::kills(prov(), KillCompleteness::Full);
        for (t, ms) in rows {
            b.add_test(&tid(t));
            for m in *ms {
                b.record(&tid(t), MutantId((*m).to_owned()), 1);
            }
        }
        b.build().expect("valid provenance")
    }
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::test_support::*;
    use super::*;

    fn cov_builder(p: Provenance, c: AttributionChoice) -> CoverageBuilder {
        CoverageBuilder::coverage(p, c)
    }

    #[test]
    fn build_should_ReturnOk_When_ToolAndVersionPresent() {
        let mut b = cov_builder(prov(), isolated());
        b.record(&tid("TestA"), CoverageUnit::new("p.go", 1), 3);
        let d = b.build().expect("ok");
        assert_eq!(d.tests, vec![tid("TestA")]);
        assert_eq!(d.index.values().next().unwrap().hits, vec![3]);
    }

    #[test]
    fn build_should_ReturnMissingProvenance_When_VersionNone() {
        let p = Provenance::new("coverage.py", None);
        let r = cov_builder(p, isolated()).build();
        assert_eq!(r.unwrap_err(), MissingProvenance { field: "version" });
        let r = KillBuilder::kills(Provenance::new("", Some("1")), KillCompleteness::Full).build();
        assert_eq!(r.unwrap_err(), MissingProvenance { field: "name" });
    }

    #[test]
    fn build_should_ReturnOkWithWarning_When_CommitNone() {
        let p = Provenance::new("coverage.py", Some("7.4"));
        let d = cov_builder(p, isolated())
            .build()
            .expect("commit is optional");
        assert!(d.provenance.commit.is_none());
        assert!(d.warnings.contains(&Warning {
            id: "input-commit-unknown",
            class: WarningClass::W,
            params: vec![],
        }));
        assert_eq!(d.cap, Some(Tier::Low));
    }

    #[test]
    fn build_should_AssumeInOrderWithWarningAndMediumCap_When_NoAttributionSupplied() {
        let choice = AttributionChoice::resolve(None, None, CoverageAttribution::InOrder);
        let p = Provenance::new("coverage.py", Some("7.4")).with_commit(Some("abc"));
        let d = cov_builder(p, choice).build().unwrap();
        assert_eq!(
            d.provenance.attribution,
            Some(AttributionChoice {
                attribution: CoverageAttribution::InOrder,
                assumed: true
            })
        );
        assert!(
            d.warnings
                .iter()
                .any(|w| w.id == "attribution-in-order" && w.class == WarningClass::W)
        );
        assert_eq!(d.cap, Some(Tier::Medium));
    }

    #[test]
    fn build_should_NotWarn_When_AttributionFlagIsolated() {
        let choice = AttributionChoice::resolve(
            None,
            Some(CoverageAttribution::Isolated),
            CoverageAttribution::InOrder,
        );
        let d = cov_builder(prov(), choice).build().unwrap();
        let a = d.provenance.attribution.unwrap();
        assert_eq!(a.attribution, CoverageAttribution::Isolated);
        assert!(!a.assumed);
        assert!(d.warnings.is_empty());
        assert_eq!(d.cap, None);
    }

    #[test]
    fn resolve_should_PreferCarriedOverFlagOverDefault() {
        let c = AttributionChoice::resolve(
            Some(CoverageAttribution::LeaveOneOut),
            Some(CoverageAttribution::Isolated),
            CoverageAttribution::InOrder,
        );
        assert_eq!(c.attribution, CoverageAttribution::LeaveOneOut);
        assert!(!c.assumed);
    }

    #[test]
    fn explicit_in_order_should_CapMediumWithoutAssumedWarning() {
        let choice = AttributionChoice::resolve(
            Some(CoverageAttribution::InOrder),
            None,
            CoverageAttribution::Isolated,
        );
        let d = cov_builder(prov(), choice).build().unwrap();
        assert!(d.warnings.is_empty());
        assert_eq!(d.cap, Some(Tier::Medium));
    }

    #[test]
    fn kills_should_CapLow_When_CompletenessNotFull() {
        for c in [
            KillCompleteness::FirstKillerOnly,
            KillCompleteness::Sampled,
            KillCompleteness::BudgetLimited,
        ] {
            let d = KillBuilder::kills(prov(), c).build().unwrap();
            assert_eq!(d.cap, Some(Tier::Low));
        }
        let d = KillBuilder::kills(prov(), KillCompleteness::Full)
            .build()
            .unwrap();
        assert_eq!(d.cap, None);
    }

    #[test]
    fn finding_should_PrintEachDimensionsProvenance_When_CoveragePyAndPitMixed() {
        let choice = AttributionChoice::resolve(None, None, CoverageAttribution::InOrder);
        let cov = cov_builder(
            Provenance::new("coverage.py", Some("7.4")).with_commit(Some("c")),
            choice,
        )
        .build()
        .unwrap();
        let kills = KillBuilder::kills(
            Provenance::new("pit", Some("1.15")).with_commit(Some("d")),
            KillCompleteness::Full,
        )
        .unit_kind(CoverageUnitKind::MutantSite)
        .build()
        .unwrap();
        let m = TestMatrix {
            coverage: Some(cov),
            kills: Some(kills),
        };
        assert_eq!(
            m.coverage.as_ref().unwrap().describe(),
            "coverage.py 7.4 (in-order)"
        );
        assert_eq!(m.kills.as_ref().unwrap().describe(), "pit 1.15 (full)");
    }

    #[test]
    fn unique_coverage_should_UsePostingListLengthOnly_When_UnitCoveredByOneTest() {
        let d = cov(&[("TestA", &[1, 2]), ("TestB", &[2])]);
        let singles: Vec<_> = d
            .singleton_units()
            .map(|(u, t)| (u.line, d.test(t).name.clone()))
            .collect();
        assert_eq!(singles, vec![(1, "TestA".to_owned())]);
    }

    #[test]
    fn test_id_ordering_should_PlaceTestABeforeTestB_When_IteratedAcrossFiles() {
        let mut b = cov_builder(prov(), isolated());
        let tb = TestId::new("b_test.go", "TestB");
        let ta = TestId::new("a_test.go", "TestA");
        b.record(&tb, CoverageUnit::new("p.go", 1), 1);
        b.record(&ta, CoverageUnit::new("p.go", 1), 1);
        let d = b.build().unwrap();
        assert_eq!(d.tests, vec![ta, tb]);
        assert_eq!(d.index.values().next().unwrap().tests, vec![0, 1]);
    }

    #[test]
    fn build_should_SumHitsAndKeepPostingSorted_When_TestRecordedTwice() {
        let mut b = cov_builder(prov(), isolated());
        b.record(&tid("TestB"), CoverageUnit::new("p.go", 1), 2);
        b.record(&tid("TestA"), CoverageUnit::new("p.go", 1), 1);
        b.record(&tid("TestB"), CoverageUnit::new("p.go", 1), 5);
        let p = b.build().unwrap().index.into_values().next().unwrap();
        assert_eq!(p.tests, vec![0, 1]);
        assert_eq!(p.hits, vec![1, 7]);
    }

    #[test]
    fn group_postings_should_MatchUnreleasedResult_When_EntryBufferReleasedEveryEntry() {
        let mut rng = Rng(0x1234_5678_9ABC_DEF1);
        let ordinal_of: Vec<u32> = (0..6).rev().collect();
        let unit_ids: BTreeMap<u32, u32> = (0..9).map(|u| (u, 8 - u)).collect();
        let entries: Vec<(u32, u32, u32)> = (0..200)
            .map(|_| (rng.below(9) as u32, rng.below(6) as u32, 1))
            .collect();
        let eager = group_postings(unit_ids.clone(), entries.clone(), &ordinal_of, 1);
        let lazy = group_postings(unit_ids, entries, &ordinal_of, usize::MAX);
        assert_eq!(eager, lazy);
        assert!(!eager.is_empty());
    }

    #[test]
    fn record_all_should_RegisterNoUnit_When_NoTestsGiven() {
        let mut b = cov_builder(prov(), isolated());
        b.record_all(std::iter::empty(), CoverageUnit::new("p.go", 1), 1);
        b.record_all([&tid("TestA")], CoverageUnit::new("p.go", 2), 3);
        let d = b.build().unwrap();
        assert_eq!(d.index.len(), 1);
        assert_eq!(d.index.keys().next(), Some(&CoverageUnit::new("p.go", 2)));
    }

    #[test]
    fn builder_should_UseLateProvenanceAndMeta_When_SetBeforeBuild() {
        let mut b = KillBuilder::kills(Provenance::new("tool", None), KillCompleteness::Full);
        b.provenance_mut().version = Some("2".to_owned());
        b.set_meta(KillCompleteness::Sampled);
        b.record(&tid("TestA"), MutantId("m".to_owned()), 1);
        let d = b.build().expect("version supplied late");
        assert_eq!(d.provenance.version.as_deref(), Some("2"));
        assert_eq!(d.meta, KillCompleteness::Sampled);
    }

    #[test]
    fn posting_lists_should_BeSortedAndDeduplicated_When_RandomEntries() {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        for _ in 0..50 {
            let mut b = cov_builder(prov(), isolated());
            for _ in 0..60 {
                let t = format!("T{}", rng.below(8));
                b.record(&tid(&t), CoverageUnit::new("p.go", rng.below(10) as u32), 1);
            }
            let d = b.build().unwrap();
            assert!(d.tests.windows(2).all(|w| w[0] < w[1]));
            for p in d.index.values() {
                assert!(p.tests.windows(2).all(|w| w[0] < w[1]));
                assert_eq!(p.tests.len(), p.hits.len());
            }
        }
    }
}
