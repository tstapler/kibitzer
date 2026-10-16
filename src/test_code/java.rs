//! Java and Kotlin profile: methods carrying `@Test`-family annotations.
//! Kinds are matched by string because the two grammars have distinct typed enums.

use std::path::Path;

use tree_sitter::Node;

use crate::checker::{CheckContext, Language};
use crate::tree_walk::walk_preorder;

use super::{AssertionCall, TestCodeProfile, TestFn, make_test_fn, node_text};

const ASSERTIONS: &[&str] = &[
    "assert*",
    "Assertions.*",
    "Assert.*",
    "MatcherAssert.*",
    "kotlin.test.assert*",
    "verify*",
    "coVerify*",
    "Mockito.verify*",
    "fail",
    "Assert.fail",
];
const SNAPSHOT_MARKERS: &[&str] = &[
    "Approvals",
    "toMatchSnapshot",
    "assertMatchesSnapshot",
    "matchesSnapshot",
];
const EXEMPTION_MARKERS: &[&str] = &["@Disabled", "@Ignore"];
const TEST_ANNOTATIONS: &[&str] = &[
    "@Test",
    "@ParameterizedTest",
    "@RepeatedTest",
    "@TestFactory",
];

pub(super) fn profile(language: Language) -> TestCodeProfile {
    TestCodeProfile {
        language,
        assertion_names: ASSERTIONS,
        snapshot_markers: SNAPSHOT_MARKERS,
        exemption_markers: EXEMPTION_MARKERS,
        is_test_file: Some(is_test_file),
        collect: if language == Language::Kotlin {
            collect_kotlin
        } else {
            collect_java
        },
    }
}

fn is_test_file(path: &Path) -> bool {
    let in_test_tree = path.components().any(|c| c.as_os_str() == "test")
        && path.components().any(|c| c.as_os_str() == "src");
    let Some(stem) = path.file_stem().and_then(|n| n.to_str()) else {
        return false;
    };
    let ext_ok = matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("java" | "kt")
    );
    ext_ok
        && (in_test_tree
            || stem.ends_with("Test")
            || stem.ends_with("Tests")
            || stem.ends_with("IT")
            || stem.starts_with("Test"))
}

fn collect_java(ctx: &CheckContext) -> Vec<TestFn> {
    collect(Language::Java, "method_declaration", ctx)
}

fn collect_kotlin(ctx: &CheckContext) -> Vec<TestFn> {
    collect(Language::Kotlin, "function_declaration", ctx)
}

fn collect(language: Language, function_kind: &str, ctx: &CheckContext) -> Vec<TestFn> {
    let Some(tree) = ctx.tree else {
        return Vec::new();
    };
    let profile = profile(language);
    let mut out = Vec::new();
    walk_preorder(tree.root_node(), &mut |node| {
        if node.kind() != function_kind || !has_test_annotation(node, ctx.source) {
            return true;
        }
        let name = function_name(node, ctx.source);
        out.push(make_test_fn(
            &profile,
            name,
            node,
            node,
            ctx.source,
            calls_in(language, node, ctx.source),
        ));
        false
    });
    out
}

fn function_name<'a>(node: Node, source: &'a str) -> &'a str {
    if let Some(name) = node.child_by_field_name("name") {
        return node_text(name, source).trim_matches('`');
    }
    let mut cursor = node.walk();
    let found = node
        .children(&mut cursor)
        .find(|c| matches!(c.kind(), "simple_identifier" | "identifier"));
    found.map_or("", |n| node_text(n, source)).trim_matches('`')
}

fn has_test_annotation(node: Node, source: &str) -> bool {
    let mut cursor = node.walk();
    let modifiers = node.children(&mut cursor).find(|c| c.kind() == "modifiers");
    let Some(text) = modifiers.map(|m| node_text(m, source)) else {
        return false;
    };
    TEST_ANNOTATIONS
        .iter()
        .any(|a| contains_annotation(text, a))
}

/// `@Test` must not match `@TestFactory` or `@TestOnly`.
fn contains_annotation(text: &str, annotation: &str) -> bool {
    text.match_indices(annotation).any(|(i, _)| {
        text[i + annotation.len()..]
            .chars()
            .next()
            .is_none_or(|c| !(c.is_alphanumeric() || c == '_'))
    })
}

fn calls_in(language: Language, root: Node, source: &str) -> Vec<AssertionCall> {
    let mut calls = Vec::new();
    walk_preorder(root, &mut |node| {
        let call = if language == Language::Kotlin {
            kotlin_call(node, source)
        } else {
            java_call(node, source)
        };
        calls.extend(call);
        true
    });
    calls
}

fn java_call(node: Node, source: &str) -> Option<AssertionCall> {
    if node.kind() != "method_invocation" || is_receiver_of(node, "method_invocation") {
        return None;
    }
    let root = java_root(node);
    Some(AssertionCall {
        callee: java_callee(root, source),
        args: root
            .child_by_field_name("arguments")
            .map_or(0, |a| a.named_child_count()),
    })
}

/// Innermost call of a chain: `verify(m)` in `verify(m).run()`; its name and arguments describe the chain.
fn java_root(mut call: Node) -> Node {
    while let Some(obj) = call
        .child_by_field_name("object")
        .filter(|o| o.kind() == "method_invocation")
    {
        call = obj;
    }
    call
}

fn java_callee(call: Node, source: &str) -> String {
    let name = call
        .child_by_field_name("name")
        .map_or("", |n| node_text(n, source));
    match call.child_by_field_name("object") {
        Some(obj) if obj.kind() == "method_invocation" => java_callee(obj, source),
        Some(obj) => format!("{}.{name}", node_text(obj, source)),
        None => name.to_string(),
    }
}

/// Whether `node` is the receiver (`object`) of a chained call, counted once by the outer call.
fn is_receiver_of(node: Node, parent_kind: &str) -> bool {
    node.parent()
        .is_some_and(|p| p.kind() == parent_kind && p.child_by_field_name("object") == Some(node))
}

fn kotlin_call(node: Node, source: &str) -> Option<AssertionCall> {
    if node.kind() != "call_expression" {
        return None;
    }
    let is_receiver = node
        .parent()
        .is_some_and(|p| p.kind() == "navigation_expression" && p.named_child(0) == Some(node));
    if is_receiver {
        return None;
    }
    let root = kotlin_root(node);
    Some(AssertionCall {
        callee: node_text(root.named_child(0)?, source).to_string(),
        args: kotlin_arg_count(root),
    })
}

/// Innermost call of a chain: `verify(m)` in `verify(m).run()`; its name and arguments describe the chain.
fn kotlin_root(mut call: Node) -> Node {
    while let Some(recv) = call
        .named_child(0)
        .filter(|c| c.kind() == "navigation_expression")
        .and_then(|nav| nav.named_child(0))
        .filter(|r| r.kind() == "call_expression")
    {
        call = recv;
    }
    call
}

/// Value arguments plus a trailing lambda, which this grammar parses as sibling children of the call.
fn kotlin_arg_count(call: Node) -> usize {
    let mut cursor = call.walk();
    call.named_children(&mut cursor)
        .map(|c| match c.kind() {
            "value_arguments" => c.named_child_count(),
            "annotated_lambda" => 1,
            _ => 0,
        })
        .sum()
}

#[cfg(test)]
mod tests {
    #![allow(non_snake_case)]
    use super::super::test_support::functions;
    use super::*;

    const JAVA: &str = "class T {\n  @Test\n  void adds() {\n    assertThat(add(1, 2)).isEqualTo(3);\n    verify(mock).run();\n  }\n  @Test @Disabled\n  void later() {}\n  void helper() { assertEquals(1, 1); }\n  @TestFactory\n  void factory() {}\n}\n";
    const KOTLIN: &str = "class T {\n  @Test\n  fun adds() {\n    assertThat(add(1, 2)).isEqualTo(3)\n    verify { mock.run() }\n    assertEquals(1, 2)\n  }\n  @Test @Ignore\n  fun later() {}\n  fun helper() { assertEquals(1, 1) }\n}\n";

    #[test]
    fn is_test_file_should_MatchSuffixAndSrcTestTree_When_JavaKotlinPaths() {
        assert!(is_test_file(Path::new("src/main/java/FooTest.java")));
        assert!(is_test_file(Path::new("app/src/test/java/Foo.kt")));
        assert!(is_test_file(Path::new("FooIT.java")));
        assert!(!is_test_file(Path::new("src/main/java/Foo.java")));
    }

    #[test]
    fn test_functions_should_FindAnnotatedMethods_When_Java() {
        let fns = functions(Language::Java, JAVA);
        let names: Vec<_> = fns.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["adds", "later", "factory"]);
        assert_eq!(
            fns[0].line, 2,
            "line is the method node, annotation included"
        );
        let p = profile(Language::Java);
        let calls = p.assertion_calls(&fns[0]);
        assert_eq!(
            calls,
            vec![
                AssertionCall {
                    callee: "assertThat".into(),
                    args: 1
                },
                AssertionCall {
                    callee: "verify".into(),
                    args: 1
                },
            ]
        );
        assert!(p.is_exempt(&fns[1]));
    }

    #[test]
    fn test_functions_should_FindAnnotatedFunctions_When_Kotlin() {
        let fns = functions(Language::Kotlin, KOTLIN);
        let names: Vec<_> = fns.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["adds", "later"]);
        let p = profile(Language::Kotlin);
        let calls = p.assertion_calls(&fns[0]);
        let shape: Vec<_> = calls.iter().map(|c| (c.callee.as_str(), c.args)).collect();
        assert_eq!(
            shape,
            [("assertThat", 1), ("verify", 1), ("assertEquals", 2)]
        );
        assert!(p.is_exempt(&fns[1]));
    }

    #[test]
    fn contains_annotation_should_RejectLongerNames_When_PrefixMatches() {
        assert!(contains_annotation("@Test", "@Test"));
        assert!(contains_annotation("@Test(timeout = 1)", "@Test"));
        assert!(!contains_annotation("@TestFactory", "@Test"));
    }
}
