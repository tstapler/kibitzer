//! `TestCodeProfile` seam: one place that knows what a test file, test function and assertion are per language
//! (Task 0.2.1a).
// Consumed by the test-quality checkers and the report (later tasks); reachable only from tests until then.
#![allow(dead_code)]

/// Heuristic tag for a test; integration and snapshot findings are downgraded (pitfalls P8, P10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TestKind {
    Unit,
    Integration,
    Snapshot,
    Fuzz,
    Example,
}
