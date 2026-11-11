//! Shared lookups for the hierarchy-cleanup checkers (`remove-subclass`,
//! `extract-superclass`, `collapse-hierarchy`). Only *resolved* `Extends` edges are
//! trusted: an unresolved target is often an external base, so it can't establish a leaf,
//! a parent's method set, or "unrelated."

use std::collections::{HashMap, HashSet};

use crate::arch_model::{ArchModel, PackageNode, SymbolKind, SymbolNode, TypeRelationKind};
use crate::checkers::is_test_file;

/// A `Type` symbol together with its package, keyed by `SymbolNode::id`.
pub(crate) fn type_index(model: &ArchModel) -> HashMap<&str, (&PackageNode, &SymbolNode)> {
    model
        .packages
        .values()
        .flat_map(|pkg| pkg.symbols.iter().map(move |s| (pkg, s)))
        .filter(|(_, s)| s.kind == SymbolKind::Type)
        .map(|(pkg, s)| (s.id.as_str(), (pkg, s)))
        .collect()
}

/// `(subtype id, supertype id)` for every resolved `Extends` edge whose endpoints are both
/// modeled types, sorted for deterministic output.
pub(crate) fn resolved_extends<'a>(
    model: &'a ArchModel,
    types: &HashMap<&str, (&PackageNode, &SymbolNode)>,
) -> Vec<(&'a str, &'a str)> {
    let mut pairs: Vec<_> = model
        .type_edges
        .iter()
        .filter(|e| e.resolved && e.kind == Some(TypeRelationKind::Extends))
        .filter(|e| types.contains_key(e.from.as_str()) && types.contains_key(e.to.as_str()))
        .map(|e| (e.from.as_str(), e.to.as_str()))
        .collect();
    pairs.sort_unstable();
    pairs.dedup();
    pairs
}

/// Test and fixture files (`fixtures/`, `testdata/`) hold deliberately odd shapes; the
/// hierarchy checks skip them.
pub(crate) fn is_ignorable_file(model: &ArchModel, file: &std::path::Path) -> bool {
    let rel = file.strip_prefix(&model.repo_root).unwrap_or(file);
    is_test_file(rel)
        || rel
            .components()
            .any(|c| matches!(c.as_os_str().to_str(), Some("fixtures" | "testdata")))
}

/// Non-test methods declared directly on `type_name` in `pkg`.
pub(crate) fn methods_of<'a>(
    model: &ArchModel,
    pkg: &'a PackageNode,
    type_name: &str,
) -> Vec<&'a SymbolNode> {
    pkg.symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Method && s.parent.as_deref() == Some(type_name))
        .filter(|s| !is_ignorable_file(model, &s.file))
        .collect()
}

/// Ids of types that some resolved `Extends` edge points at (i.e. have a subtype).
pub(crate) fn supertype_ids<'a>(pairs: &[(&'a str, &'a str)]) -> HashSet<&'a str> {
    pairs.iter().map(|&(_, parent)| parent).collect()
}

#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;
    use crate::arch_model::{PackageNode, TypeRelationEdge};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    pub fn sym(pkg: &str, name: &str, kind: SymbolKind, parent: Option<&str>) -> SymbolNode {
        let id = match parent {
            Some(p) => format!("{pkg}::{p}.{name}"),
            None => format!("{pkg}::{name}"),
        };
        SymbolNode {
            id,
            name: name.to_string(),
            kind,
            file: PathBuf::from(format!("/repo/{pkg}/a.go")),
            line: 1,
            exported: true,
            parent: parent.map(str::to_string),
            is_pure_delegation: false,
            is_constant_return: false,
            delegation_field: None,
            params: vec![],
            used_as_value: false,
            end_line: None,
        }
    }

    pub fn model(symbols: Vec<SymbolNode>, type_edges: Vec<TypeRelationEdge>) -> ArchModel {
        model_pkgs(vec![("pkg", symbols)], type_edges)
    }

    pub fn model_pkgs(
        pkgs: Vec<(&str, Vec<SymbolNode>)>,
        type_edges: Vec<TypeRelationEdge>,
    ) -> ArchModel {
        let packages: BTreeMap<String, PackageNode> = pkgs
            .into_iter()
            .map(|(path, symbols)| {
                let node = PackageNode {
                    path: path.to_string(),
                    files: vec![],
                    symbols,
                };
                (path.to_string(), node)
            })
            .collect();
        ArchModel {
            repo_root: PathBuf::from("/repo"),
            packages,
            import_edges: vec![],
            call_edges: vec![],
            field_accesses: vec![],
            type_edges,
            file_import_aliases: BTreeMap::new(),
            struct_field_names: Default::default(),
            struct_field_types: Default::default(),
            pruning: Default::default(),
        }
    }

    pub fn edge(from: &str, to: &str, resolved: bool) -> TypeRelationEdge {
        TypeRelationEdge {
            from: from.to_string(),
            to: to.to_string(),
            kind: Some(TypeRelationKind::Extends),
            resolved,
            file: PathBuf::from("/repo/pkg/a.go"),
            line: 3,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::{edge, model, sym};
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn resolved_extends_dedupes_and_drops_non_extends_unresolved_and_unmodeled() {
        let syms = vec![
            sym("pkg", "A", SymbolKind::Type, None),
            sym("pkg", "B", SymbolKind::Type, None),
        ];
        let mut implements = edge("pkg::A", "pkg::B", true);
        implements.kind = Some(TypeRelationKind::Implements);
        let mut unknown_kind = edge("pkg::B", "pkg::A", true);
        unknown_kind.kind = None;
        let edges = vec![
            edge("pkg::A", "pkg::B", true),
            edge("pkg::A", "pkg::B", true),
            implements,
            unknown_kind,
            edge("pkg::A", "ext.Base", false),
            edge("pkg::A", "pkg::Ghost", true),
        ];
        let m = model(syms, edges);
        let types = type_index(&m);
        assert_eq!(resolved_extends(&m, &types), vec![("pkg::A", "pkg::B")]);
    }

    #[test]
    fn methods_of_omits_test_and_fixture_files() {
        let mut keep = sym("pkg", "Keep", SymbolKind::Method, Some("T"));
        keep.file = PathBuf::from("/repo/pkg/a.go");
        let mut test = sym("pkg", "T1", SymbolKind::Method, Some("T"));
        test.file = PathBuf::from("/repo/pkg/a_test.go");
        let mut fixture = sym("pkg", "F1", SymbolKind::Method, Some("T"));
        fixture.file = PathBuf::from("/repo/pkg/testdata/a.go");
        let m = model(vec![keep, test, fixture], vec![]);
        let names: Vec<_> = methods_of(&m, &m.packages["pkg"], "T")
            .iter()
            .map(|s| s.name.clone())
            .collect();
        assert_eq!(names, vec!["Keep"]);
    }
}
