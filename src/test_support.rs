//! Test-only helpers shared across `#[cfg(test)]` modules that shell out to the compiled
//! `kibitzer` binary (e.g. `arch_export.rs`, `arch_diagram.rs`). Kept as its own module
//! (rather than duplicated per file) since these two functions were previously
//! byte-for-byte identical in both call sites.

#![cfg(test)]

use std::path::PathBuf;

/// The compiled `kibitzer` binary's path. Cargo only sets `CARGO_BIN_EXE_<name>` for
/// integration tests/benches (this crate has no `tests/` directory — everything is inline
/// `#[cfg(test)]`, per `validation.md`'s Test Stack section), so it's not available here;
/// `cargo test` still builds the plain `kibitzer` binary target as part of the same build,
/// so it's resolved by convention instead.
pub(crate) fn kibitzer_bin_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        })
        .join("kibitzer")
}

pub(crate) fn run_kibitzer(args: &[&str]) -> std::process::Output {
    std::process::Command::new(kibitzer_bin_path())
        .args(args)
        .output()
        .expect("kibitzer binary runs (run `cargo build` first if this fails)")
}
