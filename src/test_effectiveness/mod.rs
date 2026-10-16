//! Ingest-only test-effectiveness analysis (ADR-001, ADR-002): adapters normalise third-party coverage and
//! mutation reports into a `TestMatrix`; pure analyses read it. Nothing here runs tests.
// Hidden `kibitzer tests` wiring (Task 2.3.1a) lands later; until then the items are reachable only from tests.
#![allow(dead_code)]

pub mod adapters;
pub mod analysis;
pub mod cli;
pub mod exclusions;
pub mod locate;
pub mod matrix;
pub mod report;
#[cfg(test)]
mod scale_tests;
