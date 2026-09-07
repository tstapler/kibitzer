//! Empty on purpose. `kibitzer`'s root `Cargo.toml` depends on this crate under
//! `[dev-dependencies]` solely so the entry parses at all — a plain path dependency on a
//! bin-only sibling package fails with "ignoring invalid dependency ... which is missing a
//! lib target" (confirmed against cargo 1.98), since a dependency edge requires something
//! to link against. This does *not* expose `CARGO_BIN_EXE_kibitzer-stub-plugin(-second)` to
//! `kibitzer`'s integration tests — that needs a nightly-only artifact dependency
//! (`artifact = "bin"`, `-Z bindeps`); see `tests/plugin_contract.rs`'s
//! `sibling_workspace_binary` for how those tests actually locate the compiled binaries at
//! runtime instead. No code here is imported by `kibitzer` itself.
