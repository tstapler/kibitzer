//! Helpers shared by the Rust-specific checkers (`sentinel-return-rust`, `rust-ignored-error`,
//! `rust-error-context`, `split-loop-rust`).

use tree_sitter::Node;

/// Text of the consecutive `#[...]` attributes directly above `node`.
pub(super) fn leading_attrs(node: Node, source: &str) -> String {
    let mut out = String::new();
    let mut prev = node.prev_sibling();
    while let Some(p) = prev {
        match p.kind() {
            "attribute_item" => out.push_str(&source[p.byte_range()]),
            "line_comment" | "block_comment" => {}
            _ => break,
        }
        prev = p.prev_sibling();
    }
    out
}

/// True inside a `#[cfg(test)]` item or a `#[test]`/`#[tokio::test]` function.
pub(super) fn in_test_code(node: Node, source: &str) -> bool {
    let mut cur = Some(node);
    while let Some(n) = cur {
        if matches!(n.kind(), "mod_item" | "function_item" | "impl_item") {
            let attrs = leading_attrs(n, source);
            if attrs.contains("cfg(test)")
                || attrs.contains("#[test]")
                || attrs.contains("::test]")
                || attrs.contains("::test(")
            {
                return true;
            }
        }
        cur = n.parent();
    }
    false
}

/// Whether `file` is under a `tests/`, `benches/` or `examples/` directory.
pub(super) fn is_test_path(file: &std::path::Path) -> bool {
    file.components().any(|c| {
        matches!(
            c.as_os_str().to_str(),
            Some("tests" | "benches" | "examples")
        )
    })
}

pub(super) fn text<'a>(node: Node, source: &'a str) -> &'a str {
    &source[node.byte_range()]
}

/// Whitespace-collapsed `s`, cut to `max` chars with an ellipsis.
pub(super) fn one_line(s: &str, max: usize) -> String {
    let one = s.split_whitespace().collect::<Vec<_>>().join(" ");
    match one.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &one[..i]),
        None => one,
    }
}
