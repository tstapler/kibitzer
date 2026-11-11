//! Fowler's **Pull Up Field**: ≥2 direct sibling subtypes declare a field with the same
//! name AND the same type text, and the supertype doesn't already declare that field.
//! Reads `ArchModel::struct_field_types` (Go structs only today, so other languages never
//! fire). Name-only matches never fire.

use std::collections::BTreeMap;

use crate::arch_model::ArchModel;
use crate::architecture_checks::{ArchFinding, ArchModelChecker};
use crate::config::ArchitectureConfig;
use crate::hierarchy_checks::{
    go_subtype_is_foreign, is_ignorable_file, resolved_extends, subtypes_by_super, type_index,
};

pub struct PullUpFieldChecker;

impl ArchModelChecker for PullUpFieldChecker {
    fn name(&self) -> &str {
        "pull-up-field"
    }

    fn needs_private_symbols(&self) -> bool {
        true
    }

    fn check(&self, model: &ArchModel, _config: &ArchitectureConfig) -> Vec<ArchFinding> {
        let types = type_index(model);
        let pairs = resolved_extends(model, &types);
        let children = subtypes_by_super(&pairs);
        let mut findings = Vec::new();

        for (&sup_id, subs) in &children {
            let (sup_pkg, sup) = types[sup_id];
            let sup_fields = model.struct_field_names.get(sup_id);
            let mut groups: BTreeMap<(&str, &str), Vec<&str>> = BTreeMap::new();
            for &sub_id in subs {
                let (spkg, st) = types[sub_id];
                if st.name == sup.name
                    || is_ignorable_file(model, &st.file)
                    || go_subtype_is_foreign(spkg, st, sup_pkg)
                {
                    continue;
                }
                for (field, ty) in model.struct_field_types.get(sub_id).into_iter().flatten() {
                    if !ty.is_empty() {
                        groups.entry((field, ty)).or_default().push(sub_id);
                    }
                }
            }
            for ((field, ty), owners) in groups {
                if owners.len() < 2 || sup_fields.is_some_and(|f| f.contains(field)) {
                    continue;
                }
                let (_, first) = types[owners[0]];
                findings.push(ArchFinding {
                    file: Some(first.file.clone()),
                    line: Some(first.line),
                    message: format!(
                        "[pull-up-field] field '{field}' ({ty}) is declared in sibling subtypes \
                         {} of {sup_id} — consider pulling it up (Fowler, Pull Up Field)",
                        owners.join(", ")
                    ),
                    severity_override: None,
                    operation: None,
                });
            }
        }
        findings.sort_by(|a, b| (&a.file, a.line, &a.message).cmp(&(&b.file, b.line, &b.message)));
        findings
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch_model::SymbolKind;
    use crate::hierarchy_checks::fixtures::{edge, model, sym};
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    fn build(a: &[(&str, &str)], b: &[(&str, &str)], base: &[&str]) -> ArchModel {
        let mut m = model(
            vec![
                sym("pkg", "Base", SymbolKind::Type, None),
                sym("pkg", "A", SymbolKind::Type, None),
                sym("pkg", "B", SymbolKind::Type, None),
            ],
            vec![
                edge("pkg::A", "pkg::Base", true),
                edge("pkg::B", "pkg::Base", true),
            ],
        );
        for (id, fields) in [("pkg::A", a), ("pkg::B", b)] {
            m.struct_field_types.insert(
                id.into(),
                fields
                    .iter()
                    .map(|(n, t)| (n.to_string(), t.to_string()))
                    .collect(),
            );
        }
        m.struct_field_names.insert(
            "pkg::Base".into(),
            base.iter().map(|s| s.to_string()).collect::<BTreeSet<_>>(),
        );
        m
    }

    fn run(m: &ArchModel) -> Vec<ArchFinding> {
        PullUpFieldChecker.check(m, &ArchitectureConfig::default())
    }

    #[test]
    fn fires_on_same_name_and_type() {
        let f = run(&build(&[("count", "int")], &[("count", "int")], &[]));
        assert_eq!(f.len(), 1, "{f:?}");
        assert!(f[0].message.contains("'count' (int)"));
    }

    #[test]
    fn silent_on_same_name_different_type() {
        assert!(run(&build(&[("count", "int")], &[("count", "string")], &[])).is_empty());
    }

    #[test]
    fn silent_when_supertype_declares_it_or_only_one_sibling_has_it() {
        assert!(run(&build(&[("count", "int")], &[("count", "int")], &["count"])).is_empty());
        assert!(run(&build(&[("count", "int")], &[("other", "int")], &[])).is_empty());
    }

    #[test]
    fn go_siblings_embedding_a_foreign_package_supertype_are_skipped() {
        use crate::hierarchy_checks::fixtures::model_pkgs;
        let mut m = model_pkgs(
            vec![
                ("base", vec![sym("base", "Base", SymbolKind::Type, None)]),
                (
                    "pkg",
                    vec![
                        sym("pkg", "A", SymbolKind::Type, None),
                        sym("pkg", "B", SymbolKind::Type, None),
                    ],
                ),
            ],
            vec![
                edge("pkg::A", "base::Base", true),
                edge("pkg::B", "base::Base", true),
            ],
        );
        for id in ["pkg::A", "pkg::B"] {
            m.struct_field_types
                .insert(id.into(), [("c".to_string(), "int".to_string())].into());
        }
        assert!(run(&m).is_empty());
    }

    #[test]
    fn silent_for_unresolved_edges_and_ignorable_files() {
        let mut m = build(&[("c", "int")], &[("c", "int")], &[]);
        for e in &mut m.type_edges {
            e.resolved = false;
        }
        assert!(run(&m).is_empty());

        let mut m = build(&[("c", "int")], &[("c", "int")], &[]);
        for s in &mut m.packages.get_mut("pkg").unwrap().symbols {
            s.file = PathBuf::from("/repo/pkg/testdata/a.go");
        }
        assert!(run(&m).is_empty());
    }
}
