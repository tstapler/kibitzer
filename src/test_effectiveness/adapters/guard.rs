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

/// Parses JSON under serde_json's default recursion limit (128); deeper input is `NestingTooDeep`.
pub fn parse_json_bounded<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, JsonError> {
    serde_json::from_slice(bytes).map_err(|e| {
        if e.to_string().contains("recursion limit exceeded") {
            JsonError::NestingTooDeep
        } else {
            JsonError::Invalid(e.to_string())
        }
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
