//! Test-only helpers shared across `#[cfg(test)]` modules that shell out to the compiled
//! `kibitzer` binary (e.g. `arch_export.rs`, `arch_diagram.rs`). Kept as its own module
//! (rather than duplicated per file) since these two functions were previously
//! byte-for-byte identical in both call sites.

#![cfg(test)]

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tree_sitter::Tree;

use crate::checker::{CheckContext, Checker, Finding};

/// Parses `src` as Go with tree-sitter — half of the setup every Go native checker's own
/// tests repeat before building a `CheckContext` and calling `checker.check(...)` (see
/// [`check_go_source`] for the fully-collapsed form; this half stays separate for the one
/// caller — `rules.rs`'s `check_source` — whose tail diverges into `walk_declarations`
/// instead of a `Checker::check` call).
pub(crate) fn parse_go(src: &str) -> Result<Tree> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_go::LANGUAGE.into())
        .context("loading tree-sitter-go grammar")?;
    parser
        .parse(src, None)
        .context("parsing Go source with tree-sitter")
}

/// Parses `src` as Go and runs `checker` against it — the full boilerplate every Go
/// native checker's own tests repeat (parse, build a `CheckContext`, call
/// `checker.check(...)`) collapsed into one call, since the `Tree`'s lifetime only
/// needs to outlive the `CheckContext` borrowing it, both of which stay inside this
/// function. Extracted once a corpus backtest sweep flagged the un-collapsed version as
/// a genuine repeated block across `go_blank_imports.rs`, `primitive_obsession.rs`,
/// `go_ignored_error.rs`, and `go_error_context_tests.rs`.
pub(crate) fn check_go_source(checker: &dyn Checker, src: &str) -> Result<Vec<Finding>> {
    let tree = parse_go(src)?;
    let ctx = CheckContext {
        source: src,
        tree: Some(&tree),
    };
    checker.check(Path::new("<source>"), &ctx)
}

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
