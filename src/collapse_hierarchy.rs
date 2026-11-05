//! Fowler's **Collapse Hierarchy**: a subtype that adds almost nothing to its direct
//! supertype. Fires on a resolved `Extends` edge when fewer than
//! [`COLLAPSE_HIERARCHY_UNIQUE_THRESHOLD`] of the subtype's methods are absent (by name) from
//! the supertype. Names only, not signatures; direct parent only, not transitive.

use std::collections::HashSet;

use crate::arch_model::ArchModel;
use crate::architecture_checks::{ArchFinding, ArchModelChecker};
use crate::config::ArchitectureConfig;
use crate::hierarchy_checks::{methods_of, resolved_extends, type_index};

const COLLAPSE_HIERARCHY_UNIQUE_THRESHOLD: f64 = 0.20;
/// A subtype with 1-2 methods that all override its parent is the template-method pattern
/// (backtest: 150+ such hits on cassandra), not a hollow hierarchy.
const MIN_SUBTYPE_METHODS: usize = 3;

pub struct CollapseHierarchyChecker;

impl ArchModelChecker for CollapseHierarchyChecker {
    fn name(&self) -> &str {
        "collapse-hierarchy"
    }

    fn check(&self, model: &ArchModel, _config: &ArchitectureConfig) -> Vec<ArchFinding> {
        let types = type_index(model);
        let mut findings = Vec::new();
        for (sub_id, super_id) in resolved_extends(model, &types) {
            let (sub_pkg, sub) = types[sub_id];
            let (super_pkg, sup) = types[super_id];
            let sub_methods = methods_of(model, sub_pkg, &sub.name);
            // Same name in two files = build-tag variants of one type, not a hierarchy.
            if sub.name == sup.name || sub_methods.len() < MIN_SUBTYPE_METHODS {
                continue;
            }
            let inherited: HashSet<&str> = methods_of(model, super_pkg, &sup.name)
                .iter()
                .map(|m| m.name.as_str())
                .collect();
            let unique = sub_methods
                .iter()
                .filter(|m| !inherited.contains(m.name.as_str()))
                .count();
            let ratio = unique as f64 / sub_methods.len() as f64;
            if ratio >= COLLAPSE_HIERARCHY_UNIQUE_THRESHOLD {
                continue;
            }
            let edge = model
                .type_edges
                .iter()
                .find(|e| e.from == sub_id && e.to == super_id);
            findings.push(ArchFinding {
                file: edge
                    .map(|e| e.file.clone())
                    .or_else(|| Some(sub.file.clone())),
                line: edge.map(|e| e.line).or(Some(sub.line)),
                message: format!(
                    "[collapse-hierarchy] {sub_id} extends {super_id} but only {unique}/{} \
                     ({:.0}%) of its methods are unique — consider collapsing the hierarchy \
                     (Fowler, Collapse Hierarchy)",
                    sub_methods.len(),
                    ratio * 100.0
                ),
                severity_override: None,
                operation: None,
            });
        }
        findings.sort_by(|a, b| (&a.file, a.line).cmp(&(&b.file, b.line)));
        findings
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch_model::SymbolKind;
    use crate::hierarchy_checks::fixtures::{edge, model, sym};

    fn run(m: &ArchModel) -> Vec<ArchFinding> {
        CollapseHierarchyChecker.check(m, &ArchitectureConfig::default())
    }

    /// `S extends P`; P has methods `Shared0..4`; S overrides `shared` of them and adds `unique`.
    fn hierarchy(shared: usize, unique: usize) -> ArchModel {
        let mut syms = vec![
            sym("pkg", "P", SymbolKind::Type, None),
            sym("pkg", "S", SymbolKind::Type, None),
        ];
        for i in 0..5 {
            syms.push(sym(
                "pkg",
                &format!("Shared{i}"),
                SymbolKind::Method,
                Some("P"),
            ));
        }
        for i in 0..shared {
            syms.push(sym(
                "pkg",
                &format!("Shared{i}"),
                SymbolKind::Method,
                Some("S"),
            ));
        }
        for i in 0..unique {
            syms.push(sym(
                "pkg",
                &format!("Own{i}"),
                SymbolKind::Method,
                Some("S"),
            ));
        }
        model(syms, vec![edge("pkg::S", "pkg::P", true)])
    }

    #[test]
    fn empty_model_yields_nothing() {
        assert!(run(&model(vec![], vec![])).is_empty());
    }

    #[test]
    fn flags_subtype_with_no_unique_methods() {
        let found = run(&hierarchy(3, 0));
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(
            found[0].message.contains("0/3 (0%)"),
            "{}",
            found[0].message
        );
        assert_eq!(found[0].line, Some(3), "points at the Extends edge");
    }

    #[test]
    fn ignores_subtype_with_substantial_unique_methods() {
        assert!(run(&hierarchy(1, 3)).is_empty());
    }

    #[test]
    fn boundary_exactly_twenty_percent_does_not_fire_just_below_does() {
        assert!(run(&hierarchy(4, 1)).is_empty(), "1/5 = 20%");
        assert_eq!(run(&hierarchy(5, 1)).len(), 1, "1/6 < 20%");
    }

    #[test]
    fn ignores_unresolved_edges() {
        let mut m = hierarchy(3, 0);
        m.type_edges = vec![edge("pkg::S", "external.Base", false)];
        assert!(run(&m).is_empty());
    }

    #[test]
    fn ignores_subtype_with_no_methods() {
        assert!(run(&hierarchy(0, 0)).is_empty());
    }

    #[test]
    fn boundary_subtype_below_minimum_method_count_is_skipped() {
        assert!(run(&hierarchy(MIN_SUBTYPE_METHODS - 1, 0)).is_empty());
        assert_eq!(run(&hierarchy(MIN_SUBTYPE_METHODS, 0)).len(), 1);
    }
}
