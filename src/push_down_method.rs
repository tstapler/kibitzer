//! Fowler's **Push Down Method**: a supertype method whose only resolved callers are
//! methods of exactly one direct subtype (or that subtype's descendants) belongs on that
//! subtype. Needs `call_edges` (Go/TS/JS only), so other languages never fire — "no
//! resolved callers" is "no evidence", not "unused", and is skipped.
//!
//! Conservative gates: the supertype needs ≥2 direct subtypes (with one, it's
//! `collapse-hierarchy` territory); a caller on the supertype itself or outside the
//! hierarchy (including free functions) blocks the finding, since moving the method down
//! would break it; a method a subtype also declares (an override) or that is used as a
//! value is skipped; only exported methods are considered. A call counts as evidence only
//! when its source line goes through the calling method's own receiver (`this.m()` /
//! `recv.m()`), since `snap.m()` on another value of the supertype would break if `m`
//! moved down; an unreadable call-site line is accepted rather than blocking. Every
//! `.m(` on the line must go through that receiver.

use std::collections::{BTreeSet, HashMap, HashSet};

use crate::arch_model::{ArchModel, CallEdge, SymbolNode};
use crate::architecture_checks::{ArchFinding, ArchModelChecker};
use crate::config::ArchitectureConfig;
use crate::hierarchy_checks::{
    SourceCache, go_receiver_ident_from_line, is_constructor, is_ignorable_file, methods_of,
    owner_type_id, resolved_extends, subtree, subtypes_by_super, type_index,
};

pub struct PushDownMethodChecker;

impl ArchModelChecker for PushDownMethodChecker {
    fn name(&self) -> &str {
        "push-down-method"
    }

    /// Callers and subtypes are frequently unexported; without them the receiver check below
    /// cannot find the caller and the hierarchy is missing its private members.
    fn needs_private_symbols(&self) -> bool {
        true
    }

    fn check(&self, model: &ArchModel, _config: &ArchitectureConfig) -> Vec<ArchFinding> {
        let types = type_index(model);
        let pairs = resolved_extends(model, &types);
        let children = subtypes_by_super(&pairs);

        let mut callers: HashMap<&str, Vec<&CallEdge>> = HashMap::new();
        for e in model
            .call_edges
            .iter()
            .filter(|e| e.resolved && !is_ignorable_file(model, &e.file))
        {
            callers.entry(e.to.as_str()).or_default().push(e);
        }

        let symbols: HashMap<&str, &SymbolNode> = model
            .packages
            .values()
            .flat_map(|p| &p.symbols)
            .map(|s| (s.id.as_str(), s))
            .collect();

        let mut sources = SourceCache::default();
        let mut findings = Vec::new();
        for (&sup_id, subs) in &children {
            let (pkg, sup) = types[sup_id];
            if subs.len() < 2 || is_ignorable_file(model, &sup.file) {
                continue;
            }
            let subtrees: Vec<HashSet<&str>> = subs.iter().map(|s| subtree(s, &children)).collect();
            let overridden: HashSet<&str> = subs
                .iter()
                .flat_map(|s| {
                    let (spkg, st) = types[s];
                    methods_of(model, spkg, &st.name)
                })
                .map(|m| m.name.as_str())
                .collect();

            for m in methods_of(model, pkg, &sup.name) {
                // Private/unexported methods are out of scope: only an exported method's
                // single-subtype usage is surprising.
                if !m.exported
                    || m.used_as_value
                    || is_constructor(m)
                    || overridden.contains(m.name.as_str())
                {
                    continue;
                }
                let Some(edges) = callers.get(m.id.as_str()) else {
                    continue;
                };
                let mut owning = BTreeSet::new();
                let all_in_a_subtree = edges.iter().all(|e| {
                    let Some(owner) = owner_type_id(&e.from) else {
                        return false;
                    };
                    if !called_through_own_receiver(model, &mut sources, &symbols, e, &m.name) {
                        return false;
                    }
                    match subtrees.iter().position(|t| t.contains(owner)) {
                        Some(i) => {
                            owning.insert(i);
                            true
                        }
                        None => false,
                    }
                });
                if !all_in_a_subtree || owning.len() != 1 {
                    continue;
                }
                let only = subs[*owning.iter().next().expect("len checked")];
                findings.push(ArchFinding {
                    file: Some(m.file.clone()),
                    line: Some(m.line),
                    message: format!(
                        "[push-down-method] {} on {sup_id} is only called from methods of \
                         {only} (and not from {sup_id}, its other subtypes, or elsewhere) — \
                         consider moving it down (Fowler, Push Down Method)",
                        m.id
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

/// True unless the call-site line provably calls `name` on something other than the
/// calling method's own receiver. Every `.name(` on the line must go through it.
fn called_through_own_receiver(
    model: &ArchModel,
    sources: &mut SourceCache,
    symbols: &HashMap<&str, &SymbolNode>,
    edge: &CallEdge,
    name: &str,
) -> bool {
    // An unmodeled caller cannot be verified, so it blocks the finding.
    let Some(caller) = symbols.get(edge.from.as_str()) else {
        return false;
    };
    let Some(line) = sources
        .line(model, &edge.file, edge.line)
        .map(str::to_string)
    else {
        return true;
    };
    let recv = if caller.file.extension().is_some_and(|e| e == "go") {
        sources
            .line(model, &caller.file, caller.line)
            .and_then(go_receiver_ident_from_line)
            .unwrap_or_default()
            .to_string()
    } else {
        "this".to_string()
    };
    if recv.is_empty() {
        return true;
    }
    let needle = format!(".{name}(");
    line.match_indices(&needle).all(|(idx, _)| {
        let chain: String = line[..idx]
            .chars()
            .rev()
            .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '.')
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        chain == recv || chain.starts_with(&format!("{recv}."))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch_model::SymbolKind;
    use crate::hierarchy_checks::fixtures::{edge, model, scratch_dir, sym};
    use std::path::PathBuf;

    fn call(from: &str, to: &str) -> CallEdge {
        CallEdge {
            from: from.into(),
            to: to.into(),
            resolved: true,
            file: PathBuf::from("/repo/pkg/a.go"),
            line: 9,
        }
    }

    fn hierarchy() -> ArchModel {
        let mut syms = vec![
            sym("pkg", "Base", SymbolKind::Type, None),
            sym("pkg", "A", SymbolKind::Type, None),
            sym("pkg", "B", SymbolKind::Type, None),
            sym("pkg", "Shared", SymbolKind::Method, Some("Base")),
            sym("pkg", "RunA", SymbolKind::Method, Some("A")),
            sym("pkg", "RunB", SymbolKind::Method, Some("B")),
        ];
        syms[3].exported = true;
        model(
            syms,
            vec![
                edge("pkg::A", "pkg::Base", true),
                edge("pkg::B", "pkg::Base", true),
            ],
        )
    }

    fn run(m: &ArchModel) -> Vec<ArchFinding> {
        PushDownMethodChecker.check(m, &ArchitectureConfig::default())
    }

    #[test]
    fn fires_when_only_one_subtype_calls_it() {
        let mut m = hierarchy();
        m.call_edges = vec![call("pkg::A.RunA", "pkg::Base.Shared")];
        let f = run(&m);
        assert_eq!(f.len(), 1, "{f:?}");
        assert!(f[0].message.contains("pkg::A"), "{}", f[0].message);
    }

    #[test]
    fn silent_without_resolved_callers() {
        let mut m = hierarchy();
        assert!(run(&m).is_empty());
        m.call_edges = vec![CallEdge {
            resolved: false,
            ..call("pkg::A.RunA", "pkg::Base.Shared")
        }];
        assert!(run(&m).is_empty());
    }

    #[test]
    fn silent_when_two_subtypes_call_it() {
        let mut m = hierarchy();
        m.call_edges = vec![
            call("pkg::A.RunA", "pkg::Base.Shared"),
            call("pkg::B.RunB", "pkg::Base.Shared"),
        ];
        assert!(run(&m).is_empty());
    }

    #[test]
    fn silent_with_external_or_supertype_caller() {
        let mut m = hierarchy();
        m.call_edges = vec![
            call("pkg::A.RunA", "pkg::Base.Shared"),
            call("pkg::Other.Go", "pkg::Base.Shared"),
        ];
        assert!(run(&m).is_empty());
        m.call_edges = vec![
            call("pkg::A.RunA", "pkg::Base.Shared"),
            call("pkg::freeFn", "pkg::Base.Shared"),
        ];
        assert!(run(&m).is_empty());
        m.call_edges = vec![
            call("pkg::A.RunA", "pkg::Base.Shared"),
            call("pkg::Base.Own", "pkg::Base.Shared"),
        ];
        assert!(run(&m).is_empty());
    }

    #[test]
    fn silent_when_a_subtype_overrides_or_edge_unresolved_or_test_file() {
        let mut m = hierarchy();
        m.call_edges = vec![call("pkg::A.RunA", "pkg::Base.Shared")];
        let mut ovr = sym("pkg", "Shared", SymbolKind::Method, Some("B"));
        ovr.exported = true;
        m.packages.get_mut("pkg").unwrap().symbols.push(ovr);
        assert!(run(&m).is_empty());

        let mut m = hierarchy();
        m.call_edges = vec![call("pkg::A.RunA", "pkg::Base.Shared")];
        for e in &mut m.type_edges {
            e.resolved = false;
        }
        assert!(run(&m).is_empty());

        let mut m = hierarchy();
        m.call_edges = vec![call("pkg::A.RunA", "pkg::Base.Shared")];
        for s in &mut m.packages.get_mut("pkg").unwrap().symbols {
            s.file = PathBuf::from("/repo/pkg/testdata/a.go");
        }
        assert!(run(&m).is_empty());
    }

    #[test]
    fn caller_in_a_grandchild_counts_for_its_direct_subtype() {
        let mut m = hierarchy();
        m.packages.get_mut("pkg").unwrap().symbols.extend([
            sym("pkg", "A2", SymbolKind::Type, None),
            sym("pkg", "Go", SymbolKind::Method, Some("A2")),
        ]);
        m.type_edges.push(edge("pkg::A2", "pkg::A", true));
        m.call_edges = vec![call("pkg::A2.Go", "pkg::Base.Shared")];
        assert_eq!(run(&m).len(), 1);
    }

    #[test]
    fn a_call_on_another_value_of_the_supertype_is_not_evidence() {
        let dir = scratch_dir();
        let f = dir.join("a.go");
        std::fs::write(
            &f,
            "package pkg\n\nfunc (a *A) RunA() int {\n\tsnap := a.Snapshot()\n\treturn snap.Shared()\n}\n",
        )
        .unwrap();
        let mut m = hierarchy();
        for s in &mut m.packages.get_mut("pkg").unwrap().symbols {
            if s.id == "pkg::A.RunA" {
                s.file = f.clone();
                s.line = 3;
            }
        }
        let mut e = call("pkg::A.RunA", "pkg::Base.Shared");
        e.file = f.clone();
        e.line = 5;
        m.call_edges = vec![e.clone()];
        assert!(
            run(&m).is_empty(),
            "snap.Shared() is not a call on A's own receiver"
        );
        std::fs::write(
            &f,
            "package pkg\n\nfunc (a *A) RunA() int {\n\tx := 1\n\treturn a.Shared() + x\n}\n",
        )
        .unwrap();
        assert_eq!(run(&m).len(), 1, "a.Shared() goes through the receiver");
    }

    fn call_in(from: &str, file: &str) -> CallEdge {
        CallEdge {
            file: PathBuf::from(file),
            ..call(from, "pkg::Base.Shared")
        }
    }

    #[test]
    fn calls_from_ignorable_files_are_not_evidence() {
        for file in ["/repo/pkg/testdata/a.go", "/repo/pkg/foo_test.go"] {
            let mut m = hierarchy();
            m.call_edges = vec![call_in("pkg::A.RunA", file)];
            assert!(run(&m).is_empty(), "sole edge in {file} is no evidence");

            // The B edge lives in an ignorable file, so only A's normal edge counts.
            m.call_edges = vec![
                call("pkg::A.RunA", "pkg::Base.Shared"),
                call_in("pkg::B.RunB", file),
            ];
            assert_eq!(run(&m).len(), 1, "control: ignorable {file} edge dropped");
        }
    }

    #[test]
    fn single_subtype_exported_and_value_gates() {
        let with_call = |mut m: ArchModel| {
            m.call_edges = vec![call("pkg::A.RunA", "pkg::Base.Shared")];
            m
        };
        assert_eq!(run(&with_call(hierarchy())).len(), 1, "control");

        let mut m = with_call(hierarchy());
        m.packages
            .get_mut("pkg")
            .unwrap()
            .symbols
            .retain(|s| s.id != "pkg::B" && s.id != "pkg::B.RunB");
        m.type_edges.retain(|e| e.from != "pkg::B");
        assert!(run(&m).is_empty(), "one subtype is collapse-hierarchy turf");

        let mut m = with_call(hierarchy());
        m.packages.get_mut("pkg").unwrap().symbols[3].exported = false;
        assert!(run(&m).is_empty(), "unexported method");

        let mut m = with_call(hierarchy());
        m.packages.get_mut("pkg").unwrap().symbols[3].used_as_value = true;
        assert!(run(&m).is_empty(), "used as a value");
    }

    #[test]
    fn constructors_are_never_pushed_down() {
        let build = |name: &str| {
            let mut m = hierarchy();
            let s = &mut m.packages.get_mut("pkg").unwrap().symbols[3];
            s.name = name.into();
            s.id = format!("pkg::Base.{name}");
            m.call_edges = vec![call("pkg::A.RunA", &format!("pkg::Base.{name}"))];
            m
        };
        assert_eq!(run(&build("init")).len(), 1, "control: an ordinary name");
        assert!(run(&build("constructor")).is_empty());
    }

    #[test]
    fn a_caller_missing_from_the_model_blocks() {
        let mut m = hierarchy();
        m.call_edges = vec![call("pkg::A.RunA", "pkg::Base.Shared")];
        assert_eq!(run(&m).len(), 1, "control");
        m.call_edges = vec![call("pkg::A.Ghost", "pkg::Base.Shared")];
        assert!(run(&m).is_empty(), "A.Ghost is unmodeled");
    }

    fn ts_call_model(src: &str, relative: bool) -> ArchModel {
        let dir = scratch_dir();
        let f = dir.join("a.ts");
        std::fs::write(&f, src).unwrap();
        let mut m = hierarchy();
        m.repo_root = dir;
        let file = if relative { PathBuf::from("a.ts") } else { f };
        for s in &mut m.packages.get_mut("pkg").unwrap().symbols {
            if s.id == "pkg::A.RunA" {
                s.file = file.clone();
            }
        }
        m.call_edges = vec![CallEdge {
            file,
            line: 1,
            ..call("pkg::A.RunA", "pkg::Base.Shared")
        }];
        m
    }

    #[test]
    fn non_go_callers_must_go_through_this() {
        assert_eq!(run(&ts_call_model("this.Shared();\n", false)).len(), 1);
        assert!(run(&ts_call_model("other.Shared();\n", false)).is_empty());
    }

    #[test]
    fn every_call_on_the_line_must_use_the_own_receiver() {
        let both = ts_call_model("this.Shared(); this.Shared();\n", false);
        assert_eq!(run(&both).len(), 1, "control");
        let mixed = ts_call_model("this.Shared(); other.Shared();\n", false);
        assert!(run(&mixed).is_empty());
    }

    #[test]
    fn relative_call_sites_resolve_against_the_repo_root() {
        assert_eq!(run(&ts_call_model("this.Shared();\n", true)).len(), 1);
        assert!(
            run(&ts_call_model("other.Shared();\n", true)).is_empty(),
            "an unresolved relative path would be accepted as unreadable"
        );
    }
}
