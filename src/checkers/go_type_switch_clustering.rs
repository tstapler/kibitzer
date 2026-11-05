//! Fowler's **Replace Conditional with Polymorphism** / **Replace Type Code with
//! Subclasses**: the same `switch x.(type)` subject repeated across 2+ methods of one Go
//! type. Builds on `go-type-switch-density` (#38's OCP proxy), which counts cases in a
//! single switch but cannot see that the same switch recurs across methods. Go only.
//! See `docs/go-type-switch-clustering-false-positives.md`.
//!
//! Subjects are compared as source text, with the method's own receiver variable
//! normalized (`r.kind` and `c.kind` match when `r`/`c` are each method's receiver). Any
//! other expression, including a parameter, must match exactly.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use tree_sitter::Node;

use crate::arch_model::{ArchModel, TypeRelationEdge};
use crate::architecture_checks::{ArchFinding, ArchModelChecker};
use crate::checker::{GrammarCache, Language};
use crate::checkers::file_size::is_generated;
use crate::checkers::go_type_switch_density::{node_text, type_case_count};
use crate::checkers::is_test_file;
use crate::config::ArchitectureConfig;
use crate::node_kind::GoKind;

/// A cluster needs this many distinct methods switching on the same subject.
const MIN_CLUSTER_METHODS: usize = 2;
/// Method names listed before the message collapses the rest into "…and N more".
const MAX_LISTED_METHODS: usize = 3;
/// A one-case type switch is a plain type assertion, not a type-code branch.
const MIN_SWITCH_CASES: usize = 2;

const POLYMORPHISM: &str = "Replace Conditional with Polymorphism";
const SUBCLASSES: &str = "Replace Type Code with Subclasses";

pub struct TypeSwitchClusteringChecker;

impl ArchModelChecker for TypeSwitchClusteringChecker {
    fn name(&self) -> &str {
        "go-type-switch-clustering"
    }

    fn check(&self, model: &ArchModel, _config: &ArchitectureConfig) -> Vec<ArchFinding> {
        let mut sites = Vec::new();
        for pkg in model.packages.values() {
            for file in pkg.files.iter().filter(|f| is_go_non_test(f, model)) {
                sites.extend(sites_in_file(&pkg.path, file));
            }
        }
        let findings = clusters(sites)
            .iter()
            .map(|c| finding_for(c, &model.type_edges))
            .collect();
        crate::encapsulate_variable::drop_inline_ignored(findings, self.name())
    }
}

fn is_go_non_test(file: &Path, model: &ArchModel) -> bool {
    file.extension().is_some_and(|e| e == "go")
        && !is_test_file(file.strip_prefix(&model.repo_root).unwrap_or(file))
}

/// One `switch x.(type)` inside a method body.
#[derive(Debug, Clone)]
struct TypeSwitchSite {
    package: String,
    owner_type: String,
    method: String,
    subject: String,
    /// `subject` with a leading receiver variable replaced by `<recv>`, so `r.kind` in one
    /// method and `c.kind` in another (different receiver names) cluster together.
    normalized_subject: String,
    case_types: Vec<String>,
    file: PathBuf,
    line: usize,
}

fn sites_in_file(package: &str, file: &Path) -> Vec<TypeSwitchSite> {
    let Ok(source) = std::fs::read_to_string(file) else {
        return Vec::new();
    };
    if is_generated(&source) {
        return Vec::new();
    }
    let Ok(tree) = GrammarCache::new().parse(Language::Go, &source) else {
        return Vec::new();
    };
    let mut sites = Vec::new();
    extract_sites(tree.root_node(), &source, package, file, &mut sites);
    sites
}

fn extract_sites(
    node: Node,
    source: &str,
    package: &str,
    file: &Path,
    out: &mut Vec<TypeSwitchSite>,
) {
    if GoKind::of(node) == GoKind::MethodDeclaration {
        collect_method_switches(node, source, package, file, out);
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        extract_sites(child, source, package, file, out);
    }
}

fn collect_method_switches(
    method: Node,
    source: &str,
    package: &str,
    file: &Path,
    out: &mut Vec<TypeSwitchSite>,
) {
    let Some(owner) = crate::symbol_extract::go_receiver_type_name(method, source) else {
        return;
    };
    let owner_type = owner.split('[').next().unwrap_or(&owner).to_string();
    let method_name = method
        .child_by_field_name("name")
        .map(|n| node_text(n, source.as_bytes()))
        .unwrap_or_default();
    let receiver_var = go_receiver_var(method, source);
    let Some(body) = method.child_by_field_name("body") else {
        return;
    };
    let mut switches = Vec::new();
    find_type_switches(body, &mut switches);
    for sw in switches {
        let Some(subject) = sw.child_by_field_name("value") else {
            continue;
        };
        if type_case_count(sw) < MIN_SWITCH_CASES {
            continue;
        }
        let subject = node_text(subject, source.as_bytes());
        out.push(TypeSwitchSite {
            package: package.to_string(),
            owner_type: owner_type.clone(),
            method: method_name.clone(),
            normalized_subject: normalize_subject(&subject, receiver_var.as_deref()),
            subject,
            case_types: case_type_names(sw, source.as_bytes()),
            file: file.to_path_buf(),
            line: sw.start_position().row + 1,
        });
    }
}

fn go_receiver_var(method: Node, source: &str) -> Option<String> {
    let receiver = method.child_by_field_name("receiver")?;
    let mut cursor = receiver.walk();
    let decl = receiver
        .children(&mut cursor)
        .find(|c| GoKind::of(*c) == GoKind::ParameterDeclaration)?;
    let name = decl.child_by_field_name("name")?;
    Some(node_text(name, source.as_bytes())).filter(|n| n != "_")
}

/// `r` / `r.kind` -> `<recv>` / `<recv>.kind` when `r` is the method's receiver variable.
fn normalize_subject(subject: &str, receiver_var: Option<&str>) -> String {
    let Some(recv) = receiver_var else {
        return subject.to_string();
    };
    match subject.strip_prefix(recv) {
        Some(rest) if rest.is_empty() || rest.starts_with('.') => format!("<recv>{rest}"),
        _ => subject.to_string(),
    }
}

fn find_type_switches<'t>(node: Node<'t>, out: &mut Vec<Node<'t>>) {
    if GoKind::of(node) == GoKind::TypeSwitchStatement {
        out.push(node);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        find_type_switches(child, out);
    }
}

fn case_type_names(type_switch: Node, src: &[u8]) -> Vec<String> {
    let mut names = Vec::new();
    let mut cursor = type_switch.walk();
    for case in type_switch
        .children(&mut cursor)
        .filter(|c| GoKind::of(*c) == GoKind::TypeCase)
    {
        let mut inner = case.walk();
        for ty in case.children_by_field_name("type", &mut inner) {
            names.push(node_text(ty, src));
        }
    }
    names
}

struct Cluster {
    owner_type: String,
    subject: String,
    /// Sorted by file then line; the first is the finding's anchor.
    sites: Vec<TypeSwitchSite>,
    methods: Vec<String>,
    case_types: BTreeSet<String>,
}

fn clusters(sites: Vec<TypeSwitchSite>) -> Vec<Cluster> {
    let mut groups: BTreeMap<(String, String, String), Vec<TypeSwitchSite>> = BTreeMap::new();
    for site in sites {
        let key = (
            site.package.clone(),
            site.owner_type.clone(),
            site.normalized_subject.clone(),
        );
        groups.entry(key).or_default().push(site);
    }
    groups
        .into_values()
        .filter_map(|mut group| {
            group.sort_by(|a, b| (&a.file, a.line).cmp(&(&b.file, b.line)));
            let mut methods: Vec<String> = Vec::new();
            for s in &group {
                if !methods.contains(&s.method) {
                    methods.push(s.method.clone());
                }
            }
            if methods.len() < MIN_CLUSTER_METHODS {
                return None;
            }
            let case_types = group.iter().flat_map(|s| s.case_types.clone()).collect();
            Some(Cluster {
                owner_type: group[0].owner_type.clone(),
                subject: group[0].subject.clone(),
                methods,
                case_types,
                sites: group,
            })
        })
        .collect()
}

/// `*pkg.Foo` -> `Foo`: pointer prefix and package qualifier stripped.
fn bare_type_name(case_type: &str) -> &str {
    let t = case_type.trim().trim_start_matches('*');
    t.rsplit('.').next().unwrap_or(t)
}

/// A hierarchy exists when some type-hierarchy edge endpoint is exactly one of the
/// cluster's case types: an endpoint id is `pkg::Name`, matched by suffix at the `::`
/// boundary (never by substring, so `AuthRequest` cannot match `HttpRequest`).
fn hierarchy_exists(case_types: &BTreeSet<String>, edges: &[TypeRelationEdge]) -> bool {
    let names: BTreeSet<&str> = case_types.iter().map(|t| bare_type_name(t)).collect();
    let matches = |endpoint: &str| {
        let local = endpoint.rsplit("::").next().unwrap_or(endpoint);
        names.contains(local)
    };
    edges.iter().any(|e| matches(&e.from) || matches(&e.to))
}

fn finding_for(cluster: &Cluster, edges: &[TypeRelationEdge]) -> ArchFinding {
    let (recommendation, slug) = if hierarchy_exists(&cluster.case_types, edges) {
        (POLYMORPHISM, "replace-conditional-with-polymorphism")
    } else {
        (SUBCLASSES, "replace-type-code-with-subclasses")
    };
    let anchor = &cluster.sites[0];
    ArchFinding {
        file: Some(anchor.file.clone()),
        line: Some(anchor.line),
        message: format!(
            "[go-type-switch-clustering] `{}` switches on `{}` in {} methods ({}) — a \
             type-code switch repeated across methods is a {recommendation} candidate \
             (Fowler: https://refactoring.com/catalog/{slug}.html)",
            cluster.owner_type,
            cluster.subject,
            cluster.methods.len(),
            method_list(&cluster.methods),
        ),
        severity_override: None,
        operation: None,
    }
}

fn method_list(methods: &[String]) -> String {
    let shown: Vec<&str> = methods
        .iter()
        .take(MAX_LISTED_METHODS)
        .map(String::as_str)
        .collect();
    let mut out = shown.join(", ");
    if methods.len() > MAX_LISTED_METHODS {
        out.push_str(&format!(
            ", …and {} more",
            methods.len() - MAX_LISTED_METHODS
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch_model::{PackageNode, PruningSummary, TypeRelationKind};
    use std::path::PathBuf;

    fn write_files(label: &str, files: &[(&str, &str)]) -> Vec<PathBuf> {
        let dir = std::env::temp_dir().join(format!("kibitzer-tsc-{}-{label}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        files
            .iter()
            .map(|(name, body)| {
                let p = dir.join(name);
                std::fs::write(&p, body).unwrap();
                p
            })
            .collect()
    }

    fn model(files: Vec<PathBuf>, type_edges: Vec<TypeRelationEdge>) -> ArchModel {
        let pkg = PackageNode {
            path: "pkg".to_string(),
            files,
            symbols: vec![],
        };
        ArchModel {
            repo_root: PathBuf::new(),
            packages: BTreeMap::from([("pkg".to_string(), pkg)]),
            import_edges: vec![],
            call_edges: vec![],
            field_accesses: vec![],
            type_edges,
            file_import_aliases: BTreeMap::new(),
            pruning: PruningSummary::default(),
        }
    }

    fn run(label: &str, files: &[(&str, &str)], edges: Vec<TypeRelationEdge>) -> Vec<ArchFinding> {
        let paths = write_files(label, files);
        TypeSwitchClusteringChecker.check(&model(paths, edges), &ArchitectureConfig::default())
    }

    fn method(name: &str, subject: &str) -> String {
        format!(
            "func (r *Renderer) {name}() {{\n\tswitch {subject}.(type) {{\n\tcase *Circle:\n\tcase *Square:\n\t}}\n}}\n"
        )
    }

    fn src(methods: &[(&str, &str)]) -> String {
        let body: String = methods.iter().map(|(n, s)| method(n, s)).collect();
        format!("package pkg\n\ntype Renderer struct{{ kind interface{{}} }}\n\n{body}")
    }

    fn edge(from: &str, to: &str) -> TypeRelationEdge {
        TypeRelationEdge {
            from: from.to_string(),
            to: to.to_string(),
            kind: Some(TypeRelationKind::Implements),
            resolved: true,
            file: PathBuf::from("pkg/x.go"),
            line: 1,
        }
    }

    #[test]
    fn two_methods_same_subject_fire() {
        let s = src(&[("Draw", "r.kind"), ("Area", "r.kind")]);
        let findings = run("fires", &[("a.go", &s)], vec![]);
        assert_eq!(findings.len(), 1, "got: {findings:?}");
        let m = &findings[0].message;
        assert!(m.contains("`Renderer`") && m.contains("`r.kind`"), "{m}");
        assert!(m.contains("2 methods (Draw, Area)"), "{m}");
    }

    #[test]
    fn one_method_does_not_cluster() {
        let s = src(&[("Draw", "r.kind")]);
        assert!(run("one", &[("a.go", &s)], vec![]).is_empty());
    }

    #[test]
    fn two_switches_in_one_method_do_not_cluster() {
        let s = "package pkg\n\ntype Renderer struct{}\n\nfunc (r *Renderer) Draw() {\n\tswitch r.kind.(type) {\n\tcase *Circle:\n\tcase *Square:\n\t}\n\tswitch r.kind.(type) {\n\tcase *Circle:\n\tcase *Square:\n\t}\n}\n"
            .to_string();
        assert!(run("same-method", &[("a.go", &s)], vec![]).is_empty());
    }

    #[test]
    fn different_subjects_do_not_cluster() {
        let s = src(&[("Draw", "r.kind"), ("Area", "r.shape")]);
        assert!(run("diff", &[("a.go", &s)], vec![]).is_empty());
    }

    #[test]
    fn hierarchy_present_recommends_polymorphism() {
        let s = src(&[("Draw", "r.kind"), ("Area", "r.kind")]);
        let edges = vec![edge("pkg::Circle", "pkg::Shape")];
        let findings = run("hier", &[("a.go", &s)], edges);
        assert!(
            findings[0].message.contains(POLYMORPHISM),
            "{}",
            findings[0].message
        );
    }

    #[test]
    fn hierarchy_absent_recommends_subclasses() {
        let s = src(&[("Draw", "r.kind"), ("Area", "r.kind")]);
        let findings = run("nohier", &[("a.go", &s)], vec![]);
        assert!(
            findings[0].message.contains(SUBCLASSES),
            "{}",
            findings[0].message
        );
    }

    #[test]
    fn hierarchy_match_is_exact_not_substring() {
        let s = src(&[("Draw", "r.kind"), ("Area", "r.kind")]);
        let edges = vec![edge("pkg::BigCircle", "pkg::Shape")];
        let findings = run("substr", &[("a.go", &s)], edges);
        assert!(findings[0].message.contains(SUBCLASSES));
    }

    #[test]
    fn generated_file_is_skipped() {
        let s = format!(
            "// Code generated by mockgen. DO NOT EDIT.\n{}",
            src(&[("Draw", "r.kind"), ("Area", "r.kind")])
        );
        assert!(run("gen", &[("a.go", &s)], vec![]).is_empty());
    }

    #[test]
    fn method_list_truncates_after_three() {
        let s = src(&[
            ("A", "r.kind"),
            ("B", "r.kind"),
            ("C", "r.kind"),
            ("D", "r.kind"),
            ("E", "r.kind"),
        ]);
        let findings = run("trunc", &[("a.go", &s)], vec![]);
        assert!(
            findings[0].message.contains("(A, B, C, …and 2 more)"),
            "{}",
            findings[0].message
        );
    }

    #[test]
    fn clusters_across_two_files_in_one_package() {
        let a = src(&[("Draw", "r.kind")]);
        let b = "package pkg\n\nfunc (r *Renderer) Area() {\n\tswitch r.kind.(type) {\n\tcase *Circle:\n\tcase *Square:\n\t}\n}\n";
        let findings = run("multi", &[("a.go", &a), ("b.go", b)], vec![]);
        assert_eq!(findings.len(), 1, "got: {findings:?}");
        assert!(findings[0].message.contains("(Draw, Area)"));
    }

    #[test]
    fn different_receiver_names_cluster_on_the_same_field() {
        let a = "package pkg\n\nfunc (r *Renderer) Draw() {\n\tswitch r.kind.(type) {\n\tcase *Circle:\n\tcase *Square:\n\t}\n}\n";
        let b = "package pkg\n\nfunc (c *Renderer) Area() {\n\tswitch c.kind.(type) {\n\tcase *Circle:\n\tcase *Square:\n\t}\n}\n";
        let findings = run("recv", &[("a.go", a), ("b.go", b)], vec![]);
        assert_eq!(findings.len(), 1, "got: {findings:?}");
    }

    #[test]
    fn same_parameter_name_is_not_normalized() {
        let a = "package pkg\n\nfunc (r *Renderer) Draw(v any) {\n\tswitch v.(type) {\n\tcase *Circle:\n\tcase *Square:\n\t}\n}\n";
        let b = "package pkg\n\nfunc (c *Renderer) Area(w any) {\n\tswitch w.(type) {\n\tcase *Circle:\n\tcase *Square:\n\t}\n}\n";
        assert!(run("param", &[("a.go", a), ("b.go", b)], vec![]).is_empty());
    }

    #[test]
    fn test_files_are_skipped() {
        let s = src(&[("Draw", "r.kind"), ("Area", "r.kind")]);
        assert!(run("testfile", &[("a_test.go", &s)], vec![]).is_empty());
    }
}
