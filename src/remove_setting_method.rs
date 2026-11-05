//! Fowler's Remove Setting Method (Go only): a `Set*`/`set*` method that writes one receiver
//! field, makes no calls, and is never called or only called from constructors. Callers match
//! by callee name alone (over-counts, so fewer findings). Skips exported setters outside
//! `internal/`, test files, and types with an `implements` edge. See docs/remove-setting-method.md.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::arch_model::{AccessKind, ArchModel, SymbolKind, SymbolNode};
use crate::architecture_checks::{ArchFinding, ArchModelChecker};
use crate::checkers::is_test_file;
use crate::config::ArchitectureConfig;
use crate::middle_man::{implementer_type_ids, implements_declared_interface};

const CONSTRUCTOR_PREFIXES: [&str; 2] = ["New", "new"];
const SETTER_PREFIXES: [&str; 2] = ["Set", "set"];

pub struct RemoveSettingMethodChecker;

impl ArchModelChecker for RemoveSettingMethodChecker {
    fn name(&self) -> &str {
        "remove-setting-method"
    }

    /// Unexported setters (`setFoo`) are the main subject; without this the model prunes them.
    fn needs_private_symbols(&self) -> bool {
        true
    }

    fn check(&self, model: &ArchModel, _config: &ArchitectureConfig) -> Vec<ArchFinding> {
        let symbols_by_id: HashMap<&str, &SymbolNode> = model
            .packages
            .values()
            .flat_map(|pkg| pkg.symbols.iter())
            .map(|s| (s.id.as_str(), s))
            .collect();
        let implementers = implementer_type_ids(model);
        let makes_calls: HashSet<&str> = model.call_edges.iter().map(|e| e.from.as_str()).collect();
        let writes = single_field_writers(model);
        let callers_by_name = callers_by_callee_name(model, &symbols_by_id);

        let mut findings: Vec<ArchFinding> = model
            .packages
            .values()
            .flat_map(|pkg| pkg.symbols.iter().map(move |s| (pkg.path.as_str(), s)))
            .filter(|(pkg_path, sym)| {
                is_candidate(pkg_path, sym, &model.repo_root, &implementers)
                    && !makes_calls.contains(sym.id.as_str())
            })
            .filter_map(|(pkg_path, sym)| {
                let field = writes.get(sym.id.as_str())?;
                let callers = callers_by_name.get(sym.name.as_str());
                finding_for(pkg_path, sym, field, callers.map_or(&[][..], Vec::as_slice))
            })
            .collect();
        findings.sort_by(|a, b| (&a.file, a.line).cmp(&(&b.file, b.line)));
        findings
    }
}

/// `method id -> field name` for every method whose only field accesses are writes to one field.
fn single_field_writers(model: &ArchModel) -> HashMap<&str, &str> {
    let mut accesses: HashMap<&str, Vec<(&str, AccessKind)>> = HashMap::new();
    for edge in &model.field_accesses {
        accesses
            .entry(edge.from.as_str())
            .or_default()
            .push((edge.to.as_str(), edge.access));
    }
    accesses
        .into_iter()
        .filter_map(|(method, edges)| {
            let (field, _) = edges.first()?;
            let only_writes_one_field = edges
                .iter()
                .all(|(f, kind)| f == field && *kind == AccessKind::Write);
            only_writes_one_field
                .then(|| (method, field.rsplit_once('.').map_or(*field, |(_, f)| f)))
        })
        .collect()
}

/// `callee name -> caller symbols`, keyed by the callee's last path segment. A caller with no
/// `SymbolNode` (e.g. a func literal) is kept as `None` so it still counts as a non-constructor.
fn callers_by_callee_name<'a>(
    model: &'a ArchModel,
    symbols_by_id: &HashMap<&'a str, &'a SymbolNode>,
) -> HashMap<&'a str, Vec<Option<&'a SymbolNode>>> {
    let mut map: HashMap<&str, Vec<Option<&SymbolNode>>> = HashMap::new();
    for edge in &model.call_edges {
        let callee = edge
            .to
            .rsplit_once(['.', ':'])
            .map_or(edge.to.as_str(), |(_, c)| c);
        map.entry(callee)
            .or_default()
            .push(symbols_by_id.get(edge.from.as_str()).copied());
    }
    map
}

fn is_candidate(
    pkg_path: &str,
    sym: &SymbolNode,
    repo_root: &Path,
    implementers: &HashSet<&str>,
) -> bool {
    sym.kind == SymbolKind::Method
        && is_setter_name(&sym.name)
        && (!sym.exported || is_internal_package(pkg_path))
        && !is_test_file(sym.file.strip_prefix(repo_root).unwrap_or(&sym.file))
        && !implements_declared_interface(sym, implementers)
}

fn is_setter_name(name: &str) -> bool {
    SETTER_PREFIXES.iter().any(|prefix| {
        name.strip_prefix(prefix)
            .and_then(|rest| rest.chars().next())
            .is_some_and(char::is_uppercase)
    })
}

fn is_internal_package(pkg_path: &str) -> bool {
    pkg_path.split('/').any(|segment| segment == "internal")
}

fn is_constructor(caller: &SymbolNode, setter: &SymbolNode) -> bool {
    caller.kind == SymbolKind::Function
        && caller.file.parent() == setter.file.parent()
        && CONSTRUCTOR_PREFIXES.iter().any(|prefix| {
            caller.name.strip_prefix(prefix).is_some_and(|rest| {
                rest.is_empty()
                    || (rest.starts_with(char::is_uppercase)
                        && setter
                            .parent
                            .as_deref()
                            .is_some_and(|t| rest.to_lowercase().contains(&t.to_lowercase())))
            })
        })
}

fn finding_for(
    pkg_path: &str,
    setter: &SymbolNode,
    field: &str,
    callers: &[Option<&SymbolNode>],
) -> Option<ArchFinding> {
    let type_name = setter.parent.as_deref()?;
    let detail = if callers.is_empty() {
        format!("is never called, so '{field}' never changes after construction")
    } else if callers
        .iter()
        .all(|c| c.is_some_and(|caller| is_constructor(caller, setter)))
    {
        format!("is only called from constructors, so '{field}' is effectively set once")
    } else {
        return None;
    };
    Some(ArchFinding {
        file: Some(setter.file.clone()),
        line: Some(setter.line),
        message: format!(
            "[remove-setting-method] {pkg_path}::{type_name}.{} {detail} — make '{field}' \
             a constructor argument and remove the setter (Fowler, Remove Setting Method)",
            setter.name
        ),
        severity_override: None,
        operation: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch_model::PruneConfig;
    use std::path::PathBuf;

    fn model_from(files: Vec<(&str, &str)>) -> ArchModel {
        let files: Vec<(PathBuf, String)> = files
            .into_iter()
            .map(|(path, src)| (PathBuf::from(path), src.to_string()))
            .collect();
        crate::arch_model::build_model(
            &PathBuf::from("/repo"),
            &files,
            &crate::import_graph::ImportGraph::default(),
            &PruneConfig {
                include_private: true,
            },
        )
        .unwrap()
    }

    fn findings(files: Vec<(&str, &str)>) -> Vec<ArchFinding> {
        RemoveSettingMethodChecker.check(&model_from(files), &ArchitectureConfig::default())
    }

    const TYPE: &str = "package pkg\n\ntype t struct{ name string; n int }\n\n\
        func (x *t) setName(v string) { x.name = v }\n";

    #[test]
    fn flags_unused_setter() {
        let found = findings(vec![("/repo/pkg/a.go", TYPE)]);
        assert_eq!(found.len(), 1, "got: {found:?}");
        assert!(
            found[0].message.contains("is never called"),
            "{}",
            found[0].message
        );
        assert!(found[0].message.contains("(Fowler, Remove Setting Method)"));
    }

    #[test]
    fn flags_setter_only_called_from_constructor() {
        let src =
            format!("{TYPE}\nfunc newT(n string) *t {{ x := &t{{}}; x.setName(n); return x }}\n");
        let found = findings(vec![("/repo/pkg/a.go", &src)]);
        assert_eq!(found.len(), 1, "got: {found:?}");
        assert!(found[0].message.contains("only called from constructors"));
    }

    #[test]
    fn ignores_setter_called_outside_constructor() {
        let src = format!("{TYPE}\nfunc rename(x *t) {{ x.setName(\"y\") }}\n");
        assert!(findings(vec![("/repo/pkg/a.go", &src)]).is_empty());
    }

    #[test]
    fn ignores_setter_called_from_another_file_method() {
        let other = "package pkg\n\nfunc (x *t) reset() { x.setName(\"\") }\n";
        assert!(findings(vec![("/repo/pkg/a.go", TYPE), ("/repo/pkg/b.go", other)]).is_empty());
    }

    #[test]
    fn ignores_exported_setter_in_public_package() {
        let src = "package pkg\n\ntype T struct{ name string }\n\nfunc (x *T) SetName(v string) { x.name = v }\n";
        assert!(findings(vec![("/repo/pkg/a.go", src)]).is_empty());
    }

    #[test]
    fn flags_exported_setter_in_internal_package() {
        let src = "package pkg\n\ntype T struct{ name string }\n\nfunc (x *T) SetName(v string) { x.name = v }\n";
        assert_eq!(findings(vec![("/repo/internal/pkg/a.go", src)]).len(), 1);
    }

    #[test]
    fn ignores_method_that_reads_fields() {
        let src = "package pkg\n\ntype t struct{ n int; dirty bool }\n\n\
            func (x *t) setN(v int) { if x.n != v { x.n = v } }\n";
        assert!(findings(vec![("/repo/pkg/a.go", src)]).is_empty());
    }

    #[test]
    fn ignores_method_that_writes_two_fields() {
        let src = "package pkg\n\ntype t struct{ n int; dirty bool }\n\n\
            func (x *t) setN(v int) { x.n = v; x.dirty = true }\n";
        assert!(findings(vec![("/repo/pkg/a.go", src)]).is_empty());
    }

    #[test]
    fn ignores_non_setter_names() {
        let src = "package pkg\n\ntype t struct{ n int }\n\nfunc (x *t) reset(v int) { x.n = v }\n\
            func (x *t) settle(v int) { x.n = v }\n";
        assert!(findings(vec![("/repo/pkg/a.go", src)]).is_empty());
    }

    #[test]
    fn ignores_test_files() {
        assert!(findings(vec![("/repo/pkg/a_test.go", TYPE)]).is_empty());
    }

    #[test]
    fn flags_positive_finding_with_location_type_and_field() {
        let found = findings(vec![("/repo/pkg/a.go", TYPE)]);
        assert_eq!(found[0].file, Some(PathBuf::from("/repo/pkg/a.go")));
        assert_eq!(found[0].line, Some(5));
        assert!(
            found[0].message.contains("pkg::t.setName"),
            "{}",
            found[0].message
        );
        assert!(found[0].message.contains("'name'"), "{}", found[0].message);
    }

    #[test]
    fn ignores_setter_called_from_constructor_and_elsewhere() {
        let src = format!(
            "{TYPE}\nfunc newT() *t {{ x := &t{{}}; x.setName(\"a\"); return x }}\n\
             func rename(x *t) {{ x.setName(\"b\") }}\n"
        );
        assert!(findings(vec![("/repo/pkg/a.go", &src)]).is_empty());
    }

    #[test]
    fn ignores_setter_called_from_func_literal_in_constructor() {
        let src = format!(
            "{TYPE}\nfunc newT() *t {{ x := &t{{}}; go func() {{ x.setName(\"a\") }}(); return x }}\n"
        );
        let found = findings(vec![("/repo/pkg/a.go", &src)]);
        assert!(
            found.iter().all(|f| !f.message.contains("never called")),
            "a call inside a closure must not read as 'never called': {found:?}"
        );
    }

    #[test]
    fn ignores_setter_called_via_unresolved_receiver() {
        let src = format!("{TYPE}\nfunc use(h holder) {{ h.inner.setName(\"a\") }}\n");
        assert!(findings(vec![("/repo/pkg/a.go", &src)]).is_empty());
    }

    #[test]
    fn ignores_setter_that_also_makes_calls() {
        let src = "package pkg\n\ntype t struct{ name string }\n\n\
            func (x *t) setName(v string) { x.name = v; x.notify() }\n\
            func (x *t) notify() {}\n";
        assert!(findings(vec![("/repo/pkg/a.go", src)]).is_empty());
    }

    #[test]
    fn ignores_setter_called_from_constructor_of_another_type() {
        let src = format!("{TYPE}\nfunc NewHandler(x *t) {{ x.setName(\"a\") }}\n");
        assert!(findings(vec![("/repo/pkg/a.go", &src)]).is_empty());
    }

    #[test]
    fn flags_exported_setter_in_package_ending_in_internal() {
        let src = "package pkg\n\ntype T struct{ name string }\n\nfunc (x *T) SetName(v string) { x.name = v }\n";
        assert_eq!(findings(vec![("/repo/a/internal/a.go", src)]).len(), 1);
    }

    #[test]
    fn ignores_setter_called_from_method_named_like_constructor() {
        let src = format!("{TYPE}\nfunc (x *t) NewThing() {{ x.setName(\"a\") }}\n");
        assert!(findings(vec![("/repo/pkg/a.go", &src)]).is_empty());
    }

    #[test]
    fn constructor_prefix_requires_word_boundary() {
        let src = format!("{TYPE}\nfunc newsletter(x *t) {{ x.setName(\"a\") }}\n");
        assert!(findings(vec![("/repo/pkg/a.go", &src)]).is_empty());
    }

    #[test]
    fn flags_each_unused_setter_in_line_order() {
        let src = "package pkg\n\ntype t struct{ a int; b int }\n\n\
            func (x *t) setA(v int) { x.a = v }\n\
            func (x *t) setB(v int) { x.b = v }\n";
        let found = findings(vec![("/repo/pkg/a.go", src)]);
        assert_eq!(found.len(), 2, "got: {found:?}");
        assert!(found[0].line < found[1].line);
    }

    #[test]
    fn same_named_setter_call_on_other_type_suppresses_by_name() {
        let src = "package pkg\n\ntype a struct{ n int }\ntype b struct{ n int }\n\n\
            func (x *a) setN(v int) { x.n = v }\n\
            func (x *b) setN(v int) { x.n = v }\n\
            func use(x *a) { x.setN(1) }\n";
        assert!(findings(vec![("/repo/pkg/a.go", src)]).is_empty());
    }

    #[test]
    fn setter_names_need_an_uppercase_after_the_prefix() {
        for name in ["reset", "settle", "set", "Set", "set_name", "setup"] {
            let src = format!(
                "package pkg\n\ntype t struct{{ n int }}\n\nfunc (x *t) {name}(v int) {{ x.n = v }}\n"
            );
            assert!(
                findings(vec![("/repo/pkg/a.go", &src)]).is_empty(),
                "{name}"
            );
        }
    }

    #[test]
    fn candidate_skips_setter_on_type_with_implements_edge() {
        let model = model_from(vec![("/repo/pkg/a.go", TYPE)]);
        let setter = model
            .packages
            .values()
            .flat_map(|p| &p.symbols)
            .find(|s| s.name == "setName")
            .unwrap();
        let root = &model.repo_root;
        assert!(is_candidate("pkg", setter, root, &HashSet::new()));
        let implementers: HashSet<&str> = [setter.id.strip_suffix(".setName").unwrap()].into();
        assert!(!is_candidate("pkg", setter, root, &implementers));
    }
}
