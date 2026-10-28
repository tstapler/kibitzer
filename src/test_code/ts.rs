//! TypeScript / TSX / JavaScript profile: `it(...)` / `test(...)` calls (Jest, Vitest, Mocha style).
//! Kinds are matched by string because three grammars share this file and each has its own typed enum.

use std::path::Path;

use tree_sitter::Node;

use crate::checker::{CheckContext, Language};
use crate::tree_walk::walk_preorder;

use super::{AssertionCall, TestCodeProfile, TestFn, make_test_fn, named_arg_count, node_text};

const ASSERTIONS: &[&str] = &["expect*", "assert*", "fail", "sinon.assert*", "chai.expect"];
const SNAPSHOT_MARKERS: &[&str] = &[
    "toMatchSnapshot",
    "toMatchInlineSnapshot",
    "toMatchFileSnapshot",
];
const EXEMPTION_MARKERS: &[&str] = &[
    "it.skip(",
    "test.skip(",
    "xit(",
    "xtest(",
    "it.todo(",
    "test.todo(",
    "describe.skip",
];
const TEST_NAMES: &[&str] = &["it", "test", "xit", "xtest", "fit", "specify"];
const TEST_MODIFIERS: &[&str] = &["only", "skip", "todo", "concurrent", "failing"];
const TEST_FILE_EXTENSIONS: &[&str] = &["ts", "tsx", "js", "jsx", "mjs", "cjs", "mts", "cts"];
const FUNCTION_KINDS: &[&str] = &["arrow_function", "function_expression", "function"];

pub(super) fn profile(language: Language) -> TestCodeProfile {
    TestCodeProfile {
        language,
        assertion_names: ASSERTIONS,
        snapshot_markers: SNAPSHOT_MARKERS,
        exemption_markers: EXEMPTION_MARKERS,
        is_test_file: Some(is_test_file),
        collect: collect_for(language),
    }
}

fn collect_for(language: Language) -> fn(&CheckContext) -> Vec<TestFn> {
    match language {
        Language::TypeScript => |ctx| collect(Language::TypeScript, ctx),
        Language::Tsx => |ctx| collect(Language::Tsx, ctx),
        _ => |ctx| collect(Language::JavaScript, ctx),
    }
}

fn is_test_file(path: &Path) -> bool {
    let in_tests_dir = path.components().any(|c| c.as_os_str() == "__tests__");
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let mut parts = name.rsplit('.');
    let (Some(ext), Some(marker)) = (parts.next(), parts.next()) else {
        return false;
    };
    let suffixed = (marker == "test" || marker == "spec") && parts.next().is_some();
    TEST_FILE_EXTENSIONS.contains(&ext) && (suffixed || in_tests_dir)
}

fn collect(language: Language, ctx: &CheckContext) -> Vec<TestFn> {
    let Some(tree) = ctx.tree else {
        return Vec::new();
    };
    let profile = profile(language);
    let mut out = Vec::new();
    walk_preorder(tree.root_node(), &mut |node| {
        if node.kind() != "call_expression" {
            return true;
        }
        let Some(func) = node.child_by_field_name("function") else {
            return true;
        };
        if !is_test_callee(func, ctx.source) {
            return true;
        }
        let Some(args) = node.child_by_field_name("arguments") else {
            return true;
        };
        let Some(body) = last_function_argument(args) else {
            return true;
        };
        let name = args
            .named_child(0)
            .map_or("", |n| node_text(n, ctx.source))
            .trim_matches(['"', '\'', '`']);
        out.push(make_test_fn(
            &profile,
            name,
            node,
            node,
            ctx.source,
            calls_in(body, ctx.source),
        ));
        false
    });
    out
}

fn last_function_argument(args: Node) -> Option<Node> {
    let last = args.named_child(u32::try_from(args.named_child_count().checked_sub(1)?).ok()?)?;
    FUNCTION_KINDS.contains(&last.kind()).then_some(last)
}

/// `it`, `test.only`, `xit`, `test.each(table)` and friends.
fn is_test_callee(func: Node, source: &str) -> bool {
    if func.kind() == "call_expression" {
        let inner = func.child_by_field_name("function");
        return inner.is_some_and(|f| {
            node_text(f, source)
                .strip_suffix(".each")
                .is_some_and(is_test_name)
        });
    }
    is_test_name(node_text(func, source))
}

fn is_test_name(text: &str) -> bool {
    let mut parts = text.split('.');
    let base = parts.next().unwrap_or("");
    TEST_NAMES.contains(&base) && parts.all(|p| TEST_MODIFIERS.contains(&p))
}

fn calls_in(root: Node, source: &str) -> Vec<AssertionCall> {
    let mut calls = Vec::new();
    walk_preorder(root, &mut |node| {
        if node.kind() == "call_expression" && !is_chain_receiver(node) {
            let root = root_call(node);
            if let Some(func) = root.child_by_field_name("function") {
                calls.push(AssertionCall {
                    callee: node_text(func, source).to_string(),
                    args: named_arg_count(root.child_by_field_name("arguments")),
                });
            }
        }
        true
    });
    calls
}

/// The inner `expect(x)` of `expect(x).toBe(1)`: counted once, by the outer call.
fn is_chain_receiver(call: Node) -> bool {
    call.parent().is_some_and(|p| {
        p.kind() == "member_expression" && p.child_by_field_name("object") == Some(call)
    })
}

/// Innermost call of a chain: `expect(x)` in `expect(x).not.toBe(1)`; its callee and arguments describe the chain.
fn root_call(mut call: Node) -> Node {
    loop {
        let mut node = call.child_by_field_name("function");
        while let Some(n) =
            node.filter(|n| matches!(n.kind(), "member_expression" | "non_null_expression"))
        {
            node = n.child_by_field_name("object").or_else(|| n.named_child(0));
        }
        match node.filter(|n| n.kind() == "call_expression") {
            Some(inner) => call = inner,
            None => return call,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(non_snake_case)]
    use super::super::test_support::functions;
    use super::*;
    use crate::checker::Language as L;

    const SRC: &str = "describe('suite', () => {\n  it('adds', () => {\n    expect(add(1, 2)).toBe(3);\n  });\n  test.skip('later', () => {});\n  it('noop', async () => { await run(); });\n});\n";

    #[test]
    fn is_test_file_should_MatchTestSpecAndTestsDir_When_TsJsPaths() {
        assert!(is_test_file(Path::new("src/a.test.ts")));
        assert!(is_test_file(Path::new("src/a.spec.tsx")));
        assert!(is_test_file(Path::new("src/__tests__/a.js")));
        assert!(!is_test_file(Path::new("src/a.ts")));
        assert!(!is_test_file(Path::new("src/test.ts")));
    }

    #[test]
    fn test_functions_should_FindItAndTestCalls_When_ThreeGrammars() {
        for lang in [L::TypeScript, L::Tsx, L::JavaScript] {
            let fns = functions(lang, SRC);
            let names: Vec<_> = fns.iter().map(|f| f.name.as_str()).collect();
            assert_eq!(names, ["adds", "later", "noop"], "{lang:?}");
            assert_eq!(fns[0].line, 2);
        }
    }

    #[test]
    fn assertion_calls_should_ReduceChainToExpectRoot_When_MatcherChained() {
        let fns = functions(L::TypeScript, SRC);
        let p = profile(L::TypeScript);
        assert_eq!(
            p.assertion_calls(&fns[0]),
            vec![AssertionCall {
                callee: "expect".into(),
                args: 1
            }]
        );
        assert!(p.assertion_calls(&fns[2]).is_empty());
    }

    #[test]
    fn is_exempt_should_ReturnTrue_When_TestSkipped() {
        let fns = functions(L::JavaScript, SRC);
        let p = profile(L::JavaScript);
        assert!(p.is_exempt(&fns[1]));
        assert!(!p.is_exempt(&fns[0]));
    }

    #[test]
    fn classify_should_TagSnapshot_When_ToMatchSnapshot() {
        let fns = functions(
            L::TypeScript,
            "it('s', () => { expect(x).toMatchSnapshot(); });",
        );
        assert_eq!(fns[0].kind, super::super::TestKind::Snapshot);
    }

    #[test]
    fn test_functions_should_FindEachAndModifiers_When_TableTests() {
        let fns = functions(
            L::TypeScript,
            "test.each([1, 2])('n %i', (n) => { assert.equal(n, n); });\nit.only('o', () => { assert(true); });",
        );
        assert_eq!(fns.len(), 2);
        let callees: Vec<_> = fns
            .iter()
            .flat_map(|f| profile(L::TypeScript).assertion_calls(f))
            .map(|c| c.callee)
            .collect();
        assert_eq!(callees, ["assert.equal", "assert"]);
    }
}
