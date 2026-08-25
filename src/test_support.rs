//! Test-only helpers shared across `#[cfg(test)]` modules that shell out to the compiled
//! `kibitzer` binary (e.g. `arch_export.rs`, `arch_diagram.rs`). Kept as its own module
//! (rather than duplicated per file) since these two functions were previously
//! byte-for-byte identical in both call sites.

#![cfg(test)]

use std::path::PathBuf;

/// The compiled `kibitzer` binary's path. Cargo only sets `CARGO_BIN_EXE_<name>` for
/// integration tests/benches (this crate has no `tests/` directory — everything is inline
/// `#[cfg(test)]`, per `validation.md`'s Test Stack section), so it's not available here.
/// `cargo test` alone does NOT build this plain binary target (confirmed empirically — it
/// only builds a separate test-harness binary under `target/debug/deps/`), so CI runs an
/// explicit `cargo build` before `cargo test` (see `.github/workflows/ci.yml`). A local
/// `cargo test` run needs the same: run `cargo build` first if these tests fail to find
/// the binary.
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
