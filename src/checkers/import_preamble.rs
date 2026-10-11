//! Leading import/package/use preamble detection, shared by every duplicate-code
//! checker. Import lines are dictated by what a file references, so a repeated run of
//! them isn't something a shared function could remove. Callers go through
//! [`non_preamble_windows`] so none can forget the filter.

use std::path::Path;

use crate::checkers::duplicate_code::{MIN_BLOCK_LINES, qualifying_windows};

/// Languages whose import preamble we recognise. Anything else has no preamble.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PreambleLang {
    Go,
    JvmKt,
    Rust,
    Python,
    Js,
}

impl PreambleLang {
    fn from_ext(ext: &str) -> Option<Self> {
        let is = |names: &[&str]| names.iter().any(|n| ext.eq_ignore_ascii_case(n));
        if is(&["go"]) {
            Some(Self::Go)
        } else if is(&["kt", "kts", "java"]) {
            Some(Self::JvmKt)
        } else if is(&["rs"]) {
            Some(Self::Rust)
        } else if is(&["py"]) {
            Some(Self::Python)
        } else if is(&["js", "jsx", "ts", "tsx", "mjs", "cjs", "mts", "cts"]) {
            Some(Self::Js)
        } else {
            None
        }
    }
}

/// File extension of `path`, or `""` when it has none or isn't UTF-8.
pub(crate) fn lang_ext(path: &Path) -> &str {
    path.extension().and_then(|e| e.to_str()).unwrap_or("")
}

/// Every qualifying `MIN_BLOCK_LINES`-line window in already-trimmed `lines` that is not
/// entirely inside the file's leading import preamble (language inferred from `ext`), as
/// `(0-indexed start, window slice)`.
pub(crate) fn non_preamble_windows<'a>(
    lines: &'a [String],
    ext: &str,
) -> impl Iterator<Item = (usize, &'a [String])> {
    let import_end = leading_import_end_for(lines, PreambleLang::from_ext(ext));
    qualifying_windows(lines).filter(move |(start, _)| !in_preamble(*start, import_end))
}

/// Returns the exclusive end index of the file's leading import/use preamble: the first
/// line (of already-trimmed `lines`) that is not blank, a comment, or an import/package/use
/// declaration for the language inferred from `ext`. Multi-line imports (`from a import (`,
/// `import {`, `use a::{`, Go `import (`) stay preamble until they close. Returns 0 for
/// unrecognised extensions (nothing is suppressed) and `lines.len()` when the whole file
/// is preamble.
#[cfg(test)]
fn leading_import_end(lines: &[String], ext: &str) -> usize {
    leading_import_end_for(lines, PreambleLang::from_ext(ext))
}

fn leading_import_end_for(lines: &[String], lang: Option<PreambleLang>) -> usize {
    let Some(lang) = lang else {
        return 0;
    };
    let mut state = PreambleState::default();
    for (i, line) in lines.iter().enumerate() {
        if !state.accepts(line, lang) {
            return state.abandoned_group_start.unwrap_or(i);
        }
    }
    // A group still open at EOF never closed: treat it as malformed, not as preamble.
    state.group_start.unwrap_or(lines.len())
}

/// Whether the `MIN_BLOCK_LINES` window at `start` lies entirely inside the preamble.
fn in_preamble(start: usize, import_end: usize) -> bool {
    start + MIN_BLOCK_LINES <= import_end
}

/// An open bracket group that hasn't closed after this many lines is assumed to be an
/// unmatched opener we misread, not a real import list; the preamble ends where it began.
const MAX_GROUP_LINES: usize = 100;

#[derive(Default)]
struct PreambleState {
    /// Unclosed `(`/`{`/`[` from a multi-line import statement.
    depth: usize,
    in_block_comment: bool,
    in_go_import_group: bool,
    /// Closing delimiter of a leading Python module docstring we're inside.
    docstring_delim: Option<&'static str>,
    seen_import: bool,
    line_no: usize,
    /// Line index where the currently open bracket group began.
    group_start: Option<usize>,
    /// Set once `group_start` exceeded `MAX_GROUP_LINES`.
    abandoned_group_start: Option<usize>,
}

impl PreambleState {
    fn accepts(&mut self, line: &str, lang: PreambleLang) -> bool {
        let line_no = self.line_no;
        self.line_no += 1;
        let accepted = self.accepts_line(line, lang);
        let open = self.depth > 0 || self.in_go_import_group;
        match (open, self.group_start) {
            (true, None) => self.group_start = Some(line_no),
            (false, Some(_)) => self.group_start = None,
            _ => {}
        }
        if let Some(start) = self.group_start
            && line_no - start >= MAX_GROUP_LINES
        {
            self.abandoned_group_start = Some(start);
            return false;
        }
        accepted
    }

    fn accepts_line(&mut self, line: &str, lang: PreambleLang) -> bool {
        use PreambleLang::{Go, Js, Python};
        if self.in_block_comment {
            self.in_block_comment = !line.contains("*/");
            return true;
        }
        if let Some(delim) = self.docstring_delim {
            if line.contains(delim) {
                self.docstring_delim = None;
            }
            return true;
        }
        if line.is_empty()
            || (lang == Python && line.starts_with('#'))
            || (lang != Python && line.starts_with("//"))
            || (lang == Js && line.starts_with("#!"))
        {
            return true;
        }
        if lang != Python
            && let Some(rest) = line.strip_prefix("/*")
        {
            self.in_block_comment = !rest.contains("*/");
            return true;
        }
        if self.in_go_import_group {
            self.in_go_import_group = !line.starts_with(')');
            return true;
        }
        if self.depth > 0 {
            self.depth = adjust_depth(self.depth, line, lang);
            return true;
        }
        if lang == Python && !self.seen_import && self.open_docstring(line) {
            return true;
        }
        if !is_import_statement(line, lang) {
            return false;
        }
        self.seen_import = true;
        if lang == Go {
            let code = strip_noise(line, lang);
            self.in_go_import_group = (code.starts_with("import (") || code.starts_with("import("))
                && !code.contains(')');
        } else {
            self.depth = adjust_depth(0, line, lang);
        }
        true
    }

    /// True when `line` starts a module docstring; records its delimiter if it doesn't
    /// close on the same line.
    fn open_docstring(&mut self, line: &str) -> bool {
        for delim in ["\"\"\"", "\'\'\'"] {
            if let Some(rest) = line.strip_prefix(delim) {
                if !rest.contains(delim) {
                    self.docstring_delim = Some(delim);
                }
                return true;
            }
        }
        false
    }
}

fn adjust_depth(depth: usize, line: &str, lang: PreambleLang) -> usize {
    strip_noise(line, lang).chars().fold(depth, |d, c| match c {
        '(' | '{' | '[' => d + 1,
        ')' | '}' | ']' => d.saturating_sub(1),
        _ => d,
    })
}

/// `line` with string literals and trailing/inline comments removed, so delimiters inside
/// them (`from "a(b"`, `// (see x`) aren't counted. Not a full lexer: an unterminated
/// quote drops the rest of the line.
fn strip_noise(line: &str, lang: PreambleLang) -> String {
    use PreambleLang::{Go, Js, JvmKt, Python, Rust};
    let quotes: &[char] = match lang {
        Rust => &['"'],
        Go | Js => &['"', '\'', '`'],
        JvmKt | Python => &['"', '\''],
    };
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    let mut quote: Option<char> = None;
    while let Some(c) = chars.next() {
        if let Some(q) = quote {
            if c == '\\' && q != '`' {
                chars.next();
            } else if c == q {
                quote = None;
            }
            continue;
        }
        if quotes.contains(&c) {
            quote = Some(c);
            continue;
        }
        if lang == Python {
            if c == '#' {
                break;
            }
        } else if c == '/' {
            match chars.peek() {
                Some('/') => break,
                Some('*') => {
                    chars.next();
                    let mut prev = ' ';
                    let mut closed = false;
                    for d in chars.by_ref() {
                        if prev == '*' && d == '/' {
                            closed = true;
                            break;
                        }
                        prev = d;
                    }
                    if !closed {
                        break;
                    }
                    continue;
                }
                _ => {}
            }
        }
        out.push(c);
    }
    out
}

/// First line of an import/package/use declaration (comments and blanks are handled by the caller).
fn is_import_statement(line: &str, lang: PreambleLang) -> bool {
    match lang {
        // Go grouped-import bodies are tracked by `in_go_import_group`, not matched here,
        // so `var s = "b"` / `fmt.Println("a", "b")` can't pass as import specs.
        PreambleLang::Go => {
            line.starts_with("package ")
                || line.starts_with("import ")
                || line.starts_with("import(")
        }
        PreambleLang::JvmKt => {
            line.starts_with("package ")
                || line.starts_with("import ")
                || line.starts_with("@file:")
        }
        PreambleLang::Rust => {
            let item = strip_rust_visibility(line);
            item.starts_with("use ")
                || item.starts_with("extern crate ")
                || item.starts_with("#![")
                // `mod x {` opens a body, so only the declaration form is preamble
                || (item.starts_with("mod ") && item.ends_with(';'))
        }
        PreambleLang::Python => line.starts_with("import ") || line.starts_with("from "),
        PreambleLang::Js => line.starts_with("import ") || line.starts_with("export * from "),
    }
}

fn strip_rust_visibility(line: &str) -> &str {
    if let Some(rest) = line.strip_prefix("pub(") {
        return rest.split_once(')').map_or(line, |(_, r)| r.trim_start());
    }
    line.strip_prefix("pub ").unwrap_or(line)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::checker::{CheckContext, Checker, Finding};
    use crate::checkers::duplicate_code::{
        CrossFileDuplicate, DuplicateCodeChecker, find_cross_file_duplicates,
    };

    const KOTLIN_IMPORTS: &str = "import arrow.core.Either\n\
                                   import arrow.core.left\n\
                                   import arrow.core.right\n\
                                   import dev.stapler.stelekit.db.GraphLoader\n\
                                   import dev.stapler.stelekit.db.GraphWriter\n\
                                   import dev.stapler.stelekit.error.DomainError\n";

    fn lines_of(src: &str) -> Vec<String> {
        src.lines().map(|l| l.trim().to_string()).collect()
    }

    fn cross_file(ext: &str, preamble: &str, body: &str) -> Vec<CrossFileDuplicate> {
        let files: Vec<(PathBuf, String)> = ["a", "b", "c"]
            .iter()
            .map(|n| {
                (
                    PathBuf::from(format!("{n}.{ext}")),
                    format!("{preamble}\n{body}"),
                )
            })
            .collect();
        find_cross_file_duplicates(&files)
    }

    fn single_file(ext: &str, src: &str) -> Vec<Finding> {
        let ctx = CheckContext {
            source: src,
            tree: None,
        };
        DuplicateCodeChecker
            .check(Path::new(&format!("<source>.{ext}")), &ctx)
            .unwrap()
    }

    #[test]
    fn leading_import_end_kotlin() {
        let src = format!("package a.b\n\n{KOTLIN_IMPORTS}\nclass Foo\n");
        let lines = lines_of(&src);
        assert_eq!(leading_import_end(&lines, "kt"), lines.len() - 1);
        assert_eq!(lines[lines.len() - 1], "class Foo");
    }

    #[test]
    fn leading_import_end_go_grouped_import() {
        let src = "package main\n\nimport (\n\t\"fmt\"\n\tfoo \"example.com/foo\"\n\t_ \"example.com/bar\"\n)\n\nfunc main() {}\n";
        let lines = lines_of(src);
        assert_eq!(leading_import_end(&lines, "go"), lines.len() - 1);
    }

    #[test]
    fn leading_import_end_rust() {
        let src = "#![allow(dead_code)]\nuse std::fmt;\nuse std::io;\n\nfn main() {}\n";
        let lines = lines_of(src);
        assert_eq!(leading_import_end(&lines, "rs"), lines.len() - 1);
    }

    #[test]
    fn leading_import_end_python() {
        let src = "import os\nfrom a import b\n\ndef f():\n    pass\n";
        let lines = lines_of(src);
        assert_eq!(leading_import_end(&lines, "py"), 3);
    }

    #[test]
    fn leading_import_end_unknown_ext_is_zero() {
        let lines = lines_of("import a\nimport b\n");
        assert_eq!(leading_import_end(&lines, "txt"), 0);
    }

    #[test]
    fn leading_import_end_all_preamble_file_covers_whole_file() {
        let lines = lines_of(KOTLIN_IMPORTS);
        assert_eq!(leading_import_end(&lines, "kt"), lines.len());
    }

    #[test]
    fn does_not_flag_kotlin_import_block_cross_file() {
        let pre = format!("package a.b\n\n{KOTLIN_IMPORTS}");
        let body = "class A { val x = 1 }";
        assert!(cross_file("kt", &pre, body).is_empty());
    }

    #[test]
    fn does_not_flag_java_import_block_cross_file() {
        let pre = "package a;\n\nimport java.util.List;\nimport java.util.Map;\nimport java.util.Set;\nimport java.util.HashMap;\nimport java.util.HashSet;\nimport java.util.ArrayList;\n";
        assert!(cross_file("java", pre, "class A {}").is_empty());
    }

    #[test]
    fn does_not_flag_go_import_block_cross_file() {
        let pre = "package main\n\nimport (\n\t\"context\"\n\t\"encoding/json\"\n\t\"fmt\"\n\t\"net/http\"\n\t\"os\"\n\t\"strings\"\n)\n";
        assert!(cross_file("go", pre, "func a() {}").is_empty());
    }

    #[test]
    fn does_not_flag_python_and_js_family_import_blocks_cross_file() {
        let py = "import os\nimport sys\nimport json\nfrom collections import OrderedDict\nfrom typing import Optional\nfrom pathlib import Path\n";
        assert!(cross_file("py", py, "def f(): pass").is_empty());
        let js = "import React from 'react';\nimport { useState } from 'react';\nimport axios from 'axios';\nimport lodash from 'lodash';\nimport moment from 'moment';\nimport styles from './styles.css';\n";
        for ext in ["js", "ts", "tsx", "jsx"] {
            assert!(
                cross_file(ext, js, "export const a = 1;").is_empty(),
                "{ext}"
            );
        }
    }

    const RUST_USE_BLOCK: &str = "use std::collections::HashMap;\nuse std::collections::HashSet;\nuse std::path::{Path, PathBuf};\nuse anyhow::Result;\nuse serde::Serialize;\nuse serde::Deserialize;\n";

    #[test]
    fn does_not_flag_rust_use_block() {
        // Three copies: only the leading one is preamble, so without the filter the
        // block would reach MIN_OCCURRENCES; with it, two remain and nothing fires.
        let three =
            format!("{RUST_USE_BLOCK}fn a() {{}}\n{RUST_USE_BLOCK}fn b() {{}}\n{RUST_USE_BLOCK}");
        assert!(single_file("rs", &three).is_empty());
        // A fourth copy leaves three non-preamble occurrences, a real finding.
        let four = format!("{three}fn c() {{}}\n{RUST_USE_BLOCK}");
        assert_eq!(single_file("rs", &four).len(), 1);
    }

    #[test]
    fn rust_use_block_in_three_files_is_suppressed_by_the_filter() {
        let files: Vec<(PathBuf, String)> = ["a", "b", "c"]
            .iter()
            .map(|n| {
                (
                    PathBuf::from(format!("{n}.rs")),
                    format!("{RUST_USE_BLOCK}fn {n}() {{}}\n"),
                )
            })
            .collect();
        assert!(find_cross_file_duplicates(&files).is_empty());
    }

    #[test]
    fn go_value_declarations_and_calls_are_not_preamble() {
        let lines = lines_of("package main\n\nvar a = \"x\"\nvar b = \"y\"\n");
        assert_eq!(leading_import_end(&lines, "go"), 2);
        let lines = lines_of("package main\nfmt.Println(\"a\", \"b\")\n");
        assert_eq!(leading_import_end(&lines, "go"), 1);
    }

    #[test]
    fn still_flags_repeated_go_var_runs_across_files() {
        let body = "var a = \"alpha-value\"\nvar b = \"beta-value\"\nvar c = \"gamma-value\"\nvar d = \"delta-value\"\nvar e = \"epsilon-value\"\nvar f = \"zeta-value\"\n";
        assert_eq!(cross_file("go", "package main\n", body).len(), 1);
    }

    #[test]
    fn python_parenthesized_import_is_preamble_until_closed() {
        let src = "from a import (\n    b,\n    c,\n)\nimport os\n\ndef f():\n    pass\n";
        let lines = lines_of(src);
        assert_eq!(leading_import_end(&lines, "py"), 6);
        let block = "from pkg.module_one import (\n    alpha_function,\n    beta_function,\n    gamma_function,\n    delta_function,\n)\n";
        assert!(cross_file("py", block, "def f(): pass").is_empty());
    }

    #[test]
    fn js_multiline_import_is_preamble_until_closed() {
        let src = "import {\n  a,\n  b,\n} from 'x';\nimport y from 'y';\n\nexport const z = 1;\n";
        let lines = lines_of(src);
        for ext in ["js", "ts", "tsx", "jsx"] {
            assert_eq!(leading_import_end(&lines, ext), 6, "{ext}");
        }
        let block = "import {\n  alphaFunction,\n  betaFunction,\n  gammaFunction,\n  deltaFunction,\n} from './module';\n";
        assert!(cross_file("ts", block, "export const a = 1;").is_empty());
    }

    #[test]
    fn rust_multiline_use_and_mod_declarations_are_preamble() {
        let src = "use foo::{\n    a,\n    b,\n};\npub(crate) use bar::baz;\nmod m;\npub mod n;\n\nfn main() {}\n";
        let lines = lines_of(src);
        assert_eq!(leading_import_end(&lines, "rs"), lines.len() - 1);
        // An inline module body is code, not preamble.
        let lines = lines_of("mod m {\n    fn f() {}\n}\n");
        assert_eq!(leading_import_end(&lines, "rs"), 0);
    }

    #[test]
    fn in_preamble_window_boundaries() {
        let end = 10;
        assert!(in_preamble(end - MIN_BLOCK_LINES, end));
        assert!(!in_preamble(end - MIN_BLOCK_LINES + 1, end));
        assert!(!in_preamble(end, end));
    }

    #[test]
    fn java_license_header_and_interleaved_comments_are_preamble() {
        let src = "/*\n * Copyright 2024\n * Licensed under X\n */\npackage a;\n// group one\nimport java.util.List;\n/* inline */\nimport java.util.Map;\n\nclass A {}\n";
        let lines = lines_of(src);
        assert_eq!(leading_import_end(&lines, "java"), lines.len() - 1);
    }

    #[test]
    fn block_comment_without_star_prefixed_lines_is_preamble() {
        let src = "/*\nlicense text\nmore text\n*/\npackage a;\nclass A {}\n";
        let lines = lines_of(src);
        assert_eq!(leading_import_end(&lines, "java"), 5);
    }

    #[test]
    fn python_comments_and_module_docstring_before_imports_are_preamble() {
        let src = "#!/usr/bin/env python\n# comment\n\"\"\"Module doc.\n\nMore doc.\n\"\"\"\nimport os\nimport sys\n\ndef f():\n    pass\n";
        let lines = lines_of(src);
        assert_eq!(leading_import_end(&lines, "py"), 9);
        let one_line = lines_of("\"\"\"Doc.\"\"\"\nimport os\n\ndef f(): pass\n");
        assert_eq!(leading_import_end(&one_line, "py"), 3);
    }

    #[test]
    fn still_flags_duplicate_after_import_block() {
        let pre = format!("package a.b\n\n{KOTLIN_IMPORTS}\n");
        let body = "fun doWork(id: String): Result {\n\
                    \tval conn = openConnection(id)\n\
                    \tconn.use { c ->\n\
                    \t\tval result = c.fetch(id)\n\
                    \t\tlog.info(\"fetched $result\")\n\
                    \t\treturn c.validate(result)\n\
                    \t}\n}\n";
        let dups = cross_file("kt", &pre, body);
        assert_eq!(dups.len(), 1);
        assert!(dups[0].occurrences.iter().all(|o| o.line > 8));
    }

    #[test]
    fn does_not_suppress_mid_file_duplicate_that_happens_to_use_import_syntax() {
        // The first non-import line ends the preamble, so an identical import-looking
        // run after it is ordinary code and still counts.
        let mid = KOTLIN_IMPORTS;
        let src = format!("package a.b\n\nclass A\n\n{mid}\n{mid}\n{mid}");
        let findings = single_file("kt", &src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn unmatched_opener_in_string_or_comment_does_not_swallow_the_file() {
        let js = "import x from \"a(b\"\nimport y from \"y\";\n\nexport const z = 1;\n";
        assert_eq!(leading_import_end(&lines_of(js), "ts"), 3);
        let js = "import a from 'b' // (see x\nimport y from 'y';\n\nexport const z = 1;\n";
        assert_eq!(leading_import_end(&lines_of(js), "js"), 3);
        let py = "from a import b  # (legacy\nimport os\n\ndef f():\n    pass\n";
        assert_eq!(leading_import_end(&lines_of(py), "py"), 3);
        let kt = "@file:JvmName(\"Utils\")\n@file:Suppress(\"a(b\")\npackage a\n\nclass A\n";
        assert_eq!(leading_import_end(&lines_of(kt), "kt"), 4);
    }

    #[test]
    fn group_that_never_closes_is_abandoned_at_its_start() {
        let mut src = String::from("import a from 'a';\nimport x; const o = {\n");
        for i in 0..150 {
            src.push_str(&format!("  key{i}: value{i},\n"));
        }
        src.push_str("};\nexport const z = 1;\n");
        assert_eq!(leading_import_end(&lines_of(&src), "ts"), 1);
        // Unclosed at end of file is also abandoned.
        let eof = "import a from 'a';\nimport x; const o = {\n  a: 1,\n";
        assert_eq!(leading_import_end(&lines_of(eof), "ts"), 1);
    }

    #[test]
    fn real_duplicates_survive_an_unmatched_opener_in_the_preamble() {
        let block = "function doWork(id) {\n\
                     \tconst conn = openConnection(id);\n\
                     \tconn.configure(defaultOptions);\n\
                     \tconst result = conn.fetch(id);\n\
                     \tlogger.info('fetched', result);\n\
                     \treturn conn.validate(result);\n}\n";
        let src = format!("import x from \"a(b\";\n\n{block}\n{block}\n{block}");
        assert_eq!(single_file("ts", &src).len(), 1);
    }

    #[test]
    fn js_shebang_java_import_static_and_kotlin_file_annotations_are_preamble() {
        let js = "#!/usr/bin/env node\nimport a from 'a';\n\nconst x = 1;\n";
        for ext in ["js", "ts"] {
            assert_eq!(leading_import_end(&lines_of(js), ext), 3, "{ext}");
        }
        let java = "package a;\nimport static a.B.c;\nimport static a.B.d;\n\nclass A {}\n";
        assert_eq!(leading_import_end(&lines_of(java), "java"), 4);
        let kt = "@file:JvmName(\"Utils\")\n@file:Suppress(\"unused\")\npackage a\n\nclass A\n";
        assert_eq!(leading_import_end(&lines_of(kt), "kt"), 4);
    }

    #[test]
    fn extension_matching_is_case_insensitive_and_covers_script_variants() {
        let js = lines_of("import a from 'a';\n\nconst x = 1;\n");
        for ext in ["TS", "Tsx", "mjs", "cjs", "mts", "cts"] {
            assert_eq!(leading_import_end(&js, ext), 2, "{ext}");
        }
        let kt = lines_of("package a\n\nclass A\n");
        for ext in ["KT", "kts", "JAVA"] {
            assert_eq!(leading_import_end(&kt, ext), 2, "{ext}");
        }
    }

    #[test]
    fn leading_star_outside_a_block_comment_is_code() {
        let lines = lines_of("package main\n*p = 1\n");
        assert_eq!(leading_import_end(&lines, "go"), 1);
    }
}
