//! Go per-test cover-profile directory adapter (Story 2.2.2).
//!
//! Reads `profiles/<pkg-encoded>__<TestName>.out` plus a required `manifest.json` written by
//! `scripts/test-effectiveness/per-test-go-coverage.sh`. The binary never runs `go test`.
//!
//! - The package import path is percent-encoded into the file name (`/` as `%2F`, `_` as `%5F`, `%` as `%25`)
//!   so the first `__` always separates package from test name, and two packages with a `TestParse` stay
//!   two distinct `TestId`s. A `TestId` here is `(package import path, test name)`; the locator maps it to
//!   a source file (Story 2.1.4).
//! - Attribution comes from the manifest `mode` (`isolated` | `in_order`). There is no default: a directory
//!   without `manifest.json` is refused.
//! - Profiles are read one file at a time, each through the shared size guard, so peak memory is bounded by
//!   the matrix, not the directory total.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::guard::{self, GuardError, JsonError};
use crate::test_effectiveness::matrix::{
    AttributionChoice, CoverageAttribution, CoverageBuilder, CoverageDimension, CoverageUnit,
    CoverageUnitKind, MissingProvenance, Provenance, TestId,
};

pub const MANIFEST_FILE: &str = "manifest.json";
pub const PARTIAL_MANIFEST_FILE: &str = "manifest.partial.json";
const PROFILE_EXTENSION: &str = "out";
const NAME_SEPARATOR: &str = "__";

/// Parsed directory: the coverage dimension plus the tests the manifest records as failing, so the
/// exclusion model can mark them `Failing`.
#[derive(Debug, Clone)]
pub struct GoCoverRead {
    pub dimension: CoverageDimension,
    pub failing: Vec<TestId>,
}

#[derive(Debug)]
pub enum GoCoverError {
    Guard(GuardError),
    Json {
        file: PathBuf,
        error: JsonError,
    },
    /// `partial` is true when `manifest.partial.json` is present (an interrupted run).
    ManifestMissing {
        dir: PathBuf,
        partial: bool,
    },
    ManifestInvalid(String),
    InvalidProfileName(PathBuf),
    Malformed {
        file: PathBuf,
        line: usize,
        why: &'static str,
    },
    Provenance(MissingProvenance),
    Io {
        dir: PathBuf,
        source: std::io::Error,
    },
}

impl fmt::Display for GoCoverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Guard(e) => e.fmt(f),
            Self::Json { file, error } => write!(f, "{}: {error}", file.display()),
            Self::ManifestMissing { dir, partial } => {
                let shown = dir.display().to_string();
                let shown = shown.trim_end_matches('/');
                if *partial {
                    write!(
                        f,
                        "error: {shown}/ has manifest.partial.json and no manifest.json (interrupted run); \
                         re-run the recipe with --resume"
                    )
                } else {
                    write!(f, "error: {shown}/ has no manifest.json (partial run?)")
                }
            }
            Self::ManifestInvalid(why) => write!(f, "{MANIFEST_FILE}: {why}"),
            Self::InvalidProfileName(p) => write!(
                f,
                "{}: profile name is not <pkg-encoded>__<TestName>.out",
                p.display()
            ),
            Self::Malformed { file, line, why } => {
                write!(
                    f,
                    "{}:{line}: malformed cover profile ({why})",
                    file.display()
                )
            }
            Self::Provenance(e) => e.fmt(f),
            Self::Io { dir, source } => write!(f, "cannot read {}: {source}", dir.display()),
        }
    }
}

impl std::error::Error for GoCoverError {}

/// Reverses the recipe's package encoding; `None` on a truncated or non-hex escape.
fn decode_package(encoded: &str) -> Option<String> {
    let bytes = encoded.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = encoded.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn test_id_from_file_name(path: &Path) -> Result<TestId, GoCoverError> {
    let invalid = || GoCoverError::InvalidProfileName(path.to_path_buf());
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(invalid)?;
    let (pkg, test) = stem.split_once(NAME_SEPARATOR).ok_or_else(invalid)?;
    if pkg.is_empty() || test.is_empty() {
        return Err(invalid());
    }
    Ok(TestId::new(decode_package(pkg).ok_or_else(invalid)?, test))
}

/// Bounds per-line expansion so a hostile `1.1,4000000000.1` row cannot exhaust memory.
const MAX_BLOCK_LINES: u32 = 100_000;

/// Adds one line per covered line of each block to `lines`; `file -> line -> hits`. Blocks with count 0 are ignored.
fn parse_profile(
    file: &Path,
    text: &str,
    lines: &mut BTreeMap<String, BTreeMap<u32, u32>>,
) -> Result<(), GoCoverError> {
    let bad = |line: usize, why| GoCoverError::Malformed {
        file: file.to_path_buf(),
        line,
        why,
    };
    let mut rows = text.lines().enumerate();
    match rows.next() {
        Some((_, first)) if first.starts_with("mode:") => {}
        _ => return Err(bad(1, "first line is not `mode: ...`")),
    }
    for (idx, row) in rows {
        let n = idx + 1;
        if row.trim().is_empty() {
            continue;
        }
        let mut parts = row.rsplitn(3, ' ');
        let count: u64 = parts
            .next()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| bad(n, "count is not a number"))?;
        parts
            .next()
            .ok_or_else(|| bad(n, "missing statement count"))?;
        let span = parts.next().ok_or_else(|| bad(n, "missing block span"))?;
        let (name, range) = span
            .rsplit_once(':')
            .ok_or_else(|| bad(n, "block has no `file:` prefix"))?;
        let (start, end) = range
            .split_once(',')
            .ok_or_else(|| bad(n, "block range has no comma"))?;
        let line_of = |pos: &str| -> Option<u32> { pos.split('.').next()?.parse().ok() };
        let (first, last) = line_of(start)
            .zip(line_of(end))
            .filter(|(a, b)| a <= b)
            .ok_or_else(|| bad(n, "block range is not `line.col,line.col`"))?;
        if last - first > MAX_BLOCK_LINES {
            return Err(bad(n, "block spans an implausible number of lines"));
        }
        if count == 0 {
            continue;
        }
        let hits = u32::try_from(count).unwrap_or(u32::MAX);
        let per_file = lines.entry(name.to_string()).or_default();
        for line in first..=last {
            let slot = per_file.entry(line).or_insert(0);
            *slot = slot.saturating_add(hits);
        }
    }
    Ok(())
}

struct Manifest {
    attribution: CoverageAttribution,
    go_version: Option<String>,
    commit: Option<String>,
    failing: Vec<TestId>,
}

fn string_field(obj: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    obj.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn parse_manifest(path: &Path, max_input_bytes: u64) -> Result<Manifest, GoCoverError> {
    let bytes = guard::read_bounded(path, max_input_bytes).map_err(GoCoverError::Guard)?;
    let value: Value = guard::parse_json_bounded(&bytes).map_err(|error| GoCoverError::Json {
        file: path.to_path_buf(),
        error,
    })?;
    let Value::Object(obj) = value else {
        return Err(GoCoverError::ManifestInvalid("not a JSON object".into()));
    };
    let attribution = match string_field(&obj, "mode").as_deref() {
        Some("isolated") => CoverageAttribution::Isolated,
        Some("in_order") => CoverageAttribution::InOrder,
        Some(other) => {
            return Err(GoCoverError::ManifestInvalid(format!(
                "mode `{other}` is not `isolated` or `in_order`"
            )));
        }
        None => {
            return Err(GoCoverError::ManifestInvalid(
                "missing `mode` (`isolated` or `in_order`); attribution has no default".into(),
            ));
        }
    };
    let failing = obj
        .get("failing")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let entry = entry.as_object()?;
            Some(TestId::new(
                string_field(entry, "package")?,
                string_field(entry, "test")?,
            ))
        })
        .collect();
    Ok(Manifest {
        attribution,
        go_version: string_field(&obj, "go_version"),
        commit: string_field(&obj, "commit"),
        failing,
    })
}

/// Parses a recipe-produced profile directory. Each profile and the manifest pass the size guard first.
pub fn parse_dir(dir: &Path, max_input_bytes: u64) -> Result<GoCoverRead, GoCoverError> {
    let manifest_path = dir.join(MANIFEST_FILE);
    if !manifest_path.is_file() {
        return Err(GoCoverError::ManifestMissing {
            dir: dir.to_path_buf(),
            partial: dir.join(PARTIAL_MANIFEST_FILE).is_file(),
        });
    }
    let manifest = parse_manifest(&manifest_path, max_input_bytes)?;

    let io_err = |source| GoCoverError::Io {
        dir: dir.to_path_buf(),
        source,
    };
    let mut profiles: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(io_err)?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == PROFILE_EXTENSION))
        .collect();
    profiles.sort();

    let provenance = Provenance::new("go", manifest.go_version.as_deref())
        .with_commit(manifest.commit.as_deref());
    let choice = AttributionChoice {
        attribution: manifest.attribution,
        assumed: false,
    };
    let mut builder =
        CoverageBuilder::coverage(provenance, choice).unit_kind(CoverageUnitKind::Line);
    for id in &manifest.failing {
        builder.add_test(id);
    }
    for path in profiles {
        let id = test_id_from_file_name(&path)?;
        builder.add_test(&id);
        if manifest.failing.contains(&id) {
            continue;
        }
        let bytes = guard::read_bounded(&path, max_input_bytes).map_err(GoCoverError::Guard)?;
        let text = String::from_utf8_lossy(&bytes);
        let mut lines = BTreeMap::new();
        parse_profile(&path, &text, &mut lines)?;
        for (file, per_line) in lines {
            for (line, hits) in per_line {
                builder.record(&id, CoverageUnit::new(file.clone(), line), hits);
            }
        }
    }
    let dimension = builder.build().map_err(GoCoverError::Provenance)?;
    Ok(GoCoverRead {
        dimension,
        failing: manifest.failing,
    })
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;
    use crate::test_effectiveness::analysis::unique_coverage;
    use crate::test_effectiveness::matrix::{Tier, WARN_INPUT_COMMIT_UNKNOWN};

    const CAP: u64 = 1 << 20;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("testdata/test-effectiveness/gocover")
            .join(name)
            .join("profiles")
    }

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("kibitzer-gocover-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    const GOOD_MANIFEST: &str =
        r#"{"go_version":"go1.26.4","commit":"abc","mode":"isolated","failing":[]}"#;

    fn lines_of(d: &CoverageDimension, test: &TestId) -> Vec<(String, u32)> {
        let ord = d.ordinal_of(test).expect("test present");
        d.index
            .iter()
            .filter(|(_, p)| p.tests.contains(&ord))
            .map(|(u, _)| (u.file.clone(), u.line))
            .collect()
    }

    #[test]
    fn parse_profile_should_Reject_When_BlockSpansImplausibleLineCount() {
        let mut lines = BTreeMap::new();
        let text = "mode: set\na.go:1.1,4000000000.1 1 1\n";
        let err = parse_profile(Path::new("p.out"), text, &mut lines).unwrap_err();
        assert!(matches!(err, GoCoverError::Malformed { line: 2, .. }));
        assert!(lines.is_empty());
    }

    #[test]
    fn parse_should_MatchProfileBlocksAndSetIsolated_When_TwoProfileFixture() {
        let read = parse_dir(&fixture("twotest"), CAP).unwrap();
        let d = &read.dimension;
        let file = "example.com/tec/classify.go";
        let pos = TestId::new("example.com/tec", "TestPos");
        let neg = TestId::new("example.com/tec", "TestNeg");
        let covered = |lines: &[u32]| -> Vec<(String, u32)> {
            lines.iter().map(|&l| (file.to_string(), l)).collect()
        };
        assert_eq!(lines_of(d, &neg), covered(&[4, 5, 6, 7]));
        assert_eq!(lines_of(d, &pos), covered(&[4, 5, 8, 11]));
        assert_eq!(d.meta, CoverageAttribution::Isolated);
        assert_eq!(d.provenance.attribution.map(|a| a.assumed), Some(false));
        assert_eq!(d.cap, None);
        assert!(d.warnings.is_empty());
        let unique = unique_coverage(d);
        assert_eq!(unique[&neg].len(), 2);
        assert_eq!(unique[&pos].len(), 2);
    }

    #[test]
    fn parse_should_ReturnTwoDistinctTestIds_When_TwoPackagesEachHaveTestParse() {
        let read = parse_dir(&fixture("twopkg"), CAP).unwrap();
        assert_eq!(
            read.dimension.tests,
            vec![
                TestId::new("example.com/two/a", "TestParse"),
                TestId::new("example.com/two/b", "TestParse"),
            ]
        );
    }

    #[test]
    fn parse_should_Fail_When_ManifestMissing() {
        let dir = scratch("nomanifest");
        std::fs::write(dir.join("p__TestA.out"), "mode: set\n").unwrap();
        let err = parse_dir(&dir, CAP).unwrap_err();
        assert!(matches!(
            err,
            GoCoverError::ManifestMissing { partial: false, .. }
        ));
        assert!(
            err.to_string()
                .ends_with("/ has no manifest.json (partial run?)"),
            "{err}"
        );
        assert!(err.to_string().starts_with("error: "));
    }

    #[test]
    fn parse_should_NameThePartialMarker_When_OnlyPartialManifestExists() {
        let dir = scratch("partial");
        std::fs::write(dir.join(PARTIAL_MANIFEST_FILE), "{}").unwrap();
        let err = parse_dir(&dir, CAP).unwrap_err();
        assert!(matches!(
            err,
            GoCoverError::ManifestMissing { partial: true, .. }
        ));
        assert!(err.to_string().contains("has manifest.partial.json and no manifest.json (interrupted run); re-run the recipe with --resume"));
    }

    #[test]
    fn attribution_should_ComeFromHandWrittenSidecarOrManifest_When_NoWrapperScriptExists() {
        let dir = scratch("inorder");
        std::fs::write(
            dir.join(MANIFEST_FILE),
            r#"{"go_version":"go1.26.4","commit":"abc","mode":"in_order"}"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("p%2Fq__TestA.out"),
            "mode: count\np/q/a.go:3.1,4.2 1 7\np/q/a.go:9.1,9.5 1 0\n",
        )
        .unwrap();
        let d = parse_dir(&dir, CAP).unwrap().dimension;
        assert_eq!(d.meta, CoverageAttribution::InOrder);
        assert_eq!(d.provenance.attribution.map(|a| a.assumed), Some(false));
        assert!(d.warnings.is_empty(), "carried attribution is not assumed");
        assert_eq!(d.cap, Some(Tier::Medium));
        let a = TestId::new("p/q", "TestA");
        assert_eq!(
            lines_of(&d, &a),
            vec![("p/q/a.go".to_string(), 3), ("p/q/a.go".to_string(), 4)]
        );
        let posting = &d.index[&CoverageUnit::new("p/q/a.go", 3)];
        assert_eq!(posting.hits, vec![7]);
    }

    #[test]
    fn parse_should_Fail_When_ModeMissingOrUnknown() {
        for (name, body) in [
            ("nomode", r#"{"go_version":"go1","commit":"c"}"#),
            (
                "badmode",
                r#"{"go_version":"go1","commit":"c","mode":"parallel"}"#,
            ),
        ] {
            let dir = scratch(name);
            std::fs::write(dir.join(MANIFEST_FILE), body).unwrap();
            let err = parse_dir(&dir, CAP).unwrap_err();
            assert!(matches!(err, GoCoverError::ManifestInvalid(_)), "{err}");
        }
    }

    #[test]
    fn parse_should_Fail_When_GoVersionMissing() {
        let dir = scratch("nover");
        std::fs::write(
            dir.join(MANIFEST_FILE),
            r#"{"commit":"c","mode":"isolated"}"#,
        )
        .unwrap();
        let err = parse_dir(&dir, CAP).unwrap_err();
        assert!(matches!(err, GoCoverError::Provenance(_)), "{err}");
    }

    #[test]
    fn parse_should_CapAtLowWithWarning_When_CommitMissing() {
        let dir = scratch("nocommit");
        std::fs::write(
            dir.join(MANIFEST_FILE),
            r#"{"go_version":"go1","commit":null,"mode":"isolated"}"#,
        )
        .unwrap();
        let d = parse_dir(&dir, CAP).unwrap().dimension;
        assert_eq!(d.cap, Some(Tier::Low));
        assert!(d.warnings.iter().any(|w| w.id == WARN_INPUT_COMMIT_UNKNOWN));
    }

    #[test]
    fn parse_should_ExposeFailingTestsAndSkipTheirProfiles_When_ManifestRecordsFailing() {
        let dir = scratch("failing");
        std::fs::write(
            dir.join(MANIFEST_FILE),
            r#"{"go_version":"go1","commit":"c","mode":"isolated",
                "failing":[{"package":"p","test":"TestBad"}]}"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("p__TestBad.out"),
            "mode: set\np/a.go:1.1,2.1 1 1\n",
        )
        .unwrap();
        std::fs::write(dir.join("p__TestOk.out"), "mode: set\np/a.go:5.1,5.9 1 1\n").unwrap();
        let read = parse_dir(&dir, CAP).unwrap();
        let bad = TestId::new("p", "TestBad");
        assert_eq!(read.failing, vec![bad.clone()]);
        assert!(read.dimension.ordinal_of(&bad).is_some());
        assert!(lines_of(&read.dimension, &bad).is_empty());
        assert_eq!(
            lines_of(&read.dimension, &TestId::new("p", "TestOk")),
            vec![("p/a.go".to_string(), 5)]
        );
    }

    #[test]
    fn parse_should_DecodeEncodedPackageAndKeepUnderscoresInTestName_When_NameHasEscapes() {
        let id = test_id_from_file_name(Path::new("a%2Fb%5Fc%25d__Test_x__y.out")).unwrap();
        assert_eq!(id, TestId::new("a/b_c%d", "Test_x__y"));
        assert!(test_id_from_file_name(Path::new("nosep.out")).is_err());
        assert!(test_id_from_file_name(Path::new("a%2__T.out")).is_err());
    }

    #[test]
    fn parse_should_FailWithoutPanic_When_ProfileIsMalformed() {
        for (i, body) in [
            "not a mode line\n",
            "mode: set\np/a.go:1.1,2.1 1\n",
            "mode: set\np/a.go:9.1,2.1 1 1\n",
            "mode: set\np/a.go:1.1,2.1 1 x\n",
        ]
        .iter()
        .enumerate()
        {
            let dir = scratch(&format!("malformed{i}"));
            std::fs::write(dir.join(MANIFEST_FILE), GOOD_MANIFEST).unwrap();
            std::fs::write(dir.join("p__TestA.out"), body).unwrap();
            let err = parse_dir(&dir, CAP).unwrap_err();
            assert!(
                matches!(err, GoCoverError::Malformed { .. }),
                "{body:?}: {err}"
            );
        }
    }

    #[test]
    fn parse_should_CallGuardBeforeEachProfileAndManifest_When_FileOverCap() {
        let dir = scratch("guard");
        std::fs::write(dir.join(MANIFEST_FILE), GOOD_MANIFEST).unwrap();
        std::fs::write(dir.join("p__TestA.out"), "mode: set\n".repeat(50)).unwrap();
        let err = parse_dir(&dir, 100).unwrap_err();
        match err {
            GoCoverError::Guard(GuardError::TooLarge(e)) => {
                assert!(e.file.ends_with("p__TestA.out"), "{e}");
            }
            other => panic!("expected profile guard error, got {other}"),
        }
        let err = parse_dir(&dir, 10).unwrap_err();
        match err {
            GoCoverError::Guard(GuardError::TooLarge(e)) => {
                assert!(e.file.ends_with(MANIFEST_FILE), "{e}");
            }
            other => panic!("expected manifest guard error, got {other}"),
        }
    }
}
