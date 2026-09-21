//! Typed `<Lang>Kind` enums, one per tree-sitter grammar in [`crate::checker::Language`],
//! generated at build time by `build.rs` from that grammar's own vendored
//! `node-types.json` (`codegen/node-types/*.json`). Prefer these over comparing
//! `node.kind()` to a hand-typed string literal: `GoKind::of(node) == GoKind::IfStatement`
//! catches a typo'd variant name at compile time, where `node.kind() == "if_statment"`
//! silently never matches. See each enum's `from_kind_str`/`of`/`as_str` — `Other` is the
//! catch-all for a kind string outside that grammar's vendored, named, concrete node set
//! (a synthetic `ERROR`/`MISSING` node, or an anonymous/punctuation token no checker in
//! this crate currently matches by kind anyway).

// Migration in progress (see the SDD plan for issue-92-adjacent node-kind typing) — not
// every generated enum/variant has a caller outside this module's own tests yet.
#![allow(dead_code, unused_imports)]

include!(concat!(env!("OUT_DIR"), "/go_kind.rs"));

pub mod typescript {
    include!(concat!(env!("OUT_DIR"), "/typescript_kind.rs"));
}
pub use typescript::TypeScriptKind;

pub mod tsx {
    include!(concat!(env!("OUT_DIR"), "/tsx_kind.rs"));
}
pub use tsx::TsxKind;

pub mod javascript {
    include!(concat!(env!("OUT_DIR"), "/javascript_kind.rs"));
}
pub use javascript::JavaScriptKind;

pub mod python {
    include!(concat!(env!("OUT_DIR"), "/python_kind.rs"));
}
pub use python::PythonKind;

pub mod java {
    include!(concat!(env!("OUT_DIR"), "/java_kind.rs"));
}
pub use java::JavaKind;

pub mod kotlin {
    include!(concat!(env!("OUT_DIR"), "/kotlin_kind.rs"));
}
pub use kotlin::KotlinKind;

pub mod rust {
    include!(concat!(env!("OUT_DIR"), "/rust_kind.rs"));
}
pub use rust::RustKind;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn go_kind_of_matches_a_real_parsed_node() {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_go::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse("package main\n", None).unwrap();
        assert_eq!(GoKind::of(tree.root_node()), GoKind::SourceFile);
    }

    #[test]
    fn from_kind_str_falls_back_to_other_for_an_unknown_string() {
        assert_eq!(GoKind::from_kind_str("not_a_real_kind"), GoKind::Other);
    }

    #[test]
    fn as_str_round_trips_through_from_kind_str() {
        assert_eq!(
            GoKind::from_kind_str(GoKind::IfStatement.as_str()),
            GoKind::IfStatement
        );
    }

    #[test]
    fn every_language_gets_a_generated_enum() {
        // One assertion per language is enough to prove each module actually compiled
        // and its enum is reachable — full coverage of every variant isn't the point of
        // this test (that's node-types.json's job); a missing/renamed module is what
        // this guards against.
        assert_eq!(GoKind::from_kind_str("identifier"), GoKind::Identifier);
        assert_eq!(
            TypeScriptKind::from_kind_str("identifier"),
            TypeScriptKind::Identifier
        );
        assert_eq!(TsxKind::from_kind_str("identifier"), TsxKind::Identifier);
        assert_eq!(
            JavaScriptKind::from_kind_str("identifier"),
            JavaScriptKind::Identifier
        );
        assert_eq!(
            PythonKind::from_kind_str("identifier"),
            PythonKind::Identifier
        );
        assert_eq!(JavaKind::from_kind_str("identifier"), JavaKind::Identifier);
        assert_eq!(
            KotlinKind::from_kind_str("identifier"),
            KotlinKind::Identifier
        );
        assert_eq!(RustKind::from_kind_str("identifier"), RustKind::Identifier);
    }
}
