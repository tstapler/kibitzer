//! Fowler's **Remove Subclass**: a leaf subtype whose every method is a bare constant
//! return, and which carries no state of its own, could be a field value on its supertype.
//! Input is `SymbolNode::is_constant_return` (Go/TS only, so Java/Kotlin never fire).
//! "No additional fields" is a proxy — a field of the subtype that some method reads
//! (`FieldAccessEdge`) disqualifies it; a declared-but-unread field is a known false-negative.

use std::collections::HashSet;

use crate::arch_model::ArchModel;
use crate::architecture_checks::{ArchFinding, ArchModelChecker};
use crate::config::ArchitectureConfig;
use crate::hierarchy_checks::{
    is_ignorable_file, methods_of, resolved_extends, supertype_ids, type_index,
};

pub struct RemoveSubclassChecker;

impl ArchModelChecker for RemoveSubclassChecker {
    fn name(&self) -> &str {
        "remove-subclass"
    }

    /// Leaf subtypes are frequently unexported.
    fn needs_private_symbols(&self) -> bool {
        true
    }

    fn check(&self, model: &ArchModel, _config: &ArchitectureConfig) -> Vec<ArchFinding> {
        let types = type_index(model);
        let pairs = resolved_extends(model, &types);
        let parents = supertype_ids(&pairs);
        // Owner type id of every accessed field (`pkg::T.field` -> `pkg::T`), built once.
        let accessed_owners: HashSet<&str> = model
            .field_accesses
            .iter()
            .filter_map(|a| a.to.rsplit_once('.').map(|(owner, _)| owner))
            .collect();
        let mut findings = Vec::new();
        for &(sub_id, super_id) in &pairs {
            if parents.contains(sub_id) {
                continue;
            }
            let (pkg, sub) = types[sub_id];
            // Same name = build-tag variants of one type, not a hierarchy.
            if sub.name == types[super_id].1.name || is_ignorable_file(model, &sub.file) {
                continue;
            }
            let methods = methods_of(model, pkg, &sub.name);
            if methods.is_empty() || !methods.iter().all(|m| m.is_constant_return) {
                continue;
            }
            if accessed_owners.contains(sub_id) {
                continue;
            }
            findings.push(ArchFinding {
                file: Some(sub.file.clone()),
                line: Some(sub.line),
                message: format!(
                    "[remove-subclass] {sub_id} is a leaf subtype of {super_id} whose {} method(s) \
                     only return constants — consider replacing it with a field on the supertype \
                     (Fowler, Remove Subclass)",
                    methods.len()
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
    use crate::arch_model::{AccessKind, FieldAccessEdge, SymbolKind};
    use crate::hierarchy_checks::fixtures::{edge, model, sym};
    use std::path::PathBuf;

    fn run(m: &ArchModel) -> Vec<ArchFinding> {
        RemoveSubclassChecker.check(m, &ArchitectureConfig::default())
    }

    /// Base `P`, leaf `S extends P`, with `n_const` constant methods and `n_other` others on S.
    fn hierarchy(n_const: usize, n_other: usize) -> ArchModel {
        let mut syms = vec![
            sym("pkg", "P", SymbolKind::Type, None),
            sym("pkg", "S", SymbolKind::Type, None),
        ];
        for i in 0..n_const + n_other {
            let mut m = sym("pkg", &format!("M{i}"), SymbolKind::Method, Some("S"));
            m.is_constant_return = i < n_const;
            syms.push(m);
        }
        model(syms, vec![edge("pkg::S", "pkg::P", true)])
    }

    #[test]
    fn empty_model_yields_nothing() {
        assert!(run(&model(vec![], vec![])).is_empty());
    }

    #[test]
    fn flags_leaf_subtype_with_only_constant_returns() {
        let found = run(&hierarchy(2, 0));
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(
            found[0]
                .message
                .contains("pkg::S is a leaf subtype of pkg::P")
        );
        assert!(found[0].message.contains("(Fowler, Remove Subclass)"));
    }

    #[test]
    fn ignores_subtype_with_a_non_constant_method() {
        assert!(run(&hierarchy(2, 1)).is_empty());
    }

    #[test]
    fn boundary_single_constant_method_fires_and_zero_methods_does_not() {
        assert_eq!(run(&hierarchy(1, 0)).len(), 1);
        assert!(run(&hierarchy(0, 0)).is_empty());
    }

    #[test]
    fn ignores_non_leaf_subtype() {
        let mut m = hierarchy(1, 0);
        m.packages
            .get_mut("pkg")
            .unwrap()
            .symbols
            .push(sym("pkg", "Z", SymbolKind::Type, None));
        m.type_edges.push(edge("pkg::Z", "pkg::S", true));
        let found = run(&m);
        assert!(
            found.iter().all(|f| !f.message.contains("pkg::S is")),
            "{found:?}"
        );
    }

    #[test]
    fn ignores_subtype_with_its_own_field_in_use() {
        let mut m = hierarchy(1, 0);
        m.field_accesses.push(FieldAccessEdge {
            from: "pkg::S.M0".into(),
            to: "pkg::S.state".into(),
            access: AccessKind::Read,
            file: PathBuf::from("/repo/pkg/a.go"),
            line: 2,
        });
        assert!(run(&m).is_empty());
    }

    #[test]
    fn multi_level_chain_flags_only_the_leaf() {
        let mut syms = vec![
            sym("pkg", "P", SymbolKind::Type, None),
            sym("pkg", "S", SymbolKind::Type, None),
            sym("pkg", "Z", SymbolKind::Type, None),
        ];
        for owner in ["S", "Z"] {
            let mut m = sym("pkg", "K", SymbolKind::Method, Some(owner));
            m.is_constant_return = true;
            syms.push(m);
        }
        let m = model(
            syms,
            vec![
                edge("pkg::S", "pkg::P", true),
                edge("pkg::Z", "pkg::S", true),
            ],
        );
        let found = run(&m);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(
            found[0].message.contains("pkg::Z is a leaf"),
            "{}",
            found[0].message
        );
    }

    fn access(to: &str) -> FieldAccessEdge {
        FieldAccessEdge {
            from: "pkg::Q.m".into(),
            to: to.into(),
            access: AccessKind::Read,
            file: PathBuf::from("/repo/pkg/a.go"),
            line: 2,
        }
    }

    #[test]
    fn field_access_on_a_prefix_sharing_or_parent_type_does_not_suppress() {
        let mut m = hierarchy(1, 0);
        m.field_accesses = vec![access("pkg::S2.x"), access("pkg::Sx.x"), access("pkg::P.x")];
        assert_eq!(run(&m).len(), 1);
    }

    #[test]
    fn ignores_test_and_fixture_declarations_and_same_named_pairs() {
        let mut m = hierarchy(1, 0);
        for s in &mut m.packages.get_mut("pkg").unwrap().symbols {
            s.file = PathBuf::from("/repo/pkg/testdata/a.go");
        }
        assert!(run(&m).is_empty());
        let mut same = hierarchy(1, 0);
        same.packages.get_mut("pkg").unwrap().symbols[0].name = "S".into();
        assert!(run(&same).is_empty());
    }

    #[test]
    fn ignores_unresolved_edges() {
        let mut m = hierarchy(2, 0);
        m.type_edges = vec![edge("pkg::S", "external.Base", false)];
        assert!(run(&m).is_empty());
    }
}
