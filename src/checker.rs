use std::path::Path;

use anyhow::Result;

#[derive(Debug, PartialEq, Eq)]
pub struct Finding {
    pub line: usize,
    pub message: String,
}

/// A native, in-process check pluggable into kibitzer's CLI dispatch.
pub trait Checker {
    fn check_file(&self, path: &Path) -> Result<Vec<Finding>>;
}

