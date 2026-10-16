//! Hidden experiment-input export (Task 2.3.1h).

use std::io;
use std::path::{Path, PathBuf};

use super::{Loaded, Report};

pub fn write(
    _: &Path,
    _: &Report,
    _: &Loaded,
    _: &[(&'static str, PathBuf)],
) -> io::Result<()> {
    Ok(())
}
