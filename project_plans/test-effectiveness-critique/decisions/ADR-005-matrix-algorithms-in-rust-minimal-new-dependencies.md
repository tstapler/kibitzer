# ADR-005: Matrix algorithms are hand-written in Rust over a normalized `TestMatrix`; one new dependency (an XML reader) only for the PIT adapter

**Status**: Accepted (plan phase) | **Date**: 2026-10-11

## Context
`research/build-vs-buy.md` section 3 classifies unique-coverage, greedy set cover, and dynamic mutant subsumption as safe to hand-roll (pure functions over a matrix, property-testable), and mutation operators and instrumentation as reckless. `Cargo.toml` has `serde_json` and `regex` but no XML parser or bitset crate.

## Decision
- Implement set operations over `Vec<u64>` bitsets in-tree (no `roaring`); revisit only if the Phase 3 scale runs show a measured bottleneck at 10^5 tests.
- Greedy set cover uses a deterministic tie-break (test id order) so findings are reproducible.
- Dynamic subsumption is reported as relative to the supplied suite only; the adapter must reject or warn when a kill report lists only the first killer.
- Add `quick-xml` solely for the PIT `mutations.xml` adapter (story 2.2.5), behind that story so that the dependency lands with its only consumer.
- Do not hand-roll mutation operators, coverage instrumentation, or ML predictive mutation testing (Out of Scope).

## Alternatives rejected
- Regex-parse PIT XML: rejected; fragile on attribute order and escaping.
- `roaring` bitmaps up front: rejected; unmeasured need.

## Amendment (plan repair, 2026-10-11)
The `Vec<u64>` test-by-unit bitset decision is replaced. At 10^5 tests x ~10^6 units a dense grid is tens of GB. The matrix is an inverted index (per unit, a sorted `u32` posting list of test ordinals, plus hit count); unique coverage reads posting lists of length 1. No new dependency. `roaring` is admitted only if Story 2.1.5's scale test (100k tests, 1M units, 5 x 10^7 entries; proposed bound <= 1.5 GB, <= 120 s) fails. The "rejected: roaring up front" alternative stands.

## Amendment (triad repair, iteration 1, 2026-10-11)
Parser choice is a security surface because adapters read third-party files. `serde_json` is used with its default recursion limit (never disabled) and `quick-xml` for PIT without entity expansion or external fetches; both sit behind the size cap of Story 2.2.7 and the malformed-input corpus there. Parse memory is measured per format in Story 2.1.5 (Task 2.1.5b); an adapter that cannot stay within the bound must stream rather than build a full document. This adds tests, not dependencies.

## Amendment (triad repair, iteration 2, 2026-10-11): no new dev-dependencies
`Cargo.toml` `[dev-dependencies]` holds only `kibitzer-stub-plugin`, and the existing integration tests spawn the binary with `env!("CARGO_BIN_EXE_kibitzer")` and `std::process::Command` (`tests/hook_contract.rs`). The test plan therefore uses the std-only pattern and adds no `assert_cmd`, `insta`, `proptest`, `tempfile` or `rand`: (1) integration tests spawn the binary the same way, in a temp directory made from `std::env::temp_dir()` plus the process id; (2) golden outputs are compared with expected files checked in under `testdata/test-effectiveness/golden/`, with a regenerate switch in a small in-tree helper; (3) property tests and the scale-fixture generator use a hand-rolled seeded PRNG (xorshift64, fixed seeds and documented case counts) in a `#[cfg(test)]` support module; (4) the malformed-input "fuzz" run is a seeded mutate-the-fixtures loop marked `#[ignore]`, not `cargo fuzz`. Where this ADR's earlier text or `validation.md` names `proptest` or `insta` it means this pattern. If the owner prefers the crates, that is a new decision recorded here before Story 2.1.1 starts; `quick-xml` stays the only new normal dependency (guarded by the `cargo tree` check).
