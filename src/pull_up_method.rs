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
    use crate::hierarchy_checks::fixtures::{edge, model, scratch_dir, sym};

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
        // Move only the *types* under testdata/: the method files stay real and readable,
        // so the ignorable-file filter is the only thing that can explain the silence.
        for s in &mut m.packages.get_mut("pkg").unwrap().symbols {
            if s.kind == SymbolKind::Type {
                s.file = root.join("testdata").join("types.go");
            }
        }
        assert!(run(&m).is_empty());
        let mut control = build(GO_A, GO_B, true, false);
        assert_eq!(
            run(&control).len(),
            1,
            "same data fires without the testdata move"
        );
        control.type_edges.clear();
    }

    #[test]
    fn constructors_are_not_pull_up_method_candidates() {
        use crate::pull_up_common::{MemberKind, find_pull_up_candidates};
        let ts = "class A {\n  constructor() {\n    this.a = 1;\n    this.b = 2;\n  }\n}\n";
        let mut m = build(ts, ts, true, false);
        let dir = m.repo_root.clone();
        for s in &mut m.packages.get_mut("pkg").unwrap().symbols {
            if s.name == "Do" {
                s.name = "constructor".into();
                s.line = 2;
                s.end_line = Some(5);
                let f = dir.join(format!("{}.ts", s.parent.as_deref().unwrap()));
                std::fs::write(&f, ts).unwrap();
                s.file = f;
            }
            if s.kind == SymbolKind::Type {
                s.file = dir.join("types.ts");
            }
        }
        assert!(run(&m).is_empty());
        assert_eq!(
            find_pull_up_candidates(&m, MemberKind::Constructor).len(),
            1,
            "positive control: the same data is a constructor-body candidate"
        );
    }

    /// Two TypeScript siblings of `pkg::Base` (`pkg::A` and `<second_pkg>::<second>`) with a
    /// `Do` method whose body is `body_line` plus a fixed second line. Non-Go on purpose: the
    /// Go package guard must not be what explains a silent result.
    fn ts_siblings(second_pkg: &str, second: &str, body_line: &str) -> ArchModel {
        use crate::hierarchy_checks::fixtures::model_pkgs;
        let dir = scratch_dir();
        let src = format!("class X {{\n  Do() {{\n    {body_line}\n    this.log();\n  }}\n}}\n");
        let ty = |pkg: &str, name: &str| {
            let mut t = sym(pkg, name, SymbolKind::Type, None);
            t.file = dir.join("types.ts");
            t
        };
        let method = |pkg: &str, owner: &str, file: &str| {
            let mut m = sym(pkg, "Do", SymbolKind::Method, Some(owner));
            std::fs::write(dir.join(file), &src).unwrap();
            m.file = dir.join(file);
            m.line = 2;
            m.end_line = Some(5);
            m
        };
        let mut base_run = sym("pkg", "log", SymbolKind::Method, Some("Base"));
        base_run.file = dir.join("types.ts");
        let mut pkg_syms = vec![
            ty("pkg", "Base"),
            ty("pkg", "A"),
            method("pkg", "A", "a.ts"),
            base_run,
        ];
        let mut m = if second_pkg == "pkg" {
            pkg_syms.extend([ty("pkg", second), method("pkg", second, "b.ts")]);
            model_pkgs(vec![("pkg", pkg_syms)], vec![])
        } else {
            let other = vec![ty(second_pkg, second), method(second_pkg, second, "b.ts")];
            model_pkgs(vec![("pkg", pkg_syms), (second_pkg, other)], vec![])
        };
        m.type_edges = vec![
            edge("pkg::A", "pkg::Base", true),
            edge(&format!("{second_pkg}::{second}"), "pkg::Base", true),
        ];
        m.repo_root = dir;
        m
    }

    #[test]
    fn same_named_siblings_in_different_packages_are_copies_not_siblings() {
        let body = "this.n++;";
        assert_eq!(
            run(&ts_siblings("pkg2", "C", body)).len(),
            1,
            "positive control: differently named siblings fire"
        );
        assert!(run(&ts_siblings("pkg2", "A", body)).is_empty());
    }

    #[test]
    fn a_subtype_named_like_its_supertype_is_not_a_sibling() {
        let body = "this.n++;";
        assert_eq!(run(&ts_siblings("pkg2", "C", body)).len(), 1, "control");
        assert!(run(&ts_siblings("pkg2", "Base", body)).is_empty());
    }

    #[test]
    fn bodies_naming_an_upper_snake_constant_are_skipped() {
        assert_eq!(
            run(&ts_siblings("pkg", "B", "this.n++;")).len(),
            1,
            "control"
        );
        assert!(run(&ts_siblings("pkg", "B", "use(DEFAULT_URL);")).is_empty());
    }
}
