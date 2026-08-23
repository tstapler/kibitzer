//! Story 2.1.3: the `DeclarationChecker` trait and its registry — the third checker
//! kind ADR-001 calls for, alongside `checker::Checker` (per-file AST checks) and
//! `architecture_checks::ArchitectureChecker` (whole-repo import-graph checks). This
//! registry is empty until Story 2.2.1 adds `ContentChecker`; Phase 3 adds
//! `NamingChecker` alongside it.

use crate::architecture_checks::ArchFinding;
use crate::config::ArchitectureConfig;
use crate::declarations::DeclarationGraph;

/// Mirrors [`crate::architecture_checks::ArchitectureChecker`]'s shape, but running
/// against a whole-repo [`DeclarationGraph`] instead of an [`crate::import_graph::ImportGraph`]
/// — `content-rules`/`naming-rules` reason about individual declarations
/// (struct/class/interface/enum/function), not import edges, so they need their own
/// input type. Reuses `ArchFinding` unmodified as the output shape, same as
/// `ArchitectureChecker` (architecture.md §4) — no new finding shape for this checker
/// kind.
pub trait DeclarationChecker {
    #[allow(dead_code)]
    fn name(&self) -> &str;
    #[allow(dead_code)]
    fn check(&self, graph: &DeclarationGraph, config: &ArchitectureConfig) -> Vec<ArchFinding>;
}

/// Empty until Story 2.2.1 adds `ContentChecker` — `content-rules`/`naming-rules` both
/// resolve through `crate::check::lookup_any_architecture_checker`'s dual-registry
/// dispatch, which falls back to this registry once `architecture_checks::registry()`
/// misses.
pub fn registry() -> Vec<Box<dyn DeclarationChecker>> {
    vec![]
}

pub fn lookup(name: &str) -> Option<Box<dyn DeclarationChecker>> {
    registry().into_iter().find(|c| c.name() == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_returns_none_before_any_checker_registered() {
        assert!(lookup("content-rules").is_none());
        assert!(lookup("naming-rules").is_none());
        assert!(lookup("anything").is_none());
    }
}
