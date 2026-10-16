//! Go profile: `TestXxx(t *testing.T)` and `FuzzXxx(f *testing.F)` functions in `*_test.go`.

use std::path::Path;

use crate::checker::{CheckContext, Language};
use crate::node_kind::GoKind;
use crate::tree_walk::walk_preorder;

use super::{
    AssertionCall, TestCodeProfile, TestFn, TestKind, make_test_fn, named_arg_count, node_text,
};

const ASSERTIONS: &[&str] = &[
    "t.Error*",
    "t.Fatal*",
    "t.Fail*",
    "assert.*",
    "require.*",
    "b.Error*",
    "b.Fatal*",
];
/// Go snapshot libraries: `cupaloy.SnapshotT`, `goldie.New(t).Assert...`.
const SNAPSHOT_MARKERS: &[&str] = &["cupaloy", "goldie"];
const EXEMPTION_MARKERS: &[&str] = &["t.Skip"];

pub(super) fn profile() -> TestCodeProfile {
    TestCodeProfile {
        language: Language::Go,
        assertion_names: ASSERTIONS,
        snapshot_markers: SNAPSHOT_MARKERS,
        exemption_markers: EXEMPTION_MARKERS,
        is_test_file: Some(is_test_file),
        collect,
    }
}

fn is_test_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.ends_with("_test.go"))
}

fn collect(ctx: &CheckContext) -> Vec<TestFn> {
    let Some(tree) = ctx.tree else {
        return Vec::new();
    };
    let profile = profile();
    let mut out = Vec::new();
    walk_preorder(tree.root_node(), &mut |node| {
        if GoKind::of(node) != GoKind::FunctionDeclaration {
            return true;
        }
        let name = node
            .child_by_field_name("name")
            .map_or("", |n| node_text(n, ctx.source));
        let params = node
            .child_by_field_name("parameters")
            .map_or("", |n| node_text(n, ctx.source));
        let fuzz = name.starts_with("Fuzz") && params.contains("testing.F");
        let test = name.starts_with("Test") && params.contains("testing.T");
        if fuzz || test {
            let mut f = make_test_fn(
                &profile,
                name,
                node,
                node,
                ctx.source,
                calls_in(node, ctx.source),
            );
            if fuzz {
                f.kind = TestKind::Fuzz;
            }
            out.push(f);
        }
        false
    });
    out
}

fn calls_in(root: tree_sitter::Node, source: &str) -> Vec<AssertionCall> {
    let mut calls = Vec::new();
    walk_preorder(root, &mut |node| {
        if GoKind::of(node) == GoKind::CallExpression
            && let Some(func) = node.child_by_field_name("function")
        {
            calls.push(AssertionCall {
                callee: node_text(func, source).to_string(),
                args: named_arg_count(node.child_by_field_name("arguments")),
            });
        }
        true
    });
    calls
}

#[cfg(test)]
mod tests {
    #![allow(non_snake_case)]
    use super::super::test_support::functions;
    use super::*;

    fn go(src: &str) -> Vec<TestFn> {
        functions(Language::Go, src)
    }

    #[test]
    fn snapshot_markers_should_IncludeCupaloyAndGoldie_When_GoProfileBuilt() {
        let p = profile();
        assert!(p.snapshot_markers.contains(&"cupaloy"));
        assert!(p.snapshot_markers.contains(&"goldie"));
    }

    #[test]
    fn test_functions_should_RequireTestingTParam_When_NameStartsWithTest() {
        let fns = go(
            "package p\nfunc TestA(t *testing.T) {}\nfunc TestHelper(x int) {}\nfunc helper(t *testing.T) {}\n",
        );
        assert_eq!(
            fns.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(),
            ["TestA"]
        );
    }

    #[test]
    fn test_functions_should_CollectSubtestCallsAndTagKinds_When_BodyUsesTRun() {
        let src = "package p\nfunc TestA(t *testing.T) { t.Run(\"x\", func(t *testing.T) { t.Fatalf(\"bad\") }) }\n\
                   func FuzzA(f *testing.F) { f.Fuzz(func(t *testing.T, s string) {}) }\n\
                   func TestSnap(t *testing.T) { cupaloy.SnapshotT(t, 1) }\n\
                   func TestIntegrationDB(t *testing.T) {}\n";
        let fns = go(src);
        let kinds: Vec<_> = fns.iter().map(|f| f.kind).collect();
        assert_eq!(
            kinds,
            [
                TestKind::Unit,
                TestKind::Fuzz,
                TestKind::Snapshot,
                TestKind::Integration
            ]
        );
        let calls = profile().assertion_calls(&fns[0]);
        assert_eq!(
            calls.iter().map(|c| c.callee.as_str()).collect::<Vec<_>>(),
            ["t.Fatalf"]
        );
    }

    #[test]
    fn is_exempt_should_ReturnTrue_When_TestSkips() {
        let fns = go("package p\nfunc TestA(t *testing.T) { t.Skip(\"later\") }\n");
        assert!(profile().is_exempt(&fns[0]));
    }

    #[test]
    fn assertion_calls_should_MatchPrefixes_When_TErrorfAndRequire() {
        let fns = go(
            "package p\nfunc TestA(t *testing.T) { t.Errorf(\"x\"); require.NoError(t, err); helper(t) }\n",
        );
        let calls = profile().assertion_calls(&fns[0]);
        assert_eq!(calls.len(), 2);
    }
}
