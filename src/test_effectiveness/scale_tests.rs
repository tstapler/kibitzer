//! Scale tests (Story 2.1.5): the matrix algorithms, the four experiment-critical parsers and the locator at
//! the NFR size. The `#[ignore]`d tests run the target size (`cargo test --release scale -- --ignored`); each has
//! a small-N default-run twin that exercises the same code path so the covgate sees it.
//!
//! `KIBITZER_SCALE_ENTRIES` overrides the target entry count of the ignored parse and matrix tests, to
//! measure the memory curve below the target.

#![allow(non_snake_case)]

use std::collections::BTreeMap;
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
const TARGET_UNITS: usize = 1_000_000;
const TARGET_ENTRIES: u64 = 50_000_000;
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
        Self {
            tests: TARGET_TESTS,
            units: TARGET_UNITS,
            entries: target_entries(),
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

fn total_entries(d: &CoverageDimension) -> u64 {
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
