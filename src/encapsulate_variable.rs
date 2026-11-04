//! Fowler's **Encapsulate Variable**: an exported struct field written directly from a
//! package other than the one declaring it bypasses any invariant that package might later
//! want to enforce. Go only (`symbol_extract::field_access_supports`).
//!
//! `ArchModel::field_accesses` only records a method's accesses on its *own* receiver, so
//! it can never hold a cross-package write; the checker derives those edges itself from
//! each package's files (`extract_cross_package_writes_for_file`) and applies the rules to
//! both sets. Skipped on purpose: lowercase fields, same-package writes, reads, types whose
//! name ends in a `DTO_SUFFIXES` entry (conventionally public-and-mutable), and test files.
//! Known ceilings: `pkg.Var.Field = v` and writes through values not syntactically typed
//! (call results, interface values) are not seen. See
//! `docs/go-encapsulate-variable-false-positives.md`.

use crate::arch_model::{AccessKind, ArchModel, FieldAccessEdge};
use crate::architecture_checks::{ArchFinding, ArchModelChecker};
use crate::checker::{GrammarCache, Language};
use crate::checkers::is_test_file;
use crate::config::ArchitectureConfig;

/// First nine are the required default allowlist; the rest were added after the Kubernetes
/// backtest showed API/data-shaped types dominating the findings
/// (`docs/go-encapsulate-variable-false-positives.md`).
const DTO_SUFFIXES: [&str; 17] = [
    "Config",
    "Options",
    "Opts",
    "Params",
    "Args",
    "Request",
    "Response",
    "Settings",
    "Result",
    "Configuration",
    "Status",
    "Spec",
    "Stats",
    "Condition",
    "Info",
    "Record",
    "Row",
];

pub struct EncapsulateVariableChecker;

impl ArchModelChecker for EncapsulateVariableChecker {
    fn name(&self) -> &str {
        "encapsulate-variable"
    }

    fn check(&self, model: &ArchModel, _config: &ArchitectureConfig) -> Vec<ArchFinding> {
        let edges: Vec<FieldAccessEdge> = model
            .field_accesses
            .iter()
            .cloned()
            .chain(cross_package_write_edges(model))
            .collect();
        drop_inline_ignored(findings_for_edges(&edges, &model.repo_root))
    }
}

fn cross_package_write_edges(model: &ArchModel) -> Vec<FieldAccessEdge> {
    model
        .packages
        .values()
        .flat_map(|pkg| pkg.files.iter().map(move |file| (pkg, file)))
        .filter(|(_, file)| file.extension().is_some_and(|e| e == "go"))
        .filter(|(_, file)| !is_test_file(file.strip_prefix(&model.repo_root).unwrap_or(file)))
        .flat_map(|(pkg, file)| cross_package_write_edges_in_file(model, &pkg.path, file))
        .collect()
}

fn cross_package_write_edges_in_file(
    model: &ArchModel,
    package_path: &str,
    file: &std::path::Path,
) -> Vec<FieldAccessEdge> {
    let Ok(source) = std::fs::read_to_string(file) else {
        return Vec::new();
    };
    let Ok(tree) = GrammarCache::new().parse(Language::Go, &source) else {
        return Vec::new();
    };
    let aliases = model.file_import_aliases.get(file);
    crate::symbol_extract::extract_cross_package_writes_for_file(
        &source,
        &tree,
        package_path,
        aliases,
    )
    .into_iter()
    .map(|e| FieldAccessEdge {
        file: file.to_path_buf(),
        ..e
    })
    .collect()
}

/// `"{pkg}::{rest}"` → `(pkg, rest)`.
fn split_id(id: &str) -> Option<(&str, &str)> {
    id.split_once("::")
}

fn findings_for_edges(edges: &[FieldAccessEdge], repo_root: &std::path::Path) -> Vec<ArchFinding> {
    let mut findings: Vec<ArchFinding> = edges
        .iter()
        .filter(|e| e.access == AccessKind::Write)
        .filter(|e| !is_test_file(e.file.strip_prefix(repo_root).unwrap_or(&e.file)))
        .filter_map(finding_for_edge)
        .collect();
    findings.sort_by(|a, b| (&a.file, a.line, &a.message).cmp(&(&b.file, b.line, &b.message)));
    findings.dedup_by(|a, b| a.file == b.file && a.line == b.line && a.message == b.message);
    findings
}

/// Architecture checks bypass the per-file inline-ignore pass, so honor
/// `kibitzer:ignore encapsulate-variable` at the write site here (required by the item's
/// AC-9). Checker-local on purpose, not a dispatcher change: other architecture checks
/// deliberately don't take directives. Always `Advisory` and a default context — it only
/// matches directives, never changes severity.
fn drop_inline_ignored(findings: Vec<ArchFinding>) -> Vec<ArchFinding> {
    use crate::inline_ignores::{IgnoreTarget, InlineIgnoreContext, apply_inline_ignores};
    use std::collections::BTreeMap;

    let ctx = InlineIgnoreContext::default();
    let mut by_file: BTreeMap<std::path::PathBuf, Vec<ArchFinding>> = BTreeMap::new();
    let mut kept = Vec::new();
    for f in findings {
        match f.file.clone() {
            Some(file) if f.line.is_some() => by_file.entry(file).or_default().push(f),
            _ => kept.push(f),
        }
    }
    for (file, group) in by_file {
        let Ok(source) = std::fs::read_to_string(&file) else {
            kept.extend(group);
            continue;
        };
        let raw: Vec<crate::checker::Finding> = group
            .iter()
            .map(|f| crate::checker::Finding {
                line: f.line.unwrap_or_default(),
                message: f.message.clone(),
            })
            .collect();
        let applied = apply_inline_ignores(
            raw,
            IgnoreTarget {
                file: &file,
                source: &source,
                checker_name: "encapsulate-variable",
                severity: crate::config::Severity::Advisory,
            },
            &ctx,
        );
        kept.extend(group.into_iter().filter(|f| {
            applied
                .kept
                .iter()
                .any(|k| Some(k.line) == f.line && k.message == f.message)
        }));
    }
    kept.sort_by(|a, b| (&a.file, a.line, &a.message).cmp(&(&b.file, b.line, &b.message)));
    kept
}

fn finding_for_edge(edge: &FieldAccessEdge) -> Option<ArchFinding> {
    let (caller_pkg, caller_symbol) = split_id(&edge.from)?;
    let (field_pkg, owner_field) = split_id(&edge.to)?;
    if caller_pkg == field_pkg {
        return None;
    }
    let (type_name, field_name) = owner_field.rsplit_once('.')?;
    if !field_name.chars().next().is_some_and(char::is_uppercase) {
        return None;
    }
    if DTO_SUFFIXES.iter().any(|s| type_name.ends_with(s)) {
        return None;
    }
    Some(ArchFinding {
        file: Some(edge.file.clone()),
        line: Some(edge.line),
        message: format!(
            "[encapsulate-variable] {caller_pkg}/{caller_symbol} writes \
             {field_pkg}.{type_name}.{field_name} from outside its package — consider adding a setter."
        ),
        severity_override: None,
        operation: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch_model::PruneConfig;
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};

    fn edge(from: &str, to: &str, access: AccessKind) -> FieldAccessEdge {
        FieldAccessEdge {
            from: from.to_string(),
            to: to.to_string(),
            access,
            file: PathBuf::from("/repo/b/b.go"),
            line: 7,
        }
    }

    fn run(edges: &[FieldAccessEdge]) -> Vec<ArchFinding> {
        findings_for_edges(edges, Path::new("/repo"))
    }

    #[test]
    fn cross_package_write_to_exported_field_is_flagged_at_the_write_site() {
        let f = run(&[edge("b::U.M", "a::T.Field", AccessKind::Write)]);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].file, Some(PathBuf::from("/repo/b/b.go")));
        assert_eq!(f[0].line, Some(7));
    }

    #[test]
    fn same_package_write_is_not_flagged() {
        assert!(run(&[edge("a::T.M", "a::T.Field", AccessKind::Write)]).is_empty());
    }

    #[test]
    fn unexported_field_write_is_not_flagged() {
        assert!(run(&[edge("b::U.M", "a::T.field", AccessKind::Write)]).is_empty());
    }

    #[test]
    fn dto_suffixed_types_are_not_flagged() {
        for ty in DTO_SUFFIXES {
            let to = format!("a::Server{ty}.Field");
            assert!(
                run(&[edge("b::U.M", &to, AccessKind::Write)]).is_empty(),
                "{ty}"
            );
        }
    }

    #[test]
    fn cross_package_read_is_not_flagged() {
        assert!(run(&[edge("b::U.M", "a::T.Field", AccessKind::Read)]).is_empty());
    }

    #[test]
    fn default_allowlist_keeps_the_nine_required_suffixes() {
        for s in [
            "Config", "Options", "Opts", "Params", "Args", "Request", "Response", "Settings",
            "Result",
        ] {
            assert!(DTO_SUFFIXES.contains(&s), "{s} dropped from DTO_SUFFIXES");
        }
    }

    #[test]
    fn finding_message_has_the_documented_format() {
        let f = run(&[edge("b::U.M", "a::T.Field", AccessKind::Write)]);
        assert_eq!(
            f[0].message,
            "[encapsulate-variable] b/U.M writes a.T.Field from outside its package — consider adding a setter."
        );
    }

    #[test]
    fn test_file_write_sites_are_skipped() {
        let mut e = edge("b::U.M", "a::T.Field", AccessKind::Write);
        e.file = PathBuf::from("/repo/b/b_test.go");
        assert!(run(&[e]).is_empty());
    }

    #[test]
    fn registered_in_the_model_registry() {
        assert!(crate::architecture_checks::lookup_model("encapsulate-variable").is_some());
    }

    fn extract(src: &str) -> Vec<FieldAccessEdge> {
        let tree = GrammarCache::new().parse(Language::Go, src).unwrap();
        let aliases: HashMap<String, String> = HashMap::from([("a".to_string(), "a".to_string())]);
        crate::symbol_extract::extract_cross_package_writes_for_file(
            src,
            &tree,
            "b",
            Some(&aliases),
        )
    }

    #[test]
    fn extraction_finds_method_and_bare_function_writes_through_qualified_locals() {
        let edges = extract(
            "package b\n\nimport \"example.com/m/a\"\n\ntype U struct{}\n\n\
             func (u *U) M(x *a.T) {\n\tx.Field = 2\n}\n\n\
             func F(x *a.T) {\n\tx.Field++\n\t_ = x.Other\n}\n\n\
             func G() {\n\ty := &a.T{}\n\ty.Field = 1\n}\n",
        );
        let got: Vec<(&str, &str, usize)> = edges
            .iter()
            .map(|e| (e.from.as_str(), e.to.as_str(), e.line))
            .collect();
        assert_eq!(
            got,
            vec![
                ("b::U.M", "a::T.Field", 8),
                ("b::F", "a::T.Field", 12),
                ("b::G", "a::T.Field", 18),
            ]
        );
    }

    #[test]
    fn extraction_ignores_unresolved_aliases_and_same_package_types() {
        let edges = extract(
            "package b\n\ntype L struct{ Field int }\n\n\
             func F(x *L, y *ext.T) {\n\tx.Field = 1\n\ty.Field = 2\n}\n",
        );
        assert!(edges.is_empty(), "{edges:?}");
    }

    #[test]
    fn end_to_end_check_over_real_files_flags_the_write_and_honors_the_ignore_pass_input() {
        let dir = crate::test_support::unique_temp_dir("encapsulate-variable-e2e");
        let file = dir.join("b.go");
        std::fs::write(
            &file,
            "package b\n\nimport \"example.com/m/a\"\n\nfunc F(x *a.T) {\n\tx.Field = 1\n}\n",
        )
        .unwrap();
        let mut model = crate::arch_model::build_model(
            &dir,
            &[(file.clone(), std::fs::read_to_string(&file).unwrap())],
            &crate::import_graph::ImportGraph::default(),
            &PruneConfig {
                include_private: true,
            },
        )
        .unwrap();
        let pkg_key = model.packages.keys().next().unwrap().clone();
        model.file_import_aliases.insert(
            file.clone(),
            HashMap::from([("a".to_string(), "a".to_string())]),
        );
        let f = EncapsulateVariableChecker.check(&model, &ArchitectureConfig::default());
        assert_eq!(f.len(), 1, "{f:?}");
        assert_eq!(f[0].line, Some(6));
        assert!(
            f[0].message
                .contains(&format!("{pkg_key}/F writes a.T.Field"))
        );
    }
}
