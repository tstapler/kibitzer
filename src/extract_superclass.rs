//! Fowler's **Extract Superclass**: two types in one package with no hierarchy edge between
//! them whose methods touch largely the same fields. Each type's field set is the union of
//! its methods' accessed field *names* (field ids embed the owning type, so ids from two
//! types never match), compared by Jaccard (`crate::jaccard`).
//!
//! Go-only (field-access data is Go-only) and same-package only. Noise guards beyond the
//! stated threshold: at least [`MIN_SHARED_FIELDS`] shared field names, test/fixture files
//! and same-named build-tag variants skipped, and at most [`MAX_FINDINGS_PER_PACKAGE`]
//! findings per package, highest similarity first.

use std::collections::{BTreeSet, HashMap, HashSet};

use crate::arch_model::{ArchModel, PackageNode, SymbolKind};
use crate::architecture_checks::{ArchFinding, ArchModelChecker};
use crate::config::ArchitectureConfig;
use crate::hierarchy_checks::{is_ignorable_file, methods_of};
use crate::jaccard::jaccard;

const EXTRACT_SUPERCLASS_THRESHOLD: f64 = 0.5;
const MIN_SHARED_FIELDS: usize = 2;
const MAX_FINDINGS_PER_PACKAGE: usize = 3;
/// Methods with field-access data a type needs before it is compared. Same value as
/// `extract_class`'s clustering minimum, kept separate so retuning one doesn't move the other.
const MIN_METHODS_WITH_FIELD_ACCESS: usize = 4;

pub struct ExtractSuperclassChecker;

struct TypeFields<'a> {
    name: &'a str,
    id: &'a str,
    file: &'a std::path::Path,
    line: usize,
    fields: HashSet<&'a str>,
}

/// Method id -> field ids it accesses, built once per model.
type AccessIndex<'a> = HashMap<&'a str, Vec<&'a str>>;

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
        let mut accesses: AccessIndex = HashMap::new();
        for a in &model.field_accesses {
            accesses
                .entry(a.from.as_str())
                .or_default()
                .push(a.to.as_str());
        }
        model
            .packages
            .values()
            .flat_map(|pkg| findings_for_package(model, pkg, &accesses, &related))
            .collect()
    }
}

fn type_fields<'a>(
    model: &'a ArchModel,
    pkg: &'a PackageNode,
    accesses: &AccessIndex<'a>,
) -> Vec<TypeFields<'a>> {
    let mut out: Vec<TypeFields> = pkg
        .symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Type && !is_ignorable_file(model, &s.file))
        .filter_map(|ty| {
            let per_method: Vec<&Vec<&str>> = methods_of(model, pkg, &ty.name)
                .into_iter()
                .filter(|m| m.file.extension().is_some_and(|e| e == "go"))
                .filter_map(|m| accesses.get(m.id.as_str()))
                .collect();
            if per_method.len() < MIN_METHODS_WITH_FIELD_ACCESS {
                return None;
            }
            let fields = per_method
                .into_iter()
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
    accesses: &AccessIndex,
    related: &HashSet<(&str, &str)>,
) -> Vec<ArchFinding> {
    let types = type_fields(model, pkg, accesses);
    let mut hits: Vec<(f64, &TypeFields, &TypeFields, BTreeSet<&str>)> = Vec::new();
    for (i, a) in types.iter().enumerate() {
        for b in &types[i + 1..] {
            // Same name in one package = build-tag variants of one type, not two types.
            if a.name == b.name || related.contains(&(a.id, b.id)) {
                continue;
            }
            // Jaccard <= min/max sizes: skip the intersection when it can't reach the bar.
            let (lo, hi) = (
                a.fields.len().min(b.fields.len()),
                a.fields.len().max(b.fields.len()),
            );
            if (lo as f64) / (hi as f64) < EXTRACT_SUPERCLASS_THRESHOLD {
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
    use crate::arch_model::{AccessKind, FieldAccessEdge, SymbolNode};
    use crate::hierarchy_checks::fixtures::{edge, model, model_pkgs, sym};
    use std::path::PathBuf;

    fn run(m: &ArchModel) -> Vec<ArchFinding> {
        ExtractSuperclassChecker.check(m, &ArchitectureConfig::default())
    }

    /// Type `name` in `pkg` with one method per entry of `per_method`, touching those fields.
    fn add_type(
        syms: &mut Vec<SymbolNode>,
        acc: &mut Vec<FieldAccessEdge>,
        pkg: &str,
        name: &str,
        per_method: &[&[&str]],
    ) {
        syms.push(sym(pkg, name, SymbolKind::Type, None));
        for (i, fields) in per_method.iter().enumerate() {
            let m = sym(pkg, &format!("M{i}"), SymbolKind::Method, Some(name));
            for f in *fields {
                acc.push(FieldAccessEdge {
                    from: m.id.clone(),
                    to: format!("{pkg}::{name}.{f}"),
                    access: AccessKind::Read,
                    file: PathBuf::from("/repo/pkg/a.go"),
                    line: 1,
                });
            }
            syms.push(m);
        }
    }

    fn uniform(fields: &[&'static str], methods: usize) -> Vec<&'static [&'static str]> {
        let leaked: &'static [&'static str] = Box::leak(fields.to_vec().into_boxed_slice());
        vec![leaked; methods]
    }

    fn two_types(
        a_fields: &[&'static str],
        b_fields: &[&'static str],
        methods: usize,
    ) -> ArchModel {
        let (mut syms, mut acc) = (vec![], vec![]);
        add_type(&mut syms, &mut acc, "pkg", "A", &uniform(a_fields, methods));
        add_type(&mut syms, &mut acc, "pkg", "B", &uniform(b_fields, methods));
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
            found[0]
                .message
                .contains("pkg::A and pkg::B share 100% of their accessed fields (id, name)"),
            "{}",
            found[0].message
        );
        assert!(found[0].message.contains("(Fowler, Extract Superclass)"));
        assert_eq!(found[0].line, Some(1));
    }

    #[test]
    fn ignores_types_with_disjoint_fields() {
        assert!(run(&two_types(&["a", "b"], &["c", "d"], 4)).is_empty());
    }

    #[test]
    fn boundary_jaccard_exactly_half_fires_below_does_not() {
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
        let below = MIN_METHODS_WITH_FIELD_ACCESS - 1;
        assert!(run(&two_types(&["id", "name"], &["id", "name"], below)).is_empty());
    }

    #[test]
    fn single_shared_field_is_not_enough() {
        assert!(run(&two_types(&["id"], &["id"], 4)).is_empty());
    }

    #[test]
    fn field_set_is_the_union_across_a_types_methods() {
        let (mut syms, mut acc) = (vec![], vec![]);
        let split: &[&[&str]] = &[&["id"], &["name"], &["id"], &["name"]];
        add_type(&mut syms, &mut acc, "pkg", "A", split);
        add_type(
            &mut syms,
            &mut acc,
            "pkg",
            "B",
            &uniform(&["id", "name"], 4),
        );
        let mut m = model(syms, vec![]);
        m.field_accesses = acc;
        assert_eq!(run(&m).len(), 1);
    }

    #[test]
    fn ignores_pairs_with_an_existing_edge_in_either_direction_resolved_or_not() {
        for (from, to, resolved) in [
            ("pkg::A", "pkg::B", true),
            ("pkg::B", "pkg::A", true),
            ("pkg::A", "pkg::B", false),
            ("pkg::B", "pkg::A", false),
        ] {
            let mut m = two_types(&["id", "name"], &["id", "name"], 4);
            m.type_edges = vec![edge(from, to, resolved)];
            assert!(run(&m).is_empty(), "{from}->{to} resolved={resolved}");
        }
    }

    #[test]
    fn unrelated_unresolved_edges_do_not_suppress() {
        let mut m = two_types(&["id", "name"], &["id", "name"], 4);
        m.type_edges = vec![edge("pkg::A", "external.Base", false)];
        assert_eq!(run(&m).len(), 1);
    }

    #[test]
    fn ignores_same_named_types_from_build_tag_variants() {
        let (mut syms, mut acc) = (vec![], vec![]);
        add_type(
            &mut syms,
            &mut acc,
            "pkg",
            "A",
            &uniform(&["id", "name"], 4),
        );
        let twin: Vec<SymbolNode> = syms.clone();
        syms.extend(twin);
        let mut m = model(syms, vec![]);
        m.field_accesses = acc;
        assert!(run(&m).is_empty());
    }

    #[test]
    fn ignores_types_declared_in_test_files_and_test_only_methods() {
        let mut m = two_types(&["id", "name"], &["id", "name"], 4);
        for s in &mut m.packages.get_mut("pkg").unwrap().symbols {
            s.file = PathBuf::from("/repo/pkg/a_test.go");
        }
        assert!(run(&m).is_empty());
        // Methods living in a test file contribute nothing, so A falls below the minimum.
        let mut m = two_types(&["id", "name"], &["id", "name"], 4);
        for s in &mut m.packages.get_mut("pkg").unwrap().symbols {
            if s.parent.as_deref() == Some("A") {
                s.file = PathBuf::from("/repo/pkg/a_test.go");
            }
        }
        assert!(run(&m).is_empty());
    }

    #[test]
    fn cap_keeps_the_highest_similarity_pairs_in_order_per_package() {
        let build = |pkg: &str, syms: &mut Vec<SymbolNode>, acc: &mut Vec<FieldAccessEdge>| {
            add_type(syms, acc, pkg, "A", &uniform(&["id", "name"], 4));
            add_type(syms, acc, pkg, "B", &uniform(&["id", "name"], 4));
            add_type(syms, acc, pkg, "C", &uniform(&["id", "name", "x"], 4));
            add_type(syms, acc, pkg, "D", &uniform(&["id", "name", "x", "y"], 4));
        };
        let (mut p1, mut p2, mut acc) = (vec![], vec![], vec![]);
        build("p1", &mut p1, &mut acc);
        build("p2", &mut p2, &mut acc);
        let mut m = model_pkgs(vec![("p1", p1), ("p2", p2)], vec![]);
        m.field_accesses = acc;
        let found = run(&m);
        assert_eq!(found.len(), 2 * MAX_FINDINGS_PER_PACKAGE);
        let first_pkg: Vec<&str> = found[..3].iter().map(|f| f.message.as_str()).collect();
        assert!(
            first_pkg[0].contains("p1::A and p1::B share 100%"),
            "{}",
            first_pkg[0]
        );
        assert!(
            first_pkg[1].contains("p1::C and p1::D share 75%"),
            "{}",
            first_pkg[1]
        );
        assert!(
            first_pkg[2].contains("p1::A and p1::C share 67%"),
            "{}",
            first_pkg[2]
        );
        assert!(found[3].message.contains("p2::A and p2::B share 100%"));
    }
}
