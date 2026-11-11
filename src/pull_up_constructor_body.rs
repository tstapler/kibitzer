//! Fowler's **Pull Up Constructor Body**: ≥2 direct sibling subtypes have constructors
//! with identical setup bodies. Same engine and conditions as `pull-up-method`
//! (`pull_up_common`), restricted to constructors — TS/Kotlin `constructor`, Java a
//! method named after its owning type. Go has no constructor keyword and never fires.

use crate::arch_model::ArchModel;
use crate::architecture_checks::{ArchFinding, ArchModelChecker};
use crate::config::ArchitectureConfig;
use crate::pull_up_common::{MemberKind, find_pull_up_candidates};

pub struct PullUpConstructorBodyChecker;

impl ArchModelChecker for PullUpConstructorBodyChecker {
    fn name(&self) -> &str {
        "pull-up-constructor-body"
    }

    fn needs_private_symbols(&self) -> bool {
        true
    }

    fn check(&self, model: &ArchModel, _config: &ArchitectureConfig) -> Vec<ArchFinding> {
        find_pull_up_candidates(model, MemberKind::Constructor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch_model::SymbolKind;
    use crate::hierarchy_checks::fixtures::{edge, model, sym};
    use crate::pull_up_common::test_support::scratch_dir;

    fn build(
        ext: &str,
        ctor_name: (&str, &str),
        src: (&str, &str),
        line: usize,
        end: usize,
    ) -> ArchModel {
        let dir = scratch_dir();
        let (fa, fb) = (dir.join(format!("A.{ext}")), dir.join(format!("B.{ext}")));
        std::fs::write(&fa, src.0).unwrap();
        std::fs::write(&fb, src.1).unwrap();
        let mut syms = vec![
            sym("pkg", "Base", SymbolKind::Type, None),
            sym("pkg", "A", SymbolKind::Type, None),
            sym("pkg", "B", SymbolKind::Type, None),
            sym("pkg", ctor_name.0, SymbolKind::Method, Some("A")),
            sym("pkg", ctor_name.1, SymbolKind::Method, Some("B")),
        ];
        for (i, f) in [(3, &fa), (4, &fb)] {
            syms[i].file = f.clone();
            syms[i].line = line;
            syms[i].end_line = Some(end);
        }
        for s in syms.iter_mut().take(3) {
            s.file = dir.join(format!("types.{ext}"));
        }
        let mut m = model(
            syms,
            vec![
                edge("pkg::A", "pkg::Base", true),
                edge("pkg::B", "pkg::Base", true),
            ],
        );
        m.repo_root = dir;
        m
    }

    fn run(m: &ArchModel) -> Vec<ArchFinding> {
        PullUpConstructorBodyChecker.check(m, &ArchitectureConfig::default())
    }

    #[test]
    fn fires_for_typescript_constructors() {
        let ts = "class X {\n  constructor() {\n    this.a = 1;\n    this.b = 2;\n  }\n}\n";
        let f = run(&build("ts", ("constructor", "constructor"), (ts, ts), 2, 5));
        assert_eq!(f.len(), 1, "{f:?}");
        assert!(f[0].message.contains("[pull-up-constructor-body]"));
    }

    #[test]
    fn fires_for_java_constructors_named_after_the_owner() {
        let a = "class A {\n  A() {\n    init();\n    this.x = 1;\n  }\n}\n";
        let b = "class B {\n  B() {\n    init();\n    this.x = 1;\n  }\n}\n";
        let f = run(&build("java", ("A", "B"), (a, b), 2, 5));
        assert_eq!(f.len(), 1, "{f:?}");
    }

    #[test]
    fn silent_when_constructor_bodies_differ_or_names_are_plain_methods() {
        let a = "class X {\n  constructor() {\n    this.a = 1;\n    this.b = 2;\n  }\n}\n";
        let b = "class X {\n  constructor() {\n    this.a = 1;\n    this.b = 3;\n  }\n}\n";
        assert!(run(&build("ts", ("constructor", "constructor"), (a, b), 2, 5)).is_empty());
        // Same bodies but methods named `run`: not constructors.
        assert!(run(&build("ts", ("run", "run"), (a, a), 2, 5)).is_empty());
    }
}
