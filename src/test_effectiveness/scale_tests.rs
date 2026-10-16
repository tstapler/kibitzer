//! Scale tests (Story 2.1.5): the matrix algorithms, the four experiment-critical parsers and the locator at
//! the NFR size. The `#[ignore]`d tests run the target size (`cargo test --release scale -- --ignored`); each has
//! a small-N default-run twin that exercises the same code path so the covgate sees it.
//!
//! `KIBITZER_SCALE_ENTRIES` overrides the target entry count of the ignored parse and matrix tests, to
//! measure the memory curve below the target.

#![allow(non_snake_case)]

use std::collections::BTreeMap;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::analysis::{greedy_reduction, joint_safe_subset, unique_coverage};
use super::matrix::test_support::Rng;
use super::matrix::{
    CoverageAttribution, CoverageDimension, CoverageUnit, Dimension, Posting, Provenance, TestId,
    TestMatrix,
};

/// Proposed bounds (plan Story 2.1.5; the owner may revise them at G0).
const RSS_BOUND_KB: u64 = 1_500_000;
const WALL_BOUND: Duration = Duration::from_secs(120);

const TARGET_TESTS: usize = 100_000;
const TARGET_ENTRIES: u64 = 50_000_000;
const ENTRIES_PER_TEST: u64 = 500;
const ENTRIES_PER_UNIT: u64 = 50;
const MIN_TESTS: usize = 200;
const MIN_UNITS: usize = 1_000;
/// Every Nth unit is covered by exactly one test.
const SINGLETON_EVERY: usize = 10;
const SEED: u64 = 0x5eed_cafe_f00d_1234;

/// Peak-RSS readings share one process-wide high-water mark, so scale runs must not overlap.
static SERIAL: Mutex<()> = Mutex::new(());

fn target_entries() -> u64 {
    std::env::var("KIBITZER_SCALE_ENTRIES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(TARGET_ENTRIES)
}

/// `VmHWM` (peak resident set) in kB from `/proc/self/status`.
fn peak_rss_kb() -> u64 {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    status
        .lines()
        .find_map(|l| l.strip_prefix("VmHWM:"))
        .and_then(|v| v.trim().trim_end_matches("kB").trim().parse().ok())
        .unwrap_or(0)
}

/// Resets the kernel's high-water mark to the current RSS (Linux >= 4.0; a failed write is ignored).
fn reset_peak_rss() {
    let _ = std::fs::write("/proc/self/clear_refs", "5");
}

/// Runs `f`, prints peak RSS and wall clock for `label`, and returns both with the result.
fn measure<T>(label: &str, f: impl FnOnce() -> T) -> (T, u64, Duration) {
    reset_peak_rss();
    let start = Instant::now();
    let out = f();
    let wall = start.elapsed();
    let rss = peak_rss_kb();
    println!(
        "SCALE {label}: peak_rss={:.0} MB wall={:.1} s",
        rss as f64 / 1024.0,
        wall.as_secs_f64()
    );
    (out, rss, wall)
}

/// A temp directory removed on drop, including when an assertion panics.
struct TempTree(PathBuf);

impl TempTree {
    fn new(label: &str) -> Self {
        Self(crate::test_support::unique_temp_dir(label))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempTree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Size of a synthetic matrix; `entries` is approximate (singletons plus a near-even split over the rest).
#[derive(Clone, Copy, Debug)]
struct Shape {
    tests: usize,
    units: usize,
    entries: u64,
}

impl Shape {
    fn target() -> Self {
        Self::for_entries(target_entries())
    }

    /// The NFR shape (100k tests, 1M units) at 5e7 entries; proportionally smaller below it.
    fn for_entries(entries: u64) -> Self {
        Self {
            tests: ((entries / ENTRIES_PER_TEST) as usize).clamp(MIN_TESTS, TARGET_TESTS),
            units: ((entries / ENTRIES_PER_UNIT) as usize).max(MIN_UNITS),
            entries,
        }
    }

    /// Every tenth unit is covered by one test (so unique coverage has work); the rest share `entries` evenly.
    fn units_per_posting(&self, unit: usize) -> usize {
        let singletons = self.units.div_ceil(SINGLETON_EVERY) as u64;
        if unit % SINGLETON_EVERY == 0 {
            return 1;
        }
        let multi = (self.units as u64 - singletons).max(1);
        let rest = self.entries.saturating_sub(singletons);
        let ordinal = (unit - unit.div_ceil(SINGLETON_EVERY)) as u64;
        let k = rest / multi + u64::from(ordinal < rest % multi);
        (k as usize).clamp(1, self.tests)
    }
}

/// Calls `sink(unit, tests)` per unit with `k` distinct ascending test ordinals, deterministic in `seed`.
fn for_each_posting(shape: Shape, seed: u64, mut sink: impl FnMut(usize, &[u32])) {
    let mut rng = Rng(seed);
    let mut buf: Vec<u32> = Vec::new();
    for unit in 0..shape.units {
        let k = shape.units_per_posting(unit);
        buf.clear();
        while buf.len() < k {
            while buf.len() < k {
                buf.push(rng.below(shape.tests as u64) as u32);
            }
            buf.sort_unstable();
            buf.dedup();
        }
        sink(unit, &buf);
    }
}

fn unit_file(unit: usize) -> String {
    format!("src/f{:04}.go", unit / 1000)
}

fn test_id_of(t: u32) -> TestId {
    TestId::new(format!("t{:05}.go", t / 100), format!("Test{t:05}"))
}

fn synth_coverage(shape: Shape) -> CoverageDimension {
    let mut tests: Vec<TestId> = (0..shape.tests as u32).map(test_id_of).collect();
    tests.sort();
    let mut index = BTreeMap::new();
    for_each_posting(shape, SEED, |unit, ts| {
        let posting = Posting {
            tests: ts.to_vec(),
            hits: vec![1; ts.len()],
        };
        index.insert(
            CoverageUnit::new(unit_file(unit), (unit % 1000) as u32),
            posting,
        );
    });
    let mut provenance = Provenance::new("synthetic", Some("1")).with_commit(Some("scale"));
    provenance.attribution = None;
    Dimension {
        provenance,
        meta: CoverageAttribution::Isolated,
        unit_kind: None,
        tests,
        index,
        warnings: Vec::new(),
        cap: None,
    }
}

fn total_entries<U, M>(d: &Dimension<U, M>) -> u64 {
    d.index.values().map(|p| p.tests.len() as u64).sum()
}

/// Generates the matrix, runs the three analyses and returns the entry count, measured per analysis.
fn run_matrix_scale(shape: Shape) -> (u64, u64, Duration) {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (dim, _, _) = measure("matrix generate", || synth_coverage(shape));
    let entries = total_entries(&dim);
    println!(
        "SCALE matrix shape: tests={} units={} entries={entries}",
        dim.tests.len(),
        dim.index.len()
    );
    let flagged: Vec<TestId> = dim.tests.clone();
    let matrix = TestMatrix {
        coverage: Some(dim),
        kills: None,
    };
    let cov = matrix.coverage.as_ref().expect("coverage present");
    let ((unique, picks, safe), rss, wall) = measure("matrix analyses", || {
        let unique = unique_coverage(cov);
        let picks = greedy_reduction(cov);
        let safe = joint_safe_subset(&matrix, &flagged);
        (unique, picks, safe)
    });
    println!(
        "SCALE matrix result: unique_tests={} greedy_picks={} joint_safe={} joint_rejected={}",
        unique.len(),
        picks.len(),
        safe.safe.len(),
        safe.rejected.len()
    );
    assert_eq!(unique.len(), shape.tests, "every test has an entry");
    assert!(!picks.is_empty());
    assert_eq!(safe.safe.len() + safe.rejected.len(), shape.tests);
    (entries, rss, wall)
}

#[test]
#[ignore = "scale: run with `cargo test --release scale -- --ignored`"]
fn scale_should_FinishWithinBounds_When_100kTests1MUnits50MEntries() {
    let (entries, rss, wall) = run_matrix_scale(Shape::target());
    assert!(rss <= RSS_BOUND_KB, "peak RSS {rss} kB over bound");
    assert!(wall <= WALL_BOUND, "wall clock {wall:?} over bound");
    println!("SCALE matrix verdict: entries={entries} within bound");
}

#[test]
fn scale_should_RunAllThreeAnalyses_When_SmallMatrix() {
    let shape = Shape {
        tests: 300,
        units: 2_000,
        entries: 20_000,
    };
    let (entries, _, _) = run_matrix_scale(shape);
    assert!((19_000..=21_000).contains(&entries), "entries={entries}");
}

// ---- Task 2.1.5b: parse memory for the four experiment-critical formats ----

/// Explicit cap passed to every adapter so the built-in ceiling never refuses the scale inputs.
const SCALE_CAP: u64 = u64::MAX;
const WRITE_BUFFER: usize = 1 << 20;
const GO_TESTS_PER_PACKAGE: u32 = 100;
const LINES_PER_FILE: usize = 1_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Format {
    CoveragePy,
    GoCover,
    Stryker,
    Pit,
}

const ALL_FORMATS: [Format; 4] = [
    Format::CoveragePy,
    Format::GoCover,
    Format::Stryker,
    Format::Pit,
];

impl Format {
    fn name(self) -> &'static str {
        match self {
            Self::CoveragePy => "coveragepy",
            Self::GoCover => "gocover",
            Self::Stryker => "stryker",
            Self::Pit => "pit",
        }
    }
}

/// `KIBITZER_SCALE_FORMATS=pit,stryker` narrows the ignored umbrella test to those formats.
fn selected_formats() -> Vec<Format> {
    let wanted = std::env::var("KIBITZER_SCALE_FORMATS").unwrap_or_default();
    ALL_FORMATS
        .into_iter()
        .filter(|f| wanted.is_empty() || wanted.split(',').any(|w| w == f.name()))
        .collect()
}

fn create(path: &Path) -> BufWriter<std::fs::File> {
    BufWriter::with_capacity(WRITE_BUFFER, std::fs::File::create(path).expect("create"))
}

fn join_tests(ts: &[u32], sep: &str, name: impl Fn(u32) -> String) -> String {
    ts.iter().map(|&t| name(t)).collect::<Vec<_>>().join(sep)
}

fn py_context(t: u32) -> String {
    format!("t{:03}.py::test_{t:05}|run", t / GO_TESTS_PER_PACKAGE)
}

fn pit_test(t: u32) -> String {
    let class = format!("p.T{:03}", t / GO_TESTS_PER_PACKAGE);
    format!("{class}.t{t:05}({class})")
}

/// coverage.py `--show-contexts` JSON: one line per unit, listing the contexts that ran it.
fn write_coveragepy(path: &Path, shape: Shape) {
    let mut w = create(path);
    write!(
        w,
        r#"{{"meta":{{"version":"7.4.0","timestamp":"2026-01-01T00:00:00"}},"files":{{"#
    )
    .unwrap();
    let mut open_file: Option<usize> = None;
    for_each_posting(shape, SEED, |unit, ts| {
        let file = unit / LINES_PER_FILE;
        if open_file != Some(file) {
            let sep = match open_file {
                Some(_) => "}},",
                None => "",
            };
            write!(w, r#"{sep}"src/f{file:04}.py":{{"contexts":{{"#).unwrap();
            open_file = Some(file);
        } else {
            w.write_all(b",").unwrap();
        }
        let line = unit % LINES_PER_FILE;
        let ctx = join_tests(ts, ",", |t| format!("\"{}\"", py_context(t)));
        write!(w, r#""{line}":[{ctx}]"#).unwrap();
    });
    w.write_all(b"}}}}").unwrap();
    w.flush().unwrap();
}

/// Stryker mutation-testing-elements JSON: one `Killed` mutant per unit with its `killedBy` list.
fn write_stryker(path: &Path, shape: Shape) {
    let mut w = create(path);
    write!(
        w,
        r#"{{"schemaVersion":"2","framework":{{"name":"stryker","version":"8.0.0"}},"testFiles":{{"#
    )
    .unwrap();
    let per_file = GO_TESTS_PER_PACKAGE as usize;
    for (n, first) in (0..shape.tests).step_by(per_file).enumerate() {
        let tests = (first..(first + per_file).min(shape.tests))
            .map(|t| format!(r#"{{"id":"t{t:05}","name":"test {t}"}}"#))
            .collect::<Vec<_>>()
            .join(",");
        let sep = if n == 0 { "" } else { "," };
        write!(w, r#"{sep}"tests/s{n:04}.spec.ts":{{"tests":[{tests}]}}"#).unwrap();
    }
    w.write_all(br#"},"files":{"#).unwrap();
    let mut open_file: Option<usize> = None;
    for_each_posting(shape, SEED, |unit, ts| {
        let file = unit / LINES_PER_FILE;
        if open_file != Some(file) {
            let sep = match open_file {
                Some(_) => "]},",
                None => "",
            };
            write!(w, r#"{sep}"src/f{file:04}.ts":{{"mutants":["#).unwrap();
            open_file = Some(file);
        } else {
            w.write_all(b",").unwrap();
        }
        let killers = join_tests(ts, ",", |t| format!("\"t{t:05}\""));
        write!(
            w,
            r#"{{"id":"{unit}","status":"Killed","killedBy":[{killers}]}}"#
        )
        .unwrap();
    });
    w.write_all(b"]}}}").unwrap();
    w.flush().unwrap();
}

/// PIT `mutations.xml` with the full kill matrix (`killingTests`): one `KILLED` mutation per unit.
fn write_pit(path: &Path, shape: Shape) {
    let mut w = create(path);
    w.write_all(b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<mutations version=\"1.17.4\">\n")
        .unwrap();
    for_each_posting(shape, SEED, |unit, ts| {
        let class = format!("p.C{:04}", unit / LINES_PER_FILE);
        let line = unit % LINES_PER_FILE;
        let first = pit_test(ts[0]);
        let killers = join_tests(ts, "|", pit_test);
        writeln!(
            w,
            "<mutation detected='true' status='KILLED' numberOfTestsRun='{}'><sourceFile>C.java</sourceFile>\
             <mutatedClass>{class}</mutatedClass><mutatedMethod>m{unit}</mutatedMethod>\
             <methodDescription>()V</methodDescription><lineNumber>{line}</lineNumber>\
             <mutator>org.pitest.mutationtest.engine.gregor.mutators.MathMutator</mutator>\
             <indexes><index>1</index></indexes><blocks><block>0</block></blocks>\
             <killingTest>{first}</killingTest><killingTests>{killers}</killingTests>\
             <succeedingTests/></mutation>",
            ts.len()
        )
        .unwrap();
    });
    w.write_all(b"</mutations>\n").unwrap();
    w.flush().unwrap();
}

/// Go per-test profile directory plus `manifest.json`; each test covers `entries / tests` units.
fn write_gocover(dir: &Path, shape: Shape) {
    std::fs::write(
        dir.join("manifest.json"),
        r#"{"go_version":"go1.26.4","commit":"abc","mode":"isolated","failing":[]}"#,
    )
    .unwrap();
    let per_test = shape.entries / shape.tests as u64;
    let extra = shape.entries % shape.tests as u64;
    let mut units: Vec<u32> = Vec::new();
    for t in 0..shape.tests {
        let k = (per_test + u64::from((t as u64) < extra)) as usize;
        let mut rng = Rng(SEED ^ ((t as u64 + 1) << 20));
        units.clear();
        while units.len() < k.min(shape.units) {
            while units.len() < k.min(shape.units) {
                units.push(rng.below(shape.units as u64) as u32);
            }
            units.sort_unstable();
            units.dedup();
        }
        let pkg = t as u32 / GO_TESTS_PER_PACKAGE;
        let mut w = create(&dir.join(format!("pkg%2Fp{pkg:03}__Test{t:05}.out")));
        w.write_all(b"mode: set\n").unwrap();
        for &u in &units {
            let (file, line) = (u as usize / LINES_PER_FILE, u as usize % LINES_PER_FILE + 1);
            writeln!(w, "pkg/f{file:04}.go:{line}.1,{line}.20 1 1").unwrap();
        }
        w.flush().unwrap();
    }
}

/// What one adapter run measured.
struct ParseOutcome {
    entries: u64,
    input_bytes: u64,
    rss_kb: u64,
    wall: Duration,
}

fn dir_bytes(dir: &Path) -> u64 {
    std::fs::read_dir(dir)
        .expect("read_dir")
        .flatten()
        .filter_map(|e| e.metadata().ok())
        .map(|m| m.len())
        .sum()
}

/// Generates the input for `format` at `entries`, parses it through the real adapter and measures it.
fn run_parse_scale(format: Format, entries: u64) -> ParseOutcome {
    use super::adapters::{coveragepy, gocover, mutation_elements, pit_xml};
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let shape = Shape::for_entries(entries);
    let tree = TempTree::new(&format!("scale-{}", format.name()));
    let file = tree.path().join("input");
    let started = Instant::now();
    match format {
        Format::CoveragePy => write_coveragepy(&file, shape),
        Format::Stryker => write_stryker(&file, shape),
        Format::Pit => write_pit(&file, shape),
        Format::GoCover => write_gocover(tree.path(), shape),
    }
    let input_bytes = match format {
        Format::GoCover => dir_bytes(tree.path()),
        _ => std::fs::metadata(&file).expect("input").len(),
    };
    println!(
        "SCALE {} generated: input={:.0} MB in {:.1} s",
        format.name(),
        input_bytes as f64 / 1e6,
        started.elapsed().as_secs_f64()
    );
    let label = format!("parse {}", format.name());
    let (parsed, rss_kb, wall) = measure(&label, || parse_entries(format, tree.path(), &file));
    ParseOutcome {
        entries: parsed,
        input_bytes,
        rss_kb,
        wall,
    }
}

/// Parses with the real adapter and returns the matrix entry count; the dimension is dropped before returning.
fn parse_entries(format: Format, dir: &Path, file: &Path) -> u64 {
    use super::adapters::{coveragepy, gocover, mutation_elements, pit_xml};
    match format {
        Format::CoveragePy => {
            total_entries(&coveragepy::read_coveragepy(file, SCALE_CAP, None, None).expect("parse"))
        }
        Format::GoCover => {
            total_entries(&gocover::parse_dir(dir, SCALE_CAP).expect("parse").dimension)
        }
        Format::Stryker => total_entries(
            &mutation_elements::parse_report(file, SCALE_CAP, None, None)
                .expect("parse")
                .kills,
        ),
        Format::Pit => {
            let opts = pit_xml::PitOptions {
                max_input_bytes: SCALE_CAP,
                commit: Some("scale".to_owned()),
                ..Default::default()
            };
            let report = pit_xml::parse_file(file, &opts).expect("parse");
            total_entries(report.matrix.kills.as_ref().expect("kills"))
        }
    }
}

/// Prints the per-adapter verdict line and returns the names of the bounds it broke.
fn verdict(format: Format, expected: u64, out: &ParseOutcome) -> Vec<String> {
    let mut broken = Vec::new();
    if out.rss_kb > RSS_BOUND_KB {
        broken.push(format!("{} peak RSS {} kB", format.name(), out.rss_kb));
    }
    if out.wall > WALL_BOUND {
        broken.push(format!("{} wall {:?}", format.name(), out.wall));
    }
    println!(
        "SCALE {} verdict: entries={} (target {expected}) input={} bytes rss={} kB wall={:.1} s -> {}",
        format.name(),
        out.entries,
        out.input_bytes,
        out.rss_kb,
        out.wall.as_secs_f64(),
        if broken.is_empty() { "PASS" } else { "FAIL" }
    );
    broken
}

/// Within 2% of the requested entries (random collisions and rounding make it inexact).
fn assert_entries_close(format: Format, expected: u64, got: u64) {
    let slack = expected / 50 + 1;
    assert!(
        got.abs_diff(expected) <= slack,
        "{}: parsed {got} entries, expected about {expected}",
        format.name()
    );
}

#[test]
#[ignore = "scale: run with `cargo test --release parse_scale -- --ignored`"]
fn parse_scale_should_StayWithinMemoryBound_When_EachAdapterFormatAtTargetSize() {
    let entries = target_entries();
    let mut broken = Vec::new();
    for format in selected_formats() {
        let out = run_parse_scale(format, entries);
        assert_entries_close(format, entries, out.entries);
        broken.extend(verdict(format, entries, &out));
    }
    assert!(broken.is_empty(), "over the bound: {broken:?}");
}

#[test]
#[ignore = "scale: run with `cargo test --release parse_scale -- --ignored`"]
fn gocover_parse_scale_should_BoundPeakRssByMatrixNotDirectoryTotal_When_DirectoryOfProfilesAtTargetSize()
 {
    let entries = target_entries();
    let out = run_parse_scale(Format::GoCover, entries);
    assert_entries_close(Format::GoCover, entries, out.entries);
    assert!(verdict(Format::GoCover, entries, &out).is_empty());
    assert!(
        out.rss_kb * 1024 < out.input_bytes,
        "peak RSS {} kB is not below the directory total {} bytes",
        out.rss_kb,
        out.input_bytes
    );
}

const SMALL_ENTRIES: u64 = 20_000;

#[test]
fn parse_scale_should_ParseEveryFormatToTheRequestedEntryCount_When_SmallInputs() {
    for format in ALL_FORMATS {
        let out = run_parse_scale(format, SMALL_ENTRIES);
        assert_entries_close(format, SMALL_ENTRIES, out.entries);
        assert!(verdict(format, SMALL_ENTRIES, &out).is_empty());
    }
}
