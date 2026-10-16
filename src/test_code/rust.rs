//! Rust profile: per-function only. `#[test]` functions live inline (`#[cfg(test)] mod tests`),
//! so there is no test-file predicate.

use crate::checker::{CheckContext, Language};
use crate::node_kind::RustKind;
use crate::tree_walk::walk_preorder;

use super::{AssertionCall, TestCodeProfile, TestFn, make_test_fn, node_text};

const ASSERTIONS: &[&str] = &[
    "assert!",
    "assert_eq!",
    "assert_ne!",
    "assert_*!",
    "debug_assert*",
    "panic!",
];
const SNAPSHOT_MARKERS: &[&str] = &[
    "assert_snapshot!",
    "assert_debug_snapshot!",
    "assert_yaml_snapshot!",
    "expect_test::expect",
    "expect![",
];
const EXEMPTION_MARKERS: &[&str] = &["#[ignore", "#[should_panic"];

pub(super) fn profile() -> TestCodeProfile {
    TestCodeProfile {
        language: Language::Rust,
        assertion_names: ASSERTIONS,
        snapshot_markers: SNAPSHOT_MARKERS,
        exemption_markers: EXEMPTION_MARKERS,
        is_test_file: None,
        collect,
    }
}

fn collect(ctx: &CheckContext) -> Vec<TestFn> {
    let Some(tree) = ctx.tree else {
        return Vec::new();
    };
    let profile = profile();
    let mut out = Vec::new();
    walk_preorder(tree.root_node(), &mut |node| {
        if RustKind::of(node) != RustKind::FunctionItem {
            return true;
        }
        let attrs = leading_attributes(node, ctx.source);
        if attrs.iter().any(|a| is_test_attribute(a)) {
            let name = node
                .child_by_field_name("name")
                .map_or("", |n| node_text(n, ctx.source));
            let mut f = make_test_fn(
                &profile,
                name,
                node,
                node,
                ctx.source,
                calls_in(node, ctx.source),
            );
            // Attributes are siblings of the item, not part of it: carry them so `#[should_panic]` is visible.
            f.body = format!("{}\n{}", attrs.join("\n"), f.body);
            out.push(f);
        }
        true
    });
    out
}

/// Text of the consecutive `#[...]` attribute items directly above `node`, outermost first.
fn leading_attributes(node: tree_sitter::Node, source: &str) -> Vec<String> {
    let mut attrs = Vec::new();
    let mut prev = node.prev_sibling();
    while let Some(p) = prev {
        match RustKind::of(p) {
            RustKind::AttributeItem => attrs.push(node_text(p, source).to_string()),
            RustKind::LineComment | RustKind::BlockComment => {}
            _ => break,
        }
        prev = p.prev_sibling();
    }
    attrs.reverse();
    attrs
}

/// `#[test]`, `#[tokio::test]`, `#[tokio::test(flavor = "...")]`; `#[rstest]`-style is not matched.
fn is_test_attribute(attr: &str) -> bool {
    let inner = attr.trim().trim_start_matches("#[").trim_end_matches(']');
    let path = inner.split('(').next().unwrap_or("").trim();
    path == "test" || path.ends_with("::test")
}

fn calls_in(root: tree_sitter::Node, source: &str) -> Vec<AssertionCall> {
    let mut calls = Vec::new();
    walk_preorder(root, &mut |node| {
        match RustKind::of(node) {
            RustKind::CallExpression => {
                if let Some(func) = node.child_by_field_name("function") {
                    calls.push(AssertionCall {
                        callee: node_text(func, source).to_string(),
                        args: node
                            .child_by_field_name("arguments")
                            .map_or(0, |a| a.named_child_count()),
                    });
                }
            }
            RustKind::MacroInvocation => {
                if let Some(mac) = node.child_by_field_name("macro") {
                    calls.push(AssertionCall {
                        callee: format!("{}!", node_text(mac, source)),
                        args: macro_arg_count(node),
                    });
                }
            }
            _ => {}
        }
        true
    });
    calls
}

/// Top-level comma-separated groups in the macro's token tree (`assert_eq!(a, f(b, c))` is 2).
fn macro_arg_count(mac: tree_sitter::Node) -> usize {
    let mut cursor = mac.walk();
    let Some(tokens) = mac
        .children(&mut cursor)
        .find(|c| RustKind::of(*c) == RustKind::TokenTree)
    else {
        return 0;
    };
    let mut tc = tokens.walk();
    let inner: Vec<_> = tokens.children(&mut tc).collect();
    let body = &inner[1.min(inner.len())..inner.len().saturating_sub(1)];
    if body.is_empty() {
        return 0;
    }
    let commas = body.iter().filter(|c| c.kind() == ",").count();
    let trailing = body.last().is_some_and(|c| c.kind() == ",");
    commas + 1 - usize::from(trailing)
}

#[cfg(test)]
mod tests {
    #![allow(non_snake_case)]
    use super::super::test_support::functions;
    use super::*;

    fn rs(src: &str) -> Vec<TestFn> {
        functions(Language::Rust, src)
    }

    #[test]
    fn test_functions_should_SkipNonTests_When_OnlyAttributedFunctionsCount() {
        let src = "fn helper() { assert!(true) }\n#[test]\nfn t() { assert_eq!(1, f(2, 3)); }\n#[tokio::test]\nasync fn u() {}\n";
        let fns = rs(src);
        assert_eq!(
            fns.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(),
            ["t", "u"]
        );
        assert_eq!(fns[0].line, 3);
        let calls = profile().assertion_calls(&fns[0]);
        assert_eq!(
            calls,
            vec![AssertionCall {
                callee: "assert_eq!".into(),
                args: 2
            }]
        );
    }

    #[test]
    fn is_exempt_should_SeeShouldPanic_When_AttributeAboveFunction() {
        let fns = rs("#[test]\n#[should_panic]\nfn t() { f() }\n");
        assert!(profile().is_exempt(&fns[0]));
        assert!(profile().assertion_calls(&fns[0]).is_empty());
    }

    #[test]
    fn macro_args_should_CountTopLevelCommas_When_TrailingCommaAndMessage() {
        let fns = rs("#[test]\nfn t() { assert!(x, \"msg {}\", y,); assert!(z); }\n");
        let args: Vec<_> = profile()
            .assertion_calls(&fns[0])
            .iter()
            .map(|c| c.args)
            .collect();
        assert_eq!(args, [3, 1]);
    }
}
