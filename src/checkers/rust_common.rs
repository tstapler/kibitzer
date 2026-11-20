//! Helpers shared by the Rust-specific checkers (`sentinel-return-rust`, `rust-ignored-error`,
//! `rust-error-context`, `split-loop-rust`).

use tree_sitter::Node;

/// Looks through trailing `.await`s to the awaited expression.
pub(super) fn strip_await(mut node: Node) -> Node {
    while node.kind() == "await_expression" {
        match node.named_child(0) {
            Some(inner) => node = inner,
            None => break,
        }
    }
    node
}

/// Last path or method segment of callee text (`a::b::f` / `x.send` / `f::<T>` -> `f`/`send`/`f`).
pub(super) fn callee_last_segment(callee: &str) -> &str {
    let last = callee.rsplit(['.', ':']).next().unwrap_or(callee);
    last.split('<').next().unwrap_or(last)
}

/// Longest quoted snippet of source a finding message embeds.
pub(super) const SNIPPET_MAX: usize = 60;

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

/// True inside a test-only item: `#[cfg(..test..)]` (not `not(test)`), `#[cfg_attr(test, ..)]`,
/// `#[test]`, `#[tokio::test]`, `#[rstest]`, or a module named `tests`.
pub(super) fn in_test_code(node: Node, source: &str) -> bool {
    let mut cur = Some(node);
    while let Some(n) = cur {
        if matches!(n.kind(), "mod_item" | "function_item" | "impl_item")
            && is_test_attrs(&leading_attrs(n, source))
        {
            return true;
        }
        cur = n.parent();
    }
    false
}

fn is_test_attrs(attrs: &str) -> bool {
    attrs.split("#[").skip(1).any(|a| {
        let head = a.split(['(', ']']).next().unwrap_or("").trim();
        // `cfg_attr(test, X)` only decorates; the item still builds in production.
        (head == "cfg" && cfg_enables_only_for_test(a))
            || head == "test"
            || head == "rstest"
            || head.ends_with("::test")
    })
}

/// True when a bare `test` predicate appears outside any `not(..)` and outside string literals:
/// `cfg(test)`, `cfg(all(test, ..))`, `cfg(any(test, ..))`; not `cfg(feature = "test")` or
/// `cfg(not(any(test, ..)))`.
fn cfg_enables_only_for_test(attr: &str) -> bool {
    let b = attr.as_bytes();
    let mut stack: Vec<&str> = Vec::new();
    let mut last_ident = "";
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'"' => {
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    i += usize::from(b[i] == b'\\') + 1;
                }
                i += 1;
            }
            b'(' => {
                stack.push(last_ident);
                last_ident = "";
                i += 1;
            }
            b')' => {
                stack.pop();
                i += 1;
            }
            c if c.is_ascii_alphabetic() || c == b'_' => {
                let start = i;
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                    i += 1;
                }
                last_ident = &attr[start..i];
                let rest = attr[i..].trim_start();
                if last_ident == "test" && !rest.starts_with('=') && !stack.contains(&"not") {
                    return true;
                }
            }
            _ => i += 1,
        }
    }
    false
}

/// Whether `file` is under a `tests/`, `benches/` or `examples/` directory, or is a `tests.rs`
/// module file. Matches any path component, so a checkout under an ancestor directory with one of
/// those names is skipped wholesale (documented limit).
pub(super) fn is_test_path(file: &std::path::Path) -> bool {
    file.file_name().is_some_and(|n| n == "tests.rs")
        || file.components().any(|c| {
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
