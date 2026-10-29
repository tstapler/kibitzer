//! Node-context exclusions for `replace-magic-literal` (see `docs/syntax-rules.md`): where a
//! literal sits, rather than what it says. Split out of `rules.rs` to keep that file's size
//! in check.

use std::collections::{HashMap, HashSet};

use tree_sitter::Node;

use super::rules::LiteralOccurrence;

/// Attribute/annotation/decorator nodes across the supported grammars (Rust, Java/Kotlin,
/// Python and TS/JS). Their arguments are configuration for a framework, not logic.
const ATTRIBUTE_KINDS: &[&str] = &[
    "attribute_item",
    "inner_attribute_item",
    "annotation",
    "marker_annotation",
    "decorator",
];

/// Collection nodes whose direct elements form a data row: Go `literal_value`, Rust
/// array/tuple and macro `token_tree` (`vec![..]`), Python list/tuple, TS/JS `array`,
/// Java `array_initializer`.
const DATA_ROW_CONTAINERS: &[&str] = &[
    "literal_value",
    "array_expression",
    "tuple_expression",
    "token_tree",
    "list",
    "tuple",
    "array",
    "array_initializer",
];

/// A row of at least this many numeric elements is a table of data, not tunable constants.
const DATA_ROW_MIN_ELEMENTS: usize = 4;

/// Wrappers around a numeric element: Go's `literal_element` and the unary minus of each grammar.
const NUMERIC_WRAPPER_KINDS: &[&str] = &[
    "literal_element",
    "unary_expression",
    "unary_operator",
    "negative_literal",
];

/// Type suffixes (Rust `u8`/`f32`, Java/Kotlin `L`/`f`/`d`/`u`), longest first.
const NUMERIC_SUFFIXES: &[&str] = &[
    "usize", "isize", "u128", "i128", "u64", "i64", "u32", "i32", "u16", "i16", "u8", "i8", "f32",
    "f64", "UL", "ul", "u", "U", "l", "L", "f", "F", "d", "D",
];

/// Node ids of literal occurrences excluded by position: inside an attribute, a numeric
/// data-table row, and (when `skip_test_strings`) a string in test code. `in_test_file`
/// is the path-based verdict; a Rust `#[cfg(test)]` item is detected from the tree.
pub(super) fn structural_exclusions(
    occurrences: &HashMap<String, Vec<LiteralOccurrence>>,
    numeric_kinds: &[&str],
    src: &[u8],
    test_strings: Option<bool>,
) -> HashSet<usize> {
    let mut excluded = HashSet::new();
    let mut file_is_cfg_test: Option<bool> = None;
    for occ in occurrences.values().flatten() {
        let node = occ.node;
        let numeric = numeric_kinds.contains(&occ.kind);
        let skip = in_attribute(node)
            || (numeric && in_numeric_data_row(node, numeric_kinds))
            || (!numeric
                && test_strings.is_some_and(|in_test_file| {
                    in_test_file
                        || *file_is_cfg_test.get_or_insert_with(|| root_is_cfg_test(node, src))
                        || in_cfg_test_item(node, src)
                }));
        if skip {
            excluded.insert(node.id());
        }
    }
    excluded
}

fn in_attribute(node: Node) -> bool {
    if let Some(parent) = node.parent()
        && parent.kind() == "field_declaration"
        && parent.child_by_field_name("tag") == Some(node)
    {
        return true;
    }
    let mut cur = node.parent();
    while let Some(n) = cur {
        if ATTRIBUTE_KINDS.contains(&n.kind()) {
            return true;
        }
        cur = n.parent();
    }
    false
}

/// The numeric literal under `node` once `literal_element`/unary-minus wrappers are peeled.
fn numeric_core<'a>(node: Node<'a>, numeric_kinds: &[&str]) -> Option<Node<'a>> {
    let mut cur = node;
    while NUMERIC_WRAPPER_KINDS.contains(&cur.kind()) {
        cur = cur.named_child(cur.named_child_count().checked_sub(1)? as u32)?;
    }
    numeric_kinds.contains(&cur.kind()).then_some(cur)
}

fn in_numeric_data_row(node: Node, numeric_kinds: &[&str]) -> bool {
    let mut element = node;
    let mut parent = node.parent();
    while let Some(p) = parent
        && NUMERIC_WRAPPER_KINDS.contains(&p.kind())
    {
        element = p;
        parent = p.parent();
    }
    let Some(container) = parent else {
        return false;
    };
    if !DATA_ROW_CONTAINERS.contains(&container.kind()) || element.id() == container.id() {
        return false;
    }
    let mut cursor = container.walk();
    let numeric_siblings = container
        .named_children(&mut cursor)
        .filter(|c| numeric_core(*c, numeric_kinds).is_some())
        .count();
    numeric_siblings >= DATA_ROW_MIN_ELEMENTS
}

fn is_cfg_test_attribute(node: Node, src: &[u8]) -> bool {
    ATTRIBUTE_KINDS.contains(&node.kind())
        && node.utf8_text(src).is_ok_and(|t| {
            let compact: String = t.chars().filter(|c| !c.is_whitespace()).collect();
            compact == "#[cfg(test)]" || compact == "#![cfg(test)]"
        })
}

/// A Rust item preceded by `#[cfg(test)]`, on the literal or any enclosing item.
fn in_cfg_test_item(node: Node, src: &[u8]) -> bool {
    let mut cur = Some(node);
    while let Some(n) = cur {
        let mut sib = n.prev_named_sibling();
        while let Some(s) = sib {
            if is_cfg_test_attribute(s, src) {
                return true;
            }
            if !ATTRIBUTE_KINDS.contains(&s.kind()) && !s.kind().ends_with("comment") {
                break;
            }
            sib = s.prev_named_sibling();
        }
        cur = n.parent();
    }
    false
}

/// A crate-level `#![cfg(test)]` makes the whole file test code.
fn root_is_cfg_test(node: Node, src: &[u8]) -> bool {
    let mut root = node;
    while let Some(p) = root.parent() {
        root = p;
    }
    let mut cursor = root.walk();
    root.named_children(&mut cursor)
        .any(|c| c.kind().starts_with("inner_") && is_cfg_test_attribute(c, src))
}

/// Decimal literal with digit separators, type suffix and a trailing `.0` removed, so
/// `0u8`, `1f`, `1.0` and `1_u32` compare equal to the allowlisted `0`/`1`. Radix-prefixed
/// literals are returned untouched (`0x1f` ends in a hex digit, not a suffix).
pub(super) fn normalize_numeric(value: &str) -> String {
    let mut s: String = value.chars().filter(|c| *c != '_').collect();
    let lower = s.to_ascii_lowercase();
    if ["0x", "0b", "0o"].iter().any(|p| lower.starts_with(p)) {
        return s;
    }
    if let Some(suffix) = NUMERIC_SUFFIXES.iter().find(|x| s.ends_with(**x)) {
        s.truncate(s.len() - suffix.len());
    }
    if let Some((int, frac)) = s.split_once('.')
        && frac.chars().all(|c| c == '0')
        && !int.is_empty()
    {
        return int.to_string();
    }
    s
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use std::path::Path;

    use super::normalize_numeric;
    use crate::checker::{CheckContext, Checker, Language};
    use crate::checkers::rules::SyntaxRulesChecker;

    fn ts_grammar(lang: Language) -> tree_sitter::Language {
        match lang {
            Language::Go => tree_sitter_go::LANGUAGE.into(),
            Language::Python => tree_sitter_python::LANGUAGE.into(),
            Language::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Language::Java => tree_sitter_java::LANGUAGE.into(),
            Language::Kotlin => tree_sitter_kotlin_ng::LANGUAGE.into(),
            Language::Rust => tree_sitter_rust::LANGUAGE.into(),
            _ => unreachable!("not exercised here"),
        }
    }

    fn count(lang: Language, path: &str, src: &str) -> usize {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&ts_grammar(lang)).unwrap();
        let tree = parser.parse(src, None).unwrap();
        let ctx = CheckContext {
            source: src,
            tree: Some(&tree),
        };
        SyntaxRulesChecker::new(lang)
            .check(Path::new(path), &ctx)
            .unwrap()
            .iter()
            .filter(|f| f.message.contains("[replace-magic-literal]"))
            .count()
    }

    /// `n` statements binding `lit` to a fresh local, in `lang`'s syntax.
    fn repeated(lang: Language, lit: &str, n: usize) -> String {
        let stmts = |f: &dyn Fn(usize) -> String| (0..n).map(f).collect::<String>();
        match lang {
            Language::Go => format!(
                "package m\nfunc f() {{\n{}}}\n",
                stmts(&|i| format!("\tv{i} := {lit}\n"))
            ),
            Language::Python => format!("def f():\n{}", stmts(&|i| format!("    v{i} = {lit}\n"))),
            Language::TypeScript => format!(
                "function f() {{\n{}}}\n",
                stmts(&|i| format!("  let v{i} = {lit};\n"))
            ),
            Language::Java => format!(
                "class A {{ void f() {{\n{}}} }}\n",
                stmts(&|i| format!("  var v{i} = {lit};\n"))
            ),
            Language::Kotlin => format!(
                "fun f() {{\n{}}}\n",
                stmts(&|i| format!("  var v{i} = {lit}\n"))
            ),
            Language::Rust => format!(
                "fn f() {{\n{}}}\n",
                stmts(&|i| format!("    let v{i} = {lit};\n"))
            ),
            _ => unreachable!(),
        }
    }

    const STR: &str = "\"two words\"";

    #[test]
    fn strings_should_BeSkipped_When_InTestFileOfAnyLanguage() {
        for (lang, prod, test) in [
            (Language::Go, "a.go", "a_test.go"),
            (Language::Python, "a.py", "test_a.py"),
            (Language::TypeScript, "a.ts", "a.test.ts"),
            (Language::Java, "A.java", "ATest.java"),
            (Language::Kotlin, "A.kt", "ATest.kt"),
            (Language::Rust, "a.rs", "tests/a.rs"),
        ] {
            let src = repeated(lang, STR, 12);
            assert_eq!(count(lang, prod, &src), 1, "{prod} should flag");
            assert_eq!(count(lang, test, &src), 0, "{test} should skip strings");
        }
    }

    #[test]
    fn numerics_should_KeepTestThreshold_When_InTestFile() {
        let src = repeated(Language::Go, "86400", 12);
        assert_eq!(count(Language::Go, "a_test.go", &src), 1);
        let few = repeated(Language::Go, "86400", 9);
        assert_eq!(count(Language::Go, "a_test.go", &few), 0);
    }

    #[test]
    fn production_error_message_should_StillFlag_When_RepeatedInNonTestFile() {
        let src = repeated(Language::Go, "\"failed to open config file\"", 3);
        assert_eq!(count(Language::Go, "server.go", &src), 1);
    }

    #[test]
    fn strings_should_BeSkipped_When_InsideRustCfgTestModule() {
        let body = repeated(Language::Rust, STR, 3);
        let in_mod = format!("#[cfg(test)]\nmod t {{\n{body}}}\n");
        assert_eq!(count(Language::Rust, "src/a.rs", &body), 1);
        assert_eq!(count(Language::Rust, "src/a.rs", &in_mod), 0);
        let on_fn = format!("fn g() {{}}\n#[cfg(test)]\n{body}");
        assert_eq!(count(Language::Rust, "src/a.rs", &on_fn), 0);
        let other_attr = format!("#[derive(Debug)]\nmod t {{\n{body}}}\n");
        assert_eq!(count(Language::Rust, "src/a.rs", &other_attr), 1);
        let inner = format!("#![cfg(test)]\n{body}");
        assert_eq!(count(Language::Rust, "src/a.rs", &inner), 0);
    }

    #[test]
    fn strings_should_StillFlag_When_ProdCodeFollowsRustCfgTestModule() {
        let prod = repeated(Language::Rust, STR, 3);
        let src = format!("#[cfg(test)]\nmod t {{ fn x() {{ let a = 1; }} }}\n{prod}");
        assert_eq!(count(Language::Rust, "src/a.rs", &src), 1);
    }

    #[test]
    fn rust_attribute_arguments_should_BeSkipped_When_RepeatedInAttributes() {
        let attrs = "struct S {\n#[serde(rename = \"two words\")] a: u8,\n#[serde(rename = \"two words\")] b: u8,\n#[serde(rename = \"two words\")] c: u8,\n}\n";
        assert_eq!(count(Language::Rust, "src/a.rs", attrs), 0);
        let inner = "#![doc = \"two words\"]\n#![doc = \"two words\"]\n#![doc = \"two words\"]\n";
        assert_eq!(count(Language::Rust, "src/a.rs", inner), 0);
        let plain = repeated(Language::Rust, STR, 3);
        assert_eq!(count(Language::Rust, "src/a.rs", &plain), 1);
    }

    #[test]
    fn decorator_arguments_should_BeSkipped_When_RepeatedInPythonOrTs() {
        let py = "@route(\"two words\")\ndef a(): pass\n@route(\"two words\")\ndef b(): pass\n@route(\"two words\")\ndef c(): pass\n";
        assert_eq!(count(Language::Python, "a.py", py), 0);
        let py_plain = repeated(Language::Python, STR, 3);
        assert_eq!(count(Language::Python, "a.py", &py_plain), 1);
        let ts = "class A {\n@Input(\"two words\") a = 0;\n@Input(\"two words\") b = 0;\n@Input(\"two words\") c = 0;\n}\n";
        assert_eq!(count(Language::TypeScript, "a.ts", ts), 0);
        let ts_plain = repeated(Language::TypeScript, STR, 3);
        assert_eq!(count(Language::TypeScript, "a.ts", &ts_plain), 1);
    }

    #[test]
    fn annotation_arguments_should_BeSkipped_When_RepeatedInJavaOrKotlin() {
        let java = "class A {\n@Named(\"two words\") int a;\n@Named(\"two words\") int b;\n@Named(\"two words\") int c;\n}\n";
        assert_eq!(count(Language::Java, "A.java", java), 0);
        let java_plain = repeated(Language::Java, STR, 3);
        assert_eq!(count(Language::Java, "A.java", &java_plain), 1);
        let kt = "class A {\n@Named(\"two words\") val a = x\n@Named(\"two words\") val b = x\n@Named(\"two words\") val c = x\n}\n";
        assert_eq!(count(Language::Kotlin, "A.kt", kt), 0);
        let kt_plain = repeated(Language::Kotlin, STR, 3);
        assert_eq!(count(Language::Kotlin, "A.kt", &kt_plain), 1);
    }

    #[test]
    fn struct_tags_should_BeSkipped_When_RepeatedInGoStructs() {
        let tag = "`json:\"a,omitempty\"`";
        let tags = format!(
            "package m\ntype T struct {{\n\tA int {tag}\n\tB int {tag}\n\tC int {tag}\n}}\n"
        );
        assert_eq!(count(Language::Go, "a.go", &tags), 0);
        assert_eq!(
            count(Language::Go, "a.go", &repeated(Language::Go, tag, 3)),
            1
        );
    }

    #[test]
    fn numerics_should_BeSkipped_When_TypeSuffixNormalizesToAllowlisted() {
        for (lang, lit) in [
            (Language::Rust, "0u8"),
            (Language::Rust, "1_u32"),
            (Language::Rust, "1.0"),
            (Language::Rust, "100f64"),
            (Language::Kotlin, "1f"),
            (Language::Java, "1L"),
            (Language::Java, "1.0"),
            (Language::Python, "1.0"),
            (Language::Go, "1.0"),
            (Language::TypeScript, "1_000"),
        ] {
            assert_eq!(count(lang, "a", &repeated(lang, lit, 3)), 0, "{lit}");
        }
        let sixty = repeated(Language::Rust, "60u8", 3);
        assert_eq!(count(Language::Rust, "a.rs", &sixty), 1);
        let frac = repeated(Language::Rust, "0.75f32", 3);
        assert_eq!(count(Language::Rust, "a.rs", &frac), 1);
    }

    #[test]
    fn normalize_numeric_should_LeaveRadixLiteralsAlone_When_HexDigitLooksLikeSuffix() {
        assert_eq!(normalize_numeric("0x1f"), "0x1f");
        assert_eq!(normalize_numeric("1_u32"), "1");
        assert_eq!(normalize_numeric("2.0"), "2");
        assert_eq!(normalize_numeric("2.50"), "2.50");
        assert_eq!(normalize_numeric("0.0"), "0");
    }

    /// One row of `per_row` numbers inside `open`/`close`.
    fn row(open: &str, close: &str, per_row: usize) -> String {
        let nums = ["53", "61", "67", "71", "79"][..per_row].join(", ");
        format!("{open}{nums}{close}")
    }

    #[test]
    fn numeric_rows_should_BeSkipped_When_FourOrMoreElementsPerRow() {
        // The same row three times: each number repeats 3x, so a 3-wide row is flagged
        // once per number (3 findings) and a 4-wide row not at all.
        let go = |n| {
            let r = row("{", "}", n);
            format!("package m\nvar t = [][]int{{\n{r},\n{r},\n{r},\n}}\n")
        };
        assert_eq!(count(Language::Go, "a.go", &go(4)), 0);
        assert_eq!(count(Language::Go, "a.go", &go(3)), 3);
        let py = |n| {
            let r = row("[", "]", n);
            format!("t = [\n{r},\n{r},\n{r},\n]\n")
        };
        assert_eq!(count(Language::Python, "a.py", &py(4)), 0);
        assert_eq!(count(Language::Python, "a.py", &py(3)), 3);
        let ts = |n| {
            let r = row("[", "]", n);
            format!("const t = [\n{r},\n{r},\n{r},\n];\n")
        };
        assert_eq!(count(Language::TypeScript, "a.ts", &ts(4)), 0);
        assert_eq!(count(Language::TypeScript, "a.ts", &ts(3)), 3);
        let rs = |open, close, n| {
            let r = row(open, close, n);
            format!("fn f() {{ let t = [\n{r},\n{r},\n{r},\n]; }}\n")
        };
        assert_eq!(count(Language::Rust, "a.rs", &rs("[", "]", 4)), 0);
        assert_eq!(count(Language::Rust, "a.rs", &rs("[", "]", 3)), 3);
        assert_eq!(count(Language::Rust, "a.rs", &rs("(", ")", 5)), 0);
        assert_eq!(count(Language::Rust, "a.rs", &rs("(", ")", 3)), 3);
        let java = |n| {
            let r = row("{", "}", n);
            format!("class A {{ void f() {{ int[][] t = {{\n{r},\n{r},\n{r}\n}}; }} }}\n")
        };
        assert_eq!(count(Language::Java, "A.java", &java(4)), 0);
        assert_eq!(count(Language::Java, "A.java", &java(3)), 3);
    }

    #[test]
    fn numeric_rows_should_BeSkipped_When_NegativeElementsFillTheRow() {
        let r = "{53, -61, 67, -71}";
        let src = format!("package m\nvar t = [][]int{{\n{r},\n{r},\n{r},\n}}\n");
        assert_eq!(count(Language::Go, "a.go", &src), 0);
    }

    #[test]
    fn numerics_should_StillFlag_When_RepeatedAsCallArguments() {
        let call = "\tg(53, 61, 67, 71)\n";
        let src = format!("package m\nfunc f() {{\n{call}{call}{call}}}\n");
        assert_eq!(count(Language::Go, "a.go", &src), 4);
    }

    #[test]
    fn single_char_strings_should_BeSkipped_When_RepeatedThreeTimes() {
        for (lang, lit) in [
            (Language::Go, "\",\""),
            (Language::Python, "'x'"),
            (Language::TypeScript, "'#'"),
            (Language::Rust, "\"|\""),
        ] {
            assert_eq!(count(lang, "a", &repeated(lang, lit, 3)), 0, "{lit}");
        }
        let two = repeated(Language::Go, "\"->\"", 3);
        assert_eq!(count(Language::Go, "a.go", &two), 1);
    }

    #[test]
    fn generated_file_should_BeSkipped_When_HeaderIsOnLine34() {
        let mut src = "// Copyright The Authors.\n".repeat(33);
        src.push_str("// Code generated by protoc-gen-go. DO NOT EDIT.\n");
        src.push_str(&repeated(Language::Go, "86400", 3));
        assert_eq!(count(Language::Go, "api.pb.go", &src), 0);
        let hand_written = src.replace("Code generated", "Hand written");
        assert_eq!(count(Language::Go, "api.go", &hand_written), 1);
    }
}
