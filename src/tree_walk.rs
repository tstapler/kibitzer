use tree_sitter::Node;

/// Iterative preorder tree-sitter descent: visits `node`, then every descendant, in the
/// same order a naive `fn walk(node) { visit(node); for child in node.children() {
/// walk(child) } }` would — but using `TreeCursor`'s own `goto_first_child`/
/// `goto_next_sibling`/`goto_parent` to move around the tree instead of one Rust stack
/// frame per tree-depth level. `TreeCursor` already tracks the ancestor chain
/// internally, so this needs no manual stack.
///
/// Extracted after a corpus backtest sweep flagged the naive recursive form as a
/// duplicated block across `primitive_obsession.rs`, `java_ignored_error.rs`,
/// `java_swallowed_interrupt.rs`, and `java_lost_exception_cause.rs`
/// (2026-09-18) — but the reason to actually adopt it is stack safety, not just
/// deduplication: the naive form allocates a fresh `TreeCursor` and grows the Rust call
/// stack by one frame per level of tree *depth* (not node count), so a pathologically
/// nested or generated input (this repo's own `docs/backtest-repos.md` corpus includes
/// multi-million-line real-world repos) could in principle exhaust the default thread
/// stack. This walker's stack usage is O(1) regardless of tree depth.
///
/// `visit` returns whether to descend into that node's children — `false` lets a caller
/// that needs to thread extra per-subtree state (e.g.
/// `java_lost_exception_cause.rs`'s `enclosing_catch_name`, which changes when entering
/// a nested `catch_clause`) recurse into that subtree itself with a fresh closure, then
/// tell this walker not to also visit it generically. A caller with no such state just
/// returns `true` unconditionally.
pub(crate) fn walk_preorder<'a>(node: Node<'a>, visit: &mut impl FnMut(Node<'a>) -> bool) {
    let mut cursor = node.walk();
    loop {
        let descend = visit(cursor.node());
        if descend && cursor.goto_first_child() {
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_go(src: &str) -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_go::LANGUAGE.into())
            .unwrap();
        parser.parse(src, None).unwrap()
    }

    #[test]
    fn visits_every_node_in_preorder() {
        let tree = parse_go("package main\nfunc f() { g(); h() }\n");
        let mut kinds = Vec::new();
        walk_preorder(tree.root_node(), &mut |n| {
            kinds.push(n.kind().to_string());
            true
        });
        // Preorder: the root and every descendant appear, root first.
        assert_eq!(kinds.first().unwrap(), "source_file");
        assert!(kinds.iter().any(|k| k == "function_declaration"));
        assert!(kinds.iter().any(|k| k == "call_expression"));
    }

    #[test]
    fn returning_false_skips_that_subtree() {
        let tree = parse_go("package main\nfunc f() { g() }\n");
        let mut visited_call = false;
        walk_preorder(tree.root_node(), &mut |n| {
            if n.kind() == "function_declaration" {
                return false; // prune: never descend into f's body
            }
            if n.kind() == "call_expression" {
                visited_call = true;
            }
            true
        });
        assert!(!visited_call);
    }

    #[test]
    fn matches_naive_recursive_descent_node_count() {
        fn naive_count(node: Node, count: &mut usize) {
            *count += 1;
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                naive_count(child, count);
            }
        }

        let tree = parse_go(
            "package main\nfunc f(a, b string) { if a == b { g(a) } else { h(b) } }\n",
        );
        let mut naive = 0;
        naive_count(tree.root_node(), &mut naive);

        let mut iterative = 0;
        walk_preorder(tree.root_node(), &mut |_| {
            iterative += 1;
            true
        });

        assert_eq!(naive, iterative);
    }
}
