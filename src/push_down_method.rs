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
//! moved down; an unreadable call-site line is accepted rather than blocking.

use std::collections::{BTreeSet, HashMap, HashSet};

use crate::arch_model::{ArchModel, CallEdge};
use crate::architecture_checks::{ArchFinding, ArchModelChecker};
use crate::config::ArchitectureConfig;
use crate::hierarchy_checks::{
    is_ignorable_file, methods_of, owner_type_id, resolved_extends, subtree, subtypes_by_super,
    type_index,
};
use crate::pull_up_common::{go_receiver_name, is_constructor};

pub struct PushDownMethodChecker;

impl ArchModelChecker for PushDownMethodChecker {
    fn name(&self) -> &str {
        "push-down-method"
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

        let symbols: HashMap<&str, &crate::arch_model::SymbolNode> = model
            .packages
            .values()
            .flat_map(|p| &p.symbols)
            .map(|s| (s.id.as_str(), s))
            .collect();

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
                    if !called_through_own_receiver(&symbols, e, &m.name) {
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
        findings.sort_by(|a, b| (&a.file, a.line).cmp(&(&b.file, b.line)));
        findings
    }
}

/// True unless the call-site line provably calls `name` on something other than the
/// calling method's own receiver.
fn called_through_own_receiver(
    symbols: &HashMap<&str, &crate::arch_model::SymbolNode>,
    edge: &CallEdge,
    name: &str,
) -> bool {
    let Some(line) = std::fs::read_to_string(&edge.file).ok().and_then(|t| {
        t.lines()
            .nth(edge.line.saturating_sub(1))
            .map(str::to_string)
    }) else {
        return true;
    };
    let Some(caller) = symbols.get(edge.from.as_str()) else {
        return true;
    };
    let is_go = caller.file.extension().is_some_and(|e| e == "go");
    let recv = if is_go {
        std::fs::read_to_string(&caller.file)
            .ok()
            .and_then(|t| {
                t.lines()
                    .nth(caller.line.saturating_sub(1))
                    .and_then(|l| go_receiver_name(l).map(str::to_string))
            })
            .unwrap_or_default()
    } else {
        "this".to_string()
    };
    if recv.is_empty() {
        return true;
    }
    let needle = format!(".{name}(");
    let mut seen_call = false;
    for (idx, _) in line.match_indices(&needle) {
        seen_call = true;
        let chain: String = line[..idx]
            .chars()
            .rev()
            .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '.')
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        if chain == recv || chain.starts_with(&format!("{recv}.")) {
            return true;
        }
    }
    !seen_call
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch_model::SymbolKind;
    use crate::hierarchy_checks::fixtures::{edge, model, sym};
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
        m.packages
            .get_mut("pkg")
            .unwrap()
            .symbols
            .push(sym("pkg", "A2", SymbolKind::Type, None));
        m.type_edges.push(edge("pkg::A2", "pkg::A", true));
        m.call_edges = vec![call("pkg::A2.Go", "pkg::Base.Shared")];
        assert_eq!(run(&m).len(), 1);
    }

    #[test]
    fn a_call_on_another_value_of_the_supertype_is_not_evidence() {
        use crate::pull_up_common::test_support::scratch_dir;
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
}
