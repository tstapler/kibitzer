//! Fowler's **Push Down Field**: a supertype field accessed only from methods of exactly
//! one direct subtype (or its descendants) belongs on that subtype. Needs
//! `ArchModel::struct_field_names` and `field_accesses`, both Go-only today, so other
//! languages never fire. The supertype's own methods are not counted as evidence (per
//! the acceptance criteria); any access from outside the hierarchy blocks the finding.
//! No access evidence at all is skipped. The supertype needs ≥2 direct subtypes.
//!
//! Accesses are only recorded through a method's own receiver, so the absence of other
//! accessors proves nothing for an exported field (any package can read `x.Field` through a
//! plain variable). Only unexported fields are therefore considered, and a Go subtype in a
//! different package than the embedded supertype is ignored.

use std::collections::{BTreeSet, HashSet};

use crate::arch_model::ArchModel;
use crate::architecture_checks::{ArchFinding, ArchModelChecker};
use crate::config::ArchitectureConfig;
use crate::hierarchy_checks::{
    go_subtype_is_foreign, is_ignorable_file, owner_type_id, resolved_extends, subtree,
    subtypes_by_super, type_index,
};

pub struct PushDownFieldChecker;

impl ArchModelChecker for PushDownFieldChecker {
    fn name(&self) -> &str {
        "push-down-field"
    }

    fn check(&self, model: &ArchModel, _config: &ArchitectureConfig) -> Vec<ArchFinding> {
        let types = type_index(model);
        let pairs = resolved_extends(model, &types);
        let children = subtypes_by_super(&pairs);

        let mut findings = Vec::new();
        for (&sup_id, subs) in &children {
            let (sup_pkg, sup) = types[sup_id];
            let subs: Vec<&str> = subs
                .iter()
                .copied()
                .filter(|s| {
                    let (spkg, st) = types[s];
                    !go_subtype_is_foreign(spkg, st, sup_pkg)
                })
                .collect();
            let Some(fields) = model.struct_field_names.get(sup_id) else {
                continue;
            };
            if subs.len() < 2 || is_ignorable_file(model, &sup.file) {
                continue;
            }
            let subtrees: Vec<HashSet<&str>> = subs.iter().map(|s| subtree(s, &children)).collect();

            for field in fields.iter().filter(|f| !is_exported_name(f)) {
                let to = format!("{sup_id}.{field}");
                let mut owning = BTreeSet::new();
                let mut evidence = false;
                let mut blocked = false;
                for a in model
                    .field_accesses
                    .iter()
                    .chain(&model.promoted_field_accesses)
                    .filter(|a| a.to == to && !is_ignorable_file(model, &a.file))
                {
                    let owner = owner_type_id(&a.from);
                    if owner == Some(sup_id) {
                        continue;
                    }
                    match owner.and_then(|o| subtrees.iter().position(|t| t.contains(o))) {
                        Some(i) => {
                            evidence = true;
                            owning.insert(i);
                        }
                        None => {
                            blocked = true;
                            break;
                        }
                    }
                }
                if blocked || !evidence || owning.len() != 1 {
                    continue;
                }
                let only = subs[*owning.iter().next().expect("len checked")];
                findings.push(ArchFinding {
                    file: Some(sup.file.clone()),
                    line: Some(sup.line),
                    message: format!(
                        "[push-down-field] field '{field}' of {sup_id} is only accessed from \
                         methods of {only} (no access from other subtypes or outside the \
                         hierarchy) — consider moving it down (Fowler, Push Down Field)"
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

/// Go export rule: an upper-case first letter.
fn is_exported_name(name: &str) -> bool {
    name.chars().next().is_some_and(char::is_uppercase)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch_model::{AccessKind, FieldAccessEdge, SymbolKind};
    use crate::hierarchy_checks::fixtures::{edge, model, sym};
    use std::path::PathBuf;

    fn access(from: &str, field: &str) -> FieldAccessEdge {
        FieldAccessEdge {
            from: from.into(),
            to: format!("pkg::Base.{field}"),
            access: AccessKind::Read,
            file: PathBuf::from("/repo/pkg/a.go"),
            line: 5,
        }
    }

    fn hierarchy() -> ArchModel {
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
        m.struct_field_names
            .insert("pkg::Base".into(), BTreeSet::from(["n".to_string()]));
        m
    }

    fn run(m: &ArchModel) -> Vec<ArchFinding> {
        PushDownFieldChecker.check(m, &ArchitectureConfig::default())
    }

    #[test]
    fn fires_when_only_one_subtype_accesses_it_ignoring_own_methods() {
        let mut m = hierarchy();
        m.field_accesses = vec![access("pkg::A.Run", "n"), access("pkg::Base.Init", "n")];
        let f = run(&m);
        assert_eq!(f.len(), 1, "{f:?}");
        assert!(f[0].message.contains("'n'") && f[0].message.contains("pkg::A"));
    }

    #[test]
    fn promoted_accesses_count_as_evidence() {
        let mut m = hierarchy();
        m.promoted_field_accesses = vec![access("pkg::A.Run", "n")];
        assert_eq!(run(&m).len(), 1);
    }

    #[test]
    fn silent_without_access_evidence() {
        let mut m = hierarchy();
        assert!(run(&m).is_empty());
        m.field_accesses = vec![access("pkg::Base.Init", "n")];
        assert!(run(&m).is_empty());
    }

    #[test]
    fn silent_when_two_subtypes_or_outsiders_access_it() {
        let mut m = hierarchy();
        m.field_accesses = vec![access("pkg::A.Run", "n"), access("pkg::B.Run", "n")];
        assert!(run(&m).is_empty());
        m.field_accesses = vec![access("pkg::A.Run", "n"), access("pkg::Other.Run", "n")];
        assert!(run(&m).is_empty());
        m.field_accesses = vec![access("pkg::A.Run", "n"), access("pkg::free", "n")];
        assert!(run(&m).is_empty());
    }

    #[test]
    fn silent_for_unresolved_edges_and_ignorable_files() {
        let mut m = hierarchy();
        m.field_accesses = vec![access("pkg::A.Run", "n")];
        for e in &mut m.type_edges {
            e.resolved = false;
        }
        assert!(run(&m).is_empty());

        let mut m = hierarchy();
        m.field_accesses = vec![access("pkg::A.Run", "n")];
        m.packages.get_mut("pkg").unwrap().symbols[0].file =
            PathBuf::from("/repo/pkg/fixtures/a.go");
        assert!(run(&m).is_empty());
    }
}

#[cfg(test)]
mod guard_tests {
    use super::*;
    use crate::arch_model::{AccessKind, FieldAccessEdge, SymbolKind};
    use crate::hierarchy_checks::fixtures::{edge, model_pkgs, sym};
    use std::path::PathBuf;

    fn build(field: &str, sub_pkg: &str) -> ArchModel {
        let base = sym("pkg", "Base", SymbolKind::Type, None);
        let subs = vec![
            sym(sub_pkg, "A", SymbolKind::Type, None),
            sym(sub_pkg, "B", SymbolKind::Type, None),
        ];
        let pkgs = if sub_pkg == "pkg" {
            vec![("pkg", [vec![base], subs].concat())]
        } else {
            vec![("pkg", vec![base]), (sub_pkg, subs)]
        };
        let mut m = model_pkgs(
            pkgs,
            vec![
                edge(&format!("{sub_pkg}::A"), "pkg::Base", true),
                edge(&format!("{sub_pkg}::B"), "pkg::Base", true),
            ],
        );
        m.struct_field_names
            .insert("pkg::Base".into(), BTreeSet::from([field.to_string()]));
        m.field_accesses = vec![FieldAccessEdge {
            from: format!("{sub_pkg}::A.Run"),
            to: format!("pkg::Base.{field}"),
            access: AccessKind::Read,
            file: PathBuf::from("/repo/pkg/a.go"),
            line: 1,
        }];
        m
    }

    fn run(m: &ArchModel) -> usize {
        PushDownFieldChecker
            .check(m, &ArchitectureConfig::default())
            .len()
    }

    #[test]
    fn unexported_same_package_fires_but_exported_or_foreign_package_does_not() {
        assert_eq!(run(&build("n", "pkg")), 1);
        assert_eq!(
            run(&build("N", "pkg")),
            0,
            "exported: other accessors are unobservable"
        );
        assert_eq!(
            run(&build("n", "other")),
            0,
            "subtype in a different Go package"
        );
    }
}
