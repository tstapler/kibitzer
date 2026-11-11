//! Fowler's **Pull Up Method**: ≥2 direct sibling subtypes declare a same-named,
//! same-signature method with an identical (whitespace/comment/Go-receiver-normalised)
//! body, and the supertype doesn't declare it. Body text is sliced from source via
//! `SymbolNode::line..=end_line`; see `pull_up_common` for the shared engine and skips.

use crate::arch_model::ArchModel;
use crate::architecture_checks::{ArchFinding, ArchModelChecker};
use crate::config::ArchitectureConfig;
use crate::pull_up_common::{MemberKind, find_pull_up_candidates};

pub struct PullUpMethodChecker;

impl ArchModelChecker for PullUpMethodChecker {
    fn name(&self) -> &str {
        "pull-up-method"
    }

    fn needs_private_symbols(&self) -> bool {
        true
    }

    fn check(&self, model: &ArchModel, _config: &ArchitectureConfig) -> Vec<ArchFinding> {
        find_pull_up_candidates(model, MemberKind::Method)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch_model::SymbolKind;
    use crate::hierarchy_checks::fixtures::{edge, model, sym};
    use crate::pull_up_common::test_support::scratch_dir;

    const GO_A: &str = "package pkg\n\nfunc (a *A) Do() {\n\ta.n++\n\ta.log()\n}\n";
    const GO_B: &str = "package pkg\n\nfunc (b *B) Do() {\n\tb.n++\n\tb.log()\n}\n";

    fn build(src_a: &str, src_b: &str, with_end: bool, base_has_method: bool) -> ArchModel {
        let dir = scratch_dir();
        let (fa, fb) = (dir.join("a.go"), dir.join("b.go"));
        std::fs::write(&fa, src_a).unwrap();
        std::fs::write(&fb, src_b).unwrap();
        let mut syms = vec![
            sym("pkg", "Base", SymbolKind::Type, None),
            sym("pkg", "A", SymbolKind::Type, None),
            sym("pkg", "B", SymbolKind::Type, None),
            sym("pkg", "Do", SymbolKind::Method, Some("A")),
            sym("pkg", "Do", SymbolKind::Method, Some("B")),
        ];
        for (i, f) in [(3, &fa), (4, &fb)] {
            syms[i].file = f.clone();
            syms[i].line = 3;
            syms[i].end_line = with_end.then_some(6);
        }
        for s in syms.iter_mut().take(3) {
            s.file = dir.join("types.go");
        }
        let mut log = sym("pkg", "log", SymbolKind::Method, Some("Base"));
        log.file = dir.join("types.go");
        syms.push(log);
        if base_has_method {
            let mut b = sym("pkg", "Do", SymbolKind::Method, Some("Base"));
            b.file = dir.join("types.go");
            syms.push(b);
        }
        let mut m = model(
            syms,
            vec![
                edge("pkg::A", "pkg::Base", true),
                edge("pkg::B", "pkg::Base", true),
            ],
        );
        m.repo_root = dir;
        m.struct_field_names
            .insert("pkg::Base".into(), ["n".to_string()].into());
        m
    }

    fn run(m: &ArchModel) -> Vec<ArchFinding> {
        PullUpMethodChecker.check(m, &ArchitectureConfig::default())
    }

    #[test]
    fn fires_on_identical_sibling_bodies() {
        let f = run(&build(GO_A, GO_B, true, false));
        assert_eq!(f.len(), 1, "{f:?}");
        assert!(f[0].message.contains("[pull-up-method]"));
    }

    #[test]
    fn go_body_using_an_outer_only_member_is_not_pullable() {
        let a = "package pkg\n\nfunc (a *A) Do() {\n\ta.Opts.mark()\n\ta.log()\n}\n";
        let b = "package pkg\n\nfunc (b *B) Do() {\n\tb.Opts.mark()\n\tb.log()\n}\n";
        assert!(run(&build(a, b, true, false)).is_empty());
    }

    #[test]
    fn silent_when_only_a_minority_of_subtypes_share_it() {
        let mut m = build(GO_A, GO_B, true, false);
        for n in ["C", "D"] {
            m.packages
                .get_mut("pkg")
                .unwrap()
                .symbols
                .push(sym("pkg", n, SymbolKind::Type, None));
            m.type_edges
                .push(edge(&format!("pkg::{n}"), "pkg::Base", true));
        }
        assert!(run(&m).is_empty(), "2 of 4 subtypes is not a majority");
    }

    #[test]
    fn silent_when_a_member_is_nested_in_another_function() {
        let mut m = build(GO_A, GO_B, true, false);
        let pkg = m.packages.get_mut("pkg").unwrap();
        let file = pkg
            .symbols
            .iter()
            .find(|s| s.id == "pkg::B.Do")
            .unwrap()
            .file
            .clone();
        let mut outer = sym("pkg", "Outer", SymbolKind::Method, Some("B"));
        outer.file = file;
        outer.line = 1;
        outer.end_line = Some(20);
        pkg.symbols.push(outer);
        assert!(
            run(&m).is_empty(),
            "B.Do looks like an anonymous-class member"
        );
    }

    #[test]
    fn skips_without_end_line() {
        assert!(run(&build(GO_A, GO_B, false, false)).is_empty());
    }

    #[test]
    fn silent_when_bodies_differ_or_supertype_declares_it() {
        let other = "package pkg\n\nfunc (b *B) Do() {\n\tb.n--\n\tb.log()\n}\n";
        assert!(run(&build(GO_A, other, true, false)).is_empty());
        assert!(run(&build(GO_A, GO_B, true, true)).is_empty());
    }

    #[test]
    fn silent_for_unresolved_edges_and_ignorable_files() {
        let mut m = build(GO_A, GO_B, true, false);
        for e in &mut m.type_edges {
            e.resolved = false;
        }
        assert!(run(&m).is_empty());

        let mut m = build(GO_A, GO_B, true, false);
        let root = m.repo_root.clone();
        for s in &mut m.packages.get_mut("pkg").unwrap().symbols {
            if s.name == "Do" {
                s.file = root.join("testdata").join("a.go");
            }
        }
        assert!(run(&m).is_empty());
    }

    #[test]
    fn constructors_are_not_pull_up_method_candidates() {
        let ts = "class A {\n  constructor() {\n    this.a = 1;\n    this.b = 2;\n  }\n}\n";
        let mut m = build(ts, ts, true, false);
        for s in &mut m.packages.get_mut("pkg").unwrap().symbols {
            if s.name == "Do" {
                s.name = "constructor".into();
            }
        }
        assert!(run(&m).is_empty());
    }
}
