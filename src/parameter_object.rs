//! `parameter-object`: Fowler's *Introduce Parameter Object*. Flags a `(name, type)`
//! parameter group of 2+ that recurs, exactly, across 3+ functions/methods.
//!
//! Needs `SymbolNode::params`, which only Go and TypeScript/TSX populate — every other
//! language is silently skipped. Functions with more than `MAX_ENUMERATED_PARAMS` usable
//! params are skipped (`long-parameter-list` already covers them). Exact-subset matching only (no fuzzy/Jaccard similarity).

use std::collections::{BTreeMap, BTreeSet};

use crate::arch_model::{ArchModel, SymbolKind, SymbolNode};
use crate::architecture_checks::{ArchFinding, ArchitectureChecker};
use crate::config::{ArchitectureConfig, Severity};
use crate::import_graph::ImportGraph;

/// Go's ubiquitous first parameter; pairing it with any business parameter would flag
/// nearly every service method.
pub const DEFAULT_SUPPRESSED_PAIRS: &[(&str, &str)] = &[
    ("ctx", "context.Context"),
    ("t", "*testing.T"),
    ("b", "*testing.B"),
];

const MIN_CLUSTER_FUNCTIONS: usize = 3;
const MIN_GROUP_SIZE: usize = 2;
/// Subset enumeration is 2^n; a function with more usable params than this is skipped.
const MAX_ENUMERATED_PARAMS: usize = 10;

type Param = (String, String);

pub struct ParameterObjectChecker;

impl ArchitectureChecker for ParameterObjectChecker {
    fn name(&self) -> &str {
        "parameter-object"
    }

    /// Repeated parameter groups mostly live in unexported helpers.
    fn needs_private_symbols(&self) -> bool {
        true
    }

    fn check(
        &self,
        _graph: &ImportGraph,
        config: &ArchitectureConfig,
        model: Option<&ArchModel>,
    ) -> Vec<ArchFinding> {
        model.map_or_else(Vec::new, |model| findings_for(model, config))
    }

    fn needs_model(&self) -> bool {
        true
    }
}

fn findings_for(model: &ArchModel, config: &ArchitectureConfig) -> Vec<ArchFinding> {
    let suppressed: BTreeSet<Param> = match &config.parameter_object_suppress {
        Some(pairs) => pairs.iter().cloned().collect(),
        None => DEFAULT_SUPPRESSED_PAIRS
            .iter()
            .map(|(n, t)| (n.to_string(), t.to_string()))
            .collect(),
    };

    let mut functions: Vec<&SymbolNode> = model
        .packages
        .values()
        .flat_map(|pkg| pkg.symbols.iter())
        .filter(|s| matches!(s.kind, SymbolKind::Function | SymbolKind::Method))
        .filter(|s| !is_test_file(&s.file) && !in_fixture_dir(&s.file))
        .collect();
    functions.sort_by(|a, b| (&a.file, a.line).cmp(&(&b.file, b.line)));

    let mut index: BTreeMap<Vec<Param>, Vec<usize>> = BTreeMap::new();
    for (i, f) in functions.iter().enumerate() {
        let params = usable_params(f, &suppressed);
        for subset in subsets(&params) {
            index.entry(subset).or_default().push(i);
        }
    }

    let clusters: Vec<(&Vec<Param>, &Vec<usize>)> = index
        .iter()
        .filter(|(_, fns)| fns.len() >= MIN_CLUSTER_FUNCTIONS)
        .collect();

    // A strict superset with the same member count has identical members (its members are a
    // subset of ours), so it already reports this group.
    clusters
            .iter()
            .filter(|(subset, fns)| {
                !is_interface_implementation(&functions, subset, fns, &suppressed)
            })
            .filter(|(subset, fns)| {
                !clusters.iter().any(|(other, other_fns)| {
                    other.len() > subset.len()
                        && other_fns.len() == fns.len()
                        && subset.iter().all(|p| other.contains(p))
                })
            })
            .map(|(subset, fns)| {
                let first = functions[fns[0]];
                ArchFinding {
                    file: Some(first.file.clone()),
                    line: Some(first.line),
                    message: format!(
                        "[parameter-object] {} functions share parameter group ({}) — consider Introduce Parameter Object",
                        fns.len(),
                        format_group(subset)
                    ),
                    severity_override: Some(Severity::Advisory),
                }
            })
            .collect()
}

/// Methods on 3+ distinct types whose whole parameter list is exactly the group are one
/// interface's implementations, not a missing type.
fn is_interface_implementation(
    functions: &[&SymbolNode],
    group: &[Param],
    members: &[usize],
    suppressed: &BTreeSet<Param>,
) -> bool {
    let members: Vec<&SymbolNode> = members.iter().map(|&i| functions[i]).collect();
    let distinct_parents: BTreeSet<_> = members.iter().filter_map(|f| f.parent.as_ref()).collect();
    members
        .iter()
        .all(|f| f.kind == SymbolKind::Method && usable_params(f, suppressed).len() == group.len())
        && distinct_parents.len() >= MIN_CLUSTER_FUNCTIONS
}

fn in_fixture_dir(file: &std::path::Path) -> bool {
    file.components()
        .any(|c| matches!(c.as_os_str().to_str(), Some("fixtures" | "testdata")))
}

/// Injected-service parameters (VS Code's `logService: ILogService`, `ServicesAccessor`)
/// are dependencies, not data that wants grouping; the `I`+Capital interface-naming
/// convention is the only signal available without type resolution.
fn is_injected_service_type(ty: &str) -> bool {
    let mut chars = ty.chars();
    ty == "ServicesAccessor"
        || matches!(
            (chars.next(), chars.next(), chars.next()),
            (Some('I'), Some(u), Some(l)) if u.is_ascii_uppercase() && l.is_ascii_lowercase()
        )
}

fn is_test_file(file: &std::path::Path) -> bool {
    let name = file.file_name().and_then(|n| n.to_str()).unwrap_or("");
    name.ends_with("_test.go")
        || [".test.ts", ".test.tsx", ".spec.ts", ".spec.tsx"]
            .iter()
            .any(|suffix| name.ends_with(suffix))
}

/// Sorted, de-duplicated, named, non-suppressed params. Unnamed (`""`) and blank (`_`)
/// params carry no group identity, so they are dropped.
fn usable_params(f: &SymbolNode, suppressed: &BTreeSet<Param>) -> Vec<Param> {
    f.params
        .iter()
        .filter(|(name, _)| !name.is_empty() && name != "_")
        .filter(|p| !suppressed.contains(*p) && !is_injected_service_type(&p.1))
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// Every subset of `params` (already sorted) with at least `MIN_GROUP_SIZE` members.
/// Empty past `MAX_ENUMERATED_PARAMS` params.
fn subsets(params: &[Param]) -> Vec<Vec<Param>> {
    let n = params.len();
    if !(MIN_GROUP_SIZE..=MAX_ENUMERATED_PARAMS).contains(&n) {
        return Vec::new();
    }
    (1u32..(1 << n))
        .filter(|mask| mask.count_ones() as usize >= MIN_GROUP_SIZE)
        .map(|mask| {
            (0..n)
                .filter(|i| mask & (1 << i) != 0)
                .map(|i| params[i].clone())
                .collect()
        })
        .collect()
}

fn format_group(group: &[Param]) -> String {
    group
        .iter()
        .map(|(name, ty)| {
            if ty.is_empty() {
                name.clone()
            } else {
                format!("{name} {ty}")
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch_model::PackageNode;
    use std::path::PathBuf;

    fn func(name: &str, line: usize, params: &[(&str, &str)]) -> SymbolNode {
        SymbolNode {
            id: format!("pkg::{name}"),
            name: name.to_string(),
            kind: SymbolKind::Function,
            file: PathBuf::from("pkg/a.go"),
            line,
            exported: true,
            parent: None,
            is_pure_delegation: false,
            params: params
                .iter()
                .map(|(n, t)| (n.to_string(), t.to_string()))
                .collect(),
        }
    }

    fn model_of(symbols: Vec<SymbolNode>) -> ArchModel {
        let mut packages = BTreeMap::new();
        packages.insert(
            "pkg".to_string(),
            PackageNode {
                path: "pkg".to_string(),
                files: vec![],
                symbols,
            },
        );
        ArchModel {
            repo_root: PathBuf::new(),
            packages,
            import_edges: vec![],
            call_edges: vec![],
            field_accesses: vec![],
            type_edges: vec![],
            file_import_aliases: BTreeMap::new(),
            pruning: crate::arch_model::PruningSummary::default(),
        }
    }

    fn run(symbols: Vec<SymbolNode>) -> Vec<ArchFinding> {
        findings_for(&model_of(symbols), &ArchitectureConfig::default())
    }

    const USER: (&str, &str) = ("user_id", "string");
    const TENANT: (&str, &str) = ("tenant_id", "string");

    #[test]
    fn fires_on_exact_subset_match_in_3_or_more_functions() {
        let findings = run(vec![
            func("a", 10, &[USER, TENANT]),
            func("b", 20, &[USER, TENANT, ("x", "int")]),
            func("c", 30, &[("y", "bool"), USER, TENANT]),
        ]);
        assert_eq!(findings.len(), 1, "{findings:?}");
        let f = &findings[0];
        assert_eq!(f.line, Some(10));
        assert!(f.message.starts_with("[parameter-object] 3 functions"));
        assert!(f.message.contains("tenant_id string, user_id string"));
    }

    #[test]
    fn does_not_fire_on_cluster_of_two() {
        let findings = run(vec![
            func("a", 10, &[USER, TENANT]),
            func("b", 20, &[USER, TENANT]),
        ]);
        assert!(findings.is_empty());
    }

    #[test]
    fn does_not_fire_on_single_param_subsets() {
        let findings = run(vec![
            func("a", 10, &[USER, ("a", "int")]),
            func("b", 20, &[USER, ("b", "int")]),
            func("c", 30, &[USER, ("c", "int")]),
        ]);
        assert!(findings.is_empty());
    }

    #[test]
    fn ctx_context_pair_suppressed() {
        let ctx = ("ctx", "context.Context");
        // Only ctx + user_id is shared: with ctx suppressed that is a single-param subset.
        let findings = run(vec![
            func("a", 10, &[ctx, USER, ("a", "int")]),
            func("b", 20, &[ctx, USER, ("b", "int")]),
            func("c", 30, &[ctx, USER, ("c", "int")]),
        ]);
        assert!(findings.is_empty(), "{findings:?}");
    }

    #[test]
    fn suppression_list_is_configurable() {
        let ctx = ("ctx", "context.Context");
        let model = model_of(vec![
            func("a", 10, &[ctx, USER]),
            func("b", 20, &[ctx, USER]),
            func("c", 30, &[ctx, USER]),
        ]);
        let config = ArchitectureConfig {
            parameter_object_suppress: Some(vec![]),
            ..Default::default()
        };
        assert_eq!(findings_for(&model, &config).len(), 1);
    }

    #[test]
    fn long_parameter_lists_are_skipped() {
        let many: Vec<(String, String)> = (0..15)
            .map(|i| (format!("p{i}"), "int".to_string()))
            .collect();
        let refs: Vec<(&str, &str)> = many.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect();
        let subsets = subsets(&usable_params(&func("big", 1, &refs), &BTreeSet::new()));
        assert!(subsets.is_empty());
    }

    #[test]
    fn blank_identifier_params_never_form_a_group() {
        let findings = run(vec![
            func("a", 10, &[("_", "*v1.Pod"), ("_", "string")]),
            func("b", 20, &[("_", "*v1.Pod"), ("_", "string")]),
            func("c", 30, &[("_", "*v1.Pod"), ("_", "string")]),
        ]);
        assert!(findings.is_empty());
    }

    #[test]
    fn injected_service_types_never_form_a_group() {
        let svc = [("log", "ILogService"), ("files", "IFileService")];
        let findings = run(vec![
            func("a", 10, &svc),
            func("b", 20, &svc),
            func("c", 30, &svc),
        ]);
        assert!(findings.is_empty());
    }

    #[test]
    fn same_signature_methods_on_distinct_types_are_interface_implementations() {
        let methods: Vec<SymbolNode> = ["A", "B", "C"]
            .iter()
            .enumerate()
            .map(|(i, owner)| {
                let mut m = func("Do", 10 * (i + 1), &[USER, TENANT]);
                m.kind = SymbolKind::Method;
                m.parent = Some(owner.to_string());
                m
            })
            .collect();
        assert!(run(methods).is_empty());
    }

    #[test]
    fn id_typed_params_are_not_mistaken_for_injected_services() {
        let ids = [("user", "ID"), ("tenant", "ID")];
        let findings = run(vec![
            func("a", 10, &ids),
            func("b", 20, &ids),
            func("c", 30, &ids),
        ]);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn interface_implementations_with_a_suppressed_ctx_param_are_still_recognised() {
        let methods: Vec<SymbolNode> = ["A", "B", "C"]
            .iter()
            .enumerate()
            .map(|(i, owner)| {
                let mut m = func(
                    "Do",
                    10 * (i + 1),
                    &[("ctx", "context.Context"), USER, TENANT],
                );
                m.kind = SymbolKind::Method;
                m.parent = Some(owner.to_string());
                m
            })
            .collect();
        assert!(run(methods).is_empty());
    }

    #[test]
    fn check_reports_nothing_without_a_model() {
        let graph = ImportGraph::default();
        assert!(
            ParameterObjectChecker
                .check(&graph, &ArchitectureConfig::default(), None)
                .is_empty()
        );
    }

    #[test]
    fn check_uses_the_supplied_model() {
        let graph = ImportGraph::default();
        let model = model_of(vec![
            func("a", 10, &[USER, TENANT]),
            func("b", 20, &[USER, TENANT]),
            func("c", 30, &[USER, TENANT]),
        ]);
        let findings =
            ParameterObjectChecker.check(&graph, &ArchitectureConfig::default(), Some(&model));
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn a_pair_is_not_reported_alongside_the_triple_that_contains_it() {
        let abc = [USER, TENANT, ("x", "int")];
        let findings = run(vec![
            func("a", 10, &abc),
            func("b", 20, &abc),
            func("c", 30, &abc),
        ]);
        assert_eq!(findings.len(), 1);
        assert!(
            findings[0]
                .message
                .contains("tenant_id string, user_id string, x int")
        );
    }

    #[test]
    fn test_files_are_ignored() {
        let mut symbols = vec![
            func("a", 10, &[USER, TENANT]),
            func("b", 20, &[USER, TENANT]),
            func("c", 30, &[USER, TENANT]),
        ];
        for s in &mut symbols {
            s.file = PathBuf::from("pkg/a_test.go");
        }
        assert!(run(symbols).is_empty());
    }

    #[test]
    fn symbol_node_without_params_field_deserializes_to_empty() {
        let json = r#"{"id":"p::f","name":"f","kind":"function","file":"a.go","line":1,
            "exported":true,"parent":null}"#;
        let node: SymbolNode = serde_json::from_str(json).unwrap();
        assert!(node.params.is_empty());
    }
}
