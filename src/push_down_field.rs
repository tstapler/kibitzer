//! Fowler's **Push Down Field**: a supertype field accessed only from methods of exactly
//! one direct subtype (or its descendants) belongs on that subtype. Needs
//! `ArchModel::struct_field_names` and `field_accesses`, both Go-only today, so other
//! languages never fire. The supertype's own methods are not counted as evidence (per
//! the acceptance criteria); any access from outside the hierarchy blocks the finding.
//! No access evidence at all is skipped. The supertype needs ≥2 direct subtypes.

use std::collections::{BTreeSet, HashSet};

use crate::arch_model::ArchModel;
use crate::architecture_checks::{ArchFinding, ArchModelChecker};
use crate::config::ArchitectureConfig;
use crate::hierarchy_checks::{
    is_ignorable_file, owner_type_id, resolved_extends, subtree, subtypes_by_super, type_index,
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
            let (_, sup) = types[sup_id];
            let Some(fields) = model.struct_field_names.get(sup_id) else {
                continue;
            };
            if subs.len() < 2 || is_ignorable_file(model, &sup.file) {
                continue;
            }
            let subtrees: Vec<HashSet<&str>> = subs.iter().map(|s| subtree(s, &children)).collect();

            for field in fields {
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
