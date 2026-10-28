//! Python profile: `test_*` functions and `unittest.TestCase` / `Test*` class methods.

use std::path::Path;

use crate::checker::{CheckContext, Language};
use crate::node_kind::PythonKind;
use crate::tree_walk::walk_preorder;

use super::{AssertionCall, TestCodeProfile, TestFn, make_test_fn, named_arg_count, node_text};

const ASSERTIONS: &[&str] = &[
    "assert",
    "self.assert*",
    "self.fail",
    "pytest.raises",
    "pytest.warns",
    "pytest.fail",
    "mock.assert_*",
];
const SNAPSHOT_MARKERS: &[&str] = &["assert_match_snapshot", "snapshot", "syrupy"];
const EXEMPTION_MARKERS: &[&str] = &[
    "pytest.skip",
    "@pytest.mark.skip",
    "@unittest.skip",
    "self.skipTest",
];

pub(super) fn profile() -> TestCodeProfile {
    TestCodeProfile {
        language: Language::Python,
        assertion_names: ASSERTIONS,
        snapshot_markers: SNAPSHOT_MARKERS,
        exemption_markers: EXEMPTION_MARKERS,
        is_test_file: Some(is_test_file),
        collect,
    }
}

fn is_test_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    name.ends_with(".py") && (name.starts_with("test_") || name.ends_with("_test.py"))
}

fn collect(ctx: &CheckContext) -> Vec<TestFn> {
    let Some(tree) = ctx.tree else {
        return Vec::new();
    };
    let profile = profile();
    let mut out = Vec::new();
    walk_preorder(tree.root_node(), &mut |node| {
        if PythonKind::of(node) != PythonKind::FunctionDefinition {
            return true;
        }
        let name = node
            .child_by_field_name("name")
            .map_or("", |n| node_text(n, ctx.source));
        if name.starts_with("test") {
            // A decorated test's span includes its decorators so skip markers are visible.
            let span = node
                .parent()
                .filter(|p| PythonKind::of(*p) == PythonKind::DecoratedDefinition)
                .unwrap_or(node);
            out.push(make_test_fn(
                &profile,
                name,
                node,
                span,
                ctx.source,
                calls_in(node, ctx.source),
            ));
        }
        false
    });
    out
}

fn calls_in(root: tree_sitter::Node, source: &str) -> Vec<AssertionCall> {
    let mut calls = Vec::new();
    walk_preorder(root, &mut |node| {
        match PythonKind::of(node) {
            PythonKind::Call => {
                if let Some(func) = node.child_by_field_name("function") {
                    calls.push(AssertionCall {
                        callee: node_text(func, source).to_string(),
                        args: named_arg_count(node.child_by_field_name("arguments")),
                    });
                }
            }
            PythonKind::AssertStatement => calls.push(AssertionCall {
                callee: "assert".to_string(),
                args: node.named_child_count(),
            }),
            _ => {}
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

    fn py(src: &str) -> Vec<TestFn> {
        functions(Language::Python, src)
    }

    #[test]
    fn is_test_file_should_MatchPrefixAndSuffix_When_PythonNames() {
        assert!(is_test_file(Path::new("tests/test_a.py")));
        assert!(is_test_file(Path::new("a_test.py")));
        assert!(!is_test_file(Path::new("conftest.py")));
        assert!(!is_test_file(Path::new("a.py")));
    }

    #[test]
    fn test_functions_should_FindFunctionsAndClassMethods_When_PythonSource() {
        let src = "def test_a():\n    assert f() == 1\n\ndef helper():\n    pass\n\nclass TestK(unittest.TestCase):\n    def test_b(self):\n        self.assertEqual(1, f())\n        with pytest.raises(ValueError):\n            g()\n";
        let fns = py(src);
        assert_eq!(
            fns.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(),
            ["test_a", "test_b"]
        );
        assert_eq!(fns[0].line, 1);
        let p = profile();
        assert_eq!(
            p.assertion_calls(&fns[0]),
            vec![AssertionCall {
                callee: "assert".into(),
                args: 1
            }]
        );
        let b: Vec<_> = p
            .assertion_calls(&fns[1])
            .into_iter()
            .map(|c| c.callee)
            .collect();
        assert_eq!(b, ["self.assertEqual", "pytest.raises"]);
    }

    #[test]
    fn assertion_calls_should_ReturnEmpty_When_BodyOnlyCallsParse() {
        let fns = py("def test_a():\n    parse('x')\n");
        assert!(profile().assertion_calls(&fns[0]).is_empty());
    }

    #[test]
    fn is_exempt_should_SeeDecorators_When_TestIsSkipped() {
        let fns = py("@pytest.mark.skip\ndef test_a():\n    pass\n");
        assert_eq!(fns[0].line, 2);
        assert!(profile().is_exempt(&fns[0]));
    }
}
