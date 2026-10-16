//! Ingest guard shared by the adapters: input files are untrusted (Story 2.2.7).
//!
//! The size cap is decided from `std::fs::metadata` alone, before any open. JSON nesting
//! depth relies on serde_json's default recursion limit, which is never disabled.

use std::fmt;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;

/// An input file over `--max-input-bytes`: the facts the R26 class E message names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputTooLarge {
    pub file: PathBuf,
    pub size: u64,
    pub cap: u64,
}

impl fmt::Display for InputTooLarge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} is {} bytes, over the {} byte input cap",
            self.file.display(),
            self.size,
            self.cap
        )
    }
}

#[derive(Debug)]
pub enum GuardError {
    TooLarge(InputTooLarge),
    Io {
        file: PathBuf,
        source: std::io::Error,
    },
}

impl fmt::Display for GuardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge(e) => e.fmt(f),
            Self::Io { file, source } => write!(f, "cannot read {}: {source}", file.display()),
        }
    }
}

impl std::error::Error for GuardError {}

/// Returns the file size when it is within `max_input_bytes`; reads metadata only, never opens the file.
pub fn check_size(path: &Path, max_input_bytes: u64) -> Result<u64, GuardError> {
    let size = std::fs::metadata(path)
        .map_err(|source| GuardError::Io {
            file: path.to_path_buf(),
            source,
        })?
        .len();
    if size > max_input_bytes {
        return Err(GuardError::TooLarge(InputTooLarge {
            file: path.to_path_buf(),
            size,
            cap: max_input_bytes,
        }));
    }
    Ok(size)
}

/// Size-checks, then reads the whole file. The open happens only after the cap check passes.
pub fn read_bounded(path: &Path, max_input_bytes: u64) -> Result<Vec<u8>, GuardError> {
    check_size(path, max_input_bytes)?;
    std::fs::read(path).map_err(|source| GuardError::Io {
        file: path.to_path_buf(),
        source,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsonError {
    NestingTooDeep,
    Invalid(String),
}

impl fmt::Display for JsonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NestingTooDeep => f.write_str("not valid JSON (nesting too deep)"),
            Self::Invalid(why) => write!(f, "not valid JSON ({why})"),
        }
    }
}

impl std::error::Error for JsonError {}

/// Classifies a serde_json failure: the recursion-limit error is `NestingTooDeep`, anything else `Invalid`.
pub fn json_error(e: serde_json::Error) -> JsonError {
    if e.to_string().contains("recursion limit exceeded") {
        JsonError::NestingTooDeep
    } else {
        JsonError::Invalid(e.to_string())
    }
}

/// Parses JSON under serde_json's default recursion limit (128); deeper input is `NestingTooDeep`.
pub fn parse_json_bounded<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, JsonError> {
    serde_json::from_slice(bytes).map_err(json_error)
}

/// Size-checks, then opens the file for a streaming read. The open happens only after the cap check passes.
pub fn open_bounded(path: &Path, max_input_bytes: u64) -> Result<std::fs::File, GuardError> {
    check_size(path, max_input_bytes)?;
    std::fs::File::open(path).map_err(|source| GuardError::Io {
        file: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("kibitzer-guard-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[cfg(unix)]
    #[test]
    fn cap_error_should_NameFileSizeAndCapAndOpenNoFileForReading_When_InputOverCap() {
        use std::os::unix::fs::PermissionsExt;
        // Given a sparse file of oversize length that no one may read
        let dir = scratch("cap");
        let path = dir.join("huge.json");
        let f = std::fs::File::create(&path).unwrap();
        f.set_len(5_000).unwrap();
        drop(f);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
        // When checked against a smaller cap
        let err = read_bounded(&path, 1_000).unwrap_err();
        // Then the typed error carries file, size and cap, and no open-for-read error occurred
        match &err {
            GuardError::TooLarge(e) => {
                assert_eq!(e.file, path);
                assert_eq!((e.size, e.cap), (5_000, 1_000));
            }
            other => panic!("expected TooLarge, got {other:?}"),
        }
        let text = err.to_string();
        assert!(text.contains("huge.json") && text.contains("5000") && text.contains("1000"));
        // and a file within the cap is accepted from metadata alone
        assert_eq!(check_size(&path, 5_000).unwrap(), 5_000);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn read_bounded_should_ReturnBytes_When_InputWithinCap() {
        let dir = scratch("ok");
        let path = dir.join("a.json");
        std::fs::write(&path, b"{}").unwrap();
        assert_eq!(read_bounded(&path, 2).unwrap(), b"{}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn open_bounded_should_OpenFile_When_WithinCapAndRefuseWhenOver() {
        let dir = scratch("open");
        let path = dir.join("a.json");
        std::fs::write(&path, b"{}").unwrap();
        assert!(open_bounded(&path, 2).is_ok());
        assert!(matches!(
            open_bounded(&path, 1),
            Err(GuardError::TooLarge(_))
        ));
        assert!(matches!(
            open_bounded(&dir.join("missing.json"), 10),
            Err(GuardError::Io { .. })
        ));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn check_size_should_ReturnIoError_When_FileMissing() {
        let err = check_size(Path::new("/nonexistent/kibitzer-guard"), 10).unwrap_err();
        assert!(matches!(err, GuardError::Io { .. }));
    }

    #[test]
    fn parse_json_bounded_should_ReturnNestingTooDeepAndNotOverflowStack_When_JsonNestedBeyondRecursionLimit()
     {
        let depth = 100_000;
        let doc = format!("{}{}", "[".repeat(depth), "]".repeat(depth));
        let err = parse_json_bounded::<serde_json::Value>(doc.as_bytes()).unwrap_err();
        assert_eq!(err, JsonError::NestingTooDeep);
        assert_eq!(err.to_string(), "not valid JSON (nesting too deep)");
    }

    #[test]
    fn parse_json_bounded_should_ReturnInvalid_When_JsonTruncated() {
        let err = parse_json_bounded::<serde_json::Value>(b"{\"a\":").unwrap_err();
        assert!(matches!(err, JsonError::Invalid(_)));
    }

    fn collect_rs(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                collect_rs(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }

    #[test]
    fn source_tree_should_NeverDisableRecursionLimit_When_Grepped() {
        // Needle is assembled so this file does not match itself.
        let needle = concat!("disable_", "recursion_limit");
        let mut files = Vec::new();
        collect_rs(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut files,
        );
        assert!(!files.is_empty());
        let offenders: Vec<_> = files
            .iter()
            .filter(|p| std::fs::read_to_string(p).is_ok_and(|s| s.contains(needle)))
            .collect();
        assert!(
            offenders.is_empty(),
            "recursion limit disabled in {offenders:?}"
        );
    }
}

/// Tasks 2.2.7d and 2.2.7e: every malformed fixture and a seeded mutation of the corpus go through
/// each experiment-critical adapter, which must return `Ok` or a typed error, never panic or hang.
#[cfg(test)]
#[allow(non_snake_case)]
mod malformed_corpus {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::mpsc;
    use std::time::Duration;

    use crate::test_effectiveness::adapters::{coveragepy, gocover, mutation_elements, pit_xml};

    /// Placeholder: the plan leaves the per-adapter case count to the owner. Kept small enough for
    /// the default suite; raise it for a soak run.
    const MUTATION_CASES_PER_ADAPTER: usize = 200;
    const SEED: u64 = 0x6b69_6269_747a_6572;
    const CASE_TIME_LIMIT: Duration = Duration::from_secs(10);
    const INPUT_CAP: u64 = 1 << 20;
    /// Mutations that duplicate or insert are cut back to this, so a case stays cheap to parse.
    const MAX_MUTATED_BYTES: usize = 256 * 1024;
    /// Coarse sanity bound on peak-RSS growth across one adapter's loop (Linux only).
    const PEAK_RSS_GROWTH_BOUND_KIB: u64 = 512 * 1024;

    #[derive(Clone, Copy, Debug)]
    enum Format {
        CoveragePy,
        GoCover,
        Stryker,
        Pit,
    }

    /// One input: the files an adapter reads, as `(file name, bytes)`.
    type Parts = Vec<(String, Vec<u8>)>;

    fn data(rel: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("testdata/test-effectiveness")
            .join(rel)
    }

    fn read_parts(dir: &Path) -> Parts {
        let mut parts: Parts = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.is_file())
            .map(|p| {
                let name = p.file_name().unwrap().to_string_lossy().into_owned();
                (name, std::fs::read(&p).unwrap())
            })
            .collect();
        parts.sort();
        parts
    }

    fn single(name: &str, path: &Path) -> Parts {
        vec![(name.to_string(), std::fs::read(path).unwrap())]
    }

    /// Malformed fixtures as `(case name, input)`; a file is a case, except gocover where a directory is.
    fn malformed_cases(format: Format) -> Vec<(String, Parts)> {
        let (dir, file_name) = match format {
            Format::CoveragePy => ("malformed/coveragepy", "coverage.json"),
            Format::GoCover => ("malformed/gocover", ""),
            Format::Stryker => ("malformed/stryker", "report.json"),
            Format::Pit => ("malformed/pit", "mutations.xml"),
        };
        let mut entries: Vec<PathBuf> = std::fs::read_dir(data(dir))
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        entries.sort();
        entries
            .into_iter()
            .map(|p| {
                let name = p.file_name().unwrap().to_string_lossy().into_owned();
                let parts = if matches!(format, Format::GoCover) {
                    read_parts(&p)
                } else {
                    single(file_name, &p)
                };
                (name, parts)
            })
            .collect()
    }

    /// Well-formed inputs the mutator starts from alongside the malformed corpus.
    fn valid_seed(format: Format) -> Parts {
        match format {
            Format::CoveragePy => single(
                "coverage.json",
                &data("coveragepy/three-test/coverage.json"),
            ),
            Format::GoCover => read_parts(&data("gocover/twotest/profiles")),
            Format::Stryker => single("report.json", &data("stryker/report-v2.json")),
            Format::Pit => single("mutations.xml", &data("pit/full-matrix.xml")),
        }
    }

    fn scratch_dir() -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "kibitzer-malformed-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Runs the adapter's file-level entry point; `true` when it returned `Ok`.
    fn run_adapter(format: Format, parts: &Parts) -> bool {
        let dir = scratch_dir();
        for (name, bytes) in parts {
            std::fs::write(dir.join(name), bytes).unwrap();
        }
        let ok = match format {
            Format::CoveragePy => {
                coveragepy::read_coveragepy(&dir.join("coverage.json"), INPUT_CAP, None, None)
                    .is_ok()
            }
            Format::GoCover => gocover::parse_dir(&dir, INPUT_CAP).is_ok(),
            Format::Stryker => {
                mutation_elements::parse_report(&dir.join("report.json"), INPUT_CAP, None, None)
                    .is_ok()
            }
            Format::Pit => pit_xml::parse_file(
                &dir.join("mutations.xml"),
                &pit_xml::PitOptions {
                    max_input_bytes: INPUT_CAP,
                    ..Default::default()
                },
            )
            .is_ok(),
        };
        let _ = std::fs::remove_dir_all(&dir);
        ok
    }

    /// Runs one case on a worker thread. `Ok(returned_ok)`, or `Err` naming a panic or a timeout.
    /// A timed-out worker is abandoned (std threads cannot be killed); the test fails anyway.
    fn guarded(format: Format, parts: &Parts) -> Result<bool, String> {
        let (tx, rx) = mpsc::channel();
        let parts = parts.clone();
        std::thread::spawn(move || {
            let outcome = catch_unwind(AssertUnwindSafe(|| run_adapter(format, &parts)));
            let _ = tx.send(outcome.map_err(|p| {
                p.downcast_ref::<String>()
                    .cloned()
                    .or_else(|| p.downcast_ref::<&str>().map(|s| (*s).to_string()))
                    .unwrap_or_else(|| "non-string panic payload".into())
            }));
        });
        match rx.recv_timeout(CASE_TIME_LIMIT) {
            Ok(Ok(ok)) => Ok(ok),
            Ok(Err(msg)) => Err(format!("panicked: {msg}")),
            Err(_) => Err(format!("no result within {CASE_TIME_LIMIT:?}")),
        }
    }

    const FORMATS: [Format; 4] = [
        Format::CoveragePy,
        Format::GoCover,
        Format::Stryker,
        Format::Pit,
    ];

    #[test]
    fn adapters_should_NeverPanicOrExitOtherThanZeroOrTwo_When_MalformedCorpus() {
        let mut failures = Vec::new();
        let mut total = 0;
        for format in FORMATS {
            for (name, parts) in malformed_cases(format) {
                total += 1;
                // Ok-with-warnings or a typed Err both map to exit 0 or 2; a panic or hang does not.
                let outcome = guarded(format, &parts);
                match (&outcome, name.as_str()) {
                    (Err(why), _) => failures.push(format!("{format:?}/{name}: {why}")),
                    (Ok(true), n) if n.starts_with("empty") || n.starts_with("truncated") => {
                        failures.push(format!("{format:?}/{name}: accepted an unusable input"))
                    }
                    _ => {}
                }
            }
        }
        assert!(total >= 4 * 6, "corpus too small: {total} fixtures");
        assert!(failures.is_empty(), "{failures:#?}");
    }

    #[test]
    fn corpus_should_CoverSixFailureClassesPerFormat_When_FixturesListed() {
        let classes: [(&str, &[&str]); 7] = [
            ("truncated", &["truncated"]),
            ("empty", &["empty"]),
            (
                "invalid-utf8/wrong-encoding",
                &["invalid-utf8", "wrong-encoding"],
            ),
            ("bom", &["bom"]),
            ("duplicated-key", &["duplicated"]),
            ("wrong-type", &["wrong-type"]),
            ("out-of-range", &["out-of-range"]),
        ];
        for format in FORMATS {
            let names: Vec<String> = malformed_cases(format).into_iter().map(|c| c.0).collect();
            for (class, prefixes) in classes {
                assert!(
                    names
                        .iter()
                        .any(|n| prefixes.iter().any(|p| n.starts_with(p))),
                    "{format:?} lacks a {class} fixture in {names:?}"
                );
            }
        }
    }

    /// xorshift64*: std-only, deterministic from the seed.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
        }

        fn below(&mut self, n: usize) -> usize {
            (self.next() % n.max(1) as u64) as usize
        }
    }

    const STRUCTURAL: &[u8] = b"{}[]\"<>&;:,\\/ \n\0\xff\xef\xbb\xbf0123456789-";

    fn mutate(rng: &mut Rng, bytes: &mut Vec<u8>) {
        match rng.below(4) {
            0 if !bytes.is_empty() => {
                let i = rng.below(bytes.len());
                bytes[i] ^= 1 << rng.below(8);
            }
            1 => {
                let keep = rng.below(bytes.len() + 1);
                bytes.truncate(keep);
            }
            2 if !bytes.is_empty() => {
                let start = rng.below(bytes.len());
                let end = (start + 1 + rng.below(512)).min(bytes.len());
                let chunk = bytes[start..end].to_vec();
                let at = rng.below(bytes.len() + 1);
                bytes.splice(at..at, chunk);
            }
            _ => {
                let at = rng.below(bytes.len() + 1);
                let count = 1 + rng.below(8);
                let inserted: Vec<u8> = (0..count)
                    .map(|_| STRUCTURAL[rng.below(STRUCTURAL.len())])
                    .collect();
                bytes.splice(at..at, inserted);
            }
        }
        bytes.truncate(MAX_MUTATED_BYTES);
    }

    fn peak_rss_kib() -> Option<u64> {
        let status = std::fs::read_to_string("/proc/self/status").ok()?;
        let line = status.lines().find(|l| l.starts_with("VmHWM:"))?;
        line.split_whitespace().nth(1)?.parse().ok()
    }

    fn mutation_loop(format: Format, salt: u64) {
        let mut seeds: Vec<Parts> = malformed_cases(format).into_iter().map(|c| c.1).collect();
        seeds.push(valid_seed(format));
        let mut rng = Rng(SEED ^ salt);
        let rss_before = peak_rss_kib();
        for case in 0..MUTATION_CASES_PER_ADAPTER {
            let mut parts = seeds[rng.below(seeds.len())].clone();
            for _ in 0..=rng.below(4) {
                let which = rng.below(parts.len());
                mutate(&mut rng, &mut parts[which].1);
            }
            if let Err(why) = guarded(format, &parts) {
                panic!("{format:?} case {case} (seed {SEED:#x} ^ {salt}): {why}");
            }
        }
        if let (Some(before), Some(after)) = (rss_before, peak_rss_kib()) {
            assert!(
                after.saturating_sub(before) <= PEAK_RSS_GROWTH_BOUND_KIB,
                "{format:?}: peak RSS grew {} KiB",
                after - before
            );
        }
    }

    #[test]
    fn coveragepy_should_NeverPanicOrHang_When_SeededMutationOfCorpus() {
        mutation_loop(Format::CoveragePy, 1);
    }

    #[test]
    fn gocover_should_NeverPanicOrHang_When_SeededMutationOfCorpus() {
        mutation_loop(Format::GoCover, 2);
    }

    #[test]
    fn stryker_should_NeverPanicOrHang_When_SeededMutationOfCorpus() {
        mutation_loop(Format::Stryker, 3);
    }

    #[test]
    fn pit_should_NeverPanicOrHang_When_SeededMutationOfCorpus() {
        mutation_loop(Format::Pit, 4);
    }
}
