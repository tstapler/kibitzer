//! Phase-2 placeholder (Epic 1.2, Task 1.2.1a). `AnyArchitectureChecker`'s `Declaration`
//! variant and `lookup_any_architecture_checker` (in `src/check.rs`) need a real module
//! path to compile against ahead of Phase 2's actual declaration-based checkers
//! (`content-rules`/`naming-rules`, built on `DeclarationGraph` — see plan.md's Phase 2).
//!
//! This stub is intentionally minimal: [`lookup`] always returns `None`, so every check
//! name resolves through `architecture_checks::lookup` alone until Phase 2 (Task 2.1.3b)
//! replaces this file with the real `DeclarationChecker` registry. Do not add real
//! checkers here as part of Epic 1.2 — that's out of scope for this epic.

/// Mirrors [`crate::architecture_checks::ArchitectureChecker`]'s shape at the level
/// `AnyArchitectureChecker` needs today (a name to resolve dispatch by). Phase 2 will
/// extend this with a `check(&DeclarationGraph, &ArchitectureConfig) -> Vec<ArchFinding>`
/// method once `DeclarationGraph` exists.
pub trait DeclarationChecker {
    #[allow(dead_code)]
    fn name(&self) -> &str;
}

/// Always returns `None` — no declaration-based checkers are registered yet. Replaced by
/// Phase 2's real registry (Task 2.1.3b).
pub fn lookup(_name: &str) -> Option<Box<dyn DeclarationChecker>> {
    None
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
