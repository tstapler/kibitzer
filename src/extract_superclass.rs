//! Fowler's **Extract Superclass**: two types in one package with no hierarchy edge between
//! them whose methods touch largely the same fields. Reuses #38's per-method field-access
//! sets (`extract_class::method_field_sets`), unioned per type and compared by Jaccard over
//! field *names* (field ids embed the owning type, so ids from two types never match).
//!
//! Go-only (field-access data is Go-only) and same-package only. Noise guards beyond the
//! stated threshold: at least [`MIN_SHARED_FIELDS`] shared field names, and at most
//! [`MAX_FINDINGS_PER_PACKAGE`] findings per package, highest similarity first.

use std::collections::{BTreeSet, HashSet};

use crate::arch_model::{ArchModel, PackageNode, SymbolKind};
use crate::architecture_checks::{ArchFinding, ArchModelChecker};
use crate::checkers::is_test_file;
use crate::config::ArchitectureConfig;
use crate::extract_class::{MIN_METHODS_FOR_CLUSTERING, method_field_sets};
use crate::jaccard::jaccard;

const EXTRACT_SUPERCLASS_THRESHOLD: f64 = 0.5;
const MIN_SHARED_FIELDS: usize = 2;
const MAX_FINDINGS_PER_PACKAGE: usize = 3;

pub struct ExtractSuperclassChecker;

struct TypeFields<'a> {
    name: &'a str,
    id: &'a str,
    file: &'a std::path::Path,
    line: usize,
    fields: HashSet<&'a str>,
}

impl ArchModelChecker for ExtractSuperclassChecker {
    fn name(&self) -> &str {
        "extract-superclass"
    }

    fn check(&self, model: &ArchModel, _config: &ArchitectureConfig) -> Vec<ArchFinding> {
        // Either direction, resolved or not: any declared relationship rules a pair out.
        let related: HashSet<(&str, &str)> = model
            .type_edges
            .iter()
            .flat_map(|e| {
                [
                    (e.from.as_str(), e.to.as_str()),
                    (e.to.as_str(), e.from.as_str()),
                ]
            })
            .collect();
        model
            .packages
            .values()
            .flat_map(|pkg| findings_for_package(model, pkg, &related))
            .collect()
    }
}

fn type_fields<'a>(model: &'a ArchModel, pkg: &'a PackageNode) -> Vec<TypeFields<'a>> {
    let mut out: Vec<TypeFields> = pkg
        .symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Type)
        .filter(|s| !is_test_file(s.file.strip_prefix(&model.repo_root).unwrap_or(&s.file)))
        .filter_map(|ty| {
            let sets = method_field_sets(model, pkg, &ty.name);
            if sets.len() < MIN_METHODS_FOR_CLUSTERING {
                return None;
            }
            let fields = sets
                .values()
                .flatten()
                .map(|id| id.rsplit_once('.').map_or(*id, |(_, name)| name))
                .collect();
            Some(TypeFields {
                name: &ty.name,
                id: &ty.id,
                file: &ty.file,
                line: ty.line,
                fields,
            })
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(b.name));
    out
}

fn findings_for_package(
    model: &ArchModel,
    pkg: &PackageNode,
    related: &HashSet<(&str, &str)>,
) -> Vec<ArchFinding> {
    let types = type_fields(model, pkg);
    let mut hits: Vec<(f64, &TypeFields, &TypeFields, BTreeSet<&str>)> = Vec::new();
    for (i, a) in types.iter().enumerate() {
        for b in &types[i + 1..] {
            // Same name in one package = build-tag variants of one type (e.g. `Mounter` in
            // mount_linux.go / mount_unsupported.go), not two types.
            if a.name == b.name || related.contains(&(a.id, b.id)) {
                continue;
            }
            let shared: BTreeSet<&str> = a.fields.intersection(&b.fields).copied().collect();
            if shared.len() < MIN_SHARED_FIELDS {
                continue;
            }
            let sim = jaccard(&a.fields, &b.fields);
            if sim >= EXTRACT_SUPERCLASS_THRESHOLD {
                hits.push((sim, a, b, shared));
            }
        }
    }
    hits.sort_by(|x, y| {
        y.0.total_cmp(&x.0)
            .then_with(|| (x.1.name, x.2.name).cmp(&(y.1.name, y.2.name)))
    });
    hits.into_iter()
        .take(MAX_FINDINGS_PER_PACKAGE)
        .map(|(sim, a, b, shared)| render_finding(&pkg.path, sim, a, b, &shared))
        .collect()
}

fn render_finding(
    pkg_path: &str,
    sim: f64,
    a: &TypeFields,
    b: &TypeFields,
    shared: &BTreeSet<&str>,
) -> ArchFinding {
    ArchFinding {
        file: Some(a.file.to_path_buf()),
        line: Some(a.line),
        message: format!(
            "[extract-superclass] {pkg_path}::{} and {pkg_path}::{} share {:.0}% of their \
             accessed fields ({}) with no hierarchy relationship — consider extracting a \
             common superclass (Fowler, Extract Superclass)",
            a.name,
            b.name,
            sim * 100.0,
            shared.iter().copied().collect::<Vec<_>>().join(", ")
        ),
        severity_override: None,
        operation: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch_model::{AccessKind, FieldAccessEdge};
    use crate::hierarchy_checks::fixtures::{edge, model, sym};
    use std::path::PathBuf;

    fn run(m: &ArchModel) -> Vec<ArchFinding> {
        ExtractSuperclassChecker.check(m, &ArchitectureConfig::default())
    }

    /// Adds type `name` with `methods` methods, each touching the given field names.
    fn add_type(
        syms: &mut Vec<crate::arch_model::SymbolNode>,
        accesses: &mut Vec<FieldAccessEdge>,
        name: &str,
        methods: usize,
        fields: &[&str],
    ) {
        syms.push(sym("pkg", name, SymbolKind::Type, None));
        for i in 0..methods {
            let m = sym("pkg", &format!("M{i}"), SymbolKind::Method, Some(name));
            for f in fields {
                accesses.push(FieldAccessEdge {
                    from: m.id.clone(),
                    to: format!("pkg::{name}.{f}"),
                    access: AccessKind::Read,
                    file: PathBuf::from("/repo/pkg/a.go"),
                    line: 1,
                });
            }
            syms.push(m);
        }
    }

    fn two_types(a_fields: &[&str], b_fields: &[&str], methods: usize) -> ArchModel {
        let (mut syms, mut acc) = (vec![], vec![]);
        add_type(&mut syms, &mut acc, "A", methods, a_fields);
        add_type(&mut syms, &mut acc, "B", methods, b_fields);
        let mut m = model(syms, vec![]);
        m.field_accesses = acc;
        m
    }

    #[test]
    fn empty_model_yields_nothing() {
        assert!(run(&model(vec![], vec![])).is_empty());
    }

    #[test]
    fn flags_unrelated_types_with_identical_field_use() {
        let found = run(&two_types(&["id", "name"], &["id", "name"], 4));
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(
            found[0].message.contains("share 100%"),
            "{}",
            found[0].message
        );
        assert!(found[0].message.contains("(Fowler, Extract Superclass)"));
    }

    #[test]
    fn ignores_types_with_disjoint_fields() {
        assert!(run(&two_types(&["a", "b"], &["c", "d"], 4)).is_empty());
    }

    #[test]
    fn boundary_jaccard_exactly_half_fires_below_does_not() {
        // {id,name} vs {id,name,x,y}: 2/4 = 0.5 fires; vs {id,name,x,y,z}: 2/5 = 0.4 does not.
        assert_eq!(
            run(&two_types(&["id", "name"], &["id", "name", "x", "y"], 4)).len(),
            1
        );
        assert!(
            run(&two_types(
                &["id", "name"],
                &["id", "name", "x", "y", "z"],
                4
            ))
            .is_empty()
        );
    }

    #[test]
    fn boundary_method_count_below_minimum_is_skipped() {
        assert!(
            run(&two_types(
                &["id", "name"],
                &["id", "name"],
                MIN_METHODS_FOR_CLUSTERING - 1
            ))
            .is_empty()
        );
    }

    #[test]
    fn ignores_same_named_types_from_build_tag_variants() {
        let (mut syms, mut acc) = (vec![], vec![]);
        add_type(&mut syms, &mut acc, "A", 4, &["id", "name"]);
        let mut twin = syms.clone();
        twin.retain(|s| s.parent.as_deref() == Some("A") || s.name == "A");
        syms.extend(twin);
        let mut m = model(syms, vec![]);
        m.field_accesses = acc;
        assert!(run(&m).is_empty());
    }

    #[test]
    fn ignores_types_declared_in_test_files() {
        let mut m = two_types(&["id", "name"], &["id", "name"], 4);
        for s in &mut m.packages.get_mut("pkg").unwrap().symbols {
            s.file = PathBuf::from("/repo/pkg/a_test.go");
        }
        assert!(run(&m).is_empty());
    }

    #[test]
    fn single_shared_field_is_not_enough() {
        assert!(run(&two_types(&["id"], &["id"], 4)).is_empty());
    }

    #[test]
    fn ignores_pairs_with_an_existing_hierarchy_edge_resolved_or_not() {
        let mut m = two_types(&["id", "name"], &["id", "name"], 4);
        m.type_edges = vec![edge("pkg::A", "pkg::B", true)];
        assert!(run(&m).is_empty());
    }

    #[test]
    fn unresolved_edges_to_other_targets_do_not_suppress() {
        let mut m = two_types(&["id", "name"], &["id", "name"], 4);
        m.type_edges = vec![edge("pkg::A", "external.Base", false)];
        assert_eq!(run(&m).len(), 1);
    }

    #[test]
    fn caps_findings_per_package() {
        let (mut syms, mut acc) = (vec![], vec![]);
        for name in ["A", "B", "C", "D", "E"] {
            add_type(&mut syms, &mut acc, name, 4, &["id", "name"]);
        }
        let mut m = model(syms, vec![]);
        m.field_accesses = acc;
        assert_eq!(run(&m).len(), MAX_FINDINGS_PER_PACKAGE);
    }
}
